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

    /// Flush every block-written register local back to the CPU state struct.
    /// Precedes every block exit and mid-block bail so the interpreter (which
    /// reads registers from state) sees current values.
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
        let f = self.lay.f_base as u64;
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
        m.i32_const(0).i64_load(f + s1 as u64 * 8).op(F64_REINTERPRET_I64);
        m.i32_const(0).i64_load(f + s2 as u64 * 8).op(F64_REINTERPRET_I64);
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
        m.i32_const(0).local_get(VAL).i64_store(f + d as u64 * 8);
    }

    /// Emit a double-precision FP compare (FLE/FLT/FEQ.D) as an inline wasm
    /// f64 compare into GPR x[d]. `f3`: 0=FLE 1=FLT 2=FEQ. Bails to the
    /// interpreter if either operand is inf/nan (the exact-flag/NV cases);
    /// finite operands compare exactly with no flag change.
    fn fp_cmp_d(&self, m: &mut WasmModule, f3: u32, s1: usize, s2: usize, d: usize, pc: u64, n: u32) {
        let f = self.lay.f_base as u64;
        for &s in &[s1, s2] {
            m.i32_const(0)
                .i64_load(f + s as u64 * 8)
                .i64_const(52)
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
            m.i32_const(0).i64_load(f + s1 as u64 * 8).op(F64_REINTERPRET_I64);
            m.i32_const(0).i64_load(f + s2 as u64 * 8).op(F64_REINTERPRET_I64);
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
fn scan_regs(code: &[u8], base: u64, start_pc: u64, lay: &JitLayout) -> (u32, u32) {
    let (mut read, mut write) = (0u32, 0u32);
    let mut pc = start_pc;
    let mut n = 0u32;
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
                    (0..=3, 1, 0 | 7) => {}
                    (0x14, 1, 0..=2) => mark(&mut write, d), // FLE/FLT/FEQ -> x[d]
                    (0x1e, 1, 0) => mark(&mut read, s1),
                    (0x1c, 1, 0) => mark(&mut write, d),
                    _ => break,
                }
            }
            _ => break,
        }
        pc = next_pc;
        n += 1;
    }
    (read, write)
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

/// If the block at `start_pc` is a *structured* self-loop — a body of handled
/// ALU/FP ops plus properly-nested forward conditional branches (if-then, no
/// else, no other backward branches or jumps), ending in a conditional branch
/// back to `start_pc` — return the body's instruction count. Such a loop is
/// compiled into a single wasm `loop` with nested `if` blocks (Phase 3 / 3d-2)
/// so registers stay in locals across iterations and there is no per-iteration
/// dispatch. Anything unstructured returns None (compiled as basic blocks).
fn detect_structured_loop(code: &[u8], base: u64, start_pc: u64, lay: &JitLayout) -> Option<u32> {
    let mut pc = start_pc;
    let mut n = 0u32;
    let mut if_stack: Vec<u64> = Vec::new(); // pending forward-if end targets
    while n < MAX_BLOCK as u32 {
        while if_stack.last() == Some(&pc) {
            if_stack.pop();
        }
        let (insn, ilen) = fetch(code, base, pc)?;
        let op = opcode(insn);
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
            0x63 => {
                if !matches!(funct3(insn), 0 | 1 | 4 | 5 | 6 | 7) {
                    return None;
                }
                let target = pc.wrapping_add(imm_b(insn) as u64);
                if target == start_pc {
                    // loop back-edge: only valid at the outermost level, and it
                    // is the last instruction of the loop body.
                    return if if_stack.is_empty() && n > 0 {
                        Some(n + 1)
                    } else {
                        None
                    };
                } else if target > pc {
                    // forward if-then; must nest within the enclosing if.
                    if let Some(&top) = if_stack.last() {
                        if target > top {
                            return None;
                        }
                    }
                    if_stack.push(target);
                } else {
                    return None; // backward branch not to start = unstructured
                }
            }
            _ => return None,
        }
        pc = pc.wrapping_add(ilen);
        n += 1;
    }
    None
}

/// Translate one basic block starting at `pc`. `code` is the guest code
/// bytes and `base` its guest address. Returns None if the very first
/// instruction isn't translatable (caller interprets it instead).
pub fn translate_block(code: &[u8], base: u64, start_pc: u64, lay: JitLayout) -> Option<Block> {
    // Pass 1: find which guest registers the block reads/writes, so we can
    // cache them in wasm locals (v86-style register allocation) instead of
    // round-tripping every access through the CPU state struct in memory.
    let (read_mask, write_mask) = scan_regs(code, base, start_pc, &lay);
    let touched = read_mask | write_mask;
    // Assign an i64 local (index 5, 6, ...) to each touched register.
    let mut reg_local = [0u32; 32];
    let mut n_reg = 0u32;
    for r in 1..32 {
        if touched & (1 << r) != 0 {
            reg_local[r] = N_I64_LOCALS + 1 + n_reg;
            n_reg += 1;
        }
    }
    let c = Ctx {
        lay,
        reg_local,
        write_mask,
        idxb: N_I64_LOCALS + n_reg + 1, // i32 local sits after all i64 locals
    };
    // 4 i64 scratch (VA/PAGE/PA/VAL) + n_reg register locals + 1 i32 (IDXB).
    let mut m = WasmModule::with_locals(N_I64_LOCALS + n_reg, 1);
    // Prologue: load each touched register from the state struct into its
    // local. (Write-only regs are loaded too, so a mid-block bail can safely
    // flush the whole write-set.)
    let mut t = touched;
    while t != 0 {
        let r = t.trailing_zeros() as usize;
        t &= t - 1;
        m.i32_const(0)
            .i64_load(lay.x_base as u64 + r as u64 * 8)
            .local_set(reg_local[r]);
    }

    // Phase 3 / 3d-2: user-mode structured self-loop → wrap the body in one
    // wasm `loop` (forward if-then branches become nested wasm `if`) so the
    // register locals persist across iterations with no per-iteration dispatch.
    let self_loop = if lay.mem.is_some() {
        detect_structured_loop(code, base, start_pc, &lay)
    } else {
        None
    };
    // Pending forward-if end targets (loop path): a wasm `if` is closed when
    // pc reaches its target. `retired = ITER * body_n` slightly over-counts
    // skipped if-bodies — insn_count only, never the computation.
    let mut pending_ifs: Vec<u64> = Vec::new();
    if let Some(body_n) = self_loop {
        m.i64_const(0).local_set(ITER);
        m.op(LOOP).op(VOID); // $L (branch depth 0)
        // Iteration cap → yield to the dispatcher, resuming at start_pc.
        m.local_get(ITER).i64_const(LOOP_CAP as i64).op(I64_GE_U);
        m.op(IF).op(VOID);
        c.flush_writes(&mut m);
        c.set_pc_const(&mut m, start_pc);
        m.i32_const(0)
            .local_get(ITER)
            .i64_const(body_n as i64)
            .op(I64_MUL)
            .i64_store(lay.retired_addr as u64);
        m.op(RETURN);
        m.op(END); // close cap IF
    }

    let mut pc = start_pc;
    let mut n = 0u32;

    while n < MAX_BLOCK as u32 {
        // Close any forward-if wasm blocks whose target we've now reached.
        while pending_ifs.last() == Some(&pc) {
            m.op(END);
            pending_ifs.pop();
        }
        let Some((insn, ilen)) = fetch(code, base, pc) else {
            break;
        };
        let next_pc = pc.wrapping_add(ilen);
        let op = opcode(insn);
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));

        match op {
            // LUI / AUIPC: constants at translation time.
            0x37 | 0x17 => {
                if c.store_pre(&mut m, d) {
                    let v = if op == 0x37 {
                        imm_u(insn) as u64
                    } else {
                        pc.wrapping_add(imm_u(insn) as u64)
                    };
                    m.i64_const(v as i64);
                    c.store_post(&mut m, d);
                }
            }
            // OP-IMM
            0x13 => {
                let imm = imm_i(insn);
                let f3 = funct3(insn);
                if c.store_pre(&mut m, d) {
                    c.push_reg(&mut m, s1);
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
                    c.store_post(&mut m, d);
                }
            }
            // OP (I and M-mul only; div falls back)
            0x33 => {
                let f7 = funct7(insn);
                let f3 = funct3(insn);
                let supported = matches!((f7, f3), (0x00, _) | (0x20, 0) | (0x20, 5) | (0x01, 0));
                if !supported {
                    break;
                }
                if c.store_pre(&mut m, d) {
                    c.push_reg(&mut m, s1);
                    match (f7, f3) {
                        (0x00, 0) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_ADD);
                        }
                        (0x20, 0) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_SUB);
                        }
                        (0x01, 0) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_MUL);
                        }
                        (0x00, 1) => {
                            c.push_reg(&mut m, s2);
                            m.i64_const(0x3f).op(I64_AND).op(I64_SHL);
                        }
                        (0x00, 2) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_LT_S).op(I64_EXTEND_I32_U);
                        }
                        (0x00, 3) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_LT_U).op(I64_EXTEND_I32_U);
                        }
                        (0x00, 4) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_XOR);
                        }
                        (0x00, 5) => {
                            c.push_reg(&mut m, s2);
                            m.i64_const(0x3f).op(I64_AND).op(I64_SHR_U);
                        }
                        (0x20, 5) => {
                            c.push_reg(&mut m, s2);
                            m.i64_const(0x3f).op(I64_AND).op(I64_SHR_S);
                        }
                        (0x00, 6) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_OR);
                        }
                        (0x00, 7) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_AND);
                        }
                        _ => unreachable!(),
                    }
                    c.store_post(&mut m, d);
                }
            }
            // OP-IMM-32 (ADDIW/SLLIW/SRLIW/SRAIW): compute in 64, wrap+extend.
            0x1b => {
                let imm = imm_i(insn);
                let f3 = funct3(insn);
                if !matches!(f3, 0 | 1 | 5) {
                    break;
                }
                if c.store_pre(&mut m, d) {
                    c.push_reg(&mut m, s1);
                    match f3 {
                        0 => {
                            m.i64_const(imm).op(I64_ADD);
                        }
                        1 => {
                            m.i64_const(imm & 0x1f).op(I64_SHL);
                        }
                        _ => {
                            // shift on the 32-bit value
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
                    // sign-extend low 32 bits
                    m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                    c.store_post(&mut m, d);
                }
            }
            // OP-32 (ADDW/SUBW/MULW)
            0x3b => {
                let (f7, f3) = (funct7(insn), funct3(insn));
                if !matches!((f7, f3), (0x00, 0) | (0x20, 0) | (0x01, 0)) {
                    break;
                }
                if c.store_pre(&mut m, d) {
                    c.push_reg(&mut m, s1);
                    c.push_reg(&mut m, s2);
                    m.op(match (f7, f3) {
                        (0x00, 0) => I64_ADD,
                        (0x20, 0) => I64_SUB,
                        _ => I64_MUL,
                    });
                    m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                    c.store_post(&mut m, d);
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
                    _ => break,
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
                // address (i64 va) on stack
                c.push_reg(&mut m, s1);
                m.i64_const(imm_i(insn)).op(I64_ADD);
                let mem_off = if let Some((mem_base, size)) = lay.mem {
                    c.guest_addr(&mut m, size, len); // i32 index, traps OOB
                    mem_base as u64
                } else {
                    // system: probe TLB; leaves full linear index on stack
                    c.tlb_index(&mut m, &lay.sys.unwrap(), len, false, pc, n);
                    0
                };
                m.op(load_op).raw_uleb(len_align(len)).raw_uleb(mem_off);
                if d == 0 {
                    m.op(DROP);
                } else {
                    m.local_set(VAL);
                    c.store_pre(&mut m, d); // addr base for memory dest; nothing for a local
                    m.local_get(VAL);
                    c.store_post(&mut m, d);
                }
            }
            // STORE (user-mode direct, or system inline-TLB)
            0x23 if lay.mem.is_some() || lay.sys.is_some() => {
                let f3 = funct3(insn);
                if f3 > 3 {
                    break;
                }
                let len = 1u64 << f3;
                let store_op = match f3 {
                    0 => I64_STORE8,
                    1 => I64_STORE16,
                    2 => I64_STORE32,
                    _ => I64_STORE,
                };
                c.push_reg(&mut m, s1);
                m.i64_const(imm_s(insn)).op(I64_ADD);
                if let Some((mem_base, size)) = lay.mem {
                    c.guest_addr(&mut m, size, len);
                    c.push_reg(&mut m, s2);
                    m.op(store_op)
                        .raw_uleb(len_align(len))
                        .raw_uleb(mem_base as u64);
                } else {
                    // system: tlb_index consumes the va and leaves the RAM
                    // index on the stack; push_reg(s2) reads x[] (not touched
                    // by the probe), then store.
                    c.tlb_index(&mut m, &lay.sys.unwrap(), len, true, pc, n);
                    c.push_reg(&mut m, s2);
                    m.op(store_op).raw_uleb(len_align(len)).raw_uleb(0);
                }
            }
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
                // scratch = (x[rs1] + imm) & ~1  (compute before link write!)
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
                    _ => break, // before any stack pushes — stream stays balanced
                };
                // Phase 3 / 3d-2: inside a compiled loop, a branch back to start
                // is the loop back-edge; a forward branch is a structured
                // if-then (nested wasm `if`). Outside a loop, the branch ends
                // the block with a conditional pc select.
                if let Some(body_n) = self_loop {
                    if target == start_pc {
                        // count this iteration, then loop back if the branch is taken
                        m.local_get(ITER).i64_const(1).op(I64_ADD).local_set(ITER);
                        c.push_reg(&mut m, s1);
                        c.push_reg(&mut m, s2);
                        m.op(cmp);
                        m.br_if(0); // -> $L (loop top)
                        m.op(END); // close loop $L (fall through = branch not taken = exit)
                        c.flush_writes(&mut m);
                        c.set_pc_const(&mut m, next_pc);
                        m.i32_const(0)
                            .local_get(ITER)
                            .i64_const(body_n as i64)
                            .op(I64_MUL)
                            .i64_store(lay.retired_addr as u64);
                        return Some(Block {
                            wasm: m.finish(),
                            len: next_pc - start_pc,
                            n_insns: body_n,
                        });
                    }
                    // forward if-then: run [next_pc, target) under the NEGATED
                    // condition (the guest branch skips that range when taken).
                    let neg = match funct3(insn) {
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
                    pending_ifs.push(target);
                    // fall through: keep translating the loop body.
                } else {
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
            }
            // OP-FP: double-precision add/sub/mul/div + FMV.D.X/FMV.X.D inline
            // (Phase 2). Other FP ops (single, sqrt, fmadd, compare, convert,
            // sign-inject, and FP load/store) still end the block.
            0x53 if lay.f_base != 0 => {
                let f7 = funct7(insn);
                let (fmt, fpop, f3) = (f7 & 3, f7 >> 2, funct3(insn));
                let fb = lay.f_base as u64;
                match (fpop, fmt, f3) {
                    // FADD/FSUB/FMUL/FDIV.D (static RNE or dynamic rounding)
                    (0..=3, 1, 0 | 7) => c.fp_arith_d(&mut m, fpop, s1, s2, d, f3 == 7, pc, n),
                    // FLE/FLT/FEQ.D -> x[d]
                    (0x14, 1, 0..=2) => c.fp_cmp_d(&mut m, f3, s1, s2, d, pc, n),
                    // FMV.D.X: f[d] = x[s1] (raw 64-bit copy)
                    (0x1e, 1, 0) => {
                        m.i32_const(0);
                        c.push_reg(&mut m, s1);
                        m.i64_store(fb + d as u64 * 8);
                    }
                    // FMV.X.D: x[d] = f[s1] (raw 64-bit copy)
                    (0x1c, 1, 0) => {
                        if c.store_pre(&mut m, d) {
                            m.i32_const(0).i64_load(fb + s1 as u64 * 8);
                            c.store_post(&mut m, d);
                        }
                    }
                    _ => break,
                }
            }
            // Anything else (AMO/FP/SYSTEM, or memory ops with no memory
            // layout): end the block here; the interpreter takes over.
            _ => break,
        }

        pc = next_pc;
        n += 1;
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
