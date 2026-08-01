//! Phase-6 JIT, v1: translate straight-line RV64 basic blocks into wasm
//! modules (v86's endgame architecture).
//!
//! State contract with the host: the guest register file lives in the
//! module's imported linear memory —
//!
//! ```text
//! offset   0..256   x0..x31 (u64 LE)
//! offset 256        pc      (u64 LE)
//! ```
//!
//! A compiled block updates registers in place, stores the next pc, and
//! returns. The dispatcher (interpreter loop) looks up the next block by pc.
//!
//! v1 scope: OP-IMM/OP/OP-IMM-32/OP-32 (I+M subset), LUI/AUIPC, JAL/JALR,
//! conditional branches. Loads/stores/system/FP end the block and fall back
//! to the interpreter — the tiering seam v86 uses. Compressed instructions
//! are expanded through the same rv64-core expander before translation.

pub mod wasm_emit;

use rv64_core::compressed::expand;
use rv64_core::decode::*;
use wasm_emit::*;

const MAX_BLOCK: usize = 128;
/// Instruction cap for the trace (extended-basic-block) path specifically —
/// scan_regs and translate_block's basic-block loop. The loop and superblock
/// walkers keep MAX_BLOCK: their reach is tuned independently.
const MAX_TRACE: usize = 256;
/// Traces longer than this get an entry fuel guard (see translate_block):
/// short blocks' bounded overshoot is cheaper than a guard per dispatch.
const FUEL_GUARD_MIN: u32 = 48;
/// Uses (across all bodies) a register needs before a superblock caches it in
/// a wasm local instead of leaving it in the machine's register file memory.
const SB_HOIST_MIN: u32 = 8;
/// Same idea for a basic block, whose prologue/epilogue is paid on every
/// single dispatch.
const BLOCK_HOIST_MIN: u32 = 3;
/// Max iterations a compiled self-loop runs per block call before yielding to
/// the dispatcher (so an infinite guest loop still honours budget/interrupts).
const LOOP_CAP: u64 = 1 << 24;
// Scratch locals (local 0 is the state-pointer parameter).
// SCR/SCR+1 are the general ALU scratch pair used by JALR etc.; the
// memory path uses named i64 locals VA/PAGE/PA/VAL plus one i32 local IDXB.
const SCR: u32 = 1;
const VA: u32 = 1;
const PAGE: u32 = 2;
const PA: u32 = 3;
const VAL: u32 = 4;
/// Loop-iteration counter (Phase 3 self-loop compilation); also the retired-
/// instruction accumulator in compiled loops and superblocks.
const ITER: u32 = 5;
/// Superblock dispatch: the current target pc, fed to the internal `br_table`.
const TPC: u32 = 6;
/// Resolved fused-TLB offset for the access in flight (set on the hit path or
/// by the host's tlb_fill call).
const SCR2: u32 = 7;
/// Fuel remaining for THIS call: FUEL_CELL minus the instructions already
/// retired by earlier blocks of the same tail-call chain (see RETIRED_CELL's
/// cumulative contract). Loop/superblock yields compare ITER against this.
const BASE: u32 = 8;
/// i64 scratch for the chain stub (holds the next pc while it checks the line).
const CPC: u32 = 9;

/// Host-filled TLB misses inside compiled code (see tlb_idx_tag_fill).
/// DEFAULT OFF, measured: it removes 1.2M bails from an in-guest `tcc -c` for
/// no wall-clock change (the miss path was cheap), while the call site's
/// register pressure costs ~15% on CPython's eval loop, whose working set
/// never misses. Kept switchable — a guest with a larger working set than the
/// 4096-entry TLB is exactly where it would pay off.
static TLB_FILL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Direct block-to-block transfer via wasm tail calls (return_call_indirect).
/// Enabled by the host after feature-detecting tail-call support; blocks
/// compiled while off simply return to the dispatch loop as before.
static CHAIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Hardware fused multiply-add via f64x2.relaxed_madd. The host enables this
/// only after empirically proving BOTH that the engine validates the opcode
/// and that it is fused (a=b=1+2^-52, c=-(1+2^-51) must give 2^-104, not 0).
static HW_FMA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_hw_fma(on: bool) {
    HW_FMA.store(on, std::sync::atomic::Ordering::Relaxed);
}
fn hw_fma_enabled() -> bool {
    HW_FMA.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_chain(on: bool) {
    CHAIN.store(on, std::sync::atomic::Ordering::Relaxed);
}
fn chain_enabled() -> bool {
    CHAIN.load(std::sync::atomic::Ordering::Relaxed)
}
/// Trace prologue/epilogue register-traffic reduction (Ctx::defined): load
/// only what a trace reads, flush only what it has written by each exit.
/// A/B flag.
static DEFINED_TRACK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
pub fn set_defined_track(on: bool) {
    DEFINED_TRACK.store(on, std::sync::atomic::Ordering::Relaxed);
}
fn defined_track() -> bool {
    DEFINED_TRACK.load(std::sync::atomic::Ordering::Relaxed)
}

/// Rotated-nest acceptance in loop_region (backward exit branches).
/// DEFAULT OFF: a 6-boot parallel screen showed it net-negative on every
/// nbench kernel (NUMERIC SORT 392 -> 343, HUFFMAN 989 -> 904, ASSIGNMENT
/// 8.3 -> 7.9 medians) — the small regions it forms displace better
/// coverage. Kept behind jit_set_rotated_nests for future work on the
/// rotated-scan-nest shape it was built for.
static ROTATED_NESTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_rotated_nests(on: bool) {
    ROTATED_NESTS.store(on, std::sync::atomic::Ordering::Relaxed);
}
fn rotated_nests() -> bool {
    ROTATED_NESTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Trace (extended-basic-block) aggressiveness for translate_block:
/// 0 = classic basic blocks (end at every branch); 1 = side-exit conditional
/// branches and keep going; 2 = also follow direct calls (jal with link);
/// 3 = also follow proven/predicted returns (jalr on a traced constant).
static TRACE_LEVEL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(3);
pub fn set_trace_level(l: u32) {
    TRACE_LEVEL.store(l, std::sync::atomic::Ordering::Relaxed);
}
pub fn trace_level() -> u32 {
    TRACE_LEVEL.load(std::sync::atomic::Ordering::Relaxed)
}
/// Dispatch-line idx bit the HOST uses to mark region functions (for exit
/// attribution without a cache probe). Table indices stay far below it.
/// Emitted chain transfers must mask it off before call_indirect; the host
/// masks it before its own table call.
pub const SB_IDX_BIT: i32 = 1 << 30;

/// When set, translate_block_link skips the copy/loop detectors and returns
/// Block.wasm as the RAW body stream (no module wrapper, no trailing END)
/// with Block.locals filled — the shape translate_batch assembles into one
/// multi-function module. Single-threaded wasm host; contained to
/// translate_batch's scope.
static RAW_BODY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn raw_body() -> bool {
    RAW_BODY.load(std::sync::atomic::Ordering::Relaxed)
}

/// Linear-trace register value facts (see translate_block). `Proven` values
/// are set by the trace's own emitted code (jal link, lui, auipc) with no
/// later write — following them needs no runtime check. `Predicted` values
/// come from store-to-load forwarding through the stack (a non-leaf
/// prologue's `sd ra, N(sp)` reloaded by the epilogue's `ld ra, N(sp)`):
/// an aliasing store could have changed the slot, so a followed `ret` on a
/// prediction carries a one-compare runtime guard that side-exits with the
/// real target when the prediction misses.
#[derive(Copy, Clone, PartialEq)]
enum Known {
    No,
    Proven(u64),
    Predicted(u64),
}

/// Per-trace abstract state for return following: constant register facts,
/// the running sp displacement, and constant-valued stack slots keyed by
/// that displacement. Both walkers (scan_regs, translate_block) must step
/// this identically or their paths desync.
struct TraceFacts {
    known: [Known; 32],
    /// x2 displacement from trace entry, while provably constant.
    sp_delta: Option<i64>,
    /// sp-relative slots (key = sp_delta + store offset) holding a value
    /// that was Known at store time.
    slots: Vec<(i64, Known)>,
}

impl TraceFacts {
    fn new() -> TraceFacts {
        TraceFacts {
            known: [Known::No; 32],
            sp_delta: Some(0),
            slots: Vec::new(),
        }
    }
    /// Step the facts across one instruction. Call AFTER capturing any
    /// source-register facts the caller needs, with the instruction's
    /// decoded fields. Handles: rd clobber, lui/auipc constants, sp
    /// adjustment, sp-slot stores and loads. `jal` link values are the
    /// caller's job (it knows whether the jump was followed).
    fn step(&mut self, insn: u32, pc: u64) {
        let op = opcode(insn);
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
        let f3 = funct3(insn);
        // Loads first: an sp-slot reload may target its own base register.
        let loaded = if op == 0x03 && f3 == 3 && s1 == 2 {
            match self.sp_delta {
                Some(dl) => {
                    let key = dl + imm_i(insn);
                    self.slots
                        .iter()
                        .rev()
                        .find(|&&(k, _)| k == key)
                        .map(|&(_, v)| v)
                }
                None => None,
            }
        } else {
            None
        };
        // sp-slot stores record the stored register's fact (or clobber the
        // slot when the value is unknown).
        if op == 0x23 && f3 == 3 && s1 == 2 {
            if let Some(dl) = self.sp_delta {
                let key = dl + imm_s(insn);
                self.slots.retain(|&(k, _)| k != key);
                let v = self.known[s2];
                if v != Known::No {
                    self.slots.push((key, v));
                }
            }
        }
        if d != 0 {
            // Every GPR write lands in rd.
            self.known[d] = Known::No;
            if d == 2 {
                // sp rewritten: addi sp, sp, imm keeps the displacement,
                // anything else loses it (and every slot with it).
                if op == 0x13 && f3 == 0 && s1 == 2 {
                    self.sp_delta = self.sp_delta.map(|dl| dl + imm_i(insn));
                } else {
                    self.sp_delta = None;
                    self.slots.clear();
                }
            }
            match op {
                0x37 => self.known[d] = Known::Proven((insn & 0xffff_f000) as i32 as i64 as u64),
                0x17 => {
                    self.known[d] = Known::Proven(
                        pc.wrapping_add((insn & 0xffff_f000) as i32 as i64 as u64),
                    )
                }
                0x03 => {
                    if let Some(v) = loaded {
                        // A reload of a slot that held a Known value: only
                        // ever a PREDICTION (aliasing stores can't be ruled
                        // out) — usable behind a runtime guard.
                        self.known[d] = match v {
                            Known::No => Known::No,
                            Known::Proven(x) | Known::Predicted(x) => Known::Predicted(x),
                        };
                    }
                }
                _ => {}
            }
        }
    }
}
pub fn set_tlb_fill(on: bool) {
    TLB_FILL.store(on, std::sync::atomic::Ordering::Relaxed);
}
fn tlb_fill_enabled() -> bool {
    TLB_FILL.load(std::sync::atomic::Ordering::Relaxed)
}
/// Total i64 scratch locals to declare (register locals follow next; the
/// i32 IDXB local follows all i64 locals, so its index is dynamic).
const N_I64_LOCALS: u32 = 9;

/// Full-system memory access layout: emitted loads/stores probe the
/// interpreter's own Load/Store TLBs inline; on a hit within guest RAM
/// they access memory directly, otherwise they bail to the interpreter
/// (which walks the page table, fills the TLB, and handles MMIO/faults).
#[derive(Clone, Copy)]
pub struct SysMem {
    /// Fused JIT-TLB rows (tag then linear-offset), Cpu::jit_ftlb_ptrs() order.
    /// A hit means the page is directly accessible and `linear = va + off`.
    pub ftlb_load_tag: u32,
    pub ftlb_load_off: u32,
    pub ftlb_store_tag: u32,
    pub ftlb_store_off: u32,
    /// Index mask: jit_ftlb_size() - 1.
    pub tlb_mask: u32,
}

/// Where the emitted code finds emulator state in linear memory, and
/// (optionally) guest RAM for direct load/store translation.
#[derive(Clone, Copy)]
pub struct JitLayout {
    /// Linear-memory offset of x[0] (x1.. follow at 8-byte stride).
    pub x_base: u32,
    /// Linear-memory offset of the pc slot.
    pub pc_addr: u32,
    /// Flat guest RAM (user-mode): (linear offset of guest address 0,
    /// guest size). Loads/stores access it directly, bounds-checked.
    pub mem: Option<(u32, u64)>,
    /// Full-system memory layout (mutually exclusive with `mem`). When
    /// both are None, loads/stores end the block.
    pub sys: Option<SysMem>,
    /// Optional diagnostic counters: memory paths, register-boundary totals,
    /// and execution-weighted GPR entry/exit width buckets. None emits no code.
    pub mem_profile: Option<[u32; 17]>,
    /// Diagnostic-only: emit a second, semantics-preserving copy of every
    /// GPR prologue load and exit store to measure boundary-traffic leverage.
    pub reg_stress: bool,
    /// Base of 31 entry-load followed by 31 exit-store per-GPR counters.
    /// Zero disables identity profiling without emitting any counter code.
    pub reg_profile_base: u32,
    /// Diagnostic/promotion candidate: try the non-destructive LD+LHU
    /// multi-latch loop detector before the ordinary loop detector.
    pub multi_latch: bool,
    /// Cell that every block writes with the number of guest instructions
    /// it actually retired before returning. Sys blocks with inline memory
    /// ops can bail mid-block (TLB miss / MMIO), so the dispatcher must read
    /// this rather than assume the full block length.
    pub retired_addr: u32,
    /// Linear-memory offset of f[0] (FP register file; f1.. at 8-byte stride)
    /// and of the fcsr slot. Both 0 disables FP-in-block translation.
    pub f_base: u32,
    pub fcsr_addr: u32,
    /// Cell holding the instruction FUEL granted to this dispatch: compiled
    /// loops and superblocks yield once ITER reaches it, so a caller's
    /// execution budget and the interrupt quantum bound compiled-code
    /// residency (overshoot <= one loop iteration / basic block, <= MAX_BLOCK
    /// instructions). 0 = legacy fixed LOOP_CAP (tests/tools that don't
    /// meter fuel).
    pub fuel_addr: u32,
    /// Direct block chaining (tail calls): base address of the host's
    /// dispatch-line array ({pc: u64, idx: i32, gen: u32} x (mask+1)), the
    /// entry-index mask, and the address of cpu.map_gen. Zero = no chaining
    /// (user mode, diagnostics).
    pub dispatch_base: u32,
    pub dispatch_mask: u32,
    pub map_gen_addr: u32,
    /// Linear-memory offset of mstatus (system mode), or 0. When set, every
    /// compiled FP instruction bails unless mstatus.FS == Dirty — FS=Off must
    /// trap and Initial/Clean must transition to Dirty; one interpreter step
    /// does both exactly (fp_check/fp_dirty).
    pub mstatus_addr: u32,
    /// Diagnostic cell the copy-loop fast path bumps per bulk chunk (0 = off).
    pub copystat_addr: u32,
    /// i32 cell: nonzero = chain transfers disabled RIGHT NOW. Emitted chain
    /// exits load it first, so the host can kill chaining live when the
    /// block population goes megamorphic (tcc compiles 7.5k blocks and ran
    /// 2-2.9x slower with chains than without; nbench's small kernels win
    /// up to +23% chained). 0 = no cell, chains ungated.
    pub chain_off_addr: u32,
    /// i32 cell holding the global-table BASE index of the batch this body
    /// belongs to (host writes it at registration). Intra-batch links
    /// verify `line.idx == base + j` before a direct tail call, so a link
    /// never runs a member the dispatch cache has since replaced. 0 = not
    /// in a batch.
    pub batch_base_addr: u32,
}

impl JitLayout {
    /// Layout used by the standalone tests: x at 0, pc at 256, no memory.
    pub fn bare() -> JitLayout {
        JitLayout {
            x_base: 0,
            pc_addr: 256,
            mem: None,
            sys: None,
            mem_profile: None,
            reg_stress: false,
            reg_profile_base: 0,
            multi_latch: false,
            retired_addr: 264,
            f_base: 0,
            fcsr_addr: 0,
            fuel_addr: 0,
            dispatch_base: 0,
            dispatch_mask: 0,
            map_gen_addr: 0,
            mstatus_addr: 0,
            copystat_addr: 0,
            chain_off_addr: 0,
            batch_base_addr: 0,
        }
    }
}

/// Result of translating one block.
pub struct Block {
    pub wasm: Vec<u8>,
    /// Guest byte length of code consumed.
    pub len: u64,
    /// Number of instructions translated.
    pub n_insns: u32,
    /// Guest va range [lo, hi) of every instruction the block was compiled
    /// from. A trace that follows calls can span pages in either direction;
    /// the host must dirty-track and map-verify each spanned page, exactly
    /// as it does for multi-page regions. (0, 0) = the producer's code is
    /// wholly inside [start_pc, start_pc + len).
    pub span: (u64, u64),
    /// Locals declared by this block's body (i64, i32) — batch assembly
    /// re-declares them per body (see wasm_emit::finish_batch).
    pub locals: (u32, u32),
    /// The trace touches the FP register file (fp_read | fp_write): the
    /// host's claim policy keeps long INTEGER traces out of page functions
    /// but always lets functions claim FP traces — keeping FP traces made
    /// libm pages ping-pong between trace and function ownership and
    /// cratered FOURIER ~3x whenever integer keeping was enabled.
    pub uses_fp: bool,
    /// Exit-target pcs of a trace (side-exited branch arms, unfollowed jump
    /// targets, the fall-out continuation): demonstrably-on-the-hot-path
    /// block leaders the host should seed superblock discovery with. Trace
    /// compilation otherwise STARVES that discovery — interior pcs never
    /// tier up on their own, page functions get built from a handful of
    /// seeds, cover fragments, and measure catastrophically (nbench FP
    /// EMULATION 165 -> 32 iter/s without the demotion safety valve).
    pub seeds: Vec<u64>,
}

/// wasm memarg alignment hint (log2 of the natural access size).
fn len_align(len: u64) -> u64 {
    match len {
        1 => 0,
        2 => 1,
        4 => 2,
        _ => 3,
    }
}

/// Fetch helper over a code slice starting at `base` (guest address).
fn fetch(code: &[u8], base: u64, pc: u64) -> Option<(u32, u64)> {
    let off = pc.checked_sub(base)? as usize;
    let lo = u16::from_le_bytes(code.get(off..off + 2)?.try_into().ok()?) as u32;
    if lo & 3 == 3 {
        let hi = u16::from_le_bytes(code.get(off + 2..off + 4)?.try_into().ok()?) as u32;
        Some((lo | (hi << 16), 4))
    } else {
        expand(lo as u16).map(|e| (e, 2))
    }
}

struct Ctx {
    lay: JitLayout,
    /// Per-guest-register wasm local index, or 0 (= not cached, use memory).
    /// Registers a block touches live in i64 locals for the block's lifetime
    /// (v86's register_locals), eliminating the per-instruction load/store to
    /// the CPU state struct. Locals are loaded at the prologue and flushed to
    /// state at every exit / mid-block bail.
    reg_local: [u32; 32],
    /// Registers written anywhere in the block (flushed to state on exit).
    write_mask: u32,
    /// Dynamic index of the i32 IDXB scratch local (shifts with n_reg locals).
    idxb: u32,
    /// Per-FP-register i64 local index, or 0 (= not cached, use memory). Same
    /// scheme as reg_local but for f[0..31] (raw 64-bit bits, no NaN issues:
    /// FP arith reinterprets to f64 and back).
    fp_local: [u32; 32],
    /// FP registers written anywhere in the block (flushed to state on exit).
    fp_write_mask: u32,
    /// Base index of 8 i64 scratch locals for the FMADD fast path, or 0 if
    /// the block contains no FMADD-family instruction (locals are allocated
    /// only when needed — V8 zero-initializes locals per call).
    fma_scratch: u32,
    /// When a mid-block bail should report the retired count from a runtime
    /// local (the loop's ITER accumulator) rather than a compile-time constant.
    /// Set for compiled loops: an iteration count that can reach millions must
    /// be reported accurately or the system-mode kernel clock (derived from
    /// insn_count) stalls. `None` for basic blocks (retired == static index).
    retired_local: Option<u32>,
    /// Registers whose local currently holds a DEFINED value at the point
    /// being emitted: either loaded by the prologue or written since. A
    /// linear trace only needs to flush what it has actually written by
    /// each exit, and only needs to LOAD what it reads — a short trace
    /// otherwise pays a prologue load and an epilogue store for every
    /// register it merely writes (tcc's 8-19-instruction traces spend a
    /// large fraction of their work on that traffic). Loops and superblocks
    /// keep the conservative all-ones mask: an exit can be reached on a
    /// later iteration than the write that defined a register, so the
    /// statically-tracked set is not sound there.
    defined: std::cell::Cell<u32>,
    fp_defined: std::cell::Cell<u32>,
    /// (fs_bad, round_bad) i64 locals holding the FP gate's two conditions,
    /// evaluated ONCE at function entry. Nothing a compiled block executes can
    /// change mstatus.FS, frm or the sticky NX flag, so each FP body only has
    /// to test the flag instead of re-deriving it from fcsr/mstatus.
    fp_flags: Option<(u32, u32)>,
}

impl Ctx {
    fn memprof_addr_inc(m: &mut WasmModule, addr: u32) {
        if addr == 0 {
            return;
        }
        let addr = addr as u64;
        m.i32_const(0);
        m.i32_const(0).i64_load(addr).i64_const(1).op(I64_ADD);
        m.i64_store(addr);
    }

    fn memprof_inc(&self, m: &mut WasmModule, index: usize) {
        let Some(addrs) = self.lay.mem_profile else { return };
        Self::memprof_addr_inc(m, addrs[index]);
    }

    fn regprof_inc(m: &mut WasmModule, base: u32, index: usize) {
        if base != 0 {
            Self::memprof_addr_inc(m, base + index as u32 * 8);
        }
    }

    fn reg_width_bucket(n: u32) -> usize {
        if n <= 4 { 0 } else if n <= 8 { 1 } else if n <= 16 { 2 } else { 3 }
    }
    /// Emit `push x[r]` (reads the register; x0 is constant 0). Reads the
    /// cached local if the register has one, else falls back to memory.
    fn push_reg(&self, m: &mut WasmModule, r: usize) {
        if r == 0 {
            m.i64_const(0);
        } else if self.reg_local[r] != 0 {
            m.local_get(self.reg_local[r]);
        } else {
            m.i32_const(0)
                .i64_load(self.lay.x_base as u64 + r as u64 * 8);
        }
    }

    fn store_pre(&self, m: &mut WasmModule, rd: usize) -> bool {
        if rd == 0 {
            return false;
        }
        // Memory stores need the address base pushed first; local stores don't.
        if self.reg_local[rd] == 0 {
            m.i32_const(0);
        }
        true
    }

    fn store_post(&self, m: &mut WasmModule, rd: usize) {
        self.defined.set(self.defined.get() | (1 << rd));
        if self.reg_local[rd] != 0 {
            m.local_set(self.reg_local[rd]);
        } else {
            m.i64_store(self.lay.x_base as u64 + rd as u64 * 8);
        }
    }

    /// FSGNJ.D / FSGNJN.D / FSGNJX.D: rd takes rs1's magnitude with a sign
    /// taken from rs2 (f3=0), rs2 negated (f3=1), or the two signs XORed
    /// (f3=2) — the encodings C compilers emit for `copysign`, `fneg`/`fabs`
    /// (rs1 == rs2) and plain double moves. Pure bit manipulation of the raw
    /// pattern: no rounding, no exception flags, no NaN canonicalization.
    fn fp_sgnj_d(&self, m: &mut WasmModule, f3: u32, s1: usize, s2: usize, d: usize) {
        const SIGN: i64 = i64::MIN;
        self.store_freg_pre(m, d);
        if s1 == s2 && f3 == 0 {
            self.push_freg(m, s1); // fmv.d
        } else if f3 == 2 {
            // rd = rs1 ^ (rs2 & sign)
            self.push_freg(m, s1);
            self.push_freg(m, s2);
            m.i64_const(SIGN).op(I64_AND).op(I64_XOR);
        } else {
            self.push_freg(m, s1);
            m.i64_const(!SIGN).op(I64_AND);
            self.push_freg(m, s2);
            m.i64_const(SIGN).op(I64_AND);
            if f3 == 1 {
                m.i64_const(SIGN).op(I64_XOR); // sign of -rs2
            }
            m.op(I64_OR);
        }
        self.store_freg_post(m, d);
    }

    /// Push f[s]'s single-precision value as an f32, bailing if the register
    /// isn't properly NaN-boxed (high 32 bits all ones). RISC-V says an
    /// improperly boxed operand reads as a canonical NaN; that path also needs
    /// exception flags, so it goes to the interpreter — it only arises when
    /// code reinterprets a double as a float.
    fn push_f32(&self, m: &mut WasmModule, s: usize, pc: u64, n: u32) {
        self.push_freg(m, s);
        m.i64_const(32)
            .op(I64_SHR_U)
            .i64_const(0xffff_ffff)
            .op(I64_NE);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        self.push_freg(m, s);
        m.op(I32_WRAP_I64).op(F32_REINTERPRET_I32);
    }

    /// Bail unless f[s]'s single-precision exponent is finite (not 0xff).
    fn f32_finite_guard(&self, m: &mut WasmModule, s: usize, pc: u64, n: u32) {
        self.push_freg(m, s);
        m.i64_const(23)
            .op(I64_SHR_U)
            .i64_const(0xff)
            .op(I64_AND)
            .i64_const(0xff)
            .op(I64_EQ);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
    }

    /// The f32 twin of fp_result_normal_guard: VAL holds the boxed result;
    /// bail unless its exponent is normal (1..=0xfe), which is exactly when
    /// wasm's f32 op is bit-exact RNE with no flags to compute beyond the
    /// sticky NX the block gate already established.
    fn f32_result_normal_guard(&self, m: &mut WasmModule, pc: u64, n: u32) {
        m.local_get(VAL)
            .i64_const(23)
            .op(I64_SHR_U)
            .i64_const(0xff)
            .op(I64_AND)
            .i64_const(1)
            .op(I64_SUB)
            .i64_const(0xfd)
            .op(I64_GT_U);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
    }

    /// Store an f32 bit pattern (i32 on the stack) NaN-boxed into f[d].
    fn store_boxed32(&self, m: &mut WasmModule, d: usize) {
        m.op(I64_EXTEND_I32_U)
            .i64_const(0xffff_ffff_0000_0000u64 as i64)
            .op(I64_OR)
            .local_set(VAL);
        self.store_freg_pre(m, d);
        m.local_get(VAL);
        self.store_freg_post(m, d);
    }

    /// FADD/FSUB/FMUL/FDIV.S under the same eligibility as the double path:
    /// finite operands, no division by zero, normal result.
    #[allow(clippy::too_many_arguments)]
    fn fp_arith_s(&self, m: &mut WasmModule, op: u32, s1: usize, s2: usize, d: usize, pc: u64, n: u32) {
        self.f32_finite_guard(m, s1, pc, n);
        self.f32_finite_guard(m, s2, pc, n);
        if op == 3 {
            self.push_freg(m, s2);
            m.i64_const(0x7fff_ffff).op(I64_AND).op(I64_EQZ);
            m.op(IF).op(VOID);
            self.memprof_inc(m, 3);
            self.memprof_inc(m, 4);
            self.bail(m, pc, n);
            m.op(END);
        }
        self.push_f32(m, s1, pc, n);
        self.push_f32(m, s2, pc, n);
        m.op(match op {
            0 => F32_ADD,
            1 => F32_SUB,
            2 => F32_MUL,
            _ => F32_DIV,
        });
        m.op(I32_REINTERPRET_F32)
            .op(I64_EXTEND_I32_U)
            .local_set(VAL);
        self.fp_result_guard_s(m, op, s1, s2, pc, n);
        self.store_freg_pre(m, d);
        m.local_get(VAL)
            .i64_const(0xffff_ffff_0000_0000u64 as i64)
            .op(I64_OR);
        self.store_freg_post(m, d);
    }

    /// FSQRT.S — wasm f32.sqrt is exactly rounded, same guards as arith.
    fn fp_sqrt_s(&self, m: &mut WasmModule, s1: usize, d: usize, pc: u64, n: u32) {
        self.f32_finite_guard(m, s1, pc, n);
        self.push_f32(m, s1, pc, n);
        m.op(F32_SQRT)
            .op(I32_REINTERPRET_F32)
            .op(I64_EXTEND_I32_U)
            .local_set(VAL);
        self.f32_result_normal_guard(m, pc, n);
        self.store_freg_pre(m, d);
        m.local_get(VAL)
            .i64_const(0xffff_ffff_0000_0000u64 as i64)
            .op(I64_OR);
        self.store_freg_post(m, d);
    }

    /// FLE/FLT/FEQ.S into x[d]; inf/NaN operands bail (they carry NV).
    #[allow(clippy::too_many_arguments)]
    fn fp_cmp_s(&self, m: &mut WasmModule, f3: u32, s1: usize, s2: usize, d: usize, pc: u64, n: u32) {
        self.f32_finite_guard(m, s1, pc, n);
        self.f32_finite_guard(m, s2, pc, n);
        if self.store_pre(m, d) {
            self.push_f32(m, s1, pc, n);
            self.push_f32(m, s2, pc, n);
            m.op(match f3 {
                0 => F32_LE,
                1 => F32_LT,
                _ => F32_EQ,
            });
            m.op(I64_EXTEND_I32_U);
            self.store_post(m, d);
        }
    }

    /// FSGNJ/FSGNJN/FSGNJX.S — the single-precision sign ops, on the boxed
    /// low half (an improperly boxed operand bails, as elsewhere).
    #[allow(clippy::too_many_arguments)]
    fn fp_sgnj_s(&self, m: &mut WasmModule, f3: u32, s1: usize, s2: usize, d: usize, pc: u64, n: u32) {
        const SIGN: i64 = 0x8000_0000;
        self.push_freg(m, s1);
        m.i64_const(32).op(I64_SHR_U).i64_const(0xffff_ffff).op(I64_NE);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        self.store_freg_pre(m, d);
        if s1 == s2 && f3 == 0 {
            self.push_freg(m, s1);
        } else if f3 == 2 {
            self.push_freg(m, s1);
            self.push_freg(m, s2);
            m.i64_const(SIGN).op(I64_AND).op(I64_XOR);
        } else {
            self.push_freg(m, s1);
            m.i64_const(0x7fff_ffff).op(I64_AND);
            self.push_freg(m, s2);
            m.i64_const(SIGN).op(I64_AND);
            if f3 == 1 {
                m.i64_const(SIGN).op(I64_XOR);
            }
            m.op(I64_OR);
        }
        m.i64_const(0xffff_ffff_0000_0000u64 as i64).op(I64_OR);
        self.store_freg_post(m, d);
    }

    /// FCVT.S.D (rounds, guarded) and FCVT.D.S (exact widening).
    fn fp_cvt_s_d(&self, m: &mut WasmModule, s1: usize, d: usize, pc: u64, n: u32) {
        self.push_freg(m, s1);
        m.i64_const(52).op(I64_SHR_U).i64_const(0x7ff).op(I64_AND).i64_const(0x7ff).op(I64_EQ);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        self.push_freg(m, s1);
        m.op(F64_REINTERPRET_I64)
            .op(F32_DEMOTE_F64)
            .op(I32_REINTERPRET_F32)
            .op(I64_EXTEND_I32_U)
            .local_set(VAL);
        self.f32_result_normal_guard(m, pc, n);
        self.store_freg_pre(m, d);
        m.local_get(VAL)
            .i64_const(0xffff_ffff_0000_0000u64 as i64)
            .op(I64_OR);
        self.store_freg_post(m, d);
    }

    fn fp_cvt_d_s(&self, m: &mut WasmModule, s1: usize, d: usize, pc: u64, n: u32) {
        self.f32_finite_guard(m, s1, pc, n);
        self.store_freg_pre(m, d);
        self.push_f32(m, s1, pc, n);
        m.op(F64_PROMOTE_F32).op(I64_REINTERPRET_F64);
        self.store_freg_post(m, d);
    }

    /// FCVT.W.S (rtz) — range-guarded exactly like the double form.
    fn fp_cvt_w_s(&self, m: &mut WasmModule, s1: usize, d: usize, pc: u64, n: u32) {
        self.f32_finite_guard(m, s1, pc, n);
        // -2^31 <= f < 2^31 as f32 (the f32 grid has no -2^31-1)
        m.i32_const((-2147483648.0f32).to_bits() as i32)
            .op(F32_REINTERPRET_I32);
        self.push_f32(m, s1, pc, n);
        m.op(F32_LE);
        self.push_f32(m, s1, pc, n);
        m.i32_const(2147483648.0f32.to_bits() as i32)
            .op(F32_REINTERPRET_I32);
        m.op(F32_LT);
        m.op(I32_AND);
        m.op(I32_EQZ);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        if self.store_pre(m, d) {
            self.push_f32(m, s1, pc, n);
            m.op(I32_TRUNC_F32_S).op(I64_EXTEND_I32_S);
            self.store_post(m, d);
        }
    }

    /// FCVT.S.{W,WU,L,LU} — integer to float; inexact results are covered by
    /// the block's sticky-NX gate, and no integer converts to inf or subnormal.
    fn fp_cvt_s_int(&self, m: &mut WasmModule, s1: usize, d: usize, v: u32) {
        self.store_freg_pre(m, d);
        self.push_reg(m, s1);
        match v {
            0 => {
                m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S).op(F32_CONVERT_I64_S);
            }
            1 => {
                m.i64_const(0xffff_ffff).op(I64_AND).op(F32_CONVERT_I64_S);
            }
            2 => {
                m.op(F32_CONVERT_I64_S);
            }
            _ => {
                m.op(F32_CONVERT_I64_U);
            }
        }
        m.op(I32_REINTERPRET_F32)
            .op(I64_EXTEND_I32_U)
            .i64_const(0xffff_ffff_0000_0000u64 as i64)
            .op(I64_OR);
        self.store_freg_post(m, d);
    }

    /// Read FP register f[r] (cached local or memory).
    fn push_freg(&self, m: &mut WasmModule, r: usize) {
        if self.fp_local[r] != 0 {
            m.local_get(self.fp_local[r]);
        } else {
            m.i32_const(0).i64_load(self.lay.f_base as u64 + r as u64 * 8);
        }
    }

    /// Push the memory-store address for f[r] if it isn't cached in a local.
    fn store_freg_pre(&self, m: &mut WasmModule, r: usize) {
        if self.fp_local[r] == 0 {
            m.i32_const(0);
        }
    }

    fn store_freg_post(&self, m: &mut WasmModule, r: usize) {
        if self.fp_local[r] != 0 {
            m.local_set(self.fp_local[r]);
        } else {
            m.i64_store(self.lay.f_base as u64 + r as u64 * 8);
        }
    }

    /// Flush every block-written register local (GPR and FP) back to the CPU
    /// state struct. Precedes every block exit and mid-block bail so the
    /// interpreter (which reads registers from state) sees current values.
    fn flush_writes(&self, m: &mut WasmModule) {
        let mut w = self.write_mask & self.defined.get();
        let gpr_stores = w.count_ones();
        if gpr_stores != 0 {
            self.memprof_inc(m, 13 + Self::reg_width_bucket(gpr_stores));
        }
        while w != 0 {
            let r = w.trailing_zeros() as usize;
            w &= w - 1;
            if self.reg_local[r] != 0 {
                m.i32_const(0)
                    .local_get(self.reg_local[r])
                    .i64_store(self.lay.x_base as u64 + r as u64 * 8);
                if self.lay.reg_stress {
                    m.i32_const(0)
                        .local_get(self.reg_local[r])
                        .i64_store(self.lay.x_base as u64 + r as u64 * 8);
                }
                self.memprof_inc(m, 7);
                Self::regprof_inc(m, self.lay.reg_profile_base, 31 + r - 1);
            }
        }
        let mut w = self.fp_write_mask & self.fp_defined.get();
        while w != 0 {
            let r = w.trailing_zeros() as usize;
            w &= w - 1;
            if self.fp_local[r] != 0 {
                m.i32_const(0)
                    .local_get(self.fp_local[r])
                    .i64_store(self.lay.f_base as u64 + r as u64 * 8);
                self.memprof_inc(m, 8);
            }
        }
    }

    /// Emit a double-precision FP arithmetic op (FADD/FSUB/FMUL/FDIV.D) as an
    /// inline wasm f64 op, guarded to stay bit-exact: the interpreter's fast
    /// path applies only when rm==RNE and the inexact flag (NX) is already
    /// sticky-set, and the result is a normal number (any inf/nan/subnormal/
    /// zero result could raise OF/UF/NV/DZ, so we bail to the interpreter for
    /// exact flags). FP registers stay in memory (f_base); GPR locals are
    /// flushed by bail. `op`: 0=add 1=sub 2=mul 3=div. `dyn_rm`: rm field is
    /// 0b111 (dynamic) so we must also check frm==RNE at runtime.
    /// FP fast-path eligibility: bail unless fcsr.NX is already sticky (host
    /// f64 ops can't report new flag sets exactly) — and, for a dynamic
    /// rounding mode, unless frm == RNE (the only mode wasm f64 implements).
    fn fp_eligibility(&self, m: &mut WasmModule, dyn_rm: bool, pc: u64, n: u32) {
        let fcsr = self.lay.fcsr_addr as u64;
        m.i32_const(0).i64_load(fcsr).i64_const(1).op(I64_AND).op(I64_EQZ);
        if dyn_rm {
            m.i32_const(0)
                .i64_load(fcsr)
                .i64_const(5)
                .op(I64_SHR_U)
                .i64_const(7)
                .op(I64_AND)
                .op(I64_EQZ) // frm==0 ?
                .op(I32_EQZ) // -> frm!=0
                .op(I32_OR);
        }
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
    }

    /// System-mode FP-state guard: bail unless mstatus.FS == Dirty (0b11).
    /// FS=Off must trap (illegal instruction) and Initial/Clean must become
    /// Dirty — one interpreter step does both exactly, and once Dirty the
    /// fast path needs no writeback at all. No-op in user mode.
    fn fp_fs_guard(&self, m: &mut WasmModule, pc: u64, n: u32) {
        if self.lay.mstatus_addr == 0 {
            return;
        }
        m.i32_const(0)
            .i64_load(self.lay.mstatus_addr as u64)
            .i64_const(13)
            .op(I64_SHR_U)
            .i64_const(3)
            .op(I64_AND)
            .i64_const(3)
            .op(I64_NE);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
    }

    /// Bail unless the i64 in VAL, viewed as an f64, is a NORMAL number:
    /// exp in [1, 0x7fe]. Catches inf/nan (0x7ff) and subnormal/zero (0),
    /// whose flag/rounding corner cases the softfloat interpreter must own.
    /// Push "the result in VAL is a zero that one of the operands forced"
    /// (i32 bool): an exactly-zero product or quotient is flag-free only when
    /// it comes from a zero operand — a zero produced by underflow is inexact.
    fn f_zero_from_operand(&self, m: &mut WasmModule, s1: usize, s2: Option<usize>, single: bool) {
        let shift = if single { 33 } else { 1 };
        m.local_get(VAL).i64_const(shift).op(I64_SHL).op(I64_EQZ);
        self.push_freg(m, s1);
        m.i64_const(shift).op(I64_SHL).op(I64_EQZ);
        if let Some(s2) = s2 {
            self.push_freg(m, s2);
            m.i64_const(shift).op(I64_SHL).op(I64_EQZ);
            m.op(I32_OR);
        }
        m.op(I32_AND);
    }

    /// Result eligibility for a double-precision op, matching the
    /// interpreter's fast path exactly (Cpu::fp_fast64):
    ///   add/sub — any non-inf/NaN result, INCLUDING zero and subnormal: every
    ///             double is a multiple of 2^-1074, so an exact sum below the
    ///             normal range is representable and raises no flag.
    ///   mul/div — a normal result, or a zero forced by a zero operand.
    /// The old "must be normal" rule cost nbench FOURIER 32M bails per run on
    /// a single `x + 0.0` inside musl's pow.
    fn fp_result_guard(&self, m: &mut WasmModule, op: u32, s1: usize, s2: usize, pc: u64, n: u32) {
        if op <= 1 {
            m.local_get(VAL)
                .i64_const(52)
                .op(I64_SHR_U)
                .i64_const(0x7ff)
                .op(I64_AND)
                .i64_const(0x7ff)
                .op(I64_EQ);
        } else {
            m.local_get(VAL)
                .i64_const(52)
                .op(I64_SHR_U)
                .i64_const(0x7ff)
                .op(I64_AND)
                .i64_const(1)
                .op(I64_SUB)
                .i64_const(0x7fd)
                .op(I64_GT_U);
            self.f_zero_from_operand(m, s1, (op == 2).then_some(s2), false);
            m.op(I32_EQZ).op(I32_AND);
        }
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
    }

    /// Single-precision twin of fp_result_guard (Cpu::fp_fast32).
    fn fp_result_guard_s(&self, m: &mut WasmModule, op: u32, s1: usize, s2: usize, pc: u64, n: u32) {
        if op <= 1 {
            m.local_get(VAL)
                .i64_const(23)
                .op(I64_SHR_U)
                .i64_const(0xff)
                .op(I64_AND)
                .i64_const(0xff)
                .op(I64_EQ);
        } else {
            m.local_get(VAL)
                .i64_const(23)
                .op(I64_SHR_U)
                .i64_const(0xff)
                .op(I64_AND)
                .i64_const(1)
                .op(I64_SUB)
                .i64_const(0xfd)
                .op(I64_GT_U);
            self.f_zero_from_operand(m, s1, (op == 2).then_some(s2), true);
            m.op(I32_EQZ).op(I32_AND);
        }
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
    }

    fn fp_result_normal_guard(&self, m: &mut WasmModule, pc: u64, n: u32) {
        m.local_get(VAL)
            .i64_const(52)
            .op(I64_SHR_U)
            .i64_const(0x7ff)
            .op(I64_AND)
            .i64_const(1)
            .op(I64_SUB)
            .i64_const(0x7fd)
            .op(I64_GT_U);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
    }

    /// FSQRT.D: wasm f64.sqrt is exactly rounded (RNE), so under the same
    /// eligibility as arith it is bit-exact; negative/inf/zero inputs produce
    /// non-normal results and fall to the result guard.
    fn fp_sqrt_d(&self, m: &mut WasmModule, s1: usize, d: usize, _dyn_rm: bool, pc: u64, n: u32) {
        self.push_freg(m, s1);
        m.op(F64_REINTERPRET_I64).op(F64_SQRT);
        m.op(I64_REINTERPRET_F64).local_set(VAL);
        self.fp_result_normal_guard(m, pc, n);
        self.store_freg_pre(m, d);
        m.local_get(VAL);
        self.store_freg_post(m, d);
    }

    /// FCVT.W.D rtz: truncating double -> signed 32. Range-guarded so
    /// i64.trunc_f64_s cannot trap and NV cases (NaN / out of range, which
    /// riscv clamps + flags) bail to softfloat; NX (non-integral input) is
    /// covered by the sticky-NX eligibility.
    fn fp_cvt_w_d(&self, m: &mut WasmModule, s1: usize, d: usize, pc: u64, n: u32) {
        self.fp_eligibility(m, false, pc, n);
        // in-range: -2^31-1 < f  &&  f < 2^31  (NaN fails both -> bail)
        m.i64_const((-2147483649.0f64).to_bits() as i64)
            .op(F64_REINTERPRET_I64);
        self.push_freg(m, s1);
        m.op(F64_REINTERPRET_I64).op(F64_LT);
        self.push_freg(m, s1);
        m.op(F64_REINTERPRET_I64);
        m.i64_const((2147483648.0f64).to_bits() as i64)
            .op(F64_REINTERPRET_I64)
            .op(F64_LT)
            .op(I32_AND)
            .op(I32_EQZ);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        if self.store_pre(m, d) {
            self.push_freg(m, s1);
            m.op(F64_REINTERPRET_I64)
                .op(I64_TRUNC_F64_S)
                .op(I32_WRAP_I64)
                .op(I64_EXTEND_I32_S);
            self.store_post(m, d);
        }
    }

    /// FCVT.D.{W,WU,L,LU} (`v` = rs2 variant): int -> double. The 32-bit
    /// variants are EXACT (no flags, any rounding mode) and need no guards at
    /// all; the 64-bit ones round (NX for |v| > 2^53) — wasm's converts are
    /// exactly rounded RNE, so sticky-NX eligibility makes them bit-exact.
    fn fp_cvt_d_int(&self, m: &mut WasmModule, s1: usize, d: usize, v: u32, _dyn_rm: bool, _pc: u64, _n: u32) {
        if v >= 2 {
            }
        self.store_freg_pre(m, d);
        self.push_reg(m, s1);
        match v {
            0 => {
                m.op(I32_WRAP_I64)
                    .op(I64_EXTEND_I32_S)
                    .op(F64_CONVERT_I64_S);
            }
            1 => {
                m.i64_const(0xffff_ffff).op(I64_AND).op(F64_CONVERT_I64_S);
            }
            2 => {
                m.op(F64_CONVERT_I64_S);
            }
            _ => {
                m.op(F64_CONVERT_I64_U);
            }
        }
        m.op(I64_REINTERPRET_F64);
        self.store_freg_post(m, d);
    }

    /// FMADD.D family, bit-exact without a host fma: the wasm emission of
    /// fma_fastpath_ref (see its comment for the Dekker/TwoSum/round-to-odd
    /// proof; the fuzz test proves this 1:1 twin against softfp). Bails on:
    /// eligibility, operand/product exponent bands, t+e underflow-to-zero,
    /// non-normal result. Scratch: 8 i64 locals at Ctx::fma_scratch.
    #[allow(clippy::too_many_arguments)]
    fn fp_fmadd_d(&self, m: &mut WasmModule, s1: usize, s2: usize, s3: usize, d: usize,
                  neg_prod: bool, neg_c: bool, _dyn_rm: bool, pc: u64, n: u32) {
        debug_assert!(self.fma_scratch != 0);
        let fs = self.fma_scratch;
        let (fa, fb, fc, fp, f4, f5, f6, f7l) =
            (fs, fs + 1, fs + 2, fs + 3, fs + 4, fs + 5, fs + 6, fs + 7);
        let getf = |m: &mut WasmModule, l: u32| { m.local_get(l).op(F64_REINTERPRET_I64); };
        let setf = |m: &mut WasmModule, l: u32| { m.op(I64_REINTERPRET_F64).local_set(l); };
        let fconst = |m: &mut WasmModule, v: f64| {
            m.i64_const(v.to_bits() as i64).op(F64_REINTERPRET_I64);
        };
        // load operands (bits), applying the variant's sign flips
        self.push_freg(m, s1);
        if neg_prod {
            m.i64_const(i64::MIN).op(I64_XOR);
        }
        m.local_set(fa);
        self.push_freg(m, s2);
        m.local_set(fb);
        self.push_freg(m, s3);
        if neg_c {
            m.i64_const(i64::MIN).op(I64_XOR);
        }
        m.local_set(fc);
        // Exponent band: bail unless ((bits>>52)&0x7ff) - 0x100 <=u 0x5ff, so
        // the Dekker split and its products can't overflow or lose bits — but
        // an operand that is EXACTLY zero is fine and common (a zeroed
        // accumulator, or the `fma(x, y, 0)` a compiler emits for a bare
        // product). Its split is all zeros and the correction term vanishes,
        // leaving the exact IEEE result. Zeros used to bail: 79M times per
        // nbench FOURIER run, each one a wasted block entry plus a softfloat
        // fma in the interpreter.
        // All three operands are checked into ONE branch: a bail sequence is a
        // register flush plus a pc/retired store, so three of them inline is a
        // lot of code for V8 to carry through the hot path of an instruction
        // this dense.
        for (i, &l) in [fa, fb, fc].iter().enumerate() {
            m.local_get(l)
                .i64_const(52)
                .op(I64_SHR_U)
                .i64_const(0x7ff)
                .op(I64_AND)
                .i64_const(0x100)
                .op(I64_SUB)
                .i64_const(0x5ff)
                .op(I64_GT_U);
            m.local_get(l).i64_const(1).op(I64_SHL).op(I64_EQZ).op(I32_EQZ);
            m.op(I32_AND);
            if i > 0 {
                m.op(I32_OR);
            }
        }
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        // Hardware path: one fused f64x2.relaxed_madd replaces the entire
        // Dekker/TwoSum/round-to-odd sequence (~30 f64 ops). Bit-exact by
        // construction — a true fused a*b+c under RNE IS the guest's fmadd —
        // and the host only enables it after proving fusedness empirically.
        // The operand guard above and the result guard below are kept
        // unchanged, so the FLAG behavior is identical: in-band operands can
        // only raise NX (covered by the block's sticky-NX gate); overflow,
        // subnormal and the exotic NV cases bail to softfloat exactly as the
        // emulated path does.
        if hw_fma_enabled() {
            m.local_get(fa).i64x2_splat();
            m.local_get(fb).i64x2_splat();
            m.local_get(fc).i64x2_splat();
            m.f64x2_relaxed_madd().f64x2_extract_lane0();
            m.op(I64_REINTERPRET_F64).local_set(VAL);
            m.local_get(VAL)
                .i64_const(52)
                .op(I64_SHR_U)
                .i64_const(0x7ff)
                .op(I64_AND)
                .i64_const(1)
                .op(I64_SUB)
                .i64_const(0x7fd)
                .op(I64_GT_U);
            m.local_get(VAL).i64_const(1).op(I64_SHL).op(I64_EQZ).op(I32_EQZ);
            m.op(I32_AND);
            m.op(IF).op(VOID);
            self.bail(m, pc, n);
            m.op(END);
            self.store_freg_pre(m, d);
            m.local_get(VAL);
            self.store_freg_post(m, d);
            return;
        }
        // p = a * b, band-checked
        getf(m, fa);
        getf(m, fb);
        m.op(F64_MUL);
        setf(m, fp);
        m.local_get(fp)
            .i64_const(52)
            .op(I64_SHR_U)
            .i64_const(0x7ff)
            .op(I64_AND)
            .i64_const(0x100)
            .op(I64_SUB)
            .i64_const(0x5ff)
            .op(I64_GT_U);
        // ...unless the product is exactly zero, which only a zero operand can
        // produce here (both operands are in-band, so no underflow).
        m.local_get(fp).i64_const(1).op(I64_SHL).op(I64_EQZ).op(I32_EQZ);
        m.op(I32_AND);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        const CSPLIT: f64 = 134217729.0; // 2^27 + 1 (Dekker)
        // ah = a1 - (a1 - a), al = a - ah   (a1 = a*CSPLIT, staged in f4)
        getf(m, fa);
        fconst(m, CSPLIT);
        m.op(F64_MUL);
        setf(m, f4); // a1
        getf(m, f4);
        getf(m, f4);
        getf(m, fa);
        m.op(F64_SUB).op(F64_SUB);
        setf(m, f4); // ah
        getf(m, fa);
        getf(m, f4);
        m.op(F64_SUB);
        setf(m, f5); // al
        // bh (f6), bl (f7l)
        getf(m, fb);
        fconst(m, CSPLIT);
        m.op(F64_MUL);
        setf(m, f6); // b1
        getf(m, f6);
        getf(m, f6);
        getf(m, fb);
        m.op(F64_SUB).op(F64_SUB);
        setf(m, f6); // bh
        getf(m, fb);
        getf(m, f6);
        m.op(F64_SUB);
        setf(m, f7l); // bl
        // e = ((ah*bh - p) + ah*bl + al*bh) + al*bl   -> f4 (ah dead after)
        getf(m, f4);
        getf(m, f6);
        m.op(F64_MUL);
        getf(m, fp);
        m.op(F64_SUB);
        getf(m, f4);
        getf(m, f7l);
        m.op(F64_MUL);
        m.op(F64_ADD);
        getf(m, f5);
        getf(m, f6);
        m.op(F64_MUL);
        m.op(F64_ADD);
        getf(m, f5);
        getf(m, f7l);
        m.op(F64_MUL);
        m.op(F64_ADD);
        setf(m, f4); // e
        // s = p + c -> f5 ; TwoSum tail t -> f6 (z staged in VAL)
        getf(m, fp);
        getf(m, fc);
        m.op(F64_ADD);
        setf(m, f5); // s
        getf(m, f5);
        getf(m, fp);
        m.op(F64_SUB);
        setf(m, VAL); // z
        getf(m, fp);
        getf(m, f5);
        getf(m, VAL);
        m.op(F64_SUB);
        m.op(F64_SUB); // p - (s - z)
        getf(m, fc);
        getf(m, VAL);
        m.op(F64_SUB); // c - z
        m.op(F64_ADD);
        setf(m, f6); // t
        // u = t + e -> f7l ; TwoSum tail d -> fp (p dead; z2 staged in VAL)
        getf(m, f6);
        getf(m, f4);
        m.op(F64_ADD);
        setf(m, f7l); // u
        getf(m, f7l);
        getf(m, f6);
        m.op(F64_SUB);
        setf(m, VAL); // z2
        getf(m, f6);
        getf(m, f7l);
        getf(m, VAL);
        m.op(F64_SUB);
        m.op(F64_SUB); // t - (u - z2)
        getf(m, f4);
        getf(m, VAL);
        m.op(F64_SUB); // e - z2
        m.op(F64_ADD);
        setf(m, fp); // d
        // round-to-odd: if d != 0 { if u == 0 bail; if even(u) nudge toward d }
        getf(m, fp);
        fconst(m, 0.0);
        m.op(F64_NE);
        m.op(IF).op(VOID);
        {
            getf(m, f7l);
            fconst(m, 0.0);
            m.op(F64_EQ);
            m.op(IF).op(VOID);
            self.bail(m, pc, n);
            m.op(END);
            m.local_get(f7l).i64_const(1).op(I64_AND).op(I64_EQZ);
            m.op(IF).op(VOID);
            {
                // u += ((d > 0) != (u < 0)) ? 1 : -1   (bit-domain nudge)
                m.i64_const(1).i64_const(-1);
                getf(m, fp);
                fconst(m, 0.0);
                m.op(F64_GT);
                getf(m, f7l);
                fconst(m, 0.0);
                m.op(F64_LT);
                m.op(I32_XOR);
                m.op(SELECT);
                m.local_get(f7l).op(I64_ADD).local_set(f7l);
            }
            m.op(END);
        }
        m.op(END);
        // r = s + v — except when the correction is exactly zero, where the
        // result IS s: adding +0 to a -0 sum would flip its sign (IEEE says
        // fma(+0, -0, -0) is -0, and -0 + +0 is +0).
        getf(m, f5);
        getf(m, f7l);
        m.op(F64_ADD);
        setf(m, VAL);
        m.local_get(f7l).i64_const(1).op(I64_SHL).op(I64_EQZ);
        m.op(IF).op(VOID);
        m.local_get(f5).local_set(VAL);
        m.op(END);
        // Result guard, with the same zero allowance: every operand and the
        // product were in band or exactly zero, so a zero result is exact
        // cancellation (or a zero product plus a zero addend), never underflow.
        m.local_get(VAL)
            .i64_const(52)
            .op(I64_SHR_U)
            .i64_const(0x7ff)
            .op(I64_AND)
            .i64_const(1)
            .op(I64_SUB)
            .i64_const(0x7fd)
            .op(I64_GT_U);
        m.local_get(VAL).i64_const(1).op(I64_SHL).op(I64_EQZ).op(I32_EQZ);
        m.op(I32_AND);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        self.store_freg_pre(m, d);
        m.local_get(VAL);
        self.store_freg_post(m, d);
    }

    fn fp_arith_d(&self, m: &mut WasmModule, op: u32, s1: usize, s2: usize, d: usize, _dyn_rm: bool, pc: u64, n: u32) {
        // r = f[s1] <op> f[s2]  (as f64), reinterpreted back to i64 bits.
        self.push_freg(m, s1);
        m.op(F64_REINTERPRET_I64);
        self.push_freg(m, s2);
        m.op(F64_REINTERPRET_I64);
        m.op(match op {
            0 => F64_ADD,
            1 => F64_SUB,
            2 => F64_MUL,
            _ => F64_DIV,
        });
        m.op(I64_REINTERPRET_F64).local_set(VAL);
        self.fp_result_guard(m, op, s1, s2, pc, n);
        // f[d] = r
        self.store_freg_pre(m, d);
        m.local_get(VAL);
        self.store_freg_post(m, d);
    }

    /// Emit a double-precision FP compare (FLE/FLT/FEQ.D) as an inline wasm
    /// f64 compare into GPR x[d]. `f3`: 0=FLE 1=FLT 2=FEQ. Bails to the
    /// interpreter if either operand is inf/nan (the exact-flag/NV cases);
    /// finite operands compare exactly with no flag change.
    fn fp_cmp_d(&self, m: &mut WasmModule, f3: u32, s1: usize, s2: usize, d: usize, pc: u64, n: u32) {
        for &s in &[s1, s2] {
            self.push_freg(m, s);
            m.i64_const(52)
                .op(I64_SHR_U)
                .i64_const(0x7ff)
                .op(I64_AND)
                .i64_const(0x7ff)
                .op(I64_EQ);
            m.op(IF).op(VOID);
            self.bail(m, pc, n);
            m.op(END);
        }
        if self.store_pre(m, d) {
            self.push_freg(m, s1);
            m.op(F64_REINTERPRET_I64);
            self.push_freg(m, s2);
            m.op(F64_REINTERPRET_I64);
            m.op(match f3 {
                0 => F64_LE,
                1 => F64_LT,
                _ => F64_EQ,
            });
            m.op(I64_EXTEND_I32_U);
            self.store_post(m, d);
        }
    }

    /// Store the (constant) next pc.
    fn set_pc_const(&self, m: &mut WasmModule, pc: u64) {
        m.i32_const(0)
            .i64_const(pc as i64)
            .i64_store(self.lay.pc_addr as u64);
    }

    /// Guest address (i64) is on the stack. Bounds-check it against guest
    /// RAM and leave the wrapped i32 index on the stack. Traps (wasm
    /// `unreachable`) on out-of-range — a fatal guest fault in user mode.
    fn guest_addr(&self, m: &mut WasmModule, size: u64, len: u64) {
        m.local_set(VA);
        m.local_get(VA).i64_const((size - len) as i64).op(I64_GT_U);
        m.op(IF).op(VOID).op(UNREACHABLE).op(END);
        m.local_get(VA).op(I32_WRAP_I64);
    }

    /// ADD this block's retired count to the retirement cell. The cell is
    /// CUMULATIVE across one host dispatch: the host zeroes it before the
    /// first block of a chain, every block adds what it retired, and
    /// tail-call transfers between blocks leave it accumulating — so the
    /// host reads the whole chain's total no matter how many blocks ran.
    /// (An overwrite contract was briefly tried while chaining looked dead;
    /// a fresh probe on node 20.18.1 measured cross-instance
    /// return_call_indirect at 1.9ns/hop — chaining is back.)
    fn set_retired(&self, m: &mut WasmModule, n: u32) {
        m.i32_const(0);
        m.i32_const(0).i64_load(self.lay.retired_addr as u64);
        m.i64_const(n as i64)
            .op(I64_ADD)
            .i64_store(self.lay.retired_addr as u64);
    }

    /// Bail out of the block at instruction index `n` (retired so far),
    /// leaving pc at `pc` for the interpreter to resume. Inside a compiled
    /// loop the true retired count is the runtime ITER accumulator, not `n`.
    fn bail(&self, m: &mut WasmModule, pc: u64, n: u32) {
        self.flush_writes(m);
        self.set_pc_const(m, pc);
        if let Some(l) = self.retired_local {
            // Exact mid-segment retirement (PERFORMANCE_PROGRESS.md): ITER holds only the
            // segments/bodies flushed so far; `n` is the compile-time count of
            // instructions completed since that flush. Reporting ITER alone
            // undercounted, corrupting insn_count/minstret/clock/fuel.
            // Cumulative-cell contract as in set_retired.
            m.i32_const(0);
            m.i32_const(0).i64_load(self.lay.retired_addr as u64);
            m.local_get(l).op(I64_ADD);
            if n > 0 {
                m.i64_const(n as i64).op(I64_ADD);
            }
            m.i64_store(self.lay.retired_addr as u64);
        } else {
            self.set_retired(m, n);
        }
        m.op(RETURN);
    }

    /// Emit a fused JIT-TLB probe. `addr` (i64 va) must be on the stack. On a
    /// hit, leaves the i32 linear-memory index on the stack and continues; on a
    /// miss (or page-crossing access) sets VA and jumps to `bail`. The fused TLB
    /// entry is pre-filtered (RAM, and for stores writable + not-compiled) and
    /// stores a ready linear offset, so the whole probe is a tag match plus one
    /// add — no RAM range-check or compiled-page check (they moved to the fill).
    fn tlb_index(&self, m: &mut WasmModule, sys: &SysMem, len: u64, store: bool, pc: u64, n: u32) {
        let (tag_base, off_base) = if store {
            (sys.ftlb_store_tag, sys.ftlb_store_off)
        } else {
            (sys.ftlb_load_tag, sys.ftlb_load_off)
        };
        m.local_set(VA);
        // page-crossing guard: an access spanning two pages can't use a single
        // fused entry, so bail and let the interpreter split it.
        if len > 1 {
            m.local_get(VA)
                .i64_const(0xfff)
                .op(I64_AND)
                .i64_const((0x1000 - len) as i64)
                .op(I64_GT_U);
            m.op(IF).op(VOID);
            self.bail(m, pc, n);
            m.op(END);
        }
        // PAGE = va >> 12
        m.local_get(VA).i64_const(12).op(I64_SHR_U).local_set(PAGE);
        // Page coalescing (memory-dense blocks, scratch allocated): the last
        // successfully probed (page -> linear offset) per access class is
        // cached in block locals — repeat accesses to the same page skip the
        // fused-TLB index/tag work entirely. Nothing a block can execute
        // invalidates a va->linear mapping mid-block (no satp/SFENCE/CSR
        // compile; page jit-marking happens at compile time), so a hit is
        // always safe. Locals are initialized to an impossible page (-1) in
        // the prologue.
        let cache = if self.fma_scratch != 0 {
            let base = self.fma_scratch + 8 + if store { 2 } else { 0 };
            Some((base, base + 1)) // (cached page, cached off)
        } else {
            None
        };
        if let Some((cpg, coff)) = cache {
            m.local_get(PAGE).local_get(cpg).op(I64_EQ);
            m.op(IF).op(0x7f); // i32 result: the linear index
            self.memprof_inc(m, 0);
            m.local_get(VA).local_get(coff).op(I64_ADD).op(I32_WRAP_I64);
            m.op(ELSE);
            // slow probe (host-filled on miss); cache (page, off) for later
            self.tlb_idx_tag_fill(m, sys, tag_base, off_base, store, pc, n);
            m.local_get(SCR2).local_set(coff);
            m.local_get(PAGE).local_set(cpg);
            m.local_get(VA).local_get(coff).op(I64_ADD).op(I32_WRAP_I64);
            m.op(END);
            return;
        }
        self.tlb_idx_tag_fill(m, sys, tag_base, off_base, store, pc, n);
        // linear index = (va + off) as i32
        m.local_get(VA).local_get(SCR2).op(I64_ADD).op(I32_WRAP_I64);
    }

    /// Fused-TLB index computation + tag compare. On a miss, ask the host to
    /// walk the page tables and fill the row (a wasm->wasm call, no JS frame)
    /// and carry on with the offset it returns; only an access the host can't
    /// serve inline — unmapped, permission fault, MMIO, or a page holding
    /// compiled code — bails to the interpreter, which re-executes the
    /// instruction and raises the exact architectural fault.
    ///
    /// Before this, every TLB miss inside compiled code bailed. On `tcc -c`,
    /// whose symbol tables and allocations thrash a 4096-entry direct-mapped
    /// TLB, that was ~560k bails and 19M interpreted instructions — 94% JIT
    /// coverage that still spent most of its wall clock in the interpreter.
    ///
    /// Leaves the resolved offset in SCR2 and IDXB holding the entry index.
    fn tlb_idx_tag_fill(
        &self,
        m: &mut WasmModule,
        sys: &SysMem,
        tag_base: u32,
        off_base: u32,
        store: bool,
        pc: u64,
        n: u32,
    ) {
        if !tlb_fill_enabled() {
            self.tlb_idx_tag_check(m, sys, tag_base, pc, n);
            m.local_get_i32(self.idxb)
                .i64_load_at(off_base as u64)
                .local_set(SCR2);
            return;
        }
        m.use_tlb_fill();
        // IDXB (i32) = ((page & mask) << 3)
        m.local_get(PAGE)
            .op(I32_WRAP_I64)
            .i32_const(sys.tlb_mask as i32)
            .op(I32_AND)
            .i32_const(3)
            .op(I32_SHL)
            .local_set_i32(self.idxb);
        m.local_get_i32(self.idxb).i64_load_at(tag_base as u64);
        m.local_get(PAGE).op(I64_NE);
        m.op(IF).op(VOID);
        self.memprof_inc(m, 2);
        // miss: off = tlb_fill(va, store)
        m.local_get(VA).i32_const(store as i32);
        m.call_tlb_fill();
        m.local_set(SCR2);
        m.local_get(SCR2).i64_const(-1).op(I64_EQ);
        m.op(IF).op(VOID);
        self.memprof_inc(m, 4);
        self.bail(m, pc, n);
        m.op(END);
        m.op(ELSE);
        self.memprof_inc(m, 1);
        m.local_get_i32(self.idxb).i64_load_at(off_base as u64);
        m.local_set(SCR2);
        m.op(END);
    }

    /// Fused-TLB index computation + tag compare; bails on miss. Leaves
    /// IDXB holding the entry index for the caller's off-load.
    fn tlb_idx_tag_check(
        &self,
        m: &mut WasmModule,
        sys: &SysMem,
        tag_base: u32,
        pc: u64,
        n: u32,
    ) {
        // IDXB (i32) = ((page & mask) << 3)
        m.local_get(PAGE)
            .op(I32_WRAP_I64)
            .i32_const(sys.tlb_mask as i32)
            .op(I32_AND)
            .i32_const(3)
            .op(I32_SHL)
            .local_set_i32(self.idxb);
        // miss if ftlb_tag[idx] != page -> bail
        m.local_get_i32(self.idxb).i64_load_at(tag_base as u64);
        m.local_get(PAGE).op(I64_NE);
        m.op(IF).op(VOID);
        self.memprof_inc(m, 4);
        self.bail(m, pc, n);
        m.op(END);
        self.memprof_inc(m, 1);
    }
}

/// Pre-scan a block — walking and terminating exactly like `translate_block`
/// — to collect which guest registers it reads and writes, as 32-bit bitmaps.
/// Used to decide which registers to cache in wasm locals.
/// Returns (gpr_read, gpr_write, fp_read, fp_write) register bitmaps.
fn scan_regs(
    code: &[u8],
    base: u64,
    start_pc: u64,
    lay: &JitLayout,
    hot: &dyn Fn(u64) -> bool,
    next: &dyn Fn(u64) -> Option<u64>,
) -> (u32, u32, u32, u32, u32) {
    let (mut read, mut write) = (0u32, 0u32);
    // Uses per register: hoisting one into a wasm local costs a load in the
    // prologue and a store in the epilogue, paid on EVERY dispatch of the
    // block. A register a short block touches once or twice is cheaper left in
    // the register file, which the emitter reads and writes directly.
    let mut uses = [0u32; 32];
    let mut mem_ops = 0u32;
    let (mut fread, mut fwrite) = (0u32, 0u32);
    let mut pc = start_pc;
    let mut n = 0u32;
    // FP registers: f0 is a real register (no hardwired-zero), so mark it too.
    let fmark = |m: &mut u32, r: usize| *m |= 1 << r;
    let mut mark = |m: &mut u32, r: usize| {
        if r != 0 {
            *m |= 1 << r;
            uses[r] += 1;
        }
    };
    // Linear-trace fact tracking — MUST mirror translate_block exactly
    // (see TraceFacts): constant registers and stack slots let the walk
    // follow calls and (guarded) returns along the same path the emitter
    // will take.
    let mut tf = TraceFacts::new();
    let tl = trace_level();
    let mut seg_entry = start_pc;
    while n < MAX_TRACE as u32 {
        let Some((insn, ilen)) = fetch(code, base, pc) else {
            break;
        };
        let next_pc = pc.wrapping_add(ilen);
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
        let s1_known = tf.known[s1];
        match opcode(insn) {
            0x37 | 0x17 => mark(&mut write, d),
            0x13 => {
                mark(&mut read, s1);
                mark(&mut write, d);
            }
            0x33 => {
                if !alu_handled(0x33, funct7(insn), funct3(insn)) {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut read, s2);
                mark(&mut write, d);
            }
            0x1b => {
                if !matches!(funct3(insn), 0 | 1 | 5) {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut write, d);
            }
            0x3b => {
                if !alu_handled(0x3b, funct7(insn), funct3(insn)) {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut read, s2);
                mark(&mut write, d);
            }
            0x03 if lay.mem.is_some() || lay.sys.is_some() => {
                if funct3(insn) == 7 {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut write, d);
                mem_ops += 1;
            }
            0x2f if lay.sys.is_some() => {
                if !amo_handled(insn) {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut read, s2);
                mark(&mut write, d);
                mem_ops += 1;
            }
            0x23 if lay.mem.is_some() || lay.sys.is_some() => {
                if funct3(insn) > 3 {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut read, s2);
                mem_ops += 1;
            }
            // FLD/FSD (funct3 3, raw 8-byte copy) and FLW/FSW (funct3 2, the
            // low half with NaN-boxing) between memory and f[], user-mode
            // direct or system inline-TLB (needs f_base for the FP file).
            0x07 if (lay.mem.is_some() || lay.sys.is_some()) && lay.f_base != 0 => {
                if !matches!(funct3(insn), 2 | 3) {
                    break;
                }
                mark(&mut read, s1);
                fmark(&mut fwrite, d);
            }
            0x27 if (lay.mem.is_some() || lay.sys.is_some()) && lay.f_base != 0 => {
                if !matches!(funct3(insn), 2 | 3) {
                    break;
                }
                mark(&mut read, s1);
                fmark(&mut fread, s2);
            }
            0x6f => {
                mark(&mut write, d);
                let target = pc.wrapping_add(imm_j(insn) as u64);
                let bounded = target >= base && target < base + code.len() as u64;
                // Follow forward jumps, and calls in EITHER direction (the
                // link register is the constant next_pc): caller and callee
                // merge into one trace. Backward plain jumps are loop
                // back-edges — the loop machinery owns those. Must mirror
                // translate_block exactly.
                let follow = if d == 0 {
                    target > pc && bounded
                } else {
                    tl >= 2 && target != pc && bounded
                };
                if follow {
                    tf.step(insn, pc);
                    if d != 0 {
                        tf.known[d] = Known::Proven(next_pc);
                    }
                    pc = target;
                    n += 1;
                    continue;
                }
                break;
            }
            0x67 => {
                if funct3(insn) != 0 {
                    break;
                }
                if d == 0 && tl >= 3 {
                    let target = match s1_known {
                        Known::Proven(v) | Known::Predicted(v) => {
                            Some(v.wrapping_add(imm_i(insn) as u64) & !1)
                        }
                        Known::No => None,
                    };
                    if let Some(target) = target {
                        if target != pc && target >= base && target < base + code.len() as u64 {
                            if let Known::Predicted(_) = s1_known {
                                mark(&mut read, s1); // the guard reads it
                            }
                            tf.step(insn, pc);
                            pc = target;
                            n += 1;
                            continue;
                        }
                    }
                }
                mark(&mut read, s1);
                mark(&mut write, d);
                // Inline cache (mirror translate_block exactly): an indirect
                // jump whose observed target is in-window keeps the trace
                // going under a one-compare guard.
                if tl >= 3 {
                    if let Some(t) = next(seg_entry) {
                        if t != pc && t >= base && t < base + code.len() as u64 {
                            seg_entry = t;
                            if d != 0 {
                                tf.step(insn, pc);
                                tf.known[d] = Known::Proven(next_pc);
                            } else {
                                tf.step(insn, pc);
                            }
                            pc = t;
                            n += 1;
                            continue;
                        }
                    }
                }
                break;
            }
            0x63 => {
                if !matches!(funct3(insn), 0 | 1 | 4 | 5 | 6 | 7) {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut read, s2);
                if tl == 0 {
                    break;
                }
                // Direction decision must mirror translate_block exactly.
                let target = pc.wrapping_add(imm_b(insn) as u64);
                let t_in = target >= base && target < base + code.len() as u64;
                pc = if t_in && target > pc && hot(target) && !hot(next_pc) {
                    target
                } else {
                    next_pc
                };
                n += 1;
                continue;
            }
            // OP-FP (mirror translate_block): FP arith touches no GPRs;
            // FMV.D.X reads a GPR, FMV.X.D writes one; others end the block.
            0x53 if lay.f_base != 0 => {
                if !fp_handled(insn) {
                    break;
                }
                let f7 = funct7(insn);
                match (f7 >> 2, f7 & 3, funct3(insn)) {
                    (0..=3, 1, 0 | 7) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        fmark(&mut fwrite, d);
                    }
                    (4, 1, 0..=2) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        fmark(&mut fwrite, d);
                    }
                    (0x14, 1, 0..=2) => {
                        // FLE/FLT/FEQ: read FP s1,s2 -> write GPR x[d]
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        mark(&mut write, d);
                    }
                    (0x1e, 1, 0) => {
                        mark(&mut read, s1); // FMV.D.X: x[s1] -> f[d]
                        fmark(&mut fwrite, d);
                    }
                    (0x1c, 1, 0) => {
                        fmark(&mut fread, s1); // FMV.X.D: f[s1] -> x[d]
                        mark(&mut write, d);
                    }
                    (0x0b, 1, 0 | 7) => {
                        fmark(&mut fread, s1); // FSQRT.D
                        fmark(&mut fwrite, d);
                    }
                    (0x18, 1, 1) => {
                        fmark(&mut fread, s1); // FCVT.W.D rtz: f -> GPR
                        mark(&mut write, d);
                    }
                    (0x1a, 1, 0 | 7) => {
                        mark(&mut read, s1); // FCVT.D.int: GPR -> f
                        fmark(&mut fwrite, d);
                    }
                    (0..=3, 0, 0 | 7) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        fmark(&mut fwrite, d);
                    }
                    (4, 0, 0..=2) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        fmark(&mut fwrite, d);
                    }
                    (0x14, 0, 0..=2) => {
                        // FLE/FLT/FEQ: read FP s1,s2 -> write GPR x[d]
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        mark(&mut write, d);
                    }
                    (0x1e, 0, 0) => {
                        mark(&mut read, s1); // FMV.D.X: x[s1] -> f[d]
                        fmark(&mut fwrite, d);
                    }
                    (0x1c, 0, 0) => {
                        fmark(&mut fread, s1); // FMV.X.D: f[s1] -> x[d]
                        mark(&mut write, d);
                    }
                    (0x0b, 0, 0 | 7) => {
                        fmark(&mut fread, s1); // FSQRT.D
                        fmark(&mut fwrite, d);
                    }
                    (8, 0, 0 | 7) => {
                        fmark(&mut fread, s1); // FSQRT.D
                        fmark(&mut fwrite, d);
                    }
                    (8, 1, 0 | 7) => {
                        fmark(&mut fread, s1); // FSQRT.D
                        fmark(&mut fwrite, d);
                    }
                    (0x18, 0, 1) => {
                        fmark(&mut fread, s1); // FCVT.W.D rtz: f -> GPR
                        mark(&mut write, d);
                    }
                    (0x1a, 0, 0 | 7) => {
                        mark(&mut read, s1); // FCVT.D.int: GPR -> f
                        fmark(&mut fwrite, d);
                    }
                    _ => break,
                }
            }
            op @ (0x43 | 0x47 | 0x4b | 0x4f) if lay.f_base != 0 => {
                if !fma_handled(op, insn) {
                    break;
                }
                fmark(&mut fread, s1);
                fmark(&mut fread, s2);
                fmark(&mut fread, ((insn >> 27) & 31) as usize);
                fmark(&mut fwrite, d);
                read |= 1; // bit 0 = "block contains fma" (see build_ctx)
            }
            _ => break,
        }
        tf.step(insn, pc);
        pc = next_pc;
        n += 1;
    }
    let _ = uses; // per-block hoist filtering measured neutral: a block's
    // prologue is not where its dispatch cost lives (see BLOCK_HOIST_MIN).
    if mem_ops >= 3 {
        read |= 1; // bit 0: allocate scratch (memory page-cache; see build_ctx)
    }
    (read, write, fread, fwrite, n)
}

/// Is this OP-FP (0x53) instruction one the JIT emits inline?
///
/// THE single authority for FP, like alu_handled for integer ops: every
/// scanner and the emitter must agree or block boundaries desync. Takes the
/// whole insn because FCVT variants are selected by the rs2 FIELD.
/// Covered: D arith (FADD/FSUB/FMUL/FDIV), compares, FMV both ways, FSQRT.D,
/// FCVT.W.D (rtz), FCVT.D.{W,WU,L,LU}, FSGNJ/FSGNJN/FSGNJX.D — which is how
/// every libm spells fabs, fneg, copysign and register-to-register moves (76%
/// of everything nbench FOURIER dropped to the interpreter).
fn fp_handled(insn: u32) -> bool {
    let f7 = funct7(insn);
    let f3 = funct3(insn);
    match (f7 >> 2, f7 & 3, f3) {
        (0..=3, 1, 0 | 7) => true,        // FADD/FSUB/FMUL/FDIV.D (rne | dyn)
        (4, 1, 0..=2) => true,            // FSGNJ/FSGNJN/FSGNJX.D (bit ops)
        // --- F extension (single precision, NaN-boxed in the low 32 bits) ---
        (0..=3, 0, 0 | 7) => true,        // FADD/FSUB/FMUL/FDIV.S
        (4, 0, 0..=2) => true,            // FSGNJ/FSGNJN/FSGNJX.S
        (0x14, 0, 0..=2) => true,         // FLE/FLT/FEQ.S
        (0x0b, 0, 0 | 7) => rs2(insn) == 0, // FSQRT.S
        (0x1e, 0, 0) => rs2(insn) == 0,   // FMV.W.X
        (0x1c, 0, 0) => rs2(insn) == 0,   // FMV.X.W
        (8, 0, 0 | 7) => rs2(insn) == 1,  // FCVT.S.D
        (8, 1, 0 | 7) => rs2(insn) == 0,  // FCVT.D.S
        (0x18, 0, 1) => rs2(insn) == 0,   // FCVT.W.S (rtz)
        (0x1a, 0, 0 | 7) => rs2(insn) <= 3, // FCVT.S.{W,WU,L,LU}
        (0x14, 1, 0..=2) => true,         // FLE/FLT/FEQ.D
        (0x1e, 1, 0) => rs2(insn) == 0,   // FMV.D.X (rs2 is a fixed field)
        (0x1c, 1, 0) => rs2(insn) == 0,   // FMV.X.D (rs2 is a fixed field)
        (0x0b, 1, 0 | 7) => rs2(insn) == 0, // FSQRT.D (rs2 is a fixed field)
        (0x18, 1, 1) => rs2(insn) == 0,   // FCVT.W.D rtz only (signed 32)
        (0x1a, 1, 0 | 7) => rs2(insn) <= 3, // FCVT.D.{W,WU,L,LU} (rne | dyn)
        _ => false,
    }
}

/// Is this an atomic memory operation the JIT emits inline? Single-hart, so
/// AMO* is just load / modify / store through the same inline TLB as any other
/// access — but LR/SC are NOT compiled (they carry reservation state the
/// interpreter owns). aq/rl ordering bits are irrelevant on one hart.
fn amo_handled(insn: u32) -> bool {
    matches!(funct3(insn), 2 | 3)
        && matches!(insn >> 27, 0 | 1 | 4 | 8 | 0xc | 0x10 | 0x14 | 0x18 | 0x1c)
}

/// Is this FMADD-family (0x43/0x47/0x4b/0x4f) instruction one the JIT emits
/// inline? Double precision (fmt bits [26:25] == 1), rm RNE or dynamic.
/// Same single-authority contract as fp_handled/alu_handled.
fn fma_handled(op: u32, insn: u32) -> bool {
    matches!(op, 0x43 | 0x47 | 0x4b | 0x4f)
        && (insn >> 25) & 3 == 1
        && matches!(funct3(insn), 0 | 7)
}

/// Is `f7`/`f3` a supported OP / OP-32 / OP-IMM-32 encoding?
///
/// THE single authority on which ALU encodings compile: every walker
/// (scan_regs, loop_region, scan_regs_super) and emit_simple must consult
/// this — if a scanner and the emitter ever disagree on where a block ends,
/// register allocation desyncs from emission (historically a boot hang).
/// Missing from the M extension: MULH/MULHSU/MULHU (0x33, 0x01, 1..=3) —
/// wasm has no 64x64->high-64 multiply; emulating it costs ~20 ops.
fn alu_handled(op: u32, f7: u32, f3: u32) -> bool {
    match op {
        0x37 | 0x17 => true,
        // OP-IMM: shift encodings have RESERVED upper immediate bits — SLLI
        // funct6 must be 000000 (f7 in {0,1}: bit 0 is shamt[5]), SRLI/SRAI
        // 000000/010000. Reserved patterns must NOT compile (the interpreter
        // owns the illegal-instruction trap; see PERFORMANCE_PROGRESS.md).
        0x13 => match f3 {
            1 => matches!(f7, 0x00 | 0x01),
            5 => matches!(f7, 0x00 | 0x01 | 0x20 | 0x21),
            _ => true,
        },
        0x33 => matches!(
            (f7, f3),
            (0x00, _) | (0x20, 0) | (0x20, 5) | (0x01, 0) | (0x01, 4..=7)
        ),
        // OP-IMM-32 shifts: shamt is 5 bits — imm[5] (f7 bit 0) is reserved.
        0x1b => match f3 {
            0 => true,
            1 => f7 == 0x00,
            5 => matches!(f7, 0x00 | 0x20),
            _ => false,
        },
        0x3b => matches!(
            (f7, f3),
            (0x00, 0)
                | (0x20, 0)
                | (0x01, 0)
                | (0x00, 1) // SLLW
                | (0x00, 5) // SRLW
                | (0x20, 5) // SRAW
                | (0x01, 4..=7) // DIVW/DIVUW/REMW/REMUW
        ),
        _ => false,
    }
}

/// A compilable loop region: guest code `[start_pc, end_pc)` containing
/// properly-nested natural loops plus forward if-then / loop-exit branches.
/// `loops` is (header_pc, exit_pc) per loop; `start_pc` is the outermost
/// loop's header. Compiled into nested wasm `block`+`loop` pairs (3e-2,
/// generalising 3d-2's single straight-line self-loop) so every register local
/// persists across all iterations of all levels with no per-iteration dispatch.
struct LoopRegion {
    end_pc: u64,
    loops: Vec<(u64, u64)>,
    unconditional_latch: bool,
}

/// Detect and fully validate a structured loop region at `start_pc` (which must
/// be a natural-loop header — the target of a backward branch). User-mode only:
/// inline memory ops here only TRAP on fault (never bail mid-loop), and the FP
/// register file is present. Returns None for anything not provably structured
/// (the caller then compiles a plain basic block).
fn loop_region(code: &[u8], base: u64, start_pc: u64, lay: &JitLayout) -> Option<LoopRegion> {
    if lay.multi_latch {
        if let Some(region) = loop_region_mode(code, base, start_pc, lay, true) {
            return Some(region);
        }
    }
    loop_region_mode(code, base, start_pc, lay, false)
}

fn loop_region_mode(
    code: &[u8],
    base: u64,
    start_pc: u64,
    lay: &JitLayout,
    extend: bool,
) -> Option<LoopRegion> {
    // Compile loops for user-mode (flat memory) or system-mode (inline TLB).
    // System memory ops can bail mid-iteration; the compiled loop handles that
    // (flush locals, set pc, report ITER-retired, return) — see translate_loop.
    if lay.mem.is_none() && lay.sys.is_none() {
        return None;
    }
    // Pass A: linear walk to the back-edge that closes the outermost loop,
    // collecting every conditional branch. Every instruction must be handled.
    let mut branches: Vec<(u64, u64, u64)> = Vec::new(); // (pc, target, next)
    let mut end_pc = None;
    let mut ld_count = 0u32;
    let mut lhu_count = 0u32;
    let mut other_mem = 0u32;
    let mut unconditional_latch = false;
    let mut pc = start_pc;
    let mut n = 0u32;
    while n < MAX_BLOCK as u32 {
        let (insn, ilen) = fetch(code, base, pc)?;
        let op = opcode(insn);
        let next = pc.wrapping_add(ilen);
        match op {
            0x37 | 0x17 | 0x13 | 0x33 | 0x1b | 0x3b => {
                if !alu_handled(op, funct7(insn), funct3(insn)) {
                    return None;
                }
            }
            0x53 if lay.f_base != 0 => {
                if !fp_handled(insn) {
                    return None;
                }
            }
            0x43 | 0x47 | 0x4b | 0x4f if lay.f_base != 0 => {
                if !fma_handled(op, insn) {
                    return None;
                }
            }
            0x03 => {
                if funct3(insn) == 7 {
                    return None;
                }
                if extend {
                    match funct3(insn) {
                        3 => ld_count += 1,
                        5 => lhu_count += 1,
                        _ => other_mem += 1,
                    }
                }
            }
            0x23 => {
                if funct3(insn) > 3 {
                    return None;
                }
                if extend {
                    other_mem += 1;
                }
            }
            0x2f if lay.sys.is_some() => {
                if !amo_handled(insn) {
                    return None;
                }
            }
            0x07 | 0x27 if lay.f_base != 0 => {
                if !matches!(funct3(insn), 2 | 3) {
                    return None;
                }
                if extend {
                    other_mem += 1;
                }
            }
            0x63 => {
                if !matches!(funct3(insn), 0 | 1 | 4 | 5 | 6 | 7) {
                    return None;
                }
                let t = pc.wrapping_add(imm_b(insn) as u64);
                branches.push((pc, t, next));
                if t == start_pc {
                    if !extend {
                        end_pc = Some(next);
                        break;
                    }
                }
            }
            0x6f if extend => {
                let t = pc.wrapping_add(imm_j(insn) as u64);
                let continues = branches.iter().filter(|&&(_, bt, _)| bt == start_pc).count();
                if rd(insn) != 0
                    || t != start_pc
                    || continues < 2
                    || ld_count != 1
                    || lhu_count != 1
                    || other_mem != 0
                {
                    return None;
                }
                end_pc = Some(next);
                unconditional_latch = true;
                break;
            }
            _ => return None, // calls / jumps / system / AMO / single-FP end it
        }
        pc = next;
        n += 1;
    }
    let end_pc = end_pc?;
    // Pass B: derive loops from backward branches (target < pc); a header's
    // exit is the instruction after the last back-edge that targets it.
    let mut loops: Vec<(u64, u64)> = Vec::new();
    for &(bpc, t, bnext) in &branches {
        // A backward branch that leaves the region ([target < start_pc]) is
        // an EXIT, not a back-edge: rotated bottom-tested nests place the
        // outer increment BEFORE the inner header, so the inner loop's exit
        // jumps backward out (nbench ASSIGNMENT's scan nests — one host
        // dispatch per 11-insn iteration while this shape was rejected).
        // translate_loop emits it as a conditional bail with exact counts.
        if t < bpc && (t >= start_pc || !rotated_nests()) {
            if let Some(e) = loops.iter_mut().find(|(h, _)| *h == t) {
                if bnext > e.1 {
                    e.1 = bnext;
                }
            } else {
                loops.push((t, bnext));
            }
        }
    }
    loops.sort_by_key(|&(h, _)| h);
    if unconditional_latch {
        if let Some(e) = loops.iter_mut().find(|(h, _)| *h == start_pc) {
            e.1 = end_pc;
        }
    }
    // Reject duplicate headers and improperly-overlapping loop ranges.
    for i in 0..loops.len() {
        let (hi, ei) = loops[i];
        if hi < start_pc || ei > end_pc {
            return None;
        }
        for j in (i + 1)..loops.len() {
            let (hj, ej) = loops[j];
            if hj == hi {
                return None;
            }
            // sorted so hi < hj: allow proper nesting (ej<=ei) or disjoint (ei<=hj).
            if !(ei <= hj || ej <= ei) {
                return None;
            }
        }
    }
    // Validate every forward branch is a structured break or if-then.
    for &(bpc, t, _) in &branches {
        if t <= bpc {
            continue; // back-edges validated above
        }
        if t > end_pc {
            return None;
        }
        // break: target equals the exit of an enclosing loop.
        if loops.iter().any(|&(h, e)| h <= bpc && bpc < e && e == t) {
            continue;
        }
        // if-then: target within the innermost enclosing loop, and not jumping
        // into the middle of a nested loop.
        let bound = loops
            .iter()
            .filter(|&&(h, e)| h <= bpc && bpc < e)
            .map(|&(_, e)| e)
            .min()
            .unwrap_or(end_pc);
        if t > bound {
            return None;
        }
        if loops.iter().any(|&(h, e)| bpc < h && h < t && t < e) {
            return None;
        }
    }
    if loops.is_empty() {
        return None;
    }
    Some(LoopRegion { end_pc, loops, unconditional_latch })
}

/// Register scan over a whole loop region `[start_pc, end_pc)` (linear; every
/// instruction is already validated as handled). Returns the same four masks
/// as `scan_regs`: (gpr_read, gpr_write, fp_read, fp_write).
fn scan_regs_region(
    code: &[u8],
    base: u64,
    start_pc: u64,
    end_pc: u64,
    _lay: &JitLayout,
) -> (u32, u32, u32, u32) {
    let (mut read, mut write, mut fread, mut fwrite) = (0u32, 0u32, 0u32, 0u32);
    let mut mem_ops = 0u32;
    let fmark = |m: &mut u32, r: usize| *m |= 1 << r;
    let mark = |m: &mut u32, r: usize| {
        if r != 0 {
            *m |= 1 << r;
        }
    };
    let mut pc = start_pc;
    while pc < end_pc {
        let Some((insn, ilen)) = fetch(code, base, pc) else {
            break;
        };
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
        match opcode(insn) {
            0x37 | 0x17 => mark(&mut write, d),
            0x13 | 0x1b => {
                mark(&mut read, s1);
                mark(&mut write, d);
            }
            0x33 | 0x3b => {
                mark(&mut read, s1);
                mark(&mut read, s2);
                mark(&mut write, d);
            }
            0x03 => {
                mark(&mut read, s1);
                mark(&mut write, d);
                mem_ops += 1;
            }
            0x23 => {
                mark(&mut read, s1);
                mark(&mut read, s2);
                mem_ops += 1;
            }
            0x2f if _lay.sys.is_some() => {
                mark(&mut read, s1);
                mark(&mut read, s2);
                mem_ops += 1;
            }
            0x07 => {
                mark(&mut read, s1);
                fmark(&mut fwrite, d);
                mem_ops += 1;
            }
            0x27 => {
                mark(&mut read, s1);
                fmark(&mut fread, s2);
            }
            0x53 => {
                let f7 = funct7(insn);
                match (f7 >> 2, f7 & 3, funct3(insn)) {
                    (0..=3, 1, 0 | 7) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        fmark(&mut fwrite, d);
                    }
                    (4, 1, 0..=2) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        fmark(&mut fwrite, d);
                    }
                    (0x14, 1, 0..=2) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        mark(&mut write, d);
                    }
                    (0x1e, 1, 0) => {
                        mark(&mut read, s1);
                        fmark(&mut fwrite, d);
                    }
                    (0x1c, 1, 0) => {
                        fmark(&mut fread, s1);
                        mark(&mut write, d);
                    }
                    (0x0b, 1, 0 | 7) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fwrite, d);
                    }
                    (0x18, 1, 1) => {
                        fmark(&mut fread, s1);
                        mark(&mut write, d);
                    }
                    (0x1a, 1, 0 | 7) => {
                        mark(&mut read, s1);
                        fmark(&mut fwrite, d);
                    }
                    (0..=3, 0, 0 | 7) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        fmark(&mut fwrite, d);
                    }
                    (4, 0, 0..=2) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        fmark(&mut fwrite, d);
                    }
                    (0x14, 0, 0..=2) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fread, s2);
                        mark(&mut write, d);
                    }
                    (0x1e, 0, 0) => {
                        mark(&mut read, s1);
                        fmark(&mut fwrite, d);
                    }
                    (0x1c, 0, 0) => {
                        fmark(&mut fread, s1);
                        mark(&mut write, d);
                    }
                    (0x0b, 0, 0 | 7) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fwrite, d);
                    }
                    (8, 0, 0 | 7) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fwrite, d);
                    }
                    (8, 1, 0 | 7) => {
                        fmark(&mut fread, s1);
                        fmark(&mut fwrite, d);
                    }
                    (0x18, 0, 1) => {
                        fmark(&mut fread, s1);
                        mark(&mut write, d);
                    }
                    (0x1a, 0, 0 | 7) => {
                        mark(&mut read, s1);
                        fmark(&mut fwrite, d);
                    }
                    _ => {}
                }
            }
            0x43 | 0x47 | 0x4b | 0x4f => {
                fmark(&mut fread, s1);
                fmark(&mut fread, s2);
                fmark(&mut fread, ((insn >> 27) & 31) as usize);
                fmark(&mut fwrite, d);
                read |= 1; // bit 0 = "block contains fma" (see build_ctx)
            }
            0x63 => {
                mark(&mut read, s1);
                mark(&mut read, s2);
            }
            _ => {}
        }
        pc = pc.wrapping_add(ilen);
    }
    if mem_ops >= 3 {
        read |= 1; // bit 0: allocate scratch (memory page-cache; see build_ctx)
    }
    (read, write, fread, fwrite)
}

/// Assign wasm locals for the touched GPR/FP registers, build the module, and
/// emit the prologue that loads each touched register from state into its
/// local. Shared by the basic-block and structured-loop compilers.
fn build_ctx(
    lay: JitLayout,
    read_mask: u32,
    write_mask: u32,
    fp_read: u32,
    fp_write: u32,
) -> (Ctx, WasmModule) {
    build_ctx_load(lay, read_mask, write_mask, fp_read, fp_write, None)
}

/// build_ctx with an explicit PROLOGUE LOAD set: `Some((gpr, fp))` loads
/// only those registers (the rest start undefined and are tracked by
/// Ctx::defined), `None` loads everything touched (the conservative form
/// loops and superblocks require).
fn build_ctx_load(
    lay: JitLayout,
    read_mask: u32,
    write_mask: u32,
    fp_read: u32,
    fp_write: u32,
    load: Option<(u32, u32)>,
) -> (Ctx, WasmModule) {
    // read_mask bit 0 (x0 — never a real register) smuggles the "block
    // contains FMADD-family" flag from the scanners; strip it BEFORE any mask
    // use (a set bit 0 would make the prologue clobber local 0, the machine
    // pointer parameter).
    let want_fma = read_mask & 1 != 0;
    // write_mask bit 0 (x0 is never written) asks for the hoisted FP gate flags.
    let want_flags = write_mask & 1 != 0;
    let read_mask = read_mask & !1;
    let write_mask = write_mask & !1;
    let touched = read_mask | write_mask;
    let fp_touched = fp_read | fp_write;
    let mut reg_local = [0u32; 32];
    let mut n_reg = 0u32;
    for r in 1..32 {
        if touched & (1 << r) != 0 {
            reg_local[r] = N_I64_LOCALS + 1 + n_reg;
            n_reg += 1;
        }
    }
    let mut fp_local = [0u32; 32];
    let mut n_fp = 0u32;
    for r in 0..32 {
        if fp_touched & (1 << r) != 0 {
            fp_local[r] = N_I64_LOCALS + 1 + n_reg + n_fp;
            n_fp += 1;
        }
    }
    let n_fma = if want_fma { 12 } else { 0 };
    let n_flags = if want_flags { 2 } else { 0 };
    let c = Ctx {
        lay,
        reg_local,
        write_mask,
        // i32 local after all i64 locals (incl. the fma scratch block)
        idxb: N_I64_LOCALS + n_reg + n_fp + n_fma + n_flags + 1,
        fp_local,
        fp_write_mask: fp_write,
        fma_scratch: if want_fma { N_I64_LOCALS + 1 + n_reg + n_fp } else { 0 },
        retired_local: None,
        defined: std::cell::Cell::new(match load {
            Some((g, _)) => g & !1,
            None => u32::MAX,
        }),
        fp_defined: std::cell::Cell::new(match load {
            Some((_, f)) => f,
            None => u32::MAX,
        }),
        fp_flags: if want_flags {
            let b = N_I64_LOCALS + 1 + n_reg + n_fp + n_fma;
            Some((b, b + 1))
        } else {
            None
        },
    };
    // Two i32 locals: IDXB (TLB/dispatch index math) and IDXB+1 (the chain
    // stub's function-table index).
    let mut m = WasmModule::with_locals(N_I64_LOCALS + n_reg + n_fp + n_fma + n_flags, 2);
    let (load_g, load_f) = match load {
        Some((g, f)) => (g & touched & !1, f & fp_touched),
        None => (touched, fp_touched),
    };
    let mut t = load_g;
    if t != 0 {
        c.memprof_inc(&mut m, 9 + Ctx::reg_width_bucket(t.count_ones()));
    }
    while t != 0 {
        let r = t.trailing_zeros() as usize;
        t &= t - 1;
            m.i32_const(0)
                .i64_load(lay.x_base as u64 + r as u64 * 8)
                .local_set(reg_local[r]);
            if lay.reg_stress {
                m.i32_const(0)
                    .i64_load(lay.x_base as u64 + r as u64 * 8)
                    .local_set(reg_local[r]);
            }
            Ctx::regprof_inc(&mut m, lay.reg_profile_base, r - 1);
            if let Some(addrs) = lay.mem_profile {
                let addr = addrs[5] as u64;
                if addr != 0 {
                m.i32_const(0);
                m.i32_const(0).i64_load(addr).i64_const(1).op(I64_ADD);
                m.i64_store(addr);
                }
            }
    }
    let mut t = load_f;
    while t != 0 {
        let r = t.trailing_zeros() as usize;
        t &= t - 1;
            m.i32_const(0)
                .i64_load(lay.f_base as u64 + r as u64 * 8)
                .local_set(fp_local[r]);
            if let Some(addrs) = lay.mem_profile {
                let addr = addrs[6] as u64;
                if addr != 0 {
                m.i32_const(0);
                m.i32_const(0).i64_load(addr).i64_const(1).op(I64_ADD);
                m.i64_store(addr);
                }
            }
    }
    if want_fma {
        // memory page-cache locals ([8]=load pg, [10]=store pg) must start
        // at an impossible page: locals zero-init and page 0 is a real page.
        let base = N_I64_LOCALS + 1 + n_reg + n_fp;
        m.i64_const(-1).local_set(base + 8);
        m.i64_const(-1).local_set(base + 10);
    }
    (c, m)
}

/// Emit the hoisted per-BLOCK FP gate: mstatus.FS == Dirty (system mode),
/// fcsr.NX already sticky, and frm == RNE. None of these can change inside
/// a compiled block — CSR writes never compile, and the covered FP ops only
/// SET exception flags (never clear) — so ONE check at entry covers every
/// FP instruction the block contains, replacing the per-instruction
/// eligibility/FS checks (5-9 wasm ops each) on FP-dense code. Bails with
/// pc = block start and zero retired: nothing has executed, the interpreter
/// replays from the top, and the first FP instruction performs the
/// architectural transition (FS trap/Dirty, NX set) exactly as before.
/// Conservative for blocks whose only FP ops are static-RNE while guest
/// frm != RNE (they run interpreted until frm returns) — frm changes are
/// rare and transient in real code.
/// Does this instruction's result depend on the rounding mode / set inexact?
/// FLD/FSD/FSGNJ/FMV move bits around: they need mstatus.FS to be Dirty (they
/// touch the FP file) but nothing from fcsr. Gating those on sticky-NX would
/// wedge a block that never produces an inexact result.
fn fp_needs_round(insn: u32) -> bool {
    match opcode(insn) {
        0x43 | 0x47 | 0x4b | 0x4f => true,
        0x53 => {
            let f7 = funct7(insn);
            !matches!(f7 >> 2, 4 | 0x1c | 0x1e) // FSGNJ, FMV.X.*, FMV.*.X
        }
        _ => false, // FLD/FSD/FLW/FSW
    }
}

/// Evaluate the FP gate's two conditions into the hoisted flag locals.
/// BASE = fuel granted for this call = FUEL_CELL - RETIRED_CELL (clamped at
/// zero). The subtraction matters under tail-call chaining: earlier blocks of
/// the chain already consumed part of the grant, and their count sits in the
/// cumulative retirement cell. Without chaining the cell is zero at entry and
/// BASE == FUEL_CELL exactly as before.
fn emit_fuel_base(c: &Ctx, m: &mut WasmModule) {
    if c.lay.fuel_addr == 0 {
        m.i64_const(LOOP_CAP as i64).local_set(BASE);
        return;
    }
    m.i32_const(0).i64_load(c.lay.fuel_addr as u64);
    m.i32_const(0).i64_load(c.lay.retired_addr as u64);
    m.op(I64_SUB).local_set(BASE);
    m.local_get(BASE).i64_const(0).op(I64_LT_S);
    m.op(IF).op(VOID);
    m.i64_const(0).local_set(BASE);
    m.op(END);
}

/// Chain exit: after a block has stored its successor pc and ADDED its
/// retired count, try to transfer STRAIGHT to the next compiled block with a
/// tail call instead of returning to the host dispatch loop. The checks are
/// exactly the host fast path's — line.pc match, non-blacklisted index,
/// map generation, fuel — read from the same memory the host reads, so a
/// transfer happens only where the host would have dispatched anyway. Any
/// failed check falls back to a plain return (the host handles it). Blocks
/// that bailed must NOT chain (the interpreter owns the next instruction);
/// `iter_guard` additionally refuses to transfer when this call retired
/// nothing (a fuel-exhausted or off-entry exit) — a zero-progress transfer
/// to the same pc would tail-loop forever.
/// Transfer to the next compiled block through the HOST module's
/// chain_next export (imported as env.chain_next, exactly like tlb_fill):
/// the dispatch-line fast path runs in one Rust function and the transfer
/// is an indirect call through the table the host owns — generated modules
/// never import the table, so registration stays O(1) per block (importing
/// it made table.set O(importing instances): the V8 quadratic that killed
/// emitted return_call_indirect chaining for large populations).
/// `guard_progress`: refuse the transfer when ITER (the runtime retired
/// accumulator) is zero — a zero-progress transfer to the same pc would
/// recurse to the depth cap for nothing.
fn emit_chain_next(c: &Ctx, m: &mut WasmModule, guard_progress: bool) {
    let lay = &c.lay;
    if !chain_enabled() || lay.sys.is_none() || lay.dispatch_base == 0 || lay.fuel_addr == 0 {
        return;
    }
    if guard_progress {
        m.local_get(ITER).op(I64_EQZ);
        m.op(IF).op(VOID).op(RETURN).op(END);
    }
    m.use_chain_next();
    m.local_get(0); // machine-state pointer parameter
    m.call_chain_next();
}

/// Build the shared chain-check helper body for `lay` (WasmModule::set_helper):
/// kill-cell, dispatch-line probe (pc match, blacklist, map generation),
/// fuel — the host fast path's exact conditions — returning the verified,
/// SB_IDX_BIT-stripped table index, or -1 to fall back to the host. One copy
/// per MODULE instead of ~80 bytes inlined at every trace side exit, which
/// bloated large block populations past V8's tiering appetite (tcc: 2.4x
/// slower with the inline form emitted, even unexecuted).
/// Helper locals (no params): 0 = i64 cpc, 1 = i32 line offset, 2 = i32 idx.
fn build_chain_helper(lay: &JitLayout) -> Vec<u8> {
    let mut h = WasmModule::with_locals(0, 0);
    if lay.chain_off_addr != 0 {
        h.i32_const(0).i32_load(lay.chain_off_addr as u64);
        h.op(IF).op(VOID).i32_const(-1).op(RETURN).op(END);
    }
    h.i32_const(0).i64_load(lay.pc_addr as u64).local_set(0);
    h.local_get(0)
        .i64_const(1)
        .op(I64_SHR_U)
        .op(I32_WRAP_I64)
        .i32_const(lay.dispatch_mask as i32)
        .op(I32_AND)
        .i32_const(4)
        .op(I32_SHL)
        .local_set(1);
    h.local_get(1).i64_load_at(lay.dispatch_base as u64);
    h.local_get(0).op(I64_NE);
    h.op(IF).op(VOID).i32_const(-1).op(RETURN).op(END);
    h.local_get(1).i32_load(lay.dispatch_base as u64 + 8).local_set(2);
    h.local_get(2).i32_const(0).op(I32_LT_S);
    h.op(IF).op(VOID).i32_const(-1).op(RETURN).op(END);
    h.local_get(2).i32_const(!SB_IDX_BIT).op(I32_AND).local_set(2);
    h.local_get(1).i32_load(lay.dispatch_base as u64 + 12);
    h.i32_const(0).i32_load(lay.map_gen_addr as u64);
    h.op(I32_NE);
    h.op(IF).op(VOID).i32_const(-1).op(RETURN).op(END);
    h.i32_const(0).i64_load(lay.retired_addr as u64);
    h.i32_const(0).i64_load(lay.fuel_addr as u64);
    h.op(I64_GE_U);
    h.op(IF).op(VOID).i32_const(-1).op(RETURN).op(END);
    h.local_get(2);
    h.into_code()
}

/// Trace-exit chain transfer through the shared helper (~15 bytes/site).
/// CURRENTLY UNCALLED: any module that imports the shared function table
/// makes every subsequent table.set O(importing instances) in V8 (per-
/// instance call_indirect dispatch caches), so thousands of chain-bearing
/// trace modules turn block registration quadratic — tcc measured 2.4-3x
/// slower with chain code merely EMITTED. Loop/region blocks (few) still
/// chain. Next design: a second, chain-only table, or routing transfers
/// through a host-module chain_run export so traces never import the
/// table. Kept (with build_chain_helper) for that work.
#[allow(dead_code)]
fn emit_chain_exit_helper(c: &Ctx, m: &mut WasmModule) {
    let lay = &c.lay;
    if !chain_enabled()
        || lay.sys.is_none()
        || lay.dispatch_base == 0
        || lay.fuel_addr == 0
        || lay.map_gen_addr == 0
    {
        return;
    }
    m.use_tlb_fill();
    m.use_table();
    let hidx = m.set_helper(build_chain_helper(lay));
    let t = c.idxb + 1;
    m.call(hidx).local_set_i32(t);
    m.local_get_i32(t).i32_const(0).op(I32_LT_S);
    m.op(IF).op(VOID).op(RETURN).op(END);
    m.local_get(0); // machine-state pointer parameter
    m.local_get_i32(t);
    m.return_call_indirect(0);
}

/// Emitted ONLY at loop-region / copy-loop / region-function exits: those
/// blocks re-dispatch to a small cyclic successor set, so the tail-call
/// site stays monomorphic and costs ~2ns (probe, node 20.18.1). Trace
/// blocks deliberately do NOT chain — tcc's 7.5k-block soup made every
/// site megamorphic and ran 2.9x slower chained than host-dispatched.
fn emit_chain_exit(c: &Ctx, m: &mut WasmModule, iter_guard: bool) {
    let lay = &c.lay;
    if !chain_enabled()
        || lay.sys.is_none()
        || lay.dispatch_base == 0
        || lay.fuel_addr == 0
    {
        return;
    }
    // Live kill switch: one i32 load per chain attempt.
    if lay.chain_off_addr != 0 {
        m.i32_const(0).i32_load(lay.chain_off_addr as u64);
        m.op(IF).op(VOID).op(RETURN).op(END);
    }
    if iter_guard {
        m.local_get(ITER).op(I64_EQZ);
        m.op(IF).op(VOID).op(RETURN).op(END);
    }
    m.use_table();
    let idx2 = c.idxb + 1;
    // CPC = the successor pc the block just stored.
    m.i32_const(0).i64_load(lay.pc_addr as u64).local_set(CPC);
    // line byte offset = ((pc >> 1) & mask) << 4
    m.local_get(CPC)
        .i64_const(1)
        .op(I64_SHR_U)
        .op(I32_WRAP_I64)
        .i32_const(lay.dispatch_mask as i32)
        .op(I32_AND)
        .i32_const(4)
        .op(I32_SHL)
        .local_set_i32(c.idxb);
    // line.pc == pc?
    m.local_get_i32(c.idxb).i64_load_at(lay.dispatch_base as u64);
    m.local_get(CPC).op(I64_NE);
    m.op(IF).op(VOID).op(RETURN).op(END);
    // idx >= 0 (blacklist sentinels carry -1)?
    m.local_get_i32(c.idxb)
        .i32_load(lay.dispatch_base as u64 + 8)
        .local_set_i32(idx2);
    m.local_get_i32(idx2).i32_const(0).op(I32_LT_S);
    m.op(IF).op(VOID).op(RETURN).op(END);
    // Strip the host's region marker before using the index as a table slot.
    m.local_get_i32(idx2)
        .i32_const(!SB_IDX_BIT)
        .op(I32_AND)
        .local_set_i32(idx2);
    // line verified under the current address-space generation?
    m.local_get_i32(c.idxb).i32_load(lay.dispatch_base as u64 + 12);
    m.i32_const(0).i32_load(lay.map_gen_addr as u64);
    m.op(I32_NE);
    m.op(IF).op(VOID).op(RETURN).op(END);
    // fuel left in this grant?
    m.i32_const(0).i64_load(lay.retired_addr as u64);
    m.i32_const(0).i64_load(lay.fuel_addr as u64);
    m.op(I64_GE_U);
    m.op(IF).op(VOID).op(RETURN).op(END);
    m.local_get(0); // the machine-state pointer parameter, passed along
    m.local_get_i32(idx2);
    m.return_call_indirect(0);
}

fn emit_fp_flags(c: &Ctx, m: &mut WasmModule) {
    let Some((fs_bad, round_bad)) = c.fp_flags else {
        return;
    };
    let fcsr = c.lay.fcsr_addr as u64;
    // round_bad = NX not sticky || frm != RNE
    m.i32_const(0).i64_load(fcsr).i64_const(1).op(I64_AND).op(I64_EQZ);
    m.i32_const(0)
        .i64_load(fcsr)
        .i64_const(5)
        .op(I64_SHR_U)
        .i64_const(7)
        .op(I64_AND)
        .op(I64_EQZ)
        .op(I32_EQZ);
    m.op(I32_OR).op(I64_EXTEND_I32_U).local_set(round_bad);
    // fs_bad = mstatus.FS != Dirty (system mode only)
    if c.lay.mstatus_addr != 0 {
        m.i32_const(0)
            .i64_load(c.lay.mstatus_addr as u64)
            .i64_const(13)
            .op(I64_SHR_U)
            .i64_const(3)
            .op(I64_AND)
            .i64_const(3)
            .op(I64_NE)
            .op(I64_EXTEND_I32_U)
            .local_set(fs_bad);
    } else {
        m.i64_const(0).local_set(fs_bad);
    }
}

fn emit_block_fp_gate(c: &Ctx, m: &mut WasmModule, start_pc: u64, n: u32, round: bool) {
    // With hoisted flags (superblocks), a body's gate is a flag test.
    if let Some((fs_bad, round_bad)) = c.fp_flags {
        m.local_get(fs_bad);
        if round {
            m.local_get(round_bad).op(I64_OR);
        }
        m.op(I64_EQZ).op(I32_EQZ);
        m.op(IF).op(VOID);
        c.bail(m, start_pc, n);
        m.op(END);
        return;
    }
    let fcsr = c.lay.fcsr_addr as u64;
    if round {
        // bad = (fcsr & 1) == 0  ||  ((fcsr >> 5) & 7) != 0
        m.i32_const(0).i64_load(fcsr).i64_const(1).op(I64_AND).op(I64_EQZ);
        m.i32_const(0)
            .i64_load(fcsr)
            .i64_const(5)
            .op(I64_SHR_U)
            .i64_const(7)
            .op(I64_AND)
            .op(I64_EQZ)
            .op(I32_EQZ);
        m.op(I32_OR);
    }
    if c.lay.mstatus_addr != 0 {
        m.i32_const(0)
            .i64_load(c.lay.mstatus_addr as u64)
            .i64_const(13)
            .op(I64_SHR_U)
            .i64_const(3)
            .op(I64_AND)
            .i64_const(3)
            .op(I64_NE);
        if round {
            m.op(I32_OR);
        }
    } else if !round {
        return; // nothing to check (user mode, non-rounding FP only)
    }
    m.op(IF).op(VOID);
    c.bail(m, start_pc, n);
    m.op(END);
}

/// Scan one straight-line body from `pc` for FP work: (touches the FP file,
/// depends on rounding). Used to gate ONLY the bodies that need it — a
/// superblock covering a whole page mixes integer functions with float ones,
/// and one function-wide gate makes every entry into the integer code bail
/// (measured: nbench IDEA fell 5434 -> 1709 iter/s the moment a float routine
/// landed on its page).
fn body_fp_kind(
    code: &[u8],
    base: u64,
    mut pc: u64,
    page_end: u64,
    lay: &JitLayout,
    stop_at_branch: bool,
) -> (bool, bool) {
    let (mut any, mut round) = (false, false);
    let mut n = 0u32;
    while n < 4 * MAX_BLOCK as u32 && pc < page_end {
        let Some((insn, ilen)) = fetch(code, base, pc) else {
            break;
        };
        let op = opcode(insn);
        match op {
            0x63 | 0x6f | 0x67 => {
                if stop_at_branch {
                    break;
                }
            }
            0x07 | 0x27 if lay.f_base != 0 && matches!(funct3(insn), 2 | 3) => any = true,
            0x53 if lay.f_base != 0 && fp_handled(insn) => {
                any = true;
                round |= fp_needs_round(insn);
            }
            0x43 | 0x47 | 0x4b | 0x4f if lay.f_base != 0 && fma_handled(op, insn) => {
                any = true;
                round = true;
            }
            0x37 | 0x17 | 0x13 | 0x33 | 0x1b | 0x3b
                if alu_handled(op, funct7(insn), funct3(insn)) => {}
            0x03 if funct3(insn) != 7 => {}
            0x23 if funct3(insn) <= 3 => {}
            0x2f if amo_handled(insn) => {}
            _ => {
                if stop_at_branch {
                    break;
                }
            }
        }
        pc = pc.wrapping_add(ilen);
        n += 1;
    }
    (any, round)
}

/// Emit one non-control-flow guest instruction (LUI/AUIPC, OP-IMM(-32),
/// OP(-32), load/store, FLD/FSD, FP arith/compare/FMV). Returns false — before
/// emitting anything — if `insn` is a branch/jump or an unsupported encoding;
/// the caller then ends the block / loop region. `n` is the retired index used
/// only for mid-block bail points (system TLB miss, FP fast-path bail).
fn emit_simple(m: &mut WasmModule, c: &Ctx, lay: JitLayout, insn: u32, pc: u64, n: u32) -> bool {
    let op = opcode(insn);
    let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
    match op {
        // LUI / AUIPC: constants at translation time.
        0x37 | 0x17 => {
            if c.store_pre(m, d) {
                let v = if op == 0x37 {
                    imm_u(insn) as u64
                } else {
                    pc.wrapping_add(imm_u(insn) as u64)
                };
                m.i64_const(v as i64);
                c.store_post(m, d);
            }
        }
        // OP-IMM
        0x13 => {
            let imm = imm_i(insn);
            let f3 = funct3(insn);
            if c.store_pre(m, d) {
                c.push_reg(m, s1);
                match f3 {
                    0 => {
                        m.i64_const(imm).op(I64_ADD);
                    }
                    1 => {
                        m.i64_const(imm & 0x3f).op(I64_SHL);
                    }
                    2 => {
                        m.i64_const(imm).op(I64_LT_S).op(I64_EXTEND_I32_U);
                    }
                    3 => {
                        m.i64_const(imm).op(I64_LT_U).op(I64_EXTEND_I32_U);
                    }
                    4 => {
                        m.i64_const(imm).op(I64_XOR);
                    }
                    5 => {
                        if insn >> 26 == 0x10 {
                            m.i64_const(imm & 0x3f).op(I64_SHR_S);
                        } else {
                            m.i64_const(imm & 0x3f).op(I64_SHR_U);
                        }
                    }
                    6 => {
                        m.i64_const(imm).op(I64_OR);
                    }
                    _ => {
                        m.i64_const(imm).op(I64_AND);
                    }
                }
                c.store_post(m, d);
            }
        }
        // OP (I, M mul + div/rem; MULH* falls back)
        0x33 => {
            let f7 = funct7(insn);
            let f3 = funct3(insn);
            if !alu_handled(0x33, f7, f3) {
                return false;
            }
            if c.store_pre(m, d) {
                c.push_reg(m, s1);
                match (f7, f3) {
                    (0x00, 0) => {
                        c.push_reg(m, s2);
                        m.op(I64_ADD);
                    }
                    (0x20, 0) => {
                        c.push_reg(m, s2);
                        m.op(I64_SUB);
                    }
                    (0x01, 0) => {
                        c.push_reg(m, s2);
                        m.op(I64_MUL);
                    }
                    (0x00, 1) => {
                        c.push_reg(m, s2);
                        m.i64_const(0x3f).op(I64_AND).op(I64_SHL);
                    }
                    (0x00, 2) => {
                        c.push_reg(m, s2);
                        m.op(I64_LT_S).op(I64_EXTEND_I32_U);
                    }
                    (0x00, 3) => {
                        c.push_reg(m, s2);
                        m.op(I64_LT_U).op(I64_EXTEND_I32_U);
                    }
                    (0x00, 4) => {
                        c.push_reg(m, s2);
                        m.op(I64_XOR);
                    }
                    (0x00, 5) => {
                        c.push_reg(m, s2);
                        m.i64_const(0x3f).op(I64_AND).op(I64_SHR_U);
                    }
                    (0x20, 5) => {
                        c.push_reg(m, s2);
                        m.i64_const(0x3f).op(I64_AND).op(I64_SHR_S);
                    }
                    (0x00, 6) => {
                        c.push_reg(m, s2);
                        m.op(I64_OR);
                    }
                    (0x00, 7) => {
                        c.push_reg(m, s2);
                        m.op(I64_AND);
                    }
                    // DIV/DIVU/REM/REMU: wasm div/rem TRAP on zero divisor (and
                    // div_s on MIN/-1) where riscv defines results, so divide by
                    // a select-guarded safe divisor and select the architected
                    // result afterwards. Straight-line (select, no control flow).
                    // Stack on entry to each arm: [rs1] (the dividend).
                    (0x01, 4) => {
                        // safe = (rs2==0 || (rs1==MIN && rs2==-1)) ? 1 : rs2
                        m.i64_const(1);
                        c.push_reg(m, s2);
                        c.push_reg(m, s2);
                        m.op(I64_EQZ);
                        c.push_reg(m, s1);
                        m.i64_const(i64::MIN).op(I64_EQ);
                        c.push_reg(m, s2);
                        m.i64_const(-1).op(I64_EQ).op(I32_AND).op(I32_OR).op(SELECT);
                        m.op(I64_DIV_S);
                        // overflow (MIN/-1) -> MIN
                        m.i64_const(i64::MIN);
                        c.push_reg(m, s1);
                        m.i64_const(i64::MIN).op(I64_EQ);
                        c.push_reg(m, s2);
                        m.i64_const(-1).op(I64_EQ).op(I32_AND).op(I32_EQZ).op(SELECT);
                        // zero divisor -> -1
                        m.i64_const(-1);
                        c.push_reg(m, s2);
                        m.op(I64_EQZ).op(I32_EQZ).op(SELECT);
                    }
                    (0x01, 5) => {
                        m.i64_const(1);
                        c.push_reg(m, s2);
                        c.push_reg(m, s2);
                        m.op(I64_EQZ).op(SELECT);
                        m.op(I64_DIV_U);
                        m.i64_const(-1); // zero divisor -> all ones
                        c.push_reg(m, s2);
                        m.op(I64_EQZ).op(I32_EQZ).op(SELECT);
                    }
                    (0x01, 6 | 7) => {
                        // wasm rem_s(MIN,-1) is defined as 0 = riscv REM, so
                        // only the zero divisor needs guarding: result is rs1.
                        m.i64_const(1);
                        c.push_reg(m, s2);
                        c.push_reg(m, s2);
                        m.op(I64_EQZ).op(SELECT);
                        m.op(if f3 == 6 { I64_REM_S } else { I64_REM_U });
                        c.push_reg(m, s1);
                        c.push_reg(m, s2);
                        m.op(I64_EQZ).op(I32_EQZ).op(SELECT);
                    }
                    _ => unreachable!(),
                }
                c.store_post(m, d);
            }
        }
        // OP-IMM-32 (ADDIW/SLLIW/SRLIW/SRAIW): compute in 64, wrap+extend.
        0x1b => {
            let imm = imm_i(insn);
            let f3 = funct3(insn);
            if !matches!(f3, 0 | 1 | 5) {
                return false;
            }
            if c.store_pre(m, d) {
                c.push_reg(m, s1);
                match f3 {
                    0 => {
                        m.i64_const(imm).op(I64_ADD);
                    }
                    1 => {
                        m.i64_const(imm & 0x1f).op(I64_SHL);
                    }
                    _ => {
                        m.op(I32_WRAP_I64).op(I64_EXTEND_I32_U);
                        if funct7(insn) == 0x20 {
                            m.op(I32_WRAP_I64)
                                .op(I64_EXTEND_I32_S)
                                .i64_const(imm & 0x1f)
                                .op(I64_SHR_S);
                        } else {
                            m.i64_const(0xffff_ffff)
                                .op(I64_AND)
                                .i64_const(imm & 0x1f)
                                .op(I64_SHR_U);
                        }
                    }
                }
                m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                c.store_post(m, d);
            }
        }
        // OP-32 (ADDW/SUBW/MULW, W-shifts, DIVW/DIVUW/REMW/REMUW)
        0x3b => {
            let (f7, f3) = (funct7(insn), funct3(insn));
            if !alu_handled(0x3b, f7, f3) {
                return false;
            }
            if c.store_pre(m, d) {
                // Operand pushers: signed = sext32(x[r]), unsigned = low 32
                // zero-extended. (Recomputed per use — 3 ops from a local.)
                let push_s = |m: &mut WasmModule, c: &Ctx, r: usize| {
                    c.push_reg(m, r);
                    m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                };
                let push_u = |m: &mut WasmModule, c: &Ctx, r: usize| {
                    c.push_reg(m, r);
                    m.i64_const(0xffff_ffff).op(I64_AND);
                };
                const MIN32: i64 = i32::MIN as i64;
                match (f7, f3) {
                    (0x00, 0) | (0x20, 0) | (0x01, 0) => {
                        c.push_reg(m, s1);
                        c.push_reg(m, s2);
                        m.op(match (f7, f3) {
                            (0x00, 0) => I64_ADD,
                            (0x20, 0) => I64_SUB,
                            _ => I64_MUL,
                        });
                    }
                    (0x00, 1) => {
                        // SLLW: shift in 64, final wrap+sext truncates.
                        c.push_reg(m, s1);
                        c.push_reg(m, s2);
                        m.i64_const(0x1f).op(I64_AND).op(I64_SHL);
                    }
                    (0x00, 5) => {
                        // SRLW: logical shift of the low 32 bits.
                        push_u(m, c, s1);
                        c.push_reg(m, s2);
                        m.i64_const(0x1f).op(I64_AND).op(I64_SHR_U);
                    }
                    (0x20, 5) => {
                        // SRAW: arithmetic shift of sext32(rs1).
                        push_s(m, c, s1);
                        c.push_reg(m, s2);
                        m.i64_const(0x1f).op(I64_AND).op(I64_SHR_S);
                    }
                    // 32-bit div/rem: same select-guard scheme as the 64-bit
                    // forms (see 0x33), on sext32/zext32 operands. The final
                    // shared wrap+sext below narrows every result (including
                    // the -1 / MIN32 / rs1 fallbacks) to riscv's sext32.
                    (0x01, 4) => {
                        push_s(m, c, s1);
                        m.i64_const(1);
                        push_s(m, c, s2);
                        push_s(m, c, s2);
                        m.op(I64_EQZ);
                        push_s(m, c, s1);
                        m.i64_const(MIN32).op(I64_EQ);
                        push_s(m, c, s2);
                        m.i64_const(-1).op(I64_EQ).op(I32_AND).op(I32_OR).op(SELECT);
                        m.op(I64_DIV_S);
                        m.i64_const(MIN32);
                        push_s(m, c, s1);
                        m.i64_const(MIN32).op(I64_EQ);
                        push_s(m, c, s2);
                        m.i64_const(-1).op(I64_EQ).op(I32_AND).op(I32_EQZ).op(SELECT);
                        m.i64_const(-1);
                        push_s(m, c, s2);
                        m.op(I64_EQZ).op(I32_EQZ).op(SELECT);
                    }
                    (0x01, 5) => {
                        push_u(m, c, s1);
                        m.i64_const(1);
                        push_u(m, c, s2);
                        push_u(m, c, s2);
                        m.op(I64_EQZ).op(SELECT);
                        m.op(I64_DIV_U);
                        m.i64_const(-1);
                        push_u(m, c, s2);
                        m.op(I64_EQZ).op(I32_EQZ).op(SELECT);
                    }
                    (0x01, 6) => {
                        push_s(m, c, s1);
                        m.i64_const(1);
                        push_s(m, c, s2);
                        push_s(m, c, s2);
                        m.op(I64_EQZ).op(SELECT);
                        m.op(I64_REM_S);
                        push_s(m, c, s1);
                        push_s(m, c, s2);
                        m.op(I64_EQZ).op(I32_EQZ).op(SELECT);
                    }
                    (0x01, 7) => {
                        push_u(m, c, s1);
                        m.i64_const(1);
                        push_u(m, c, s2);
                        push_u(m, c, s2);
                        m.op(I64_EQZ).op(SELECT);
                        m.op(I64_REM_U);
                        push_u(m, c, s1);
                        push_u(m, c, s2);
                        m.op(I64_EQZ).op(I32_EQZ).op(SELECT);
                    }
                    _ => unreachable!(),
                }
                m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                c.store_post(m, d);
            }
        }
        // LOAD (user-mode direct, or system inline-TLB)
        0x03 if lay.mem.is_some() || lay.sys.is_some() => {
            let f3 = funct3(insn);
            let len = match f3 {
                0 | 4 => 1,
                1 | 5 => 2,
                2 | 6 => 4,
                3 => 8,
                _ => return false,
            };
            let load_op = match f3 {
                0 => I64_LOAD8_S,
                1 => I64_LOAD16_S,
                2 => I64_LOAD32_S,
                3 => I64_LOAD,
                4 => I64_LOAD8_U,
                5 => I64_LOAD16_U,
                _ => I64_LOAD32_U,
            };
            c.push_reg(m, s1);
            m.i64_const(imm_i(insn)).op(I64_ADD);
            let mem_off = if let Some((mem_base, size)) = lay.mem {
                c.guest_addr(m, size, len); // i32 index, traps OOB
                mem_base as u64
            } else {
                c.tlb_index(m, &lay.sys.unwrap(), len, false, pc, n);
                0
            };
            m.op(load_op).raw_uleb(len_align(len)).raw_uleb(mem_off);
            if d == 0 {
                m.op(DROP);
            } else {
                m.local_set(VAL);
                c.store_pre(m, d);
                m.local_get(VAL);
                c.store_post(m, d);
            }
        }
        // STORE (user-mode direct, or system inline-TLB)
        0x23 if lay.mem.is_some() || lay.sys.is_some() => {
            let f3 = funct3(insn);
            if f3 > 3 {
                return false;
            }
            let len = 1u64 << f3;
            let store_op = match f3 {
                0 => I64_STORE8,
                1 => I64_STORE16,
                2 => I64_STORE32,
                _ => I64_STORE,
            };
            c.push_reg(m, s1);
            m.i64_const(imm_s(insn)).op(I64_ADD);
            if let Some((mem_base, size)) = lay.mem {
                c.guest_addr(m, size, len);
                c.push_reg(m, s2);
                m.op(store_op)
                    .raw_uleb(len_align(len))
                    .raw_uleb(mem_base as u64);
            } else {
                c.tlb_index(m, &lay.sys.unwrap(), len, true, pc, n);
                c.push_reg(m, s2);
                m.op(store_op).raw_uleb(len_align(len)).raw_uleb(0);
            }
        }
        // FLD: f[d] = mem[x[s1]+imm] (double). Raw 8-byte copy, bit-exact.
        // User-mode direct access or system inline-TLB.
        0x07 if (lay.mem.is_some() || lay.sys.is_some()) && lay.f_base != 0 => {
            let f3 = funct3(insn);
            if f3 != 3 && f3 != 2 {
                return false;
            }
            let len = if f3 == 3 { 8 } else { 4 };
            c.push_reg(m, s1);
            m.i64_const(imm_i(insn)).op(I64_ADD);
            let off = if let Some((mem_base, size)) = lay.mem {
                c.guest_addr(m, size, len);
                mem_base as u64
            } else {
                c.tlb_index(m, &lay.sys.unwrap(), len, false, pc, n);
                0
            };
            // FLW NaN-boxes into the high half; FLD is a raw 8-byte copy.
            m.op(if f3 == 3 { I64_LOAD } else { I64_LOAD32_U })
                .raw_uleb(len_align(len))
                .raw_uleb(off);
            if f3 == 2 {
                m.i64_const(0xffff_ffff_0000_0000u64 as i64).op(I64_OR);
            }
            m.local_set(VAL);
            c.store_freg_pre(m, d);
            m.local_get(VAL);
            c.store_freg_post(m, d);
        }
        // FSD: mem[x[s1]+imm] = f[s2] (double). Raw 8-byte copy.
        0x27 if (lay.mem.is_some() || lay.sys.is_some()) && lay.f_base != 0 => {
            let f3 = funct3(insn);
            if f3 != 3 && f3 != 2 {
                return false;
            }
            let len = if f3 == 3 { 8 } else { 4 };
            // FSW writes the low 32 bits (the boxed single) untouched.
            let st = if f3 == 3 { I64_STORE } else { I64_STORE32 };
            c.push_reg(m, s1);
            m.i64_const(imm_s(insn)).op(I64_ADD);
            if let Some((mem_base, size)) = lay.mem {
                c.guest_addr(m, size, len);
                c.push_freg(m, s2);
                m.op(st).raw_uleb(len_align(len)).raw_uleb(mem_base as u64);
            } else {
                c.tlb_index(m, &lay.sys.unwrap(), len, true, pc, n);
                c.push_freg(m, s2);
                m.op(st).raw_uleb(len_align(len)).raw_uleb(0);
            }
        }
        // AMO (single hart): read-modify-write through the inline store TLB.
        0x2f if lay.sys.is_some() => {
            if !amo_handled(insn) {
                return false;
            }
            let len = if funct3(insn) == 3 { 8 } else { 4 };
            let f5 = insn >> 27;
            c.push_reg(m, s1);
            c.tlb_index(m, &lay.sys.unwrap(), len, true, pc, n);
            m.op(I64_EXTEND_I32_U).local_set(PA);
            // Natural alignment is architectural for AMO; a misaligned one
            // faults, so leave it to the interpreter.
            m.local_get(VA)
                .i64_const((len - 1) as i64)
                .op(I64_AND)
                .op(I64_EQZ)
                .op(I32_EQZ);
            m.op(IF).op(VOID);
            c.bail(m, pc, n);
            m.op(END);
            // old = mem[addr] (sign-extended for .W, as rd receives it)
            m.local_get(PA).op(I32_WRAP_I64);
            m.op(if len == 8 { I64_LOAD } else { I64_LOAD32_S })
                .raw_uleb(len_align(len))
                .raw_uleb(0);
            m.local_set(VAL);
            // mem[addr] = op(old, x[rs2])
            m.local_get(PA).op(I32_WRAP_I64);
            let push_rs2 = |m: &mut WasmModule| c.push_reg(m, s2);
            match f5 {
                0 => {
                    m.local_get(VAL);
                    push_rs2(m);
                    m.op(I64_ADD);
                }
                1 => push_rs2(m),
                4 => {
                    m.local_get(VAL);
                    push_rs2(m);
                    m.op(I64_XOR);
                }
                8 => {
                    m.local_get(VAL);
                    push_rs2(m);
                    m.op(I64_OR);
                }
                0xc => {
                    m.local_get(VAL);
                    push_rs2(m);
                    m.op(I64_AND);
                }
                // min/max: select between the two values on a comparison of
                // their .W-truncated (or full .D) forms.
                _ => {
                    let unsigned = f5 == 0x18 || f5 == 0x1c;
                    let want_less = f5 == 0x10 || f5 == 0x18; // MIN / MINU
                    m.local_get(VAL);
                    push_rs2(m);
                    // comparison operands
                    m.local_get(VAL);
                    if len == 4 && unsigned {
                        m.i64_const(0xffff_ffff).op(I64_AND);
                    }
                    push_rs2(m);
                    if len == 4 {
                        if unsigned {
                            m.i64_const(0xffff_ffff).op(I64_AND);
                        } else {
                            m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                        }
                    }
                    m.op(match (unsigned, want_less) {
                        (false, true) => I64_LT_S,
                        (false, false) => I64_GT_S,
                        (true, true) => I64_LT_U,
                        (true, false) => I64_GT_U,
                    });
                    m.op(SELECT); // cond ? old : x[rs2]
                }
            }
            m.op(if len == 8 { I64_STORE } else { I64_STORE32 })
                .raw_uleb(len_align(len))
                .raw_uleb(0);
            // x[rd] = old
            if c.store_pre(m, d) {
                m.local_get(VAL);
                c.store_post(m, d);
            }
        }
        // OP-FP: double add/sub/mul/div + compares + FMV.D.X/FMV.X.D inline.
        0x53 if lay.f_base != 0 => {
            if !fp_handled(insn) {
                return false;
            }
            let f7 = funct7(insn);
            let (fmt, fpop, f3) = (f7 & 3, f7 >> 2, funct3(insn));
            match (fpop, fmt, f3) {
                (0..=3, 1, 0 | 7) => c.fp_arith_d(m, fpop, s1, s2, d, f3 == 7, pc, n),
                (4, 1, 0..=2) => c.fp_sgnj_d(m, f3, s1, s2, d),
                (0..=3, 0, 0 | 7) => c.fp_arith_s(m, fpop, s1, s2, d, pc, n),
                (4, 0, 0..=2) => c.fp_sgnj_s(m, f3, s1, s2, d, pc, n),
                (0x14, 0, 0..=2) => c.fp_cmp_s(m, f3, s1, s2, d, pc, n),
                (0x0b, 0, 0 | 7) => c.fp_sqrt_s(m, s1, d, pc, n),
                (0x1e, 0, 0) => {
                    c.store_freg_pre(m, d);
                    c.push_reg(m, s1);
                    m.i64_const(0xffff_ffff).op(I64_AND);
                    m.i64_const(0xffff_ffff_0000_0000u64 as i64).op(I64_OR);
                    c.store_freg_post(m, d);
                }
                (0x1c, 0, 0) => {
                    if c.store_pre(m, d) {
                        c.push_freg(m, s1);
                        m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                        c.store_post(m, d);
                    }
                }
                (8, 0, 0 | 7) => c.fp_cvt_s_d(m, s1, d, pc, n),
                (8, 1, 0 | 7) => c.fp_cvt_d_s(m, s1, d, pc, n),
                (0x18, 0, 1) => c.fp_cvt_w_s(m, s1, d, pc, n),
                (0x1a, 0, 0 | 7) => c.fp_cvt_s_int(m, s1, d, s2 as u32),
                (0x14, 1, 0..=2) => c.fp_cmp_d(m, f3, s1, s2, d, pc, n),
                (0x1e, 1, 0) => {
                            c.store_freg_pre(m, d);
                    c.push_reg(m, s1);
                    c.store_freg_post(m, d);
                }
                (0x1c, 1, 0) => {
                            if c.store_pre(m, d) {
                        c.push_freg(m, s1);
                        c.store_post(m, d);
                    }
                }
                (0x0b, 1, 0 | 7) => c.fp_sqrt_d(m, s1, d, f3 == 7, pc, n),
                (0x18, 1, 1) => c.fp_cvt_w_d(m, s1, d, pc, n),
                (0x1a, 1, 0 | 7) => c.fp_cvt_d_int(m, s1, d, s2 as u32, f3 == 7, pc, n),
                _ => return false,
            }
        }
        // FMADD/FMSUB/FNMSUB/FNMADD.D — exact emulated fma (see fp_fmadd_d).
        0x43 | 0x47 | 0x4b | 0x4f if lay.f_base != 0 => {
            if !fma_handled(op, insn) {
                return false;
            }
            let s3 = ((insn >> 27) & 31) as usize;
            let neg_prod = op == 0x4b || op == 0x4f;
            let neg_c = op == 0x47 || op == 0x4f;
            c.fp_fmadd_d(m, s1, s2, s3, d, neg_prod, neg_c, funct3(insn) == 7, pc, n);
        }
        _ => return false,
    }
    true
}

/// Translate a block starting at `start_pc`. `code` is the guest code bytes and
/// `base` its guest address. Returns None if the first instruction isn't
/// translatable (caller interprets it instead).

/// A forward bulk-copyable self-loop — clang's musl memcpy/memmove(fwd) and
/// fastmem shape: k pairs of `ld T, 8i(S); sd T, 8i(D)` at ascending offsets
/// 0,8,..,8(k-1), three ADDIs advancing D and S by 8k and N by -8k, then
/// `bltu L, N` back to the start (continue while N > L).
struct CopyLoop {
    s: usize,
    d: usize,
    n: usize,
    /// limit register for `bltu l, n` back-edges; 0 (x0) for `bnez n` loops
    /// (continue while n != 0 — identical emission since x0 reads as 0).
    l: usize,
    t_mask: u32,
    /// bytes moved per iteration (= element count x element size)
    stride: i64,
    /// lowest load offset relative to S at iteration entry (window is
    /// [w0, w0 + stride) for both directions)
    w0: i64,
    body_n: u32,
    end_pc: u64,
    /// true = pointers/count decrease each iteration
    bwd: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Val {
    /// origin-register value plus a compile-time offset (reg 0 = constant 0)
    Affine(u8, i64),
    /// holds the value loaded from S + offset this iteration
    Loaded(i64),
    Unknown,
}

/// Symbolically evaluate up to 24 instructions from `start_pc` as ONE
/// iteration of a candidate copy loop, in ANY instruction order/staging
/// (clang emits at least four layouts of the same loop). Accepts when the
/// iteration's complete architectural effect is exactly:
///   - k same-size loads from a contiguous window [w0, w0+stride) off S,
///   - the same window stored to D (each store's value = the same-offset load),
///   - S and D net-advanced by +/-stride, N net-decremented by stride,
///   - any number of temp registers clobbered (their final values are
///     reproduced by the real tail iterations the emitter always leaves),
///   - back-edge `bltu L, N -> start` (L loop-invariant) or `bnez N -> start`
///     (encoded as L = x0: continue while 0 <u N).
fn detect_copy_loop(code: &[u8], base: u64, start_pc: u64) -> Option<CopyLoop> {
    let mut val: [Val; 32] = [Val::Unknown; 32];
    for (r, v) in val.iter_mut().enumerate() {
        *v = Val::Affine(r as u8, 0);
    }
    let mut s_reg = usize::MAX;
    let mut d_reg = usize::MAX;
    let mut loads: Vec<(i64, u64)> = Vec::new(); // (offset, size)
    let mut stores: Vec<(i64, u64)> = Vec::new();
    let mut written = 0u32;
    let mut pc = start_pc;
    let mut body_n = 0u32;
    loop {
        if body_n > 24 {
            return None;
        }
        let (insn, il) = fetch(code, base, pc)?;
        let (rd_i, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
        match opcode(insn) {
            // loads: ld (f3=3) and lbu (f3=4)
            0x03 if matches!(funct3(insn), 3 | 4) => {
                let sz = if funct3(insn) == 3 { 8 } else { 1 };
                let Val::Affine(b, c) = val[s1] else { return None };
                if b == 0 {
                    return None;
                }
                if s_reg == usize::MAX {
                    s_reg = b as usize;
                } else if b as usize != s_reg {
                    return None;
                }
                let off = c + imm_i(insn);
                if loads.iter().any(|&(o, _)| o == off) || rd_i == 0 {
                    return None;
                }
                loads.push((off, sz));
                val[rd_i] = Val::Loaded(off);
                written |= 1 << rd_i;
            }
            // stores: sd (f3=3) and sb (f3=0)
            0x23 if matches!(funct3(insn), 0 | 3) => {
                let sz = if funct3(insn) == 3 { 8 } else { 1 };
                let Val::Affine(b, c) = val[s1] else { return None };
                if b == 0 {
                    return None;
                }
                if d_reg == usize::MAX {
                    d_reg = b as usize;
                } else if b as usize != d_reg {
                    return None;
                }
                let off = c + imm_s(insn);
                let Val::Loaded(lo) = val[s2] else { return None };
                if lo != off {
                    return None; // value must come from the same window slot
                }
                if stores.iter().any(|&(o, _)| o == off) {
                    return None;
                }
                // sizes must match the load of that offset
                if loads.iter().find(|&&(o, _)| o == off)?.1 != sz {
                    return None;
                }
                stores.push((off, sz));
            }
            // addi rd, rs1, imm — affine arithmetic
            0x13 if funct3(insn) == 0 => {
                if rd_i == 0 {
                    return None;
                }
                val[rd_i] = match val[s1] {
                    Val::Affine(b, c) => Val::Affine(b, c + imm_i(insn)),
                    _ => Val::Unknown,
                };
                written |= 1 << rd_i;
            }
            // add rd, x0, rs2 (mv) — value copy; other ADD forms unsupported
            0x33 if funct3(insn) == 0 && funct7(insn) == 0 => {
                if rd_i == 0 {
                    return None;
                }
                if s1 == 0 {
                    val[rd_i] = val[s2];
                } else if s2 == 0 {
                    val[rd_i] = val[s1];
                } else {
                    return None;
                }
                written |= 1 << rd_i;
            }
            // back-edge: bltu L, N (f3=6) or bne N, x0 (f3=1, rs2=x0)
            0x63 => {
                let (l_reg, n_reg) = match funct3(insn) {
                    6 => (s1, s2),
                    1 if s2 == 0 => (0, s1),
                    _ => return None,
                };
                if pc.wrapping_add(imm_b(insn) as u64) != start_pc {
                    return None;
                }
                body_n += 1;
                let end_pc = pc.wrapping_add(il);
                // --- validate the iteration's net effect ---
                if s_reg == usize::MAX || d_reg == usize::MAX || s_reg == d_reg {
                    return None;
                }
                if loads.len() != stores.len() || loads.is_empty() {
                    return None;
                }
                let sz = loads[0].1;
                if loads.iter().any(|&(_, z)| z != sz) {
                    return None;
                }
                let mut offs: Vec<i64> = loads.iter().map(|&(o, _)| o).collect();
                offs.sort_unstable();
                let w0 = offs[0];
                for (i, &o) in offs.iter().enumerate() {
                    if o != w0 + (i as i64) * sz as i64 {
                        return None; // window must be contiguous
                    }
                }
                let stride = (loads.len() as i64) * sz as i64;
                // net effects on S, D, N; L must be untouched (loop-invariant)
                let step = match (val[s_reg], val[d_reg]) {
                    (Val::Affine(bs, cs), Val::Affine(bd, cd))
                        if bs as usize == s_reg && bd as usize == d_reg && cs == cd =>
                    {
                        cs
                    }
                    _ => return None,
                };
                if step != stride && step != -stride {
                    return None;
                }
                let bwd = step < 0;
                match val[n_reg] {
                    Val::Affine(bn, cn) if bn as usize == n_reg && cn == -stride => {}
                    _ => return None,
                }
                if l_reg != 0 {
                    match val[l_reg] {
                        Val::Affine(bl, 0) if bl as usize == l_reg => {}
                        _ => return None,
                    }
                }
                if n_reg == 0
                    || s_reg == 0
                    || d_reg == 0
                    || n_reg == s_reg
                    || n_reg == d_reg
                    || l_reg == s_reg
                    || l_reg == d_reg
                    || l_reg == n_reg
                {
                    return None;
                }
                // temp/clobber set: everything written except S, D, N
                let t_mask =
                    written & !(1u32 << s_reg) & !(1 << d_reg) & !(1 << n_reg);
                if t_mask & (1 << l_reg) != 0 && l_reg != 0 {
                    return None;
                }
                return Some(CopyLoop {
                    s: s_reg,
                    d: d_reg,
                    n: n_reg,
                    l: l_reg,
                    t_mask,
                    stride,
                    w0,
                    body_n,
                    end_pc,
                    bwd,
                });
            }
            _ => return None,
        }
        pc = pc.wrapping_add(il);
        body_n += 1;
    }
}

/// Compile a detected copy loop: the fast path performs the architectural
/// effect of many iterations with ONE wasm `memory.copy` per page-bounded
/// chunk (through the fused load/store TLBs, so MMIO / non-writable /
/// compiled-page invariants hold), and ALWAYS leaves the tail (N' > L
/// guaranteed) to the in-block normal body — which sets the temp registers
/// exactly as real execution would. Retirement is exact: each chunk adds
/// iterations x body_n to ITER; the normal body adds body_n; mid-body bails
/// report ITER + position (Ctx::bail). This is the riscv64 counter to v86's
/// rep-movs bulk-copy fast path.
fn translate_copy_loop(
    cl: &CopyLoop,
    code: &[u8],
    base: u64,
    start_pc: u64,
    lay: JitLayout,
) -> Option<Block> {
    let sys = lay.sys?;
    let read_mask =
        (1u32 << cl.s) | (1 << cl.d) | (1 << cl.n) | (1 << cl.l) | cl.t_mask | 1; // bit 0: scratch
    let write_mask = (1u32 << cl.s) | (1 << cl.d) | (1 << cl.n) | cl.t_mask;
    let (mut c, mut m) = build_ctx(lay, read_mask, write_mask, 0, 0);
    c.retired_local = Some(ITER);
    let fs = c.fma_scratch;
    debug_assert!(fs != 0);
    let (srci, dsti, kb) = (fs, fs + 1, fs + 2);
    let (rs, rd_, rn) = (c.reg_local[cl.s], c.reg_local[cl.d], c.reg_local[cl.n]);
    let rl = if cl.l == 0 { 0 } else { c.reg_local[cl.l] };
    // push the loop limit: constant 0 for bnez-style loops (l == x0)
    let push_l = |m: &mut WasmModule| {
        if rl == 0 {
            m.i64_const(0);
        } else {
            m.local_get(rl);
        }
    };
    let w = cl.stride;
    // anchor addend: the copy window per iteration is [P + w0, P + w0 + stride);
    // ascending chunks start at (P + w0), descending chunks end (exclusive) at
    // (P + w0 + stride) — page rooms and probe addresses derive from these.
    let adj = cl.w0 + if cl.bwd { cl.stride } else { 0 };

    m.i64_const(0).local_set(ITER);
    emit_fuel_base(&c, &mut m);
    m.op(LOOP).op(VOID); // $head
    // fuel guard (safe yield at the loop head)
    m.local_get(ITER);
    m.local_get(BASE);
    m.op(I64_GE_U);
    m.op(IF).op(VOID);
    c.flush_writes(&mut m);
    c.set_pc_const(&mut m, start_pc);
    m.i32_const(0);
    m.i32_const(0).i64_load(lay.retired_addr as u64);
    m.local_get(ITER).op(I64_ADD).i64_store(lay.retired_addr as u64);
    emit_chain_next(&c, &mut m, true);
    m.op(RETURN);
    m.op(END);

    m.op(BLOCK).op(VOID); // $normal — bulk path brs here to fall back
    {
        // guest loop must be continuing at all: L <u N, else normal path exits
        push_l(&mut m);
        m.local_get(rn).op(I64_LT_U).op(I32_EQZ).br_if(0);
        // kb = iterations we may bulk = (N - L - 1) / w, page-bounded both sides
        m.local_get(rn);
        push_l(&mut m);
        m.op(I64_SUB)
            .i64_const(1)
            .op(I64_SUB)
            .i64_const(w)
            .op(I64_DIV_U)
            .local_set(kb);
        m.local_get(kb).op(I64_EQZ).br_if(0);
        // in-page room in ITERATIONS for each pointer; direction decides the
        // room formula: ascending copies have 4096 - (P & 4095) bytes above P,
        // descending have ((P - 1) & 4095) + 1 bytes below (exclusive-top P).
        let room = |m: &mut WasmModule, ptr: u32, bwd: bool| {
            // anchor = ptr + adj (window start for fwd, exclusive top for bwd)
            let anchor = |m: &mut WasmModule| {
                m.local_get(ptr);
                if adj != 0 {
                    m.i64_const(adj).op(I64_ADD);
                }
            };
            if bwd {
                anchor(m);
                m.i64_const(1)
                    .op(I64_SUB)
                    .i64_const(4095)
                    .op(I64_AND)
                    .i64_const(1)
                    .op(I64_ADD);
            } else {
                m.i64_const(4096);
                anchor(m);
                m.i64_const(4095).op(I64_AND);
                m.op(I64_SUB);
            }
            m.i64_const(w).op(I64_DIV_U);
        };
        room(&mut m, rs, cl.bwd);
        m.local_set(srci);
        m.local_get(srci).local_get(kb);
        m.local_get(srci).local_get(kb).op(I64_LT_U).op(SELECT);
        m.local_set(kb);
        room(&mut m, rd_, cl.bwd);
        m.local_set(srci);
        m.local_get(srci).local_get(kb);
        m.local_get(srci).local_get(kb).op(I64_LT_U).op(SELECT);
        m.local_set(kb);
        m.local_get(kb).op(I64_EQZ).br_if(0);
        // kb <- BYTES
        m.local_get(kb).i64_const(w).op(I64_MUL).local_set(kb);
        // overlap-propagation hazard: the REAL loop reads bytes it has just
        // written when the trailing pointer is within `bytes` ahead of the
        // leading one — ascending: 0 <= D-S < bytes; descending: 0 <= S-D <
        // bytes. memory.copy is memmove-semantics and would differ; fall back
        // to the exact normal body. (Equality is conservatively included.)
        if cl.bwd {
            m.local_get(rs).local_get(rd_).op(I64_SUB);
        } else {
            m.local_get(rd_).local_get(rs).op(I64_SUB);
        }
        m.local_get(kb).op(I64_LT_U).br_if(0);
        // probe src range START (load class); miss -> $normal
        m.local_get(rs);
        if adj != 0 {
            m.i64_const(adj).op(I64_ADD);
        }
        if cl.bwd {
            m.local_get(kb).op(I64_SUB);
        }
        m.local_set(VA);
        m.local_get(VA).i64_const(12).op(I64_SHR_U).local_set(PAGE);
        m.local_get(PAGE)
            .op(I32_WRAP_I64)
            .i32_const(sys.tlb_mask as i32)
            .op(I32_AND)
            .i32_const(3)
            .op(I32_SHL)
            .local_set_i32(c.idxb);
        m.local_get_i32(c.idxb).i64_load_at(sys.ftlb_load_tag as u64);
        m.local_get(PAGE).op(I64_NE).br_if(0);
        m.local_get(VA);
        m.local_get_i32(c.idxb).i64_load_at(sys.ftlb_load_off as u64);
        m.op(I64_ADD).local_set(srci);
        // probe dst range START (store class); miss -> $normal
        m.local_get(rd_);
        if adj != 0 {
            m.i64_const(adj).op(I64_ADD);
        }
        if cl.bwd {
            m.local_get(kb).op(I64_SUB);
        }
        m.local_set(VA);
        m.local_get(VA).i64_const(12).op(I64_SHR_U).local_set(PAGE);
        m.local_get(PAGE)
            .op(I32_WRAP_I64)
            .i32_const(sys.tlb_mask as i32)
            .op(I32_AND)
            .i32_const(3)
            .op(I32_SHL)
            .local_set_i32(c.idxb);
        m.local_get_i32(c.idxb).i64_load_at(sys.ftlb_store_tag as u64);
        m.local_get(PAGE).op(I64_NE).br_if(0);
        m.local_get(VA);
        m.local_get_i32(c.idxb).i64_load_at(sys.ftlb_store_off as u64);
        m.op(I64_ADD).local_set(dsti);
        if lay.copystat_addr != 0 {
            // diagnostic: accumulate BYTES bulk-copied (kb holds bytes here)
            m.i32_const(0);
            m.i32_const(0).i64_load(lay.copystat_addr as u64);
            m.local_get(kb).op(I64_ADD);
            m.i64_store(lay.copystat_addr as u64);
        }
        // memory.copy(dst, src, bytes)
        m.local_get(dsti).op(I32_WRAP_I64);
        m.local_get(srci).op(I32_WRAP_I64);
        m.local_get(kb).op(I32_WRAP_I64);
        m.memory_copy();
        // S/D advance in the copy direction; N always decreases
        let ptr_step = if cl.bwd { I64_SUB } else { I64_ADD };
        m.local_get(rs).local_get(kb).op(ptr_step).local_set(rs);
        m.local_get(rd_).local_get(kb).op(ptr_step).local_set(rd_);
        m.local_get(rn).local_get(kb).op(I64_SUB).local_set(rn);
        // ITER += (bytes / w) * body_n
        m.local_get(kb)
            .i64_const(w)
            .op(I64_DIV_U)
            .i64_const(cl.body_n as i64)
            .op(I64_MUL)
            .local_get(ITER)
            .op(I64_ADD)
            .local_set(ITER);
        m.br(1); // continue $head (fuel re-checked per chunk)
    }
    m.op(END); // $normal

    // NORMAL BODY: one real iteration (exact temps/flags/bail semantics)
    let mut pc = start_pc;
    let mut i = 0u32;
    while i < cl.body_n - 1 {
        let (insn, il) = fetch(code, base, pc)?;
        if !emit_simple(&mut m, &c, lay, insn, pc, i) {
            return None;
        }
        pc = pc.wrapping_add(il);
        i += 1;
    }
    m.local_get(ITER)
        .i64_const(cl.body_n as i64)
        .op(I64_ADD)
        .local_set(ITER);
    // continue while L <u N (L == const 0 for bnez-style loops)
    push_l(&mut m);
    m.local_get(rn).op(I64_LT_U).br_if(0);
    c.flush_writes(&mut m);
    c.set_pc_const(&mut m, cl.end_pc);
    m.i32_const(0);
    m.i32_const(0).i64_load(lay.retired_addr as u64);
    m.local_get(ITER).op(I64_ADD).i64_store(lay.retired_addr as u64);
    emit_chain_next(&c, &mut m, true);
    m.op(RETURN);
    m.op(END); // loop

    Some(Block {
        wasm: m.finish(),
        span: (0, 0),
        seeds: Vec::new(),
        uses_fp: false,
        locals: (0, 0),
        len: cl.end_pc - start_pc,
        n_insns: cl.body_n,
    })
}

/// Host-side twin of the FMADD.D fast path the JIT emits (fp_fmadd_d): every
/// f64 operation here corresponds 1:1 to an emitted wasm op, and both are
/// IEEE-754 binary64 round-to-nearest-even — so the fuzz test proving this
/// function bit-exact against softfp::sf64::fma proves the emission.
///
/// Returns Some(result bits) exactly when the emitted code produces a result;
/// None where it bails to softfloat. The algorithm: Dekker TwoProduct gives
/// p + e == a*b EXACTLY; Knuth TwoSum gives s + t == p + c EXACTLY; therefore
/// a*b + c == s + (t + e) as reals. If t + e is exactly representable
/// (checked by its own TwoSum tail d == 0), the single rounded add s + u IS
/// round(a*b + c) — no double rounding, rigorously. Anything else bails:
/// operand/product exponents outside the band that keeps the splits and the
/// error chain exact, d != 0, or a non-normal final result.
pub fn fma_fastpath_ref(ab: u64, bb: u64, cb: u64) -> Option<u64> {
    let exp = |x: u64| ((x >> 52) & 0x7ff) as i64;
    let is_zero = |x: u64| x << 1 == 0;
    for &x in &[ab, bb, cb] {
        let e = exp(x);
        if !(0x100..=0x6ff).contains(&e) && !is_zero(x) {
            return None;
        }
    }
    let a = f64::from_bits(ab);
    let b = f64::from_bits(bb);
    let c = f64::from_bits(cb);
    let p = a * b;
    if !(0x100..=0x6ff).contains(&exp(p.to_bits())) && !is_zero(p.to_bits()) {
        return None;
    }
    const CSPLIT: f64 = 134217729.0; // 2^27 + 1 (Dekker)
    let a1 = a * CSPLIT;
    let ah = a1 - (a1 - a);
    let al = a - ah;
    let b1 = b * CSPLIT;
    let bh = b1 - (b1 - b);
    let bl = b - bh;
    let e = ((ah * bh - p) + ah * bl + al * bh) + al * bl; // p + e == a*b exactly
    let s = p + c;
    let z = s - p;
    let t = (p - (s - z)) + (c - z); // s + t == p + c exactly (Knuth TwoSum)
    let u = t + e;
    let z2 = u - t;
    let d = (t - (u - z2)) + (e - z2); // u + d == t + e exactly
    // Round-to-odd correction (Boldo-Melquiond): when t + e rounded (d != 0),
    // replace u by its neighbor with an ODD last mantissa bit, on the side of
    // the true value. The final RNE add of s + RO(t+e) is then provably the
    // correctly rounded a*b + c — no double rounding, for ALL inputs in band.
    let v = if d != 0.0 {
        let ub = u.to_bits();
        if u == 0.0 {
            return None; // t + e underflowed; softfloat owns it
        }
        if ub & 1 == 0 {
            let toward_larger_bits = (d > 0.0) != (u < 0.0);
            f64::from_bits(if toward_larger_bits { ub + 1 } else { ub - 1 })
        } else {
            u // already odd
        }
    } else {
        u
    };
    // == round(a*b + c). A zero correction leaves the result AS s: adding +0
    // to a -0 sum would flip its sign (fma(+0, -0, -0) is -0).
    let r = if v.to_bits() << 1 == 0 { s } else { s + v };
    if !(1..=0x7fe).contains(&exp(r.to_bits())) && !is_zero(r.to_bits()) {
        return None;
    }
    Some(r.to_bits())
}

pub fn translate_block(code: &[u8], base: u64, start_pc: u64, lay: JitLayout) -> Option<Block> {
    translate_block_hot(code, base, start_pc, lay, &|_| false)
}

/// translate_block with a hotness oracle: at a conditional branch whose
/// TAKEN target the caller knows to be hot compiled code while the
/// fall-through is not, the trace follows the taken arm (side-exiting on
/// not-taken). Forward targets only — backward taken arms are loop
/// back-edges and belong to the loop machinery.
pub fn translate_block_hot(
    code: &[u8],
    base: u64,
    start_pc: u64,
    lay: JitLayout,
    hot: &dyn Fn(u64) -> bool,
) -> Option<Block> {
    translate_block_link(code, base, start_pc, lay, hot, &|_| None)
}

/// Emit the intra-batch link at a fixed-target exit: verify the dispatch
/// line still names OUR co-member (pc, generation, and idx == base + j),
/// verify fuel, then DIRECT tail call. The target pc is a compile-time
/// constant, so its dispatch slot is too — every load here is at a constant
/// address and the whole sequence needs no scratch locals. Falls through to
/// the ordinary host return when any check fails.
/// Finish a trace body: full module normally, raw stream in batch mode.
fn seal(m: WasmModule) -> (Vec<u8>, (u32, u32)) {
    let locals = m.locals();
    if raw_body() {
        (m.into_code(), locals)
    } else {
        (m.finish(), locals)
    }
}

fn emit_batch_link(m: &mut WasmModule, lay: &JitLayout, target: u64, j: u32) {
    let off = ((target >> 1) as u64 & lay.dispatch_mask as u64) << 4;
    let line = lay.dispatch_base as u64 + off;
    // line.pc == target?
    m.i32_const(0).i64_load(line);
    m.i64_const(target as i64).op(I64_NE);
    m.op(IF).op(VOID).op(RETURN).op(END);
    // verified under the current address-space generation?
    m.i32_const(0).i32_load(line + 12);
    m.i32_const(0).i32_load(lay.map_gen_addr as u64);
    m.op(I32_NE);
    m.op(IF).op(VOID).op(RETURN).op(END);
    // NO base/idx check: `line.pc == target` already proves the dispatch
    // cache holds THIS pc, and the map-generation check proves the mapping
    // is current; the dirty-page tracker clears the line whenever the code
    // bytes change. Our direct callee is by construction the body compiled
    // for exactly `target`, so it is architecturally correct even if the
    // cache has since pointed that pc at a newer block or page function.
    // fuel left in this grant?
    m.i32_const(0).i64_load(lay.retired_addr as u64);
    m.i32_const(0).i64_load(lay.fuel_addr as u64);
    m.op(I64_GE_U);
    m.op(IF).op(VOID).op(RETURN).op(END);
    m.local_get(0);
    m.return_call(1 + j); // func index space: tlb_fill(0), bodies 1..=N
}

/// translate_block_hot plus an intra-batch LINK oracle: link(target_pc) =
/// Some(member_index) when the fixed-target exit can transfer to a
/// co-member of the same batch module by direct tail call.
pub fn translate_block_link(
    code: &[u8],
    base: u64,
    start_pc: u64,
    lay: JitLayout,
    hot: &dyn Fn(u64) -> bool,
    link: &dyn Fn(u64) -> Option<u32>,
) -> Option<Block> {
    translate_block_ic(code, base, start_pc, lay, hot, link, &|_| None)
}

/// translate_block_link with an INLINE CACHE oracle: `next(pc)` is the
/// target an indirect jump at `pc` was last observed to take. Indirect
/// control flow — function pointers, switch tables, returns whose base is
/// not a traced constant — is what ends tcc-shaped traces at ~15
/// instructions against a 256-instruction cap, so extending traces THROUGH
/// those edges is the only thing that reduces the dispatch COUNT (the
/// binding constraint; seven dispatch-COST designs measured neutral).
/// The guard is one compare against the cached target: hit continues the
/// trace inline, miss publishes the real target and exits to the host.
#[allow(clippy::too_many_arguments)]
pub fn translate_block_ic(
    code: &[u8],
    base: u64,
    start_pc: u64,
    lay: JitLayout,
    hot: &dyn Fn(u64) -> bool,
    link: &dyn Fn(u64) -> Option<u32>,
    next: &dyn Fn(u64) -> Option<u64>,
) -> Option<Block> {
    let skip_detectors = raw_body();
    // The loop/copy detectors see the complete translation window supplied by
    // the host. In diagnostic wide-window mode, a loop that straddles a page
    // boundary can close into a loop region and span registration handles the
    // multi-page dirty/map bookkeeping. The measured host default deliberately
    // supplies one page; region/superblock discovery still spans pages later.
    // Bulk-copyable self-loop (memcpy/memmove word loops): one wasm
    // memory.copy per page-bounded chunk — see translate_copy_loop.
    if lay.sys.is_some() && !skip_detectors {
        if let Some(cl) = detect_copy_loop(code, base, start_pc) {
            if let Some(b) = translate_copy_loop(&cl, code, base, start_pc, lay) {
                return Some(b);
            }
        }
    }
    // Structured loop region (nested loops + forward if-then/break) → compile
    // the whole thing as one wasm function so register locals persist across
    // every iteration of every level (3e-2 / v86 control-flow structuring).
    if (lay.mem.is_some() || lay.sys.is_some()) && !skip_detectors {
        if let Some(region) = loop_region(code, base, start_pc, &lay) {
            let (rm, wm, fr, fw) = scan_regs_region(code, base, start_pc, region.end_pc, &lay);
            let (mut c, mut m) = build_ctx(lay, rm, wm, fr, fw);
            // Mid-loop bails (system TLB miss/MMIO, FP fast-path) must report the
            // live iteration count, not a static index — see Ctx::retired_local.
            c.retired_local = Some(ITER);
            if fr | fw != 0 {
                // ITER is a zero-initialized wasm local here, so the bail
                // correctly reports zero retired before the loop starts. The
                // region is a closed loop, so scanning to its end covers every
                // instruction that can run inside it.
                let (_, round) = body_fp_kind(code, base, start_pc, region.end_pc, &lay, false);
                emit_block_fp_gate(&c, &mut m, start_pc, 0, round);
            }
            if let Some(b) = translate_loop(m, &c, code, base, start_pc, &region, &lay) {
                return Some(b);
            }
        }
    }

    // Trace path: an extended basic block — straight-line through side-exited
    // conditional branches and followed direct jumps/calls, to the first
    // indirect transfer or unhandled op. Registers the trace touches live in
    // wasm locals for its lifetime.
    let (read_mask, write_mask, fp_read, fp_write, scan_n) =
        scan_regs(code, base, start_pc, &lay, hot, next);
    // Trace prologue loads only what the trace READS; write-only registers
    // start undefined and each exit flushes only what has been written by
    // that point (Ctx::defined). A linear trace makes this exact — unlike a
    // loop, no exit can be reached before a write that a later iteration
    // performed. FP keeps the conservative set (its emitters write locals
    // outside store_post).
    let (c, mut m) = build_ctx_load(
        lay,
        read_mask,
        write_mask,
        fp_read,
        fp_write,
        defined_track().then_some((read_mask, fp_read | fp_write)),
    );
    // Budget guard: a long trace retires its whole length once dispatched,
    // which would overshoot small fuel grants (the user_run(1) contract).
    // Bail with zero retired when the grant can't cover the trace — the
    // dispatcher hands the pc to the interpreter, which meters exactly.
    if scan_n > FUEL_GUARD_MIN && lay.fuel_addr != 0 {
        m.i32_const(0).i64_load(lay.fuel_addr as u64);
        m.i64_const(scan_n as i64).op(I64_LT_U);
        m.op(IF).op(VOID).op(RETURN).op(END);
    }
    // The FP gate is emitted AT the first FP instruction (see the loop),
    // not at entry: a long trace whose integer prefix always runs must make
    // progress even when FS is off, or it zero-retire-thrashes.
    let mut fp_gated = false;

    let mut pc = start_pc;
    let mut n = 0u32;
    // Linear-trace constant tracking: registers whose value is a proven
    // compile-time constant along THIS trace — jal links (next_pc), lui,
    // auipc. Any write clears the fact. Sound because the trace is straight-
    // line: the emitted code itself set the value and nothing since wrote it.
    // A `jalr` whose base register carries a known constant is a followable
    // jump — which is how a leaf callee's `ret` continues the caller's trace.
    let mut tf = TraceFacts::new();
    let tl = trace_level();
    // Entry pc of the current observation segment (see the inline cache).
    let mut seg_entry = start_pc;
    // Actual va range consumed (a trace can run backward through a followed
    // call) — the host dirty-tracks and map-verifies every page in it.
    let (mut lo, mut hi) = (start_pc, start_pc);
    // Exit targets = hot-path block leaders (see Block::seeds).
    let mut seeds: Vec<u64> = Vec::new();
    let mut seed = |seeds: &mut Vec<u64>, t: u64| {
        if seeds.len() < 48 && !seeds.contains(&t) {
            seeds.push(t);
        }
    };
    while n < MAX_TRACE as u32 {
        let Some((insn, ilen)) = fetch(code, base, pc) else {
            break;
        };
        let next_pc = pc.wrapping_add(ilen);
        lo = lo.min(pc);
        hi = hi.max(next_pc);
        let op = opcode(insn);
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
        let s1_known = tf.known[s1];

        if !fp_gated
            && (fp_read | fp_write) != 0
            && (matches!(op, 0x53 | 0x43 | 0x47 | 0x4b | 0x4f)
                || (matches!(op, 0x07 | 0x27) && lay.f_base != 0))
        {
            let (_, round) = body_fp_kind(code, base, pc, u64::MAX, &lay, true);
            emit_block_fp_gate(&c, &mut m, pc, n, round);
            fp_gated = true;
        }

        if emit_simple(&mut m, &c, lay, insn, pc, n) {
            tf.step(insn, pc);
            pc = next_pc;
            n += 1;
            continue;
        }

        match op {
            // JAL: link; follow forward jumps AND forward calls — the link
            // register is the compile-time constant next_pc, so the trace
            // continues straight into the callee (caller+callee prefix as
            // one block; the callee's own entry still gets its own block
            // when it is hot in its own right). Backward or out-of-window
            // targets end the block with a constant pc.
            0x6f => {
                let target = pc.wrapping_add(imm_j(insn) as u64);
                if c.store_pre(&mut m, d) {
                    m.i64_const(next_pc as i64);
                    c.store_post(&mut m, d);
                }
                let bounded = target >= base && target < base + code.len() as u64;
                let follow = if d == 0 {
                    target > pc && bounded
                } else {
                    tl >= 2 && target != pc && bounded
                };
                if follow {
                    tf.step(insn, pc);
                    if d != 0 {
                        tf.known[d] = Known::Proven(next_pc);
                    }
                    pc = target;
                    n += 1;
                    continue;
                }
                seed(&mut seeds, target);
                c.flush_writes(&mut m);
                c.set_pc_const(&mut m, target);
                c.set_retired(&mut m, n + 1);
                if let Some(lj) = link(target) {
                    emit_batch_link(&mut m, &lay, target, lj);
                } else {
                    emit_chain_next(&c, &mut m, false);
                }
                let (wasm, locals) = seal(m);
                return Some(Block {
                    wasm,
                    locals,
                    span: (lo, hi),
                    seeds: core::mem::take(&mut seeds),
                    uses_fp: (fp_read | fp_write) != 0,
                    len: next_pc.saturating_sub(start_pc),
                    n_insns: n + 1,
                });
            }
            // JALR: dynamic target — unless the base register carries a
            // constant proven along this trace (a jal link, lui or auipc),
            // in which case the target is compile-time known and the trace
            // FOLLOWS it: a leaf callee's `ret` continues the caller's
            // trace, so call+body+return become one straight line. No code
            // is emitted for a followed ret (pc-only effect); it still
            // counts one retired instruction. funct3 != 0 is a reserved
            // encoding — don't compile it (interpreter owns the trap).
            0x67 => {
                if funct3(insn) != 0 {
                    break;
                }
                if d == 0 && tl >= 3 {
                    let target = match s1_known {
                        Known::Proven(v) | Known::Predicted(v) => {
                            Some(v.wrapping_add(imm_i(insn) as u64) & !1)
                        }
                        Known::No => None,
                    };
                    if let Some(target) = target {
                        if target != pc && target >= base && target < base + code.len() as u64 {
                            if let Known::Predicted(_) = s1_known {
                                // The prediction came through the stack; an
                                // aliasing store could have changed it. One
                                // compare guards it: on a miss, publish the
                                // REAL target and side-exit.
                                c.push_reg(&mut m, s1);
                                m.i64_const(imm_i(insn))
                                    .op(I64_ADD)
                                    .i64_const(!1)
                                    .op(I64_AND)
                                    .local_set(SCR);
                                m.local_get(SCR).i64_const(target as i64).op(I64_NE);
                                m.op(IF).op(VOID);
                                m.i32_const(0)
                                    .local_get(SCR)
                                    .i64_store(lay.pc_addr as u64);
                                c.flush_writes(&mut m);
                                c.set_retired(&mut m, n + 1);
                emit_chain_next(&c, &mut m, false);
                                                m.op(RETURN);
                                m.op(END);
                            }
                            tf.step(insn, pc);
                            pc = target;
                            n += 1;
                            continue;
                        }
                    }
                }
                // Target into SCR FIRST (rd may alias rs1), then the
                // architectural link write, then the inline-cache guard.
                c.push_reg(&mut m, s1);
                m.i64_const(imm_i(insn))
                    .op(I64_ADD)
                    .i64_const(!1)
                    .op(I64_AND)
                    .local_set(SCR);
                if c.store_pre(&mut m, d) {
                    m.i64_const(next_pc as i64);
                    c.store_post(&mut m, d);
                }
                // INLINE CACHE: continue the trace at the OBSERVED target
                // under one compare. A miss publishes the real target (SCR)
                // and exits exactly as an uncached jalr would. This is the
                // only construct that reduces the dispatch COUNT for
                // indirect-heavy code — traces otherwise end here at ~15
                // instructions against a 256-instruction cap.
                // Observations are keyed by the entry pc of a dispatched
                // block: "the block entered at E exited to T" describes E's
                // FIRST indirect site. After an inline cache continues the
                // trace at T, T was itself a dispatched block entry, so
                // next(T) describes the NEXT indirect site. Walking that
                // chain lets one compile thread several indirect edges
                // instead of needing a recompile per edge.
                let ic = if tl >= 3 {
                    next(seg_entry).filter(|&t| {
                        t != pc && t >= base && t < base + code.len() as u64
                    })
                } else {
                    None
                };
                if let Some(t) = ic {
                    seg_entry = t;
                    m.local_get(SCR).i64_const(t as i64).op(I64_NE);
                    m.op(IF).op(VOID);
                    m.i32_const(0).local_get(SCR).i64_store(lay.pc_addr as u64);
                    c.flush_writes(&mut m);
                    c.set_retired(&mut m, n + 1);
                    emit_chain_next(&c, &mut m, false);
                    m.op(RETURN);
                    m.op(END);
                    seed(&mut seeds, t);
                    tf.step(insn, pc);
                    if d != 0 {
                        // The link is the constant next_pc, so a later
                        // `ret` through it can itself be followed.
                        tf.known[d] = Known::Proven(next_pc);
                    }
                    pc = t;
                    n += 1;
                    continue;
                }
                m.i32_const(0).local_get(SCR).i64_store(lay.pc_addr as u64);
                c.flush_writes(&mut m);
                c.set_retired(&mut m, n + 1);
                emit_chain_next(&c, &mut m, false);
                // Dynamic target (return/indirect): the chain call site
                // would be megamorphic — dispatch through the host.
                let (wasm, locals) = seal(m);
                return Some(Block {
                    wasm,
                    locals,
                    span: (lo, hi),
                    seeds: core::mem::take(&mut seeds),
                    uses_fp: (fp_read | fp_write) != 0,
                    len: next_pc.saturating_sub(start_pc),
                    n_insns: n + 1,
                });
            }
            // BRANCH: side-exit on taken, trace continues on the fall-through
            // (extended basic block). The taken arm publishes its target pc
            // and exact retired count and returns; the block keeps going —
            // branchy call-shaped code (tcc, CPython) otherwise fragments
            // into ~8-insn blocks whose per-dispatch overhead dominates.
            0x63 => {
                let target = pc.wrapping_add(imm_b(insn) as u64);
                let (cmp, cmp_inv) = match funct3(insn) {
                    0 => (I64_EQ, I64_NE),
                    1 => (I64_NE, I64_EQ),
                    4 => (I64_LT_S, I64_GE_S),
                    5 => (I64_GE_S, I64_LT_S),
                    6 => (I64_LT_U, I64_GE_U),
                    7 => (I64_GE_U, I64_LT_U),
                    _ => break,
                };
                c.push_reg(&mut m, s1);
                c.push_reg(&mut m, s2);
                if tl >= 1 {
                    // Hot-biased direction (must mirror scan_regs): follow
                    // the taken arm when the cache proves it hot and the
                    // fall-through cold. (Backward-branch unrolling was tried
                    // here — self-back-edge only and detector-gated variants
                    // both measured net-negative: displaced loop regions cost
                    // more than the saved dispatches.)
                    let t_in = target >= base && target < base + code.len() as u64;
                    let (follow_pc, exit_pc, op) = if t_in && target > pc && hot(target) && !hot(next_pc) {
                        (target, next_pc, cmp_inv)
                    } else {
                        (next_pc, target, cmp)
                    };
                    seed(&mut seeds, exit_pc);
                    m.op(op);
                    m.op(IF).op(VOID);
                    c.set_pc_const(&mut m, exit_pc);
                    c.flush_writes(&mut m);
                    c.set_retired(&mut m, n + 1);
                    if let Some(lj) = link(exit_pc) {
                        emit_batch_link(&mut m, &lay, exit_pc, lj);
                    } else {
                        emit_chain_next(&c, &mut m, false);
                    }
                    m.op(RETURN);
                    m.op(END);
                    pc = follow_pc;
                    n += 1;
                    continue;
                }
                m.op(cmp);
                m.op(IF).op(VOID);
                c.set_pc_const(&mut m, target);
                m.op(ELSE);
                c.set_pc_const(&mut m, next_pc);
                m.op(END);
                c.flush_writes(&mut m);
                c.set_retired(&mut m, n + 1);
                emit_chain_next(&c, &mut m, false);
                let (wasm, locals) = seal(m);
                return Some(Block {
                    wasm,
                    locals,
                    span: (lo, hi),
                    seeds: core::mem::take(&mut seeds),
                    uses_fp: (fp_read | fp_write) != 0,
                    len: next_pc.saturating_sub(start_pc),
                    n_insns: n + 1,
                });
            }
            // AMO / SYSTEM / single-FP / memory with no layout: end the block.
            _ => break,
        }
    }

    if n == 0 {
        return None;
    }
    c.flush_writes(&mut m);
    c.set_pc_const(&mut m, pc);
    c.set_retired(&mut m, n);
    if let Some(lj) = link(pc) {
        emit_batch_link(&mut m, &lay, pc, lj);
    } else {
        emit_chain_next(&c, &mut m, false);
    }
    let (wasm, locals) = seal(m);
    Some(Block {
        wasm,
        locals,
        span: (lo, hi),
        seeds: core::mem::take(&mut seeds),
        uses_fp: (fp_read | fp_write) != 0,
        len: pc.saturating_sub(start_pc),
        n_insns: n,
    })
}

/// Compile a validated structured loop region into one wasm function. Nested
/// natural loops become nested `block`+`loop` pairs; forward branches become
/// wasm `if` (if-then) or `br` to an enclosing `block` (break). Register locals
/// persist across every iteration of every level. Retired-instruction
/// accounting is exact — each basic block adds its length to the accumulator
/// once, conditionally inside an `if` body — so coverage/insn-count stay right.
/// Local ITER doubles as that accumulator and the loop-cap guard.
fn translate_loop(
    mut m: WasmModule,
    c: &Ctx,
    code: &[u8],
    base: u64,
    start_pc: u64,
    region: &LoopRegion,
    lay: &JitLayout,
) -> Option<Block> {
    m.i64_const(0).local_set(ITER); // ITER = retired-instruction accumulator
    emit_fuel_base(c, &mut m);
    // Scope stack entry: (kind, close_pc, header). kind 0=block 1=loop 2=if.
    let mut scopes: Vec<(u8, u64, u64)> = Vec::new();
    let mut pc = start_pc;
    let mut static_n = 0u32;
    let mut seg = 0u64; // straight-line insns since the last retired flush
    let mut guard = 0u32;
    loop {
        guard += 1;
        if guard > 8192 {
            return None;
        }
        // Close scopes ending here. An `if` first flushes its (conditional)
        // body length into retired, still inside the `if`.
        while let Some(&(kind, cp, _)) = scopes.last() {
            if cp != pc {
                break;
            }
            if kind == 2 && seg > 0 {
                m.local_get(ITER)
                    .i64_const(seg as i64)
                    .op(I64_ADD)
                    .local_set(ITER);
                seg = 0;
            }
            m.op(END);
            scopes.pop();
        }
        // Open a loop at a header: flush the unconditional straight-line run
        // preceding it, then emit block+loop and the loop-top cap guard.
        if let Some(&(h, e)) = region.loops.iter().find(|&&(h, _)| h == pc) {
            if seg > 0 {
                m.local_get(ITER)
                    .i64_const(seg as i64)
                    .op(I64_ADD)
                    .local_set(ITER);
                seg = 0;
            }
            m.op(BLOCK).op(VOID);
            scopes.push((0, e, h));
            m.op(LOOP).op(VOID);
            scopes.push((1, e, h));
            // Fuel guard at the loop top — a safe yield point: resume at header
            // with registers flushed (no partial iteration state to lose).
            // Fuel = min(caller budget, interrupt quantum), granted per
            // dispatch by the host (P0: budget/interrupt-latency contract).
            m.local_get(ITER);
            m.local_get(BASE);
            m.op(I64_GE_U);
            m.op(IF).op(VOID);
            c.flush_writes(&mut m);
            c.set_pc_const(&mut m, h);
            m.i32_const(0);
    m.i32_const(0).i64_load(lay.retired_addr as u64);
    m.local_get(ITER).op(I64_ADD).i64_store(lay.retired_addr as u64);
    emit_chain_next(c, &mut m, true);
            m.op(RETURN);
            m.op(END);
        }
        if pc == region.end_pc {
            break;
        }
        let (insn, ilen) = fetch(code, base, pc)?;
        let next = pc.wrapping_add(ilen);
        if region.unconditional_latch && opcode(insn) == 0x6f {
            let target = pc.wrapping_add(imm_j(insn) as u64);
            if rd(insn) != 0 || target >= pc {
                return None;
            }
            m.local_get(ITER)
                .i64_const((seg + 1) as i64)
                .op(I64_ADD)
                .local_set(ITER);
            seg = 0;
            let li = scopes.iter().rposition(|&(k, _, h)| k == 1 && h == target)?;
            let depth = (scopes.len() - 1 - li) as u32;
            m.br(depth);
            pc = next;
            static_n += 1;
            continue;
        }
        if opcode(insn) != 0x63 {
            // Pass the SEGMENT-relative completed count: a mid-block bail
            // reports ITER (flushed segments) + this (see Ctx::bail).
            if !emit_simple(&mut m, c, *lay, insn, pc, seg as u32) {
                return None;
            }
            seg += 1;
            pc = next;
            static_n += 1;
            continue;
        }
        // Conditional branch: continue (back-edge) / break / if-then.
        let (s1, s2) = (rs1(insn), rs2(insn));
        let f3 = funct3(insn);
        let target = pc.wrapping_add(imm_b(insn) as u64);
        let cmp = match f3 {
            0 => I64_EQ,
            1 => I64_NE,
            4 => I64_LT_S,
            5 => I64_GE_S,
            6 => I64_LT_U,
            7 => I64_GE_U,
            _ => return None,
        };
        // The branch always executes on reaching it: flush the straight-line
        // segment plus this instruction into retired, unconditionally.
        m.local_get(ITER)
            .i64_const((seg + 1) as i64)
            .op(I64_ADD)
            .local_set(ITER);
        seg = 0;
        if target < start_pc {
            // Backward EXIT (rotated nest): leave the region with pc =
            // target. The branch instruction was already flushed into ITER
            // above, so bail(target, 0) publishes the exact count.
            c.push_reg(&mut m, s1);
            c.push_reg(&mut m, s2);
            m.op(cmp);
            m.op(IF).op(VOID);
            c.bail(&mut m, target, 0);
            m.op(END);
            pc = next;
            static_n += 1;
            continue;
        }
        if target < pc {
            // back-edge → continue the loop whose header == target.
            let li = scopes.iter().rposition(|&(k, _, h)| k == 1 && h == target)?;
            let depth = (scopes.len() - 1 - li) as u32;
            c.push_reg(&mut m, s1);
            c.push_reg(&mut m, s2);
            m.op(cmp);
            m.br_if(depth);
        } else if let Some(bi) = scopes.iter().rposition(|&(k, cp, _)| k == 0 && cp == target) {
            // forward branch to an enclosing loop's exit → break.
            let depth = (scopes.len() - 1 - bi) as u32;
            c.push_reg(&mut m, s1);
            c.push_reg(&mut m, s2);
            m.op(cmp);
            m.br_if(depth);
        } else {
            // forward if-then: run [next, target) under the NEGATED condition.
            let neg = match f3 {
                0 => I64_NE,
                1 => I64_EQ,
                4 => I64_GE_S,
                5 => I64_LT_S,
                6 => I64_GE_U,
                _ => I64_LT_U,
            };
            c.push_reg(&mut m, s1);
            c.push_reg(&mut m, s2);
            m.op(neg);
            m.op(IF).op(VOID);
            scopes.push((2, target, 0));
        }
        pc = next;
        static_n += 1;
    }
    if !scopes.is_empty() {
        return None; // unbalanced — refuse rather than emit broken wasm
    }
    if seg > 0 {
        m.local_get(ITER)
            .i64_const(seg as i64)
            .op(I64_ADD)
            .local_set(ITER);
    }
    c.flush_writes(&mut m);
    c.set_pc_const(&mut m, region.end_pc);
    m.i32_const(0);
    m.i32_const(0).i64_load(lay.retired_addr as u64);
    m.local_get(ITER).op(I64_ADD).i64_store(lay.retired_addr as u64);
    emit_chain_next(c, &mut m, true);
    Some(Block {
        wasm: m.finish(),
        span: (0, 0),
        seeds: Vec::new(),
        uses_fp: false,
        locals: (0, 0),
        len: region.end_pc - start_pc,
        n_insns: static_n.max(1),
    })
}

/// Scan the touched GP/FP registers across every entry block of a page-
/// superblock (each block walked to its terminating control-flow / unhandled
/// instruction). Over-approximating is safe (an unused local just gets loaded).
/// Diagnostic wrapper: the register union a superblock over `entries` would
/// load at every entry and store at every exit.
pub fn scan_regs_super_pub(
    code: &[u8],
    base: u64,
    page_end: u64,
    entries: &[u64],
    lay: &JitLayout,
) -> (u32, u32, u32, u32) {
    let _ = page_end;
    let (r, w, fr, fw, _) = scan_regs_super(code, &[base], entries, lay);
    (r, w, fr, fw)
}

fn scan_regs_super(
    code: &[u8],
    page_vas: &[u64],
    entries: &[u64],
    lay: &JitLayout,
) -> (u32, u32, u32, u32, bool) {
    let (mut r, mut w, mut fr, mut fw) = (0u32, 0u32, 0u32, 0u32);
    let mut mem_ops = 0u32;
    // Uses per register across every body, so rarely-touched registers can be
    // left in memory (see SB_HOIST_MIN below).
    let mut uses = [0u32; 32];
    let mut fuses = [0u32; 32];
    let fmark = |m: &mut u32, u: &mut [u32; 32], x: usize| {
        *m |= 1 << x;
        u[x] += 1;
    };
    let mark = |m: &mut u32, u: &mut [u32; 32], x: usize| {
        if x != 0 {
            *m |= 1 << x;
            u[x] += 1;
        }
    };
    for &e in entries {
        let Some(pi) = page_vas
            .iter()
            .position(|&va| e.wrapping_sub(va) < 0x1000)
        else {
            continue;
        };
        let base = page_vas[pi] - (pi as u64) * 0x1000;
        // Same boundary rule as emission: flow across contiguous neighbours,
        // stop at a gap.
        let mut pj = pi;
        while pj + 1 < page_vas.len() && page_vas[pj + 1] == page_vas[pj] + 0x1000 {
            pj += 1;
        }
        let page_end = page_vas[pj] + 0x1000;
        let code = &code[..(pj + 1) * 0x1000];
        let mut pc = e;
        let mut n = 0u32;
        while n < MAX_BLOCK as u32 && pc < page_end {
            let Some((insn, ilen)) = fetch(code, base, pc) else {
                break;
            };
            let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
            let op = opcode(insn);
            match op {
                0x63 => {
                    mark(&mut r, &mut uses, s1);
                    mark(&mut r, &mut uses, s2);
                    break;
                }
                0x6f => {
                    mark(&mut w, &mut uses, d);
                    break;
                }
                0x67 => {
                    mark(&mut r, &mut uses, s1);
                    mark(&mut w, &mut uses, d);
                    break;
                }
                0x37 | 0x17 => mark(&mut w, &mut uses, d),
                0x13 | 0x1b => {
                    mark(&mut r, &mut uses, s1);
                    mark(&mut w, &mut uses, d);
                }
                0x33 | 0x3b => {
                    if !alu_handled(op, funct7(insn), funct3(insn)) {
                        break;
                    }
                    mark(&mut r, &mut uses, s1);
                    mark(&mut r, &mut uses, s2);
                    mark(&mut w, &mut uses, d);
                }
                0x03 => {
                    if funct3(insn) == 7 {
                        break;
                    }
                    mem_ops += 1;
                    mark(&mut r, &mut uses, s1);
                    mark(&mut w, &mut uses, d);
                }
                0x23 => {
                    if funct3(insn) > 3 {
                        break;
                    }
                    mem_ops += 1;
                    mark(&mut r, &mut uses, s1);
                    mark(&mut r, &mut uses, s2);
                }
                0x2f if lay.sys.is_some() => {
                    if !amo_handled(insn) {
                        break;
                    }
                    mark(&mut r, &mut uses, s1);
                    mark(&mut r, &mut uses, s2);
                    mark(&mut w, &mut uses, d);
                    mem_ops += 1;
                }
                0x07 if lay.f_base != 0 => {
                    if !matches!(funct3(insn), 2 | 3) {
                        break;
                    }
                    mem_ops += 1;
                    mark(&mut r, &mut uses, s1);
                    fmark(&mut fw, &mut fuses, d);
                }
                0x27 if lay.f_base != 0 => {
                    if !matches!(funct3(insn), 2 | 3) {
                        break;
                    }
                    mem_ops += 1;
                    mark(&mut r, &mut uses, s1);
                    fmark(&mut fr, &mut fuses, s2);
                }
                0x53 if lay.f_base != 0 => {
                    if !fp_handled(insn) {
                        break;
                    }
                    let f7 = funct7(insn);
                    match (f7 >> 2, f7 & 3, funct3(insn)) {
                        (0..=3, 1, 0 | 7) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fr, &mut fuses, s2);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (4, 1, 0..=2) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fr, &mut fuses, s2);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (0x14, 1, 0..=2) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fr, &mut fuses, s2);
                            mark(&mut w, &mut uses, d);
                        }
                        (0x1e, 1, 0) => {
                            mark(&mut r, &mut uses, s1);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (0x1c, 1, 0) => {
                            fmark(&mut fr, &mut fuses, s1);
                            mark(&mut w, &mut uses, d);
                        }
                        (0x0b, 1, 0 | 7) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (0x18, 1, 1) => {
                            fmark(&mut fr, &mut fuses, s1);
                            mark(&mut w, &mut uses, d);
                        }
                        (0x1a, 1, 0 | 7) => {
                            mark(&mut r, &mut uses, s1);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (0..=3, 0, 0 | 7) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fr, &mut fuses, s2);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (4, 0, 0..=2) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fr, &mut fuses, s2);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (0x14, 0, 0..=2) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fr, &mut fuses, s2);
                            mark(&mut w, &mut uses, d);
                        }
                        (0x1e, 0, 0) => {
                            mark(&mut r, &mut uses, s1);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (0x1c, 0, 0) => {
                            fmark(&mut fr, &mut fuses, s1);
                            mark(&mut w, &mut uses, d);
                        }
                        (0x0b, 0, 0 | 7) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (8, 0, 0 | 7) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (8, 1, 0 | 7) => {
                            fmark(&mut fr, &mut fuses, s1);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        (0x18, 0, 1) => {
                            fmark(&mut fr, &mut fuses, s1);
                            mark(&mut w, &mut uses, d);
                        }
                        (0x1a, 0, 0 | 7) => {
                            mark(&mut r, &mut uses, s1);
                            fmark(&mut fw, &mut fuses, d);
                        }
                        _ => break,
                    }
                }
                op @ (0x43 | 0x47 | 0x4b | 0x4f) if lay.f_base != 0 => {
                    if !fma_handled(op, insn) {
                        break;
                    }
                    fmark(&mut fr, &mut fuses, s1);
                    fmark(&mut fr, &mut fuses, s2);
                    fmark(&mut fr, &mut fuses, ((insn >> 27) & 31) as usize);
                    fmark(&mut fw, &mut fuses, d);
                    r |= 1; // bit 0 = "block contains fma" (see build_ctx)
                }
                _ => break,
            }
            pc = pc.wrapping_add(ilen);
            n += 1;
        }
    }
    let _ = mem_ops; // super coalescing measured net-negative: multi-array
    // superblocks thrash a 1-page cache (IDEA 1768->1670); basic/region only.
    //
    // Hoist only registers the function actually leans on. EVERY entry into a
    // superblock loads the hoisted read set and every exit stores the hoisted
    // write set, so a register touched twice in one cold body costs two memory
    // ops on every single entry and saves at most one. Leaving it in memory
    // (the emitter reads/writes x_base directly when a register has no local)
    // makes entry/exit proportional to the hot core instead of to the size of
    // the page. Measured on nbench: superblock pages carried unions of 25 GPRs
    // + 15 FP registers — 60 memory ops per entry against ~15 retired
    // instructions on FOURIER's cross-page libm calls, which is why whole-page
    // superblocks were LOSING to individual blocks there.
    // The FP gate must key off whether the region CONTAINS FP work, never off
    // the hoisted mask — filtering every FP register into memory would
    // otherwise silently drop the gate and run FP without its FS/frm/NX
    // checks.
    let uses_fp = (fr | fw) != 0;
    for x in 1..32 {
        if uses[x] < SB_HOIST_MIN {
            r &= !(1 << x);
            w &= !(1 << x);
        }
        if fuses[x] < SB_HOIST_MIN {
            fr &= !(1 << x);
            fw &= !(1 << x);
        }
    }
    if fuses[0] < SB_HOIST_MIN {
        fr &= !1;
        fw &= !1;
    }
    (r, w, fr, fw, uses_fp)
}

impl Ctx {
    /// Set the superblock target-pc local to a compile-time constant.
    fn set_tpc(&self, m: &mut WasmModule, pc: u64) {
        m.i64_const(pc as i64).local_set(TPC);
    }
    /// Emit `ITER += k` (the retired-instruction accumulator), skipping k==0.
    fn add_retired(&self, m: &mut WasmModule, k: u32) {
        if k != 0 {
            m.local_get(ITER)
                .i64_const(k as i64)
                .op(I64_ADD)
                .local_set(ITER);
        }
    }
    /// Emit one entry block's straight-line body: run until a control-flow /
    /// unhandled instruction, add its length to `retired`, set TPC to the
    /// successor, and `br depth_l` back to the dispatch loop (so the next block
    /// is selected there, or the loop exits if the successor isn't in-page).
    /// End a superblock entry body: continue the dispatch loop (`br depth_l`)
    /// after counting the block. But if the block compiled ZERO instructions
    /// (its first instruction is unhandled / off-page), it can make no progress,
    /// so `br depth_exit` back to the host instead — otherwise setting TPC to
    /// this same entry re-dispatches to itself forever (the cap can't help: it
    /// never retires anything).
    fn super_end(&self, m: &mut WasmModule, pc: u64, len: u32, depth_l: u32, depth_exit: u32) {
        if len == 0 {
            self.set_tpc(m, pc);
            m.br(depth_exit);
        } else {
            self.add_retired(m, len);
            self.set_tpc(m, pc);
            m.br(depth_l);
        }
    }
    fn emit_super_body(
        &self,
        m: &mut WasmModule,
        lay: JitLayout,
        code: &[u8],
        base: u64,
        entry_pc: u64,
        page_end: u64,
        depth_l: u32,
        depth_exit: u32,
    ) {
        let mut pc = entry_pc;
        let mut len = 0u32;
        if lay.f_base != 0 {
            let (any, round) = body_fp_kind(code, base, entry_pc, page_end, &lay, true);
            if any {
                emit_block_fp_gate(self, m, entry_pc, 0, round);
            }
        }
        loop {
            if pc >= page_end || len >= MAX_BLOCK as u32 {
                self.super_end(m, pc, len, depth_l, depth_exit);
                return;
            }
            let Some((insn, ilen)) = fetch(code, base, pc) else {
                self.super_end(m, pc, len, depth_l, depth_exit);
                return;
            };
            let op = opcode(insn);
            let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
            let next = pc.wrapping_add(ilen);
            match op {
                // Conditional branch: TPC = cond ? taken : next.
                0x63 => {
                    let cmp = match funct3(insn) {
                        0 => I64_EQ,
                        1 => I64_NE,
                        4 => I64_LT_S,
                        5 => I64_GE_S,
                        6 => I64_LT_U,
                        _ => I64_GE_U,
                    };
                    if !matches!(funct3(insn), 0 | 1 | 4 | 5 | 6 | 7) {
                        self.super_end(m, pc, len, depth_l, depth_exit);
                        return;
                    }
                    self.add_retired(m, len + 1);
                    let taken = pc.wrapping_add(imm_b(insn) as u64);
                    self.push_reg(m, s1);
                    self.push_reg(m, s2);
                    m.op(cmp);
                    m.op(IF).op(VOID);
                    self.set_tpc(m, taken);
                    m.op(ELSE);
                    self.set_tpc(m, next);
                    m.op(END);
                    m.br(depth_l); // IF closed above, back at body level
                    return;
                }
                // JAL: link then TPC = target.
                0x6f => {
                    self.add_retired(m, len + 1);
                    let target = pc.wrapping_add(imm_j(insn) as u64);
                    if self.store_pre(m, d) {
                        m.i64_const(next as i64);
                        self.store_post(m, d);
                    }
                    self.set_tpc(m, target);
                    m.br(depth_l);
                    return;
                }
                // JALR: TPC = (x[s1]+imm) & ~1, link.
                0x67 => {
                    self.add_retired(m, len + 1);
                    self.push_reg(m, s1);
                    m.i64_const(imm_i(insn))
                        .op(I64_ADD)
                        .i64_const(!1)
                        .op(I64_AND)
                        .local_set(SCR);
                    if self.store_pre(m, d) {
                        m.i64_const(next as i64);
                        self.store_post(m, d);
                    }
                    m.local_get(SCR).local_set(TPC);
                    m.br(depth_l);
                    return;
                }
                _ => {
                    if emit_simple(m, self, lay, insn, pc, len) {
                        pc = next;
                        len += 1;
                    } else {
                        // Unhandled: leave the JIT at this pc (dispatch will exit).
                        self.super_end(m, pc, len, depth_l, depth_exit);
                        return;
                    }
                }
            }
        }
    }
}

/// Is `start_pc` a structured-loop header? Such blocks compile to a tight wasm
/// loop (register-locals across iterations) and must NOT be folded into a
/// superblock, whose per-iteration `br_table` dispatch would be far slower.
/// Every JAL call target that leaves this page (the edges of the call
/// graph): used to pick which pages join a sparse superblock region.
pub fn page_call_targets(code: &[u8], base: u64) -> Vec<u64> {
    let mut out = Vec::new();
    let mut pc = base;
    let end = base + code.len() as u64;
    while pc < end {
        let Some((insn, ilen)) = fetch(code, base, pc) else {
            break;
        };
        if opcode(insn) == 0x6f && rd(insn) != 0 {
            let t = pc.wrapping_add(imm_j(insn) as u64);
            if t & !0xfff != base & !0xfff {
                out.push(t);
            }
        }
        pc = pc.wrapping_add(ilen);
    }
    out
}

/// Can the block emitter make progress at `pc`? A superblock leader whose
/// FIRST instruction can't be emitted becomes an exit stub: every dispatch
/// into it enters the function, reloads the hoisted registers, exits having
/// retired nothing, and leaves the interpreter to step ONE instruction —
/// strictly worse than not knowing the pc at all, where the dispatcher hands
/// the interpreter a full slice. Measured on nbench FOURIER: 60M zero-retire
/// dispatches into two such stubs inside musl's pow.
pub fn emittable_at(code: &[u8], base: u64, pc: u64, lay: JitLayout) -> bool {
    let Some((insn, _)) = fetch(code, base, pc) else {
        return false;
    };
    let op = opcode(insn);
    match op {
        0x63 | 0x6f | 0x67 => true, // control flow: always emittable
        0x37 | 0x17 | 0x13 | 0x33 | 0x1b | 0x3b => alu_handled(op, funct7(insn), funct3(insn)),
        0x03 => funct3(insn) != 7,
        0x23 => funct3(insn) <= 3,
        0x2f => lay.sys.is_some() && amo_handled(insn),
        0x07 | 0x27 => lay.f_base != 0 && matches!(funct3(insn), 2 | 3),
        0x53 => lay.f_base != 0 && fp_handled(insn),
        0x43 | 0x47 | 0x4b | 0x4f => lay.f_base != 0 && fma_handled(op, insn),
        _ => false,
    }
}

pub fn is_loop_at(code: &[u8], base: u64, start_pc: u64, lay: JitLayout) -> bool {
    loop_region(code, base, start_pc, &lay).is_some()
}

/// Statically discover every basic-block leader in a code page reachable from
/// `seeds` (v86's page-analysis pass). Walks each pending leader forward,
/// adding in-page branch targets, fallthroughs, and post-call return sites as
/// new leaders until fixpoint. A superblock compiled over the FULL leader set
/// keeps intra-page control flow inside one wasm function — compiling only the
/// handful of individually-hot pcs misses most of the page and forces a
/// recompile storm (or, compiled once, a permanently sparse br_table).
///
/// `max_leaders` bounds pathological pages (jump-table-dense code). Returns a
/// sorted, deduplicated list. Leaders may start with an instruction the block
/// emitter can't handle — the superblock emitter turns those into exit stubs.
pub fn discover_page_leaders(
    code: &[u8],
    base: u64,
    page_base: u64,
    page_span: u64,
    seeds: &[u64],
    max_leaders: usize,
) -> Vec<u64> {
    discover_page_leaders_ext(code, base, page_base, page_span, seeds, max_leaders).0
}

/// As `discover_page_leaders`, plus the leaders that some BACKWARD branch
/// targets — the only pcs that can be loop headers. Testing every leader with
/// `is_loop_at` costs a 128-instruction walk each, which dominated superblock
/// compile time (232 page compiles cost ~3s of a 3s tcc run); the back-edge
/// set is typically a couple of dozen pcs out of hundreds.
pub fn discover_page_leaders_ext(
    code: &[u8],
    base: u64,
    page_base: u64,
    page_span: u64,
    seeds: &[u64],
    max_leaders: usize,
) -> (Vec<u64>, std::collections::BTreeSet<u64>) {
    let page_end = page_base + page_span;
    let in_page = |pc: u64| pc >= page_base && pc < page_end;
    let mut leaders: std::collections::BTreeSet<u64> = seeds.iter().copied().collect();
    let mut back_targets: std::collections::BTreeSet<u64> = Default::default();
    let mut done: std::collections::BTreeSet<u64> = Default::default();
    let mut pending: Vec<u64> = seeds.to_vec();
    while let Some(start) = pending.pop() {
        if !done.insert(start) {
            continue;
        }
        let mut pc = start;
        let mut n = 0u32;
        while n < MAX_BLOCK as u32 && pc < page_end {
            let Some((insn, ilen)) = fetch(code, base, pc) else {
                break;
            };
            let next = pc.wrapping_add(ilen);
            let mut add = |t: u64, leaders: &mut std::collections::BTreeSet<u64>,
                           pending: &mut Vec<u64>| {
                if in_page(t) && leaders.len() < max_leaders && leaders.insert(t) {
                    pending.push(t);
                }
            };
            match opcode(insn) {
                // conditional branch: target + fallthrough are leaders; block ends
                0x63 => {
                    let target = pc.wrapping_add(imm_b(insn) as u64);
                    if target <= pc {
                        back_targets.insert(target);
                    }
                    add(target, &mut leaders, &mut pending);
                    add(next, &mut leaders, &mut pending);
                    break;
                }
                // JAL: target is a leader if in page; a CALL (rd != 0) also makes
                // the return site a leader (the callee's ret dispatches back there)
                0x6f => {
                    let target = pc.wrapping_add(imm_j(insn) as u64);
                    if target <= pc && rd(insn) == 0 {
                        back_targets.insert(target); // backward tail jump
                    }
                    add(target, &mut leaders, &mut pending);
                    if rd(insn) != 0 {
                        add(next, &mut leaders, &mut pending);
                    }
                    break;
                }
                // JALR: dynamic target; if it links (call), the return site is a
                // leader. Block ends either way.
                0x67 => {
                    if rd(insn) != 0 {
                        add(next, &mut leaders, &mut pending);
                    }
                    break;
                }
                // Anything the emitter can't inline ends the block; execution
                // resumes at the next insn after the interpreter steps it.
                op => {
                    let handled = match op {
                        0x37 | 0x17 | 0x13 | 0x33 | 0x1b | 0x3b => {
                            alu_handled(op, funct7(insn), funct3(insn))
                        }
                        0x03 => funct3(insn) != 7,
                        0x23 => funct3(insn) <= 3,
                        0x2f => amo_handled(insn),
                        0x07 | 0x27 => matches!(funct3(insn), 2 | 3),
                        0x53 => fp_handled(insn),
                        0x43 | 0x47 | 0x4b | 0x4f => fma_handled(op, insn),
                        _ => false,
                    };
                    if !handled {
                        add(next, &mut leaders, &mut pending);
                        break;
                    }
                }
            }
            pc = next;
            n += 1;
        }
    }
    (leaders.into_iter().collect(), back_targets)
}

/// Compile a whole page of basic blocks (v86's function-per-page) into one wasm
/// function with an internal `br_table` dispatch loop and all touched registers
/// cached in locals for the function's lifetime — so execution flows between
/// blocks with no per-block prologue/epilogue, `call_indirect` or pa-verify (the
/// per-dispatch overhead that dominates branchy code like the CPython eval
/// loop). `entries` are the block-start pcs discovered hot in this page.
/// Contiguous wrapper: pages are consecutive from `page_base`.
pub fn translate_superblock(
    code: &[u8],
    base: u64,
    page_base: u64,
    page_span: u64,
    entries: &[u64],
    lay: JitLayout,
) -> Option<Block> {
    let _ = base;
    let vas: Vec<u64> = (0..page_span / 0x1000).map(|k| page_base + k * 0x1000).collect();
    translate_superblock_sparse(code, &vas, entries, lay, false)
}

/// Compile a SPARSE set of code pages as one wasm function (the call-graph
/// region). `code` is the pages' bytes concatenated in `page_vas` order;
/// pages need not be virtually contiguous — the dispatch prologue resolves
/// TPC to (page index, slot) with one compare per page, so a caller and a
/// callee any distance apart still transfer inside the function, with no
/// call_indirect, no pa-verify, and no per-block prologue. This is what a
/// page-contiguous region could never give tcc-like code, where the hot call
/// graph spans a few hundred KB (measured: 9 insns per host dispatch).
pub fn translate_superblock_sparse(
    code: &[u8],
    page_vas: &[u64],
    entries: &[u64],
    lay: JitLayout,
    regs_in_memory: bool,
) -> Option<Block> {
    let n = entries.len();
    let np = page_vas.len();
    if n == 0 || np == 0 || np * 0x1000 != code.len() || np > 16 {
        return None;
    }
    // Page index for a pc, or None if outside every page.
    let pidx = |pc: u64| page_vas.iter().position(|&va| pc.wrapping_sub(va) < 0x1000);
    // fetch()-compatible base for a pc on page i: offset = pc - vbase(i).
    let vbase = |i: usize| page_vas[i] - (i as u64) * 0x1000;

    let (rm, wm, fr, fw, uses_fp) = scan_regs_super(code, page_vas, entries, &lay);
    // Ask for the hoisted FP gate flags when any body will need a gate.
    // `regs_in_memory` drops every register local (the emitters fall back to
    // direct state loads/stores): a function whose stays are short pays the
    // full register-UNION load at every entry and the written union at every
    // exit — for call-shaped code (tcc: ~8-insn stays) that overhead is
    // larger than the work, and measured stay length picks the mode. Bit 0
    // of rm (the scanners' FMADD flag) and the gate-flag request survive.
    let (mut c, mut m) = if regs_in_memory {
        build_ctx(lay, rm & 1, u32::from(uses_fp), 0, 0)
    } else {
        build_ctx(lay, rm, wm | u32::from(uses_fp), fr, fw)
    };
    c.retired_local = Some(ITER);

    // slot (= concat offset / 2) -> entry index, else n (default -> exit).
    let mut slot_depth = vec![n as u32; np * 0x800];
    for (i, &e) in entries.iter().enumerate() {
        let pi = pidx(e)?;
        slot_depth[(pi * 0x800) + ((e & 0xfff) >> 1) as usize] = i as u32;
    }

    m.i64_const(0).local_set(ITER); // retired accumulator
    emit_fuel_base(&c, &mut m);
    m.i32_const(0).i64_load(lay.pc_addr as u64).local_set(TPC);
    // Evaluate the hoisted FP gate flags ONCE at entry — the per-body gates
    // test these locals. The sparse rewrite dropped this call, leaving the
    // flag locals zero-initialized, so every body gate silently passed: FP
    // bodies ran unguarded under non-RNE rounding and FS != Dirty.
    emit_fp_flags(&c, &mut m);

    m.op(BLOCK).op(VOID); // $exit  (depth 1 from loop body)
    m.op(LOOP).op(VOID); // $L      (depth 0 from loop body)

    // Fuel -> yield to the host (budget + interrupt-latency contract).
    m.local_get(ITER);
    m.local_get(BASE);
    m.op(I64_GE_U).br_if(1);
    // Resolve TPC -> concat offset in SCR: one subtract+compare per
    // CONTIGUOUS RUN of pages (a fully contiguous region pays exactly the
    // single range check the contiguous translator had — paying per page
    // cost FP EMULATION a third of its throughput).
    m.op(BLOCK).op(VOID); // $resolve
    let mut i = 0usize;
    while i < np {
        let mut j2 = i;
        while j2 + 1 < np && page_vas[j2 + 1] == page_vas[j2] + 0x1000 {
            j2 += 1;
        }
        let run_len = ((j2 - i + 1) as i64) << 12;
        m.local_get(TPC)
            .i64_const(page_vas[i] as i64)
            .op(I64_SUB)
            .local_set(SCR);
        m.local_get(SCR).i64_const(run_len).op(I64_LT_U);
        m.op(IF).op(VOID);
        if i != 0 {
            m.local_get(SCR)
                .i64_const((i as i64) << 12)
                .op(I64_ADD)
                .local_set(SCR);
        }
        m.br(1); // out of $resolve, offset in SCR
        m.op(END);
        i = j2 + 1;
    }
    m.br(2); // no page matched -> $exit
    m.op(END); // $resolve

    // Open the dispatch nest: block $default, then $e_{n-1}..$e_0 (innermost).
    m.op(BLOCK).op(VOID); // $default (br_table default depth = n)
    for _ in 0..n {
        m.op(BLOCK).op(VOID);
    }
    // idx = offset >> 1 (i32); dispatch.
    m.local_get(SCR)
        .i64_const(1)
        .op(I64_SHR_U)
        .op(I32_WRAP_I64);
    m.br_table(&slot_depth, n as u32);

    // Close $e_0..$e_{n-1}, emitting each entry body after its block's end.
    // At entry i's body the loop $L is at depth (n - i).
    for i in 0..n {
        m.op(END); // close $e_i
        let pi = pidx(entries[i]).unwrap();
        // The body may flow across VIRTUALLY CONTIGUOUS neighbours (their
        // concat offsets line up with their addresses, so fetch stays
        // correct — this is what holds a loop that straddles a page
        // boundary), but must stop at a GAP: the next concat page there
        // belongs to a distant address, and a 32-bit instruction on the last
        // halfword must not complete itself from its bytes. The truncated
        // slice makes fetch() fail at the gap and the body ends.
        let mut pj = pi;
        while pj + 1 < np && page_vas[pj + 1] == page_vas[pj] + 0x1000 {
            pj += 1;
        }
        c.emit_super_body(
            &mut m,
            lay,
            &code[..(pj + 1) * 0x1000],
            vbase(pi),
            entries[i],
            page_vas[pj] + 0x1000,
            (n - i) as u32,
            (n - i + 1) as u32,
        );
    }
    m.op(END); // close $default
    // default: TPC wasn't a known entry in-page -> exit ($exit at depth 1).
    m.br(1);

    m.op(END); // close loop $L
    m.op(END); // close block $exit

    // Exit: flush registers, publish TPC + retired.
    c.flush_writes(&mut m);
    m.i32_const(0).local_get(TPC).i64_store(lay.pc_addr as u64);
    m.i32_const(0);
    m.i32_const(0).i64_load(lay.retired_addr as u64);
    m.local_get(ITER).op(I64_ADD).i64_store(lay.retired_addr as u64);
    emit_chain_next(&c, &mut m, true);

    Some(Block {
        wasm: m.finish(),
        span: (0, 0),
        seeds: Vec::new(),
        uses_fp: false,
        locals: (0, 0),
        len: (np * 0x1000) as u64,
        n_insns: 0,
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Reserved encodings must not compile (PERFORMANCE_PROGRESS.md): shift immediates
    /// with reserved upper bits, OP-IMM-32 shamt[5], FMV/FSQRT fixed rs2.
    #[test]
    fn reserved_encodings_rejected() {
        // SLLI with funct7 = 0x10 (reserved)
        assert!(!alu_handled(0x13, 0x10, 1));
        assert!(alu_handled(0x13, 0x00, 1));
        assert!(alu_handled(0x13, 0x01, 1)); // shamt[5]=1 is VALID on rv64
        // SRxI reserved funct7
        assert!(!alu_handled(0x13, 0x10, 5));
        assert!(alu_handled(0x13, 0x21, 5));
        // SLLIW/SRLIW/SRAIW: imm[5] reserved
        assert!(!alu_handled(0x1b, 0x01, 1));
        assert!(!alu_handled(0x1b, 0x21, 5));
        assert!(alu_handled(0x1b, 0x20, 5));
        // FMV.D.X with rs2 != 0 (fixed field violated): f7=0x79, f3=0, rs2=1
        let bad_fmv = 0x53 | (0 << 12) | (0x79 << 25) | (1 << 20);
        assert!(!fp_handled(bad_fmv));
        let good_fmv = 0x53 | (0 << 12) | (0x79 << 25);
        assert!(fp_handled(good_fmv));
        // FSQRT.D with rs2 != 0: f7=0x2d
        let bad_sqrt = 0x53 | (0 << 12) | (0x2d << 25) | (2 << 20);
        assert!(!fp_handled(bad_sqrt));
    }

    /// The backward copy-loop detector must match musl memmove's descending
    /// loop VERBATIM (encodings lifted from the nbench musl binary's disasm).
    #[test]
    fn detects_musl_memmove_bwd_loop() {
        let words: &[u32] = &[
            0xff873583, 0xfeb6bc23, // ld a1,-8(a4);  sd a1,-8(a3)
            0xff073583, 0xfeb6b823, // ld a1,-16(a4); sd a1,-16(a3)
            0xfe873583, 0xfeb6b423, // -24
            0xfe073583, 0xfeb6b023, // -32
            0xfd873583, 0xfcb6bc23, // -40
            0xfd073583, 0xfcb6b823, // -48
            0xfc873583, 0xfcb6b423, // -56
            0xfc073883, // ld a7,-64(a4)
            0xfc068593, // addi a1,a3,-64
            0xfc070793, // addi a5,a4,-64
            0xfc060613, // addi a2,a2,-64
            0xfd16b023, // sd a7,-64(a3)
        ];
        let mut code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        code.extend_from_slice(&0x873eu16.to_le_bytes()); // c.mv a4,a5
        code.extend_from_slice(&0x86aeu16.to_le_bytes()); // c.mv a3,a1
        // bltu a6,a2, back to start: offset = -(len so far)
        let off = -(code.len() as i64);
        let imm = off as u32;
        let bltu = 0x63
            | (6 << 12)
            | (16 << 15) // rs1 = a6
            | (12 << 20) // rs2 = a2
            | (((imm >> 11) & 1) << 7)
            | (((imm >> 1) & 0xf) << 8)
            | (((imm >> 5) & 0x3f) << 25)
            | (((imm >> 12) & 1) << 31);
        code.extend_from_slice(&bltu.to_le_bytes());
        let cl = detect_copy_loop(&code, 0x1000, 0x1000);
        assert!(cl.is_some(), "bwd copy loop not detected");
        let cl = cl.unwrap();
        assert!(cl.bwd);
        assert_eq!((cl.stride, cl.w0), (64, -64));
        assert_eq!((cl.s, cl.d, cl.n, cl.l), (14, 13, 12, 16)); // a4,a3,a2,a6
        assert_eq!(cl.body_n, 22);
        assert_eq!(cl.end_pc, 0x1000 + code.len() as u64);
    }

    /// The symbolic matcher must also cover memmove's 8-byte descending tail
    /// loop and memcpy's ascending byte loop (encodings from the shipped
    /// binary; these small-move paths dominate STRING SORT's time).
    #[test]
    fn detects_tail_copy_loops() {
        // 8B bwd: ld a7,-8(a5); addi a4,a5,-8; addi a3,a1,-8; addi a2,a2,-8;
        //         sd a7,-8(a1); mv a5,a4; mv a1,a3; bltu a6,a2,start
        let words: &[u32] = &[0xff87b883, 0xff878713, 0xff858693];
        let mut code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        code.extend_from_slice(&0x1661u16.to_le_bytes()); // c.addi a2,-8
        code.extend_from_slice(&0xff15bc23u32.to_le_bytes());
        code.extend_from_slice(&0x87bau16.to_le_bytes()); // c.mv a5,a4
        code.extend_from_slice(&0x85b6u16.to_le_bytes()); // c.mv a1,a3
        let off = -(code.len() as i64);
        let imm = off as u32;
        let bltu = 0x63
            | (6 << 12)
            | (16 << 15)
            | (12 << 20)
            | (((imm >> 11) & 1) << 7)
            | (((imm >> 1) & 0xf) << 8)
            | (((imm >> 5) & 0x3f) << 25)
            | (((imm >> 12) & 1) << 31);
        code.extend_from_slice(&bltu.to_le_bytes());
        let cl = detect_copy_loop(&code, 0x1000, 0x1000).expect("8B bwd tail");
        assert!(cl.bwd);
        assert_eq!((cl.stride, cl.w0), (8, -8));
        assert_eq!((cl.s, cl.d, cl.n, cl.l), (15, 11, 12, 16)); // a5,a1,a2,a6

        // byte fwd: lbu a3,0(a1); c.addi a2,-1; c.addi a1,1; addi a4,a5,1;
        //           sb a3,0(a5); c.mv a5,a4; bnez a2,start
        let mut code: Vec<u8> = 0x0005c683u32.to_le_bytes().to_vec();
        code.extend_from_slice(&0x167du16.to_le_bytes()); // c.addi a2,-1
        code.extend_from_slice(&0x0585u16.to_le_bytes()); // c.addi a1,1
        code.extend_from_slice(&0x00178713u32.to_le_bytes());
        code.extend_from_slice(&0x00d78023u32.to_le_bytes());
        code.extend_from_slice(&0x87bau16.to_le_bytes()); // c.mv a5,a4
        // bne a2, x0 -> start
        let off = -(code.len() as i64);
        let imm = off as u32;
        let bne = 0x63
            | (1 << 12)
            | (12 << 15) // rs1 = a2
            | (0 << 20)  // rs2 = x0
            | (((imm >> 11) & 1) << 7)
            | (((imm >> 1) & 0xf) << 8)
            | (((imm >> 5) & 0x3f) << 25)
            | (((imm >> 12) & 1) << 31);
        code.extend_from_slice(&bne.to_le_bytes());
        let cl = detect_copy_loop(&code, 0x1000, 0x1000).expect("byte fwd tail");
        assert!(!cl.bwd);
        assert_eq!((cl.stride, cl.w0), (1, 0));
        assert_eq!((cl.s, cl.d, cl.n, cl.l), (11, 15, 12, 0)); // a1,a5,a2,x0
    }

    /// Fuzz the FMADD fast-path twin against the softfloat oracle: every
    /// input where the fast path produces a result must be bit-identical to
    /// sf64::fma under RNE. Also reports (via the pass counter assert) that
    /// the fast path actually fires on a meaningful share of libm-like values.
    #[test]
    fn fma_fastpath_matches_softfp() {
        use rv64_core::softfp::sf64;
        let mut state = 0x243f_6a88_85a3_08d3u64;
        let mut rnd = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut checked = 0u64;
        let mut passed = 0u64;
        let mut check = |ab: u64, bb: u64, cb: u64| {
            if let Some(r) = fma_fastpath_ref(ab, bb, cb) {
                let mut fl = 0u32;
                let want = sf64::fma(ab, bb, cb, 0, &mut fl);
                assert_eq!(
                    r, want,
                    "fma mismatch a={ab:#x} b={bb:#x} c={cb:#x}: fast={r:#x} soft={want:#x}"
                );
                1u64
            } else {
                0
            }
        };
        // zeros in every position, against normal and zero partners — the
        // fast path accepts exact zeros (see the band check), and IEEE's
        // signed-zero rules for a*b+c are exactly where a wrong allowance
        // would show up.
        {
            let zeros = [0u64, 1u64 << 63];
            let vals = [
                0u64,
                1u64 << 63,
                1.0f64.to_bits(),
                (-1.0f64).to_bits(),
                2.5f64.to_bits(),
                (-0.75f64).to_bits(),
            ];
            for &z in &zeros {
                for &x in &vals {
                    for &y in &vals {
                        checked += 3;
                        passed += check(z, x, y);
                        passed += check(x, z, y);
                        passed += check(x, y, z);
                    }
                }
            }
        }
        // libm-like: values near 1.0 (exponents 1023 +/- 40), all sign mixes
        let mark0 = (checked, passed);
        for _ in 0..2_000_000 {
            let m = |r: u64| {
                let mant = r & 0xf_ffff_ffff_ffff;
                let e = 1023i64 + ((r >> 52) as i64 % 81) - 40;
                let sgn = (r >> 63) << 63;
                sgn | ((e as u64) << 52) | mant
            };
            let (x, y, z) = (m(rnd()), m(rnd()), m(rnd()));
            checked += 1;
            passed += check(x, y, z);
        }
        let libm_rate = (passed - mark0.1) * 100 / (checked - mark0.0);
        // near-cancellation: c ~= -(a*b)
        let mark1 = (checked, passed);
        for _ in 0..500_000 {
            let m = |r: u64| {
                let mant = r & 0xf_ffff_ffff_ffff;
                (1023u64 << 52) | mant
            };
            let (x, y) = (m(rnd()), m(rnd()));
            let prod = f64::from_bits(x) * f64::from_bits(y);
            let cb = (-prod).to_bits() ^ (rnd() & 3); // c near -(a*b), jiggled ulps
            checked += 1;
            passed += check(x, y, cb);
        }
        let cancel_rate = (passed - mark1.1) * 100 / (checked - mark1.0);
        // fully random bit patterns (mostly bail; must never MIS-match)
        for _ in 0..2_000_000 {
            checked += 1;
            passed += check(rnd(), rnd(), rnd());
        }
        println!("hit rates: libm-like {libm_rate}%, near-cancel {cancel_rate}%, total {}%", passed * 100 / checked);
        // the fast path must be worth emitting: solid hit rate on libm-like values
        assert!(libm_rate >= 90, "libm-like hit rate too low: {libm_rate}%");
    }

    // sum 1..10 program from the core tests
    const PROG: [u32; 7] = [
        0x00000093, 0x00100113, 0x00b00193, 0x002080b3, 0x00110113, 0xfe311ce3, 0x00000073,
    ];

    fn code_bytes() -> Vec<u8> {
        PROG.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn branch(f3: u32, rs1: u32, rs2: u32, off: i32) -> u32 {
        let imm = off as u32;
        0x63
            | (f3 << 12)
            | (rs1 << 15)
            | (rs2 << 20)
            | (((imm >> 11) & 1) << 7)
            | (((imm >> 1) & 0xf) << 8)
            | (((imm >> 5) & 0x3f) << 25)
            | (((imm >> 12) & 1) << 31)
    }

    fn jal(rd: u32, off: i32) -> u32 {
        let imm = off as u32;
        0x6f
            | (rd << 7)
            | (((imm >> 12) & 0xff) << 12)
            | (((imm >> 11) & 1) << 20)
            | (((imm >> 1) & 0x3ff) << 21)
            | (((imm >> 20) & 1) << 31)
    }

    #[test]
    fn multi_latch_falls_back_to_ordinary_loop() {
        let code = code_bytes();
        let mut ordinary = JitLayout::bare();
        ordinary.mem = Some((0, 0x2000));
        let expected = loop_region(&code, 0x1000, 0x100c, &ordinary).unwrap();

        let mut enabled = ordinary;
        enabled.multi_latch = true;
        let actual = loop_region(&code, 0x1000, 0x100c, &enabled).unwrap();
        assert_eq!(actual.end_pc, expected.end_pc);
        assert_eq!(actual.loops, expected.loops);
        assert!(!actual.unconditional_latch);
        assert_eq!(translate_block(&code, 0x1000, 0x100c, enabled).unwrap().n_insns, 3);
    }

    #[test]
    fn detects_ld_lhu_multi_latch_loop() {
        // ld t0,0(a0); bnez t0,start; lhu t1,0(a1);
        // bnez t1,start; j start
        let words = [
            0x03 | (5 << 7) | (3 << 12) | (10 << 15),
            branch(1, 5, 0, -4),
            0x03 | (6 << 7) | (5 << 12) | (11 << 15),
            branch(1, 6, 0, -12),
            jal(0, -16),
        ];
        let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut lay = JitLayout::bare();
        lay.mem = Some((0, 0x2000));

        let ordinary = loop_region(&code, 0, 0, &lay).unwrap();
        assert_eq!(ordinary.end_pc, 8);
        assert!(!ordinary.unconditional_latch);

        lay.multi_latch = true;
        let extended = loop_region(&code, 0, 0, &lay).unwrap();
        assert_eq!(extended.end_pc, 20);
        assert!(extended.unconditional_latch);
        assert_eq!(extended.loops, vec![(0, 20)]);
        assert_eq!(translate_block(&code, 0, 0, lay).unwrap().n_insns, 5);
    }

    #[test]
    fn translates_leading_block() {
        // Block 1: three addis then falls into the loop body... the block
        // actually extends through the branch (bne terminates it).
        let b = translate_block(&code_bytes(), 0x1000, 0x1000, JitLayout::bare()).unwrap();
        assert_eq!(b.n_insns, 6); // addi,addi,addi,add,addi,bne
        assert!(b.wasm.starts_with(&[0x00, 0x61, 0x73, 0x6d])); // \0asm
    }

    #[test]
    fn loop_body_block() {
        let b = translate_block(&code_bytes(), 0x1000, 0x100c, JitLayout::bare()).unwrap();
        assert_eq!(b.n_insns, 3); // add, addi, bne
    }

    #[test]
    fn ecall_not_translatable() {
        assert!(translate_block(&code_bytes(), 0x1000, 0x1018, JitLayout::bare()).is_none());
    }

    #[test]
    fn compressed_input_translates() {
        // c.li a0, 21 ; c.mv a1, a0 ; c.add a0, a1 ; ecall(32-bit)
        let mut code = Vec::new();
        for h in [0x4555u16, 0x85aa, 0x952e] {
            code.extend_from_slice(&h.to_le_bytes());
        }
        code.extend_from_slice(&0x0000_0073u32.to_le_bytes());
        let b = translate_block(&code, 0, 0, JitLayout::bare()).unwrap();
        assert_eq!(b.n_insns, 3);
        assert_eq!(b.len, 6);
    }
}


/// One member of a compiled batch: its entry pc and how many instructions
/// its body retires (the host caches these like ordinary blocks).
pub struct BatchMember {
    pub pc: u64,
    pub n_insns: u32,
    pub span: (u64, u64),
    pub uses_fp: bool,
    pub seeds: Vec<u64>,
}

/// Compile a hot pc AND its fixed-target successors as ONE module whose
/// members transfer by direct tail call (~2ns), with no table import and
/// therefore none of the O(importing instances) table.set cost that killed
/// every earlier chaining design. Discovery is breadth-first over each
/// trace's own exit seeds, bounded by `cap`; only pcs the caller's `want`
/// predicate accepts (in-window, not already compiled, ...) join.
///
/// Returns the module bytes plus one BatchMember per exported body, in
/// export order — member j is export "r{j}" and table index base + j.
pub fn translate_batch(
    code: &[u8],
    base: u64,
    start_pc: u64,
    lay: JitLayout,
    hot: &dyn Fn(u64) -> bool,
    want: &dyn Fn(u64) -> bool,
    cap: usize,
) -> Option<(Vec<u8>, Vec<BatchMember>)> {
    translate_batch_obs(code, base, start_pc, lay, hot, want, &|_| None, cap)
}

/// translate_batch with an OBSERVED-SUCCESSOR oracle: `next(pc)` is the pc
/// execution was last seen to take after `pc`. Members are discovered along
/// that chain FIRST (the next-executing-tail of trace-tree JITs) and only
/// then from static exit seeds — a batch built from where control actually
/// goes keeps far more exits inside the module than one built from where
/// the instruction stream textually leads.
#[allow(clippy::too_many_arguments)]
pub fn translate_batch_obs(
    code: &[u8],
    base: u64,
    start_pc: u64,
    lay: JitLayout,
    hot: &dyn Fn(u64) -> bool,
    want: &dyn Fn(u64) -> bool,
    next: &dyn Fn(u64) -> Option<u64>,
    cap: usize,
) -> Option<(Vec<u8>, Vec<BatchMember>)> {
    // Pass 1 (no links): discover the member set breadth-first from the
    // seed pc's exits. Bodies are re-emitted in pass 2 once every member's
    // index is known, so a link can name a member discovered after it.
    RAW_BODY.store(true, std::sync::atomic::Ordering::Relaxed);
    let no_link = |_: u64| None;
    let mut pcs: Vec<u64> = vec![start_pc];
    let mut probed = 0usize;
    // A loop header keeps its structured region: never pull one into a
    // batch as a plain trace.
    let is_loop_hdr = |t: u64| is_loop_at(code, base, t, lay);
    while probed < pcs.len() && pcs.len() < cap {
        let p = pcs[probed];
        probed += 1;
        // Members are compiled WITH the inline cache oracle: a batch member
        // that still ended at every indirect jump would defeat the point —
        // the two mechanisms compose (IC extends a member through an edge,
        // links carry the exits that remain to co-members).
        let Some(b) = translate_block_ic(code, base, p, lay, hot, &no_link, next) else {
            continue;
        };
        // Observed successor first: it is where execution goes, so the link
        // that matters most is the one that reaches it.
        if let Some(nx) = next(p) {
            if pcs.len() < cap && !pcs.contains(&nx) && want(nx) && !is_loop_hdr(nx) {
                pcs.push(nx);
            }
        }
        for sd in b.seeds {
            if pcs.len() >= cap {
                break;
            }
            if !pcs.contains(&sd) && want(sd) && !is_loop_hdr(sd) {
                pcs.push(sd);
            }
        }
    }
    // Pass 2: emit every member with links resolved against the final set.
    let index_of = |t: u64| pcs.iter().position(|&q| q == t).map(|k| k as u32);
    let mut bodies: Vec<(Vec<u8>, u32, u32)> = Vec::new();
    let mut members: Vec<BatchMember> = Vec::new();
    let mut ok = true;
    for &p in &pcs {
        match translate_block_ic(code, base, p, lay, hot, &index_of, next) {
            Some(b) => {
                bodies.push((b.wasm, b.locals.0, b.locals.1));
                members.push(BatchMember {
                    pc: p,
                    n_insns: b.n_insns,
                    span: b.span,
                    uses_fp: b.uses_fp,
                    seeds: b.seeds,
                });
            }
            None => {
                ok = false;
                break;
            }
        }
    }
    RAW_BODY.store(false, std::sync::atomic::Ordering::Relaxed);
    if !ok || bodies.is_empty() {
        return None;
    }
    Some((wasm_emit::finish_batch(bodies), members))
}
