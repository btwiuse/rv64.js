//! RV64 instruction fetch and lifting into typed SSA.

use crate::ir::{
    BinaryOp, Builder, DivideOp, ExactFpOp, ExitKind, LoadKind, MulHighKind, Op, Region,
    ReservationOp, StoreKind, ValueId,
};
use rv64_core::decode::{funct3, funct7, imm_b, imm_i, imm_j, imm_s, imm_u, opcode, rd, rs1, rs2};
use std::collections::{BTreeSet, HashSet, VecDeque};

pub(crate) const T1_MAX_INSNS: u32 = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FpMode {
    Disabled,
    User,
    System,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Metrics {
    pub uses_fp: bool,
    pub trace_mix: [u16; 5],
    pub trace_stack_memory: u16,
    pub trace_mem: [u16; 10],
    pub trace_control: [u16; 3],
    pub trace_alu: [u16; 5],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LiftedRegion {
    pub ir: Region,
    pub metrics: Metrics,
    pub byte_len: u64,
    pub span: (u64, u64),
    pub seeds: Vec<u64>,
    pub loop_backedge: Option<LoopBackedge>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LoopBackedge {
    /// `None` denotes an unconditional backedge to the region entry.
    pub condition: Option<ValueId>,
    pub exit_pc: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Decoded {
    pub(crate) insn: u32,
    pub(crate) len: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StepOutcome {
    Continue,
    ContinueAt(u64),
    Exit(ValueId),
    IndirectExit(ValueId),
    Unsupported,
}

pub(crate) fn decode_at(code: &[u8], base: u64, pc: u64) -> Option<Decoded> {
    if pc < base || pc & 1 != 0 {
        return None;
    }
    let offset = usize::try_from(pc - base).ok()?;
    let lo = u16::from_le_bytes([*code.get(offset)?, *code.get(offset + 1)?]);
    if lo & 3 != 3 {
        return Some(Decoded {
            insn: rv64_core::compressed::expand(lo)?,
            len: 2,
        });
    }
    let hi = u16::from_le_bytes([*code.get(offset + 2)?, *code.get(offset + 3)?]);
    Some(Decoded {
        insn: u32::from(lo) | (u32::from(hi) << 16),
        len: 4,
    })
}

/// Discover basic-block leaders reachable from measured entry seeds.
///
/// This pass is deliberately independent of the compiler IR. It only follows
/// architectural direct control flow and stops at indirect control or an
/// instruction that can synchronously redirect execution. The T1 lifter is
/// still the authority on whether a discovered leader can be compiled.
pub(crate) fn discover_leaders(
    code: &[u8],
    base: u64,
    lo: u64,
    span: u64,
    seeds: &[u64],
    capacity: usize,
) -> (Vec<u64>, BTreeSet<u64>, BTreeSet<u64>) {
    discover_leaders_with_cross_page_calls(code, base, lo, span, seeds, capacity, true)
}

/// Variant used by the page-policy experiment that keeps ordinary CFG
/// closure but does not merge a direct callee living on another page into its
/// caller's region. Same-page calls are still followed, as are cross-page
/// conditional branches, tail jumps, and fallthrough.
pub(crate) fn discover_leaders_without_cross_page_calls(
    code: &[u8],
    base: u64,
    lo: u64,
    span: u64,
    seeds: &[u64],
    capacity: usize,
) -> (Vec<u64>, BTreeSet<u64>, BTreeSet<u64>) {
    discover_leaders_with_cross_page_calls(code, base, lo, span, seeds, capacity, false)
}

fn discover_leaders_with_cross_page_calls(
    code: &[u8],
    base: u64,
    lo: u64,
    span: u64,
    seeds: &[u64],
    capacity: usize,
    follow_cross_page_calls: bool,
) -> (Vec<u64>, BTreeSet<u64>, BTreeSet<u64>) {
    if capacity == 0 || span == 0 {
        return (Vec::new(), BTreeSet::new(), BTreeSet::new());
    }
    let hi = lo.saturating_add(span);
    let code_hi = base.saturating_add(code.len() as u64);
    let in_range = |pc: u64| pc >= lo && pc < hi && pc >= base && pc < code_hi && pc & 1 == 0;

    let mut queued = HashSet::new();
    let mut queue = VecDeque::new();
    for &seed in seeds {
        if in_range(seed) && queued.insert(seed) {
            queue.push_back(seed);
        }
    }

    let mut leaders = Vec::with_capacity(capacity.min(seeds.len().max(8)));
    let mut scanned = HashSet::new();
    let mut backedges = BTreeSet::new();
    let mut reachable_pages = BTreeSet::new();
    // A malformed byte stream must not make discovery super-linear. Every
    // halfword can be decoded at most once in the ordinary fallthrough path.
    let mut decode_budget = span
        .min(code.len() as u64)
        .div_ceil(2)
        .saturating_add(capacity as u64 * 4);

    while let Some(entry) = queue.pop_front() {
        if leaders.len() >= capacity {
            break;
        }
        if !scanned.insert(entry) || decode_at(code, base, entry).is_none() {
            continue;
        }
        leaders.push(entry);
        let mut pc = entry;

        loop {
            if decode_budget == 0 || !in_range(pc) {
                break;
            }
            if pc != entry && scanned.contains(&pc) {
                break;
            }
            let Some(decoded) = decode_at(code, base, pc) else {
                break;
            };
            reachable_pages.insert(pc & !0xfff);
            decode_budget -= 1;
            let fallthrough = pc.wrapping_add(decoded.len);

            let enqueue = |target: u64, queue: &mut VecDeque<u64>, queued: &mut HashSet<u64>| {
                if in_range(target) && queued.insert(target) {
                    queue.push_back(target);
                }
            };

            match opcode(decoded.insn) {
                // Conditional branches have two possible successor leaders.
                0x63 => {
                    let target = pc.wrapping_add(imm_b(decoded.insn) as u64);
                    if target <= pc && in_range(target) {
                        backedges.insert(target);
                    }
                    enqueue(target, &mut queue, &mut queued);
                    enqueue(fallthrough, &mut queue, &mut queued);
                    break;
                }
                // Direct jumps terminate a block. Calls additionally expose
                // their architectural return address as a reachable leader.
                0x6f => {
                    let target = pc.wrapping_add(imm_j(decoded.insn) as u64);
                    if target <= pc && in_range(target) {
                        backedges.insert(target);
                    }
                    let is_call = rd(decoded.insn) != 0;
                    if !is_call || follow_cross_page_calls || target & !0xfff == pc & !0xfff {
                        enqueue(target, &mut queue, &mut queued);
                    }
                    if is_call {
                        enqueue(fallthrough, &mut queue, &mut queued);
                    }
                    break;
                }
                // Indirect control and environment instructions have no safe
                // static successor. CSR instructions (funct3 != 0) remain
                // ordinary fallthrough; the lifter decides if it supports one.
                0x67 => break,
                0x73 if funct3(decoded.insn) == 0 => break,
                _ => {
                    pc = fallthrough;
                    if queued.contains(&pc) {
                        break;
                    }
                }
            }
        }
    }

    (leaders, backedges, reachable_pages)
}

/// Return cross-page direct-call targets found in one code snapshot.
pub(crate) fn direct_call_targets(code: &[u8], base: u64) -> Vec<u64> {
    let page = base & !0xfff;
    let mut targets = BTreeSet::new();
    let mut pc = base;
    let hi = base.saturating_add(code.len() as u64);
    while pc < hi {
        let Some(decoded) = decode_at(code, base, pc) else {
            pc = pc.saturating_add(2);
            continue;
        };
        if opcode(decoded.insn) == 0x6f && matches!(rd(decoded.insn), 1 | 5) {
            let target = pc.wrapping_add(imm_j(decoded.insn) as u64);
            if target & !0xfff != page {
                targets.insert(target);
            }
        }
        pc = pc.saturating_add(decoded.len);
    }
    targets.into_iter().collect()
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn lift_t1(
    code: &[u8],
    base: u64,
    entry_pc: u64,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
) -> Option<LiftedRegion> {
    lift_t1_with_vector(
        code,
        base,
        entry_pc,
        allow_memory,
        fp_mode,
        allow_reservation,
        false,
    )
}

pub(crate) fn lift_t1_with_vector(
    code: &[u8],
    base: u64,
    entry_pc: u64,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
    allow_vector: bool,
) -> Option<LiftedRegion> {
    lift_t1_profiled_with_vector(
        code,
        base,
        entry_pc,
        allow_memory,
        fp_mode,
        allow_reservation,
        allow_vector,
        None,
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn lift_t1_profiled(
    code: &[u8],
    base: u64,
    entry_pc: u64,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
    profiled_indirect_target: Option<u64>,
) -> Option<LiftedRegion> {
    lift_t1_profiled_with_vector(
        code,
        base,
        entry_pc,
        allow_memory,
        fp_mode,
        allow_reservation,
        false,
        profiled_indirect_target,
    )
}

pub(crate) fn lift_t1_profiled_with_vector(
    code: &[u8],
    base: u64,
    entry_pc: u64,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
    allow_vector: bool,
    profiled_indirect_target: Option<u64>,
) -> Option<LiftedRegion> {
    let targets: Vec<u64> = profiled_indirect_target.into_iter().collect();
    lift_t1_profiled_targets_with_vector(
        code,
        base,
        entry_pc,
        allow_memory,
        fp_mode,
        allow_reservation,
        allow_vector,
        &targets,
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn lift_t1_profiled_targets(
    code: &[u8],
    base: u64,
    entry_pc: u64,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
    profiled_indirect_targets: &[u64],
) -> Option<LiftedRegion> {
    lift_t1_profiled_targets_with_vector(
        code,
        base,
        entry_pc,
        allow_memory,
        fp_mode,
        allow_reservation,
        false,
        profiled_indirect_targets,
    )
}

pub(crate) fn lift_t1_profiled_targets_with_vector(
    code: &[u8],
    base: u64,
    entry_pc: u64,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
    allow_vector: bool,
    profiled_indirect_targets: &[u64],
) -> Option<LiftedRegion> {
    lift_t1_profiled_targets_mode(
        code,
        base,
        entry_pc,
        allow_memory,
        fp_mode,
        allow_reservation,
        allow_vector,
        profiled_indirect_targets,
        true,
    )
}

/// Lift one architectural basic block for a multi-entry CFG function. Unlike
/// T1 traces, forward control transfers terminate the member so both covered
/// successors can remain inside the shared Wasm dispatcher.
#[allow(dead_code)]
pub(crate) fn lift_basic_block(
    code: &[u8],
    base: u64,
    entry_pc: u64,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
) -> Option<LiftedRegion> {
    lift_basic_block_with_vector(
        code,
        base,
        entry_pc,
        allow_memory,
        fp_mode,
        allow_reservation,
        false,
    )
}

pub(crate) fn lift_basic_block_with_vector(
    code: &[u8],
    base: u64,
    entry_pc: u64,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
    allow_vector: bool,
) -> Option<LiftedRegion> {
    lift_t1_profiled_targets_mode(
        code,
        base,
        entry_pc,
        allow_memory,
        fp_mode,
        allow_reservation,
        allow_vector,
        &[],
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn lift_t1_profiled_targets_mode(
    code: &[u8],
    base: u64,
    entry_pc: u64,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
    allow_vector: bool,
    profiled_indirect_targets: &[u64],
    trace_forward: bool,
) -> Option<LiftedRegion> {
    if entry_pc < base || entry_pc & 1 != 0 {
        return None;
    }

    let mut builder = Builder::new(entry_pc);
    let mut metrics = Metrics::default();
    let mut pc = entry_pc;
    let mut retired = 0;
    let mut exit = ExitKind::RegionLimit;
    let mut source_lo = entry_pc;
    let mut source_hi = entry_pc;
    let mut seeds = vec![entry_pc];
    let mut profiled_index = 0usize;
    let next_pc;

    loop {
        if retired >= T1_MAX_INSNS {
            next_pc = builder.guest_pc(pc, pc);
            break;
        }
        let Some(decoded) = decode_at(code, base, pc) else {
            if retired == 0 {
                return None;
            }
            next_pc = builder.guest_pc(pc, pc);
            break;
        };
        let fallthrough = pc.wrapping_add(decoded.len);
        let outcome = lift_one(
            &mut builder,
            &mut metrics,
            decoded.insn,
            pc,
            fallthrough,
            retired,
            allow_memory,
            fp_mode,
            allow_reservation,
            allow_vector,
            trace_forward,
        );
        if !matches!(outcome, StepOutcome::Unsupported) {
            source_lo = source_lo.min(pc);
            source_hi = source_hi.max(fallthrough);
        }
        match outcome {
            StepOutcome::Continue => {
                retired += 1;
                pc = fallthrough;
            }
            StepOutcome::ContinueAt(target) => {
                retired += 1;
                if target > pc && decode_at(code, base, target).is_some() {
                    pc = target;
                } else {
                    exit = ExitKind::Dispatch;
                    next_pc = builder.guest_pc(target, pc);
                    pc = fallthrough;
                    break;
                }
            }
            StepOutcome::Exit(target) => {
                retired += 1;
                pc = fallthrough;
                exit = ExitKind::Dispatch;
                next_pc = target;
                break;
            }
            StepOutcome::IndirectExit(target) => {
                retired += 1;
                if let Some(&observed) = profiled_indirect_targets.get(profiled_index) {
                    if decode_at(code, base, observed).is_some() {
                        profiled_index += 1;
                        builder.guard_target(target, observed, retired);
                        if !seeds.contains(&observed) {
                            seeds.push(observed);
                        }
                        if observed == entry_pc {
                            pc = fallthrough;
                            exit = ExitKind::Dispatch;
                            next_pc = builder.guest_pc(entry_pc, pc);
                            break;
                        }
                        pc = observed;
                        continue;
                    }
                }
                pc = fallthrough;
                exit = ExitKind::Dispatch;
                next_pc = target;
                break;
            }
            StepOutcome::Unsupported => {
                if retired == 0 {
                    return None;
                }
                exit = ExitKind::Unsupported;
                next_pc = builder.guest_pc(pc, pc);
                break;
            }
        }
    }

    let mut ir = builder.finish(pc, next_pc, retired, exit);
    ir.trace_mix = metrics.trace_mix;
    ir.trace_stack_memory = metrics.trace_stack_memory;
    ir.writes_x2 = ir.outputs.iter().any(|(reg, _)| *reg == 2);
    // Vector helpers can invalidate every scalar carry. Until the dedicated
    // loop carrier models that barrier, retain compiled vector bodies but let
    // the outer dispatcher own their backedges.
    let loop_backedge = (!ir.has_vector_helper())
        .then(|| detect_single_latch_loop(&ir))
        .flatten();
    Some(LiftedRegion {
        byte_len: source_hi.saturating_sub(source_lo),
        span: (source_lo, source_hi),
        ir,
        metrics,
        seeds,
        loop_backedge,
    })
}

fn detect_single_latch_loop(region: &Region) -> Option<LoopBackedge> {
    if (!region.f_outputs.is_empty() || region.fcsr_output.is_some()) && !region.has_effects() {
        return None;
    }
    if let Some(target) = static_guest_pc(region, region.next_pc) {
        return (target == region.entry_pc).then_some(LoopBackedge {
            condition: None,
            exit_pc: region.entry_pc,
        });
    }
    let Op::SelectI64 {
        condition,
        if_true,
        if_false,
    } = region.values.get(region.next_pc.0)?.op
    else {
        return None;
    };
    let taken = static_guest_pc(region, if_true)?;
    let not_taken = static_guest_pc(region, if_false)?;
    ((taken == region.entry_pc) && (not_taken == region.end_pc)).then_some(LoopBackedge {
        condition: Some(condition),
        exit_pc: not_taken,
    })
}

fn static_guest_pc(region: &Region, value: ValueId) -> Option<u64> {
    match region.values.get(value.0)?.op {
        Op::GuestPc(pc) => Some(pc),
        // Keep hand-built IR in unit tests and external diagnostics compatible.
        Op::ConstI64(pc) => Some(pc as u64),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn lift_one(
    b: &mut Builder,
    metrics: &mut Metrics,
    insn: u32,
    pc: u64,
    fallthrough: u64,
    retired: u32,
    allow_memory: bool,
    fp_mode: FpMode,
    allow_reservation: bool,
    allow_vector: bool,
    trace_forward: bool,
) -> StepOutcome {
    let allow_fp = fp_mode != FpMode::Disabled;
    let system_fp = fp_mode == FpMode::System;
    let op = opcode(insn);
    let result = match op {
        // LUI
        0x37 => {
            let value = b.const_i64(imm_u(insn), pc);
            b.write_x(rd(insn), value);
            StepOutcome::Continue
        }
        // AUIPC
        0x17 => {
            let value = b.guest_pc(pc.wrapping_add(imm_u(insn) as u64), pc);
            b.write_x(rd(insn), value);
            StepOutcome::Continue
        }
        // JAL
        0x6f => {
            let link = b.guest_pc(fallthrough, pc);
            b.write_x(rd(insn), link);
            let target_pc = pc.wrapping_add(imm_j(insn) as u64);
            metrics.trace_control[1] = metrics.trace_control[1].saturating_add(1);
            if trace_forward && target_pc > pc {
                StepOutcome::ContinueAt(target_pc)
            } else {
                let target = b.guest_pc(target_pc, pc);
                StepOutcome::Exit(target)
            }
        }
        // JALR. Read rs1 before writing rd: rd and rs1 are allowed to alias.
        0x67 if funct3(insn) == 0 => {
            let base = b.read_x(rs1(insn), pc);
            let immediate = b.const_i64(imm_i(insn), pc);
            let sum = b.binary(BinaryOp::I64Add, base, immediate, pc);
            let mask = b.const_i64(-2, pc);
            let target = b.binary(BinaryOp::I64And, sum, mask, pc);
            let link = b.guest_pc(fallthrough, pc);
            b.write_x(rd(insn), link);
            metrics.trace_control[2] = metrics.trace_control[2].saturating_add(1);
            StepOutcome::IndirectExit(target)
        }
        // Conditional control: forward branches become guarded traces;
        // backward branches terminate or become entry-latch loops.
        0x63 => {
            let lhs = b.read_x(rs1(insn), pc);
            let rhs = b.read_x(rs2(insn), pc);
            let cmp = match funct3(insn) {
                0 => BinaryOp::I64Eq,
                1 => BinaryOp::I64Ne,
                4 => BinaryOp::I64LtS,
                5 => BinaryOp::I64GeS,
                6 => BinaryOp::I64LtU,
                7 => BinaryOp::I64GeU,
                _ => return StepOutcome::Unsupported,
            };
            let condition = b.binary(cmp, lhs, rhs, pc);
            let taken_pc = pc.wrapping_add(imm_b(insn) as u64);
            // Forward branches form a guarded trace along fallthrough. This
            // joins common if/else-shaped basic blocks without constructing a
            // general CFG; the taken edge exits with exact post-branch state.
            // Backward edges remain terminal except for the entry backedge,
            // which the loop recognizer lowers to an internal Wasm loop.
            if trace_forward && taken_pc > pc && taken_pc != fallthrough {
                b.guard(condition, taken_pc, retired + 1);
                metrics.trace_control[0] = metrics.trace_control[0].saturating_add(1);
                StepOutcome::Continue
            } else {
                let taken = b.guest_pc(taken_pc, pc);
                let not_taken = b.guest_pc(fallthrough, pc);
                let target = b.select_i64(condition, taken, not_taken, pc);
                metrics.trace_control[0] = metrics.trace_control[0].saturating_add(1);
                StepOutcome::Exit(target)
            }
        }
        // Integer loads require either a checked flat-memory capability or a
        // full-system fused-translation capability.
        0x03 if allow_memory => {
            let base = b.read_x(rs1(insn), pc);
            let immediate = b.const_i64(imm_i(insn), pc);
            let address = b.binary(BinaryOp::I64Add, base, immediate, pc);
            let (kind, width_index) = match funct3(insn) {
                0 => (LoadKind::I8S, 0),
                1 => (LoadKind::I16S, 1),
                2 => (LoadKind::I32S, 2),
                3 => (LoadKind::I64, 3),
                4 => (LoadKind::I8U, 0),
                5 => (LoadKind::I16U, 1),
                6 => (LoadKind::I32U, 2),
                _ => return StepOutcome::Unsupported,
            };
            let value = b.load(address, kind, pc, retired);
            b.write_x(rd(insn), value);
            metrics.trace_mem[width_index] = metrics.trace_mem[width_index].saturating_add(1);
            if rs1(insn) == 2 {
                metrics.trace_mem[8] = metrics.trace_mem[8].saturating_add(1);
            }
            StepOutcome::Continue
        }
        // Integer stores are ordered explicitly in the IR.
        0x23 if allow_memory => {
            let base = b.read_x(rs1(insn), pc);
            let immediate = b.const_i64(imm_s(insn), pc);
            let address = b.binary(BinaryOp::I64Add, base, immediate, pc);
            let value = b.read_x(rs2(insn), pc);
            let (kind, width_index) = match funct3(insn) {
                0 => (StoreKind::I8, 4),
                1 => (StoreKind::I16, 5),
                2 => (StoreKind::I32, 6),
                3 => (StoreKind::I64, 7),
                _ => return StepOutcome::Unsupported,
            };
            b.store(address, value, kind, pc, retired);
            metrics.trace_mem[width_index] = metrics.trace_mem[width_index].saturating_add(1);
            if rs1(insn) == 2 {
                metrics.trace_mem[9] = metrics.trace_mem[9].saturating_add(1);
            }
            StepOutcome::Continue
        }
        // RVV memory encodings share LOAD-FP/STORE-FP major opcodes but use
        // the otherwise-unused width fields. The typed helper performs the
        // complete mandatory base-V operation against canonical machine state.
        0x07 if allow_memory && allow_vector && matches!(funct3(insn), 0 | 5 | 6 | 7) => {
            b.vector(insn, pc, fallthrough, retired);
            StepOutcome::Continue
        }
        0x27 if allow_memory && allow_vector && matches!(funct3(insn), 0 | 5 | 6 | 7) => {
            b.vector(insn, pc, fallthrough, retired);
            StepOutcome::Continue
        }
        0x57 if allow_vector => {
            b.vector(insn, pc, fallthrough, retired);
            StepOutcome::Continue
        }
        // Floating-point loads preserve raw IEEE bits in the architectural F
        // file. FLW writes a NaN-boxed 32-bit value in RV64.
        0x07 if allow_memory && allow_fp => {
            let base = b.read_x(rs1(insn), pc);
            let immediate = b.const_i64(imm_i(insn), pc);
            let address = b.binary(BinaryOp::I64Add, base, immediate, pc);
            let (kind, width_index) = match funct3(insn) {
                2 => (LoadKind::I32U, 2),
                3 => (LoadKind::I64, 3),
                _ => return StepOutcome::Unsupported,
            };
            if system_fp {
                b.fp_state(true, pc, retired);
            }
            let mut value = b.load(address, kind, pc, retired);
            if funct3(insn) == 2 {
                let box_mask = b.const_i64(0xffff_ffff_0000_0000u64 as i64, pc);
                value = b.binary(BinaryOp::I64Or, value, box_mask, pc);
            }
            b.write_f(rd(insn), value);
            metrics.uses_fp = true;
            metrics.trace_mem[width_index] = metrics.trace_mem[width_index].saturating_add(1);
            if rs1(insn) == 2 {
                metrics.trace_mem[8] = metrics.trace_mem[8].saturating_add(1);
            }
            StepOutcome::Continue
        }
        // Floating-point stores write the low 32 bits for FSW and all bits for
        // FSD; malformed NaN boxes are stored verbatim, as required.
        0x27 if allow_memory && allow_fp => {
            let base = b.read_x(rs1(insn), pc);
            let immediate = b.const_i64(imm_s(insn), pc);
            let address = b.binary(BinaryOp::I64Add, base, immediate, pc);
            let value = b.read_f(rs2(insn), pc);
            let (kind, width_index) = match funct3(insn) {
                2 => (StoreKind::I32, 6),
                3 => (StoreKind::I64, 7),
                _ => return StepOutcome::Unsupported,
            };
            if system_fp {
                b.fp_state(false, pc, retired);
            }
            b.store(address, value, kind, pc, retired);
            metrics.uses_fp = true;
            metrics.trace_mem[width_index] = metrics.trace_mem[width_index].saturating_add(1);
            if rs1(insn) == 2 {
                metrics.trace_mem[9] = metrics.trace_mem[9].saturating_add(1);
            }
            StepOutcome::Continue
        }
        // Fused multiply-add family. All four architectural sign variants
        // share one exact helper operation after applying their prescribed
        // product/addend sign flips to raw operand bits.
        0x43 | 0x47 | 0x4b | 0x4f if allow_fp && lift_fp_fma(b, insn, pc, retired, system_fp) => {
            metrics.uses_fp = true;
            StepOutcome::Continue
        }
        // Bit-exact OP-FP subset: moves and sign injection need no rounding
        // or exception-flag emulation and are naturally expressed as i64 SSA.
        0x53 if allow_fp
            && (lift_fp_bits(b, insn, pc, retired, system_fp)
                || lift_exact_fp(b, insn, pc, retired, system_fp)) =>
        {
            metrics.uses_fp = true;
            StepOutcome::Continue
        }
        // Read-only vector CSRs can remain in generated code. `vlenb` is a
        // machine constant; the ordered state effect preserves the exact
        // full-system VS=Off illegal-instruction boundary without dirtying VS.
        0x73 if allow_vector && lift_vector_csr_read(b, insn, pc, retired) => StepOutcome::Continue,
        // User-mode floating-point CSRs. Full-system compilation keeps these
        // in T0 until mstatus.FS checking/dirtying is part of the typed ABI.
        0x73 if allow_fp && lift_fp_csr(b, insn, pc, retired, system_fp) => {
            metrics.uses_fp = true;
            StepOutcome::Continue
        }
        // FENCE/FENCE.I are architectural no-ops for the emulator's single
        // in-order hart, matching Cpu::step. Generated-code stores invalidate
        // translated code pages eagerly, so FENCE.I needs no deferred cache
        // action either. Keeping these inside a region avoids a high-frequency
        // interpreter boundary in libc and kernel synchronization paths.
        0x0f => StepOutcome::Continue,
        // OP-IMM
        0x13 => {
            let lhs = b.read_x(rs1(insn), pc);
            let immediate = b.const_i64(imm_i(insn), pc);
            let value = match funct3(insn) {
                0 => b.binary(BinaryOp::I64Add, lhs, immediate, pc),
                1 => b.binary(BinaryOp::I64Shl, lhs, immediate, pc),
                2 => {
                    let cmp = b.binary(BinaryOp::I64LtS, lhs, immediate, pc);
                    b.extend_i32_u(cmp, pc)
                }
                3 => {
                    let cmp = b.binary(BinaryOp::I64LtU, lhs, immediate, pc);
                    b.extend_i32_u(cmp, pc)
                }
                4 => b.binary(BinaryOp::I64Xor, lhs, immediate, pc),
                5 if insn >> 26 == 0x10 => b.binary(BinaryOp::I64ShrS, lhs, immediate, pc),
                5 => b.binary(BinaryOp::I64ShrU, lhs, immediate, pc),
                6 => b.binary(BinaryOp::I64Or, lhs, immediate, pc),
                7 => b.binary(BinaryOp::I64And, lhs, immediate, pc),
                _ => unreachable!(),
            };
            b.write_x(rd(insn), value);
            StepOutcome::Continue
        }
        // OP-IMM-32
        0x1b => {
            let lhs64 = b.read_x(rs1(insn), pc);
            let lhs = b.wrap_i32(lhs64, pc);
            let shamt = b.const_i32((imm_i(insn) as u32 & 0x1f) as i32, pc);
            let value32 = match funct3(insn) {
                0 => {
                    let immediate = b.const_i32(imm_i(insn) as i32, pc);
                    b.binary(BinaryOp::I32Add, lhs, immediate, pc)
                }
                1 => b.binary(BinaryOp::I32Shl, lhs, shamt, pc),
                5 if funct7(insn) == 0x20 => b.binary(BinaryOp::I32ShrS, lhs, shamt, pc),
                5 => b.binary(BinaryOp::I32ShrU, lhs, shamt, pc),
                _ => return StepOutcome::Unsupported,
            };
            let value = b.extend_i32_s(value32, pc);
            b.write_x(rd(insn), value);
            StepOutcome::Continue
        }
        // OP, including the directly representable part of M.
        0x33 => {
            let lhs = b.read_x(rs1(insn), pc);
            let rhs = b.read_x(rs2(insn), pc);
            let value = match (funct7(insn), funct3(insn)) {
                (0x00, 0) => b.binary(BinaryOp::I64Add, lhs, rhs, pc),
                (0x20, 0) => b.binary(BinaryOp::I64Sub, lhs, rhs, pc),
                (0x00, 1) => b.binary(BinaryOp::I64Shl, lhs, rhs, pc),
                (0x00, 2) => {
                    let cmp = b.binary(BinaryOp::I64LtS, lhs, rhs, pc);
                    b.extend_i32_u(cmp, pc)
                }
                (0x00, 3) => {
                    let cmp = b.binary(BinaryOp::I64LtU, lhs, rhs, pc);
                    b.extend_i32_u(cmp, pc)
                }
                (0x00, 4) => b.binary(BinaryOp::I64Xor, lhs, rhs, pc),
                (0x00, 5) => b.binary(BinaryOp::I64ShrU, lhs, rhs, pc),
                (0x20, 5) => b.binary(BinaryOp::I64ShrS, lhs, rhs, pc),
                (0x00, 6) => b.binary(BinaryOp::I64Or, lhs, rhs, pc),
                (0x00, 7) => b.binary(BinaryOp::I64And, lhs, rhs, pc),
                (0x01, 0) => b.binary(BinaryOp::I64Mul, lhs, rhs, pc),
                (0x01, 1) => b.mul_high(MulHighKind::SignedSigned, lhs, rhs, pc),
                (0x01, 2) => b.mul_high(MulHighKind::SignedUnsigned, lhs, rhs, pc),
                (0x01, 3) => b.mul_high(MulHighKind::UnsignedUnsigned, lhs, rhs, pc),
                (0x01, 4) => b.divide(DivideOp::I64DivS, lhs, rhs, pc),
                (0x01, 5) => b.divide(DivideOp::I64DivU, lhs, rhs, pc),
                (0x01, 6) => b.divide(DivideOp::I64RemS, lhs, rhs, pc),
                (0x01, 7) => b.divide(DivideOp::I64RemU, lhs, rhs, pc),
                _ => return StepOutcome::Unsupported,
            };
            b.write_x(rd(insn), value);
            StepOutcome::Continue
        }
        // OP-32, including MULW.
        0x3b => {
            let lhs64 = b.read_x(rs1(insn), pc);
            let rhs64 = b.read_x(rs2(insn), pc);
            let lhs = b.wrap_i32(lhs64, pc);
            let rhs = b.wrap_i32(rhs64, pc);
            let value32 = match (funct7(insn), funct3(insn)) {
                (0x00, 0) => b.binary(BinaryOp::I32Add, lhs, rhs, pc),
                (0x20, 0) => b.binary(BinaryOp::I32Sub, lhs, rhs, pc),
                (0x00, 1) => b.binary(BinaryOp::I32Shl, lhs, rhs, pc),
                (0x00, 5) => b.binary(BinaryOp::I32ShrU, lhs, rhs, pc),
                (0x20, 5) => b.binary(BinaryOp::I32ShrS, lhs, rhs, pc),
                (0x01, 0) => b.binary(BinaryOp::I32Mul, lhs, rhs, pc),
                (0x01, 4) => b.divide(DivideOp::I32DivS, lhs, rhs, pc),
                (0x01, 5) => b.divide(DivideOp::I32DivU, lhs, rhs, pc),
                (0x01, 6) => b.divide(DivideOp::I32RemS, lhs, rhs, pc),
                (0x01, 7) => b.divide(DivideOp::I32RemU, lhs, rhs, pc),
                _ => return StepOutcome::Unsupported,
            };
            let value = b.extend_i32_s(value32, pc);
            b.write_x(rd(insn), value);
            StepOutcome::Continue
        }
        // Single-hart memory atomics. LR/SC additionally require a typed
        // reservation helper capability for the concrete machine layout.
        0x2f if allow_memory && lift_amo(b, metrics, insn, pc, retired, allow_reservation) => {
            StepOutcome::Continue
        }
        _ => StepOutcome::Unsupported,
    };

    if !matches!(result, StepOutcome::Unsupported) {
        if matches!(op, 0x03 | 0x07 | 0x23 | 0x27 | 0x2f) && rs1(insn) == 2 {
            metrics.trace_stack_memory = metrics.trace_stack_memory.saturating_add(1);
        }
        match op {
            0x03 | 0x07 => metrics.trace_mix[1] = metrics.trace_mix[1].saturating_add(1),
            0x23 | 0x27 | 0x2f => metrics.trace_mix[2] = metrics.trace_mix[2].saturating_add(1),
            0x43 | 0x47 | 0x4b | 0x4f | 0x53 | 0x73 if metrics.uses_fp => {
                metrics.trace_mix[4] = metrics.trace_mix[4].saturating_add(1)
            }
            0x6f | 0x67 | 0x63 => metrics.trace_mix[3] = metrics.trace_mix[3].saturating_add(1),
            0x33 | 0x3b if funct7(insn) == 0x01 && funct3(insn) <= 3 => {
                metrics.trace_mix[0] = metrics.trace_mix[0].saturating_add(1);
                metrics.trace_alu[3] = metrics.trace_alu[3].saturating_add(1);
            }
            0x33 | 0x3b if funct7(insn) == 0x01 => {
                metrics.trace_mix[0] = metrics.trace_mix[0].saturating_add(1);
                metrics.trace_alu[4] = metrics.trace_alu[4].saturating_add(1);
            }
            _ => {
                metrics.trace_mix[0] = metrics.trace_mix[0].saturating_add(1);
                metrics.trace_alu[0] = metrics.trace_alu[0].saturating_add(1);
            }
        }
    }
    result
}

fn lift_amo(
    b: &mut Builder,
    metrics: &mut Metrics,
    insn: u32,
    pc: u64,
    retired: u32,
    allow_reservation: bool,
) -> bool {
    let funct5 = funct7(insn) >> 2;
    let (load_kind, store_kind, width_index) = match funct3(insn) {
        2 => (LoadKind::I32S, StoreKind::I32, 2),
        3 => (LoadKind::I64, StoreKind::I64, 3),
        _ => return false,
    };
    if funct5 == 0x02 {
        if !allow_reservation {
            return false;
        }
        let address = b.read_x(rs1(insn), pc);
        let old = b.load(address, load_kind, pc, retired);
        b.reservation(ReservationOp::LoadReserved, address, pc);
        b.write_x(rd(insn), old);
        metrics.trace_mem[width_index] = metrics.trace_mem[width_index].saturating_add(1);
        return true;
    }
    if funct5 == 0x03 {
        if !allow_reservation {
            return false;
        }
        let address = b.read_x(rs1(insn), pc);
        let source = b.read_x(rs2(insn), pc);
        let success = b.reservation(ReservationOp::StoreConditional, address, pc);
        b.store_conditional(success, address, source, store_kind, pc, retired);
        let one = b.const_i32(1, pc);
        let failed = b.binary(BinaryOp::I32Xor, success, one, pc);
        let result = b.extend_i32_u(failed, pc);
        b.write_x(rd(insn), result);
        metrics.trace_mem[width_index + 4] = metrics.trace_mem[width_index + 4].saturating_add(1);
        return true;
    }
    if !matches!(
        funct5,
        0x00 | 0x01 | 0x04 | 0x08 | 0x0c | 0x10 | 0x14 | 0x18 | 0x1c
    ) {
        return false;
    }

    let address = b.read_x(rs1(insn), pc);
    let source = b.read_x(rs2(insn), pc);
    let old = b.load(address, load_kind, pc, retired);
    let (compare_old, compare_source) = if funct3(insn) == 2 {
        let old32 = b.wrap_i32(old, pc);
        let source32 = b.wrap_i32(source, pc);
        if matches!(funct5, 0x18 | 0x1c) {
            (b.extend_i32_u(old32, pc), b.extend_i32_u(source32, pc))
        } else {
            (b.extend_i32_s(old32, pc), b.extend_i32_s(source32, pc))
        }
    } else {
        (old, source)
    };

    let new = match funct5 {
        0x01 => source,
        0x00 => b.binary(BinaryOp::I64Add, old, source, pc),
        0x04 => b.binary(BinaryOp::I64Xor, old, source, pc),
        0x0c => b.binary(BinaryOp::I64And, old, source, pc),
        0x08 => b.binary(BinaryOp::I64Or, old, source, pc),
        0x10 | 0x14 => {
            let old_lt_source = b.binary(BinaryOp::I64LtS, compare_old, compare_source, pc);
            let choose_old = if funct5 == 0x10 {
                old_lt_source
            } else {
                let one = b.const_i32(1, pc);
                b.binary(BinaryOp::I32Xor, old_lt_source, one, pc)
            };
            b.select_i64(choose_old, old, source, pc)
        }
        0x18 | 0x1c => {
            let old_lt_source = b.binary(BinaryOp::I64LtU, compare_old, compare_source, pc);
            let choose_old = if funct5 == 0x18 {
                old_lt_source
            } else {
                let one = b.const_i32(1, pc);
                b.binary(BinaryOp::I32Xor, old_lt_source, one, pc)
            };
            b.select_i64(choose_old, old, source, pc)
        }
        _ => unreachable!(),
    };

    // Record the store side exit before publishing rd: a failing AMO has not
    // written its architectural destination. The preceding load uses the same
    // address/width and normally proves this second check, but retaining both
    // effects keeps precise semantics explicit until a bounds-check CSE pass.
    b.store(address, new, store_kind, pc, retired);
    b.write_x(rd(insn), old);
    metrics.trace_mem[width_index] = metrics.trace_mem[width_index].saturating_add(1);
    metrics.trace_mem[width_index + 4] = metrics.trace_mem[width_index + 4].saturating_add(1);
    true
}

fn lift_fp_bits(b: &mut Builder, insn: u32, pc: u64, retired: u32, system_fp: bool) -> bool {
    let destination = rd(insn);
    let source1 = rs1(insn);
    let source2 = rs2(insn);
    match (funct7(insn), funct3(insn), source2) {
        // FSGNJ{,N,X}.S and .D.
        (0x10, mode @ 0..=2, _) | (0x11, mode @ 0..=2, _) => {
            if system_fp {
                b.fp_state(true, pc, retired);
            }
            let single = funct7(insn) == 0x10;
            let mut lhs = b.read_f(source1, pc);
            let mut rhs = b.read_f(source2, pc);
            let sign = if single {
                lhs = unbox_f32(b, lhs, pc);
                rhs = unbox_f32(b, rhs, pc);
                0x8000_0000
            } else {
                i64::MIN
            };
            let sign_mask = b.const_i64(sign, pc);
            let rhs_sign = b.binary(BinaryOp::I64And, rhs, sign_mask, pc);
            let value = if mode == 2 {
                b.binary(BinaryOp::I64Xor, lhs, rhs_sign, pc)
            } else {
                let magnitude_mask = b.const_i64(!sign, pc);
                let magnitude = b.binary(BinaryOp::I64And, lhs, magnitude_mask, pc);
                let selected_sign = if mode == 1 {
                    b.binary(BinaryOp::I64Xor, rhs_sign, sign_mask, pc)
                } else {
                    rhs_sign
                };
                b.binary(BinaryOp::I64Or, magnitude, selected_sign, pc)
            };
            b.write_f(destination, value);
            true
        }
        // FMV.X.W / FMV.X.D.
        (0x70, 0, 0) => {
            if system_fp {
                b.fp_state(true, pc, retired);
            }
            let bits = b.read_f(source1, pc);
            let low = b.wrap_i32(bits, pc);
            let value = b.extend_i32_s(low, pc);
            b.write_x(destination, value);
            true
        }
        (0x71, 0, 0) => {
            if system_fp {
                b.fp_state(true, pc, retired);
            }
            let bits = b.read_f(source1, pc);
            b.write_x(destination, bits);
            true
        }
        // FMV.W.X / FMV.D.X.
        (0x78, 0, 0) => {
            if system_fp {
                b.fp_state(true, pc, retired);
            }
            let bits = b.read_x(source1, pc);
            let low = b.wrap_i32(bits, pc);
            let low = b.extend_i32_u(low, pc);
            let box_mask = b.const_i64(0xffff_ffff_0000_0000u64 as i64, pc);
            let boxed = b.binary(BinaryOp::I64Or, low, box_mask, pc);
            b.write_f(destination, boxed);
            true
        }
        (0x79, 0, 0) => {
            if system_fp {
                b.fp_state(true, pc, retired);
            }
            let bits = b.read_x(source1, pc);
            b.write_f(destination, bits);
            true
        }
        _ => false,
    }
}

fn lift_rounding_mode(b: &mut Builder, field: u32, pc: u64) -> Option<ValueId> {
    match field {
        mode @ 0..=4 => b.const_i32(mode as i32, pc),
        7 => {
            let fcsr = b.read_fcsr(pc);
            let shift = b.const_i32(5, pc);
            let frm = b.binary(BinaryOp::I32ShrU, fcsr, shift, pc);
            let mask = b.const_i32(7, pc);
            b.binary(BinaryOp::I32And, frm, mask, pc)
        }
        _ => return None,
    }
    .into()
}

fn lift_exact_fp(b: &mut Builder, insn: u32, pc: u64, retired: u32, system_fp: bool) -> bool {
    let f7 = funct7(insn);
    let f3 = funct3(insn);
    let s2 = rs2(insn);
    // (operation, consumes rm, writes an X register, lhs comes from X,
    //  has a second floating-point operand)
    let (op, uses_rm, writes_x, lhs_is_x, has_rhs) = match (f7, f3, s2) {
        (0x00, _, _) => (ExactFpOp::Add32, true, false, false, true),
        (0x04, _, _) => (ExactFpOp::Sub32, true, false, false, true),
        (0x08, _, _) => (ExactFpOp::Mul32, true, false, false, true),
        (0x0c, _, _) => (ExactFpOp::Div32, true, false, false, true),
        (0x01, _, _) => (ExactFpOp::Add64, true, false, false, true),
        (0x05, _, _) => (ExactFpOp::Sub64, true, false, false, true),
        (0x09, _, _) => (ExactFpOp::Mul64, true, false, false, true),
        (0x0d, _, _) => (ExactFpOp::Div64, true, false, false, true),
        (0x2c, _, 0) => (ExactFpOp::Sqrt32, true, false, false, false),
        (0x2d, _, 0) => (ExactFpOp::Sqrt64, true, false, false, false),
        (0x14, 0, _) => (ExactFpOp::Min32, false, false, false, true),
        (0x14, 1, _) => (ExactFpOp::Max32, false, false, false, true),
        (0x15, 0, _) => (ExactFpOp::Min64, false, false, false, true),
        (0x15, 1, _) => (ExactFpOp::Max64, false, false, false, true),
        (0x50, 2, _) => (ExactFpOp::Eq32, false, true, false, true),
        (0x50, 1, _) => (ExactFpOp::Lt32, false, true, false, true),
        (0x50, 0, _) => (ExactFpOp::Le32, false, true, false, true),
        (0x51, 2, _) => (ExactFpOp::Eq64, false, true, false, true),
        (0x51, 1, _) => (ExactFpOp::Lt64, false, true, false, true),
        (0x51, 0, _) => (ExactFpOp::Le64, false, true, false, true),
        (0x20, _, 1) => (ExactFpOp::Cvt32From64, true, false, false, false),
        (0x21, _, 0) => (ExactFpOp::Cvt64From32, false, false, false, false),
        (0x60, _, 0) => (ExactFpOp::CvtI32From32, true, true, false, false),
        (0x60, _, 1) => (ExactFpOp::CvtU32From32, true, true, false, false),
        (0x60, _, 2) => (ExactFpOp::CvtI64From32, true, true, false, false),
        (0x60, _, 3) => (ExactFpOp::CvtU64From32, true, true, false, false),
        (0x61, _, 0) => (ExactFpOp::CvtI32From64, true, true, false, false),
        (0x61, _, 1) => (ExactFpOp::CvtU32From64, true, true, false, false),
        (0x61, _, 2) => (ExactFpOp::CvtI64From64, true, true, false, false),
        (0x61, _, 3) => (ExactFpOp::CvtU64From64, true, true, false, false),
        (0x68, _, 0) => (ExactFpOp::Cvt32FromI32, true, false, true, false),
        (0x68, _, 1) => (ExactFpOp::Cvt32FromU32, true, false, true, false),
        (0x68, _, 2) => (ExactFpOp::Cvt32FromI64, true, false, true, false),
        (0x68, _, 3) => (ExactFpOp::Cvt32FromU64, true, false, true, false),
        (0x69, _, 0) => (ExactFpOp::Cvt64FromI32, true, false, true, false),
        (0x69, _, 1) => (ExactFpOp::Cvt64FromU32, true, false, true, false),
        (0x69, _, 2) => (ExactFpOp::Cvt64FromI64, true, false, true, false),
        (0x69, _, 3) => (ExactFpOp::Cvt64FromU64, true, false, true, false),
        (0x70, 1, 0) => (ExactFpOp::Class32, false, true, false, false),
        (0x71, 1, 0) => (ExactFpOp::Class64, false, true, false, false),
        _ => return false,
    };
    let rm = if uses_rm {
        let Some(rm) = lift_rounding_mode(b, f3, pc) else {
            return false;
        };
        rm
    } else {
        b.const_i32(0, pc)
    };
    if system_fp {
        b.fp_state(true, pc, retired);
    }
    let lhs = if lhs_is_x {
        b.read_x(rs1(insn), pc)
    } else {
        b.read_f(rs1(insn), pc)
    };
    let rhs = if has_rhs {
        b.read_f(s2, pc)
    } else {
        b.const_i64(0, pc)
    };
    let third = b.const_i64(0, pc);
    let result = b.exact_fp(op, lhs, rhs, third, rm, pc, retired);
    if writes_x {
        b.write_x(rd(insn), result);
    } else {
        b.write_f(rd(insn), result);
    }
    true
}

fn lift_fp_fma(b: &mut Builder, insn: u32, pc: u64, retired: u32, system_fp: bool) -> bool {
    let (op, sign_mask) = match (insn >> 25) & 3 {
        0 => (ExactFpOp::Fma32, 0x8000_0000),
        1 => (ExactFpOp::Fma64, i64::MIN),
        _ => return false,
    };
    let Some(rm) = lift_rounding_mode(b, funct3(insn), pc) else {
        return false;
    };
    if system_fp {
        b.fp_state(true, pc, retired);
    }
    let mut lhs = b.read_f(rs1(insn), pc);
    let rhs = b.read_f(rs2(insn), pc);
    let mut third = b.read_f((insn >> 27) as usize, pc);
    let mask = b.const_i64(sign_mask, pc);
    if matches!(opcode(insn), 0x4b | 0x4f) {
        lhs = b.binary(BinaryOp::I64Xor, lhs, mask, pc);
    }
    if matches!(opcode(insn), 0x47 | 0x4f) {
        third = b.binary(BinaryOp::I64Xor, third, mask, pc);
    }
    let result = b.exact_fp(op, lhs, rhs, third, rm, pc, retired);
    b.write_f(rd(insn), result);
    true
}

fn lift_fp_csr(b: &mut Builder, insn: u32, pc: u64, retired: u32, system_fp: bool) -> bool {
    let f3 = funct3(insn);
    let csr = insn >> 20;
    if !(1..=3).contains(&csr) || !matches!(f3, 1..=3 | 5..=7) {
        return false;
    }
    if system_fp {
        b.fp_state(true, pc, retired);
    }

    let (shift_bits, value_mask, field_mask) = match csr {
        1 => (0, 0x1f, 0x1f),
        2 => (5, 0x07, 0xe0),
        3 => (0, 0xff, 0xff),
        _ => unreachable!(),
    };
    let old_fcsr = b.read_fcsr(pc);
    let shift = b.const_i32(shift_bits, pc);
    let old_value = if shift_bits == 0 {
        old_fcsr
    } else {
        b.binary(BinaryOp::I32ShrU, old_fcsr, shift, pc)
    };
    let value_mask_node = b.const_i32(value_mask, pc);
    let old_value = b.binary(BinaryOp::I32And, old_value, value_mask_node, pc);

    // Source must be captured before rd is written; rd and rs1 may alias.
    let source = if f3 >= 5 {
        b.const_i32(rs1(insn) as i32, pc)
    } else {
        let source = b.read_x(rs1(insn), pc);
        b.wrap_i32(source, pc)
    };
    let source = b.binary(BinaryOp::I32And, source, value_mask_node, pc);
    let positioned = if shift_bits == 0 {
        source
    } else {
        b.binary(BinaryOp::I32Shl, source, shift, pc)
    };
    let source_is_statically_zero = rs1(insn) == 0;
    let write = match f3 & 3 {
        1 => Some(positioned),
        2 if !source_is_statically_zero => {
            Some(b.binary(BinaryOp::I32Or, old_fcsr, positioned, pc))
        }
        3 if !source_is_statically_zero => {
            let all_ones = b.const_i32(-1, pc);
            let inverse = b.binary(BinaryOp::I32Xor, positioned, all_ones, pc);
            Some(b.binary(BinaryOp::I32And, old_fcsr, inverse, pc))
        }
        _ => None,
    };
    if f3 & 3 == 1 {
        let preserve_mask = b.const_i32(!field_mask, pc);
        let preserved = b.binary(BinaryOp::I32And, old_fcsr, preserve_mask, pc);
        let new_fcsr = b.binary(BinaryOp::I32Or, preserved, write.unwrap(), pc);
        b.write_fcsr(new_fcsr);
    } else if let Some(new_fcsr) = write {
        b.write_fcsr(new_fcsr);
    }
    let old_value = b.extend_i32_u(old_value, pc);
    b.write_x(rd(insn), old_value);
    true
}

fn lift_vector_csr_read(b: &mut Builder, insn: u32, pc: u64, retired: u32) -> bool {
    const VLENB: u32 = 0xc22;
    let f3 = funct3(insn);
    let read_only = matches!(f3, 2 | 3 | 6 | 7) && rs1(insn) == 0;
    if insn >> 20 != VLENB || !read_only {
        return false;
    }

    b.vector_state(pc, retired);
    let value = b.const_i64(rv64_core::cpu::VLEN_BYTES as i64, pc);
    b.write_x(rd(insn), value);
    true
}

/// RV64 NaN-boxing: a single-precision source whose upper bits are not all
/// ones behaves as the canonical quiet NaN for computational instructions.
fn unbox_f32(b: &mut Builder, value: ValueId, pc: u64) -> ValueId {
    let shift = b.const_i64(32, pc);
    let upper = b.binary(BinaryOp::I64ShrU, value, shift, pc);
    let ones = b.const_i64(0xffff_ffff, pc);
    let valid = b.binary(BinaryOp::I64Eq, upper, ones, pc);
    let canonical = b.const_i64(0xffff_ffff_7fc0_0000u64 as i64, pc);
    b.select_i64(valid, value, canonical, pc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Effect, Op};

    fn enc_i(op: u32, rd: u32, f3: u32, rs1: u32, imm: i32) -> u32 {
        op | (rd << 7) | (f3 << 12) | (rs1 << 15) | (((imm as u32) & 0xfff) << 20)
    }

    fn enc_r(op: u32, rd: u32, f3: u32, rs1: u32, rs2: u32, f7: u32) -> u32 {
        op | (rd << 7) | (f3 << 12) | (rs1 << 15) | (rs2 << 20) | (f7 << 25)
    }

    fn enc_s(op: u32, f3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
        let imm = imm as u32 & 0xfff;
        op | ((imm & 0x1f) << 7) | (f3 << 12) | (rs1 << 15) | (rs2 << 20) | ((imm >> 5) << 25)
    }

    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn lifts_and_forwards_a_straight_line_until_an_unsupported_instruction() {
        let code = bytes(&[
            enc_i(0x13, 1, 0, 0, 5),
            enc_i(0x13, 2, 0, 1, 7),
            enc_r(0x33, 3, 0, 1, 2, 0),
            0x0000_0073,
        ]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, false, FpMode::Disabled, false).unwrap();
        assert_eq!(lifted.ir.retired, 3);
        assert_eq!(lifted.ir.end_pc, 0x100c);
        assert_eq!(lifted.ir.exit, ExitKind::Unsupported);
        assert_eq!(lifted.ir.outputs.len(), 3);
        assert!(lifted.ir.validate().is_ok());
        assert!(lifted
            .ir
            .values
            .iter()
            .all(|value| !matches!(value.op, Op::ReadX(_))));
    }

    #[test]
    fn lifts_fence_and_fence_i_as_retired_single_hart_noops() {
        let code = bytes(&[
            0x0aa0_000f,             // fence rw,rw
            0x0000_100f,             // fence.i
            enc_i(0x13, 1, 0, 0, 7), // addi x1,x0,7
            0x0000_0073,             // ecall
        ]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, false, FpMode::Disabled, false).unwrap();
        assert_eq!(lifted.ir.retired, 3);
        assert_eq!(lifted.ir.end_pc, 0x100c);
        assert_eq!(lifted.ir.exit, ExitKind::Unsupported);
        assert_eq!(lifted.ir.outputs.len(), 1);
        assert!(lifted.ir.effects.is_empty());
        assert!(lifted.ir.validate().is_ok());
    }

    #[test]
    fn expands_compressed_instructions_before_lifting() {
        // c.li a0, 1 followed by ecall.
        let mut code = 0x4505u16.to_le_bytes().to_vec();
        code.extend_from_slice(&0x0000_0073u32.to_le_bytes());
        let lifted = lift_t1(&code, 0x2000, 0x2000, false, FpMode::Disabled, false).unwrap();
        assert_eq!(lifted.ir.retired, 1);
        assert_eq!(lifted.byte_len, 2);
        assert_eq!(lifted.ir.end_pc, 0x2002);
    }

    #[test]
    fn jalr_keeps_the_old_source_when_rd_aliases_rs1() {
        let code = bytes(&[enc_i(0x67, 1, 0, 1, 8)]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, false, FpMode::Disabled, false).unwrap();
        assert_eq!(lifted.ir.outputs.len(), 1);
        assert!(lifted
            .ir
            .values
            .iter()
            .any(|value| matches!(value.op, Op::ReadX(1))));
    }

    #[test]
    fn profiled_indirect_target_is_a_precise_dynamic_guard() {
        let mut code = bytes(&[
            enc_i(0x13, 5, 0, 0, 0x10), // addi x5,x0,0x10
            enc_i(0x67, 0, 0, 5, 0),    // jalr x0,0(x5)
            0x0000_0013,
            0x0000_0013,
            enc_i(0x13, 6, 0, 6, 1), // observed target: addi x6,x6,1
            0x0000_0073,
        ]);
        code.resize(32, 0);
        let lifted =
            lift_t1_profiled(&code, 0, 0, false, FpMode::Disabled, false, Some(0x10)).unwrap();
        assert_eq!(lifted.seeds, vec![0, 0x10]);
        assert!(lifted.ir.effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::GuardTarget {
                    expected: 0x10,
                    exit,
                    ..
                } if exit.retired == 2
            )
        }));
        assert!(lifted.ir.validate().is_ok());
    }

    #[test]
    fn profiled_indirect_cycle_becomes_an_unconditional_fuel_loop() {
        let code = bytes(&[
            enc_i(0x13, 5, 0, 0, 8),
            enc_i(0x67, 0, 0, 5, 0),
            enc_i(0x13, 6, 0, 6, 1),
            enc_i(0x13, 5, 0, 0, 0),
            enc_i(0x67, 0, 0, 5, 0),
        ]);
        let lifted =
            lift_t1_profiled_targets(&code, 0, 0, false, FpMode::Disabled, false, &[8, 0]).unwrap();
        assert_eq!(lifted.ir.retired, 5);
        assert_eq!(lifted.seeds, vec![0, 8]);
        assert_eq!(
            lifted.loop_backedge,
            Some(LoopBackedge {
                condition: None,
                exit_pc: 0,
            })
        );
        assert_eq!(
            lifted
                .ir
                .effects
                .iter()
                .filter(|effect| matches!(effect, Effect::GuardTarget { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn recognizes_a_conditional_backedge_to_the_region_entry() {
        // addi x1,x1,1; bne x1,x2,-4
        let branch = 0xfe20_9ee3;
        let code = bytes(&[enc_i(0x13, 1, 0, 1, 1), branch]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, false, FpMode::Disabled, false).unwrap();
        let loop_info = lifted.loop_backedge.expect("must recognize loop");
        assert_eq!(loop_info.exit_pc, 0x1008);
    }

    #[test]
    fn forms_a_precise_guarded_trace_across_a_forward_branch() {
        // addi x1,x0,1; beq x2,x0,+8; addi x1,x1,2; addi x3,x0,7; ecall
        let code = bytes(&[
            enc_i(0x13, 1, 0, 0, 1),
            0x0001_0463,
            enc_i(0x13, 1, 0, 1, 2),
            enc_i(0x13, 3, 0, 0, 7),
            0x0000_0073,
        ]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, false, FpMode::Disabled, false).unwrap();
        assert_eq!(lifted.ir.retired, 4);
        assert_eq!(lifted.ir.end_pc, 0x1010);
        let guard = lifted
            .ir
            .effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Guard { exit, .. } => Some(exit),
                _ => None,
            })
            .expect("forward branch must form a guard");
        assert_eq!(guard.guest_pc, 0x100c);
        assert_eq!(guard.retired, 2);
        assert_eq!(guard.outputs.len(), 1);
        assert!(lifted.ir.validate().is_ok());
    }

    #[test]
    fn follows_a_forward_direct_call_until_its_indirect_return() {
        let code = bytes(&[
            0x0080_00efu32,           // jal x1,+8
            enc_i(0x13, 2, 0, 0, 99), // skipped
            enc_i(0x13, 3, 0, 0, 7),  // call target
            enc_i(0x67, 0, 0, 1, 0),  // jalr x0,0(x1)
        ]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, false, FpMode::Disabled, false).unwrap();
        assert_eq!(lifted.ir.retired, 3);
        assert_eq!(lifted.ir.end_pc, 0x1010);
        assert!(lifted.ir.outputs.iter().any(|&(reg, _)| reg == 1));
        assert!(lifted.ir.outputs.iter().any(|&(reg, _)| reg == 3));
        assert!(!lifted.ir.outputs.iter().any(|&(reg, _)| reg == 2));
        assert!(matches!(
            lifted.ir.values[lifted.ir.next_pc.0].op,
            Op::GuestPc(0x1004)
        ));
    }

    #[test]
    fn lifts_flat_memory_with_precise_pre_instruction_exits() {
        let code = bytes(&[
            enc_i(0x13, 1, 0, 0, 64),
            enc_i(0x13, 2, 0, 0, -1),
            enc_s(0x23, 2, 1, 2, 4), // sw x2,4(x1)
            enc_i(0x03, 3, 6, 1, 4), // lwu x3,4(x1)
            0x0000_0073,
        ]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, true, FpMode::User, false).unwrap();
        assert_eq!(lifted.ir.retired, 4);
        assert_eq!(lifted.metrics.trace_mix, [2, 1, 1, 0, 0]);
        assert_eq!(lifted.metrics.trace_mem[2], 1);
        assert_eq!(lifted.metrics.trace_mem[6], 1);
        assert_eq!(lifted.ir.effects.len(), 1);
        assert!(lifted.ir.has_effects());
        assert!(lifted.ir.validate().is_ok());

        let Effect::Store { exit, .. } = &lifted.ir.effects[0] else {
            panic!("first effect must be the store")
        };
        assert_eq!(exit.guest_pc, 0x1008);
        assert_eq!(exit.retired, 2);
        let load_exit = lifted
            .ir
            .values
            .iter()
            .find_map(|value| match &value.op {
                Op::Load { exit, .. } => Some(exit),
                _ => None,
            })
            .unwrap();
        assert_eq!(load_exit.guest_pc, 0x100c);
        assert_eq!(load_exit.retired, 3);
    }

    #[test]
    fn vector_capability_lifts_complete_ordered_effects_and_breaks_ssa_forwarding() {
        let words = [
            enc_i(0x13, 16, 0, 8, -128), // addi a6,s0,-128
            0xcc08_7057,                 // vsetivli zero,16,e8,m1,ta,ma
            0x0285_0457,                 // vadd.vx v8,v8,a0
            0x0207_0407,                 // vle8.v v8,(a4)
            0x0208_0427,                 // vse8.v v8,(a6)
            enc_i(0x13, 17, 0, 16, 1),   // addi a7,a6,1
            0x0000_0073,                 // ecall
        ];
        let code = bytes(&words);

        let scalar_only = lift_t1(&code, 0x1000, 0x1000, true, FpMode::User, false).unwrap();
        assert_eq!(scalar_only.ir.retired, 1);
        assert!(!scalar_only.ir.has_vector_helper());

        let lifted = lift_t1_with_vector(&code, 0x1000, 0x1000, true, FpMode::User, false, true)
            .expect("typed vector capability lifts the full sequence");
        assert_eq!(lifted.ir.retired, 6);
        assert_eq!(
            lifted
                .ir
                .effects
                .iter()
                .filter(|effect| matches!(effect, Effect::Vector { .. }))
                .count(),
            4
        );
        let first = lifted
            .ir
            .effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Vector {
                    insn,
                    fallthrough,
                    exit,
                    ..
                } => Some((*insn, *fallthrough, exit)),
                _ => None,
            })
            .expect("first RVV instruction is an effect");
        assert_eq!(first.0, words[1]);
        assert_eq!(first.1, 0x1008);
        assert_eq!(first.2.guest_pc, 0x1004);
        assert_eq!(first.2.retired, 1);
        assert!(first.2.outputs.iter().any(|&(reg, _)| reg == 16));

        // The post-vector add must reread a6 from canonical state rather than
        // forwarding the pre-vector value through the opaque boundary.
        assert!(lifted
            .ir
            .values
            .iter()
            .any(|value| matches!(value.op, Op::ReadX(16)) && value.guest_pc == 0x1014));
        assert!(lifted.ir.validate().is_ok());
    }

    #[test]
    fn lifts_read_only_vlenb_with_an_exact_vector_state_boundary() {
        let csrr_vlenb = enc_i(0x73, 10, 2, 0, 0xc22);
        let code = bytes(&[csrr_vlenb, enc_i(0x13, 11, 0, 10, 1), 0x0000_0073]);

        assert!(
            lift_t1_with_vector(&code, 0x1000, 0x1000, false, FpMode::Disabled, false, false,)
                .is_none()
        );

        let lifted =
            lift_t1_with_vector(&code, 0x1000, 0x1000, false, FpMode::Disabled, false, true)
                .expect("vector capability should lift vlenb and its scalar consumer");
        assert_eq!(lifted.ir.retired, 2);
        let Effect::VectorState { exit, .. } = &lifted.ir.effects[0] else {
            panic!("vlenb must retain its full-system VS check")
        };
        assert_eq!(exit.guest_pc, 0x1000);
        assert_eq!(exit.retired, 0);
        assert!(lifted.ir.values.iter().any(
            |value| matches!(value.op, Op::ConstI64(value) if value == rv64_core::cpu::VLEN_BYTES as i64)
        ));
        assert!(lifted.ir.outputs.iter().any(|&(reg, _)| reg == 10));
        assert!(lifted.ir.outputs.iter().any(|&(reg, _)| reg == 11));
        assert!(lifted.ir.validate().is_ok());

        let illegal_write = bytes(&[enc_i(0x73, 10, 1, 0, 0xc22)]);
        assert!(lift_t1_with_vector(
            &illegal_write,
            0x1000,
            0x1000,
            false,
            FpMode::Disabled,
            false,
            true,
        )
        .is_none());
    }

    #[test]
    fn vector_regions_leave_backedges_to_the_outer_dispatcher() {
        let code = bytes(&[
            0x0285_0457,              // vadd.vx v8,v8,a0
            enc_i(0x13, 1, 0, 1, -1), // addi x1,x1,-1
            0xfe00_9ce3,              // bnez x1,-8
        ]);
        let lifted = lift_t1_with_vector(&code, 0x2000, 0x2000, true, FpMode::User, false, true)
            .expect("vector loop lifts");
        assert!(lifted.ir.has_vector_helper());
        assert!(lifted.loop_backedge.is_none());
    }

    #[test]
    fn leaves_memory_for_the_interpreter_without_a_flat_capability() {
        let code = bytes(&[enc_i(0x03, 1, 3, 2, 0)]); // ld x1,0(sp)
        assert!(lift_t1(&code, 0x1000, 0x1000, false, FpMode::Disabled, false).is_none());
    }

    #[test]
    fn lifts_bit_exact_fp_moves_and_sign_injection() {
        let code = bytes(&[
            enc_r(0x53, 1, 0, 2, 0, 0x79), // fmv.d.x f1,x2
            enc_r(0x53, 3, 1, 1, 1, 0x11), // fsgnjn.d f3,f1,f1
            enc_r(0x53, 4, 0, 3, 0, 0x71), // fmv.x.d x4,f3
            0x0000_0073,
        ]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, true, FpMode::User, false).unwrap();
        assert_eq!(lifted.ir.retired, 3);
        assert!(lifted.metrics.uses_fp);
        assert!(lifted.ir.f_outputs.iter().any(|&(reg, _)| reg == 1));
        assert!(lifted.ir.f_outputs.iter().any(|&(reg, _)| reg == 3));
        assert!(lifted.ir.outputs.iter().any(|&(reg, _)| reg == 4));
        assert!(lifted.ir.validate().is_ok());
    }

    #[test]
    fn lifts_nan_boxed_fp_memory_operations() {
        let code = bytes(&[
            enc_i(0x07, 1, 2, 2, 0), // flw f1,0(sp)
            enc_s(0x27, 3, 2, 1, 8), // fsd f1,8(sp)
            0x0000_0073,
        ]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, true, FpMode::User, false).unwrap();
        assert_eq!(lifted.ir.retired, 2);
        assert!(lifted.ir.has_effects());
        assert_eq!(lifted.ir.f_outputs.len(), 1);
        let Effect::Store { exit, .. } = &lifted.ir.effects[0] else {
            panic!("second instruction must be a store effect")
        };
        assert_eq!(exit.f_outputs.len(), 1);
        assert_eq!(exit.retired, 1);
        assert!(lifted.ir.validate().is_ok());
    }

    #[test]
    fn lifts_user_fp_csr_read_modify_write_semantics() {
        let code = bytes(&[
            enc_i(0x13, 1, 0, 0, 3),
            enc_i(0x73, 2, 1, 1, 2),  // csrrw x2,frm,x1
            enc_i(0x73, 3, 2, 0, 2),  // csrrs x3,frm,x0
            enc_i(0x73, 0, 5, 31, 1), // csrwi fflags,31
            enc_i(0x73, 4, 2, 0, 3),  // csrrs x4,fcsr,x0
            0x0000_0073,
        ]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, true, FpMode::User, false).unwrap();
        assert_eq!(lifted.ir.retired, 5);
        assert!(lifted.ir.fcsr_output.is_some());
        assert!(lifted.ir.outputs.iter().any(|&(reg, _)| reg == 4));
        assert!(lifted.ir.validate().is_ok());
    }

    #[test]
    fn lifts_exact_fp_arithmetic_with_dynamic_rounding_state() {
        let code = bytes(&[
            enc_r(0x53, 3, 7, 1, 2, 0x01), // fadd.d f3,f1,f2,dyn
            0x0000_0073,
        ]);
        let lifted = lift_t1(&code, 0x1000, 0x1000, true, FpMode::User, false).unwrap();
        assert_eq!(lifted.ir.retired, 1);
        assert!(lifted.ir.has_fp_helper());
        assert!(lifted.ir.fcsr_output.is_some());
        assert!(lifted.ir.values.iter().any(|value| {
            matches!(
                value.op,
                Op::ExactFp {
                    op: ExactFpOp::Add64,
                    ..
                }
            )
        }));
        assert!(lifted.ir.validate().is_ok());
    }
}
