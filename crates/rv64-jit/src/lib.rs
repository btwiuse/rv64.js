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
/// Loop-iteration counter (Phase 3 self-loop compilation); also the retired-
/// instruction accumulator in compiled loops and superblocks.
const ITER: u32 = 5;
/// Superblock dispatch: the current target pc, fed to the internal `br_table`.
const TPC: u32 = 6;
/// Total i64 scratch locals to declare (register locals follow next; the
/// i32 IDXB local follows all i64 locals, so its index is dynamic).
const N_I64_LOCALS: u32 = 6;

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

    /// Bail unless the i64 in VAL, viewed as an f64, is a NORMAL number:
    /// exp in [1, 0x7fe]. Catches inf/nan (0x7ff) and subnormal/zero (0),
    /// whose flag/rounding corner cases the softfloat interpreter must own.
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
    fn fp_sqrt_d(&self, m: &mut WasmModule, s1: usize, d: usize, dyn_rm: bool, pc: u64, n: u32) {
        self.fp_eligibility(m, dyn_rm, pc, n);
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
    fn fp_cvt_d_int(&self, m: &mut WasmModule, s1: usize, d: usize, v: u32, dyn_rm: bool, pc: u64, n: u32) {
        if v >= 2 {
            self.fp_eligibility(m, dyn_rm, pc, n);
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
                  neg_prod: bool, neg_c: bool, dyn_rm: bool, pc: u64, n: u32) {
        debug_assert!(self.fma_scratch != 0);
        let fs = self.fma_scratch;
        let (fa, fb, fc, fp, f4, f5, f6, f7l) =
            (fs, fs + 1, fs + 2, fs + 3, fs + 4, fs + 5, fs + 6, fs + 7);
        let getf = |m: &mut WasmModule, l: u32| { m.local_get(l).op(F64_REINTERPRET_I64); };
        let setf = |m: &mut WasmModule, l: u32| { m.op(I64_REINTERPRET_F64).local_set(l); };
        let fconst = |m: &mut WasmModule, v: f64| {
            m.i64_const(v.to_bits() as i64).op(F64_REINTERPRET_I64);
        };
        self.fp_eligibility(m, dyn_rm, pc, n);
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
        // exponent band: bail unless ((bits>>52)&0x7ff) - 0x100 <=u 0x5ff
        for &l in &[fa, fb, fc] {
            m.local_get(l)
                .i64_const(52)
                .op(I64_SHR_U)
                .i64_const(0x7ff)
                .op(I64_AND)
                .i64_const(0x100)
                .op(I64_SUB)
                .i64_const(0x5ff)
                .op(I64_GT_U);
            m.op(IF).op(VOID);
            self.bail(m, pc, n);
            m.op(END);
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
        // r = s + v, result-normal guard, store
        getf(m, f5);
        getf(m, f7l);
        m.op(F64_ADD);
        setf(m, VAL);
        self.fp_result_normal_guard(m, pc, n);
        self.store_freg_pre(m, d);
        m.local_get(VAL);
        self.store_freg_post(m, d);
    }

    fn fp_arith_d(&self, m: &mut WasmModule, op: u32, s1: usize, s2: usize, d: usize, dyn_rm: bool, pc: u64, n: u32) {
        self.fp_eligibility(m, dyn_rm, pc, n);
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
        self.fp_result_normal_guard(m, pc, n);
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
    /// leaving pc at `pc` for the interpreter to resume. Inside a compiled
    /// loop the true retired count is the runtime ITER accumulator, not `n`.
    fn bail(&self, m: &mut WasmModule, pc: u64, n: u32) {
        self.flush_writes(m);
        self.set_pc_const(m, pc);
        if let Some(l) = self.retired_local {
            m.i32_const(0).local_get(l).i64_store(self.lay.retired_addr as u64);
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
        self.bail(m, pc, n);
        m.op(END);
        // linear index = (va + ftlb_off[idx]) as i32
        m.local_get(VA);
        m.local_get_i32(self.idxb).i64_load_at(off_base as u64);
        m.op(I64_ADD).op(I32_WRAP_I64);
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
            }
            0x23 if lay.mem.is_some() || lay.sys.is_some() => {
                if funct3(insn) > 3 {
                    break;
                }
                mark(&mut read, s1);
                mark(&mut read, s2);
            }
            // FLD / FSD (double, funct3==3): raw 8-byte copy mem<->f[], user-mode
            // direct or system inline-TLB (needs f_base for the FP file).
            0x07 if (lay.mem.is_some() || lay.sys.is_some()) && lay.f_base != 0 => {
                if funct3(insn) != 3 {
                    break;
                }
                mark(&mut read, s1);
                fmark(&mut fwrite, d);
            }
            0x27 if (lay.mem.is_some() || lay.sys.is_some()) && lay.f_base != 0 => {
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
        pc = next_pc;
        n += 1;
    }
    (read, write, fread, fwrite)
}

/// Is this OP-FP (0x53) instruction one the JIT emits inline?
///
/// THE single authority for FP, like alu_handled for integer ops: every
/// scanner and the emitter must agree or block boundaries desync. Takes the
/// whole insn because FCVT variants are selected by the rs2 FIELD.
/// Covered: D arith (FADD/FSUB/FMUL/FDIV), compares, FMV both ways, FSQRT.D,
/// FCVT.W.D (rtz), FCVT.D.{W,WU,L,LU}.
fn fp_handled(insn: u32) -> bool {
    let f7 = funct7(insn);
    let f3 = funct3(insn);
    match (f7 >> 2, f7 & 3, f3) {
        (0..=3, 1, 0 | 7) => true,        // FADD/FSUB/FMUL/FDIV.D (rne | dyn)
        (0x14, 1, 0..=2) => true,         // FLE/FLT/FEQ.D
        (0x1e, 1, 0) => true,             // FMV.D.X
        (0x1c, 1, 0) => true,             // FMV.X.D
        (0x0b, 1, 0 | 7) => true,         // FSQRT.D (rne | dyn)
        (0x18, 1, 1) => rs2(insn) == 0,   // FCVT.W.D rtz only (signed 32)
        (0x1a, 1, 0 | 7) => rs2(insn) <= 3, // FCVT.D.{W,WU,L,LU} (rne | dyn)
        _ => false,
    }
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
        0x37 | 0x17 | 0x13 => true,
        0x33 => matches!(
            (f7, f3),
            (0x00, _) | (0x20, 0) | (0x20, 5) | (0x01, 0) | (0x01, 4..=7)
        ),
        0x1b => matches!(f3, 0 | 1 | 5),
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
}

/// Detect and fully validate a structured loop region at `start_pc` (which must
/// be a natural-loop header — the target of a backward branch). User-mode only:
/// inline memory ops here only TRAP on fault (never bail mid-loop), and the FP
/// register file is present. Returns None for anything not provably structured
/// (the caller then compiles a plain basic block).
fn loop_region(code: &[u8], base: u64, start_pc: u64, lay: &JitLayout) -> Option<LoopRegion> {
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
    // read_mask bit 0 (x0 — never a real register) smuggles the "block
    // contains FMADD-family" flag from the scanners; strip it BEFORE any mask
    // use (a set bit 0 would make the prologue clobber local 0, the machine
    // pointer parameter).
    let want_fma = read_mask & 1 != 0;
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
    let n_fma = if want_fma { 8 } else { 0 };
    let c = Ctx {
        lay,
        reg_local,
        write_mask,
        // i32 local after all i64 locals (incl. the fma scratch block)
        idxb: N_I64_LOCALS + n_reg + n_fp + n_fma + 1,
        fp_local,
        fp_write_mask: fp_write,
        fma_scratch: if want_fma { N_I64_LOCALS + 1 + n_reg + n_fp } else { 0 },
        retired_local: None,
    };
    let mut m = WasmModule::with_locals(N_I64_LOCALS + n_reg + n_fp + n_fma, 1);
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
            if funct3(insn) != 3 {
                return false;
            }
            c.push_reg(m, s1);
            m.i64_const(imm_i(insn)).op(I64_ADD);
            let off = if let Some((mem_base, size)) = lay.mem {
                c.guest_addr(m, size, 8);
                mem_base as u64
            } else {
                c.tlb_index(m, &lay.sys.unwrap(), 8, false, pc, n);
                0
            };
            m.op(I64_LOAD).raw_uleb(len_align(8)).raw_uleb(off);
            m.local_set(VAL);
            c.store_freg_pre(m, d);
            m.local_get(VAL);
            c.store_freg_post(m, d);
        }
        // FSD: mem[x[s1]+imm] = f[s2] (double). Raw 8-byte copy.
        0x27 if (lay.mem.is_some() || lay.sys.is_some()) && lay.f_base != 0 => {
            if funct3(insn) != 3 {
                return false;
            }
            c.push_reg(m, s1);
            m.i64_const(imm_s(insn)).op(I64_ADD);
            if let Some((mem_base, size)) = lay.mem {
                c.guest_addr(m, size, 8);
                c.push_freg(m, s2);
                m.op(I64_STORE).raw_uleb(len_align(8)).raw_uleb(mem_base as u64);
            } else {
                c.tlb_index(m, &lay.sys.unwrap(), 8, true, pc, n);
                c.push_freg(m, s2);
                m.op(I64_STORE).raw_uleb(len_align(8)).raw_uleb(0);
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
    for &x in &[ab, bb, cb] {
        let e = exp(x);
        if !(0x100..=0x6ff).contains(&e) {
            return None;
        }
    }
    let a = f64::from_bits(ab);
    let b = f64::from_bits(bb);
    let c = f64::from_bits(cb);
    let p = a * b;
    if !(0x100..=0x6ff).contains(&exp(p.to_bits())) {
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
    let r = s + v; // == round(a*b + c)
    if !(1..=0x7fe).contains(&exp(r.to_bits())) {
        return None;
    }
    Some(r.to_bits())
}

pub fn translate_block(code: &[u8], base: u64, start_pc: u64, lay: JitLayout) -> Option<Block> {
    // Structured loop region (nested loops + forward if-then/break) → compile
    // the whole thing as one wasm function so register locals persist across
    // every iteration of every level (3e-2 / v86 control-flow structuring).
    if lay.mem.is_some() || lay.sys.is_some() {
        if let Some(region) = loop_region(code, base, start_pc, &lay) {
            let (rm, wm, fr, fw) = scan_regs_region(code, base, start_pc, region.end_pc, &lay);
            let (mut c, m) = build_ctx(lay, rm, wm, fr, fw);
            // Mid-loop bails (system TLB miss/MMIO, FP fast-path) must report the
            // live iteration count, not a static index — see Ctx::retired_local.
            c.retired_local = Some(ITER);
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

/// Scan the touched GP/FP registers across every entry block of a page-
/// superblock (each block walked to its terminating control-flow / unhandled
/// instruction). Over-approximating is safe (an unused local just gets loaded).
fn scan_regs_super(
    code: &[u8],
    base: u64,
    page_end: u64,
    entries: &[u64],
    lay: &JitLayout,
) -> (u32, u32, u32, u32) {
    let (mut r, mut w, mut fr, mut fw) = (0u32, 0u32, 0u32, 0u32);
    let fmark = |m: &mut u32, x: usize| *m |= 1 << x;
    let mark = |m: &mut u32, x: usize| {
        if x != 0 {
            *m |= 1 << x;
        }
    };
    for &e in entries {
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
                    mark(&mut r, s1);
                    mark(&mut r, s2);
                    break;
                }
                0x6f => {
                    mark(&mut w, d);
                    break;
                }
                0x67 => {
                    mark(&mut r, s1);
                    mark(&mut w, d);
                    break;
                }
                0x37 | 0x17 => mark(&mut w, d),
                0x13 | 0x1b => {
                    mark(&mut r, s1);
                    mark(&mut w, d);
                }
                0x33 | 0x3b => {
                    if !alu_handled(op, funct7(insn), funct3(insn)) {
                        break;
                    }
                    mark(&mut r, s1);
                    mark(&mut r, s2);
                    mark(&mut w, d);
                }
                0x03 => {
                    if funct3(insn) == 7 {
                        break;
                    }
                    mark(&mut r, s1);
                    mark(&mut w, d);
                }
                0x23 => {
                    if funct3(insn) > 3 {
                        break;
                    }
                    mark(&mut r, s1);
                    mark(&mut r, s2);
                }
                0x07 if lay.f_base != 0 => {
                    if funct3(insn) != 3 {
                        break;
                    }
                    mark(&mut r, s1);
                    fmark(&mut fw, d);
                }
                0x27 if lay.f_base != 0 => {
                    if funct3(insn) != 3 {
                        break;
                    }
                    mark(&mut r, s1);
                    fmark(&mut fr, s2);
                }
                0x53 if lay.f_base != 0 => {
                    if !fp_handled(insn) {
                        break;
                    }
                    let f7 = funct7(insn);
                    match (f7 >> 2, f7 & 3, funct3(insn)) {
                        (0..=3, 1, 0 | 7) => {
                            fmark(&mut fr, s1);
                            fmark(&mut fr, s2);
                            fmark(&mut fw, d);
                        }
                        (0x14, 1, 0..=2) => {
                            fmark(&mut fr, s1);
                            fmark(&mut fr, s2);
                            mark(&mut w, d);
                        }
                        (0x1e, 1, 0) => {
                            mark(&mut r, s1);
                            fmark(&mut fw, d);
                        }
                        (0x1c, 1, 0) => {
                            fmark(&mut fr, s1);
                            mark(&mut w, d);
                        }
                        (0x0b, 1, 0 | 7) => {
                            fmark(&mut fr, s1);
                            fmark(&mut fw, d);
                        }
                        (0x18, 1, 1) => {
                            fmark(&mut fr, s1);
                            mark(&mut w, d);
                        }
                        (0x1a, 1, 0 | 7) => {
                            mark(&mut r, s1);
                            fmark(&mut fw, d);
                        }
                        _ => break,
                    }
                }
                op @ (0x43 | 0x47 | 0x4b | 0x4f) if lay.f_base != 0 => {
                    if !fma_handled(op, insn) {
                        break;
                    }
                    fmark(&mut fr, s1);
                    fmark(&mut fr, s2);
                    fmark(&mut fr, ((insn >> 27) & 31) as usize);
                    fmark(&mut fw, d);
                    r |= 1; // bit 0 = "block contains fma" (see build_ctx)
                }
                _ => break,
            }
            pc = pc.wrapping_add(ilen);
            n += 1;
        }
    }
    (r, w, fr, fw)
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
    let page_end = page_base + page_span;
    let in_page = |pc: u64| pc >= page_base && pc < page_end;
    let mut leaders: std::collections::BTreeSet<u64> = seeds.iter().copied().collect();
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
                    add(pc.wrapping_add(imm_b(insn) as u64), &mut leaders, &mut pending);
                    add(next, &mut leaders, &mut pending);
                    break;
                }
                // JAL: target is a leader if in page; a CALL (rd != 0) also makes
                // the return site a leader (the callee's ret dispatches back there)
                0x6f => {
                    add(pc.wrapping_add(imm_j(insn) as u64), &mut leaders, &mut pending);
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
                        0x07 | 0x27 => funct3(insn) == 3,
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
    leaders.into_iter().collect()
}

/// Compile a whole page of basic blocks (v86's function-per-page) into one wasm
/// function with an internal `br_table` dispatch loop and all touched registers
/// cached in locals for the function's lifetime — so execution flows between
/// blocks with no per-block prologue/epilogue, `call_indirect` or pa-verify (the
/// per-dispatch overhead that dominates branchy code like the CPython eval
/// loop). `entries` are the block-start pcs discovered hot in this page.
pub fn translate_superblock(
    code: &[u8],
    base: u64,
    page_base: u64,
    page_span: u64,
    entries: &[u64],
    lay: JitLayout,
) -> Option<Block> {
    let n = entries.len();
    if n == 0 || page_span == 0 || page_span > (1 << 16) {
        return None;
    }
    let page_end = page_base + page_span;
    let (rm, wm, fr, fw) = scan_regs_super(code, base, page_end, entries, &lay);
    let (mut c, mut m) = build_ctx(lay, rm, wm, fr, fw);
    c.retired_local = Some(ITER);

    // slot (= (pc-page_base)/2) -> entry index, else n (= default -> exit).
    let slots = (page_span / 2) as usize;
    let mut slot_depth = vec![n as u32; slots];
    for (i, &e) in entries.iter().enumerate() {
        if e < page_base || e >= page_end {
            return None;
        }
        slot_depth[((e - page_base) / 2) as usize] = i as u32;
    }

    m.i64_const(0).local_set(ITER); // retired accumulator
    m.i32_const(0).i64_load(lay.pc_addr as u64).local_set(TPC);

    m.op(BLOCK).op(VOID); // $exit  (depth 1 from loop body)
    m.op(LOOP).op(VOID); // $L      (depth 0 from loop body)

    // Cap → yield to the host (bound interrupt latency).
    m.local_get(ITER).i64_const(LOOP_CAP as i64).op(I64_GE_U).br_if(1);
    // Bounds: offset = TPC - page_base; exit if offset >=u span (also catches
    // TPC < page_base, which wraps to a huge unsigned value).
    m.local_get(TPC)
        .i64_const(page_base as i64)
        .op(I64_SUB)
        .local_set(SCR);
    m.local_get(SCR).i64_const(page_span as i64).op(I64_GE_U).br_if(1);

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
        c.emit_super_body(&mut m, lay, code, base, entries[i], page_end, (n - i) as u32, (n - i + 1) as u32);
    }
    m.op(END); // close $default
    // default: TPC wasn't a known entry in-page → exit ($exit at depth 1).
    m.br(1);

    m.op(END); // close loop $L
    m.op(END); // close block $exit

    // Exit: flush registers, publish TPC + retired.
    c.flush_writes(&mut m);
    m.i32_const(0).local_get(TPC).i64_store(lay.pc_addr as u64);
    m.i32_const(0).local_get(ITER).i64_store(lay.retired_addr as u64);

    Some(Block {
        wasm: m.finish(),
        len: page_span,
        n_insns: n as u32,
    })
}


#[cfg(test)]
mod tests {
    use super::*;

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
