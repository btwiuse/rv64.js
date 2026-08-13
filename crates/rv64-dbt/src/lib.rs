//! Clean-room RV64 to WebAssembly dynamic binary translator.
//!
//! It provides the typed IR, RV64 lifter, bounded T1 trace/loop builder, T2
//! multi-entry region builder, and WebAssembly emitter used by the runtime.
//! Precise exits retain the interpreter as the correctness fallback. No source
//! from the deleted `rv64-jit` crate is used here.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub mod ir;
mod lift;
mod structure;
mod wasm;

/// Marker historically carried in host table indices. The compatibility host
/// still needs a non-overlapping value until its dispatcher is replaced.
pub const SB_IDX_BIT: i32 = 1 << 30;

/// One direct-mapped translation row owned by the architectural CPU. A tag
/// hit proves that the complete guest page is directly accessible in Wasm
/// linear memory and `linear_address = guest_address + offsets[index]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TranslationRow {
    pub tags: u32,
    pub offsets: u32,
}

/// Behavior on a fused-translation-row miss.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlbMissPolicy {
    /// Materialize the precise pre-instruction state and let T0 perform the
    /// access. A successful interpreter access warms the row for next time.
    SideExit,
    /// Call the typed Wasm-to-Wasm `tlb_fill` capability, re-probe the row,
    /// and continue only if the runtime published a direct-RAM translation.
    Refill,
}

/// Concrete machine layout whose canonical CPU reservation state a generated
/// LR/SC helper may update. The distinction prevents an opaque state pointer
/// from ever being cast to the wrong Rust machine type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReservationCapability {
    User,
    System,
}

/// Concrete machine whose canonical CPU and bus a generated RVV effect may
/// access. Keeping this explicit prevents an opaque state pointer from being
/// cast to the wrong Rust machine type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorCapability {
    User,
    System,
}

/// Canonical RVV state cells exposed to generated modules. These are concrete
/// Wasm-linear-memory addresses for one live machine, just like `x_base` and
/// `f_base`; the typed helper remains available whenever a direct lowering
/// guard does not hold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VectorStateLayout {
    pub regs_base: u32,
    pub vl_addr: u32,
    pub vtype_addr: u32,
    pub vstart_addr: u32,
    /// Architecture-independent diagnostic counter incremented once per RVV
    /// instruction completed by generated SIMD rather than the helper.
    pub simd_count_addr: u32,
}

/// Architectural-state strategy for a multi-entry generated function.
///
/// All variants preserve the same precise-exit contract. They exist so region
/// policy can measure the embedding engine's trade-off between entry traffic,
/// Wasm locals/branches, and time spent inside one invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MultiEntryState {
    /// Load the complete register live-in union once, retain it in locals, and
    /// commit only registers that some member can write.
    RegisterEager,
    /// Allocate the same bounded register union, but load each register on its
    /// first dynamic read and commit it only after it became valid. This favors
    /// sparse/short paths through a large region at the cost of validity tests.
    RegisterLazy,
    /// Keep eager register residency, but represent the member set as a
    /// structured dense switch. Statically known covered successors set the
    /// next member index directly and bypass the PC comparison tree.
    RegisterDirect,
    /// Lift architectural basic blocks instead of guarded traces and route
    /// both covered conditional successors through the dense switch.
    RegisterCfg,
    /// Lift architectural basic blocks and reconstruct reducible control flow
    /// with nested Wasm blocks and loops. Dispatchers remain only at function
    /// entry and around multi-entry SCCs that exceed the duplication budget.
    /// Keep RV64C's fixed x1/x2/x8--x15 integer bank resident and materialize
    /// other integer registers at member boundaries to bound function-wide
    /// local pressure.
    RegisterStructured,
    /// Materialize architectural state at every member boundary.
    Memory,
    /// Materialize at member boundaries but connect statically known covered
    /// successors with the Wasm tail-call proposal. This is an optional engine
    /// experiment, not part of the portable default.
    MemoryTailCall,
}

/// Full-system memory capability supplied by the runtime.
///
/// Load and store rows are distinct because their permission proofs differ.
/// Store entries additionally prove that the physical page does not currently
/// hold compiled guest code, preserving self-modifying-code invalidation.
/// Cross-page accesses never consume a row and return to T0 for the CPU's
/// split-access semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemMemory {
    pub load: TranslationRow,
    pub store: TranslationRow,
    /// Address of the runtime-owned u64 permission-context tag. Generated
    /// code snapshots it once on entry and folds it into every row probe.
    pub context_addr: u32,
    pub index_mask: u32,
    /// Fold the virtual page number by this many bits before masking the
    /// direct-mapped row index. Zero selects the conventional low-VPN index.
    pub index_hash_shift: u8,
    pub page_shift: u8,
    pub miss: TlbMissPolicy,
    /// A long-running multi-entry invocation may retain one proven load and
    /// store translation by guest page in dense direct-copy members.
    /// Context/mapping changes remain precise because compiled code exits
    /// before privileged invalidation operations.
    pub cache_within_invocation: bool,
    /// Minimum same-width load-to-store pairs in one member before emitting
    /// both page caches. A singleton access cannot reuse a translation, while
    /// general memory members regress from the extra comparison on some Wasm
    /// engines. Unrolled memory-copy members cross this threshold widely.
    pub cache_min_accesses: u16,
}

impl SystemMemory {
    pub const fn fused_4k(
        load_tags: u32,
        load_offsets: u32,
        store_tags: u32,
        store_offsets: u32,
        context_addr: u32,
        index_mask: u32,
        refill_on_miss: bool,
    ) -> Self {
        Self {
            load: TranslationRow {
                tags: load_tags,
                offsets: load_offsets,
            },
            store: TranslationRow {
                tags: store_tags,
                offsets: store_offsets,
            },
            context_addr,
            index_mask,
            index_hash_shift: 0,
            page_shift: 12,
            miss: if refill_on_miss {
                TlbMissPolicy::Refill
            } else {
                TlbMissPolicy::SideExit
            },
            cache_within_invocation: false,
            cache_min_accesses: 2,
        }
    }

    pub const fn with_invocation_cache(mut self, enabled: bool) -> Self {
        self.cache_within_invocation = enabled;
        self
    }

    pub const fn with_index_hash_shift(mut self, shift: u8) -> Self {
        self.index_hash_shift = shift;
        self
    }

    pub const fn with_invocation_cache_min_accesses(mut self, accesses: u16) -> Self {
        self.cache_min_accesses = if accesses == 0 { 1 } else { accesses };
        self
    }
}

/// Temporary layout adapter for the surviving Wasm orchestration layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JitLayout {
    pub x_base: u32,
    pub pc_addr: u32,
    pub mem: Option<(u32, u64)>,
    pub sys: Option<SystemMemory>,
    pub mem_profile: Option<[u32; 17]>,
    pub reg_stress: bool,
    pub reg_profile_base: u32,
    /// Opt-in structured-member execution counters: member entries, scheduled
    /// guest instructions, five scheduled instruction-mix buckets, stack
    /// attribution. These diagnostic counters are emitted only when the
    /// embedding supplies concrete cells; ordinary production modules remain
    /// byte-for-byte free of the instrumentation.
    pub structured_profile: Option<[u32; 9]>,
    pub multi_latch: bool,
    pub retired_addr: u32,
    pub f_base: u32,
    pub fcsr_addr: u32,
    /// Typed canonical reservation-state helper for LR/SC, if this concrete
    /// machine layout supports one.
    pub reservation: Option<ReservationCapability>,
    /// Typed one-instruction RVV executor for this concrete machine layout.
    pub vector: Option<VectorCapability>,
    /// Direct access to canonical vector state. `None` keeps the exact helper
    /// lowering and is valid for embedders that intentionally expose only the
    /// opaque vector capability.
    pub vector_state: Option<VectorStateLayout>,
    pub fuel_addr: u32,
    pub mstatus_addr: u32,
    pub copystat_addr: u32,
    pub chain_off_addr: u32,
    pub batch_base_addr: u32,
    pub dispatch_base: u32,
    pub dispatch_mask: u32,
    pub map_gen_addr: u32,
    /// Counter incremented before a generated cross-module tail transfer.
    pub chain_hops_addr: u32,
    /// Optional cold-path feedback cells. A failed monomorphic indirect guard
    /// writes its owning entry and actual target before returning to T0.
    pub ic_miss_owner_addr: u32,
    pub ic_miss_target_addr: u32,
    /// Emit architectural PC-derived values relative to this guest code page.
    /// Such modules import an immutable `env.guest_base` i64 and therefore
    /// have identical bytes for identical code mapped at another page-aligned
    /// virtual address. `None` preserves the ordinary absolute encoding.
    pub pic_code_base: Option<u64>,
}

impl JitLayout {
    pub const fn bare() -> Self {
        Self {
            x_base: 0,
            pc_addr: 32 * 8,
            mem: None,
            sys: None,
            mem_profile: None,
            reg_stress: false,
            reg_profile_base: 0,
            structured_profile: None,
            multi_latch: false,
            retired_addr: 33 * 8,
            f_base: 0,
            fcsr_addr: 0,
            reservation: None,
            vector: None,
            vector_state: None,
            fuel_addr: 0,
            mstatus_addr: 0,
            copystat_addr: 0,
            chain_off_addr: 0,
            batch_base_addr: 0,
            dispatch_base: 0,
            dispatch_mask: 0,
            map_gen_addr: 0,
            chain_hops_addr: 0,
            ic_miss_owner_addr: 0,
            ic_miss_target_addr: 0,
            pic_code_base: None,
        }
    }
}

/// Output of one independently compiled region.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompiledRegion {
    pub wasm: Vec<u8>,
    pub n_insns: u32,
    pub uses_fp: bool,
    /// Number of members emitted with the range-checked dense-copy lowering.
    pub dense_copy_members: u16,
    /// Number of members emitted with the range-checked dense-store lowering.
    pub dense_store_members: u16,
    /// Number of members emitted with the proved whole-loop copy lowering.
    pub bulk_copy_members: u16,
    pub span: (u64, u64),
    pub len: u64,
    pub seeds: Vec<u64>,
    /// Actual lifted entry PCs represented by the emitted function. Runtime
    /// caches use this instead of assuming every requested seed survived
    /// decode and validation.
    pub entries: Vec<u64>,
    pub trace_mix: [u16; 5],
    pub trace_mem: [u16; 10],
    pub trace_control: [u16; 3],
    pub trace_alu: [u16; 5],
}

/// Metadata for one member of a batched Wasm module.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BatchMember {
    pub pc: u64,
    pub n_insns: u32,
    pub uses_fp: bool,
    /// Whether this member uses the range-checked dense-copy lowering.
    pub dense_copy: bool,
    /// Whether this member uses the range-checked dense-store lowering.
    pub dense_store: bool,
    /// Whether this member uses the proved whole-loop copy lowering.
    pub bulk_copy: bool,
    pub span: (u64, u64),
    pub seeds: Vec<u64>,
    pub trace_mix: [u16; 5],
    pub trace_mem: [u16; 10],
    pub trace_control: [u16; 3],
    pub trace_alu: [u16; 5],
}

fn fp_mode(layout: JitLayout) -> lift::FpMode {
    if layout.mem.is_some() {
        lift::FpMode::User
    } else if layout.sys.is_some() && layout.mstatus_addr != 0 {
        lift::FpMode::System
    } else {
        lift::FpMode::Disabled
    }
}

// A translation that cannot satisfy the typed layout or precise-exit contract
// returns `None`, leaving the runtime to execute that PC in the interpreter.

pub fn translate_block(
    code: &[u8],
    base: u64,
    pc: u64,
    layout: JitLayout,
) -> Option<CompiledRegion> {
    let lifted = lift::lift_t1_with_vector(
        code,
        base,
        pc,
        layout.mem.is_some() || layout.sys.is_some(),
        fp_mode(layout),
        layout.reservation.is_some(),
        layout.vector.is_some(),
    )?;
    let wasm = wasm::emit(&lifted.ir, layout, lifted.loop_backedge).ok()?;
    Some(CompiledRegion {
        wasm,
        n_insns: lifted.ir.retired,
        uses_fp: lifted.metrics.uses_fp,
        dense_copy_members: u16::from(wasm::uses_dense_copy_plan(&lifted.ir, layout)),
        dense_store_members: u16::from(wasm::uses_dense_store_plan(&lifted.ir, layout)),
        bulk_copy_members: u16::from(wasm::uses_bulk_copy_loop_plan(&lifted.ir, layout)),
        span: lifted.span,
        len: lifted.byte_len,
        seeds: lifted.seeds,
        entries: vec![lifted.ir.entry_pc],
        trace_mix: lifted.metrics.trace_mix,
        trace_mem: lifted.metrics.trace_mem,
        trace_control: lifted.metrics.trace_control,
        trace_alu: lifted.metrics.trace_alu,
    })
}

pub fn translate_block_ic<FHot, FDirect, FIndirect>(
    code: &[u8],
    base: u64,
    pc: u64,
    layout: JitLayout,
    hot: &FHot,
    direct: &FDirect,
    indirect: &FIndirect,
) -> Option<CompiledRegion>
where
    FHot: Fn(u64) -> bool,
    FDirect: Fn(u64) -> Option<u64>,
    FIndirect: Fn(u64) -> Option<u64>,
{
    let mut targets = Vec::new();
    let mut current = pc;
    for _ in 0..8 {
        let Some(target) = indirect(current)
            .or_else(|| direct(current))
            .filter(|&target| hot(target))
        else {
            break;
        };
        targets.push(target);
        current = target;
        if target == pc {
            break;
        }
    }
    let lifted = lift_at_profiled_targets(code, base, pc, layout, &targets)?;
    compiled_from_lifted(
        vec![lifted],
        layout,
        false,
        false,
        MultiEntryState::RegisterEager,
    )
}

/// Compile one indirect source together with up to two independently lifted
/// target entries. The source keeps its computed PC (no speculative guard),
/// and the register-resident dispatcher routes either observed target inside
/// the module. Any other target leaves precisely through the common exit.
pub fn translate_block_pic(
    code: &[u8],
    base: u64,
    pc: u64,
    layout: JitLayout,
    targets: &[u64],
) -> Option<CompiledRegion> {
    let mut lifted = vec![lift_at(code, base, pc, layout)?];
    let mut seen = BTreeSet::from([pc]);
    for &target in targets.iter().take(2) {
        if seen.insert(target) {
            if let Some(region) = lift_at(code, base, target, layout) {
                lifted.push(region);
            }
        }
    }
    compiled_from_lifted(lifted, layout, false, true, MultiEntryState::RegisterEager)
}

/// Sparse-page form of [`translate_block_pic`]. `code` is a concatenation of
/// complete 4 KiB pages corresponding one-for-one with `page_vas`. This lets a
/// rare PIC upgrade capture exactly its source and target pages without making
/// every ordinary T1 translation copy a wide virtual-address window.
pub fn translate_block_pic_sparse(
    code: &[u8],
    page_vas: &[u64],
    pc: u64,
    layout: JitLayout,
    targets: &[u64],
) -> Option<CompiledRegion> {
    let mut entries = Vec::with_capacity(3);
    entries.push(pc);
    for &target in targets.iter().take(2) {
        if !entries.contains(&target) {
            entries.push(target);
        }
    }
    translate_superblock_sparse(code, page_vas, &entries, layout, false)
}

pub fn translate_superblock(
    code: &[u8],
    base: u64,
    lo: u64,
    span: u64,
    entries: &[u64],
    layout: JitLayout,
) -> Option<CompiledRegion> {
    let hi = lo.saturating_add(span);
    let bounded_len = usize::try_from(hi.saturating_sub(base))
        .ok()
        .map_or(code.len(), |len| len.min(code.len()));
    let bounded = &code[..bounded_len];
    let mut seen = BTreeSet::new();
    let lifted: Vec<_> = entries
        .iter()
        .copied()
        .filter(|pc| *pc >= lo && *pc < hi && seen.insert(*pc))
        .filter_map(|pc| lift_at(bounded, base, pc, layout))
        .collect();
    compiled_from_lifted(lifted, layout, false, true, MultiEntryState::RegisterEager)
}

pub fn translate_superblock_sparse(
    code: &[u8],
    page_vas: &[u64],
    entries: &[u64],
    layout: JitLayout,
    registers_in_memory: bool,
) -> Option<CompiledRegion> {
    translate_superblock_sparse_state(
        code,
        page_vas,
        entries,
        layout,
        if registers_in_memory {
            MultiEntryState::Memory
        } else {
            MultiEntryState::RegisterEager
        },
    )
}

/// Translate a sparse multi-page region with an explicit architectural-state
/// strategy. The boolean compatibility wrapper above remains the production
/// API for existing callers; lifecycle/corpus experiments use this form so a
/// lazy-load candidate is compared from identical lifted regions and bytes.
pub fn translate_superblock_sparse_state(
    code: &[u8],
    page_vas: &[u64],
    entries: &[u64],
    layout: JitLayout,
    state: MultiEntryState,
) -> Option<CompiledRegion> {
    const PAGE_SIZE: usize = 4096;
    if page_vas.is_empty() || code.len() < page_vas.len().checked_mul(PAGE_SIZE)? {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut lifted = Vec::new();
    for &entry in entries {
        if !seen.insert(entry) {
            continue;
        }
        let page = entry & !0xfff;
        let Some(mut first) = page_vas.iter().position(|&va| va == page) else {
            continue;
        };
        let mut last = first;
        while first > 0 && page_vas[first - 1].checked_add(0x1000) == Some(page_vas[first]) {
            first -= 1;
        }
        while last + 1 < page_vas.len()
            && page_vas[last].checked_add(0x1000) == Some(page_vas[last + 1])
        {
            last += 1;
        }
        let bytes = &code[first * PAGE_SIZE..(last + 1) * PAGE_SIZE];
        let region = if matches!(
            state,
            MultiEntryState::RegisterCfg | MultiEntryState::RegisterStructured
        ) {
            lift_block_at(bytes, page_vas[first], entry, layout)
        } else {
            lift_at(bytes, page_vas[first], entry, layout)
        };
        if let Some(region) = region {
            lifted.push(region);
        }
    }
    compiled_from_lifted(lifted, layout, false, true, state)
}

#[allow(clippy::too_many_arguments)]
pub fn translate_batch_obs<FHot, FWant, FNext>(
    code: &[u8],
    base: u64,
    pc: u64,
    layout: JitLayout,
    hot: &FHot,
    want: &FWant,
    next: &FNext,
    capacity: usize,
) -> Option<(Vec<u8>, Vec<BatchMember>)>
where
    FHot: Fn(u64) -> bool,
    FWant: Fn(u64) -> bool,
    FNext: Fn(u64) -> Option<u64>,
{
    if capacity < 2 {
        return None;
    }
    let mut pcs = vec![pc];
    let mut seen = BTreeSet::from([pc]);
    let mut cursor = 0;
    while cursor < pcs.len() && pcs.len() < capacity {
        let source = pcs[cursor];
        cursor += 1;
        let Some(target) = next(source) else { continue };
        if want(target) && seen.insert(target) {
            // Existing compiled targets are placed immediately after their
            // predecessor. Warm-but-not-yet-compiled targets are still valid
            // members; `hot` is consumed as a priority signal for future
            // polymorphic expansion rather than a correctness predicate.
            let _already_compiled = hot(target);
            pcs.push(target);
        }
    }
    if pcs.len() < 2 {
        return None;
    }

    let lifted: Vec<_> = pcs
        .iter()
        .copied()
        .filter_map(|entry| {
            let mut targets = Vec::new();
            let mut current = entry;
            for _ in 0..capacity.min(8) {
                let Some(target) = next(current) else { break };
                if !pcs.contains(&target) {
                    break;
                }
                targets.push(target);
                current = target;
                if target == entry {
                    break;
                }
            }
            lift_at_profiled_targets(code, base, entry, layout, &targets)
        })
        .collect();
    if lifted.len() < 2 {
        return None;
    }
    let members: Vec<BatchMember> = lifted
        .iter()
        .map(|region| batch_member(region, layout))
        .collect();
    let compiled =
        compiled_from_lifted(lifted, layout, true, false, MultiEntryState::RegisterEager)?;
    Some((compiled.wasm, members))
}

fn lift_at(code: &[u8], base: u64, pc: u64, layout: JitLayout) -> Option<lift::LiftedRegion> {
    lift::lift_t1_with_vector(
        code,
        base,
        pc,
        layout.mem.is_some() || layout.sys.is_some(),
        fp_mode(layout),
        layout.reservation.is_some(),
        layout.vector.is_some(),
    )
}

fn lift_block_at(code: &[u8], base: u64, pc: u64, layout: JitLayout) -> Option<lift::LiftedRegion> {
    lift::lift_basic_block_with_vector(
        code,
        base,
        pc,
        layout.mem.is_some() || layout.sys.is_some(),
        fp_mode(layout),
        layout.reservation.is_some(),
        layout.vector.is_some(),
    )
}

fn lift_at_profiled_targets(
    code: &[u8],
    base: u64,
    pc: u64,
    layout: JitLayout,
    profiled_indirect_targets: &[u64],
) -> Option<lift::LiftedRegion> {
    lift::lift_t1_profiled_targets_with_vector(
        code,
        base,
        pc,
        layout.mem.is_some() || layout.sys.is_some(),
        fp_mode(layout),
        layout.reservation.is_some(),
        layout.vector.is_some(),
        profiled_indirect_targets,
    )
}

fn compiled_from_lifted(
    lifted: Vec<lift::LiftedRegion>,
    layout: JitLayout,
    export_members: bool,
    dynamic_region: bool,
    state: MultiEntryState,
) -> Option<CompiledRegion> {
    let first = lifted.first()?;
    let wasm = if lifted.len() == 1 {
        wasm::emit(&first.ir, layout, first.loop_backedge).ok()?
    } else {
        let refs: Vec<_> = lifted
            .iter()
            .map(|region| (&region.ir, region.loop_backedge))
            .collect();
        match state {
            MultiEntryState::RegisterEager => {
                wasm::emit_multi_entry(&refs, layout, export_members).ok()?
            }
            _ => wasm::emit_multi_entry_mode(&refs, layout, export_members, state).ok()?,
        }
    };

    let mut out = CompiledRegion {
        wasm,
        n_insns: if dynamic_region {
            0
        } else {
            lifted.iter().map(|region| region.ir.retired).sum()
        },
        uses_fp: lifted.iter().any(|region| region.metrics.uses_fp),
        dense_copy_members: lifted
            .iter()
            .filter(|region| wasm::uses_dense_copy_plan(&region.ir, layout))
            .count()
            .try_into()
            .unwrap_or(u16::MAX),
        dense_store_members: lifted
            .iter()
            .filter(|region| wasm::uses_dense_store_plan(&region.ir, layout))
            .count()
            .try_into()
            .unwrap_or(u16::MAX),
        bulk_copy_members: lifted
            .iter()
            .filter(|region| wasm::uses_bulk_copy_loop_plan(&region.ir, layout))
            .count()
            .try_into()
            .unwrap_or(u16::MAX),
        span: (
            lifted.iter().map(|region| region.span.0).min()?,
            lifted.iter().map(|region| region.span.1).max()?,
        ),
        len: 0,
        seeds: {
            let mut seeds = Vec::new();
            for region in &lifted {
                for &seed in &region.seeds {
                    if !seeds.contains(&seed) {
                        seeds.push(seed);
                    }
                }
            }
            seeds
        },
        entries: lifted.iter().map(|region| region.ir.entry_pc).collect(),
        ..CompiledRegion::default()
    };
    out.len = out.span.1.saturating_sub(out.span.0);
    for region in &lifted {
        add_metrics(&mut out, &region.metrics);
    }
    Some(out)
}

fn batch_member(region: &lift::LiftedRegion, layout: JitLayout) -> BatchMember {
    BatchMember {
        pc: region.ir.entry_pc,
        n_insns: region.ir.retired,
        uses_fp: region.metrics.uses_fp,
        dense_copy: wasm::uses_dense_copy_plan(&region.ir, layout),
        dense_store: wasm::uses_dense_store_plan(&region.ir, layout),
        bulk_copy: wasm::uses_bulk_copy_loop_plan(&region.ir, layout),
        span: region.span,
        seeds: region.seeds.clone(),
        trace_mix: region.metrics.trace_mix,
        trace_mem: region.metrics.trace_mem,
        trace_control: region.metrics.trace_control,
        trace_alu: region.metrics.trace_alu,
    }
}

fn add_metrics(out: &mut CompiledRegion, metrics: &lift::Metrics) {
    for (dst, src) in out.trace_mix.iter_mut().zip(metrics.trace_mix) {
        *dst = dst.saturating_add(src);
    }
    for (dst, src) in out.trace_mem.iter_mut().zip(metrics.trace_mem) {
        *dst = dst.saturating_add(src);
    }
    for (dst, src) in out.trace_control.iter_mut().zip(metrics.trace_control) {
        *dst = dst.saturating_add(src);
    }
    for (dst, src) in out.trace_alu.iter_mut().zip(metrics.trace_alu) {
        *dst = dst.saturating_add(src);
    }
}

pub fn page_call_targets(code: &[u8], base: u64) -> Vec<u64> {
    lift::direct_call_targets(code, base)
}

pub fn discover_page_leaders(
    code: &[u8],
    base: u64,
    lo: u64,
    span: u64,
    seeds: &[u64],
    capacity: usize,
) -> Vec<u64> {
    lift::discover_leaders(code, base, lo, span, seeds, capacity).0
}

pub fn discover_page_leaders_ext(
    code: &[u8],
    base: u64,
    lo: u64,
    span: u64,
    seeds: &[u64],
    capacity: usize,
) -> (Vec<u64>, BTreeSet<u64>) {
    let (leaders, backedges, _) = lift::discover_leaders(code, base, lo, span, seeds, capacity);
    (leaders, backedges)
}

/// Return the virtual code pages reached by the same bounded direct-CFG walk
/// used for page-leader discovery. Callers use this to decide whether adding a
/// neighbouring code snapshot can actually keep an edge inside a region;
/// merely being adjacent is not sufficient evidence.
pub fn discover_page_reachable_pages(
    code: &[u8],
    base: u64,
    lo: u64,
    span: u64,
    seeds: &[u64],
    capacity: usize,
) -> BTreeSet<u64> {
    lift::discover_leaders(code, base, lo, span, seeds, capacity).2
}

/// Return pages reachable without treating a cross-page direct call as a
/// reason to merge caller and callee into one Wasm function. This retains
/// page-straddling branches, tail jumps, and ordinary fallthrough.
pub fn discover_page_reachable_noncall_pages(
    code: &[u8],
    base: u64,
    lo: u64,
    span: u64,
    seeds: &[u64],
    capacity: usize,
) -> BTreeSet<u64> {
    lift::discover_leaders_without_cross_page_calls(code, base, lo, span, seeds, capacity).2
}

pub fn emittable_at(code: &[u8], base: u64, pc: u64, layout: JitLayout) -> bool {
    lift::lift_t1_with_vector(
        code,
        base,
        pc,
        layout.mem.is_some() || layout.sys.is_some(),
        fp_mode(layout),
        layout.reservation.is_some(),
        layout.vector.is_some(),
    )
    .is_some()
}

pub fn is_loop_at(code: &[u8], base: u64, pc: u64, layout: JitLayout) -> bool {
    lift::lift_t1_with_vector(
        code,
        base,
        pc,
        layout.mem.is_some() || layout.sys.is_some(),
        fp_mode(layout),
        layout.reservation.is_some(),
        layout.vector.is_some(),
    )
    .is_some_and(|lifted| lifted.loop_backedge.is_some())
}

/// Proof-only per-member shape used by architecture-general backend models.
/// This contains no execution profile, symbol, workload, or policy decision;
/// callers receive the same basic-block IR that RegisterStructured consumes.
#[cfg(feature = "r111-diagnostics")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberShape {
    pub entry_pc: u64,
    pub read_x: u32,
    pub write_x: u32,
    pub read_f: u32,
    pub write_f: u32,
    pub read_fcsr: bool,
    pub write_fcsr: bool,
    pub i32_values: u32,
    pub i64_values: u32,
    pub retired: u32,
    pub successors: Vec<u64>,
}

#[cfg(feature = "r111-diagnostics")]
fn diagnostic_static_guest_pc(region: &ir::Region, value: ir::ValueId) -> Option<u64> {
    match region.values.get(value.0)?.op {
        ir::Op::GuestPc(pc) => Some(pc),
        ir::Op::ConstI64(pc) => Some(pc as u64),
        _ => None,
    }
}

/// Lift the requested entries as production structured-CFG basic blocks and
/// return only static resource/edge facts. This API is consumed by examples
/// and tests; it does not alter emission or the runtime selection path.
#[cfg(feature = "r111-diagnostics")]
pub fn scan_basic_block_shapes_pub(
    code: &[u8],
    base: u64,
    end: u64,
    entries: &[u64],
    layout: &JitLayout,
) -> Vec<MemberShape> {
    let mut shapes = Vec::new();
    for &entry_pc in entries {
        if entry_pc < base || entry_pc >= end {
            continue;
        }
        let Some(lifted) = lift_block_at(code, base, entry_pc, *layout) else {
            continue;
        };
        let region = &lifted.ir;
        let mut read_x = 0u32;
        let mut read_f = 0u32;
        let mut read_fcsr = false;
        let mut i32_values = 0u32;
        let mut i64_values = 0u32;
        for value in &region.values {
            match value.ty {
                ir::ValueType::I32 => i32_values += 1,
                ir::ValueType::I64 => i64_values += 1,
            }
            match value.op {
                ir::Op::ReadX(reg) => read_x |= 1u32 << reg,
                ir::Op::ReadF(reg) => read_f |= 1u32 << reg,
                ir::Op::ReadFcsr => read_fcsr = true,
                _ => {}
            }
        }
        let write_x = region
            .outputs
            .iter()
            .fold(0u32, |mask, &(reg, _)| mask | (1u32 << reg));
        let write_f = region
            .f_outputs
            .iter()
            .fold(0u32, |mask, &(reg, _)| mask | (1u32 << reg));

        let mut successors = Vec::with_capacity(2);
        if let Some(pc) = diagnostic_static_guest_pc(region, region.next_pc) {
            successors.push(pc);
        } else if let Some(ir::ValueData {
            op: ir::Op::SelectI64 {
                if_true, if_false, ..
            },
            ..
        }) = region.values.get(region.next_pc.0)
        {
            successors.extend(
                [*if_true, *if_false]
                    .into_iter()
                    .filter_map(|value| diagnostic_static_guest_pc(region, value)),
            );
        }
        successors.sort_unstable();
        successors.dedup();
        shapes.push(MemberShape {
            entry_pc,
            read_x,
            write_x,
            read_f,
            write_f,
            read_fcsr,
            write_fcsr: region.fcsr_output.is_some(),
            i32_values,
            i64_values,
            retired: region.retired,
            successors,
        });
    }
    shapes
}

pub fn scan_regs_super_pub(
    code: &[u8],
    base: u64,
    end: u64,
    entries: &[u64],
    layout: &JitLayout,
) -> (u32, u32, u32, u32) {
    let mut read_x = 0u32;
    let mut write_x = 0u32;
    let mut read_f = 0u32;
    let mut write_f = 0u32;
    for &entry in entries {
        if entry < base || entry >= end {
            continue;
        }
        let Some(lifted) = lift_at(code, base, entry, *layout) else {
            continue;
        };
        for value in &lifted.ir.values {
            match value.op {
                ir::Op::ReadX(reg) => read_x |= 1u32 << reg,
                ir::Op::ReadF(reg) => read_f |= 1u32 << reg,
                _ => {}
            }
        }
        for &(reg, _) in &lifted.ir.outputs {
            write_x |= 1u32 << reg;
        }
        for &(reg, _) in &lifted.ir.f_outputs {
            write_f |= 1u32 << reg;
        }
    }
    (read_x, write_x, read_f, write_f)
}

static HARDWARE_FMA: AtomicBool = AtomicBool::new(false);
static CHAINING: AtomicBool = AtomicBool::new(false);
static REGION_TAIL_CHAINING: AtomicBool = AtomicBool::new(false);
static DEFINED_TRACKING: AtomicBool = AtomicBool::new(false);
static ROTATED_NESTS: AtomicBool = AtomicBool::new(false);
static TLB_FILL: AtomicBool = AtomicBool::new(false);
static TRACE_LEVEL: AtomicU32 = AtomicU32::new(0);

pub fn set_hw_fma(on: bool) {
    HARDWARE_FMA.store(on, Ordering::Relaxed);
}

pub(crate) fn hardware_fma_enabled() -> bool {
    HARDWARE_FMA.load(Ordering::Relaxed)
}

pub fn set_chain(on: bool) {
    CHAINING.store(on, Ordering::Relaxed);
}

pub(crate) fn chain_enabled() -> bool {
    CHAINING.load(Ordering::Relaxed)
}

pub fn set_region_tail_chain(on: bool) {
    REGION_TAIL_CHAINING.store(on, Ordering::Relaxed);
}

pub fn region_tail_chain_enabled() -> bool {
    REGION_TAIL_CHAINING.load(Ordering::Relaxed)
}

pub fn set_defined_track(on: bool) {
    DEFINED_TRACKING.store(on, Ordering::Relaxed);
}

pub fn set_rotated_nests(on: bool) {
    ROTATED_NESTS.store(on, Ordering::Relaxed);
}

pub fn set_tlb_fill(on: bool) {
    TLB_FILL.store(on, Ordering::Relaxed);
}

pub fn tlb_fill_enabled() -> bool {
    TLB_FILL.load(Ordering::Relaxed)
}

pub fn set_trace_level(level: u32) {
    TRACE_LEVEL.store(level, Ordering::Relaxed);
}

pub fn trace_level() -> u32 {
    TRACE_LEVEL.load(Ordering::Relaxed)
}

/// Compact identity for every process-global option that can change emitted
/// Wasm without changing the input bytes or [`JitLayout`].  Embedders that
/// retain a compiled function for semantically identical guest code can use
/// this as part of the cache key.  It is deliberately an identity, not a
/// persistent format: adding another emission option must add another bit.
pub fn emission_config_signature() -> u64 {
    u64::from(HARDWARE_FMA.load(Ordering::Relaxed))
        | (u64::from(CHAINING.load(Ordering::Relaxed)) << 1)
        | (u64::from(REGION_TAIL_CHAINING.load(Ordering::Relaxed)) << 2)
        | (u64::from(DEFINED_TRACKING.load(Ordering::Relaxed)) << 3)
        | (u64::from(ROTATED_NESTS.load(Ordering::Relaxed)) << 4)
        | (u64::from(TLB_FILL.load(Ordering::Relaxed)) << 5)
        | (u64::from(TRACE_LEVEL.load(Ordering::Relaxed)) << 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn empty_input_never_emits_code() {
        assert!(translate_block(&[], 0, 0, JitLayout::bare()).is_none());
    }

    #[test]
    fn bare_layout_is_stable() {
        let layout = JitLayout::bare();
        assert_eq!(layout.pc_addr, 256);
        assert_eq!(layout.retired_addr, 264);
        assert!(layout.mem.is_none());
        assert!(layout.sys.is_none());
    }

    #[test]
    fn discovers_direct_cfg_leaders_and_backedges() {
        let code = words(&[
            0x0010_8093, // addi x1,x1,1
            0x0000_0463, // beq x0,x0,+8 -> 0x100c
            0x0011_0113, // addi x2,x2,1
            0x0010_8093, // addi x1,x1,1
            0xfe20_9ee3, // bne x1,x2,-4 -> 0x100c
            0x0002_8067, // jalr x0,0(x5)
        ]);
        let (leaders, backedges) =
            discover_page_leaders_ext(&code, 0x1000, 0x1000, code.len() as u64, &[0x1000], 16);
        assert_eq!(leaders, vec![0x1000, 0x100c, 0x1008, 0x1014]);
        assert_eq!(backedges, BTreeSet::from([0x100c]));
    }

    #[test]
    fn reports_only_pages_reached_by_the_bounded_cfg_walk() {
        let mut code = vec![0; 0x3000];
        // jal x0,+0x1000 from the first page into the second. The third page is
        // present in the snapshot but has no direct path from the seed.
        code[..4].copy_from_slice(&0x0000_106fu32.to_le_bytes());
        // jalr x0,0(x0): terminate discovery in page two.
        code[0x1000..0x1004].copy_from_slice(&0x0000_0067u32.to_le_bytes());
        assert_eq!(
            discover_page_reachable_pages(&code, 0x1000, 0x1000, code.len() as u64, &[0x1000], 16,),
            BTreeSet::from([0x1000, 0x2000]),
        );
    }

    #[test]
    fn noncall_page_discovery_separates_callees_from_page_straddling_jumps() {
        let mut call = vec![0; 0x2000];
        // jal ra,+0x1000, followed by an indirect return-shaped terminator.
        call[..4].copy_from_slice(&0x0000_10efu32.to_le_bytes());
        call[4..8].copy_from_slice(&0x0000_0067u32.to_le_bytes());
        call[0x1000..0x1004].copy_from_slice(&0x0000_0067u32.to_le_bytes());
        assert_eq!(
            discover_page_reachable_pages(&call, 0x1000, 0x1000, call.len() as u64, &[0x1000], 16),
            BTreeSet::from([0x1000, 0x2000]),
        );
        assert_eq!(
            discover_page_reachable_noncall_pages(
                &call,
                0x1000,
                0x1000,
                call.len() as u64,
                &[0x1000],
                16,
            ),
            BTreeSet::from([0x1000]),
        );

        // The same encoding with rd=x0 is a tail jump, not a call, and must
        // still close across the page boundary.
        call[..4].copy_from_slice(&0x0000_106fu32.to_le_bytes());
        assert_eq!(
            discover_page_reachable_noncall_pages(
                &call,
                0x1000,
                0x1000,
                call.len() as u64,
                &[0x1000],
                16,
            ),
            BTreeSet::from([0x1000, 0x2000]),
        );
    }

    #[test]
    fn emits_a_valid_multi_entry_superblock() {
        let code = words(&[
            0x0000_0093, // addi x1,x0,0
            0x0010_0113, // addi x2,x0,1
            0x00b0_0193, // addi x3,x0,11
            0x0020_80b3, // add x1,x1,x2
            0x0011_0113, // addi x2,x2,1
            0xfe31_1ce3, // bne x2,x3,-8
            0x0000_0073, // ecall
        ]);
        let region = translate_superblock(
            &code,
            0x1000,
            0x1000,
            code.len() as u64,
            &[0x1000, 0x100c],
            JitLayout::bare(),
        )
        .expect("multi-entry region");
        assert_eq!(region.n_insns, 0);
        assert_eq!(region.seeds, vec![0x1000, 0x100c]);
        wasmparser::Validator::new()
            .validate_all(&region.wasm)
            .expect("valid multi-entry Wasm");
    }

    #[test]
    fn observed_batch_and_monomorphic_extension_share_the_multi_entry_backend() {
        let mut code = words(&[
            0x0010_8093, // 0x1000: addi x1,x1,1
            0x0002_8067, // jalr x0,0(x5)
            0x0000_0013,
            0x0000_0013,
            0x0011_0113, // 0x1010: addi x2,x2,1
            0x0003_0067, // jalr x0,0(x6)
        ]);
        code.resize(64, 0);
        let batch = translate_batch_obs(
            &code,
            0x1000,
            0x1000,
            JitLayout::bare(),
            &|pc| pc == 0x1010,
            &|pc| pc == 0x1010,
            &|pc| (pc == 0x1000).then_some(0x1010),
            8,
        )
        .expect("two-member observed batch");
        assert_eq!(
            batch.1.iter().map(|member| member.pc).collect::<Vec<_>>(),
            [0x1000, 0x1010]
        );
        wasmparser::Validator::new()
            .validate_all(&batch.0)
            .expect("valid batch Wasm");

        let extended = translate_block_ic(
            &code,
            0x1000,
            0x1000,
            JitLayout::bare(),
            &|pc| pc == 0x1010,
            &|_| None,
            &|pc| (pc == 0x1000).then_some(0x1010),
        )
        .expect("monomorphic target extension");
        assert_eq!(extended.seeds, vec![0x1000, 0x1010]);
        wasmparser::Validator::new()
            .validate_all(&extended.wasm)
            .expect("valid extended Wasm");
    }

    #[test]
    fn sparse_regions_select_cached_or_materialized_state_explicitly() {
        let jal = |offset: i32| {
            let value = offset as u32;
            0x6f | (((value >> 12) & 0xff) << 12)
                | (((value >> 11) & 1) << 20)
                | (((value >> 1) & 0x3ff) << 21)
                | (((value >> 20) & 1) << 31)
        };
        let mut code = words(&[
            0x0010_8093, // 0x1000: addi x1,x1,1
            jal(0x0c),   // jal x0,0x1010
            0x0000_0013,
            0x0000_0013,
            0x0011_0113, // 0x1010: addi x2,x2,1
            jal(-0x14),  // jal x0,0x1000
        ]);
        code.resize(4096, 0);
        let entries = [0x1000, 0x1010];
        let cached =
            translate_superblock_sparse(&code, &[0x1000], &entries, JitLayout::bare(), false)
                .expect("register-resident region");
        let lazy = translate_superblock_sparse_state(
            &code,
            &[0x1000],
            &entries,
            JitLayout::bare(),
            MultiEntryState::RegisterLazy,
        )
        .expect("lazy register-resident region");
        let direct = translate_superblock_sparse_state(
            &code,
            &[0x1000],
            &entries,
            JitLayout::bare(),
            MultiEntryState::RegisterDirect,
        )
        .expect("direct-edge register-resident region");
        let cfg = translate_superblock_sparse_state(
            &code,
            &[0x1000],
            &entries,
            JitLayout::bare(),
            MultiEntryState::RegisterCfg,
        )
        .expect("CFG basic-block register-resident region");
        let structured = translate_superblock_sparse_state(
            &code,
            &[0x1000],
            &entries,
            JitLayout::bare(),
            MultiEntryState::RegisterStructured,
        )
        .expect("structured CFG register-resident region");
        let materialized =
            translate_superblock_sparse(&code, &[0x1000], &entries, JitLayout::bare(), true)
                .expect("materialized-state region");
        let mut tail_layout = JitLayout::bare();
        tail_layout.fuel_addr = 272;
        let tail = translate_superblock_sparse_state(
            &code,
            &[0x1000],
            &entries,
            tail_layout,
            MultiEntryState::MemoryTailCall,
        )
        .expect("same-module tail-call region");

        let defined_functions = |bytes: &[u8]| {
            wasmparser::Parser::new(0)
                .parse_all(bytes)
                .find_map(|payload| match payload.expect("parse generated module") {
                    wasmparser::Payload::CodeSectionStart { count, .. } => Some(count),
                    _ => None,
                })
                .expect("generated code section")
        };
        assert_eq!(defined_functions(&cached.wasm), 1);
        assert_eq!(defined_functions(&lazy.wasm), 1);
        assert_eq!(defined_functions(&direct.wasm), 1);
        assert_eq!(defined_functions(&cfg.wasm), 1);
        assert_eq!(defined_functions(&structured.wasm), 1);
        assert_eq!(defined_functions(&materialized.wasm), 3);
        assert_eq!(defined_functions(&tail.wasm), 5);
        wasmparser::Validator::new()
            .validate_all(&cached.wasm)
            .expect("valid register-resident module");
        wasmparser::Validator::new()
            .validate_all(&lazy.wasm)
            .expect("valid lazy register-resident module");
        wasmparser::Validator::new()
            .validate_all(&direct.wasm)
            .expect("valid direct-edge register-resident module");
        wasmparser::Validator::new()
            .validate_all(&cfg.wasm)
            .expect("valid CFG basic-block register-resident module");
        wasmparser::Validator::new()
            .validate_all(&structured.wasm)
            .expect("valid structured CFG register-resident module");
        let has_switch = wasmparser::Parser::new(0)
            .parse_all(&direct.wasm)
            .filter_map(Result::ok)
            .filter_map(|payload| match payload {
                wasmparser::Payload::CodeSectionEntry(body) => {
                    Some(body.get_operators_reader().expect("direct operators"))
                }
                _ => None,
            })
            .any(|mut operators| {
                while !operators.eof() {
                    if matches!(
                        operators.read().expect("direct operator"),
                        wasmparser::Operator::BrTable { .. }
                    ) {
                        return true;
                    }
                }
                false
            });
        assert!(has_switch, "direct-edge mode must emit a structured switch");
        wasmparser::Validator::new()
            .validate_all(&materialized.wasm)
            .expect("valid materialized-state module");
        wasmparser::Validator::new()
            .validate_all(&tail.wasm)
            .expect("valid same-module tail-call module");
        let has_tail_call = wasmparser::Parser::new(0)
            .parse_all(&tail.wasm)
            .filter_map(Result::ok)
            .filter_map(|payload| match payload {
                wasmparser::Payload::CodeSectionEntry(body) => {
                    Some(body.get_operators_reader().expect("tail operators"))
                }
                _ => None,
            })
            .any(|mut operators| {
                while !operators.eof() {
                    if matches!(
                        operators.read().expect("tail operator"),
                        wasmparser::Operator::ReturnCall { .. }
                    ) {
                        return true;
                    }
                }
                false
            });
        assert!(has_tail_call, "tail mode must emit return_call");
    }

    #[test]
    fn position_independent_page_aliases_emit_identical_wasm() {
        let mut code = words(&[
            0x0000_0297, // auipc x5,0
            0x0102_8293, // addi x5,x5,16
            0x0000_8067, // jalr x0,0(x1)
            0x0000_0013, // padding
            0x0000_0317, // auipc x6,0
            0xff03_0313, // addi x6,x6,-16
            0x0000_8067, // jalr x0,0(x1)
        ]);
        code.resize(4096, 0);

        let compile = |base: u64| {
            let mut layout = JitLayout::bare();
            layout.pic_code_base = Some(base);
            translate_superblock_sparse_state(
                &code,
                &[base],
                &[base, base + 0x10],
                layout,
                MultiEntryState::RegisterStructured,
            )
            .expect("position-independent page")
            .wasm
        };
        let low = compile(0x1000);
        let high = compile(0x7fff_8123_4000);
        assert_eq!(low, high, "an ASLR alias must share one compiled module");
        wasmparser::Validator::new()
            .validate_all(&low)
            .expect("valid position-independent module");
    }

    #[test]
    fn two_way_indirect_pic_uses_the_register_resident_dispatcher() {
        let mut code = words(&[
            0x0002_8067, // 0x1000: jalr x0,0(x5)
            0x0000_0013,
            0x0000_0013,
            0x0000_0013,
            0x0010_8093, // 0x1010: addi x1,x1,1
            0x0003_0067, // jalr x0,0(x6)
            0x0000_0013,
            0x0000_0013,
            0x0011_0113, // 0x1020: addi x2,x2,1
            0x0003_8067, // jalr x0,0(x7)
        ]);
        code.resize(64, 0);
        let pic = translate_block_pic(&code, 0x1000, 0x1000, JitLayout::bare(), &[0x1010, 0x1020])
            .expect("two-way PIC region");
        assert_eq!(pic.n_insns, 0);
        assert_eq!(pic.seeds, vec![0x1000, 0x1010, 0x1020]);
        let function_count = wasmparser::Parser::new(0)
            .parse_all(&pic.wasm)
            .find_map(|payload| match payload.expect("parse PIC module") {
                wasmparser::Payload::CodeSectionStart { count, .. } => Some(count),
                _ => None,
            })
            .expect("PIC code section");
        assert_eq!(function_count, 1);
        let pooled_locals = wasmparser::Parser::new(0)
            .parse_all(&pic.wasm)
            .find_map(|payload| match payload.expect("parse PIC function") {
                wasmparser::Payload::CodeSectionEntry(body) => Some(
                    body.get_locals_reader()
                        .expect("PIC local declarations")
                        .into_iter()
                        .map(|local| local.expect("valid local declaration").0)
                        .sum::<u32>(),
                ),
                _ => None,
            })
            .expect("PIC function body");
        let lifted = [0x1000, 0x1010, 0x1020]
            .map(|entry| lift_at(&code, 0x1000, entry, JitLayout::bare()).unwrap());
        let max_i32 = lifted
            .iter()
            .map(|region| {
                region
                    .ir
                    .values
                    .iter()
                    .filter(|value| value.ty == ir::ValueType::I32)
                    .count()
            })
            .max()
            .unwrap() as u32;
        let max_i64 = lifted
            .iter()
            .map(|region| {
                region
                    .ir
                    .values
                    .iter()
                    .filter(|value| value.ty == ir::ValueType::I64)
                    .count()
            })
            .max()
            .unwrap() as u32;
        let mut need_x = 0u32;
        for region in &lifted {
            for value in &region.ir.values {
                if let ir::Op::ReadX(reg) = value.op {
                    need_x |= 1 << reg;
                }
            }
            for &(reg, _) in &region.ir.outputs {
                need_x |= 1 << reg;
            }
        }
        // Cached GPRs + pc/retirement + largest-member SSA pools + the
        // dispatcher matched/hop locals. A sum-of-members allocation would
        // exceed this exact bound as the region grows.
        let expected_locals = (need_x & !1).count_ones() + 2 + max_i32 + max_i64 + 2;
        assert_eq!(pooled_locals, expected_locals);
        wasmparser::Validator::new()
            .validate_all(&pic.wasm)
            .expect("valid PIC module");
    }

    #[test]
    fn pic_keeps_a_valid_source_when_a_profiled_target_is_unavailable() {
        let code = words(&[0x0002_8067]); // jalr x0,0(x5)
        let pic = translate_block_pic(&code, 0x1000, 0x1000, JitLayout::bare(), &[0x9000])
            .expect("source-only fallback");
        assert_eq!(pic.n_insns, 0);
        assert_eq!(pic.seeds, vec![0x1000]);
        wasmparser::Validator::new()
            .validate_all(&pic.wasm)
            .expect("valid source-only Wasm");
    }

    #[test]
    fn sparse_pic_covers_noncontiguous_target_pages() {
        let mut code = vec![0; 8192];
        let first_page = words(&[
            0x0002_8067, // 0x1000: jalr x0,0(x5)
            0x0000_0013,
            0x0000_0013,
            0x0000_0013,
            0x0010_8093, // 0x1010: addi x1,x1,1
            0x0003_0067, // jalr x0,0(x6)
        ]);
        code[..first_page.len()].copy_from_slice(&first_page);
        let second_page = words(&[
            0x0011_0113, // 0x9000: addi x2,x2,1
            0x0003_8067, // jalr x0,0(x7)
        ]);
        code[4096..4096 + second_page.len()].copy_from_slice(&second_page);

        let pic = translate_block_pic_sparse(
            &code,
            &[0x1000, 0x9000],
            0x1000,
            JitLayout::bare(),
            &[0x1010, 0x9000],
        )
        .expect("sparse two-way PIC");
        assert_eq!(pic.n_insns, 0);
        assert_eq!(pic.seeds, vec![0x1000, 0x1010, 0x9000]);
        let function_count = wasmparser::Parser::new(0)
            .parse_all(&pic.wasm)
            .find_map(|payload| match payload.expect("parse sparse PIC module") {
                wasmparser::Payload::CodeSectionStart { count, .. } => Some(count),
                _ => None,
            })
            .expect("sparse PIC code section");
        assert_eq!(function_count, 1);
        wasmparser::Validator::new()
            .validate_all(&pic.wasm)
            .expect("valid sparse PIC module");
    }
}
