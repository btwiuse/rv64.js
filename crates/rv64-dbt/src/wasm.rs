//! WebAssembly emitter for T1 regions.
//!
//! Pure regions use a small stackifier. Regions containing guest-memory
//! effects use one local per SSA value so every precise side exit can commit
//! the architectural state that existed immediately before the faulting
//! instruction.

use crate::ir::{
    BinaryOp, DivideOp, Effect, ExactFpOp, LoadKind, Op, Region, ReservationOp, SideExit,
    StoreKind, ValueData, ValueId, ValueType,
};
use crate::lift::LoopBackedge;
use crate::structure::{self, Structure};
use crate::{
    JitLayout, MultiEntryState, ReservationCapability, SystemMemory, TlbMissPolicy, TranslationRow,
};
use std::collections::VecDeque;
use std::fmt;
use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    GlobalType, ImportSection, Instruction, MemArg, MemoryType, Module, TypeSection, ValType,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EmitError(pub String);

impl fmt::Display for EmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EmitError {}

#[derive(Clone, Copy, Debug, Default)]
struct HelperImports {
    fp: bool,
    reservation: Option<ReservationCapability>,
    tlb_fill: bool,
    bulk_copy: bool,
    chain: bool,
    tail_chain: bool,
}

impl HelperImports {
    fn for_region(region: &Region, layout: JitLayout) -> Self {
        Self {
            fp: region.has_fp_helper(),
            reservation: region
                .has_reservation_helper()
                .then_some(layout.reservation)
                .flatten(),
            tlb_fill: region_uses_memory(region)
                && layout
                    .sys
                    .is_some_and(|memory| memory.miss == TlbMissPolicy::Refill),
            bulk_copy: layout.sys.is_some() && bulk_copy_loop_plan(region).is_some(),
            chain: false,
            tail_chain: false,
        }
    }

    const fn fp_index(self) -> Option<u32> {
        if self.fp {
            Some(0)
        } else {
            None
        }
    }

    const fn reservation_index(self) -> Option<u32> {
        if self.reservation.is_some() {
            Some(self.fp as u32)
        } else {
            None
        }
    }

    const fn tlb_fill_index(self) -> Option<u32> {
        if self.tlb_fill {
            Some(self.fp as u32 + self.reservation.is_some() as u32)
        } else {
            None
        }
    }

    const fn bulk_copy_index(self) -> Option<u32> {
        if self.bulk_copy {
            Some(self.fp as u32 + self.reservation.is_some() as u32 + self.tlb_fill as u32)
        } else {
            None
        }
    }

    const fn count(self) -> u32 {
        self.fp as u32
            + self.reservation.is_some() as u32
            + self.tlb_fill as u32
            + self.bulk_copy as u32
            + self.chain as u32
            + self.tail_chain as u32
    }

    const fn chain_index(self) -> Option<u32> {
        if self.chain {
            Some(
                self.fp as u32
                    + self.reservation.is_some() as u32
                    + self.tlb_fill as u32
                    + self.bulk_copy as u32,
            )
        } else {
            None
        }
    }

    const fn tail_chain_index(self) -> Option<u32> {
        if self.tail_chain {
            Some(
                self.fp as u32
                    + self.reservation.is_some() as u32
                    + self.tlb_fill as u32
                    + self.bulk_copy as u32
                    + self.chain as u32,
            )
        } else {
            None
        }
    }

    fn include(&mut self, other: Self) -> Result<(), EmitError> {
        self.fp |= other.fp;
        self.tlb_fill |= other.tlb_fill;
        self.bulk_copy |= other.bulk_copy;
        self.chain |= other.chain;
        self.tail_chain |= other.tail_chain;
        match (self.reservation, other.reservation) {
            (Some(lhs), Some(rhs)) if lhs != rhs => {
                return Err(EmitError(
                    "one module cannot mix user and system reservation capabilities".into(),
                ));
            }
            (None, reservation) => self.reservation = reservation,
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct MemoryTemps {
    index: u32,
    offset: u32,
    context: u32,
    page: Option<u32>,
    load_cache: Option<TranslationCacheTemps>,
    store_cache: Option<TranslationCacheTemps>,
    copy: Option<DenseCopyTemps>,
    bulk_copy: Option<BulkCopyTemps>,
}

#[derive(Clone, Copy, Debug)]
struct TranslationCacheTemps {
    page: u32,
    offset: u32,
}

#[derive(Clone, Copy, Debug)]
struct DenseCopyTemps {
    source_address: u32,
    destination_address: u32,
    source_linear: u32,
    destination_linear: u32,
}

#[derive(Clone, Copy, Debug)]
struct BulkCopyTemps {
    request: u32,
    fuel_bytes: u32,
    result: u32,
}

// RV64C's architecture-defined compact register bank, plus the dedicated
// return-address and stack-pointer registers used by compressed control/stack
// forms.  This is deliberately invariant across guests, PCs, and host engines.
const STRUCTURED_RESIDENT_X_MASK: u32 = 0x0000_ff06;

#[derive(Clone, Debug)]
struct CachedStateLocals {
    x: [Option<u32>; 32],
    /// Referenced integer registers whose canonical CPU cells are synchronized
    /// at every structured member boundary instead of retained function-wide.
    materialized_x: u32,
    f: [Option<u32>; 32],
    fcsr: Option<u32>,
    valid_x: Option<u32>,
    valid_f: Option<u32>,
    valid_fcsr: Option<u32>,
    write_x: u32,
    write_f: u32,
    write_fcsr: bool,
    pc: u32,
    retired: u32,
    fuel: Option<u32>,
}

impl CachedStateLocals {
    fn lazy(&self) -> bool {
        self.valid_x.is_some() || self.valid_f.is_some() || self.valid_fcsr.is_some()
    }
}

fn region_uses_memory(region: &Region) -> bool {
    region
        .values
        .iter()
        .any(|value| matches!(value.op, Op::Load { .. }))
        || region
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Store { .. }))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RegionMemoryProfile {
    loads: usize,
    stores: usize,
    direct_copies: usize,
}

fn region_memory_profile(region: &Region) -> RegionMemoryProfile {
    let loads = region
        .values
        .iter()
        .filter(|value| matches!(value.op, Op::Load { .. }))
        .count();
    let mut stores = 0;
    let mut direct_copies = 0;
    for effect in &region.effects {
        let Effect::Store {
            value,
            kind,
            condition,
            ..
        } = effect
        else {
            continue;
        };
        stores += 1;
        if condition.is_some() {
            continue;
        }
        if let Some(ValueData {
            op: Op::Load {
                kind: load_kind, ..
            },
            ..
        }) = region.values.get(value.0)
        {
            if load_kind.bytes() == kind.bytes() {
                direct_copies += 1;
            }
        }
    }
    RegionMemoryProfile {
        loads,
        stores,
        direct_copies,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DenseCopyAccess {
    load: ValueId,
    store_position: usize,
    store_address: ValueId,
    source_offset: u64,
    destination_offset: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DenseCopyPlan {
    /// Value/effect position immediately before which both ranges are
    /// validated and translated. Usually this is the first load. If the
    /// destination register is first read by the following store (the normal
    /// compiler shape for `ld tmp; sd tmp`), the first load remains ordinary
    /// and setup moves to the first store, whose precise exit already includes
    /// that completed load.
    setup_position: usize,
    source_root: ValueId,
    destination_root: ValueId,
    source_base_offset: i64,
    destination_base_offset: i64,
    bytes: u64,
    accesses: Vec<DenseCopyAccess>,
}

/// A deliberately narrow whole-loop form of [`DenseCopyPlan`].  Compilers
/// commonly lower RV64 memcpy/memmove to eight `ld`/`sd` pairs followed by
/// three 64-byte induction updates and `bltu 63, remaining, loop`.  Executing
/// that loop literally in generated Wasm still costs roughly twenty guest
/// operations per 64 bytes, while x86 engines collapse `rep movs` to a bulk
/// copy.  This plan proves the complete induction shape; the emitter also
/// guards the loop-limit register at runtime before calling the system helper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BulkCopyLoopPlan {
    source_reg: u8,
    destination_reg: u8,
    count_reg: u8,
    limit_reg: u8,
    value_reg: u8,
    condition: ValueId,
    next_pc: ValueId,
    entry_load: ValueId,
    bytes_per_iteration: u64,
    limit_value: u64,
    step: i64,
    exit_pc: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DenseStoreAccess {
    store_position: usize,
    store_address: ValueId,
    destination_offset: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DenseStorePlan {
    setup_position: usize,
    destination_root: ValueId,
    destination_base_offset: i64,
    bytes: u64,
    /// A common 64-bit value for every store, enabling a guarded bulk-memory
    /// fill when that word is a repeated byte (the compiler's memset shape).
    fill_value: Option<ValueId>,
    accesses: Vec<DenseStoreAccess>,
}

impl DenseStorePlan {
    fn store_access(&self, position: usize, address: ValueId) -> Option<DenseStoreAccess> {
        self.accesses
            .iter()
            .copied()
            .find(|access| access.store_position == position && access.store_address == address)
    }
}

impl DenseCopyPlan {
    fn load_access(&self, load: ValueId) -> Option<DenseCopyAccess> {
        if load.0 < self.setup_position {
            return None;
        }
        self.accesses
            .iter()
            .copied()
            .find(|access| access.load == load)
    }

    fn store_access(
        &self,
        position: usize,
        address: ValueId,
        value: ValueId,
    ) -> Option<DenseCopyAccess> {
        if position < self.setup_position {
            return None;
        }
        self.accesses.iter().copied().find(|access| {
            access.store_position == position
                && access.store_address == address
                && access.load == value
        })
    }
}

fn const_i64(region: &Region, value: ValueId) -> Option<i64> {
    match region.values.get(value.0)?.op {
        Op::ConstI64(constant) => Some(constant),
        _ => None,
    }
}

fn static_guest_pc(region: &Region, value: ValueId) -> Option<u64> {
    match region.values.get(value.0)?.op {
        Op::GuestPc(pc) => Some(pc),
        // Hand-built IR and older diagnostic callers may still deliberately
        // use a plain constant as a static successor.
        Op::ConstI64(pc) => Some(pc as u64),
        _ => None,
    }
}

fn read_x_reg(region: &Region, value: ValueId) -> Option<u8> {
    match region.values.get(value.0)?.op {
        Op::ReadX(reg) => Some(reg),
        _ => None,
    }
}

fn read_x_value(region: &Region, reg: u8) -> Option<ValueId> {
    region.values.iter().enumerate().find_map(|(index, data)| {
        matches!(data.op, Op::ReadX(candidate) if candidate == reg).then_some(ValueId(index))
    })
}

fn output_for_reg(region: &Region, reg: u8) -> Option<ValueId> {
    region
        .outputs
        .iter()
        .find_map(|&(candidate, value)| (candidate == reg).then_some(value))
}

fn add_constant_from(region: &Region, value: ValueId, root: ValueId) -> Option<i64> {
    let Op::Binary { op, lhs, rhs } = region.values.get(value.0)?.op else {
        return None;
    };
    match op {
        BinaryOp::I64Add if lhs == root => const_i64(region, rhs),
        BinaryOp::I64Add if rhs == root => const_i64(region, lhs),
        BinaryOp::I64Sub if lhs == root => const_i64(region, rhs)?.checked_neg(),
        _ => None,
    }
}

fn address_root_offset(region: &Region, value: ValueId) -> (ValueId, i64) {
    let Some(data) = region.values.get(value.0) else {
        return (value, 0);
    };
    match data.op {
        Op::Binary {
            op: BinaryOp::I64Add,
            lhs,
            rhs,
        } => {
            if let Some(offset) = const_i64(region, rhs) {
                let (root, base) = address_root_offset(region, lhs);
                return (root, base.wrapping_add(offset));
            }
            if let Some(offset) = const_i64(region, lhs) {
                let (root, base) = address_root_offset(region, rhs);
                return (root, base.wrapping_add(offset));
            }
            (value, 0)
        }
        Op::Binary {
            op: BinaryOp::I64Sub,
            lhs,
            rhs,
        } => {
            if let Some(offset) = const_i64(region, rhs) {
                let (root, base) = address_root_offset(region, lhs);
                (root, base.wrapping_sub(offset))
            } else {
                (value, 0)
            }
        }
        _ => (value, 0),
    }
}

/// Recognize a straight-line, same-width unrolled copy. Translation and
/// same-page checks can be hoisted once for the complete ranges while the
/// original load/store order remains intact, preserving overlap semantics.
fn dense_copy_plan(region: &Region, minimum: usize) -> Option<DenseCopyPlan> {
    let profile = region_memory_profile(region);
    if profile.loads < minimum
        || profile.stores != profile.loads
        || profile.direct_copies != profile.loads
    {
        return None;
    }
    // Hoisting a translation check across a guard could fault an access the
    // guest would not execute. Dense-copy plans are deliberately confined to
    // one unconditional, ordered memory sequence.
    if region
        .effects
        .iter()
        .any(|effect| !matches!(effect, Effect::Store { .. }))
    {
        return None;
    }

    let mut raw: Vec<(ValueId, usize, ValueId, ValueId, i64, ValueId, i64)> =
        Vec::with_capacity(profile.loads);
    for effect in &region.effects {
        let Effect::Store {
            position,
            address: store_address,
            value,
            kind: StoreKind::I64,
            condition: None,
            ..
        } = effect
        else {
            continue;
        };
        let ValueData {
            op:
                Op::Load {
                    address: load_address,
                    kind: LoadKind::I64,
                    ..
                },
            ..
        } = region.values.get(value.0)?
        else {
            return None;
        };
        if value.0 >= *position || raw.iter().any(|(load, ..)| load == value) {
            return None;
        }
        let (source_root, source_offset) = address_root_offset(region, *load_address);
        let (destination_root, destination_offset) = address_root_offset(region, *store_address);
        raw.push((
            *value,
            *position,
            *store_address,
            source_root,
            source_offset,
            destination_root,
            destination_offset,
        ));
    }
    if raw.len() != profile.loads {
        return None;
    }

    let source_root = raw.first()?.3;
    let destination_root = raw.first()?.5;
    if raw
        .iter()
        .any(|entry| entry.3 != source_root || entry.5 != destination_root)
    {
        return None;
    }
    let first_load = raw.iter().map(|entry| entry.0).min()?;
    let first_store = raw.iter().map(|entry| entry.1).min()?;
    let setup_position = if source_root.0 < first_load.0 && destination_root.0 < first_load.0 {
        first_load.0
    } else {
        // IR store operands always precede the effect position. Delaying setup
        // until the first store is precise: earlier loads have no externally
        // visible effect, and this store's side exit captures their register
        // results before the interpreter resumes at the store.
        if source_root.0 >= first_store || destination_root.0 >= first_store {
            return None;
        }
        first_store
    };

    let source_base_offset = raw.iter().map(|entry| entry.4).min()?;
    let source_end_offset = raw.iter().map(|entry| entry.4).max()?;
    let destination_base_offset = raw.iter().map(|entry| entry.6).min()?;
    let destination_end_offset = raw.iter().map(|entry| entry.6).max()?;
    let bytes = u64::try_from(raw.len()).ok()?.checked_mul(8)?;
    let expected_span = i64::try_from(bytes.checked_sub(8)?).ok()?;
    if source_end_offset.checked_sub(source_base_offset)? != expected_span
        || destination_end_offset.checked_sub(destination_base_offset)? != expected_span
    {
        return None;
    }

    let mut seen = vec![false; raw.len()];
    let mut accesses = Vec::with_capacity(raw.len());
    for (load, store_position, store_address, _, source, _, destination) in raw {
        let source_offset = u64::try_from(source.checked_sub(source_base_offset)?).ok()?;
        let destination_offset =
            u64::try_from(destination.checked_sub(destination_base_offset)?).ok()?;
        if source_offset != destination_offset || source_offset & 7 != 0 {
            return None;
        }
        let slot = usize::try_from(source_offset / 8).ok()?;
        if slot >= seen.len() || seen[slot] {
            return None;
        }
        seen[slot] = true;
        accesses.push(DenseCopyAccess {
            load,
            store_position,
            store_address,
            source_offset,
            destination_offset,
        });
    }
    if seen.iter().any(|seen| !seen) {
        return None;
    }
    Some(DenseCopyPlan {
        setup_position,
        source_root,
        destination_root,
        source_base_offset,
        destination_base_offset,
        bytes,
        accesses,
    })
}

/// Prove the compiler-generated 64-byte copy loop used by the modern Alpine
/// benchmark binaries.  Keeping this stricter than the per-iteration dense
/// copy recognizer is intentional: a helper call may represent many guest
/// iterations, so every loop-carried architectural result must be known.
fn bulk_copy_loop_plan(region: &Region) -> Option<BulkCopyLoopPlan> {
    let copy = dense_copy_plan(region, 1)?;
    if !matches!(copy.bytes, 8 | 64)
        || copy.accesses.len() != usize::try_from(copy.bytes / 8).ok()?
    {
        return None;
    }

    let source_reg = read_x_reg(region, copy.source_root)?;
    let destination_reg = read_x_reg(region, copy.destination_root)?;
    if source_reg == destination_reg {
        return None;
    }
    let source_output = output_for_reg(region, source_reg)?;
    let destination_output = output_for_reg(region, destination_reg)?;
    let source_step = add_constant_from(region, source_output, copy.source_root)?;
    let destination_step = add_constant_from(region, destination_output, copy.destination_root)?;
    let iteration_step = i64::try_from(copy.bytes).ok()?;
    if source_step != destination_step
        || !matches!(source_step, step if step == -iteration_step || step == iteration_step)
    {
        return None;
    }
    let expected_base = if source_step > 0 { 0 } else { -iteration_step };
    if copy.source_base_offset != expected_base || copy.destination_base_offset != expected_base {
        return None;
    }

    let Op::SelectI64 {
        condition,
        if_true,
        if_false,
    } = region.values.get(region.next_pc.0)?.op
    else {
        return None;
    };
    if static_guest_pc(region, if_true)? != region.entry_pc {
        return None;
    }
    let exit_pc = static_guest_pc(region, if_false)?;
    if exit_pc != region.end_pc {
        return None;
    }
    let Op::Binary {
        op: BinaryOp::I64LtU,
        lhs: limit_root,
        rhs: count_output,
    } = region.values.get(condition.0)?.op
    else {
        return None;
    };
    let limit_reg = read_x_reg(region, limit_root)?;

    let (count_reg, count_root) =
        region
            .values
            .iter()
            .enumerate()
            .find_map(|(index, data)| match data.op {
                Op::ReadX(reg)
                    if output_for_reg(region, reg) == Some(count_output)
                        && add_constant_from(region, count_output, ValueId(index))
                            == Some(-iteration_step) =>
                {
                    Some((reg, ValueId(index)))
                }
                _ => None,
            })?;
    debug_assert_eq!(
        add_constant_from(region, count_output, count_root),
        Some(-iteration_step)
    );

    // The last load in guest order is the only live value of the scratch
    // register after an ordinary compiler-generated unrolled copy.
    let final_load = copy
        .accesses
        .iter()
        .max_by_key(|access| access.store_position)?
        .load;
    let value_reg = region
        .outputs
        .iter()
        .find_map(|&(reg, value)| (value == final_load).then_some(reg))?;

    let mut expected_outputs = vec![source_reg, destination_reg, count_reg, value_reg];
    expected_outputs.sort_unstable();
    expected_outputs.dedup();
    let mut actual_outputs: Vec<u8> = region.outputs.iter().map(|&(reg, _)| reg).collect();
    actual_outputs.sort_unstable();
    actual_outputs.dedup();
    if actual_outputs != expected_outputs
        || [source_reg, destination_reg, count_reg, limit_reg, value_reg]
            .into_iter()
            .any(|reg| reg == 0)
    {
        return None;
    }

    Some(BulkCopyLoopPlan {
        source_reg,
        destination_reg,
        count_reg,
        limit_reg,
        value_reg,
        condition,
        next_pc: region.next_pc,
        entry_load: copy.accesses.iter().map(|access| access.load).min()?,
        bytes_per_iteration: copy.bytes,
        limit_value: copy.bytes - 1,
        step: source_step,
        exit_pc,
    })
}

/// Recognize an unconditional contiguous run of 64-bit stores with no loads.
/// This covers compiler-generated memset/page-initialization blocks while
/// retaining every original scalar store and value in architectural order.
/// Proving the complete destination range once removes repeated TLB probes;
/// a failed proof exits before the first store so the interpreter observes the
/// exact original fault/MMIO/store-to-code behavior.
fn dense_store_plan(region: &Region, minimum: usize) -> Option<DenseStorePlan> {
    let profile = region_memory_profile(region);
    if profile.loads != 0 || profile.stores < minimum {
        return None;
    }
    if region
        .effects
        .iter()
        .any(|effect| !matches!(effect, Effect::Store { .. }))
    {
        return None;
    }

    let mut raw: Vec<(usize, ValueId, ValueId, ValueId, i64)> = Vec::with_capacity(profile.stores);
    for effect in &region.effects {
        let Effect::Store {
            position,
            address,
            value,
            kind: StoreKind::I64,
            condition: None,
            ..
        } = effect
        else {
            return None;
        };
        let (root, offset) = address_root_offset(region, *address);
        raw.push((*position, *address, *value, root, offset));
    }
    if raw.len() != profile.stores {
        return None;
    }
    let destination_root = raw.first()?.3;
    if raw.iter().any(|entry| entry.3 != destination_root) {
        return None;
    }
    let setup_position = raw.iter().map(|entry| entry.0).min()?;
    if destination_root.0 >= setup_position {
        return None;
    }

    let destination_base_offset = raw.iter().map(|entry| entry.4).min()?;
    let destination_end_offset = raw.iter().map(|entry| entry.4).max()?;
    let bytes = u64::try_from(raw.len()).ok()?.checked_mul(8)?;
    let expected_span = i64::try_from(bytes.checked_sub(8)?).ok()?;
    if destination_end_offset.checked_sub(destination_base_offset)? != expected_span {
        return None;
    }

    let mut seen = vec![false; raw.len()];
    let mut accesses = Vec::with_capacity(raw.len());
    let first_value = raw.first()?.2;
    let fill_value = raw
        .iter()
        .all(|entry| entry.2 == first_value)
        .then_some(first_value);
    for (store_position, store_address, _, _, destination) in raw {
        let destination_offset =
            u64::try_from(destination.checked_sub(destination_base_offset)?).ok()?;
        if destination_offset & 7 != 0 {
            return None;
        }
        let slot = usize::try_from(destination_offset / 8).ok()?;
        if slot >= seen.len() || seen[slot] {
            return None;
        }
        seen[slot] = true;
        accesses.push(DenseStoreAccess {
            store_position,
            store_address,
            destination_offset,
        });
    }
    if seen.iter().any(|seen| !seen) {
        return None;
    }
    Some(DenseStorePlan {
        setup_position,
        destination_root,
        destination_base_offset,
        bytes,
        fill_value,
        accesses,
    })
}

/// Report whether this region will use the range-checked dense-copy lowering
/// for the supplied production layout. Keep this beside the recognizer so
/// runtime diagnostics measure the emitter's actual selection rule rather
/// than maintaining a second, approximate copy of it.
pub(crate) fn uses_dense_copy_plan(region: &Region, layout: JitLayout) -> bool {
    let Some(memory) = layout.sys else {
        return false;
    };
    memory.cache_within_invocation
        && dense_copy_plan(region, usize::from(memory.cache_min_accesses.max(1))).is_some()
}

pub(crate) fn uses_dense_store_plan(region: &Region, layout: JitLayout) -> bool {
    let Some(memory) = layout.sys else {
        return false;
    };
    memory.cache_within_invocation
        && dense_store_plan(region, usize::from(memory.cache_min_accesses.max(1))).is_some()
}

pub(crate) fn uses_bulk_copy_loop_plan(region: &Region, layout: JitLayout) -> bool {
    layout.sys.is_some() && bulk_copy_loop_plan(region).is_some()
}

/// Use invocation-local translation caches only in dense direct-copy members.
/// Other members retain the cheaper direct TLB probe even when they share the
/// module function and its allocated cache locals.
fn memory_temps_for_region(
    mut temps: Option<MemoryTemps>,
    region: &Region,
    layout: JitLayout,
) -> Option<MemoryTemps> {
    let Some(ref mut temps) = temps else {
        return None;
    };
    let Some(memory) = layout.sys else {
        return Some(*temps);
    };
    let minimum = usize::from(memory.cache_min_accesses.max(1));
    let copy = dense_copy_plan(region, minimum).is_some();
    let store = dense_store_plan(region, minimum).is_some();
    if bulk_copy_loop_plan(region).is_none() {
        temps.bulk_copy = None;
    }
    if !copy && !store {
        temps.load_cache = None;
        temps.store_cache = None;
        temps.copy = None;
    } else if store {
        temps.load_cache = None;
    }
    Some(*temps)
}

fn allocate_memory_temps(
    local_types: &mut Vec<ValType>,
    region: &Region,
    layout: JitLayout,
) -> Option<MemoryTemps> {
    if layout.sys.is_none() || !region_uses_memory(region) {
        return None;
    }
    let memory = layout.sys.expect("system-memory layout");
    let minimum = usize::from(memory.cache_min_accesses.max(1));
    let cache_copy = memory.cache_within_invocation && dense_copy_plan(region, minimum).is_some();
    let cache_store = memory.cache_within_invocation && dense_store_plan(region, minimum).is_some();
    let bulk_copy = bulk_copy_loop_plan(region).is_some();
    let cache_range = cache_copy || cache_store;
    let page = cache_range.then(|| alloc_local(local_types, ValType::I64));
    let mut alloc_cache = || TranslationCacheTemps {
        page: alloc_local(local_types, ValType::I64),
        offset: alloc_local(local_types, ValType::I64),
    };
    let load_cache = cache_copy.then(&mut alloc_cache);
    let store_cache = cache_range.then(&mut alloc_cache);
    let copy = (cache_range || bulk_copy).then(|| DenseCopyTemps {
        source_address: alloc_local(local_types, ValType::I64),
        destination_address: alloc_local(local_types, ValType::I64),
        source_linear: alloc_local(local_types, ValType::I32),
        destination_linear: alloc_local(local_types, ValType::I32),
    });
    let bulk_copy = bulk_copy.then(|| BulkCopyTemps {
        request: alloc_local(local_types, ValType::I64),
        fuel_bytes: alloc_local(local_types, ValType::I64),
        result: alloc_local(local_types, ValType::I64),
    });
    Some(MemoryTemps {
        index: alloc_local(local_types, ValType::I32),
        offset: alloc_local(local_types, ValType::I64),
        context: alloc_local(local_types, ValType::I64),
        page,
        load_cache,
        store_cache,
        copy,
        bulk_copy,
    })
}

/// Snapshot the runtime's effective data-access context once per generated
/// invocation. Privileged instructions are precise side exits, so this value
/// cannot change before the function returns (including internal CFG hops).
fn emit_memory_context_init(
    function: &mut Function,
    layout: JitLayout,
    temps: Option<MemoryTemps>,
) {
    let (Some(memory), Some(temps)) = (layout.sys, temps) else {
        return;
    };
    function.instruction(&Instruction::I32Const(memory.context_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalSet(temps.context));
    // Wasm locals start at zero. Page zero is a valid numeric page, so an
    // uninitialized cache tag must not be mistaken for a proven translation.
    for cache in [temps.load_cache, temps.store_cache].into_iter().flatten() {
        function.instruction(&Instruction::I64Const(-1));
        function.instruction(&Instruction::LocalSet(cache.page));
    }
}

fn val_type(ty: ValueType) -> ValType {
    match ty {
        ValueType::I32 => ValType::I32,
        ValueType::I64 => ValType::I64,
    }
}

const fn memarg(align: u32, offset: u64) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

/// Add a static amount to one embedding-owned diagnostic counter.  Callers
/// only reach this helper when `JitLayout::structured_profile` is present, so
/// disabled production emission pays neither code size nor a runtime branch.
fn emit_profile_counter_add(function: &mut Function, address: u32, amount: u64) {
    if amount == 0 {
        return;
    }
    function.instruction(&Instruction::I32Const(address as i32));
    function.instruction(&Instruction::I32Const(address as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::I64Const(amount as i64));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
}

fn emit_structured_profile(function: &mut Function, layout: JitLayout, region: &Region) {
    let Some(counters) = layout.structured_profile else {
        return;
    };
    emit_profile_counter_add(function, counters[0], 1);
    emit_profile_counter_add(function, counters[1], u64::from(region.retired));
    for (address, amount) in counters[2..7].iter().zip(region.trace_mix) {
        emit_profile_counter_add(function, *address, u64::from(amount));
    }
    if region.writes_x2 {
        emit_profile_counter_add(function, counters[7], 1);
    }
    emit_profile_counter_add(function, counters[8], u64::from(region.trace_stack_memory));
}

/// Emit an architectural address whose value is derived from a guest PC.
/// Ordinary modules retain the single absolute constant they used before.
/// Position-independent page modules import one immutable base and encode only
/// the wrapping offset, making their bytes independent of an ASLR alias.
fn emit_guest_pc(function: &mut Function, pc: u64, layout: JitLayout) {
    if let Some(code_base) = layout.pic_code_base {
        function.instruction(&Instruction::GlobalGet(0));
        let offset = pc.wrapping_sub(code_base) as i64;
        if offset != 0 {
            function.instruction(&Instruction::I64Const(offset));
            function.instruction(&Instruction::I64Add);
        }
    } else {
        function.instruction(&Instruction::I64Const(pc as i64));
    }
}

pub(crate) fn emit(
    region: &Region,
    layout: JitLayout,
    loop_backedge: Option<LoopBackedge>,
) -> Result<Vec<u8>, EmitError> {
    validate_emission(region, layout)?;
    let helpers = HelperImports::for_region(region, layout);
    let function = emit_function(region, layout, loop_backedge, helpers)?;
    Ok(finish_module(function, helpers, layout))
}

/// Emit several independently validated region bodies behind one in-module
/// PC dispatcher. All public entries may point at the same dispatcher: it
/// reads the architectural PC, selects a body with a balanced decision tree,
/// and keeps following covered edges until fuel is spent or execution leaves
/// the member set. This is portable core Wasm and needs neither JS callbacks
/// nor the tail-call proposal.
pub(crate) fn emit_multi_entry(
    regions: &[(&Region, Option<LoopBackedge>)],
    layout: JitLayout,
    export_members: bool,
) -> Result<Vec<u8>, EmitError> {
    emit_multi_entry_mode(
        regions,
        layout,
        export_members,
        MultiEntryState::RegisterEager,
    )
}

/// Emit a multi-entry module with either invocation-local architectural state
/// or body-boundary materialization. The latter is useful for measured regions
/// that usually leave after one body: it avoids loading a large register union
/// merely to cache it for no internal edge. Both modes share validation,
/// helpers, dispatch semantics, and precise exits.
pub(crate) fn emit_multi_entry_mode(
    regions: &[(&Region, Option<LoopBackedge>)],
    layout: JitLayout,
    export_members: bool,
    state: MultiEntryState,
) -> Result<Vec<u8>, EmitError> {
    if regions.len() < 2 {
        return Err(EmitError(
            "a multi-entry module requires at least two regions".into(),
        ));
    }

    let mut helpers = HelperImports::default();
    let mut seen = std::collections::BTreeSet::new();
    for &(region, _) in regions {
        validate_emission(region, layout)?;
        if !seen.insert(region.entry_pc) {
            return Err(EmitError("duplicate multi-entry PC".into()));
        }
        helpers.include(HelperImports::for_region(region, layout))?;
    }

    // General multi-entry regions keep their architectural union in locals.
    // A single-latch member becomes a nested local loop over the same cached
    // state, so hot backedges avoid both architectural materialization and the
    // outer PC decision tree.
    if matches!(
        state,
        MultiEntryState::RegisterEager
            | MultiEntryState::RegisterLazy
            | MultiEntryState::RegisterDirect
            | MultiEntryState::RegisterCfg
            | MultiEntryState::RegisterStructured
    ) {
        return emit_cached_multi_entry(
            regions,
            layout,
            helpers,
            export_members,
            state == MultiEntryState::RegisterLazy,
            matches!(
                state,
                MultiEntryState::RegisterDirect | MultiEntryState::RegisterCfg
            ),
            state == MultiEntryState::RegisterStructured,
        );
    }

    let mut functions = Vec::with_capacity(regions.len());
    for &(region, loop_backedge) in regions {
        functions.push(emit_function(region, layout, loop_backedge, helpers)?);
    }
    let entries: Vec<u64> = regions.iter().map(|(region, _)| region.entry_pc).collect();
    let tail_calls = state == MultiEntryState::MemoryTailCall && layout.fuel_addr != 0;
    let wrappers = if tail_calls {
        let body_base = helpers.count();
        let wrapper_base = body_base + regions.len() as u32;
        regions
            .iter()
            .enumerate()
            .map(|(index, (region, _))| {
                let target = static_guest_pc(region, region.next_pc).and_then(|pc| {
                    entries
                        .iter()
                        .position(|entry| *entry == pc)
                        .map(|member| (pc, wrapper_base + member as u32))
                });
                emit_tail_wrapper(body_base + index as u32, target, layout)
            })
            .collect()
    } else {
        Vec::new()
    };
    let dispatch_base = if wrappers.is_empty() {
        helpers.count()
    } else {
        helpers.count() + regions.len() as u32
    };
    let dispatcher = emit_multi_dispatch(&entries, layout, dispatch_base);
    Ok(finish_multi_module(
        functions,
        wrappers,
        dispatcher,
        helpers,
        export_members,
        layout,
    ))
}

fn alloc_local(local_types: &mut Vec<ValType>, ty: ValType) -> u32 {
    let local = 1 + local_types.len() as u32;
    local_types.push(ty);
    local
}

fn emit_cached_multi_entry(
    regions: &[(&Region, Option<LoopBackedge>)],
    layout: JitLayout,
    mut helpers: HelperImports,
    export_members: bool,
    lazy: bool,
    direct_dispatch: bool,
    structured_cfg: bool,
) -> Result<Vec<u8>, EmitError> {
    // Cross-module chaining is useful only after the in-module structured CFG
    // has exhausted its covered successors. A function import avoids the
    // shared-table publication cost that made per-module table imports scale
    // quadratically in V8.
    helpers.chain = structured_cfg && crate::chain_enabled() && layout.fuel_addr != 0;
    helpers.tail_chain = structured_cfg
        && crate::region_tail_chain_enabled()
        && layout.fuel_addr != 0
        && layout.dispatch_base != 0
        && layout.map_gen_addr != 0
        && layout.chain_hops_addr != 0;
    let mut need_x = 0u32;
    let mut need_f = 0u32;
    let mut need_fcsr = false;
    let mut write_x = 0u32;
    let mut write_f = 0u32;
    let mut write_fcsr = false;
    for &(region, _) in regions {
        for value in &region.values {
            match value.op {
                Op::ReadX(reg) => need_x |= 1u32 << reg,
                Op::ReadF(reg) => need_f |= 1u32 << reg,
                Op::ReadFcsr => need_fcsr = true,
                _ => {}
            }
        }
        for &(reg, _) in &region.outputs {
            need_x |= 1u32 << reg;
            write_x |= 1u32 << reg;
        }
        for &(reg, _) in &region.f_outputs {
            need_f |= 1u32 << reg;
            write_f |= 1u32 << reg;
        }
        need_fcsr |= region.fcsr_output.is_some();
        write_fcsr |= region.fcsr_output.is_some();
    }

    let mut local_types = Vec::new();
    let mut state = CachedStateLocals {
        x: [None; 32],
        materialized_x: if structured_cfg {
            need_x & !STRUCTURED_RESIDENT_X_MASK & !1
        } else {
            0
        },
        f: [None; 32],
        fcsr: None,
        valid_x: None,
        valid_f: None,
        valid_fcsr: None,
        write_x,
        write_f,
        write_fcsr,
        pc: 0,
        retired: 0,
        fuel: None,
    };
    for reg in 1..32 {
        if need_x & (1u32 << reg) != 0 && state.materialized_x & (1u32 << reg) == 0 {
            state.x[reg] = Some(alloc_local(&mut local_types, ValType::I64));
        }
    }
    for reg in 0..32 {
        if need_f & (1u32 << reg) != 0 {
            state.f[reg] = Some(alloc_local(&mut local_types, ValType::I64));
        }
    }
    if need_fcsr {
        state.fcsr = Some(alloc_local(&mut local_types, ValType::I32));
    }
    if lazy {
        if need_x != 0 {
            state.valid_x = Some(alloc_local(&mut local_types, ValType::I64));
        }
        if need_f != 0 {
            state.valid_f = Some(alloc_local(&mut local_types, ValType::I64));
        }
        if need_fcsr {
            state.valid_fcsr = Some(alloc_local(&mut local_types, ValType::I32));
        }
    }
    state.pc = alloc_local(&mut local_types, ValType::I64);
    state.retired = alloc_local(&mut local_types, ValType::I64);
    if layout.fuel_addr != 0 {
        state.fuel = Some(alloc_local(&mut local_types, ValType::I64));
    }

    // Member bodies are mutually exclusive within one dispatcher iteration,
    // and every architectural result is copied into `state` before another
    // body can run. Reuse type-specific SSA temporary pools across members
    // instead of exposing the sum of all member values as Wasm locals. This
    // bounds the embedding engine's local/register-allocation pressure by the
    // largest member, not by region population.
    let max_i32 = regions
        .iter()
        .map(|(region, _)| {
            region
                .values
                .iter()
                .filter(|value| value.ty == ValueType::I32)
                .count()
        })
        .max()
        .unwrap_or(0);
    let max_i64 = regions
        .iter()
        .map(|(region, _)| {
            region
                .values
                .iter()
                .filter(|value| value.ty == ValueType::I64)
                .count()
        })
        .max()
        .unwrap_or(0);
    let i32_pool: Vec<u32> = (0..max_i32)
        .map(|_| alloc_local(&mut local_types, ValType::I32))
        .collect();
    let i64_pool: Vec<u32> = (0..max_i64)
        .map(|_| alloc_local(&mut local_types, ValType::I64))
        .collect();
    let mut local_maps = Vec::with_capacity(regions.len());
    for &(region, _) in regions {
        let mut next_i32 = 0;
        let mut next_i64 = 0;
        let map = region
            .values
            .iter()
            .map(|value| {
                Some(match value.ty {
                    ValueType::I32 => {
                        let local = i32_pool[next_i32];
                        next_i32 += 1;
                        local
                    }
                    ValueType::I64 => {
                        let local = i64_pool[next_i64];
                        next_i64 += 1;
                        local
                    }
                })
            })
            .collect();
        local_maps.push(map);
    }
    let memory_temps =
        if layout.sys.is_some() && regions.iter().any(|(region, _)| region_uses_memory(region)) {
            let memory = layout.sys.expect("system-memory layout");
            let minimum = usize::from(memory.cache_min_accesses.max(1));
            let cache_copy = memory.cache_within_invocation
                && regions
                    .iter()
                    .any(|(region, _)| dense_copy_plan(region, minimum).is_some());
            let cache_store_range = memory.cache_within_invocation
                && regions
                    .iter()
                    .any(|(region, _)| dense_store_plan(region, minimum).is_some());
            let has_bulk_copy = regions
                .iter()
                .any(|(region, _)| bulk_copy_loop_plan(region).is_some());
            let cache_range = cache_copy || cache_store_range;
            let mut alloc_cache = || TranslationCacheTemps {
                page: alloc_local(&mut local_types, ValType::I64),
                offset: alloc_local(&mut local_types, ValType::I64),
            };
            let load_cache = cache_copy.then(&mut alloc_cache);
            let store_cache = cache_range.then(&mut alloc_cache);
            let page = cache_range.then(|| alloc_local(&mut local_types, ValType::I64));
            let copy = (cache_range || has_bulk_copy).then(|| DenseCopyTemps {
                source_address: alloc_local(&mut local_types, ValType::I64),
                destination_address: alloc_local(&mut local_types, ValType::I64),
                source_linear: alloc_local(&mut local_types, ValType::I32),
                destination_linear: alloc_local(&mut local_types, ValType::I32),
            });
            let bulk_copy = has_bulk_copy.then(|| BulkCopyTemps {
                request: alloc_local(&mut local_types, ValType::I64),
                fuel_bytes: alloc_local(&mut local_types, ValType::I64),
                result: alloc_local(&mut local_types, ValType::I64),
            });
            Some(MemoryTemps {
                index: alloc_local(&mut local_types, ValType::I32),
                offset: alloc_local(&mut local_types, ValType::I64),
                context: alloc_local(&mut local_types, ValType::I64),
                page,
                load_cache,
                store_cache,
                copy,
                bulk_copy,
            })
        } else {
            None
        };
    let selector_local = alloc_local(&mut local_types, ValType::I32);
    let hops_local = alloc_local(&mut local_types, ValType::I32);
    let chain_start_local =
        (helpers.chain || helpers.tail_chain).then(|| alloc_local(&mut local_types, ValType::I64));
    let tail_dispatch_local = helpers
        .tail_chain
        .then(|| alloc_local(&mut local_types, ValType::I32));
    let mut function = Function::new_with_locals_types(local_types);

    emit_memory_context_init(&mut function, layout, memory_temps);
    emit_cached_state_load(&mut function, layout, &state);
    if let Some(chain_start) = chain_start_local {
        function.instruction(&Instruction::LocalGet(state.retired));
        function.instruction(&Instruction::LocalSet(chain_start));
    }
    if let Some(temps) = memory_temps {
        // A logical page is at most 2^(64-page_shift)-1, so -1 is an
        // impossible tag for every validated system page geometry.
        for cache in [temps.load_cache, temps.store_cache].into_iter().flatten() {
            function.instruction(&Instruction::I64Const(-1));
            function.instruction(&Instruction::LocalSet(cache.page));
        }
    }
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(hops_local));
    if structured_cfg {
        emit_cached_structured_cfg(
            &mut function,
            regions,
            &local_maps,
            layout,
            helpers,
            memory_temps,
            &state,
            selector_local,
            hops_local,
        )?;
    } else if direct_dispatch {
        emit_cached_direct_dispatch(
            &mut function,
            regions,
            &local_maps,
            layout,
            helpers,
            memory_temps,
            &state,
            selector_local,
            hops_local,
        )?;
    } else {
        function.instruction(&Instruction::Block(BlockType::Empty));
        function.instruction(&Instruction::Loop(BlockType::Empty));
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::LocalSet(selector_local));

        let mut order: Vec<usize> = (0..regions.len()).collect();
        order.sort_unstable_by_key(|&index| regions[index].0.entry_pc);
        emit_cached_dispatch_tree(
            &mut function,
            &order,
            regions,
            &local_maps,
            layout,
            helpers,
            memory_temps,
            &state,
            selector_local,
        )?;

        function.instruction(&Instruction::LocalGet(selector_local));
        function.instruction(&Instruction::I32Eqz);
        function.instruction(&Instruction::BrIf(1));
        if let Some(fuel) = state.fuel {
            function.instruction(&Instruction::LocalGet(state.retired));
            function.instruction(&Instruction::LocalGet(fuel));
            function.instruction(&Instruction::I64GeU);
            function.instruction(&Instruction::BrIf(1));
        } else {
            function.instruction(&Instruction::LocalGet(hops_local));
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalTee(hops_local));
            function.instruction(&Instruction::I32Const(MULTI_ENTRY_HOP_CAP));
            function.instruction(&Instruction::I32GeU);
            function.instruction(&Instruction::BrIf(1));
        }
        function.instruction(&Instruction::Br(0));
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::End);
    }

    emit_cached_state_commit(&mut function, layout, &state);
    if let (Some(chain_start), Some(chain_index)) = (chain_start_local, helpers.chain_index()) {
        // Never recurse after a precise first-instruction side exit: without
        // this progress check the same entry would call itself until the
        // defensive runtime depth cap on every TLB/MMIO miss.
        function.instruction(&Instruction::LocalGet(state.retired));
        function.instruction(&Instruction::LocalGet(chain_start));
        function.instruction(&Instruction::I64GtU);
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::Call(chain_index));
        function.instruction(&Instruction::End);
    }
    if let (Some(chain_start), Some(dispatch_local), Some(fuel), Some(tail_chain_index)) = (
        chain_start_local,
        tail_dispatch_local,
        state.fuel,
        helpers.tail_chain_index(),
    ) {
        emit_region_tail_chain(
            &mut function,
            layout,
            &state,
            chain_start,
            dispatch_local,
            fuel,
            tail_chain_index,
        );
    }
    function.instruction(&Instruction::End);
    Ok(finish_shared_module(
        function,
        helpers,
        regions.len() as u32,
        export_members,
        layout,
    ))
}

fn emit_cached_state_load(function: &mut Function, layout: JitLayout, state: &CachedStateLocals) {
    if !state.lazy() {
        for (reg, local) in state.x.iter().copied().enumerate() {
            if let Some(local) = local {
                function.instruction(&Instruction::I32Const(layout.x_base as i32));
                function.instruction(&Instruction::I64Load(memarg(3, reg as u64 * 8)));
                function.instruction(&Instruction::LocalSet(local));
            }
        }
        for (reg, local) in state.f.iter().copied().enumerate() {
            if let Some(local) = local {
                function.instruction(&Instruction::I32Const(layout.f_base as i32));
                function.instruction(&Instruction::I64Load(memarg(3, reg as u64 * 8)));
                function.instruction(&Instruction::LocalSet(local));
            }
        }
        if let Some(local) = state.fcsr {
            function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
            function.instruction(&Instruction::I32Load(memarg(2, 0)));
            function.instruction(&Instruction::LocalSet(local));
        }
    }
    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalSet(state.pc));
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalSet(state.retired));
    if let Some(local) = state.fuel {
        function.instruction(&Instruction::I32Const(layout.fuel_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::LocalSet(local));
    }
}

fn emit_cached_state_commit(function: &mut Function, layout: JitLayout, state: &CachedStateLocals) {
    for (reg, local) in state.x.iter().copied().enumerate() {
        let Some(local) = local else { continue };
        if state.write_x & (1u32 << reg) == 0 {
            continue;
        }
        if let Some(valid) = state.valid_x {
            emit_valid_i64(function, valid, reg);
            function.instruction(&Instruction::If(BlockType::Empty));
        }
        function.instruction(&Instruction::I32Const(layout.x_base as i32));
        function.instruction(&Instruction::LocalGet(local));
        function.instruction(&Instruction::I64Store(memarg(3, reg as u64 * 8)));
        if state.valid_x.is_some() {
            function.instruction(&Instruction::End);
        }
    }
    for (reg, local) in state.f.iter().copied().enumerate() {
        let Some(local) = local else { continue };
        if state.write_f & (1u32 << reg) == 0 {
            continue;
        }
        if let Some(valid) = state.valid_f {
            emit_valid_i64(function, valid, reg);
            function.instruction(&Instruction::If(BlockType::Empty));
        }
        function.instruction(&Instruction::I32Const(layout.f_base as i32));
        function.instruction(&Instruction::LocalGet(local));
        function.instruction(&Instruction::I64Store(memarg(3, reg as u64 * 8)));
        if state.valid_f.is_some() {
            function.instruction(&Instruction::End);
        }
    }
    if let Some(local) = state.fcsr.filter(|_| state.write_fcsr) {
        if let Some(valid) = state.valid_fcsr {
            function.instruction(&Instruction::LocalGet(valid));
            function.instruction(&Instruction::If(BlockType::Empty));
        }
        function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
        function.instruction(&Instruction::LocalGet(local));
        function.instruction(&Instruction::I32Store(memarg(2, 0)));
        if state.valid_fcsr.is_some() {
            function.instruction(&Instruction::End);
        }
    }
    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    function.instruction(&Instruction::LocalGet(state.pc));
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::LocalGet(state.retired));
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
}

/// Continue at an already-published generated entry without growing the Wasm
/// call stack. The dispatch line is only a fast-path hint: matching the full
/// architectural PC and the current mapping generation is the same proof the
/// outer runtime requires before entering generated code. Any miss simply
/// returns to that runtime, which owns mapping validation and compilation.
fn emit_region_tail_chain(
    function: &mut Function,
    layout: JitLayout,
    state: &CachedStateLocals,
    chain_start: u32,
    dispatch_local: u32,
    fuel: u32,
    tail_chain_index: u32,
) {
    debug_assert_ne!(layout.dispatch_base, 0);
    debug_assert_ne!(layout.map_gen_addr, 0);
    debug_assert_ne!(layout.chain_hops_addr, 0);

    // A precise side exit can retire zero instructions. It must return to T0
    // instead of tail-calling the same entry forever. Fuel is cumulative for
    // the complete chain, so a transfer cannot evade the scheduler budget.
    function.instruction(&Instruction::LocalGet(state.retired));
    function.instruction(&Instruction::LocalGet(chain_start));
    function.instruction(&Instruction::I64GtU);
    function.instruction(&Instruction::LocalGet(state.retired));
    function.instruction(&Instruction::LocalGet(fuel));
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::If(BlockType::Empty));

    // dispatch[((pc >> 1) & mask)] where each repr(C) line is 16 bytes.
    function.instruction(&Instruction::LocalGet(state.pc));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I64Const(i64::from(layout.dispatch_mask)));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Const(4));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Const(layout.dispatch_base as i32));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(dispatch_local));

    // Full PC tag, live map generation, non-sentinel generation, and a
    // published non-negative table index jointly authorize the fast transfer.
    function.instruction(&Instruction::LocalGet(dispatch_local));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalGet(state.pc));
    function.instruction(&Instruction::I64Eq);

    function.instruction(&Instruction::LocalGet(dispatch_local));
    function.instruction(&Instruction::I32Load(memarg(2, 12)));
    function.instruction(&Instruction::I32Const(layout.map_gen_addr as i32));
    function.instruction(&Instruction::I32Load(memarg(2, 0)));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::I32And);

    function.instruction(&Instruction::LocalGet(dispatch_local));
    function.instruction(&Instruction::I32Load(memarg(2, 12)));
    function.instruction(&Instruction::I32Const(-1));
    function.instruction(&Instruction::I32Ne);
    function.instruction(&Instruction::I32And);

    function.instruction(&Instruction::LocalGet(dispatch_local));
    function.instruction(&Instruction::I32Load(memarg(2, 8)));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32GeS);
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::If(BlockType::Empty));

    function.instruction(&Instruction::I32Const(layout.chain_hops_addr as i32));
    function.instruction(&Instruction::I32Const(layout.chain_hops_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Store(memarg(3, 0)));

    // Tail-call the single host-created trampoline. It alone imports the
    // shared function table and performs return_call_indirect, so generated
    // modules remain table-independent and table.set does not become
    // O(generated instances) in V8. Clearing SB_IDX_BIT converts the runtime's
    // region-attribution tag back to the actual function-table slot.
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalGet(dispatch_local));
    function.instruction(&Instruction::I32Load(memarg(2, 8)));
    function.instruction(&Instruction::I32Const(!crate::SB_IDX_BIT));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::ReturnCall(tail_chain_index));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
}

#[allow(clippy::too_many_arguments)]
fn emit_cached_direct_dispatch(
    function: &mut Function,
    regions: &[(&Region, Option<LoopBackedge>)],
    local_maps: &[Vec<Option<u32>>],
    layout: JitLayout,
    helpers: HelperImports,
    memory_temps: Option<MemoryTemps>,
    state: &CachedStateLocals,
    selector_local: u32,
    hops_local: u32,
) -> Result<(), EmitError> {
    function.instruction(&Instruction::I32Const(-1));
    function.instruction(&Instruction::LocalSet(selector_local));
    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));

    // A known covered successor leaves its dense member index in the local.
    // Only an external/dynamic successor pays the PC-to-index comparison tree.
    function.instruction(&Instruction::LocalGet(selector_local));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32LtS);
    function.instruction(&Instruction::If(BlockType::Empty));
    let mut order: Vec<usize> = (0..regions.len()).collect();
    order.sort_unstable_by_key(|&index| regions[index].0.entry_pc);
    emit_cached_index_tree(function, &order, regions, layout, state, selector_local);
    function.instruction(&Instruction::End);

    // Classic structured switch: target i leaves the i innermost case blocks,
    // landing immediately before body i. The default leaves the outer exit
    // block, after which the common architectural commit runs.
    for _ in regions {
        function.instruction(&Instruction::Block(BlockType::Empty));
    }
    let targets: Vec<u32> = (0..regions.len() as u32).collect();
    function.instruction(&Instruction::LocalGet(selector_local));
    function.instruction(&Instruction::BrTable(
        targets.into(),
        regions.len() as u32 + 1,
    ));

    for (index, &(region, loop_backedge)) in regions.iter().enumerate() {
        function.instruction(&Instruction::End);
        emit_cached_body(
            function,
            region,
            layout,
            helpers,
            memory_temps,
            state,
            &local_maps[index],
            loop_backedge,
        )?;

        let member_for = |value: ValueId| {
            let pc = static_guest_pc(region, value)?;
            regions
                .iter()
                .position(|(candidate, _)| candidate.entry_pc == pc)
                .map(|member| member as i32)
        };
        if let Some(pc) = static_guest_pc(region, region.next_pc) {
            let successor = regions
                .iter()
                .position(|(candidate, _)| candidate.entry_pc == pc)
                .map(|member| member as i32)
                .unwrap_or(-1);
            function.instruction(&Instruction::I32Const(successor));
        } else {
            match region.values.get(region.next_pc.0).map(|value| &value.op) {
                Some(Op::SelectI64 {
                    condition,
                    if_true,
                    if_false,
                }) => {
                    // A CFG basic block exposes both conditional successors as
                    // constants. Select their dense indices directly; an uncovered
                    // side uses -1 and falls through the exact PC lookup/exit path.
                    function
                        .instruction(&Instruction::I32Const(member_for(*if_true).unwrap_or(-1)));
                    function
                        .instruction(&Instruction::I32Const(member_for(*if_false).unwrap_or(-1)));
                    function.instruction(&Instruction::LocalGet(
                        local_maps[index][condition.0].expect("cached branch condition local"),
                    ));
                    function.instruction(&Instruction::Select);
                }
                _ => {
                    function.instruction(&Instruction::I32Const(-1));
                }
            }
        }
        function.instruction(&Instruction::LocalSet(selector_local));

        let loop_depth = (regions.len() - 1 - index) as u32;
        let exit_depth = loop_depth + 1;
        if let Some(fuel) = state.fuel {
            function.instruction(&Instruction::LocalGet(state.retired));
            function.instruction(&Instruction::LocalGet(fuel));
            function.instruction(&Instruction::I64GeU);
            function.instruction(&Instruction::BrIf(exit_depth));
        } else {
            function.instruction(&Instruction::LocalGet(hops_local));
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalTee(hops_local));
            function.instruction(&Instruction::I32Const(MULTI_ENTRY_HOP_CAP));
            function.instruction(&Instruction::I32GeU);
            function.instruction(&Instruction::BrIf(exit_depth));
        }
        function.instruction(&Instruction::Br(loop_depth));
    }

    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    Ok(())
}

const STRUCTURED_CFG_DUPLICATION_LIMIT: usize = 250;

#[derive(Clone, Copy, Debug)]
struct StructuredDestination {
    scope: u32,
    selector: Option<i32>,
}

enum StructuredWork {
    Node(Structure),
    End {
        scope: u32,
        targets: Vec<usize>,
        previous: Vec<Option<StructuredDestination>>,
    },
}

fn cfg_member_for_value(
    regions: &[(&Region, Option<LoopBackedge>)],
    region: &Region,
    value: ValueId,
) -> Option<usize> {
    let pc = static_guest_pc(region, value)?;
    regions
        .iter()
        .position(|(candidate, _)| candidate.entry_pc == pc)
}

fn cfg_successors(regions: &[(&Region, Option<LoopBackedge>)]) -> Vec<Vec<usize>> {
    regions
        .iter()
        .map(|(region, _)| {
            let mut successors = Vec::with_capacity(2);
            match region.values.get(region.next_pc.0).map(|value| &value.op) {
                Some(Op::ConstI64(_) | Op::GuestPc(_)) => {
                    if let Some(member) = cfg_member_for_value(regions, region, region.next_pc) {
                        successors.push(member);
                    }
                }
                Some(Op::SelectI64 {
                    if_true, if_false, ..
                }) => {
                    for value in [*if_true, *if_false] {
                        if let Some(member) = cfg_member_for_value(regions, region, value) {
                            if !successors.contains(&member) {
                                successors.push(member);
                            }
                        }
                    }
                }
                _ => {}
            }
            successors
        })
        .collect()
}

fn structured_depth(active_scopes: &[u32], scope: u32) -> Result<u32, EmitError> {
    active_scopes
        .iter()
        .rev()
        .position(|active| *active == scope)
        .map(|depth| depth as u32)
        .ok_or_else(|| EmitError("structured CFG branch target is not active".into()))
}

fn emit_structured_unconditional_branch(
    function: &mut Function,
    destination: StructuredDestination,
    selector_local: u32,
    active_scopes: &[u32],
) -> Result<(), EmitError> {
    if let Some(selector) = destination.selector {
        function.instruction(&Instruction::I32Const(selector));
        function.instruction(&Instruction::LocalSet(selector_local));
    }
    function.instruction(&Instruction::Br(structured_depth(
        active_scopes,
        destination.scope,
    )?));
    Ok(())
}

fn emit_structured_conditional_branch(
    function: &mut Function,
    condition_local: u32,
    invert: bool,
    destination: StructuredDestination,
    selector_local: u32,
    active_scopes: &[u32],
) -> Result<(), EmitError> {
    function.instruction(&Instruction::LocalGet(condition_local));
    if invert {
        function.instruction(&Instruction::I32Eqz);
    }
    if let Some(selector) = destination.selector {
        // The selector write must happen only on the taken arm. The temporary
        // `if` adds one control depth around the requested target.
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::I32Const(selector));
        function.instruction(&Instruction::LocalSet(selector_local));
        function.instruction(&Instruction::Br(
            structured_depth(active_scopes, destination.scope)? + 1,
        ));
        function.instruction(&Instruction::End);
    } else {
        function.instruction(&Instruction::BrIf(structured_depth(
            active_scopes,
            destination.scope,
        )?));
    }
    Ok(())
}

fn emit_structured_selector_branch(
    function: &mut Function,
    member: usize,
    destination: StructuredDestination,
    selector_local: u32,
    active_scopes: &[u32],
) -> Result<(), EmitError> {
    function.instruction(&Instruction::LocalGet(selector_local));
    function.instruction(&Instruction::I32Const(member as i32));
    function.instruction(&Instruction::I32Eq);
    if let Some(selector) = destination.selector {
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::I32Const(selector));
        function.instruction(&Instruction::LocalSet(selector_local));
        function.instruction(&Instruction::Br(
            structured_depth(active_scopes, destination.scope)? + 1,
        ));
        function.instruction(&Instruction::End);
    } else {
        function.instruction(&Instruction::BrIf(structured_depth(
            active_scopes,
            destination.scope,
        )?));
    }
    Ok(())
}

fn emit_structured_safety(
    function: &mut Function,
    state: &CachedStateLocals,
    hops_local: u32,
    exit_scope: u32,
    active_scopes: &[u32],
) -> Result<(), EmitError> {
    let exit_depth = structured_depth(active_scopes, exit_scope)?;
    if let Some(fuel) = state.fuel {
        function.instruction(&Instruction::LocalGet(state.retired));
        function.instruction(&Instruction::LocalGet(fuel));
        function.instruction(&Instruction::I64GeU);
        function.instruction(&Instruction::BrIf(exit_depth));
    } else {
        function.instruction(&Instruction::LocalGet(hops_local));
        function.instruction(&Instruction::I32Const(1));
        function.instruction(&Instruction::I32Add);
        function.instruction(&Instruction::LocalTee(hops_local));
        function.instruction(&Instruction::I32Const(MULTI_ENTRY_HOP_CAP));
        function.instruction(&Instruction::I32GeU);
        function.instruction(&Instruction::BrIf(exit_depth));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum StructuredPath {
    Fallthrough { selector: Option<i32> },
    Branch(StructuredDestination),
}

fn structured_path(
    member: Option<usize>,
    next_heads: &[usize],
    labels: &[Option<StructuredDestination>],
    exit: StructuredDestination,
) -> Result<StructuredPath, EmitError> {
    let Some(member) = member else {
        return Ok(StructuredPath::Branch(exit));
    };
    if next_heads.contains(&member) {
        return Ok(StructuredPath::Fallthrough {
            selector: (next_heads.len() > 1).then_some(member as i32),
        });
    }
    Ok(StructuredPath::Branch(labels[member].ok_or_else(|| {
        EmitError(format!(
            "structured CFG has no active label for member {member}"
        ))
    })?))
}

fn emit_structured_fallthrough_selector(
    function: &mut Function,
    selector: Option<i32>,
    selector_local: u32,
) {
    if let Some(selector) = selector {
        function.instruction(&Instruction::I32Const(selector));
        function.instruction(&Instruction::LocalSet(selector_local));
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_structured_successor(
    function: &mut Function,
    regions: &[(&Region, Option<LoopBackedge>)],
    region_index: usize,
    local_map: &[Option<u32>],
    next_heads: &[usize],
    labels: &[Option<StructuredDestination>],
    selector_local: u32,
    main_scope: u32,
    exit_scope: u32,
    active_scopes: &[u32],
) -> Result<(), EmitError> {
    let region = regions[region_index].0;
    let exit = StructuredDestination {
        scope: exit_scope,
        selector: None,
    };
    match region.values.get(region.next_pc.0).map(|value| &value.op) {
        Some(Op::ConstI64(_) | Op::GuestPc(_)) => {
            let member = cfg_member_for_value(regions, region, region.next_pc);
            match structured_path(member, next_heads, labels, exit)? {
                StructuredPath::Fallthrough { selector } => {
                    emit_structured_fallthrough_selector(function, selector, selector_local);
                }
                StructuredPath::Branch(destination) => emit_structured_unconditional_branch(
                    function,
                    destination,
                    selector_local,
                    active_scopes,
                )?,
            }
        }
        Some(Op::SelectI64 {
            condition,
            if_true,
            if_false,
        }) => {
            let condition_local = local_map[condition.0]
                .ok_or_else(|| EmitError("missing structured branch condition local".into()))?;
            let if_true = structured_path(
                cfg_member_for_value(regions, region, *if_true),
                next_heads,
                labels,
                exit,
            )?;
            let if_false = structured_path(
                cfg_member_for_value(regions, region, *if_false),
                next_heads,
                labels,
                exit,
            )?;
            match (if_true, if_false) {
                (
                    StructuredPath::Fallthrough {
                        selector: true_selector,
                    },
                    StructuredPath::Fallthrough {
                        selector: false_selector,
                    },
                ) => match (true_selector, false_selector) {
                    (Some(if_true), Some(if_false)) if if_true != if_false => {
                        function.instruction(&Instruction::I32Const(if_true));
                        function.instruction(&Instruction::I32Const(if_false));
                        function.instruction(&Instruction::LocalGet(condition_local));
                        function.instruction(&Instruction::Select);
                        function.instruction(&Instruction::LocalSet(selector_local));
                    }
                    (selector, _) => {
                        emit_structured_fallthrough_selector(function, selector, selector_local)
                    }
                },
                (StructuredPath::Fallthrough { selector }, StructuredPath::Branch(destination)) => {
                    emit_structured_conditional_branch(
                        function,
                        condition_local,
                        true,
                        destination,
                        selector_local,
                        active_scopes,
                    )?;
                    emit_structured_fallthrough_selector(function, selector, selector_local);
                }
                (StructuredPath::Branch(destination), StructuredPath::Fallthrough { selector }) => {
                    emit_structured_conditional_branch(
                        function,
                        condition_local,
                        false,
                        destination,
                        selector_local,
                        active_scopes,
                    )?;
                    emit_structured_fallthrough_selector(function, selector, selector_local);
                }
                (StructuredPath::Branch(if_true), StructuredPath::Branch(if_false)) => {
                    emit_structured_conditional_branch(
                        function,
                        condition_local,
                        false,
                        if_true,
                        selector_local,
                        active_scopes,
                    )?;
                    emit_structured_unconditional_branch(
                        function,
                        if_false,
                        selector_local,
                        active_scopes,
                    )?;
                }
            }
        }
        _ => {
            // Dynamic control can re-enter this module, but only through the
            // single outer entry dispatcher. It resolves the exact PC on the
            // next loop iteration and exits if the destination is external.
            function.instruction(&Instruction::I32Const(-1));
            function.instruction(&Instruction::LocalSet(selector_local));
            function.instruction(&Instruction::Br(structured_depth(
                active_scopes,
                main_scope,
            )?));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_cached_structured_member(
    function: &mut Function,
    region: &Region,
    layout: JitLayout,
    helpers: HelperImports,
    memory_temps: Option<MemoryTemps>,
    state: &CachedStateLocals,
    local_map: &[Option<u32>],
) -> Result<(), EmitError> {
    emit_cached_body(
        function,
        region,
        layout,
        helpers,
        memory_temps,
        state,
        local_map,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_cached_structured_cfg(
    function: &mut Function,
    regions: &[(&Region, Option<LoopBackedge>)],
    local_maps: &[Vec<Option<u32>>],
    layout: JitLayout,
    helpers: HelperImports,
    memory_temps: Option<MemoryTemps>,
    state: &CachedStateLocals,
    selector_local: u32,
    hops_local: u32,
) -> Result<(), EmitError> {
    let successors = cfg_successors(regions);
    let entries: Vec<usize> = (0..regions.len()).collect();
    let structures = structure::stackify(&successors, &entries, STRUCTURED_CFG_DUPLICATION_LIMIT);

    function.instruction(&Instruction::I32Const(-1));
    function.instruction(&Instruction::LocalSet(selector_local));

    let exit_scope = 0u32;
    let main_scope = 1u32;
    let mut next_scope = 2u32;
    let mut active_scopes = vec![exit_scope, main_scope];
    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));

    // A direct structured edge already carries a dense selector when one is
    // required. Initial and dynamic entries resolve the architectural PC once.
    function.instruction(&Instruction::LocalGet(selector_local));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32LtS);
    function.instruction(&Instruction::If(BlockType::Empty));
    let mut pc_order: Vec<usize> = (0..regions.len()).collect();
    pc_order.sort_unstable_by_key(|&index| regions[index].0.entry_pc);
    emit_cached_index_tree(function, &pc_order, regions, layout, state, selector_local);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(selector_local));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32LtS);
    function.instruction(&Instruction::BrIf(structured_depth(
        &active_scopes,
        exit_scope,
    )?));

    let mut labels = vec![None; regions.len()];
    let mut work: VecDeque<StructuredWork> =
        structures.into_iter().map(StructuredWork::Node).collect();

    while let Some(item) = work.pop_front() {
        let next_heads = work.iter().find_map(|pending| match pending {
            StructuredWork::Node(structure) => Some(structure.head()),
            StructuredWork::End { .. } => None,
        });
        match item {
            StructuredWork::Node(Structure::Basic(member)) => {
                // Stackification can duplicate a basic member. Instrument the
                // emitted occurrence, rather than its unique IR node, so the
                // counters follow the path the generated Wasm actually takes.
                // A precise side exit can retire a prefix of the scheduled
                // member; `DPROF_REGION_INSNS` remains the authoritative exact
                // retirement total and makes that difference observable.
                emit_structured_profile(function, layout, regions[member].0);
                emit_cached_structured_member(
                    function,
                    regions[member].0,
                    layout,
                    helpers,
                    memory_temps,
                    state,
                    &local_maps[member],
                )?;
                emit_structured_safety(function, state, hops_local, exit_scope, &active_scopes)?;
                emit_structured_successor(
                    function,
                    regions,
                    member,
                    &local_maps[member],
                    next_heads.as_deref().unwrap_or(&[]),
                    &labels,
                    selector_local,
                    main_scope,
                    exit_scope,
                    &active_scopes,
                )?;
            }
            StructuredWork::Node(Structure::Dispatcher(dispatch_entries)) => {
                let next_heads = next_heads.unwrap_or_default();
                for member in dispatch_entries {
                    if next_heads.contains(&member) {
                        continue;
                    }
                    let destination = labels[member].ok_or_else(|| {
                        EmitError(format!(
                            "structured dispatcher has no label for member {member}"
                        ))
                    })?;
                    emit_structured_selector_branch(
                        function,
                        member,
                        destination,
                        selector_local,
                        &active_scopes,
                    )?;
                }
                // Valid selectors matching a head reach the next structure by
                // fallthrough. Every module/dynamic entry was range-checked at
                // the main-loop header.
            }
            StructuredWork::Node(Structure::Block(children)) => {
                let targets = next_heads.unwrap_or_default();
                let scope = next_scope;
                next_scope += 1;
                function.instruction(&Instruction::Block(BlockType::Empty));
                active_scopes.push(scope);
                let multi_target = targets.len() > 1;
                let mut previous = Vec::with_capacity(targets.len());
                for &target in &targets {
                    previous.push(labels[target].replace(StructuredDestination {
                        scope,
                        selector: multi_target.then_some(target as i32),
                    }));
                }
                work.push_front(StructuredWork::End {
                    scope,
                    targets,
                    previous,
                });
                for child in children.into_iter().rev() {
                    work.push_front(StructuredWork::Node(child));
                }
            }
            StructuredWork::Node(Structure::Loop(children)) => {
                let targets = children.first().map_or_else(Vec::new, Structure::head);
                if targets.is_empty() {
                    return Err(EmitError("structured CFG contains an empty loop".into()));
                }
                let scope = next_scope;
                next_scope += 1;
                function.instruction(&Instruction::Loop(BlockType::Empty));
                active_scopes.push(scope);
                let multi_target = targets.len() > 1;
                let mut previous = Vec::with_capacity(targets.len());
                for &target in &targets {
                    previous.push(labels[target].replace(StructuredDestination {
                        scope,
                        selector: multi_target.then_some(target as i32),
                    }));
                }
                work.push_front(StructuredWork::End {
                    scope,
                    targets,
                    previous,
                });
                for child in children.into_iter().rev() {
                    work.push_front(StructuredWork::Node(child));
                }
            }
            StructuredWork::End {
                scope,
                targets,
                previous,
            } => {
                if active_scopes.pop() != Some(scope) {
                    return Err(EmitError("unbalanced structured CFG scope".into()));
                }
                for (target, old) in targets.into_iter().zip(previous) {
                    labels[target] = old;
                }
                function.instruction(&Instruction::End);
            }
        }
    }

    if active_scopes != [exit_scope, main_scope] {
        return Err(EmitError("structured CFG left an open scope".into()));
    }
    // Defensive fallthrough: a well-formed terminal basic block has already
    // branched either to a covered successor or the common exact exit.
    function.instruction(&Instruction::Br(structured_depth(
        &active_scopes,
        exit_scope,
    )?));
    active_scopes.pop();
    function.instruction(&Instruction::End);
    active_scopes.pop();
    function.instruction(&Instruction::End);
    Ok(())
}

fn emit_cached_index_tree(
    function: &mut Function,
    order: &[usize],
    regions: &[(&Region, Option<LoopBackedge>)],
    layout: JitLayout,
    state: &CachedStateLocals,
    selector_local: u32,
) {
    if order.len() <= 3 {
        for &index in order {
            function.instruction(&Instruction::LocalGet(state.pc));
            emit_guest_pc(function, regions[index].0.entry_pc, layout);
            function.instruction(&Instruction::I64Eq);
            function.instruction(&Instruction::If(BlockType::Empty));
            function.instruction(&Instruction::I32Const(index as i32));
            function.instruction(&Instruction::LocalSet(selector_local));
            function.instruction(&Instruction::End);
        }
        return;
    }

    let middle = order.len() / 2;
    let pivot = regions[order[middle]].0.entry_pc;
    function.instruction(&Instruction::LocalGet(state.pc));
    emit_guest_pc(function, pivot, layout);
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_cached_index_tree(
        function,
        &order[..middle],
        regions,
        layout,
        state,
        selector_local,
    );
    function.instruction(&Instruction::Else);
    emit_cached_index_tree(
        function,
        &order[middle..],
        regions,
        layout,
        state,
        selector_local,
    );
    function.instruction(&Instruction::End);
}

fn emit_valid_i64(function: &mut Function, valid_local: u32, bit: usize) {
    function.instruction(&Instruction::LocalGet(valid_local));
    function.instruction(&Instruction::I64Const(1i64 << bit));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::I64Ne);
}

fn emit_mark_valid_i64(function: &mut Function, valid_local: u32, bit: usize) {
    function.instruction(&Instruction::LocalGet(valid_local));
    function.instruction(&Instruction::I64Const(1i64 << bit));
    function.instruction(&Instruction::I64Or);
    function.instruction(&Instruction::LocalSet(valid_local));
}

fn emit_lazy_state_read_i64(
    function: &mut Function,
    base: u32,
    state_local: u32,
    valid_local: Option<u32>,
    reg: usize,
) {
    if let Some(valid) = valid_local {
        emit_valid_i64(function, valid, reg);
        function.instruction(&Instruction::I32Eqz);
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::I32Const(base as i32));
        function.instruction(&Instruction::I64Load(memarg(3, reg as u64 * 8)));
        function.instruction(&Instruction::LocalSet(state_local));
        emit_mark_valid_i64(function, valid, reg);
        function.instruction(&Instruction::End);
    }
    function.instruction(&Instruction::LocalGet(state_local));
}

fn is_materialized_x(state: &CachedStateLocals, reg: u8) -> bool {
    state.materialized_x & (1u32 << reg) != 0
}

fn cached_x_member_local(
    region: &Region,
    state: &CachedStateLocals,
    local_map: &[Option<u32>],
    reg: u8,
) -> Result<u32, EmitError> {
    if let Some(local) = state.x[reg as usize] {
        return Ok(local);
    }
    if !is_materialized_x(state, reg) {
        return Err(EmitError(format!(
            "x{reg} has neither resident nor materialized cached state"
        )));
    }
    let value = read_x_value(region, reg)
        .ok_or_else(|| EmitError(format!("materialized x{reg} has no member ReadX value")))?;
    local_map[value.0]
        .ok_or_else(|| EmitError(format!("materialized x{reg} has no member SSA local")))
}

/// Make one cached-member integer input available in its canonical local.
/// Resident state keeps the existing eager/lazy behavior. Materialized state
/// loads directly into the member's ReadX SSA local.
fn emit_cached_x_member_input(
    function: &mut Function,
    region: &Region,
    layout: JitLayout,
    state: &CachedStateLocals,
    local_map: &[Option<u32>],
    reg: u8,
) -> Result<u32, EmitError> {
    let local = cached_x_member_local(region, state, local_map, reg)?;
    if state.x[reg as usize].is_some() {
        emit_lazy_state_read_i64(function, layout.x_base, local, state.valid_x, reg as usize);
        function.instruction(&Instruction::Drop);
    } else {
        function.instruction(&Instruction::I32Const(layout.x_base as i32));
        function.instruction(&Instruction::I64Load(memarg(3, u64::from(reg) * 8)));
        function.instruction(&Instruction::LocalSet(local));
    }
    Ok(local)
}

fn emit_materialized_x_store(
    function: &mut Function,
    layout: JitLayout,
    reg: u8,
    value_local: u32,
) {
    function.instruction(&Instruction::I32Const(layout.x_base as i32));
    function.instruction(&Instruction::LocalGet(value_local));
    function.instruction(&Instruction::I64Store(memarg(3, u64::from(reg) * 8)));
}

fn emit_lazy_fcsr_read(
    function: &mut Function,
    layout: JitLayout,
    state_local: u32,
    valid_local: Option<u32>,
) {
    if let Some(valid) = valid_local {
        function.instruction(&Instruction::LocalGet(valid));
        function.instruction(&Instruction::I32Eqz);
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
        function.instruction(&Instruction::I32Load(memarg(2, 0)));
        function.instruction(&Instruction::LocalSet(state_local));
        function.instruction(&Instruction::I32Const(1));
        function.instruction(&Instruction::LocalSet(valid));
        function.instruction(&Instruction::End);
    }
    function.instruction(&Instruction::LocalGet(state_local));
}

#[allow(clippy::too_many_arguments)]
fn emit_cached_dispatch_tree(
    function: &mut Function,
    order: &[usize],
    regions: &[(&Region, Option<LoopBackedge>)],
    local_maps: &[Vec<Option<u32>>],
    layout: JitLayout,
    helpers: HelperImports,
    memory_temps: Option<MemoryTemps>,
    state: &CachedStateLocals,
    matched_local: u32,
) -> Result<(), EmitError> {
    if order.len() <= 3 {
        for &index in order {
            let (region, loop_backedge) = regions[index];
            function.instruction(&Instruction::LocalGet(state.pc));
            emit_guest_pc(function, region.entry_pc, layout);
            function.instruction(&Instruction::I64Eq);
            function.instruction(&Instruction::If(BlockType::Empty));
            emit_cached_body(
                function,
                region,
                layout,
                helpers,
                memory_temps,
                state,
                &local_maps[index],
                loop_backedge,
            )?;
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::LocalSet(matched_local));
            function.instruction(&Instruction::End);
        }
        return Ok(());
    }

    let middle = order.len() / 2;
    let pivot = regions[order[middle]].0.entry_pc;
    function.instruction(&Instruction::LocalGet(state.pc));
    emit_guest_pc(function, pivot, layout);
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_cached_dispatch_tree(
        function,
        &order[..middle],
        regions,
        local_maps,
        layout,
        helpers,
        memory_temps,
        state,
        matched_local,
    )?;
    function.instruction(&Instruction::Else);
    emit_cached_dispatch_tree(
        function,
        &order[middle..],
        regions,
        local_maps,
        layout,
        helpers,
        memory_temps,
        state,
        matched_local,
    )?;
    function.instruction(&Instruction::End);
    Ok(())
}

fn emit_copy_base_address(
    function: &mut Function,
    root_local: u32,
    offset: i64,
    destination_local: u32,
) {
    function.instruction(&Instruction::LocalGet(root_local));
    if offset != 0 {
        function.instruction(&Instruction::I64Const(offset));
        function.instruction(&Instruction::I64Add);
    }
    function.instruction(&Instruction::LocalSet(destination_local));
}

#[allow(clippy::too_many_arguments)]
fn emit_dense_copy_setup(
    function: &mut Function,
    plan: &DenseCopyPlan,
    layout: JitLayout,
    helpers: HelperImports,
    temps: MemoryTemps,
    local_map: &[Option<u32>],
    mut side_exit: impl FnMut(&mut Function) -> Result<(), EmitError>,
) -> Result<(), EmitError> {
    let copy = temps
        .copy
        .ok_or_else(|| EmitError("dense-copy plan lacks address temporaries".into()))?;
    emit_copy_base_address(
        function,
        local_map[plan.source_root.0].expect("dense-copy source root local"),
        plan.source_base_offset,
        copy.source_address,
    );
    emit_memory_address(
        function,
        layout,
        helpers,
        Some(temps),
        copy.source_address,
        plan.bytes,
        false,
        |function| side_exit(function),
    )?;
    function.instruction(&Instruction::LocalSet(copy.source_linear));

    emit_copy_base_address(
        function,
        local_map[plan.destination_root.0].expect("dense-copy destination root local"),
        plan.destination_base_offset,
        copy.destination_address,
    );
    emit_memory_address(
        function,
        layout,
        helpers,
        Some(temps),
        copy.destination_address,
        plan.bytes,
        true,
        |function| side_exit(function),
    )?;
    function.instruction(&Instruction::LocalSet(copy.destination_linear));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_dense_store_setup(
    function: &mut Function,
    plan: &DenseStorePlan,
    layout: JitLayout,
    helpers: HelperImports,
    temps: MemoryTemps,
    local_map: &[Option<u32>],
    mut side_exit: impl FnMut(&mut Function) -> Result<(), EmitError>,
) -> Result<(), EmitError> {
    let range = temps
        .copy
        .ok_or_else(|| EmitError("dense-store plan lacks address temporaries".into()))?;
    emit_copy_base_address(
        function,
        local_map[plan.destination_root.0].expect("dense-store destination root local"),
        plan.destination_base_offset,
        range.destination_address,
    );
    emit_memory_address(
        function,
        layout,
        helpers,
        Some(temps),
        range.destination_address,
        plan.bytes,
        true,
        |function| side_exit(function),
    )?;
    function.instruction(&Instruction::LocalSet(range.destination_linear));
    Ok(())
}

/// Execute a common-word dense store as one `memory.fill` when the runtime
/// value is a broadcast byte (zeroing and ordinary memset). The guarded scalar
/// arm preserves exact behavior for every other repeated 64-bit word. The
/// caller has already proved the complete range is ordinary direct RAM, so no
/// guest-visible fault or MMIO observation can occur between these stores.
fn emit_dense_fill(
    function: &mut Function,
    plan: &DenseStorePlan,
    destination_linear: u32,
    value_local: u32,
) -> Result<(), EmitError> {
    let bytes = i32::try_from(plan.bytes)
        .map_err(|_| EmitError("dense-store range exceeds Wasm32 bulk-memory length".into()))?;

    function.instruction(&Instruction::LocalGet(value_local));
    function.instruction(&Instruction::I64Const(0xff));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(0x0101_0101_0101_0101));
    function.instruction(&Instruction::I64Mul);
    function.instruction(&Instruction::LocalGet(value_local));
    function.instruction(&Instruction::I64Eq);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(destination_linear));
    function.instruction(&Instruction::LocalGet(value_local));
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Const(bytes));
    function.instruction(&Instruction::MemoryFill(0));
    function.instruction(&Instruction::Else);
    let mut accesses = plan.accesses.clone();
    accesses.sort_unstable_by_key(|access| access.destination_offset);
    for access in accesses {
        function.instruction(&Instruction::LocalGet(destination_linear));
        function.instruction(&Instruction::LocalGet(value_local));
        function.instruction(&Instruction::I64Store(memarg(3, access.destination_offset)));
    }
    function.instruction(&Instruction::End);
    Ok(())
}

fn emit_fuel_value(function: &mut Function, fuel_local: Option<u32>, fuel_addr: u32) -> bool {
    if let Some(local) = fuel_local {
        function.instruction(&Instruction::LocalGet(local));
        true
    } else if fuel_addr != 0 {
        function.instruction(&Instruction::I32Const(fuel_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        true
    } else {
        false
    }
}

fn emit_min_i64_local(function: &mut Function, target: u32, candidate: u32) {
    function.instruction(&Instruction::LocalGet(target));
    function.instruction(&Instruction::LocalGet(candidate));
    function.instruction(&Instruction::LocalGet(target));
    function.instruction(&Instruction::LocalGet(candidate));
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::Select);
    function.instruction(&Instruction::LocalSet(target));
}

/// Call the runtime's proved-loop bulk-copy helper from standalone and cached
/// loop bodies. The helper validates each source/destination page pair before
/// modifying memory and returns only the number of complete guest iterations
/// it committed. A zero return falls through to the untouched scalar loop.
#[allow(clippy::too_many_arguments)]
fn emit_bulk_copy_call(
    function: &mut Function,
    plan: BulkCopyLoopPlan,
    region_retired: u32,
    helpers: HelperImports,
    temps: BulkCopyTemps,
    source_local: u32,
    destination_local: u32,
    count_local: u32,
    limit_local: u32,
    retired_local: u32,
    fuel_local: Option<u32>,
    fuel_addr: u32,
) -> Result<(), EmitError> {
    // The proved loop runs floor(count/iteration_bytes) iterations. The guard
    // on the invariant limit register is essential: it is initialized
    // by the predecessor block and therefore appears as ReadX, not a constant,
    // in the loop member itself.
    function.instruction(&Instruction::LocalGet(count_local));
    function.instruction(&Instruction::I64Const(-(plan.bytes_per_iteration as i64)));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::LocalSet(temps.request));

    // Preserve the scalar loop's fuel contract.  The first iteration is
    // always permitted; subsequent iterations fit ceil(remaining/body_cost).
    if emit_fuel_value(function, fuel_local, fuel_addr) {
        function.instruction(&Instruction::LocalGet(retired_local));
        function.instruction(&Instruction::I64GtU);
        function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        emit_fuel_value(function, fuel_local, fuel_addr);
        function.instruction(&Instruction::LocalGet(retired_local));
        function.instruction(&Instruction::I64Sub);
        function.instruction(&Instruction::Else);
        function.instruction(&Instruction::I64Const(0));
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::LocalSet(temps.fuel_bytes));

        function.instruction(&Instruction::LocalGet(temps.fuel_bytes));
        function.instruction(&Instruction::I64Const(i64::from(region_retired)));
        function.instruction(&Instruction::I64DivU);
        function.instruction(&Instruction::LocalGet(temps.fuel_bytes));
        function.instruction(&Instruction::I64Const(i64::from(region_retired)));
        function.instruction(&Instruction::I64RemU);
        function.instruction(&Instruction::I64Const(0));
        function.instruction(&Instruction::I64Ne);
        function.instruction(&Instruction::I64ExtendI32U);
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::LocalSet(temps.fuel_bytes));

        function.instruction(&Instruction::LocalGet(temps.fuel_bytes));
        function.instruction(&Instruction::I64Const(1));
        function.instruction(&Instruction::I64LtU);
        function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        function.instruction(&Instruction::I64Const(1));
        function.instruction(&Instruction::Else);
        function.instruction(&Instruction::LocalGet(temps.fuel_bytes));
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::I64Const(plan.bytes_per_iteration as i64));
        function.instruction(&Instruction::I64Mul);
        function.instruction(&Instruction::LocalSet(temps.fuel_bytes));

        emit_min_i64_local(function, temps.request, temps.fuel_bytes);
    }

    function.instruction(&Instruction::LocalGet(limit_local));
    function.instruction(&Instruction::I64Const(plan.limit_value as i64));
    function.instruction(&Instruction::I64Eq);
    function.instruction(&Instruction::LocalGet(temps.request));
    function.instruction(&Instruction::I64Const(plan.bytes_per_iteration as i64));
    function.instruction(&Instruction::I64GeU);
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalGet(source_local));
    function.instruction(&Instruction::LocalGet(destination_local));
    function.instruction(&Instruction::LocalGet(temps.request));
    function.instruction(&Instruction::I32Const(plan.bytes_per_iteration as i32));
    function.instruction(&Instruction::I32Const(i32::from(plan.step < 0)));
    function.instruction(&Instruction::I32Const(i32::from(plan.value_reg)));
    function.instruction(&Instruction::Call(helpers.bulk_copy_index().ok_or_else(
        || EmitError("bulk-copy loop lacks the system helper import".into()),
    )?));
    function.instruction(&Instruction::LocalSet(temps.result));
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::LocalSet(temps.result));
    function.instruction(&Instruction::End);
    Ok(())
}

fn emit_add_or_sub_local(
    function: &mut Function,
    target_local: u32,
    amount_local: u32,
    subtract: bool,
) {
    function.instruction(&Instruction::LocalGet(target_local));
    function.instruction(&Instruction::LocalGet(amount_local));
    function.instruction(&if subtract {
        Instruction::I64Sub
    } else {
        Instruction::I64Add
    });
    function.instruction(&Instruction::LocalSet(target_local));
}

fn emit_bulk_retired(
    function: &mut Function,
    retired_local: u32,
    result_local: u32,
    bytes_per_iteration: u64,
    retired_per_iteration: u32,
) {
    function.instruction(&Instruction::LocalGet(retired_local));
    function.instruction(&Instruction::LocalGet(result_local));
    function.instruction(&Instruction::I64Const(bytes_per_iteration as i64));
    function.instruction(&Instruction::I64DivU);
    function.instruction(&Instruction::I64Const(i64::from(retired_per_iteration)));
    function.instruction(&Instruction::I64Mul);
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(retired_local));
}

#[allow(clippy::too_many_arguments)]
fn emit_cached_body(
    function: &mut Function,
    region: &Region,
    layout: JitLayout,
    helpers: HelperImports,
    memory_temps: Option<MemoryTemps>,
    state: &CachedStateLocals,
    local_map: &[Option<u32>],
    loop_backedge: Option<LoopBackedge>,
) -> Result<(), EmitError> {
    let memory_temps = memory_temps_for_region(memory_temps, region, layout);
    let copy_plan = match (layout.sys, memory_temps.and_then(|temps| temps.copy)) {
        (Some(memory), Some(_)) => {
            dense_copy_plan(region, usize::from(memory.cache_min_accesses.max(1)))
        }
        _ => None,
    }
    // A failed whole-loop helper must fall back to the exact scalar body. The
    // per-iteration range hoist would itself side-exit when either 64-byte
    // range straddles a page, resuming at interior ld/sd PCs and permanently
    // fragmenting the canonical loop header that the bulk path needs.
    .filter(|_| bulk_copy_loop_plan(region).is_none());
    let store_plan = match (layout.sys, memory_temps.and_then(|temps| temps.copy)) {
        (Some(memory), Some(_)) => {
            dense_store_plan(region, usize::from(memory.cache_min_accesses.max(1)))
        }
        _ => None,
    };
    // Structured-CFG modules own backedges outside this member and therefore
    // intentionally pass `None` here. The IR recognizer itself proves the
    // canonical backedge, so it is the authority for the bulk path; the
    // optional marker controls only whether the scalar fallback forms a
    // nested local Wasm loop.
    let bulk_plan = bulk_copy_loop_plan(region);
    let bulk_temps = bulk_plan.and_then(|_| memory_temps.and_then(|temps| temps.bulk_copy));
    let all_defined = vec![true; region.values.len()];
    let mut bulk_preloaded_x = 0u32;
    let bulk_x_locals = if let (Some(plan), Some(temps)) = (bulk_plan, bulk_temps) {
        let mut inputs = vec![
            plan.source_reg,
            plan.destination_reg,
            plan.count_reg,
            plan.limit_reg,
        ];
        inputs.sort_unstable();
        inputs.dedup();
        for reg in inputs {
            emit_cached_x_member_input(function, region, layout, state, local_map, reg)?;
            if is_materialized_x(state, reg) {
                bulk_preloaded_x |= 1u32 << reg;
            }
        }
        let source_local = cached_x_member_local(region, state, local_map, plan.source_reg)?;
        let destination_local =
            cached_x_member_local(region, state, local_map, plan.destination_reg)?;
        let count_local = cached_x_member_local(region, state, local_map, plan.count_reg)?;
        let limit_local = cached_x_member_local(region, state, local_map, plan.limit_reg)?;
        emit_bulk_copy_call(
            function,
            plan,
            region.retired,
            helpers,
            temps,
            source_local,
            destination_local,
            count_local,
            limit_local,
            state.retired,
            state.fuel,
            layout.fuel_addr,
        )?;
        function.instruction(&Instruction::LocalGet(temps.result));
        function.instruction(&Instruction::I64Eqz);
        function.instruction(&Instruction::If(BlockType::Empty));
        Some((source_local, destination_local, count_local, limit_local))
    } else {
        None
    };
    if loop_backedge.is_some() {
        function.instruction(&Instruction::Loop(BlockType::Empty));
    }
    for position in 0..=region.values.len() {
        for effect in &region.effects {
            match effect {
                Effect::Store {
                    position: effect_position,
                    address,
                    value,
                    kind,
                    condition,
                    exit,
                } if *effect_position == position => {
                    if let Some(condition) = condition {
                        function.instruction(&Instruction::LocalGet(
                            local_map[condition.0].expect("cached condition local"),
                        ));
                        function.instruction(&Instruction::If(BlockType::Empty));
                    }
                    let copy_access = copy_plan
                        .as_ref()
                        .and_then(|plan| plan.store_access(*effect_position, *address, *value));
                    let address_local = local_map[address.0].expect("cached address local");
                    if let (Some(access), Some(copy)) =
                        (copy_access, memory_temps.and_then(|temps| temps.copy))
                    {
                        if copy_plan
                            .as_ref()
                            .is_some_and(|plan| plan.setup_position == *effect_position)
                        {
                            emit_dense_copy_setup(
                                function,
                                copy_plan.as_ref().expect("copy plan"),
                                layout,
                                helpers,
                                memory_temps.expect("copy memory temporaries"),
                                local_map,
                                |function| {
                                    emit_cached_side_exit(
                                        function, region, layout, state, local_map, exit, None,
                                    )
                                },
                            )?;
                        }
                        function.instruction(&Instruction::LocalGet(copy.destination_linear));
                        function.instruction(&Instruction::LocalGet(
                            local_map[value.0].expect("cached copy value local"),
                        ));
                        function.instruction(&Instruction::I64Store(memarg(
                            3,
                            access.destination_offset,
                        )));
                    } else if let (Some(access), Some(range)) = (
                        store_plan
                            .as_ref()
                            .and_then(|plan| plan.store_access(*effect_position, *address)),
                        memory_temps.and_then(|temps| temps.copy),
                    ) {
                        if store_plan
                            .as_ref()
                            .is_some_and(|plan| plan.setup_position == *effect_position)
                        {
                            emit_dense_store_setup(
                                function,
                                store_plan.as_ref().expect("store plan"),
                                layout,
                                helpers,
                                memory_temps.expect("store memory temporaries"),
                                local_map,
                                |function| {
                                    emit_cached_side_exit(
                                        function, region, layout, state, local_map, exit, None,
                                    )
                                },
                            )?;
                        }
                        if let Some(fill_value) =
                            store_plan.as_ref().and_then(|plan| plan.fill_value)
                        {
                            if store_plan
                                .as_ref()
                                .is_some_and(|plan| plan.setup_position == *effect_position)
                            {
                                emit_dense_fill(
                                    function,
                                    store_plan.as_ref().expect("store plan"),
                                    range.destination_linear,
                                    local_map[fill_value.0].expect("cached fill value local"),
                                )?;
                            }
                        } else {
                            function.instruction(&Instruction::LocalGet(range.destination_linear));
                            function.instruction(&Instruction::LocalGet(
                                local_map[value.0].expect("cached store value local"),
                            ));
                            function.instruction(&Instruction::I64Store(memarg(
                                3,
                                access.destination_offset,
                            )));
                        }
                    } else {
                        emit_memory_address(
                            function,
                            layout,
                            helpers,
                            memory_temps,
                            address_local,
                            kind.bytes(),
                            true,
                            |function| {
                                emit_cached_side_exit(
                                    function, region, layout, state, local_map, exit, None,
                                )
                            },
                        )?;
                        function.instruction(&Instruction::LocalGet(
                            local_map[value.0].expect("cached store value local"),
                        ));
                        function.instruction(&match kind {
                            StoreKind::I8 => Instruction::I64Store8(memarg(0, 0)),
                            StoreKind::I16 => Instruction::I64Store16(memarg(0, 0)),
                            StoreKind::I32 => Instruction::I64Store32(memarg(0, 0)),
                            StoreKind::I64 => Instruction::I64Store(memarg(0, 0)),
                        });
                    }
                    if condition.is_some() {
                        function.instruction(&Instruction::End);
                        emit_reservation_clear(function, helpers, address_local)?;
                    }
                }
                Effect::Guard {
                    position: effect_position,
                    condition,
                    exit,
                } if *effect_position == position => {
                    function.instruction(&Instruction::LocalGet(
                        local_map[condition.0].expect("cached guard local"),
                    ));
                    function.instruction(&Instruction::If(BlockType::Empty));
                    emit_cached_side_exit(function, region, layout, state, local_map, exit, None)?;
                    function.instruction(&Instruction::Return);
                    function.instruction(&Instruction::End);
                }
                Effect::GuardTarget {
                    position: effect_position,
                    target,
                    expected,
                    exit,
                } if *effect_position == position => {
                    let target_local = local_map[target.0].expect("cached target local");
                    function.instruction(&Instruction::LocalGet(target_local));
                    emit_guest_pc(function, *expected, layout);
                    function.instruction(&Instruction::I64Ne);
                    function.instruction(&Instruction::If(BlockType::Empty));
                    emit_cached_side_exit(
                        function,
                        region,
                        layout,
                        state,
                        local_map,
                        exit,
                        Some(target_local),
                    )?;
                    emit_ic_guard_miss(function, region, layout, target_local);
                    function.instruction(&Instruction::Return);
                    function.instruction(&Instruction::End);
                }
                Effect::FpState {
                    position: effect_position,
                    dirty,
                    exit,
                } if *effect_position == position => {
                    emit_fp_state(function, layout, *dirty, |function| {
                        emit_cached_side_exit(
                            function, region, layout, state, local_map, exit, None,
                        )
                    })?;
                }
                _ => {}
            }
        }

        let Some(data) = region.values.get(position) else {
            continue;
        };
        let output_local = local_map[position].expect("cached SSA local");
        match &data.op {
            Op::ReadX(reg) => {
                if let Some(state_local) = state.x[*reg as usize] {
                    emit_lazy_state_read_i64(
                        function,
                        layout.x_base,
                        state_local,
                        state.valid_x,
                        *reg as usize,
                    );
                } else if is_materialized_x(state, *reg) {
                    if bulk_preloaded_x & (1u32 << *reg) != 0 {
                        function.instruction(&Instruction::LocalGet(output_local));
                    } else {
                        function.instruction(&Instruction::I32Const(layout.x_base as i32));
                        function.instruction(&Instruction::I64Load(memarg(3, u64::from(*reg) * 8)));
                    }
                } else {
                    return Err(EmitError(format!(
                        "cached integer input x{reg} has no state representation"
                    )));
                }
            }
            Op::ReadF(reg) => {
                emit_lazy_state_read_i64(
                    function,
                    layout.f_base,
                    state.f[*reg as usize].expect("cached FP input"),
                    state.valid_f,
                    *reg as usize,
                );
            }
            Op::ReadFcsr => {
                emit_lazy_fcsr_read(
                    function,
                    layout,
                    state.fcsr.expect("cached fcsr input"),
                    state.valid_fcsr,
                );
            }
            Op::Load {
                address,
                kind,
                exit,
            } => {
                let copy_access = copy_plan
                    .as_ref()
                    .and_then(|plan| plan.load_access(ValueId(position)));
                if let (Some(access), Some(copy)) =
                    (copy_access, memory_temps.and_then(|temps| temps.copy))
                {
                    if copy_plan
                        .as_ref()
                        .is_some_and(|plan| plan.setup_position == position)
                    {
                        emit_dense_copy_setup(
                            function,
                            copy_plan.as_ref().expect("copy plan"),
                            layout,
                            helpers,
                            memory_temps.expect("copy memory temporaries"),
                            local_map,
                            |function| {
                                emit_cached_side_exit(
                                    function, region, layout, state, local_map, exit, None,
                                )
                            },
                        )?;
                    }
                    function.instruction(&Instruction::LocalGet(copy.source_linear));
                    function.instruction(&Instruction::I64Load(memarg(3, access.source_offset)));
                } else {
                    let address_local = local_map[address.0].expect("cached load address local");
                    emit_memory_address(
                        function,
                        layout,
                        helpers,
                        memory_temps,
                        address_local,
                        kind.bytes(),
                        false,
                        |function| {
                            emit_cached_side_exit(
                                function, region, layout, state, local_map, exit, None,
                            )
                        },
                    )?;
                    function.instruction(&match kind {
                        LoadKind::I8S => Instruction::I64Load8S(memarg(0, 0)),
                        LoadKind::I16S => Instruction::I64Load16S(memarg(0, 0)),
                        LoadKind::I32S => Instruction::I64Load32S(memarg(0, 0)),
                        LoadKind::I64 => Instruction::I64Load(memarg(0, 0)),
                        LoadKind::I8U => Instruction::I64Load8U(memarg(0, 0)),
                        LoadKind::I16U => Instruction::I64Load16U(memarg(0, 0)),
                        LoadKind::I32U => Instruction::I64Load32U(memarg(0, 0)),
                    });
                }
            }
            Op::ExactFp {
                op,
                lhs,
                rhs,
                third,
                rm,
                fcsr,
                exit,
            } => {
                function.instruction(&Instruction::LocalGet(
                    local_map[rm.0].expect("cached rounding local"),
                ));
                function.instruction(&Instruction::I32Const(4));
                function.instruction(&Instruction::I32GtU);
                function.instruction(&Instruction::If(BlockType::Empty));
                emit_cached_side_exit(function, region, layout, state, local_map, exit, None)?;
                function.instruction(&Instruction::Return);
                function.instruction(&Instruction::End);
                emit_exact_fp_value(
                    function,
                    *op,
                    local_map[lhs.0].expect("cached helper lhs local"),
                    local_map[rhs.0].expect("cached helper rhs local"),
                    local_map[third.0].expect("cached helper third local"),
                    local_map[rm.0].expect("cached helper rounding local"),
                    local_map[fcsr.0].expect("cached helper fcsr local"),
                    output_local,
                    layout,
                    helpers,
                )?;
            }
            Op::Reservation { op, address } => {
                function.instruction(&Instruction::I32Const(match op {
                    ReservationOp::LoadReserved => 0,
                    ReservationOp::StoreConditional => 1,
                }));
                function.instruction(&Instruction::LocalGet(0));
                function.instruction(&Instruction::LocalGet(
                    local_map[address.0].expect("cached reservation address local"),
                ));
                function.instruction(&Instruction::Call(
                    helpers
                        .reservation_index()
                        .ok_or_else(|| EmitError("missing reservation helper import".into()))?,
                ));
            }
            _ => emit_value_body(
                function,
                region,
                layout,
                local_map,
                &all_defined,
                ValueId(position),
            )?,
        };
        function.instruction(&Instruction::LocalSet(output_local));
    }

    for &(reg, value) in &region.outputs {
        let value_local = local_map[value.0].expect("cached integer output local");
        if let Some(state_local) = state.x[reg as usize] {
            function.instruction(&Instruction::LocalGet(value_local));
            function.instruction(&Instruction::LocalSet(state_local));
            if let Some(valid) = state.valid_x {
                emit_mark_valid_i64(function, valid, reg as usize);
            }
        } else if is_materialized_x(state, reg) {
            emit_materialized_x_store(function, layout, reg, value_local);
        } else {
            return Err(EmitError(format!(
                "cached integer output x{reg} has no state representation"
            )));
        }
    }
    for &(reg, value) in &region.f_outputs {
        function.instruction(&Instruction::LocalGet(
            local_map[value.0].expect("cached FP output local"),
        ));
        function.instruction(&Instruction::LocalSet(
            state.f[reg as usize].expect("cached FP state local"),
        ));
        if let Some(valid) = state.valid_f {
            emit_mark_valid_i64(function, valid, reg as usize);
        }
    }
    if let Some(value) = region.fcsr_output {
        function.instruction(&Instruction::LocalGet(
            local_map[value.0].expect("cached fcsr output local"),
        ));
        function.instruction(&Instruction::LocalSet(
            state.fcsr.expect("cached fcsr state local"),
        ));
        if let Some(valid) = state.valid_fcsr {
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::LocalSet(valid));
        }
    }
    function.instruction(&Instruction::LocalGet(
        local_map[region.next_pc.0].expect("cached next-PC local"),
    ));
    function.instruction(&Instruction::LocalSet(state.pc));
    function.instruction(&Instruction::LocalGet(state.retired));
    function.instruction(&Instruction::I64Const(i64::from(region.retired)));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(state.retired));
    if loop_backedge.is_some() {
        // Re-enter only for the architectural backedge and while another
        // iteration is permitted by this invocation's cumulative budget. With
        // no fuel capability, fall through to the outer dispatcher so its
        // defensive hop cap still bounds malformed standalone embeddings.
        if let Some(fuel) = state.fuel {
            function.instruction(&Instruction::LocalGet(state.pc));
            emit_guest_pc(function, region.entry_pc, layout);
            function.instruction(&Instruction::I64Eq);
            function.instruction(&Instruction::LocalGet(state.retired));
            function.instruction(&Instruction::LocalGet(fuel));
            function.instruction(&Instruction::I64LtU);
            function.instruction(&Instruction::I32And);
            function.instruction(&Instruction::BrIf(0));
        }
        function.instruction(&Instruction::End);
    }
    if let (Some(plan), Some(temps), Some((source_local, destination_local, count_local, _))) =
        (bulk_plan, bulk_temps, bulk_x_locals)
    {
        function.instruction(&Instruction::Else);
        let subtract_pointers = plan.step < 0;
        for (reg, local, subtract) in [
            (plan.source_reg, source_local, subtract_pointers),
            (plan.destination_reg, destination_local, subtract_pointers),
            (plan.count_reg, count_local, true),
        ] {
            emit_add_or_sub_local(function, local, temps.result, subtract);
            if is_materialized_x(state, reg) {
                emit_materialized_x_store(function, layout, reg, local);
            }
        }
        if let Some(value_state) = state.x[plan.value_reg as usize] {
            function.instruction(&Instruction::I32Const(layout.x_base as i32));
            function.instruction(&Instruction::I64Load(memarg(
                3,
                u64::from(plan.value_reg) * 8,
            )));
            function.instruction(&Instruction::LocalSet(value_state));
        } else if !is_materialized_x(state, plan.value_reg) {
            return Err(EmitError(format!(
                "bulk-copy value x{} has no state representation",
                plan.value_reg
            )));
        }
        if let Some(valid) = state.valid_x {
            for reg in [
                plan.source_reg,
                plan.destination_reg,
                plan.count_reg,
                plan.value_reg,
            ] {
                if state.x[reg as usize].is_some() {
                    emit_mark_valid_i64(function, valid, reg as usize);
                }
            }
        }

        function.instruction(&Instruction::I64Const(plan.limit_value as i64));
        function.instruction(&Instruction::LocalGet(count_local));
        function.instruction(&Instruction::I64LtU);
        function.instruction(&Instruction::LocalSet(
            local_map[plan.condition.0].expect("bulk-copy condition local"),
        ));
        emit_guest_pc(function, region.entry_pc, layout);
        emit_guest_pc(function, plan.exit_pc, layout);
        function.instruction(&Instruction::LocalGet(
            local_map[plan.condition.0].expect("bulk-copy condition local"),
        ));
        function.instruction(&Instruction::Select);
        function.instruction(&Instruction::LocalTee(
            local_map[plan.next_pc.0].expect("bulk-copy next-PC local"),
        ));
        function.instruction(&Instruction::LocalSet(state.pc));
        emit_bulk_retired(
            function,
            state.retired,
            temps.result,
            plan.bytes_per_iteration,
            region.retired,
        );
        function.instruction(&Instruction::End);
    }
    Ok(())
}

fn emit_cached_side_exit(
    function: &mut Function,
    region: &Region,
    layout: JitLayout,
    state: &CachedStateLocals,
    local_map: &[Option<u32>],
    exit: &SideExit,
    dynamic_pc: Option<u32>,
) -> Result<(), EmitError> {
    for (reg, state_local) in state.x.iter().copied().enumerate() {
        let Some(state_local) = state_local else {
            continue;
        };
        if state.write_x & (1u32 << reg) == 0 {
            continue;
        }
        let exit_value = exit.outputs.iter().find(|&&(r, _)| r as usize == reg);
        if exit_value.is_none() {
            if let Some(valid) = state.valid_x {
                emit_valid_i64(function, valid, reg);
                function.instruction(&Instruction::If(BlockType::Empty));
            }
        }
        function.instruction(&Instruction::I32Const(layout.x_base as i32));
        if let Some(&(_, value)) = exit_value {
            function.instruction(&Instruction::LocalGet(
                local_map[value.0].ok_or_else(|| EmitError("missing cached exit value".into()))?,
            ));
        } else {
            function.instruction(&Instruction::LocalGet(state_local));
        }
        function.instruction(&Instruction::I64Store(memarg(3, reg as u64 * 8)));
        if exit_value.is_none() && state.valid_x.is_some() {
            function.instruction(&Instruction::End);
        }
    }
    // Previously completed members have already synchronized materialized
    // registers.  A precise exit therefore stores only values dirtied by the
    // current member before the fault/guard, exactly as captured by SideExit.
    for &(reg, value) in &exit.outputs {
        if !is_materialized_x(state, reg) {
            continue;
        }
        let value_local = local_map[value.0]
            .ok_or_else(|| EmitError("missing materialized integer exit value".into()))?;
        emit_materialized_x_store(function, layout, reg, value_local);
    }
    for (reg, state_local) in state.f.iter().copied().enumerate() {
        let Some(state_local) = state_local else {
            continue;
        };
        if state.write_f & (1u32 << reg) == 0 {
            continue;
        }
        let exit_value = exit.f_outputs.iter().find(|&&(r, _)| r as usize == reg);
        if exit_value.is_none() {
            if let Some(valid) = state.valid_f {
                emit_valid_i64(function, valid, reg);
                function.instruction(&Instruction::If(BlockType::Empty));
            }
        }
        function.instruction(&Instruction::I32Const(layout.f_base as i32));
        if let Some(&(_, value)) = exit_value {
            function.instruction(&Instruction::LocalGet(
                local_map[value.0]
                    .ok_or_else(|| EmitError("missing cached FP exit value".into()))?,
            ));
        } else {
            function.instruction(&Instruction::LocalGet(state_local));
        }
        function.instruction(&Instruction::I64Store(memarg(3, reg as u64 * 8)));
        if exit_value.is_none() && state.valid_f.is_some() {
            function.instruction(&Instruction::End);
        }
    }
    if let Some(state_local) = state.fcsr.filter(|_| state.write_fcsr) {
        let exit_value = exit.fcsr_output;
        if exit_value.is_none() {
            if let Some(valid) = state.valid_fcsr {
                function.instruction(&Instruction::LocalGet(valid));
                function.instruction(&Instruction::If(BlockType::Empty));
            }
        }
        function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
        if let Some(value) = exit_value {
            function.instruction(&Instruction::LocalGet(
                local_map[value.0]
                    .ok_or_else(|| EmitError("missing cached fcsr exit value".into()))?,
            ));
        } else {
            function.instruction(&Instruction::LocalGet(state_local));
        }
        function.instruction(&Instruction::I32Store(memarg(2, 0)));
        if exit_value.is_none() && state.valid_fcsr.is_some() {
            function.instruction(&Instruction::End);
        }
    }
    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    if let Some(local) = dynamic_pc {
        function.instruction(&Instruction::LocalGet(local));
    } else {
        emit_guest_pc(function, exit.guest_pc, layout);
    }
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::LocalGet(state.retired));
    if exit.retired != 0 {
        function.instruction(&Instruction::I64Const(i64::from(exit.retired)));
        function.instruction(&Instruction::I64Add);
    }
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    // The region argument documents that the side-exit snapshot belongs to
    // this body and keeps the API symmetric with the ordinary emitter.
    let _ = region;
    Ok(())
}

fn validate_emission(region: &Region, layout: JitLayout) -> Result<(), EmitError> {
    region
        .validate()
        .map_err(|error| EmitError(error.to_string()))?;
    if layout.mem.is_some() && layout.sys.is_some() {
        return Err(EmitError(
            "a region cannot use flat and full-system memory capabilities together".into(),
        ));
    }
    if let Some(memory) = layout.sys {
        validate_system_memory(memory)?;
    }
    Ok(())
}

fn emit_function(
    region: &Region,
    layout: JitLayout,
    loop_backedge: Option<LoopBackedge>,
    helpers: HelperImports,
) -> Result<Function, EmitError> {
    if region.has_effects() {
        return match loop_backedge {
            Some(loop_backedge) => emit_effectful_loop(region, layout, loop_backedge, helpers),
            None => emit_effectful(region, layout, helpers),
        };
    }

    if let Some(loop_backedge) = loop_backedge {
        return emit_single_latch_loop(region, layout, loop_backedge);
    }

    let uses = region.use_counts();
    // Values with multiple users are materialized once. Architectural reads
    // are always materialized before any state store, even with one user: a
    // JALR may write the same register it used to compute its target.
    let materialized: Vec<bool> = region
        .values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            uses[index] > 1 || matches!(value.op, Op::ReadX(_) | Op::ReadF(_) | Op::ReadFcsr)
        })
        .collect();
    let mut local_map = vec![None; region.values.len()];
    let mut local_types = Vec::new();
    for (index, value) in region.values.iter().enumerate() {
        if materialized[index] {
            // Local zero is the opaque state-pointer parameter.
            let local = 1 + local_types.len() as u32;
            local_map[index] = Some(local);
            local_types.push(val_type(value.ty));
        }
    }

    let mut function = Function::new_with_locals_types(local_types);
    let mut defined = vec![false; region.values.len()];
    for index in 0..region.values.len() {
        if let Some(local) = local_map[index] {
            emit_value_body(
                &mut function,
                region,
                layout,
                &local_map,
                &defined,
                ValueId(index),
            )?;
            function.instruction(&Instruction::LocalSet(local));
            defined[index] = true;
        }
    }

    // Commit only final dirty register values.
    for &(reg, value) in &region.outputs {
        function.instruction(&Instruction::I32Const(layout.x_base as i32));
        emit_value(&mut function, region, layout, &local_map, &defined, value)?;
        function.instruction(&Instruction::I64Store(memarg(3, u64::from(reg) * 8)));
    }
    emit_commit_f_outputs(
        &mut function,
        layout,
        &region.f_outputs,
        |function, value| emit_value(function, region, layout, &local_map, &defined, value),
    )?;
    if let Some(value) = region.fcsr_output {
        function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
        emit_value(&mut function, region, layout, &local_map, &defined, value)?;
        function.instruction(&Instruction::I32Store(memarg(2, 0)));
    }

    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    emit_value(
        &mut function,
        region,
        layout,
        &local_map,
        &defined,
        region.next_pc,
    )?;
    function.instruction(&Instruction::I64Store(memarg(3, 0)));

    // Retirement is cumulative across a host dispatch so future compiled edge
    // transfers can remain inside Wasm without losing accounting.
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::I64Const(i64::from(region.retired)));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    function.instruction(&Instruction::End);

    Ok(function)
}

/// Emit an ordered region with explicit guest-memory bounds checks.
///
/// A WebAssembly out-of-bounds trap is a host failure, not an RV64 exception,
/// so every access is checked against the flat guest-memory capability first.
/// Failure commits state as of immediately before the access and returns to
/// the architectural interpreter, which performs the instruction and reports
/// the exact guest exception when appropriate.
fn emit_effectful(
    region: &Region,
    layout: JitLayout,
    helpers: HelperImports,
) -> Result<Function, EmitError> {
    // Ordered effects make unrestricted stackification invalid across a load
    // or store. Locals also give side exits stable values to commit.
    let local_map: Vec<Option<u32>> = (0..region.values.len())
        .map(|index| Some(1 + index as u32))
        .collect();
    let mut local_types: Vec<ValType> = region
        .values
        .iter()
        .map(|value| val_type(value.ty))
        .collect();
    let memory_temps = allocate_memory_temps(&mut local_types, region, layout);
    let mut function = Function::new_with_locals_types(local_types);
    emit_memory_context_init(&mut function, layout, memory_temps);
    // Validation guarantees operands precede their users. Marking all locals
    // as addressable makes the shared pure-value emitter issue LocalGet rather
    // than recursively duplicating an earlier computation.
    let all_defined = vec![true; region.values.len()];
    let copy_plan = match (layout.sys, memory_temps.and_then(|temps| temps.copy)) {
        (Some(memory), Some(_)) => {
            dense_copy_plan(region, usize::from(memory.cache_min_accesses.max(1)))
        }
        _ => None,
    }
    .filter(|_| bulk_copy_loop_plan(region).is_none());
    let store_plan = match (layout.sys, memory_temps.and_then(|temps| temps.copy)) {
        (Some(memory), Some(_)) => {
            dense_store_plan(region, usize::from(memory.cache_min_accesses.max(1)))
        }
        _ => None,
    };
    for position in 0..=region.values.len() {
        for effect in &region.effects {
            match effect {
                Effect::Store {
                    position: store_position,
                    address,
                    value,
                    kind,
                    condition,
                    exit,
                } if *store_position == position => {
                    if let Some(condition) = condition {
                        function.instruction(&Instruction::LocalGet(1 + condition.0 as u32));
                        function.instruction(&Instruction::If(BlockType::Empty));
                    }
                    let copy_access = copy_plan
                        .as_ref()
                        .and_then(|plan| plan.store_access(*store_position, *address, *value));
                    if let (Some(access), Some(copy)) =
                        (copy_access, memory_temps.and_then(|temps| temps.copy))
                    {
                        if copy_plan
                            .as_ref()
                            .is_some_and(|plan| plan.setup_position == *store_position)
                        {
                            emit_dense_copy_setup(
                                &mut function,
                                copy_plan.as_ref().expect("copy plan"),
                                layout,
                                helpers,
                                memory_temps.expect("copy memory temporaries"),
                                &local_map,
                                |function| {
                                    emit_side_exit(
                                        function,
                                        region,
                                        layout,
                                        &local_map,
                                        &all_defined,
                                        exit,
                                    )
                                },
                            )?;
                        }
                        function.instruction(&Instruction::LocalGet(copy.destination_linear));
                        function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
                        function.instruction(&Instruction::I64Store(memarg(
                            3,
                            access.destination_offset,
                        )));
                    } else if let (Some(access), Some(range)) = (
                        store_plan
                            .as_ref()
                            .and_then(|plan| plan.store_access(*store_position, *address)),
                        memory_temps.and_then(|temps| temps.copy),
                    ) {
                        if store_plan
                            .as_ref()
                            .is_some_and(|plan| plan.setup_position == *store_position)
                        {
                            emit_dense_store_setup(
                                &mut function,
                                store_plan.as_ref().expect("store plan"),
                                layout,
                                helpers,
                                memory_temps.expect("store memory temporaries"),
                                &local_map,
                                |function| {
                                    emit_side_exit(
                                        function,
                                        region,
                                        layout,
                                        &local_map,
                                        &all_defined,
                                        exit,
                                    )
                                },
                            )?;
                        }
                        if let Some(fill_value) =
                            store_plan.as_ref().and_then(|plan| plan.fill_value)
                        {
                            if store_plan
                                .as_ref()
                                .is_some_and(|plan| plan.setup_position == *store_position)
                            {
                                emit_dense_fill(
                                    &mut function,
                                    store_plan.as_ref().expect("store plan"),
                                    range.destination_linear,
                                    1 + fill_value.0 as u32,
                                )?;
                            }
                        } else {
                            function.instruction(&Instruction::LocalGet(range.destination_linear));
                            function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
                            function.instruction(&Instruction::I64Store(memarg(
                                3,
                                access.destination_offset,
                            )));
                        }
                    } else {
                        emit_memory_address(
                            &mut function,
                            layout,
                            helpers,
                            memory_temps,
                            1 + address.0 as u32,
                            kind.bytes(),
                            true,
                            |function| {
                                emit_side_exit(
                                    function,
                                    region,
                                    layout,
                                    &local_map,
                                    &all_defined,
                                    exit,
                                )
                            },
                        )?;
                        function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
                        let instruction = match kind {
                            StoreKind::I8 => Instruction::I64Store8(memarg(0, 0)),
                            StoreKind::I16 => Instruction::I64Store16(memarg(0, 0)),
                            StoreKind::I32 => Instruction::I64Store32(memarg(0, 0)),
                            StoreKind::I64 => Instruction::I64Store(memarg(0, 0)),
                        };
                        function.instruction(&instruction);
                    }
                    if condition.is_some() {
                        function.instruction(&Instruction::End);
                        emit_reservation_clear(&mut function, helpers, 1 + address.0 as u32)?;
                    }
                }
                Effect::Guard {
                    position: guard_position,
                    condition,
                    exit,
                } if *guard_position == position => {
                    function.instruction(&Instruction::LocalGet(1 + condition.0 as u32));
                    function.instruction(&Instruction::If(BlockType::Empty));
                    emit_side_exit(
                        &mut function,
                        region,
                        layout,
                        &local_map,
                        &all_defined,
                        exit,
                    )?;
                    function.instruction(&Instruction::Return);
                    function.instruction(&Instruction::End);
                }
                Effect::GuardTarget {
                    position: guard_position,
                    target,
                    expected,
                    exit,
                } if *guard_position == position => {
                    function.instruction(&Instruction::LocalGet(1 + target.0 as u32));
                    emit_guest_pc(&mut function, *expected, layout);
                    function.instruction(&Instruction::I64Ne);
                    function.instruction(&Instruction::If(BlockType::Empty));
                    emit_side_exit(
                        &mut function,
                        region,
                        layout,
                        &local_map,
                        &all_defined,
                        exit,
                    )?;
                    emit_pc_from_local(&mut function, layout, 1 + target.0 as u32);
                    emit_ic_guard_miss(&mut function, region, layout, 1 + target.0 as u32);
                    function.instruction(&Instruction::Return);
                    function.instruction(&Instruction::End);
                }
                Effect::FpState {
                    position: fp_position,
                    dirty,
                    exit,
                } if *fp_position == position => {
                    emit_fp_state(&mut function, layout, *dirty, |function| {
                        emit_side_exit(function, region, layout, &local_map, &all_defined, exit)
                    })?;
                }
                _ => {}
            }
        }

        let Some(data) = region.values.get(position) else {
            continue;
        };
        match &data.op {
            Op::Load {
                address,
                kind,
                exit,
            } => {
                let copy_access = copy_plan
                    .as_ref()
                    .and_then(|plan| plan.load_access(ValueId(position)));
                if let (Some(access), Some(copy)) =
                    (copy_access, memory_temps.and_then(|temps| temps.copy))
                {
                    if copy_plan
                        .as_ref()
                        .is_some_and(|plan| plan.setup_position == position)
                    {
                        emit_dense_copy_setup(
                            &mut function,
                            copy_plan.as_ref().expect("copy plan"),
                            layout,
                            helpers,
                            memory_temps.expect("copy memory temporaries"),
                            &local_map,
                            |function| {
                                emit_side_exit(
                                    function,
                                    region,
                                    layout,
                                    &local_map,
                                    &all_defined,
                                    exit,
                                )
                            },
                        )?;
                    }
                    function.instruction(&Instruction::LocalGet(copy.source_linear));
                    function.instruction(&Instruction::I64Load(memarg(3, access.source_offset)));
                } else {
                    emit_memory_address(
                        &mut function,
                        layout,
                        helpers,
                        memory_temps,
                        1 + address.0 as u32,
                        kind.bytes(),
                        false,
                        |function| {
                            emit_side_exit(function, region, layout, &local_map, &all_defined, exit)
                        },
                    )?;
                    let instruction = match kind {
                        LoadKind::I8S => Instruction::I64Load8S(memarg(0, 0)),
                        LoadKind::I16S => Instruction::I64Load16S(memarg(0, 0)),
                        LoadKind::I32S => Instruction::I64Load32S(memarg(0, 0)),
                        LoadKind::I64 => Instruction::I64Load(memarg(0, 0)),
                        LoadKind::I8U => Instruction::I64Load8U(memarg(0, 0)),
                        LoadKind::I16U => Instruction::I64Load16U(memarg(0, 0)),
                        LoadKind::I32U => Instruction::I64Load32U(memarg(0, 0)),
                    };
                    function.instruction(&instruction);
                }
            }
            Op::ExactFp {
                op,
                lhs,
                rhs,
                third,
                rm,
                fcsr,
                exit,
            } => {
                // Dynamic frm values 5..7 are reserved. Returning to T0 lets
                // the interpreter raise the architectural illegal instruction
                // instead of invoking the helper with an invented mode.
                function.instruction(&Instruction::LocalGet(1 + rm.0 as u32));
                function.instruction(&Instruction::I32Const(4));
                function.instruction(&Instruction::I32GtU);
                function.instruction(&Instruction::If(BlockType::Empty));
                emit_side_exit(
                    &mut function,
                    region,
                    layout,
                    &local_map,
                    &all_defined,
                    exit,
                )?;
                function.instruction(&Instruction::Return);
                function.instruction(&Instruction::End);

                emit_exact_fp_value(
                    &mut function,
                    *op,
                    1 + lhs.0 as u32,
                    1 + rhs.0 as u32,
                    1 + third.0 as u32,
                    1 + rm.0 as u32,
                    1 + fcsr.0 as u32,
                    1 + position as u32,
                    layout,
                    helpers,
                )?;
            }
            Op::Reservation { op, address } => {
                function.instruction(&Instruction::I32Const(match op {
                    ReservationOp::LoadReserved => 0,
                    ReservationOp::StoreConditional => 1,
                }));
                function.instruction(&Instruction::LocalGet(0));
                function.instruction(&Instruction::LocalGet(1 + address.0 as u32));
                function.instruction(&Instruction::Call(
                    helpers
                        .reservation_index()
                        .ok_or_else(|| EmitError("missing reservation helper import".into()))?,
                ));
            }
            _ => emit_value_body(
                &mut function,
                region,
                layout,
                &local_map,
                &all_defined,
                ValueId(position),
            )?,
        }
        function.instruction(&Instruction::LocalSet(1 + position as u32));
    }

    emit_commit_outputs(&mut function, layout, &region.outputs, |function, value| {
        function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
        Ok(())
    })?;
    emit_commit_f_outputs(
        &mut function,
        layout,
        &region.f_outputs,
        |function, value| {
            function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
            Ok(())
        },
    )?;
    if let Some(value) = region.fcsr_output {
        function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
        function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
        function.instruction(&Instruction::I32Store(memarg(2, 0)));
    }
    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    function.instruction(&Instruction::LocalGet(1 + region.next_pc.0 as u32));
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    emit_retirement_const(&mut function, layout.retired_addr, region.retired);
    function.instruction(&Instruction::End);

    Ok(function)
}

/// Memory-aware single-latch loop lowering. Dirty architectural registers get
/// dedicated carry locals so a memory fault at the beginning of a later
/// iteration can still materialize the state produced by earlier iterations.
fn emit_effectful_loop(
    region: &Region,
    layout: JitLayout,
    loop_backedge: LoopBackedge,
    helpers: HelperImports,
) -> Result<Function, EmitError> {
    let local_map: Vec<Option<u32>> = (0..region.values.len())
        .map(|index| Some(1 + index as u32))
        .collect();
    let retired_local = 1 + region.values.len() as u32;
    let next_pc_local = retired_local + 1;
    let mut carry_map = [None; 32];
    let mut f_carry_map = [None; 32];
    let mut fcsr_carry = None;
    let mut local_types: Vec<ValType> = region.values.iter().map(|v| val_type(v.ty)).collect();
    local_types.extend([ValType::I64, ValType::I64]);
    for &(reg, _) in &region.outputs {
        carry_map[reg as usize] = Some(1 + local_types.len() as u32);
        local_types.push(ValType::I64);
    }
    for &(reg, _) in &region.f_outputs {
        f_carry_map[reg as usize] = Some(1 + local_types.len() as u32);
        local_types.push(ValType::I64);
    }
    if region.fcsr_output.is_some() {
        fcsr_carry = Some(1 + local_types.len() as u32);
        local_types.push(ValType::I32);
    }
    let memory_temps = allocate_memory_temps(&mut local_types, region, layout);
    let mut function = Function::new_with_locals_types(local_types);
    emit_memory_context_init(&mut function, layout, memory_temps);
    let all_defined = vec![true; region.values.len()];
    let copy_plan = match (layout.sys, memory_temps.and_then(|temps| temps.copy)) {
        (Some(memory), Some(_)) => {
            dense_copy_plan(region, usize::from(memory.cache_min_accesses.max(1)))
        }
        _ => None,
    }
    .filter(|_| bulk_copy_loop_plan(region).is_none());
    let store_plan = match (layout.sys, memory_temps.and_then(|temps| temps.copy)) {
        (Some(memory), Some(_)) => {
            dense_store_plan(region, usize::from(memory.cache_min_accesses.max(1)))
        }
        _ => None,
    };
    let bulk_plan = bulk_copy_loop_plan(region).filter(|plan| {
        loop_backedge.condition == Some(plan.condition) && loop_backedge.exit_pc == plan.exit_pc
    });
    let bulk_temps = bulk_plan.and_then(|_| memory_temps.and_then(|temps| temps.bulk_copy));

    // Capture the complete dirty-register state at loop entry. This includes
    // registers written but not read by the static body.
    for &(reg, _) in &region.outputs {
        function.instruction(&Instruction::I32Const(layout.x_base as i32));
        function.instruction(&Instruction::I64Load(memarg(3, u64::from(reg) * 8)));
        function.instruction(&Instruction::LocalSet(
            carry_map[reg as usize].expect("dirty register has a carry local"),
        ));
    }
    for &(reg, _) in &region.f_outputs {
        function.instruction(&Instruction::I32Const(layout.f_base as i32));
        function.instruction(&Instruction::I64Load(memarg(3, u64::from(reg) * 8)));
        function.instruction(&Instruction::LocalSet(
            f_carry_map[reg as usize].expect("dirty FP register has a carry local"),
        ));
    }
    if let Some(carry) = fcsr_carry {
        function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
        function.instruction(&Instruction::I32Load(memarg(2, 0)));
        function.instruction(&Instruction::LocalSet(carry));
    }
    for (index, value) in region.values.iter().enumerate() {
        match value.op {
            Op::ReadX(reg) => {
                if let Some(carry) = carry_map[reg as usize] {
                    function.instruction(&Instruction::LocalGet(carry));
                } else {
                    function.instruction(&Instruction::I32Const(layout.x_base as i32));
                    function.instruction(&Instruction::I64Load(memarg(3, u64::from(reg) * 8)));
                }
            }
            Op::ReadF(reg) => {
                if let Some(carry) = f_carry_map[reg as usize] {
                    function.instruction(&Instruction::LocalGet(carry));
                } else {
                    function.instruction(&Instruction::I32Const(layout.f_base as i32));
                    function.instruction(&Instruction::I64Load(memarg(3, u64::from(reg) * 8)));
                }
            }
            Op::ReadFcsr => {
                if let Some(carry) = fcsr_carry {
                    function.instruction(&Instruction::LocalGet(carry));
                } else {
                    function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
                    function.instruction(&Instruction::I32Load(memarg(2, 0)));
                }
            }
            _ => continue,
        }
        function.instruction(&Instruction::LocalSet(1 + index as u32));
    }
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::LocalSet(retired_local));

    if let (Some(plan), Some(temps)) = (bulk_plan, bulk_temps) {
        let source = read_x_value(region, plan.source_reg)
            .ok_or_else(|| EmitError("bulk-copy source has no ReadX value".into()))?;
        let destination = read_x_value(region, plan.destination_reg)
            .ok_or_else(|| EmitError("bulk-copy destination has no ReadX value".into()))?;
        let count = read_x_value(region, plan.count_reg)
            .ok_or_else(|| EmitError("bulk-copy count has no ReadX value".into()))?;
        let limit = read_x_value(region, plan.limit_reg)
            .ok_or_else(|| EmitError("bulk-copy limit has no ReadX value".into()))?;
        emit_bulk_copy_call(
            &mut function,
            plan,
            region.retired,
            helpers,
            temps,
            1 + source.0 as u32,
            1 + destination.0 as u32,
            1 + count.0 as u32,
            1 + limit.0 as u32,
            retired_local,
            None,
            layout.fuel_addr,
        )?;
        function.instruction(&Instruction::LocalGet(temps.result));
        function.instruction(&Instruction::I64Const(0));
        function.instruction(&Instruction::I64Ne);
        function.instruction(&Instruction::If(BlockType::Empty));
        let subtract_pointers = plan.step < 0;
        emit_add_or_sub_local(
            &mut function,
            carry_map[plan.source_reg as usize].expect("bulk-copy source carry"),
            temps.result,
            subtract_pointers,
        );
        emit_add_or_sub_local(
            &mut function,
            carry_map[plan.destination_reg as usize].expect("bulk-copy destination carry"),
            temps.result,
            subtract_pointers,
        );
        emit_add_or_sub_local(
            &mut function,
            carry_map[plan.count_reg as usize].expect("bulk-copy count carry"),
            temps.result,
            true,
        );
        function.instruction(&Instruction::I32Const(layout.x_base as i32));
        function.instruction(&Instruction::I64Load(memarg(
            3,
            u64::from(plan.value_reg) * 8,
        )));
        function.instruction(&Instruction::LocalSet(
            carry_map[plan.value_reg as usize].expect("bulk-copy value carry"),
        ));
        function.instruction(&Instruction::I64Const(plan.limit_value as i64));
        function.instruction(&Instruction::LocalGet(
            carry_map[plan.count_reg as usize].expect("bulk-copy count carry"),
        ));
        function.instruction(&Instruction::I64LtU);
        function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        emit_guest_pc(&mut function, region.entry_pc, layout);
        function.instruction(&Instruction::Else);
        emit_guest_pc(&mut function, plan.exit_pc, layout);
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::LocalSet(next_pc_local));
        emit_bulk_retired(
            &mut function,
            retired_local,
            temps.result,
            plan.bytes_per_iteration,
            region.retired,
        );
        for &(reg, _) in &region.outputs {
            function.instruction(&Instruction::I32Const(layout.x_base as i32));
            function.instruction(&Instruction::LocalGet(
                carry_map[reg as usize].expect("bulk-copy output carry"),
            ));
            function.instruction(&Instruction::I64Store(memarg(3, u64::from(reg) * 8)));
        }
        function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
        function.instruction(&Instruction::LocalGet(next_pc_local));
        function.instruction(&Instruction::I64Store(memarg(3, 0)));
        emit_retirement_local(&mut function, layout.retired_addr, retired_local, 0);
        function.instruction(&Instruction::Return);
        function.instruction(&Instruction::End);
    }

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    for position in 0..=region.values.len() {
        for effect in &region.effects {
            match effect {
                Effect::Store {
                    position: store_position,
                    address,
                    value,
                    kind,
                    condition,
                    exit,
                } if *store_position == position => {
                    if let Some(condition) = condition {
                        function.instruction(&Instruction::LocalGet(1 + condition.0 as u32));
                        function.instruction(&Instruction::If(BlockType::Empty));
                    }
                    let copy_access = copy_plan
                        .as_ref()
                        .and_then(|plan| plan.store_access(*store_position, *address, *value));
                    if let (Some(access), Some(copy)) =
                        (copy_access, memory_temps.and_then(|temps| temps.copy))
                    {
                        if copy_plan
                            .as_ref()
                            .is_some_and(|plan| plan.setup_position == *store_position)
                        {
                            emit_dense_copy_setup(
                                &mut function,
                                copy_plan.as_ref().expect("copy plan"),
                                layout,
                                helpers,
                                memory_temps.expect("copy memory temporaries"),
                                &local_map,
                                |function| {
                                    emit_loop_side_exit(
                                        function,
                                        region,
                                        layout,
                                        exit,
                                        retired_local,
                                        &carry_map,
                                        &f_carry_map,
                                        fcsr_carry,
                                    )
                                },
                            )?;
                        }
                        function.instruction(&Instruction::LocalGet(copy.destination_linear));
                        function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
                        function.instruction(&Instruction::I64Store(memarg(
                            3,
                            access.destination_offset,
                        )));
                    } else if let (Some(access), Some(range)) = (
                        store_plan
                            .as_ref()
                            .and_then(|plan| plan.store_access(*store_position, *address)),
                        memory_temps.and_then(|temps| temps.copy),
                    ) {
                        if store_plan
                            .as_ref()
                            .is_some_and(|plan| plan.setup_position == *store_position)
                        {
                            emit_dense_store_setup(
                                &mut function,
                                store_plan.as_ref().expect("store plan"),
                                layout,
                                helpers,
                                memory_temps.expect("store memory temporaries"),
                                &local_map,
                                |function| {
                                    emit_loop_side_exit(
                                        function,
                                        region,
                                        layout,
                                        exit,
                                        retired_local,
                                        &carry_map,
                                        &f_carry_map,
                                        fcsr_carry,
                                    )
                                },
                            )?;
                        }
                        if let Some(fill_value) =
                            store_plan.as_ref().and_then(|plan| plan.fill_value)
                        {
                            if store_plan
                                .as_ref()
                                .is_some_and(|plan| plan.setup_position == *store_position)
                            {
                                emit_dense_fill(
                                    &mut function,
                                    store_plan.as_ref().expect("store plan"),
                                    range.destination_linear,
                                    1 + fill_value.0 as u32,
                                )?;
                            }
                        } else {
                            function.instruction(&Instruction::LocalGet(range.destination_linear));
                            function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
                            function.instruction(&Instruction::I64Store(memarg(
                                3,
                                access.destination_offset,
                            )));
                        }
                    } else {
                        emit_memory_address(
                            &mut function,
                            layout,
                            helpers,
                            memory_temps,
                            1 + address.0 as u32,
                            kind.bytes(),
                            true,
                            |function| {
                                emit_loop_side_exit(
                                    function,
                                    region,
                                    layout,
                                    exit,
                                    retired_local,
                                    &carry_map,
                                    &f_carry_map,
                                    fcsr_carry,
                                )
                            },
                        )?;
                        function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
                        let instruction = match kind {
                            StoreKind::I8 => Instruction::I64Store8(memarg(0, 0)),
                            StoreKind::I16 => Instruction::I64Store16(memarg(0, 0)),
                            StoreKind::I32 => Instruction::I64Store32(memarg(0, 0)),
                            StoreKind::I64 => Instruction::I64Store(memarg(0, 0)),
                        };
                        function.instruction(&instruction);
                    }
                    if condition.is_some() {
                        function.instruction(&Instruction::End);
                        emit_reservation_clear(&mut function, helpers, 1 + address.0 as u32)?;
                    }
                }
                Effect::Guard {
                    position: guard_position,
                    condition,
                    exit,
                } if *guard_position == position => {
                    function.instruction(&Instruction::LocalGet(1 + condition.0 as u32));
                    function.instruction(&Instruction::If(BlockType::Empty));
                    emit_loop_side_exit(
                        &mut function,
                        region,
                        layout,
                        exit,
                        retired_local,
                        &carry_map,
                        &f_carry_map,
                        fcsr_carry,
                    )?;
                    function.instruction(&Instruction::Return);
                    function.instruction(&Instruction::End);
                }
                Effect::GuardTarget {
                    position: guard_position,
                    target,
                    expected,
                    exit,
                } if *guard_position == position => {
                    function.instruction(&Instruction::LocalGet(1 + target.0 as u32));
                    emit_guest_pc(&mut function, *expected, layout);
                    function.instruction(&Instruction::I64Ne);
                    function.instruction(&Instruction::If(BlockType::Empty));
                    emit_loop_side_exit(
                        &mut function,
                        region,
                        layout,
                        exit,
                        retired_local,
                        &carry_map,
                        &f_carry_map,
                        fcsr_carry,
                    )?;
                    emit_pc_from_local(&mut function, layout, 1 + target.0 as u32);
                    emit_ic_guard_miss(&mut function, region, layout, 1 + target.0 as u32);
                    function.instruction(&Instruction::Return);
                    function.instruction(&Instruction::End);
                }
                Effect::FpState {
                    position: fp_position,
                    dirty,
                    exit,
                } if *fp_position == position => {
                    emit_fp_state(&mut function, layout, *dirty, |function| {
                        emit_loop_side_exit(
                            function,
                            region,
                            layout,
                            exit,
                            retired_local,
                            &carry_map,
                            &f_carry_map,
                            fcsr_carry,
                        )
                    })?;
                }
                _ => {}
            }
        }

        let Some(data) = region.values.get(position) else {
            continue;
        };
        if matches!(data.op, Op::ReadX(_) | Op::ReadF(_) | Op::ReadFcsr) {
            continue;
        }
        match &data.op {
            Op::Load {
                address,
                kind,
                exit,
            } => {
                let copy_access = copy_plan
                    .as_ref()
                    .and_then(|plan| plan.load_access(ValueId(position)));
                if let (Some(access), Some(copy)) =
                    (copy_access, memory_temps.and_then(|temps| temps.copy))
                {
                    if copy_plan
                        .as_ref()
                        .is_some_and(|plan| plan.setup_position == position)
                    {
                        emit_dense_copy_setup(
                            &mut function,
                            copy_plan.as_ref().expect("copy plan"),
                            layout,
                            helpers,
                            memory_temps.expect("copy memory temporaries"),
                            &local_map,
                            |function| {
                                emit_loop_side_exit(
                                    function,
                                    region,
                                    layout,
                                    exit,
                                    retired_local,
                                    &carry_map,
                                    &f_carry_map,
                                    fcsr_carry,
                                )
                            },
                        )?;
                    }
                    function.instruction(&Instruction::LocalGet(copy.source_linear));
                    function.instruction(&Instruction::I64Load(memarg(3, access.source_offset)));
                } else {
                    emit_memory_address(
                        &mut function,
                        layout,
                        helpers,
                        memory_temps,
                        1 + address.0 as u32,
                        kind.bytes(),
                        false,
                        |function| {
                            emit_loop_side_exit(
                                function,
                                region,
                                layout,
                                exit,
                                retired_local,
                                &carry_map,
                                &f_carry_map,
                                fcsr_carry,
                            )
                        },
                    )?;
                    let instruction = match kind {
                        LoadKind::I8S => Instruction::I64Load8S(memarg(0, 0)),
                        LoadKind::I16S => Instruction::I64Load16S(memarg(0, 0)),
                        LoadKind::I32S => Instruction::I64Load32S(memarg(0, 0)),
                        LoadKind::I64 => Instruction::I64Load(memarg(0, 0)),
                        LoadKind::I8U => Instruction::I64Load8U(memarg(0, 0)),
                        LoadKind::I16U => Instruction::I64Load16U(memarg(0, 0)),
                        LoadKind::I32U => Instruction::I64Load32U(memarg(0, 0)),
                    };
                    function.instruction(&instruction);
                }
            }
            Op::ExactFp {
                op,
                lhs,
                rhs,
                third,
                rm,
                fcsr,
                exit,
            } => {
                function.instruction(&Instruction::LocalGet(1 + rm.0 as u32));
                function.instruction(&Instruction::I32Const(4));
                function.instruction(&Instruction::I32GtU);
                function.instruction(&Instruction::If(BlockType::Empty));
                emit_loop_side_exit(
                    &mut function,
                    region,
                    layout,
                    exit,
                    retired_local,
                    &carry_map,
                    &f_carry_map,
                    fcsr_carry,
                )?;
                function.instruction(&Instruction::Return);
                function.instruction(&Instruction::End);
                emit_exact_fp_value(
                    &mut function,
                    *op,
                    1 + lhs.0 as u32,
                    1 + rhs.0 as u32,
                    1 + third.0 as u32,
                    1 + rm.0 as u32,
                    1 + fcsr.0 as u32,
                    1 + position as u32,
                    layout,
                    helpers,
                )?;
            }
            Op::Reservation { op, address } => {
                function.instruction(&Instruction::I32Const(match op {
                    ReservationOp::LoadReserved => 0,
                    ReservationOp::StoreConditional => 1,
                }));
                function.instruction(&Instruction::LocalGet(0));
                function.instruction(&Instruction::LocalGet(1 + address.0 as u32));
                function.instruction(&Instruction::Call(
                    helpers
                        .reservation_index()
                        .ok_or_else(|| EmitError("missing reservation helper import".into()))?,
                ));
            }
            _ => emit_value_body(
                &mut function,
                region,
                layout,
                &local_map,
                &all_defined,
                ValueId(position),
            )?,
        }
        function.instruction(&Instruction::LocalSet(1 + position as u32));
    }

    function.instruction(&Instruction::LocalGet(retired_local));
    function.instruction(&Instruction::I64Const(i64::from(region.retired)));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(retired_local));
    if let Some(condition) = loop_backedge.condition {
        function.instruction(&Instruction::LocalGet(1 + condition.0 as u32));
    } else {
        function.instruction(&Instruction::I32Const(1));
    }
    if layout.fuel_addr != 0 {
        function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::LocalGet(retired_local));
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::I32Const(layout.fuel_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::I64LtU);
        function.instruction(&Instruction::I32And);
    } else {
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::If(BlockType::Empty));

    // Parallel-copy-safe backedge: all output values remain in their own SSA
    // locals while carry and ReadX locals are updated.
    for &(reg, output) in &region.outputs {
        function.instruction(&Instruction::LocalGet(1 + output.0 as u32));
        function.instruction(&Instruction::LocalSet(
            carry_map[reg as usize].expect("dirty register has a carry local"),
        ));
    }
    for &(reg, output) in &region.f_outputs {
        function.instruction(&Instruction::LocalGet(1 + output.0 as u32));
        function.instruction(&Instruction::LocalSet(
            f_carry_map[reg as usize].expect("dirty FP register has a carry local"),
        ));
    }
    if let (Some(output), Some(carry)) = (region.fcsr_output, fcsr_carry) {
        function.instruction(&Instruction::LocalGet(1 + output.0 as u32));
        function.instruction(&Instruction::LocalSet(carry));
    }
    for (index, value) in region.values.iter().enumerate() {
        let output = match value.op {
            Op::ReadX(reg) => region.outputs.iter().find(|&&(r, _)| r == reg).copied(),
            Op::ReadF(reg) => region.f_outputs.iter().find(|&&(r, _)| r == reg).copied(),
            Op::ReadFcsr => region.fcsr_output.map(|output| (0, output)),
            _ => None,
        };
        if let Some((_, output)) = output {
            function.instruction(&Instruction::LocalGet(1 + output.0 as u32));
            function.instruction(&Instruction::LocalSet(1 + index as u32));
        }
    }
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(1 + region.next_pc.0 as u32));
    function.instruction(&Instruction::LocalSet(next_pc_local));
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    emit_commit_outputs(&mut function, layout, &region.outputs, |function, value| {
        function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
        Ok(())
    })?;
    emit_commit_f_outputs(
        &mut function,
        layout,
        &region.f_outputs,
        |function, value| {
            function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
            Ok(())
        },
    )?;
    if let Some(value) = region.fcsr_output {
        function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
        function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
        function.instruction(&Instruction::I32Store(memarg(2, 0)));
    }
    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    function.instruction(&Instruction::LocalGet(next_pc_local));
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    emit_retirement_local(&mut function, layout.retired_addr, retired_local, 0);
    function.instruction(&Instruction::End);
    Ok(function)
}

#[allow(clippy::too_many_arguments)]
fn emit_loop_side_exit(
    function: &mut Function,
    region: &Region,
    layout: JitLayout,
    exit: &SideExit,
    retired_local: u32,
    carry_map: &[Option<u32>; 32],
    f_carry_map: &[Option<u32>; 32],
    fcsr_carry: Option<u32>,
) -> Result<(), EmitError> {
    for &(reg, final_output) in &region.outputs {
        function.instruction(&Instruction::I32Const(layout.x_base as i32));
        if let Some(&(_, current)) = exit.outputs.iter().find(|&&(r, _)| r == reg) {
            function.instruction(&Instruction::LocalGet(1 + current.0 as u32));
        } else {
            let carry = carry_map[reg as usize].ok_or_else(|| {
                EmitError(format!(
                    "missing loop carry for side-exit output {final_output:?}"
                ))
            })?;
            function.instruction(&Instruction::LocalGet(carry));
        }
        function.instruction(&Instruction::I64Store(memarg(3, u64::from(reg) * 8)));
    }
    for &(reg, final_output) in &region.f_outputs {
        function.instruction(&Instruction::I32Const(layout.f_base as i32));
        if let Some(&(_, current)) = exit.f_outputs.iter().find(|&&(r, _)| r == reg) {
            function.instruction(&Instruction::LocalGet(1 + current.0 as u32));
        } else {
            let carry = f_carry_map[reg as usize].ok_or_else(|| {
                EmitError(format!(
                    "missing FP loop carry for side-exit output {final_output:?}"
                ))
            })?;
            function.instruction(&Instruction::LocalGet(carry));
        }
        function.instruction(&Instruction::I64Store(memarg(3, u64::from(reg) * 8)));
    }
    if region.fcsr_output.is_some() {
        function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
        if let Some(current) = exit.fcsr_output {
            function.instruction(&Instruction::LocalGet(1 + current.0 as u32));
        } else {
            function
                .instruction(&Instruction::LocalGet(fcsr_carry.ok_or_else(|| {
                    EmitError("missing fcsr loop carry for side exit".into())
                })?));
        }
        function.instruction(&Instruction::I32Store(memarg(2, 0)));
    }
    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    emit_guest_pc(function, exit.guest_pc, layout);
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    emit_retirement_local(function, layout.retired_addr, retired_local, exit.retired);
    Ok(())
}

fn emit_linear_address(function: &mut Function, memory_base: u32, address_local: u32) {
    function.instruction(&Instruction::I32Const(memory_base as i32));
    function.instruction(&Instruction::LocalGet(address_local));
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Add);
}

fn validate_system_memory(memory: SystemMemory) -> Result<(), EmitError> {
    if !(3..=31).contains(&memory.page_shift) {
        return Err(EmitError(format!(
            "system-memory page shift {} is outside the supported memory32 range",
            memory.page_shift
        )));
    }
    let slots = memory
        .index_mask
        .checked_add(1)
        .ok_or_else(|| EmitError("system-memory translation mask cannot be u32::MAX".into()))?;
    if !slots.is_power_of_two() {
        return Err(EmitError(
            "system-memory translation mask must be power-of-two minus one".into(),
        ));
    }
    for (name, row) in [("load", memory.load), ("store", memory.store)] {
        if row.tags & 7 != 0 || row.offsets & 7 != 0 {
            return Err(EmitError(format!(
                "system-memory {name} row bases must be eight-byte aligned"
            )));
        }
    }
    if memory.context_addr == 0 || memory.context_addr & 7 != 0 {
        return Err(EmitError(
            "system-memory context address must be non-zero and eight-byte aligned".into(),
        ));
    }
    Ok(())
}

fn emit_translation_probe(
    function: &mut Function,
    row: TranslationRow,
    temps: MemoryTemps,
    address_local: u32,
    page_shift: u8,
) {
    function.instruction(&Instruction::I32Const(row.tags as i32));
    function.instruction(&Instruction::LocalGet(temps.index));
    function.instruction(&Instruction::I32Const(3));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalGet(address_local));
    let page_mask = !((1u64 << page_shift) - 1);
    function.instruction(&Instruction::I64Const(page_mask as i64));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::LocalGet(temps.context));
    function.instruction(&Instruction::I64Or);
    function.instruction(&Instruction::I64Eq);
}

fn emit_translation_offset(function: &mut Function, row: TranslationRow, temps: MemoryTemps) {
    function.instruction(&Instruction::I32Const(row.offsets as i32));
    function.instruction(&Instruction::LocalGet(temps.index));
    function.instruction(&Instruction::I32Const(3));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
}

fn emit_fp_state(
    function: &mut Function,
    layout: JitLayout,
    dirty: bool,
    mut side_exit: impl FnMut(&mut Function) -> Result<(), EmitError>,
) -> Result<(), EmitError> {
    if layout.mstatus_addr == 0 {
        return Err(EmitError(
            "full-system FP effect requires an mstatus capability".into(),
        ));
    }
    const FS_MASK: i64 = 3 << 13;
    function.instruction(&Instruction::I32Const(layout.mstatus_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::I64Const(FS_MASK));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    side_exit(function)?;
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);

    if dirty {
        function.instruction(&Instruction::I32Const(layout.mstatus_addr as i32));
        function.instruction(&Instruction::I32Const(layout.mstatus_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::I64Const(FS_MASK));
        function.instruction(&Instruction::I64Or);
        function.instruction(&Instruction::I64Store(memarg(3, 0)));
    }
    Ok(())
}

/// Leave whether the raw f64 bits in `local` encode a finite value.  The
/// generated native-arithmetic path deliberately mirrors rv64-wasm's proven
/// `fast64` predicate: NaNs and infinities always use the exact soft-float
/// helper, so Wasm NaN canonicalisation can never become architectural state.
fn emit_f64_finite(function: &mut Function, local: u32) {
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I64Const(52));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I64Const(0x7ff));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(0x7ff));
    function.instruction(&Instruction::I64Ne);
}

fn emit_f64_zero(function: &mut Function, local: u32) {
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64Shl);
    function.instruction(&Instruction::I64Eqz);
}

fn emit_f64_normal(function: &mut Function, local: u32) {
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I64Const(52));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I64Const(0x7ff));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::I64GtU);
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I64Const(52));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I64Const(0x7ff));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(0x7ff));
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::I32And);
}

fn emit_exact_fp_helper_call(
    function: &mut Function,
    op: ExactFpOp,
    lhs_local: u32,
    rhs_local: u32,
    third_local: u32,
    rm_local: u32,
    fcsr_addr: u32,
    helpers: HelperImports,
) -> Result<(), EmitError> {
    function.instruction(&Instruction::I32Const(op.helper_code()));
    for local in [lhs_local, rhs_local, third_local] {
        function.instruction(&Instruction::LocalGet(local));
    }
    function.instruction(&Instruction::LocalGet(rm_local));
    function.instruction(&Instruction::I32Const(fcsr_addr as i32));
    function.instruction(&Instruction::Call(
        helpers
            .fp_index()
            .ok_or_else(|| EmitError("missing exact-FP helper import".into()))?,
    ));
    Ok(())
}

/// Emit one exact FP result, inlining the subset that is already proven safe
/// by the runtime's randomized soft-float differential:
///
/// * round-to-nearest/even;
/// * NX is already sticky, so losing another NX event is unobservable;
/// * finite operands; and
/// * a result for which no flag other than NX can arise.
///
/// Everything else takes the existing Wasm-to-Wasm exact helper.  Keeping the
/// predicate in generated code removes a cross-instance call from ordinary
/// libm arithmetic while retaining bit/flag exactness at every boundary.
fn emit_exact_fp_value(
    function: &mut Function,
    op: ExactFpOp,
    lhs_local: u32,
    rhs_local: u32,
    third_local: u32,
    rm_local: u32,
    fcsr_local: u32,
    output_local: u32,
    layout: JitLayout,
    helpers: HelperImports,
) -> Result<(), EmitError> {
    // ReloadFcsr follows every ExactFp node.  Publish forwarded state even on
    // the inline arm so that load observes exactly the same value it did when
    // every operation unconditionally called the helper.
    function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
    function.instruction(&Instruction::LocalGet(fcsr_local));
    function.instruction(&Instruction::I32Store(memarg(2, 0)));

    if matches!(op, ExactFpOp::Eq64 | ExactFpOp::Lt64 | ExactFpOp::Le64) {
        // Ordered comparisons of finite operands are exact and cannot accrue
        // flags. NaNs retain the helper's quiet/signalling distinction.
        emit_f64_finite(function, lhs_local);
        emit_f64_finite(function, rhs_local);
        function.instruction(&Instruction::I32And);
        function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        function.instruction(&Instruction::LocalGet(lhs_local));
        function.instruction(&Instruction::F64ReinterpretI64);
        function.instruction(&Instruction::LocalGet(rhs_local));
        function.instruction(&Instruction::F64ReinterpretI64);
        function.instruction(&match op {
            ExactFpOp::Eq64 => Instruction::F64Eq,
            ExactFpOp::Lt64 => Instruction::F64Lt,
            ExactFpOp::Le64 => Instruction::F64Le,
            _ => unreachable!(),
        });
        function.instruction(&Instruction::I64ExtendI32U);
        function.instruction(&Instruction::Else);
        emit_exact_fp_helper_call(
            function,
            op,
            lhs_local,
            rhs_local,
            third_local,
            rm_local,
            layout.fcsr_addr,
            helpers,
        )?;
        function.instruction(&Instruction::End);
        return Ok(());
    }

    let inline_fma = op == ExactFpOp::Fma64 && crate::hardware_fma_enabled();
    if !matches!(
        op,
        ExactFpOp::Add64 | ExactFpOp::Sub64 | ExactFpOp::Mul64 | ExactFpOp::Div64
    ) && !inline_fma
    {
        return emit_exact_fp_helper_call(
            function,
            op,
            lhs_local,
            rhs_local,
            third_local,
            rm_local,
            layout.fcsr_addr,
            helpers,
        );
    }

    // rm == RNE && (fcsr & NX) != 0 && finite arithmetic operands.
    function.instruction(&Instruction::LocalGet(rm_local));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::LocalGet(fcsr_local));
    function.instruction(&Instruction::I32Const(1)); // FFLAG_INEXACT
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::I32And);
    emit_f64_finite(function, lhs_local);
    function.instruction(&Instruction::I32And);
    emit_f64_finite(function, rhs_local);
    function.instruction(&Instruction::I32And);
    if inline_fma {
        emit_f64_finite(function, third_local);
        function.instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));

    if inline_fma {
        for local in [lhs_local, rhs_local, third_local] {
            function.instruction(&Instruction::LocalGet(local));
            function.instruction(&Instruction::F64ReinterpretI64);
            function.instruction(&Instruction::F64x2Splat);
        }
        function.instruction(&Instruction::F64x2RelaxedMadd);
        function.instruction(&Instruction::F64x2ExtractLane(0));
    } else {
        function.instruction(&Instruction::LocalGet(lhs_local));
        function.instruction(&Instruction::F64ReinterpretI64);
        function.instruction(&Instruction::LocalGet(rhs_local));
        function.instruction(&Instruction::F64ReinterpretI64);
        function.instruction(&match op {
            ExactFpOp::Add64 => Instruction::F64Add,
            ExactFpOp::Sub64 => Instruction::F64Sub,
            ExactFpOp::Mul64 => Instruction::F64Mul,
            ExactFpOp::Div64 => Instruction::F64Div,
            _ => unreachable!(),
        });
    }
    function.instruction(&Instruction::I64ReinterpretF64);
    function.instruction(&Instruction::LocalSet(output_local));

    match op {
        // Add/sub cannot produce an inexact subnormal result.  Any finite
        // result therefore has no newly observable flag beyond sticky NX.
        ExactFpOp::Add64 | ExactFpOp::Sub64 => {
            function.instruction(&Instruction::LocalGet(output_local));
            function.instruction(&Instruction::I64Const(52));
            function.instruction(&Instruction::I64ShrU);
            function.instruction(&Instruction::I64Const(0x7ff));
            function.instruction(&Instruction::I64And);
            function.instruction(&Instruction::I64Const(0x7ff));
            function.instruction(&Instruction::I64Ne);
        }
        // A normal result is safe.  Exact zero is additionally safe when
        // multiplication was forced by a zero operand, or division had a
        // zero numerator; other zero/subnormal results may carry UF.
        ExactFpOp::Mul64 | ExactFpOp::Div64 => {
            emit_f64_normal(function, output_local);
            emit_f64_zero(function, output_local);
            emit_f64_zero(function, lhs_local);
            if op == ExactFpOp::Mul64 {
                emit_f64_zero(function, rhs_local);
                function.instruction(&Instruction::I32Or);
            }
            function.instruction(&Instruction::I32And);
            function.instruction(&Instruction::I32Or);
        }
        // For finite fused inputs, a normal result cannot accrue any flag
        // beyond NX. Zero/subnormal/overflow results conservatively fall back.
        ExactFpOp::Fma64 => emit_f64_normal(function, output_local),
        _ => unreachable!(),
    }
    function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
    function.instruction(&Instruction::LocalGet(output_local));
    function.instruction(&Instruction::Else);
    emit_exact_fp_helper_call(
        function,
        op,
        lhs_local,
        rhs_local,
        third_local,
        rm_local,
        layout.fcsr_addr,
        helpers,
    )?;
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::Else);
    emit_exact_fp_helper_call(
        function,
        op,
        lhs_local,
        rhs_local,
        third_local,
        rm_local,
        layout.fcsr_addr,
        helpers,
    )?;
    function.instruction(&Instruction::End);
    Ok(())
}

/// Leave one validated memory32 linear address on the Wasm stack. Flat-user
/// memory uses an unsigned length check. Full-system memory requires a
/// same-page fused-row hit; a typed refill may publish the row on a miss, but
/// the generated code always re-probes it and never trusts a helper sentinel.
fn emit_memory_address(
    function: &mut Function,
    layout: JitLayout,
    helpers: HelperImports,
    temps: Option<MemoryTemps>,
    address_local: u32,
    access_bytes: u64,
    store: bool,
    mut side_exit: impl FnMut(&mut Function) -> Result<(), EmitError>,
) -> Result<(), EmitError> {
    if let Some((memory_base, memory_len)) = layout.mem {
        if let Some(last_valid) = memory_len.checked_sub(access_bytes) {
            function.instruction(&Instruction::LocalGet(address_local));
            function.instruction(&Instruction::I64Const(last_valid as i64));
            function.instruction(&Instruction::I64GtU);
        } else {
            function.instruction(&Instruction::I32Const(1));
        }
        function.instruction(&Instruction::If(BlockType::Empty));
        side_exit(function)?;
        function.instruction(&Instruction::Return);
        function.instruction(&Instruction::End);
        emit_linear_address(function, memory_base, address_local);
        return Ok(());
    }

    let memory = layout.sys.ok_or_else(|| {
        EmitError("memory operation requires a flat or full-system capability".into())
    })?;
    validate_system_memory(memory)?;
    let temps = temps.ok_or_else(|| EmitError("missing full-system memory temporaries".into()))?;
    let page_bytes = 1u64 << memory.page_shift;
    let last_same_page = page_bytes.checked_sub(access_bytes).ok_or_else(|| {
        EmitError("memory access is wider than the configured system page".into())
    })?;

    // Cross-page accesses require two independently faultable translations.
    // T0 already implements those bytewise semantics and remains the precise
    // slow path until a typed split-access IR effect is introduced.
    function.instruction(&Instruction::LocalGet(address_local));
    function.instruction(&Instruction::I64Const((page_bytes - 1) as i64));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(last_same_page as i64));
    function.instruction(&Instruction::I64GtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    side_exit(function)?;
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);

    let row = if store { memory.store } else { memory.load };
    let cache = if store {
        temps.store_cache
    } else {
        temps.load_cache
    };
    if let (Some(cache), Some(page_local)) = (cache, temps.page) {
        function.instruction(&Instruction::LocalGet(address_local));
        function.instruction(&Instruction::I64Const(i64::from(memory.page_shift)));
        function.instruction(&Instruction::I64ShrU);
        function.instruction(&Instruction::LocalSet(page_local));

        function.instruction(&Instruction::LocalGet(page_local));
        function.instruction(&Instruction::LocalGet(cache.page));
        function.instruction(&Instruction::I64Eq);
        function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        function.instruction(&Instruction::LocalGet(cache.offset));
        function.instruction(&Instruction::Else);
        emit_system_translation_offset(
            function,
            memory,
            row,
            temps,
            helpers,
            address_local,
            store,
            &mut side_exit,
        )?;
        function.instruction(&Instruction::LocalSet(temps.offset));
        function.instruction(&Instruction::LocalGet(page_local));
        function.instruction(&Instruction::LocalSet(cache.page));
        function.instruction(&Instruction::LocalGet(temps.offset));
        function.instruction(&Instruction::LocalSet(cache.offset));
        function.instruction(&Instruction::LocalGet(temps.offset));
        function.instruction(&Instruction::End);
    } else {
        // `temps.page` is a scratch local initialized only by the cached path
        // above. Selective caching can leave (for example) a store cache live
        // while a singleton load deliberately bypasses caching; that load
        // must derive its page from its own address rather than consume the
        // store path's stale scratch value.
        let mut uncached_temps = temps;
        uncached_temps.page = None;
        emit_system_translation_offset(
            function,
            memory,
            row,
            uncached_temps,
            helpers,
            address_local,
            store,
            &mut side_exit,
        )?;
    }
    function.instruction(&Instruction::LocalSet(temps.offset));

    function.instruction(&Instruction::LocalGet(address_local));
    function.instruction(&Instruction::LocalGet(temps.offset));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I32WrapI64);
    Ok(())
}

/// Leave one proven signed linear-memory offset on the stack. A refill helper
/// may mutate the direct-mapped architectural row, so the row is always
/// re-probed before its offset is consumed. Invocation-local translation
/// caching happens outside this primitive only after that proof succeeds.
#[allow(clippy::too_many_arguments)]
fn emit_system_translation_offset(
    function: &mut Function,
    memory: SystemMemory,
    row: TranslationRow,
    temps: MemoryTemps,
    helpers: HelperImports,
    address_local: u32,
    store: bool,
    side_exit: &mut impl FnMut(&mut Function) -> Result<(), EmitError>,
) -> Result<(), EmitError> {
    emit_translation_index(function, memory, temps, address_local);

    emit_translation_probe(function, row, temps, address_local, memory.page_shift);
    function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
    emit_translation_offset(function, row, temps);
    function.instruction(&Instruction::Else);
    if memory.miss == TlbMissPolicy::Refill {
        function.instruction(&Instruction::LocalGet(address_local));
        function.instruction(&Instruction::I32Const(i32::from(store)));
        function.instruction(&Instruction::Call(
            helpers
                .tlb_fill_index()
                .ok_or_else(|| EmitError("missing full-system TLB refill import".into()))?,
        ));
        function.instruction(&Instruction::Drop);

        emit_translation_probe(function, row, temps, address_local, memory.page_shift);
        function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        emit_translation_offset(function, row, temps);
        function.instruction(&Instruction::Else);
        side_exit(function)?;
        function.instruction(&Instruction::Return);
        function.instruction(&Instruction::End);
    } else {
        side_exit(function)?;
        function.instruction(&Instruction::Return);
    }
    function.instruction(&Instruction::End);
    Ok(())
}

fn emit_translation_index(
    function: &mut Function,
    memory: SystemMemory,
    temps: MemoryTemps,
    address_local: u32,
) {
    if let Some(page_local) = temps.page {
        function.instruction(&Instruction::LocalGet(page_local));
    } else {
        function.instruction(&Instruction::LocalGet(address_local));
        function.instruction(&Instruction::I64Const(i64::from(memory.page_shift)));
        function.instruction(&Instruction::I64ShrU);
    }
    if memory.index_hash_shift != 0 {
        // Preserve the page in the otherwise-free offset temporary while
        // folding its upper VPN bits into the direct-map index. Tags still
        // carry the complete VA/context proof, so hashing changes collision
        // behavior only—not hit validity.
        function.instruction(&Instruction::LocalTee(temps.offset));
        function.instruction(&Instruction::LocalGet(temps.offset));
        function.instruction(&Instruction::I64Const(i64::from(memory.index_hash_shift)));
        function.instruction(&Instruction::I64ShrU);
        function.instruction(&Instruction::I64Xor);
    }
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Const(memory.index_mask as i32));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(temps.index));
}

fn emit_side_exit(
    function: &mut Function,
    region: &Region,
    layout: JitLayout,
    local_map: &[Option<u32>],
    defined: &[bool],
    exit: &SideExit,
) -> Result<(), EmitError> {
    emit_commit_outputs(function, layout, &exit.outputs, |function, value| {
        emit_value(function, region, layout, local_map, defined, value)
    })?;
    emit_commit_f_outputs(function, layout, &exit.f_outputs, |function, value| {
        emit_value(function, region, layout, local_map, defined, value)
    })?;
    if let Some(value) = exit.fcsr_output {
        function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
        emit_value(function, region, layout, local_map, defined, value)?;
        function.instruction(&Instruction::I32Store(memarg(2, 0)));
    }
    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    emit_guest_pc(function, exit.guest_pc, layout);
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    emit_retirement_const(function, layout.retired_addr, exit.retired);
    Ok(())
}

fn emit_pc_from_local(function: &mut Function, layout: JitLayout, local: u32) {
    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
}

/// Publish a cold monomorphic-guard miss for the runtime's bounded
/// polymorphic-target profiler. The owner is written last and acts as the
/// validity tag after `call_block` cleared it before entry.
fn emit_ic_guard_miss(
    function: &mut Function,
    region: &Region,
    layout: JitLayout,
    target_local: u32,
) {
    if layout.ic_miss_owner_addr == 0 || layout.ic_miss_target_addr == 0 {
        return;
    }
    function.instruction(&Instruction::I32Const(layout.ic_miss_target_addr as i32));
    function.instruction(&Instruction::LocalGet(target_local));
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    function.instruction(&Instruction::I32Const(layout.ic_miss_owner_addr as i32));
    emit_guest_pc(function, region.entry_pc, layout);
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
}

fn emit_commit_outputs(
    function: &mut Function,
    layout: JitLayout,
    outputs: &[(u8, ValueId)],
    mut emit_output: impl FnMut(&mut Function, ValueId) -> Result<(), EmitError>,
) -> Result<(), EmitError> {
    for &(reg, value) in outputs {
        function.instruction(&Instruction::I32Const(layout.x_base as i32));
        emit_output(function, value)?;
        function.instruction(&Instruction::I64Store(memarg(3, u64::from(reg) * 8)));
    }
    Ok(())
}

fn emit_commit_f_outputs(
    function: &mut Function,
    layout: JitLayout,
    outputs: &[(u8, ValueId)],
    mut emit_output: impl FnMut(&mut Function, ValueId) -> Result<(), EmitError>,
) -> Result<(), EmitError> {
    for &(reg, value) in outputs {
        function.instruction(&Instruction::I32Const(layout.f_base as i32));
        emit_output(function, value)?;
        function.instruction(&Instruction::I64Store(memarg(3, u64::from(reg) * 8)));
    }
    Ok(())
}

fn emit_retirement_const(function: &mut Function, retired_addr: u32, retired: u32) {
    if retired == 0 {
        return;
    }
    function.instruction(&Instruction::I32Const(retired_addr as i32));
    function.instruction(&Instruction::I32Const(retired_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::I64Const(i64::from(retired)));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
}

fn emit_retirement_local(function: &mut Function, retired_addr: u32, local: u32, extra: u32) {
    function.instruction(&Instruction::I32Const(retired_addr as i32));
    function.instruction(&Instruction::I32Const(retired_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I64Add);
    if extra != 0 {
        function.instruction(&Instruction::I64Const(i64::from(extra)));
        function.instruction(&Instruction::I64Add);
    }
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
}

/// Finish an SC attempt after its conditional store has either completed or
/// been skipped. Reservation op 1 is deliberately a non-destructive probe:
/// store-address validation can side-exit, in which case the interpreter must
/// re-execute the same SC with the reservation still live. Only a path that
/// stays in generated code reaches this unconditional clear.
fn emit_reservation_clear(
    function: &mut Function,
    helpers: HelperImports,
    address_local: u32,
) -> Result<(), EmitError> {
    function.instruction(&Instruction::I32Const(2));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalGet(address_local));
    function.instruction(&Instruction::Call(helpers.reservation_index().ok_or_else(
        || EmitError("conditional store lacks a reservation helper".into()),
    )?));
    function.instruction(&Instruction::Drop);
    Ok(())
}

fn import_guest_base(imports: &mut ImportSection, layout: JitLayout) {
    if layout.pic_code_base.is_some() {
        imports.import(
            "env",
            "guest_base",
            EntityType::Global(GlobalType {
                val_type: ValType::I64,
                mutable: false,
                shared: false,
            }),
        );
    }
}

fn finish_module(function: Function, helpers: HelperImports, layout: JitLayout) -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32], []);
    if helpers.fp {
        types.ty().function(
            [
                ValType::I32,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I32,
                ValType::I32,
            ],
            [ValType::I64],
        );
    }
    if helpers.reservation.is_some() {
        types
            .ty()
            .function([ValType::I32, ValType::I32, ValType::I64], [ValType::I32]);
    }
    if helpers.tlb_fill {
        types
            .ty()
            .function([ValType::I64, ValType::I32], [ValType::I64]);
    }
    if helpers.bulk_copy {
        types.ty().function(
            [
                ValType::I32,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I32,
                ValType::I32,
                ValType::I32,
            ],
            [ValType::I64],
        );
    }
    module.section(&types);

    let mut imports = ImportSection::new();
    imports.import(
        "env",
        "memory",
        MemoryType {
            minimum: 0,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        },
    );
    import_guest_base(&mut imports, layout);
    if helpers.fp {
        imports.import("env", "fp_exec", EntityType::Function(1));
    }
    if let Some(reservation) = helpers.reservation {
        imports.import(
            "env",
            match reservation {
                ReservationCapability::User => "user_reservation",
                ReservationCapability::System => "system_reservation",
            },
            EntityType::Function(1 + helpers.fp as u32),
        );
    }
    if helpers.tlb_fill {
        imports.import(
            "env",
            "tlb_fill",
            EntityType::Function(1 + helpers.fp as u32 + helpers.reservation.is_some() as u32),
        );
    }
    if helpers.bulk_copy {
        imports.import(
            "env",
            "system_bulk_copy",
            EntityType::Function(
                1 + helpers.fp as u32
                    + helpers.reservation.is_some() as u32
                    + helpers.tlb_fill as u32,
            ),
        );
    }
    if helpers.chain {
        imports.import("env", "chain_next", EntityType::Function(0));
    }
    module.section(&imports);

    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);

    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, helpers.count());
    module.section(&exports);

    let mut code = CodeSection::new();
    code.function(&function);
    module.section(&code);
    module.finish()
}

/// Finish a module whose public entries all name one register-resident
/// dispatcher/body function. Keeping this distinct from `finish_multi_module`
/// makes the function-index calculation explicit: helper imports precede the
/// sole defined function, while linear memory is not in the function index
/// space.
fn finish_shared_module(
    function: Function,
    helpers: HelperImports,
    member_count: u32,
    export_members: bool,
    layout: JitLayout,
) -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32], []);
    if helpers.fp {
        types.ty().function(
            [
                ValType::I32,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I32,
                ValType::I32,
            ],
            [ValType::I64],
        );
    }
    if helpers.reservation.is_some() {
        types
            .ty()
            .function([ValType::I32, ValType::I32, ValType::I64], [ValType::I32]);
    }
    if helpers.tlb_fill {
        types
            .ty()
            .function([ValType::I64, ValType::I32], [ValType::I64]);
    }
    if helpers.bulk_copy {
        types.ty().function(
            [
                ValType::I32,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I32,
                ValType::I32,
                ValType::I32,
            ],
            [ValType::I64],
        );
    }
    if helpers.tail_chain {
        types.ty().function([ValType::I32, ValType::I32], []);
    }
    module.section(&types);

    let mut imports = ImportSection::new();
    imports.import(
        "env",
        "memory",
        MemoryType {
            minimum: 0,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        },
    );
    import_guest_base(&mut imports, layout);
    if helpers.fp {
        imports.import("env", "fp_exec", EntityType::Function(1));
    }
    if let Some(reservation) = helpers.reservation {
        imports.import(
            "env",
            match reservation {
                ReservationCapability::User => "user_reservation",
                ReservationCapability::System => "system_reservation",
            },
            EntityType::Function(1 + helpers.fp as u32),
        );
    }
    if helpers.tlb_fill {
        imports.import(
            "env",
            "tlb_fill",
            EntityType::Function(1 + helpers.fp as u32 + helpers.reservation.is_some() as u32),
        );
    }
    if helpers.bulk_copy {
        imports.import(
            "env",
            "system_bulk_copy",
            EntityType::Function(
                1 + helpers.fp as u32
                    + helpers.reservation.is_some() as u32
                    + helpers.tlb_fill as u32,
            ),
        );
    }
    if helpers.chain {
        imports.import("env", "chain_next", EntityType::Function(0));
    }
    if helpers.tail_chain {
        imports.import(
            "env",
            "tail_chain",
            EntityType::Function(
                1 + helpers.fp as u32
                    + helpers.reservation.is_some() as u32
                    + helpers.tlb_fill as u32
                    + helpers.bulk_copy as u32,
            ),
        );
    }
    module.section(&imports);

    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);

    let function_index = helpers.count();
    let mut exports = ExportSection::new();
    if export_members {
        for index in 0..member_count {
            exports.export(&format!("r{index}"), ExportKind::Func, function_index);
        }
    } else {
        exports.export("run", ExportKind::Func, function_index);
    }
    module.section(&exports);

    let mut code = CodeSection::new();
    code.function(&function);
    module.section(&code);
    module.finish()
}

const MULTI_ENTRY_HOP_CAP: i32 = 65_536;

fn emit_multi_dispatch(entries: &[u64], layout: JitLayout, function_base: u32) -> Function {
    // local 0 is the opaque state parameter.
    const PC_LOCAL: u32 = 1;
    const RETIRED_BEFORE_LOCAL: u32 = 2;
    const MATCHED_LOCAL: u32 = 3;
    const HOPS_LOCAL: u32 = 4;
    let mut function =
        Function::new_with_locals_types([ValType::I64, ValType::I64, ValType::I32, ValType::I32]);

    let mut sorted: Vec<(u64, u32)> = entries
        .iter()
        .enumerate()
        .map(|(index, &pc)| (pc, function_base + index as u32))
        .collect();
    sorted.sort_unstable_by_key(|&(pc, _)| pc);

    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(HOPS_LOCAL));
    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));

    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalSet(PC_LOCAL));
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalSet(RETIRED_BEFORE_LOCAL));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(MATCHED_LOCAL));

    emit_dispatch_tree(&mut function, &sorted, layout, PC_LOCAL, MATCHED_LOCAL);

    // An uncovered PC or a zero-retirement precise side exit belongs to T0.
    function.instruction(&Instruction::LocalGet(MATCHED_LOCAL));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::BrIf(1));
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalGet(RETIRED_BEFORE_LOCAL));
    function.instruction(&Instruction::I64Eq);
    function.instruction(&Instruction::BrIf(1));

    // RETIRED_CELL is cumulative for the entire public invocation. Loop
    // bodies compare against the same cumulative budget (see their guards),
    // and the dispatcher stops before beginning another member once it is
    // exhausted.
    if layout.fuel_addr != 0 {
        function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::I32Const(layout.fuel_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::I64GeU);
        function.instruction(&Instruction::BrIf(1));
    }

    // Keep a malformed/no-fuel embedding bounded even when every member
    // retires one instruction and cycles entirely inside the module.
    function.instruction(&Instruction::LocalGet(HOPS_LOCAL));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalTee(HOPS_LOCAL));
    function.instruction(&Instruction::I32Const(MULTI_ENTRY_HOP_CAP));
    function.instruction(&Instruction::I32GeU);
    function.instruction(&Instruction::BrIf(1));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function
}

fn emit_dispatch_tree(
    function: &mut Function,
    entries: &[(u64, u32)],
    layout: JitLayout,
    pc_local: u32,
    matched_local: u32,
) {
    if entries.len() <= 3 {
        for &(pc, target) in entries {
            function.instruction(&Instruction::LocalGet(pc_local));
            emit_guest_pc(function, pc, layout);
            function.instruction(&Instruction::I64Eq);
            function.instruction(&Instruction::If(BlockType::Empty));
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::Call(target));
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::LocalSet(matched_local));
            function.instruction(&Instruction::End);
        }
        return;
    }

    let middle = entries.len() / 2;
    function.instruction(&Instruction::LocalGet(pc_local));
    emit_guest_pc(function, entries[middle].0, layout);
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_dispatch_tree(
        function,
        &entries[..middle],
        layout,
        pc_local,
        matched_local,
    );
    function.instruction(&Instruction::Else);
    emit_dispatch_tree(
        function,
        &entries[middle..],
        layout,
        pc_local,
        matched_local,
    );
    function.instruction(&Instruction::End);
}

fn emit_tail_wrapper(body_index: u32, target: Option<(u64, u32)>, layout: JitLayout) -> Function {
    // local 0 is the opaque state pointer; local 1 snapshots cumulative
    // retirement so a zero-progress precise side exit can never tail-spin.
    let mut function = Function::new_with_locals_types([ValType::I64]);
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalSet(1));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::Call(body_index));

    if let Some((target_pc, target_index)) = target {
        function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::LocalGet(1));
        function.instruction(&Instruction::I64GtU);
        function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        emit_guest_pc(&mut function, target_pc, layout);
        function.instruction(&Instruction::I64Eq);
        function.instruction(&Instruction::I32And);
        function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::I32Const(layout.fuel_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::I64LtU);
        function.instruction(&Instruction::I32And);
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::ReturnCall(target_index));
        function.instruction(&Instruction::End);
    }
    function.instruction(&Instruction::End);
    function
}

fn finish_multi_module(
    functions: Vec<Function>,
    wrappers: Vec<Function>,
    dispatcher: Function,
    helpers: HelperImports,
    export_members: bool,
    layout: JitLayout,
) -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32], []);
    if helpers.fp {
        types.ty().function(
            [
                ValType::I32,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I32,
                ValType::I32,
            ],
            [ValType::I64],
        );
    }
    if helpers.reservation.is_some() {
        types
            .ty()
            .function([ValType::I32, ValType::I32, ValType::I64], [ValType::I32]);
    }
    if helpers.tlb_fill {
        types
            .ty()
            .function([ValType::I64, ValType::I32], [ValType::I64]);
    }
    if helpers.bulk_copy {
        types.ty().function(
            [
                ValType::I32,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I32,
                ValType::I32,
                ValType::I32,
            ],
            [ValType::I64],
        );
    }
    module.section(&types);

    let mut imports = ImportSection::new();
    imports.import(
        "env",
        "memory",
        MemoryType {
            minimum: 0,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        },
    );
    import_guest_base(&mut imports, layout);
    if helpers.fp {
        imports.import("env", "fp_exec", EntityType::Function(1));
    }
    if let Some(reservation) = helpers.reservation {
        imports.import(
            "env",
            match reservation {
                ReservationCapability::User => "user_reservation",
                ReservationCapability::System => "system_reservation",
            },
            EntityType::Function(1 + helpers.fp as u32),
        );
    }
    if helpers.tlb_fill {
        imports.import(
            "env",
            "tlb_fill",
            EntityType::Function(1 + helpers.fp as u32 + helpers.reservation.is_some() as u32),
        );
    }
    if helpers.bulk_copy {
        imports.import(
            "env",
            "system_bulk_copy",
            EntityType::Function(
                1 + helpers.fp as u32
                    + helpers.reservation.is_some() as u32
                    + helpers.tlb_fill as u32,
            ),
        );
    }
    module.section(&imports);

    let body_count = functions.len() as u32;
    let wrapper_count = wrappers.len() as u32;
    let mut function_section = FunctionSection::new();
    for _ in 0..=body_count + wrapper_count {
        function_section.function(0);
    }
    module.section(&function_section);

    let dispatcher_index = helpers.count() + body_count + wrapper_count;
    let mut exports = ExportSection::new();
    if export_members {
        for index in 0..body_count {
            exports.export(&format!("r{index}"), ExportKind::Func, dispatcher_index);
        }
    } else {
        exports.export("run", ExportKind::Func, dispatcher_index);
    }
    module.section(&exports);

    let mut code = CodeSection::new();
    for function in &functions {
        code.function(function);
    }
    for wrapper in &wrappers {
        code.function(wrapper);
    }
    code.function(&dispatcher);
    module.section(&code);
    module.finish()
}

fn emit_single_latch_loop(
    region: &Region,
    layout: JitLayout,
    loop_backedge: LoopBackedge,
) -> Result<Function, EmitError> {
    if !region.f_outputs.is_empty() || region.fcsr_output.is_some() {
        return Err(EmitError(
            "floating-point state is not yet supported by the loop carrier".into(),
        ));
    }
    // Every SSA value gets a local because the loop body recomputes values on
    // every iteration. ReadX locals are loop parameters initialized once from
    // architectural state and updated from final outputs on the backedge.
    let local_map: Vec<Option<u32>> = (0..region.values.len())
        .map(|index| Some(1 + index as u32))
        .collect();
    let retired_local = 1 + region.values.len() as u32;
    let next_pc_local = retired_local + 1;
    let mut local_types: Vec<ValType> = region.values.iter().map(|v| val_type(v.ty)).collect();
    local_types.extend([ValType::I64, ValType::I64]);
    let mut function = Function::new_with_locals_types(local_types);

    // Initialize loop-carried architectural inputs before any possible state
    // commit. Other values are recomputed inside the loop.
    for (index, value) in region.values.iter().enumerate() {
        if matches!(value.op, Op::ReadX(_)) {
            emit_value_body(
                &mut function,
                region,
                layout,
                &local_map,
                &vec![true; region.values.len()],
                ValueId(index),
            )?;
            function.instruction(&Instruction::LocalSet(1 + index as u32));
        }
    }
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::LocalSet(retired_local));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));

    let all_defined = vec![true; region.values.len()];
    for (index, value) in region.values.iter().enumerate() {
        if matches!(value.op, Op::ReadX(_)) {
            continue;
        }
        emit_value_body(
            &mut function,
            region,
            layout,
            &local_map,
            &all_defined,
            ValueId(index),
        )?;
        function.instruction(&Instruction::LocalSet(1 + index as u32));
    }

    function.instruction(&Instruction::LocalGet(retired_local));
    function.instruction(&Instruction::I64Const(i64::from(region.retired)));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(retired_local));

    // Continue only when the guest branch is taken and another full iteration
    // fits the current dispatch's fuel. The first iteration is always allowed,
    // preserving the bounded-basic-block overshoot contract.
    if let Some(condition) = loop_backedge.condition {
        function.instruction(&Instruction::LocalGet(1 + condition.0 as u32));
    } else {
        function.instruction(&Instruction::I32Const(1));
    }
    if layout.fuel_addr != 0 {
        function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::LocalGet(retired_local));
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::I32Const(layout.fuel_addr as i32));
        function.instruction(&Instruction::I64Load(memarg(3, 0)));
        function.instruction(&Instruction::I64LtU);
        function.instruction(&Instruction::I32And);
    } else {
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::If(BlockType::Empty));

    // Backedge parallel-copy: outputs are already materialized, so assigning
    // them to ReadX parameter locals cannot clobber another source.
    for (index, value) in region.values.iter().enumerate() {
        let Op::ReadX(reg) = value.op else { continue };
        if let Some(&(_, output)) = region.outputs.iter().find(|&&(r, _)| r == reg) {
            function.instruction(&Instruction::LocalGet(1 + output.0 as u32));
            function.instruction(&Instruction::LocalSet(1 + index as u32));
        }
    }
    // From inside the `if`, depth one is the surrounding loop.
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);

    // The SSA select already says fallthrough when the condition is false and
    // the loop header when it was true but fuel ended.
    function.instruction(&Instruction::LocalGet(1 + region.next_pc.0 as u32));
    function.instruction(&Instruction::LocalSet(next_pc_local));
    // Break out of the surrounding block (loop depth 0, block depth 1).
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    for &(reg, value) in &region.outputs {
        function.instruction(&Instruction::I32Const(layout.x_base as i32));
        function.instruction(&Instruction::LocalGet(1 + value.0 as u32));
        function.instruction(&Instruction::I64Store(memarg(3, u64::from(reg) * 8)));
    }
    function.instruction(&Instruction::I32Const(layout.pc_addr as i32));
    function.instruction(&Instruction::LocalGet(next_pc_local));
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::I32Const(layout.retired_addr as i32));
    function.instruction(&Instruction::I64Load(memarg(3, 0)));
    function.instruction(&Instruction::LocalGet(retired_local));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Store(memarg(3, 0)));
    function.instruction(&Instruction::End);

    Ok(function)
}

fn emit_value(
    function: &mut Function,
    region: &Region,
    layout: JitLayout,
    local_map: &[Option<u32>],
    defined: &[bool],
    value: ValueId,
) -> Result<(), EmitError> {
    if let Some(local) = local_map[value.0] {
        if defined[value.0] {
            function.instruction(&Instruction::LocalGet(local));
            return Ok(());
        }
    }
    emit_value_body(function, region, layout, local_map, defined, value)
}

fn emit_value_body(
    function: &mut Function,
    region: &Region,
    layout: JitLayout,
    local_map: &[Option<u32>],
    defined: &[bool],
    value: ValueId,
) -> Result<(), EmitError> {
    let data = &region.values[value.0];
    let operand = |function: &mut Function, value: ValueId| {
        emit_value(function, region, layout, local_map, defined, value)
    };
    match data.op {
        Op::ConstI32(value) => {
            function.instruction(&Instruction::I32Const(value));
        }
        Op::ConstI64(value) => {
            function.instruction(&Instruction::I64Const(value));
        }
        Op::GuestPc(pc) => emit_guest_pc(function, pc, layout),
        Op::ReadX(reg) => {
            function.instruction(&Instruction::I32Const(layout.x_base as i32));
            function.instruction(&Instruction::I64Load(memarg(3, u64::from(reg) * 8)));
        }
        Op::ReadF(reg) => {
            function.instruction(&Instruction::I32Const(layout.f_base as i32));
            function.instruction(&Instruction::I64Load(memarg(3, u64::from(reg) * 8)));
        }
        Op::ReadFcsr => {
            function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
            function.instruction(&Instruction::I32Load(memarg(2, 0)));
        }
        Op::Binary { op, lhs, rhs } => {
            operand(function, lhs)?;
            operand(function, rhs)?;
            let instruction = match op {
                BinaryOp::I64Add => Instruction::I64Add,
                BinaryOp::I64Sub => Instruction::I64Sub,
                BinaryOp::I64Mul => Instruction::I64Mul,
                BinaryOp::I64And => Instruction::I64And,
                BinaryOp::I64Or => Instruction::I64Or,
                BinaryOp::I64Xor => Instruction::I64Xor,
                BinaryOp::I64Shl => Instruction::I64Shl,
                BinaryOp::I64ShrU => Instruction::I64ShrU,
                BinaryOp::I64ShrS => Instruction::I64ShrS,
                BinaryOp::I32Add => Instruction::I32Add,
                BinaryOp::I32Sub => Instruction::I32Sub,
                BinaryOp::I32Mul => Instruction::I32Mul,
                BinaryOp::I32And => Instruction::I32And,
                BinaryOp::I32Or => Instruction::I32Or,
                BinaryOp::I32Xor => Instruction::I32Xor,
                BinaryOp::I32Shl => Instruction::I32Shl,
                BinaryOp::I32ShrU => Instruction::I32ShrU,
                BinaryOp::I32ShrS => Instruction::I32ShrS,
                BinaryOp::I64Eq => Instruction::I64Eq,
                BinaryOp::I64Ne => Instruction::I64Ne,
                BinaryOp::I64LtS => Instruction::I64LtS,
                BinaryOp::I64LtU => Instruction::I64LtU,
                BinaryOp::I64GeS => Instruction::I64GeS,
                BinaryOp::I64GeU => Instruction::I64GeU,
            };
            function.instruction(&instruction);
        }
        Op::Divide { op, lhs, rhs } => {
            emit_guarded_divide(function, op, |function, which| {
                operand(function, if which == 0 { lhs } else { rhs })
            })?;
        }
        Op::WrapI64ToI32(value) => {
            operand(function, value)?;
            function.instruction(&Instruction::I32WrapI64);
        }
        Op::ExtendI32S(value) => {
            operand(function, value)?;
            function.instruction(&Instruction::I64ExtendI32S);
        }
        Op::ExtendI32U(value) => {
            operand(function, value)?;
            function.instruction(&Instruction::I64ExtendI32U);
        }
        Op::SelectI64 {
            condition,
            if_true,
            if_false,
        } => {
            operand(function, if_true)?;
            operand(function, if_false)?;
            operand(function, condition)?;
            function.instruction(&Instruction::Select);
        }
        Op::Load { .. } => {
            return Err(EmitError(
                "effectful load reached the pure stackifying emitter".into(),
            ));
        }
        Op::ExactFp { .. } => {
            return Err(EmitError(
                "effectful FP helper reached the pure stackifying emitter".into(),
            ));
        }
        Op::Reservation { .. } => {
            return Err(EmitError(
                "reservation helper reached the pure stackifying emitter".into(),
            ));
        }
        Op::ReloadFcsr(_) => {
            function.instruction(&Instruction::I32Const(layout.fcsr_addr as i32));
            function.instruction(&Instruction::I32Load(memarg(2, 0)));
        }
    }
    Ok(())
}

/// Lower RISC-V's non-trapping divide contract to structured Wasm. Wasm's
/// integer division is partial for a zero divisor and for MIN / -1, so `select`
/// cannot guard it (both select operands are evaluated). Nested result-typed
/// `if`s keep the trapping operator entirely off the exceptional path.
fn emit_guarded_divide(
    function: &mut Function,
    op: DivideOp,
    mut operand: impl FnMut(&mut Function, u8) -> Result<(), EmitError>,
) -> Result<(), EmitError> {
    let ty = op.value_type();
    operand(function, 1)?;
    function.instruction(&match ty {
        ValueType::I32 => Instruction::I32Eqz,
        ValueType::I64 => Instruction::I64Eqz,
    });
    function.instruction(&Instruction::If(BlockType::Result(val_type(ty))));
    if op.is_remainder() {
        operand(function, 0)?;
    } else {
        function.instruction(&match ty {
            ValueType::I32 => Instruction::I32Const(-1),
            ValueType::I64 => Instruction::I64Const(-1),
        });
    }
    function.instruction(&Instruction::Else);

    if op.is_signed() {
        operand(function, 0)?;
        function.instruction(&match ty {
            ValueType::I32 => Instruction::I32Const(i32::MIN),
            ValueType::I64 => Instruction::I64Const(i64::MIN),
        });
        function.instruction(&match ty {
            ValueType::I32 => Instruction::I32Eq,
            ValueType::I64 => Instruction::I64Eq,
        });
        operand(function, 1)?;
        function.instruction(&match ty {
            ValueType::I32 => Instruction::I32Const(-1),
            ValueType::I64 => Instruction::I64Const(-1),
        });
        function.instruction(&match ty {
            ValueType::I32 => Instruction::I32Eq,
            ValueType::I64 => Instruction::I64Eq,
        });
        function.instruction(&Instruction::I32And);
        function.instruction(&Instruction::If(BlockType::Result(val_type(ty))));
        if op.is_remainder() {
            function.instruction(&match ty {
                ValueType::I32 => Instruction::I32Const(0),
                ValueType::I64 => Instruction::I64Const(0),
            });
        } else {
            operand(function, 0)?;
        }
        function.instruction(&Instruction::Else);
        emit_raw_divide(function, op, &mut operand)?;
        function.instruction(&Instruction::End);
    } else {
        emit_raw_divide(function, op, &mut operand)?;
    }
    function.instruction(&Instruction::End);
    Ok(())
}

fn emit_raw_divide(
    function: &mut Function,
    op: DivideOp,
    operand: &mut impl FnMut(&mut Function, u8) -> Result<(), EmitError>,
) -> Result<(), EmitError> {
    operand(function, 0)?;
    operand(function, 1)?;
    let instruction = match op {
        DivideOp::I64DivS => Instruction::I64DivS,
        DivideOp::I64DivU => Instruction::I64DivU,
        DivideOp::I64RemS => Instruction::I64RemS,
        DivideOp::I64RemU => Instruction::I64RemU,
        DivideOp::I32DivS => Instruction::I32DivS,
        DivideOp::I32DivU => Instruction::I32DivU,
        DivideOp::I32RemS => Instruction::I32RemS,
        DivideOp::I32RemU => Instruction::I32RemU,
    };
    function.instruction(&instruction);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Builder, ExitKind};
    use wasmparser::{Operator, Parser, Payload};

    fn count_i64_state_ops(bytes: &[u8], offset: u64) -> (usize, usize) {
        let mut loads = 0;
        let mut stores = 0;
        for payload in Parser::new(0).parse_all(bytes) {
            let Payload::CodeSectionEntry(body) = payload.expect("generated payload parses") else {
                continue;
            };
            let mut operators = body
                .get_operators_reader()
                .expect("generated function operators parse");
            while !operators.eof() {
                match operators.read().expect("generated operator parses") {
                    Operator::I64Load { memarg } if memarg.offset == offset => loads += 1,
                    Operator::I64Store { memarg } if memarg.offset == offset => stores += 1,
                    _ => {}
                }
            }
        }
        (loads, stores)
    }

    fn two_pair_memory_region(transform_values: bool) -> Region {
        let mut builder = Builder::new(0x1000);
        let source = builder.read_x(1, 0x1000);
        let destination = builder.read_x(2, 0x1000);
        let eight = builder.const_i64(8, 0x1000);
        let one = builder.const_i64(1, 0x1000);
        let loaded0 = builder.load(source, LoadKind::I64, 0x1000, 0);
        let stored0 = if transform_values {
            builder.binary(BinaryOp::I64Add, loaded0, one, 0x1004)
        } else {
            loaded0
        };
        builder.store(destination, stored0, StoreKind::I64, 0x1004, 1);
        let source8 = builder.binary(BinaryOp::I64Add, source, eight, 0x1008);
        let destination8 = builder.binary(BinaryOp::I64Add, destination, eight, 0x1008);
        let loaded1 = builder.load(source8, LoadKind::I64, 0x1008, 2);
        let stored1 = if transform_values {
            builder.binary(BinaryOp::I64Add, loaded1, one, 0x100c)
        } else {
            loaded1
        };
        builder.store(destination8, stored1, StoreKind::I64, 0x100c, 3);
        let next = builder.const_i64(0x1010, 0x100c);
        builder.finish(0x1010, next, 4, ExitKind::Dispatch)
    }

    /// Match the common compiler ordering `ld tmp, (src); sd tmp, (dst)`: the
    /// destination architectural register is first materialized only while
    /// lifting the store, after the first load SSA value already exists.
    fn two_pair_late_destination_region() -> Region {
        let mut builder = Builder::new(0x1000);
        let source = builder.read_x(1, 0x1000);
        let loaded0 = builder.load(source, LoadKind::I64, 0x1000, 0);
        let destination = builder.read_x(2, 0x1004);
        builder.store(destination, loaded0, StoreKind::I64, 0x1004, 1);
        let eight = builder.const_i64(8, 0x1008);
        let source8 = builder.binary(BinaryOp::I64Add, source, eight, 0x1008);
        let loaded1 = builder.load(source8, LoadKind::I64, 0x1008, 2);
        let destination8 = builder.binary(BinaryOp::I64Add, destination, eight, 0x100c);
        builder.store(destination8, loaded1, StoreKind::I64, 0x100c, 3);
        let next = builder.const_i64(0x1010, 0x100c);
        builder.finish(0x1010, next, 4, ExitKind::Dispatch)
    }

    fn four_store_fill_region() -> Region {
        let mut builder = Builder::new(0x1000);
        let destination = builder.read_x(1, 0x1000);
        let fill = builder.read_x(2, 0x1000);
        for (index, offset) in [0i64, 8, 16, 24].into_iter().enumerate() {
            let immediate = builder.const_i64(offset, 0x1000 + index as u64 * 4);
            let address = builder.binary(
                BinaryOp::I64Add,
                destination,
                immediate,
                0x1000 + index as u64 * 4,
            );
            builder.store(
                address,
                fill,
                StoreKind::I64,
                0x1000 + index as u64 * 4,
                index as u32,
            );
        }
        let next = builder.const_i64(0x1010, 0x100c);
        builder.finish(0x1010, next, 4, ExitKind::Dispatch)
    }

    fn unrolled_copy_loop(step: i64) -> Region {
        let mut builder = Builder::new(0x2000);
        let source = builder.read_x(13, 0x2000);
        let destination = builder.read_x(14, 0x2000);
        let limit = builder.read_x(11, 0x2000);
        let count = builder.read_x(12, 0x2000);
        let offsets: Vec<i64> = if step > 0 {
            (0..8).map(|slot| slot * 8).collect()
        } else {
            (1..=8).map(|slot| -(slot * 8)).collect()
        };
        for (slot, offset) in offsets.into_iter().enumerate() {
            let immediate = builder.const_i64(offset, 0x2000 + slot as u64 * 8);
            let source_address = builder.binary(
                BinaryOp::I64Add,
                source,
                immediate,
                0x2000 + slot as u64 * 8,
            );
            let loaded = builder.load(
                source_address,
                LoadKind::I64,
                0x2000 + slot as u64 * 8,
                (slot * 2) as u32,
            );
            builder.write_x(15, loaded);
            let destination_address = builder.binary(
                BinaryOp::I64Add,
                destination,
                immediate,
                0x2004 + slot as u64 * 8,
            );
            builder.store(
                destination_address,
                loaded,
                StoreKind::I64,
                0x2004 + slot as u64 * 8,
                (slot * 2 + 1) as u32,
            );
        }
        let step_value = builder.const_i64(step, 0x2040);
        let next_destination = builder.binary(BinaryOp::I64Add, destination, step_value, 0x2040);
        builder.write_x(14, next_destination);
        let minus_64 = builder.const_i64(-64, 0x2044);
        let next_count = builder.binary(BinaryOp::I64Add, count, minus_64, 0x2044);
        builder.write_x(12, next_count);
        let next_source = builder.binary(BinaryOp::I64Add, source, step_value, 0x2048);
        builder.write_x(13, next_source);
        let condition = builder.binary(BinaryOp::I64LtU, limit, next_count, 0x204c);
        let taken = builder.const_i64(0x2000, 0x204c);
        let not_taken = builder.const_i64(0x2050, 0x204c);
        let next = builder.select_i64(condition, taken, not_taken, 0x204c);
        builder.finish(0x2050, next, 20, ExitKind::Dispatch)
    }

    fn single_word_copy_loop(step: i64) -> Region {
        let mut builder = Builder::new(0x4000);
        let source = builder.read_x(13, 0x4000);
        let destination = builder.read_x(14, 0x4000);
        let limit = builder.read_x(11, 0x4000);
        let count = builder.read_x(12, 0x4000);
        let offset = builder.const_i64(if step < 0 { -8 } else { 0 }, 0x4000);
        let source_address = builder.binary(BinaryOp::I64Add, source, offset, 0x4000);
        let loaded = builder.load(source_address, LoadKind::I64, 0x4000, 0);
        builder.write_x(15, loaded);
        let destination_address = builder.binary(BinaryOp::I64Add, destination, offset, 0x4004);
        builder.store(destination_address, loaded, StoreKind::I64, 0x4004, 1);
        let step_value = builder.const_i64(step, 0x4008);
        let next_destination = builder.binary(BinaryOp::I64Add, destination, step_value, 0x4008);
        builder.write_x(14, next_destination);
        let minus_eight = builder.const_i64(-8, 0x400c);
        let next_count = builder.binary(BinaryOp::I64Add, count, minus_eight, 0x400c);
        builder.write_x(12, next_count);
        let next_source = builder.binary(BinaryOp::I64Add, source, step_value, 0x4010);
        builder.write_x(13, next_source);
        let condition = builder.binary(BinaryOp::I64LtU, limit, next_count, 0x4014);
        let taken = builder.const_i64(0x4000, 0x4014);
        let not_taken = builder.const_i64(0x4018, 0x4014);
        let next = builder.select_i64(condition, taken, not_taken, 0x4014);
        builder.finish(0x4018, next, 6, ExitKind::Dispatch)
    }

    #[test]
    fn invocation_translation_cache_selects_only_dense_direct_copies() {
        let copied = two_pair_memory_region(false);
        assert_eq!(
            region_memory_profile(&copied),
            RegionMemoryProfile {
                loads: 2,
                stores: 2,
                direct_copies: 2,
            },
        );
        assert!(dense_copy_plan(&copied, 3).is_none());
        let plan = dense_copy_plan(&copied, 2).expect("two direct pairs form one copy range");
        assert_eq!(plan.bytes, 16);
        assert_eq!(plan.source_base_offset, 0);
        assert_eq!(plan.destination_base_offset, 0);
        assert_eq!(
            plan.accesses
                .iter()
                .map(|access| access.source_offset)
                .collect::<Vec<_>>(),
            vec![0, 8],
        );

        let late_destination = two_pair_late_destination_region();
        let late_plan = dense_copy_plan(&late_destination, 2)
            .expect("setup can move to the first store after its destination root");
        let first_store_position = late_destination
            .effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Store { position, .. } => Some(*position),
                _ => None,
            })
            .min()
            .expect("copy has stores");
        assert_eq!(late_plan.setup_position, first_store_position);
        assert!(late_plan.load_access(late_plan.accesses[0].load).is_none());
        assert!(late_plan.load_access(late_plan.accesses[1].load).is_some());

        let transformed = two_pair_memory_region(true);
        assert_eq!(region_memory_profile(&transformed).direct_copies, 0);
        assert!(dense_copy_plan(&transformed, 2).is_none());

        let fill = four_store_fill_region();
        let fill_plan = dense_store_plan(&fill, 4).expect("contiguous stores form one range");
        assert_eq!(fill_plan.bytes, 32);
        assert_eq!(fill_plan.destination_base_offset, 0);
        let first_fill_store = match fill.effects[0] {
            Effect::Store { position, .. } => position,
            _ => unreachable!("fill contains only stores"),
        };
        assert_eq!(fill_plan.setup_position, first_fill_store);
        assert_eq!(
            fill_plan
                .accesses
                .iter()
                .map(|access| access.destination_offset)
                .collect::<Vec<_>>(),
            vec![0, 8, 16, 24],
        );
        let mut fill_layout = system_layout(false);
        fill_layout.sys = fill_layout.sys.map(|memory| {
            memory
                .with_invocation_cache(true)
                .with_invocation_cache_min_accesses(4)
        });
        assert!(uses_dense_store_plan(&fill, fill_layout));
        let fill_wasm = emit(&fill, fill_layout, None).expect("dense store emits");
        wasmparser::Validator::new()
            .validate_all(&fill_wasm)
            .expect("dense-store module validates");
    }

    #[test]
    fn recognizes_only_the_proven_whole_copy_loop_shape() {
        for step in [64, -64] {
            let region = unrolled_copy_loop(step);
            let plan = bulk_copy_loop_plan(&region).expect("canonical 64-byte copy loop");
            assert_eq!(plan.source_reg, 13);
            assert_eq!(plan.destination_reg, 14);
            assert_eq!(plan.count_reg, 12);
            assert_eq!(plan.limit_reg, 11);
            assert_eq!(plan.value_reg, 15);
            assert_eq!(plan.step, step);
            assert_eq!(plan.bytes_per_iteration, 64);
            assert_eq!(plan.exit_pc, 0x2050);

            let loop_backedge = LoopBackedge {
                condition: Some(plan.condition),
                exit_pc: plan.exit_pc,
            };
            let layout = system_layout(true);
            let standalone = emit(&region, layout, Some(loop_backedge))
                .expect("whole-copy standalone module emits");
            wasmparser::Validator::new()
                .validate_all(&standalone)
                .expect("whole-copy standalone module validates");

            let mut other_builder = Builder::new(0x3000);
            let old = other_builder.read_x(1, 0x3000);
            let one = other_builder.const_i64(1, 0x3000);
            let next_value = other_builder.binary(BinaryOp::I64Add, old, one, 0x3000);
            other_builder.write_x(1, next_value);
            let next = other_builder.const_i64(0x3004, 0x3000);
            let other = other_builder.finish(0x3004, next, 1, ExitKind::Dispatch);
            let cached = emit_multi_entry(
                &[(&region, Some(loop_backedge)), (&other, None)],
                layout,
                true,
            )
            .expect("whole-copy cached module emits");
            wasmparser::Validator::new()
                .validate_all(&cached)
                .expect("whole-copy cached module validates");
            let structured = emit_multi_entry_mode(
                &[(&region, Some(loop_backedge)), (&other, None)],
                layout,
                true,
                MultiEntryState::RegisterStructured,
            )
            .expect("whole-copy structured module emits");
            wasmparser::Validator::new()
                .validate_all(&structured)
                .expect("whole-copy structured module validates");
        }

        for step in [8, -8] {
            let region = single_word_copy_loop(step);
            let plan = bulk_copy_loop_plan(&region).expect("canonical 8-byte copy loop");
            assert_eq!(plan.bytes_per_iteration, 8);
            assert_eq!(plan.limit_value, 7);
            assert_eq!(plan.step, step);
            let loop_backedge = LoopBackedge {
                condition: Some(plan.condition),
                exit_pc: plan.exit_pc,
            };
            let bytes = emit(&region, system_layout(true), Some(loop_backedge))
                .expect("8-byte copy module emits");
            wasmparser::Validator::new()
                .validate_all(&bytes)
                .expect("8-byte copy module validates");
        }

        let mut wrong_count = unrolled_copy_loop(64);
        let count_output = output_for_reg(&wrong_count, 12).expect("count output");
        let Op::Binary { rhs, .. } = wrong_count.values[count_output.0].op else {
            panic!("count induction must be binary")
        };
        wrong_count.values[rhs.0].op = Op::ConstI64(-8);
        assert!(bulk_copy_loop_plan(&wrong_count).is_none());
    }

    #[test]
    fn structured_execution_profile_is_opt_in_and_valid() {
        let mut first_builder = Builder::new(0x1000);
        let old = first_builder.read_x(1, 0x1000);
        let one = first_builder.const_i64(1, 0x1000);
        let value = first_builder.binary(BinaryOp::I64Add, old, one, 0x1000);
        first_builder.write_x(1, value);
        let next = first_builder.const_i64(0x1004, 0x1000);
        let mut first = first_builder.finish(0x1004, next, 1, ExitKind::Dispatch);
        first.trace_mix = [1, 0, 0, 0, 0];

        let mut second_builder = Builder::new(0x1004);
        let old = second_builder.read_x(2, 0x1004);
        let one = second_builder.const_i64(1, 0x1004);
        let value = second_builder.binary(BinaryOp::I64Add, old, one, 0x1004);
        second_builder.write_x(2, value);
        let next = second_builder.const_i64(0x1008, 0x1004);
        let mut second = second_builder.finish(0x1008, next, 1, ExitKind::Dispatch);
        second.trace_mix = [1, 0, 0, 0, 0];

        let members = [(&first, None), (&second, None)];
        let control = emit_multi_entry_mode(
            &members,
            JitLayout::bare(),
            true,
            MultiEntryState::RegisterStructured,
        )
        .expect("uninstrumented structured module emits");

        let mut profiled_layout = JitLayout::bare();
        profiled_layout.structured_profile =
            Some([1024, 1032, 1040, 1048, 1056, 1064, 1072, 1080, 1088]);
        let profiled = emit_multi_entry_mode(
            &members,
            profiled_layout,
            true,
            MultiEntryState::RegisterStructured,
        )
        .expect("profiled structured module emits");

        assert_ne!(control, profiled);
        assert!(profiled.len() > control.len());
        wasmparser::Validator::new()
            .validate_all(&profiled)
            .expect("profiled structured module validates");
    }

    #[test]
    fn structured_state_keeps_only_the_fixed_rvc_bank_resident() {
        let member = |entry_pc, next_pc| {
            let mut builder = Builder::new(entry_pc);
            let resident = builder.read_x(1, entry_pc);
            let materialized = builder.read_x(3, entry_pc);
            let one = builder.const_i64(1, entry_pc);
            let resident = builder.binary(BinaryOp::I64Add, resident, one, entry_pc);
            let materialized = builder.binary(BinaryOp::I64Add, materialized, one, entry_pc);
            builder.write_x(1, resident);
            builder.write_x(3, materialized);
            let next = builder.const_i64(next_pc as i64, entry_pc);
            builder.finish(next_pc, next, 1, ExitKind::Dispatch)
        };
        let first = member(0x1000, 0x1004);
        let second = member(0x1004, 0x1008);
        let bytes = emit_multi_entry_mode(
            &[(&first, None), (&second, None)],
            JitLayout::bare(),
            true,
            MultiEntryState::RegisterStructured,
        )
        .expect("hybrid structured module emits");
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("hybrid structured module validates");

        // x1 is loaded/committed once around the generated function. x3 has no
        // function-wide state local and is synchronized by both member bodies.
        assert_eq!(count_i64_state_ops(&bytes, 1 * 8), (1, 1));
        assert_eq!(count_i64_state_ops(&bytes, 3 * 8), (2, 2));
    }

    fn system_layout(refill_on_miss: bool) -> JitLayout {
        let mut layout = JitLayout::bare();
        layout.sys = Some(SystemMemory::fused_4k(
            4096,
            4096 + 32 * 1024,
            4096 + 64 * 1024,
            4096 + 96 * 1024,
            4096 + 128 * 1024,
            4095,
            refill_on_miss,
        ));
        layout.fuel_addr = 272;
        layout
    }

    #[test]
    fn emits_a_valid_imported_memory_module() {
        let mut builder = Builder::new(0x1000);
        let old = builder.read_x(1, 0x1000);
        let one = builder.const_i64(1, 0x1000);
        let value = builder.binary(BinaryOp::I64Add, old, one, 0x1000);
        builder.write_x(1, value);
        let next = builder.const_i64(0x1004, 0x1000);
        let region = builder.finish(0x1004, next, 1, ExitKind::Dispatch);
        let bytes = emit(&region, JitLayout::bare(), None).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated module must validate");
    }

    #[test]
    fn emits_a_valid_fuel_metered_loop() {
        // addi x1,x1,1; bne x1,x2,-4
        let words = [0x0010_8093u32, 0xfe20_9ee3u32];
        let code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let lifted = crate::lift::lift_t1(
            &code,
            0x1000,
            0x1000,
            false,
            crate::lift::FpMode::Disabled,
            false,
        )
        .unwrap();
        let mut layout = JitLayout::bare();
        layout.fuel_addr = 272;
        let bytes = emit(&lifted.ir, layout, lifted.loop_backedge).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated loop module must validate");
    }

    #[test]
    fn emits_valid_checked_flat_memory_accesses() {
        let mut builder = Builder::new(0x1000);
        let address = builder.read_x(1, 0x1000);
        let loaded = builder.load(address, LoadKind::I32U, 0x1000, 0);
        builder.write_x(2, loaded);
        let four = builder.const_i64(4, 0x1004);
        let next_address = builder.binary(BinaryOp::I64Add, address, four, 0x1004);
        builder.store(next_address, loaded, StoreKind::I64, 0x1004, 1);
        let next = builder.const_i64(0x1008, 0x1004);
        let region = builder.finish(0x1008, next, 2, ExitKind::Dispatch);
        let mut layout = JitLayout::bare();
        layout.mem = Some((4096, 65536));
        let bytes = emit(&region, layout, None).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated memory module must validate");
    }

    #[test]
    fn emits_a_valid_memory_loop_with_precise_exits() {
        let branch = |rs1: u32, rs2: u32, offset: i32| {
            let immediate = offset as u32 & 0x1fff;
            0x63 | (1 << 12)
                | (rs1 << 15)
                | (rs2 << 20)
                | (((immediate >> 11) & 1) << 7)
                | (((immediate >> 1) & 0xf) << 8)
                | (((immediate >> 5) & 0x3f) << 25)
                | (((immediate >> 12) & 1) << 31)
        };
        // ld x3,0(x1); addi x1,x1,8; addi x2,x2,-1; bne x2,x0,-12
        let words = [0x0000_b183u32, 0x0080_8093, 0xfff1_0113, branch(2, 0, -12)];
        let code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let lifted = crate::lift::lift_t1(
            &code,
            0x1000,
            0x1000,
            true,
            crate::lift::FpMode::User,
            false,
        )
        .unwrap();
        assert!(lifted.loop_backedge.is_some());
        let mut layout = JitLayout::bare();
        layout.mem = Some((4096, 65536));
        layout.fuel_addr = 272;
        let bytes = emit(&lifted.ir, layout, lifted.loop_backedge).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated effectful loop module must validate");
    }

    #[test]
    fn emits_valid_full_system_memory_for_side_exit_and_refill_misses() {
        let branch = |rs1: u32, rs2: u32, offset: i32| {
            let immediate = offset as u32 & 0x1fff;
            0x63 | (1 << 12)
                | (rs1 << 15)
                | (rs2 << 20)
                | (((immediate >> 11) & 1) << 7)
                | (((immediate >> 1) & 0xf) << 8)
                | (((immediate >> 5) & 0x3f) << 25)
                | (((immediate >> 12) & 1) << 31)
        };
        // ld x3,0(x1); addi x1,x1,8; addi x2,x2,-1; bne x2,x0,-12
        let words = [0x0000_b183u32, 0x0080_8093, 0xfff1_0113, branch(2, 0, -12)];
        let code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let lifted = crate::lift::lift_t1(
            &code,
            0x1000,
            0x1000,
            true,
            crate::lift::FpMode::Disabled,
            false,
        )
        .unwrap();
        assert!(lifted.loop_backedge.is_some());
        for refill in [false, true] {
            let bytes = emit(&lifted.ir, system_layout(refill), lifted.loop_backedge).unwrap();
            wasmparser::Validator::new()
                .validate_all(&bytes)
                .unwrap_or_else(|error| panic!("system module (refill={refill}) failed: {error}"));
        }
    }

    #[test]
    fn emits_valid_full_system_fp_state_memory_and_helper_ordering() {
        let branch = |rs1: u32, rs2: u32, offset: i32| {
            let immediate = offset as u32 & 0x1fff;
            0x63 | (1 << 12)
                | (rs1 << 15)
                | (rs2 << 20)
                | (((immediate >> 11) & 1) << 7)
                | (((immediate >> 1) & 0xf) << 8)
                | (((immediate >> 5) & 0x3f) << 25)
                | (((immediate >> 12) & 1) << 31)
        };
        // fld f1,0(x1); fadd.d f1,f1,f2,rne; fsd f1,0(x1);
        // addi x3,x3,-1; bne x3,x0,-16
        let words = [
            0x0000_b087u32,
            0x0220_80d3,
            0x0010_b027,
            0xfff1_8193,
            branch(3, 0, -16),
        ];
        let code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let lifted = crate::lift::lift_t1(
            &code,
            0x1000,
            0x1000,
            true,
            crate::lift::FpMode::System,
            false,
        )
        .unwrap();
        assert!(lifted.loop_backedge.is_some());
        assert!(lifted
            .ir
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::FpState { dirty: false, .. })));
        let mut layout = system_layout(true);
        layout.f_base = 1024;
        layout.fcsr_addr = 1280;
        layout.mstatus_addr = 1288;
        let bytes = emit(&lifted.ir, layout, lifted.loop_backedge).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated system FP/memory module must validate");
    }

    #[test]
    fn emits_a_valid_lr_sc_loop_with_a_typed_reservation_import() {
        let branch = |rs1: u32, rs2: u32, offset: i32| {
            let immediate = offset as u32 & 0x1fff;
            0x63 | (1 << 12)
                | (rs1 << 15)
                | (rs2 << 20)
                | (((immediate >> 11) & 1) << 7)
                | (((immediate >> 1) & 0xf) << 8)
                | (((immediate >> 5) & 0x3f) << 25)
                | (((immediate >> 12) & 1) << 31)
        };
        let amo = |funct5: u32, rd: u32, rs1: u32, rs2: u32| {
            0x2f | (rd << 7) | (3 << 12) | (rs1 << 15) | (rs2 << 20) | (funct5 << 27)
        };
        // lr.d x3,(x1); addi x3,x3,1; sc.d x4,x3,(x1);
        // addi x2,x2,-1; bne x2,x0,-16
        let words = [
            amo(0x02, 3, 1, 0),
            0x0011_8193,
            amo(0x03, 4, 1, 3),
            0xfff1_0113,
            branch(2, 0, -16),
        ];
        let code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let lifted =
            crate::lift::lift_t1(&code, 0x1000, 0x1000, true, crate::lift::FpMode::User, true)
                .unwrap();
        assert!(lifted.loop_backedge.is_some());
        assert!(lifted.ir.has_reservation_helper());
        let mut layout = JitLayout::bare();
        layout.mem = Some((4096, 65536));
        layout.reservation = Some(ReservationCapability::User);
        layout.fuel_addr = 272;
        let bytes = emit(&lifted.ir, layout, lifted.loop_backedge).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated LR/SC module must validate");
    }

    #[test]
    fn emits_a_guarded_trace_without_requiring_guest_memory() {
        // addi x1,x0,1; beq x2,x0,+8; addi x1,x1,2; ecall
        let words = [0x0010_0093u32, 0x0001_0463, 0x0020_8093, 0x0000_0073];
        let code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let lifted = crate::lift::lift_t1(
            &code,
            0x1000,
            0x1000,
            false,
            crate::lift::FpMode::Disabled,
            false,
        )
        .unwrap();
        assert!(lifted.ir.has_effects());
        let bytes = emit(&lifted.ir, JitLayout::bare(), None).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated guarded trace module must validate");
    }

    #[test]
    fn refuses_effectful_regions_without_a_memory_capability() {
        let mut builder = Builder::new(0x1000);
        let address = builder.read_x(1, 0x1000);
        let loaded = builder.load(address, LoadKind::I64, 0x1000, 0);
        builder.write_x(2, loaded);
        let next = builder.const_i64(0x1004, 0x1000);
        let region = builder.finish(0x1004, next, 1, ExitKind::Dispatch);
        assert!(emit(&region, JitLayout::bare(), None).is_err());
    }

    #[test]
    fn emits_valid_non_trapping_division_paths() {
        let mut builder = Builder::new(0x1000);
        let lhs = builder.read_x(1, 0x1000);
        let rhs = builder.read_x(2, 0x1000);
        let div = builder.divide(DivideOp::I64DivS, lhs, rhs, 0x1000);
        let rem = builder.divide(DivideOp::I64RemU, lhs, rhs, 0x1004);
        builder.write_x(3, div);
        builder.write_x(4, rem);
        let lhs32 = builder.wrap_i32(lhs, 0x1008);
        let rhs32 = builder.wrap_i32(rhs, 0x1008);
        let div32 = builder.divide(DivideOp::I32DivU, lhs32, rhs32, 0x1008);
        let div32 = builder.extend_i32_s(div32, 0x1008);
        builder.write_x(5, div32);
        let next = builder.const_i64(0x100c, 0x1008);
        let region = builder.finish(0x100c, next, 3, ExitKind::Dispatch);
        let bytes = emit(&region, JitLayout::bare(), None).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated guarded division module must validate");
    }

    #[test]
    fn emits_valid_fp_register_and_memory_state() {
        // fmv.d.x f1,x2; fsd f1,0(x3); fld f4,0(x3); fmv.x.d x5,f4; ecall
        let words = [
            0xf201_00d3u32,
            0x0011_b027,
            0x0001_b207,
            0xe202_82d3,
            0x0000_0073,
        ];
        let code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let lifted = crate::lift::lift_t1(
            &code,
            0x1000,
            0x1000,
            true,
            crate::lift::FpMode::User,
            false,
        )
        .unwrap();
        let mut layout = JitLayout::bare();
        layout.f_base = 1024;
        layout.fcsr_addr = 1280;
        layout.mem = Some((4096, 65536));
        let bytes = emit(&lifted.ir, layout, lifted.loop_backedge).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated FP state module must validate");
    }

    #[test]
    fn emits_a_valid_exact_fp_helper_call() {
        let words = [0x0220_81d3u32, 0x0000_0073]; // fadd.d f3,f1,f2,rne; ecall
        let code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let lifted = crate::lift::lift_t1(
            &code,
            0x1000,
            0x1000,
            true,
            crate::lift::FpMode::User,
            false,
        )
        .unwrap();
        let mut layout = JitLayout::bare();
        layout.f_base = 1024;
        layout.fcsr_addr = 1280;
        let bytes = emit(&lifted.ir, layout, lifted.loop_backedge).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated exact FP helper module must validate");
    }

    #[test]
    fn emits_a_valid_fp_helper_loop_with_carried_fcsr() {
        let branch = |rs1: u32, rs2: u32, offset: i32| {
            let immediate = offset as u32 & 0x1fff;
            0x63 | (1 << 12)
                | (rs1 << 15)
                | (rs2 << 20)
                | (((immediate >> 11) & 1) << 7)
                | (((immediate >> 1) & 0xf) << 8)
                | (((immediate >> 5) & 0x3f) << 25)
                | (((immediate >> 12) & 1) << 31)
        };
        // fadd.d f1,f1,f2,rne; addi x3,x3,-1; bne x3,x0,-8
        let words = [0x0220_80d3u32, 0xfff1_8193, branch(3, 0, -8)];
        let code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let lifted = crate::lift::lift_t1(
            &code,
            0x1000,
            0x1000,
            true,
            crate::lift::FpMode::User,
            false,
        )
        .unwrap();
        assert!(lifted.loop_backedge.is_some());
        let mut layout = JitLayout::bare();
        layout.f_base = 1024;
        layout.fcsr_addr = 1280;
        layout.fuel_addr = 1288;
        let bytes = emit(&lifted.ir, layout, lifted.loop_backedge).unwrap();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("generated exact FP loop module must validate");
    }
}
