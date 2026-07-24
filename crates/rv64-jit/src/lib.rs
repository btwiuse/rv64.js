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
/// Loop-iteration counter (Phase 3 self-loop compilation).
const ITER: u32 = 5;
/// Total i64 scratch locals to declare (register locals follow at 6+; the
/// i32 IDXB local follows all i64 locals, so its index is dynamic).
const N_I64_LOCALS: u32 = 5;

/// Full-system memory access layout: emitted loads/stores probe the
/// interpreter's own Load/Store TLBs inline; on a hit within guest RAM
/// they access memory directly, otherwise they bail to the interpreter
/// (which walks the page table, fills the TLB, and handles MMIO/faults).
#[derive(Clone, Copy)]
pub struct SysMem {
    /// TLB rows (tag then pa-va diff), Cpu::jit_tlb_ptrs() order.
    pub tlb_load_tag: u32,
    pub tlb_load_diff: u32,
    pub tlb_store_tag: u32,
    pub tlb_store_diff: u32,
    /// Index mask: jit_tlb_size() - 1.
    pub tlb_mask: u32,
    /// Guest RAM: linear offset of ram[0], guest-physical base, byte size.
    pub ram_off: u32,
    pub ram_base: u64,
    pub ram_size: u64,
    /// Compiled-code page bitset (u64 words, bit set = page holds JIT
    /// code): stores into such pages bail so the interpreter records the
    /// dirty page for invalidation.
    pub jit_pages_off: u32,
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
    /// Cell that every block writes with the number of guest instructions
    /// it actually retired before returning. Sys blocks with inline memory
    /// ops can bail mid-block (TLB miss / MMIO), so the dispatcher must read
    /// this rather than assume the full block length.
    pub retired_addr: u32,
    /// Linear-memory offset of f[0] (FP register file; f1.. at 8-byte stride)
    /// and of the fcsr slot. Both 0 disables FP-in-block translation.
    pub f_base: u32,
    pub fcsr_addr: u32,
}

impl JitLayout {
    /// Layout used by the standalone tests: x at 0, pc at 256, no memory.
    pub fn bare() -> JitLayout {
        JitLayout {
            x_base: 0,
            pc_addr: 256,
            mem: None,
            sys: None,
            retired_addr: 264,
            f_base: 0,
            fcsr_addr: 0,
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
}

impl Ctx {
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
        if self.reg_local[rd] != 0 {
            m.local_set(self.reg_local[rd]);
        } else {
            m.i64_store(self.lay.x_base as u64 + rd as u64 * 8);
        }
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
        let mut w = self.write_mask;
        while w != 0 {
            let r = w.trailing_zeros() as usize;
            w &= w - 1;
            if self.reg_local[r] != 0 {
                m.i32_const(0)
                    .local_get(self.reg_local[r])
                    .i64_store(self.lay.x_base as u64 + r as u64 * 8);
            }
        }
        let mut w = self.fp_write_mask;
        while w != 0 {
            let r = w.trailing_zeros() as usize;
            w &= w - 1;
            if self.fp_local[r] != 0 {
                m.i32_const(0)
                    .local_get(self.fp_local[r])
                    .i64_store(self.lay.f_base as u64 + r as u64 * 8);
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
    fn fp_arith_d(&self, m: &mut WasmModule, op: u32, s1: usize, s2: usize, d: usize, dyn_rm: bool, pc: u64, n: u32) {
        let fcsr = self.lay.fcsr_addr as u64;
        // Eligibility: bail if NX not set (fcsr&1==0) — or, for dynamic rm,
        // if frm != RNE ((fcsr>>5)&7 != 0).
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
        // Bail unless the result is a normal number: exp in [1, 0x7fe], i.e.
        // (exp - 1) <=u 0x7fd. Catches inf/nan (0x7ff) and subnormal/zero (0).
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

    /// Write the retired-instruction count for this block exit.
    fn set_retired(&self, m: &mut WasmModule, n: u32) {
        m.i32_const(0)
            .i64_const(n as i64)
            .i64_store(self.lay.retired_addr as u64);
    }

    /// Bail out of the block at instruction index `n` (retired so far),
    /// leaving pc at `pc` for the interpreter to resume.
    fn bail(&self, m: &mut WasmModule, pc: u64, n: u32) {
        self.flush_writes(m);
        self.set_pc_const(m, pc);
        self.set_retired(m, n);
        m.op(RETURN);
    }

    /// Emit an inline-TLB probe. `addr` (i64 va) must be on the stack.
    /// On a hit that lands in guest RAM, leaves the i32 linear-memory index
    /// on the stack and continues. On a miss (or MMIO / RAM-crossing), sets
    /// VA-side state and jumps to `bail` — i.e. emits `if miss { bail }`.
    /// `store` selects the store TLB and adds the compiled-code-page check.
    fn tlb_index(&self, m: &mut WasmModule, sys: &SysMem, len: u64, store: bool, pc: u64, n: u32) {
        let (tag_base, diff_base) = if store {
            (sys.tlb_store_tag, sys.tlb_store_diff)
        } else {
            (sys.tlb_load_tag, sys.tlb_load_diff)
        };
        m.local_set(VA);
        // page-crossing guard: if (va & 0xfff) > 0x1000 - len, the access
        // spans two pages whose physical mappings need not be contiguous —
        // bail so the interpreter splits it.
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
        // IDXB (i32) = ((page & mask) << 3)
        m.local_get(PAGE)
            .op(I32_WRAP_I64)
            .i32_const(sys.tlb_mask as i32)
            .op(I32_AND)
            .i32_const(3)
            .op(I32_SHL)
            .local_set_i32(self.idxb);
        // miss if tlb_tag[idx] != page  -> bail
        m.local_get_i32(self.idxb).i64_load_at(tag_base as u64);
        m.local_get(PAGE).op(I64_NE);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        // PA = va + diff
        m.local_get(VA);
        m.local_get_i32(self.idxb).i64_load_at(diff_base as u64);
        m.op(I64_ADD).local_set(PA);
        // range check: (pa - ram_base) >u ram_size - len  -> bail (MMIO/cross)
        m.local_get(PA)
            .i64_const(sys.ram_base as i64)
            .op(I64_SUB)
            .i64_const((sys.ram_size - len) as i64)
            .op(I64_GT_U);
        m.op(IF).op(VOID);
        self.bail(m, pc, n);
        m.op(END);
        if store {
            // bail if the target physical page holds compiled code:
            // ppage = (pa - ram_base) >> 12; word = jit_pages[ppage>>6];
            // if (word >> (ppage & 63)) & 1 -> bail
            m.local_get(PA)
                .i64_const(sys.ram_base as i64)
                .op(I64_SUB)
                .i64_const(12)
                .op(I64_SHR_U)
                .local_set(PAGE); // PAGE now = ppage
                                  // word address = jit_pages_off + (ppage >> 6) * 8
            m.local_get(PAGE)
                .i64_const(6)
                .op(I64_SHR_U)
                .op(I32_WRAP_I64)
                .i32_const(3)
                .op(I32_SHL)
                .i64_load_at(sys.jit_pages_off as u64);
            // >> (ppage & 63)
            m.local_get(PAGE).i64_const(63).op(I64_AND).op(I64_SHR_U);
            m.i64_const(1).op(I64_AND);
            m.i64_const(0).op(I64_NE);
            m.op(IF).op(VOID);
            self.bail(m, pc, n);
            m.op(END);
        }
        // linear index = ram_off + (pa - ram_base)   (i32)
        m.local_get(PA)
            .i64_const(sys.ram_base as i64)
            .op(I64_SUB)
            .op(I32_WRAP_I64)
            .i32_const(sys.ram_off as i32)
            .op(I32_ADD);
    }
}

/// Pre-scan a block — walking and terminating exactly like `translate_block`
/// — to collect which guest registers it reads and writes, as 32-bit bitmaps.
/// Used to decide which registers to cache in wasm locals.
/// Returns (gpr_read, gpr_write, fp_read, fp_write) register bitmaps.
fn scan_regs(code: &[u8], base: u64, start_pc: u64, lay: &JitLayout) -> (u32, u32, u32, u32) {
    let (mut read, mut write) = (0u32, 0u32);
    let (mut fread, mut fwrite) = (0u32, 0u32);
    let mut pc = start_pc;
    let mut n = 0u32;
    // FP registers: f0 is a real register (no hardwired-zero), so mark it too.
    let fmark = |m: &mut u32, r: usize| *m |= 1 << r;
    let mark = |m: &mut u32, r: usize| {
        if r != 0 {
            *m |= 1 << r;
        }
    };
    while n < MAX_BLOCK as u32 {
        let Some((insn, ilen)) = fetch(code, base, pc) else {
            break;
        };
        let next_pc = pc.wrapping_add(ilen);
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
        match opcode(insn) {
            0x37 | 0x17 => mark(&mut write, d),
            0x13 => {
                mark(&mut read, s1);
                mark(&mut write, d);
            }
            0x33 => {
                if !matches!(
                    (funct7(insn), funct3(insn)),
                    (0x00, _) | (0x20, 0) | (0x20, 5) | (0x01, 0)
                ) {
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
                if !matches!((funct7(insn), funct3(insn)), (0x00, 0) | (0x20, 0) | (0x01, 0)) {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut write, d);
            }
            0x03 if lay.mem.is_some() || lay.sys.is_some() => {
                if funct3(insn) == 7 {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut write, d);
            }
            0x23 if lay.mem.is_some() || lay.sys.is_some() => {
                if funct3(insn) > 3 {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut read, s2);
            }
            // FLD / FSD (double, funct3==3): raw 8-byte copy mem<->f[]. User-
            // mode only (needs f_base for the FP file; loop-opt is user-mode).
            0x07 if lay.mem.is_some() && lay.f_base != 0 => {
                if funct3(insn) != 3 {
                    break;
                }
                mark(&mut read, s1);
                fmark(&mut fwrite, d);
            }
            0x27 if lay.mem.is_some() && lay.f_base != 0 => {
                if funct3(insn) != 3 {
                    break;
                }
                mark(&mut read, s1);
                fmark(&mut fread, s2);
            }
            0x6f => {
                mark(&mut write, d);
                let target = pc.wrapping_add(imm_j(insn) as u64);
                let in_window = target > pc && target >= base && target < base + code.len() as u64;
                if d == 0 && in_window {
                    pc = target;
                    n += 1;
                    continue;
                }
                break;
            }
            0x67 => {
                mark(&mut read, s1);
                mark(&mut write, d);
                break;
            }
            0x63 => {
                if !matches!(funct3(insn), 0 | 1 | 4 | 5 | 6 | 7) {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut read, s2);
                break;
            }
            // OP-FP (mirror translate_block): FP arith touches no GPRs;
            // FMV.D.X reads a GPR, FMV.X.D writes one; others end the block.
            0x53 if lay.f_base != 0 => {
                let f7 = funct7(insn);
                match (f7 >> 2, f7 & 3, funct3(insn)) {
                    (0..=3, 1, 0 | 7) => {
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
                    _ => break,
                }
            }
            _ => break,
        }
        pc = next_pc;
        n += 1;
    }
    (read, write, fread, fwrite)
}

/// Is `f7`/`f3` a FP op the JIT emits inline (arith / compare / FMV)?
fn fp_handled(f7: u32, f3: u32) -> bool {
    matches!(
        (f7 >> 2, f7 & 3, f3),
        (0..=3, 1, 0 | 7) | (0x14, 1, 0..=2) | (0x1e, 1, 0) | (0x1c, 1, 0)
    )
}

/// Is `f7`/`f3` a supported OP / OP-32 / OP-IMM-32 encoding?
fn alu_handled(op: u32, f7: u32, f3: u32) -> bool {
    match op {
        0x37 | 0x17 | 0x13 => true,
        0x33 => matches!((f7, f3), (0x00, _) | (0x20, 0) | (0x20, 5) | (0x01, 0)),
        0x1b => matches!(f3, 0 | 1 | 5),
        0x3b => matches!((f7, f3), (0x00, 0) | (0x20, 0) | (0x01, 0)),
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
}

/// Detect and fully validate a structured loop region at `start_pc` (which must
/// be a natural-loop header — the target of a backward branch). User-mode only:
/// inline memory ops here only TRAP on fault (never bail mid-loop), and the FP
/// register file is present. Returns None for anything not provably structured
/// (the caller then compiles a plain basic block).
fn loop_region(code: &[u8], base: u64, start_pc: u64, lay: &JitLayout) -> Option<LoopRegion> {
    lay.mem?; // user-mode direct memory only
    // Pass A: linear walk to the back-edge that closes the outermost loop,
    // collecting every conditional branch. Every instruction must be handled.
    let mut branches: Vec<(u64, u64, u64)> = Vec::new(); // (pc, target, next)
    let mut end_pc = None;
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
                if !fp_handled(funct7(insn), funct3(insn)) {
                    return None;
                }
            }
            0x03 => {
                if funct3(insn) == 7 {
                    return None;
                }
            }
            0x23 => {
                if funct3(insn) > 3 {
                    return None;
                }
            }
            0x07 | 0x27 if lay.f_base != 0 => {
                if funct3(insn) != 3 {
                    return None;
                }
            }
            0x63 => {
                if !matches!(funct3(insn), 0 | 1 | 4 | 5 | 6 | 7) {
                    return None;
                }
                let t = pc.wrapping_add(imm_b(insn) as u64);
                branches.push((pc, t, next));
                if t == start_pc {
                    end_pc = Some(next);
                    break;
                }
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
        if t < bpc {
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
    Some(LoopRegion { end_pc, loops })
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
            }
            0x23 => {
                mark(&mut read, s1);
                mark(&mut read, s2);
            }
            0x07 => {
                mark(&mut read, s1);
                fmark(&mut fwrite, d);
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
                    _ => {}
                }
            }
            0x63 => {
                mark(&mut read, s1);
                mark(&mut read, s2);
            }
            _ => {}
        }
        pc = pc.wrapping_add(ilen);
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
    let c = Ctx {
        lay,
        reg_local,
        write_mask,
        idxb: N_I64_LOCALS + n_reg + n_fp + 1, // i32 local after all i64 locals
        fp_local,
        fp_write_mask: fp_write,
    };
    let mut m = WasmModule::with_locals(N_I64_LOCALS + n_reg + n_fp, 1);
    let mut t = touched;
    while t != 0 {
        let r = t.trailing_zeros() as usize;
        t &= t - 1;
        m.i32_const(0)
            .i64_load(lay.x_base as u64 + r as u64 * 8)
            .local_set(reg_local[r]);
    }
    let mut t = fp_touched;
    while t != 0 {
        let r = t.trailing_zeros() as usize;
        t &= t - 1;
        m.i32_const(0)
            .i64_load(lay.f_base as u64 + r as u64 * 8)
            .local_set(fp_local[r]);
    }
    (c, m)
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
        // OP (I and M-mul only; div falls back)
        0x33 => {
            let f7 = funct7(insn);
            let f3 = funct3(insn);
            let supported = matches!((f7, f3), (0x00, _) | (0x20, 0) | (0x20, 5) | (0x01, 0));
            if !supported {
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
        // OP-32 (ADDW/SUBW/MULW)
        0x3b => {
            let (f7, f3) = (funct7(insn), funct3(insn));
            if !matches!((f7, f3), (0x00, 0) | (0x20, 0) | (0x01, 0)) {
                return false;
            }
            if c.store_pre(m, d) {
                c.push_reg(m, s1);
                c.push_reg(m, s2);
                m.op(match (f7, f3) {
                    (0x00, 0) => I64_ADD,
                    (0x20, 0) => I64_SUB,
                    _ => I64_MUL,
                });
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
        0x07 if lay.mem.is_some() && lay.f_base != 0 => {
            if funct3(insn) != 3 {
                return false;
            }
            let (mem_base, size) = lay.mem.unwrap();
            c.push_reg(m, s1);
            m.i64_const(imm_i(insn)).op(I64_ADD);
            c.guest_addr(m, size, 8);
            m.op(I64_LOAD).raw_uleb(len_align(8)).raw_uleb(mem_base as u64);
            m.local_set(VAL);
            c.store_freg_pre(m, d);
            m.local_get(VAL);
            c.store_freg_post(m, d);
        }
        // FSD: mem[x[s1]+imm] = f[s2] (double). Raw 8-byte copy.
        0x27 if lay.mem.is_some() && lay.f_base != 0 => {
            if funct3(insn) != 3 {
                return false;
            }
            let (mem_base, size) = lay.mem.unwrap();
            c.push_reg(m, s1);
            m.i64_const(imm_s(insn)).op(I64_ADD);
            c.guest_addr(m, size, 8);
            c.push_freg(m, s2);
            m.op(I64_STORE).raw_uleb(len_align(8)).raw_uleb(mem_base as u64);
        }
        // OP-FP: double add/sub/mul/div + compares + FMV.D.X/FMV.X.D inline.
        0x53 if lay.f_base != 0 => {
            let f7 = funct7(insn);
            let (fmt, fpop, f3) = (f7 & 3, f7 >> 2, funct3(insn));
            match (fpop, fmt, f3) {
                (0..=3, 1, 0 | 7) => c.fp_arith_d(m, fpop, s1, s2, d, f3 == 7, pc, n),
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
                _ => return false,
            }
        }
        _ => return false,
    }
    true
}

/// Translate a block starting at `start_pc`. `code` is the guest code bytes and
/// `base` its guest address. Returns None if the first instruction isn't
/// translatable (caller interprets it instead).
pub fn translate_block(code: &[u8], base: u64, start_pc: u64, lay: JitLayout) -> Option<Block> {
    // Structured loop region (nested loops + forward if-then/break) → compile
    // the whole thing as one wasm function so register locals persist across
    // every iteration of every level (3e-2 / v86 control-flow structuring).
    if lay.mem.is_some() {
        if let Some(region) = loop_region(code, base, start_pc, &lay) {
            let (rm, wm, fr, fw) = scan_regs_region(code, base, start_pc, region.end_pc, &lay);
            let (c, m) = build_ctx(lay, rm, wm, fr, fw);
            if let Some(b) = translate_loop(m, &c, code, base, start_pc, &region, &lay) {
                return Some(b);
            }
        }
    }

    // Basic-block path: a straight-line run to the first branch/jump/unhandled
    // op. Registers the block touches live in wasm locals for its lifetime.
    let (read_mask, write_mask, fp_read, fp_write) = scan_regs(code, base, start_pc, &lay);
    let (c, mut m) = build_ctx(lay, read_mask, write_mask, fp_read, fp_write);

    let mut pc = start_pc;
    let mut n = 0u32;
    while n < MAX_BLOCK as u32 {
        let Some((insn, ilen)) = fetch(code, base, pc) else {
            break;
        };
        let next_pc = pc.wrapping_add(ilen);
        let op = opcode(insn);
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));

        if emit_simple(&mut m, &c, lay, insn, pc, n) {
            pc = next_pc;
            n += 1;
            continue;
        }

        match op {
            // JAL: link; follow plain forward jumps (superblock chaining),
            // otherwise end the block with a constant pc.
            0x6f => {
                let target = pc.wrapping_add(imm_j(insn) as u64);
                if c.store_pre(&mut m, d) {
                    m.i64_const(next_pc as i64);
                    c.store_post(&mut m, d);
                }
                let in_window = target > pc && target >= base && target < base + code.len() as u64;
                if d == 0 && in_window {
                    pc = target;
                    n += 1;
                    continue;
                }
                c.flush_writes(&mut m);
                c.set_pc_const(&mut m, target);
                c.set_retired(&mut m, n + 1);
                return Some(Block {
                    wasm: m.finish(),
                    len: next_pc - start_pc,
                    n_insns: n + 1,
                });
            }
            // JALR: dynamic target; block ends.
            0x67 => {
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
                m.i32_const(0).local_get(SCR).i64_store(lay.pc_addr as u64);
                c.flush_writes(&mut m);
                c.set_retired(&mut m, n + 1);
                return Some(Block {
                    wasm: m.finish(),
                    len: next_pc - start_pc,
                    n_insns: n + 1,
                });
            }
            // BRANCH: conditional pc select; block ends.
            0x63 => {
                let target = pc.wrapping_add(imm_b(insn) as u64);
                let cmp = match funct3(insn) {
                    0 => I64_EQ,
                    1 => I64_NE,
                    4 => I64_LT_S,
                    5 => I64_GE_S,
                    6 => I64_LT_U,
                    7 => I64_GE_U,
                    _ => break,
                };
                c.push_reg(&mut m, s1);
                c.push_reg(&mut m, s2);
                m.op(cmp);
                m.op(IF).op(VOID);
                c.set_pc_const(&mut m, target);
                m.op(ELSE);
                c.set_pc_const(&mut m, next_pc);
                m.op(END);
                c.flush_writes(&mut m);
                c.set_retired(&mut m, n + 1);
                return Some(Block {
                    wasm: m.finish(),
                    len: next_pc - start_pc,
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
    Some(Block {
        wasm: m.finish(),
        len: pc - start_pc,
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
            // Cap guard at the loop top — a safe yield point: resume at header
            // with registers flushed (no partial iteration state to lose).
            m.local_get(ITER).i64_const(LOOP_CAP as i64).op(I64_GE_U);
            m.op(IF).op(VOID);
            c.flush_writes(&mut m);
            c.set_pc_const(&mut m, h);
            m.i32_const(0).local_get(ITER).i64_store(lay.retired_addr as u64);
            m.op(RETURN);
            m.op(END);
        }
        if pc == region.end_pc {
            break;
        }
        let (insn, ilen) = fetch(code, base, pc)?;
        let next = pc.wrapping_add(ilen);
        if opcode(insn) != 0x63 {
            if !emit_simple(&mut m, c, *lay, insn, pc, static_n) {
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
    m.i32_const(0).local_get(ITER).i64_store(lay.retired_addr as u64);
    Some(Block {
        wasm: m.finish(),
        len: region.end_pc - start_pc,
        n_insns: static_n.max(1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // sum 1..10 program from the core tests
    const PROG: [u32; 7] = [
        0x00000093, 0x00100113, 0x00b00193, 0x002080b3, 0x00110113, 0xfe311ce3, 0x00000073,
    ];

    fn code_bytes() -> Vec<u8> {
        PROG.iter().flat_map(|w| w.to_le_bytes()).collect()
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
