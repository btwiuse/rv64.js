use crate::bus::Bus;
use crate::csr::*;
use crate::decode::*;
use crate::exception::Exception;

mod vector;
pub use vector::VectorState;

/// Why `step`/`run` returned control to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Instruction budget exhausted; just call run() again.
    Budget,
    /// ECALL executed. The host services user-mode syscalls and may opt into
    /// servicing supervisor-mode SBI calls for direct Linux boot.
    Ecall,
    /// EBREAK executed (user-mode emulation only).
    Break,
    /// An exception with no handler configured (user-mode emulation only).
    Trap(Exception),
    /// WFI with no pending interrupt (full-system only): host may idle.
    Wfi,
}

/// Result of the integrated scalar Tier-0 decoder.  Ordinary scalar
/// instructions return their next PC without materializing `Cpu::pc` or
/// `Cpu::insn_count`; uncommon instruction families fall back to `step`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarT0Step {
    Retired(u64),
    Stop(u64, StopReason),
    Slow,
}

/// One permission-proved executable page carried in Wasm locals by the
/// integrated scalar driver. Instruction bytes are still reread on every
/// dispatch; this contains no decoded instruction or workload identity.
#[derive(Debug, Clone, Copy)]
struct ScalarFetchCapability {
    tag: u64,
    pa_diff: u64,
    linear_off: i64,
}

/// One successfully retired instruction observed by the opt-in policy tracer.
/// This is deliberately produced by [`Cpu::run_traced`], not by the ordinary
/// [`Cpu::run`] path: compile-policy experiments may inspect every retired
/// instruction without adding a branch or callback to production interpreter
/// execution. Addresses and privileged state describe the instruction before
/// it executed; `next_va` is the architectural PC after it retired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstructionTrace {
    pub icount: u64,
    pub va: u64,
    pub pa: u64,
    pub next_va: u64,
    pub satp: u64,
    pub mode: u8,
    pub ilen: u8,
}

/// Memory access type, for translation and fault selection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Fetch,
    Load,
    Store,
}

// 12 bits = 4096 entries/class. 256 was fine for small-working-set loops but
// direct-mapped-thrashes on multi-MB working sets (a compiler's symbol tables
// and mallocs): every conflict is a page walk in the interpreter and a block
// BAIL in the JIT. v86 never capacity-misses (full 2^20-page index for 32-bit
// guests); 4096 entries is the closest affordable equivalent for sv39.
const TLB_BITS: u32 = 12;
const TLB_SIZE: usize = 1 << TLB_BITS;
const TLB_INVALID: u64 = !0;
// Translation tags retain the complete page-aligned VA and use otherwise-zero
// page-offset bits for the permission context.  This makes a cached U-mode
// proof impossible to consume in S/M mode (and vice versa) without throwing
// every unrelated translation away at each trap boundary.
const TLB_PAGE_MASK: u64 = !0xfff;
const TLB_CTX_SUM: u64 = 1 << 2;
const TLB_CTX_MXR: u64 = 1 << 3;
/// Instructions between interrupt polls in the interpreter (see irq_poll_cd).
const IRQ_POLL_INTERVAL: u32 = 32;

#[inline(always)]
fn compressed_register(field: u32) -> usize {
    (field + 8) as usize
}

#[inline(always)]
fn compressed_sext(value: u32, bits: u32) -> i64 {
    (((value << (32 - bits)) as i32) >> (32 - bits)) as i64
}

/// RV64I hart state + interpreter.
///
/// Generic over [`Bus`] — the same execute code runs user-mode (flat memory)
/// and, later, full-system (MMU + MMIO). See /DESIGN.md.
pub struct Cpu {
    /// x0..x31; x0 reads as zero (enforced at write sites).
    pub x: [u64; 32],
    pub pc: u64,
    /// Retired instruction count (minstret / rdinstret).
    pub insn_count: u64,
    /// LR/SC reservation address (A extension); None = no reservation.
    pub reservation: Option<u64>,
    /// Debug ring buffer of the last user ecalls: (a7 syscall nr, satp).
    /// Written on every U-mode ecall; dumped by the host to diagnose hangs.
    pub syscall_log: [(u64, u64); 64],
    pub syscall_log_pos: usize,
    /// f0..f31 (F/D extensions). f32 values are NaN-boxed in the low bits.
    pub f: [u64; 32],
    /// fcsr: fflags[4:0] | frm[7:5].
    pub fcsr: u32,
    /// RVV 1.0 architectural state. This first implementation fixes VLEN at
    /// 128 bits and ELEN at 64 bits (the Zvl128b baseline required by V).
    pub vector: VectorState,
    /// Privileged state; None = pure user-mode emulation (no MMU/traps).
    pub sys: Option<SysCsrs>,
    /// Return supervisor ECALLs to the machine instead of trapping to M-mode.
    /// Used by firmware-free Linux boot, where the emulator is the SBI layer.
    pub host_sbi: bool,
    /// Diagnostics: exception counts by cause, interrupt counts by cause.
    pub exc_counts: [u64; 16],
    pub irq_counts: [u64; 16],
    /// Architectural state immediately before the most recent supervisor
    /// timer trap delivered from user mode: pc, x4/tp, sscratch, mstatus,
    /// satp, origin mode, rdtime, minstret, time scale/offset, minimum and
    /// maximum rdtime deltas, and backwards-step count. Diagnostic only.
    pub last_timer_trap: [u64; 13],
    /// Bumped whenever the va→pa code mapping may have changed (satp write,
    /// SFENCE.VMA). A JIT host keyed on virtual pc flushes its cache when
    /// this changes, which removes the need to re-verify pa on every block
    /// dispatch. Privilege changes do NOT bump it: context-tagged translation
    /// entries remain safe, and va→pa identity for a given satp is unchanged.
    pub jit_flush_gen: u64,
    /// Instructions still to run before the next interrupt poll. Sampling the
    /// bus's interrupt lines is a virtual call plus a CLINT/PLIC evaluation; at
    /// one poll per instruction it was most of the interpreter's cost. Between
    /// polls the lines can only change when devices advance (the host's
    /// sync_devices) — a guest write that could make an interrupt deliverable
    /// resets this to zero, so enabling interrupts still takes effect at once.
    irq_poll_cd: u32,
    /// Bumped on every event after which a cached va→pa translation may be
    /// stale (SFENCE.VMA, satp write). Cheaper sibling of jit_flush_gen: the
    /// JIT dispatcher re-verifies a block's code mapping only when this moved,
    /// instead of a fetch-TLB probe on every single dispatch.
    pub map_gen: u64,
    /// Diagnostic count of full translation-row invalidations.  Together with
    /// `map_gen` and `jit_flush_gen`, this separates SFENCE.VMA, changed-SATP,
    /// and non-mapping/configuration flushes without touching generated code.
    pub tlb_flushes: u64,
    /// Diagnostic count of conservative fused store-row invalidations when a
    /// physical page first becomes generated code.
    pub store_jtlb_clears: u64,
    /// SFENCE.VMA attribution. Page fences invalidate only the matching
    /// direct-map slots; global fences retain the full-row fallback.
    pub sfence_all: u64,
    pub sfence_page: u64,
    pub sfence_foreign_asid: u64,
    // Direct-mapped TLBs (page-aligned VA + permission-context tag -> pa-va
    // diff), one per access type so permission bits never need re-checking on
    // a hit.  Context-tagging lets entries survive U/S/M trap transitions.
    tlb_tag: [[u64; TLB_SIZE]; 3],
    tlb_diff: [[u64; TLB_SIZE]; 3],
    // Fused JIT-TLB ([0]=load, [1]=store): stores a *linear memory offset*
    // (`linear_index = va + off`) instead of a pa-va diff, and is filled ONLY
    // for pages the JIT can access directly — in guest RAM (and, for stores,
    // writable and not holding compiled code). So a hit lets a JIT block skip
    // the RAM range-check and store-to-compiled-page check entirely; the whole
    // inline memory op becomes tag-match + one add. Filled lazily by the
    // interpreter's own loads/stores (i.e. on JIT bail).  The context cell is
    // synchronized before generated execution and loaded once per invocation.
    jit_tlb_context: u64,
    jtlb_tag: [[u64; TLB_SIZE]; 2],
    jtlb_off: [[i64; TLB_SIZE]; 2],
    // Index policy for the fused rows. A single bank avoids adding another
    // 128 KiB to every by-value Cpu construction frame; changing the policy
    // invalidates the bank, while full tags keep old-policy modules safe.
    jtlb_hashed: bool,
    // R054 production path. A successful fused-row proof already
    // establishes permission, context, RAM bounds, and (for stores) that the
    // physical page contains no generated code. The interpreter may consume
    // the exact pointer capability directly instead of repeating its ordinary
    // translation and Bus dispatch.
    interpreter_fused_memory: bool,
    // Diagnostic A/B switch. Production retains context-tagged entries across
    // U/S/M transitions; disabling this restores the former full-flush policy
    // while leaving the generated probe shape identical.
    retain_tlb_on_privilege_change: bool,
    // One direct executable-page capability for the integrated interpreter.
    // The exact instruction bytes are still read on every execution; this
    // only amortizes Sv39-TLB lookup and physical-RAM dispatch within a page.
    interpreter_fetch_tag: u64,
    interpreter_fetch_map_gen: u64,
    interpreter_fetch_pa_diff: u64,
    interpreter_fetch_linear_off: i64,
}

/// NaN-box an f32 into a 64-bit F register (high 32 bits all-ones).
#[inline]
fn box32(v: f32) -> u64 {
    0xffff_ffff_0000_0000 | v.to_bits() as u64
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            x: [0; 32],
            pc: 0,
            insn_count: 0,
            syscall_log: [(u64::MAX, 0); 64],
            syscall_log_pos: 0,
            reservation: None,
            f: [0; 32],
            fcsr: 0,
            vector: VectorState::default(),
            sys: None,
            host_sbi: false,
            exc_counts: [0; 16],
            irq_counts: [0; 16],
            last_timer_trap: [0; 13],
            jit_flush_gen: 0,
            irq_poll_cd: 0,
            map_gen: 0,
            tlb_flushes: 0,
            store_jtlb_clears: 0,
            sfence_all: 0,
            sfence_page: 0,
            sfence_foreign_asid: 0,
            tlb_tag: [[TLB_INVALID; TLB_SIZE]; 3],
            tlb_diff: [[0; TLB_SIZE]; 3],
            jit_tlb_context: 0,
            jtlb_tag: [[TLB_INVALID; TLB_SIZE]; 2],
            jtlb_off: [[0; TLB_SIZE]; 2],
            jtlb_hashed: false,
            interpreter_fused_memory: false,
            retain_tlb_on_privilege_change: true,
            interpreter_fetch_tag: TLB_INVALID,
            interpreter_fetch_map_gen: 0,
            interpreter_fetch_pa_diff: 0,
            interpreter_fetch_linear_off: 0,
        }
    }

    /// Enable full-system mode: M/S/U privileges, MMU, traps. The hart
    /// resets to M-mode at `pc` with a0=hartid, a1=dtb (set by caller).
    pub fn enable_system(&mut self, hartid: u64) {
        let mut sys = SysCsrs::new();
        sys.mhartid = hartid;
        self.sys = Some(sys);
        self.interpreter_fetch_tag = TLB_INVALID;
    }

    /// Route supervisor-mode ECALLs to the host as [`StopReason::Ecall`].
    pub fn enable_host_sbi(&mut self) {
        self.host_sbi = true;
    }

    pub fn flush_tlb(&mut self) {
        self.tlb_flushes = self.tlb_flushes.wrapping_add(1);
        self.tlb_tag = [[TLB_INVALID; TLB_SIZE]; 3];
        self.jtlb_tag = [[TLB_INVALID; TLB_SIZE]; 2];
        self.interpreter_fetch_tag = TLB_INVALID;
    }

    /// Drop all fused store-TLB entries — called when a new block is compiled
    /// (a page may now hold code, so stores to it must bail to invalidate).
    pub fn clear_store_jtlb(&mut self) {
        self.store_jtlb_clears = self.store_jtlb_clears.wrapping_add(1);
        self.jtlb_tag[1] = [TLB_INVALID; TLB_SIZE];
    }

    /// Apply the architectural rs1/rs2 scope of SFENCE.VMA to the rows this
    /// hart can currently consume. SATP changes already discard the previous
    /// address space, so a fence for another ASID cannot match any resident
    /// entry. A nonzero rs1 names one virtual page; because our TLBs cache at
    /// 4 KiB granularity, invalidating its selected slots is sufficient even
    /// when the page-table leaf itself is a superpage.
    fn sfence_vma(&mut self, va: Option<u64>, asid: Option<u16>) {
        let current_asid = self
            .sys
            .as_ref()
            .map_or(0, |sys| ((sys.satp >> 44) & 0xffff) as u16);
        if asid.is_some_and(|target| target != current_asid) {
            self.sfence_foreign_asid = self.sfence_foreign_asid.wrapping_add(1);
            return;
        }

        self.map_gen = self.map_gen.wrapping_add(1);
        let Some(va) = va else {
            self.sfence_all = self.sfence_all.wrapping_add(1);
            self.flush_tlb();
            return;
        };

        self.sfence_page = self.sfence_page.wrapping_add(1);
        let page = va & TLB_PAGE_MASK;
        let index = ((va >> 12) as usize) & (TLB_SIZE - 1);
        for tags in &mut self.tlb_tag {
            if tags[index] & TLB_PAGE_MASK == page {
                tags[index] = TLB_INVALID;
            }
        }
        let jit_index = self.jit_tlb_index(va);
        for tags in &mut self.jtlb_tag {
            if tags[jit_index] & TLB_PAGE_MASK == page {
                tags[jit_index] = TLB_INVALID;
            }
        }
    }

    /// Fused JIT-TLB rows (load tag, load off, store tag, store off), for JIT
    /// blocks that probe them inline. A hit requires the page-aligned VA plus
    /// the current permission context; `linear_index = va + off[idx]` then
    /// needs no range or compiled-page check.
    /// Address of mstatus for the JIT's FP-state guard (0 in user mode):
    /// compiled FP instructions bail unless mstatus.FS == Dirty, so FS=Off
    /// traps and Initial/Clean transition through the interpreter exactly
    /// like fp_check/fp_dirty.
    /// Address of cpu.map_gen (u64; blocks compare its low 32 bits against a
    /// dispatch line's generation stamp before tail-calling the next block).
    pub fn jit_map_gen_ptr(&self) -> usize {
        &self.map_gen as *const u64 as usize
    }

    pub fn jit_mstatus_ptr(&self) -> usize {
        self.sys
            .as_ref()
            .map_or(0, |s| &s.mstatus as *const u64 as usize)
    }

    pub fn jit_ftlb_ptrs(&self, hashed: bool) -> (usize, usize, usize, usize) {
        debug_assert_eq!(hashed, self.jtlb_hashed);
        (
            self.jtlb_tag[0].as_ptr() as usize,
            self.jtlb_off[0].as_ptr() as usize,
            self.jtlb_tag[1].as_ptr() as usize,
            self.jtlb_off[1].as_ptr() as usize,
        )
    }

    /// Select the index function used when publishing fused translations.
    /// Policy changes are rare configuration events, so invalidate once rather
    /// than permanently carrying a second large bank in the CPU object.
    pub fn set_jit_tlb_hash(&mut self, hashed: bool) {
        if self.jtlb_hashed != hashed {
            self.jtlb_hashed = hashed;
            self.jtlb_tag = [[TLB_INVALID; TLB_SIZE]; 2];
        }
    }

    /// Enable direct interpreter consumption of permission-checked fused TLB
    /// rows. This is a runtime performance candidate; disabling it preserves
    /// the original load/store path byte-for-byte at the semantic boundary.
    pub fn set_interpreter_fused_memory(&mut self, enabled: bool) {
        self.interpreter_fused_memory = enabled;
    }

    #[inline(always)]
    fn interpreter_fused_ptr(&self, va: u64, store: bool) -> Option<usize> {
        if !self.interpreter_fused_memory {
            return None;
        }
        let index = self.jit_tlb_index(va);
        let expected = (va & TLB_PAGE_MASK) | self.jit_tlb_context;
        let (tag, offset) = self.jtlb_entry(store, index);
        (tag == expected).then(|| va.wrapping_add(offset as u64) as usize)
    }

    #[inline(always)]
    unsafe fn interpreter_direct_load<const N: u32>(address: usize) -> u64 {
        match N {
            1 => unsafe {
                core::ptr::read(core::ptr::with_exposed_provenance::<u8>(address)) as u64
            },
            2 => unsafe {
                u16::from_le(core::ptr::read_unaligned(
                    core::ptr::with_exposed_provenance::<u16>(address),
                )) as u64
            },
            4 => unsafe {
                u32::from_le(core::ptr::read_unaligned(
                    core::ptr::with_exposed_provenance::<u32>(address),
                )) as u64
            },
            8 => unsafe {
                u64::from_le(core::ptr::read_unaligned(
                    core::ptr::with_exposed_provenance::<u64>(address),
                ))
            },
            _ => unreachable!("RV64 memory widths are 1, 2, 4, or 8 bytes"),
        }
    }

    #[inline(always)]
    unsafe fn interpreter_direct_store<const N: u32>(address: usize, value: u64) {
        match N {
            1 => unsafe {
                core::ptr::write(
                    core::ptr::with_exposed_provenance_mut::<u8>(address),
                    value as u8,
                )
            },
            2 => unsafe {
                core::ptr::write_unaligned(
                    core::ptr::with_exposed_provenance_mut::<u16>(address),
                    (value as u16).to_le(),
                )
            },
            4 => unsafe {
                core::ptr::write_unaligned(
                    core::ptr::with_exposed_provenance_mut::<u32>(address),
                    (value as u32).to_le(),
                )
            },
            8 => unsafe {
                core::ptr::write_unaligned(
                    core::ptr::with_exposed_provenance_mut::<u64>(address),
                    value.to_le(),
                )
            },
            _ => unreachable!("RV64 memory widths are 1, 2, 4, or 8 bytes"),
        }
    }

    /// Select whether privilege transitions retain context-tagged entries.
    /// Turning retention off is an instrumentation baseline, not a distinct
    /// architectural mode; flush immediately so an A/B toggle starts cold.
    pub fn set_privilege_tlb_retention(&mut self, retain: bool) {
        if self.retain_tlb_on_privilege_change != retain {
            self.retain_tlb_on_privilege_change = retain;
            self.flush_tlb();
        }
    }

    /// Address of the context tag generated code folds into every fused-TLB
    /// probe. The runtime synchronizes it once before entering a generated
    /// chain; compiled code cannot change privilege or translation CSRs.
    pub fn jit_tlb_context_ptr(&self) -> usize {
        &self.jit_tlb_context as *const u64 as usize
    }

    /// Refresh the generated-code view of the effective data-access context.
    /// This is public so embedders that mutate privileged state directly can
    /// establish the same entry invariant as the production dispatcher.
    #[inline]
    pub fn sync_jit_tlb_context(&mut self) {
        self.jit_tlb_context = self.data_tlb_context();
    }

    #[inline]
    fn privilege_changed(&mut self) {
        if !self.retain_tlb_on_privilege_change {
            self.flush_tlb();
        }
        self.sync_jit_tlb_context();
    }

    #[inline]
    fn publish_jtlb(&mut self, store: bool, index: usize, tag: u64, offset: i64) {
        self.jtlb_tag[store as usize][index] = tag;
        self.jtlb_off[store as usize][index] = offset;
    }

    #[inline]
    fn jtlb_entry(&self, store: bool, index: usize) -> (u64, i64) {
        (
            self.jtlb_tag[store as usize][index],
            self.jtlb_off[store as usize][index],
        )
    }

    /// Populate a fused JIT-TLB entry if `bus` says the page is directly
    /// accessible. Called from the interpreter's own load/store path, so the
    /// entry is warm the next time a JIT block reaches it.
    #[inline]
    fn fill_jtlb<B: Bus>(&mut self, bus: &B, va: u64, pa: u64, store: bool) {
        if let Some(off) = bus.jit_fast_off(va, pa, store) {
            let idx = self.jit_tlb_index(va);
            self.publish_jtlb(
                store,
                idx,
                (va & TLB_PAGE_MASK) | self.data_tlb_context(),
                off,
            );
        }
    }

    /// Fill the fused JIT-TLB row for `va` without raising a fault, returning
    /// the offset a compiled block needs (`linear_index = va + off`), or None
    /// if the access can't be served inline — unmapped, permission-denied,
    /// MMIO, or a page holding compiled code. Compiled blocks call this on a
    /// TLB miss and carry on; None sends them to the interpreter, which
    /// re-executes the instruction and raises the exact architectural fault.
    pub fn jit_fill_tlb<B: Bus>(&mut self, bus: &mut B, va: u64, store: bool) -> Option<i64> {
        let access = if store { Access::Store } else { Access::Load };
        let pa = self.translate(bus, va, access).ok()?;
        let off = bus.jit_fast_off(va, pa, store)?;
        let idx = self.jit_tlb_index(va);
        self.publish_jtlb(
            store,
            idx,
            (va & TLB_PAGE_MASK) | self.data_tlb_context(),
            off,
        );
        Some(off)
    }

    /// Return an existing permission/context-checked fused JIT-TLB mapping
    /// without walking the guest page tables. Runtime helpers use this before
    /// `jit_fill_tlb`: generated scalar accesses normally perform the same
    /// probe inline, but a Wasm-to-Wasm helper must not turn every bulk-memory
    /// chunk into an unconditional Sv39 translation.
    #[inline]
    pub fn jit_probe_tlb(&self, va: u64, store: bool) -> Option<i64> {
        let idx = self.jit_tlb_index(va);
        let expected = (va & TLB_PAGE_MASK) | self.data_tlb_context();
        let (tag, offset) = self.jtlb_entry(store, idx);
        (tag == expected).then_some(offset)
    }

    /// Classify why a generated fused-TLB probe reached its refill helper.
    /// Diagnostic only: 0 is an unexpected hit, 1 an empty slot, 2 the same
    /// virtual page under another permission context, and 3 a different page
    /// occupying the selected slot.
    pub fn jit_tlb_miss_kind(&self, va: u64, store: bool) -> u8 {
        let (tag, _) = self.jtlb_entry(store, self.jit_tlb_index(va));
        let expected = (va & TLB_PAGE_MASK) | self.data_tlb_context();
        if tag == expected {
            0
        } else if tag == TLB_INVALID {
            1
        } else if tag & TLB_PAGE_MASK == va & TLB_PAGE_MASK {
            2
        } else {
            3
        }
    }

    /// Translate a fetch address without raising a fault (JIT support:
    /// verify that a va-keyed compiled block still maps to the same
    /// physical code before dispatching to it).
    pub fn jit_probe_fetch<B: Bus>(&mut self, bus: &mut B, va: u64) -> Option<u64> {
        self.translate(bus, va, Access::Fetch).ok()
    }

    /// Addresses of the Load/Store TLB rows (tag then pa-va diff, each
    /// TLB_SIZE u64 entries), for JIT blocks that probe the TLB inline.
    /// Layout contract: tag[i] equals the page-aligned VA plus access context;
    /// pa = va + diff[i];
    /// index = (va>>12) & (jit_tlb_size()-1). Entries are only ever filled
    /// by successful translations (permissions + A/D already applied) and
    /// are flushed on satp/mstatus/priv changes — so a hit is always safe
    /// to use directly.
    pub fn jit_tlb_ptrs(&self) -> (usize, usize, usize, usize) {
        let l = Access::Load as usize;
        let s = Access::Store as usize;
        (
            self.tlb_tag[l].as_ptr() as usize,
            self.tlb_diff[l].as_ptr() as usize,
            self.tlb_tag[s].as_ptr() as usize,
            self.tlb_diff[s].as_ptr() as usize,
        )
    }

    pub fn jit_tlb_size() -> usize {
        TLB_SIZE
    }

    pub const fn jit_tlb_hash_shift() -> u8 {
        TLB_BITS as u8
    }

    #[inline]
    fn jit_tlb_hash_index(va: u64) -> usize {
        let page = va >> 12;
        ((page ^ (page >> TLB_BITS)) as usize) & (TLB_SIZE - 1)
    }

    #[inline]
    fn jit_tlb_index(&self, va: u64) -> usize {
        if self.jtlb_hashed {
            Self::jit_tlb_hash_index(va)
        } else {
            ((va >> 12) as usize) & (TLB_SIZE - 1)
        }
    }

    #[inline]
    fn wr(&mut self, rd: usize, val: u64) {
        if rd != 0 {
            self.x[rd] = val;
        }
    }

    /// FP instructions are illegal while mstatus.FS = Off (system mode).
    #[inline]
    fn fp_check(&self, insn: u32) -> Result<(), Exception> {
        if let Some(sys) = &self.sys {
            if sys.mstatus & MSTATUS_FS == 0 {
                return Err(Exception::IllegalInstruction { insn });
            }
        }
        Ok(())
    }

    /// Mark FP state dirty (mstatus.FS = 11) after FP execution.
    #[inline]
    fn fp_dirty(&mut self) {
        if let Some(sys) = &mut self.sys {
            sys.mstatus |= MSTATUS_FS;
        }
    }

    // ---- address translation --------------------------------------------

    #[inline]
    fn fault(access: Access, addr: u64) -> Exception {
        match access {
            Access::Fetch => Exception::InstructionPageFault { addr },
            Access::Load => Exception::LoadPageFault { addr },
            Access::Store => Exception::StorePageFault { addr },
        }
    }

    /// Effective privilege for data accesses (MPRV) or fetch.
    #[inline(always)]
    fn eff_mode(&self, access: Access) -> Mode {
        let sys = self.sys.as_ref().unwrap();
        if access != Access::Fetch && sys.mstatus & MSTATUS_MPRV != 0 {
            Mode::from_bits((sys.mstatus & MSTATUS_MPP) >> 11)
        } else {
            sys.mode
        }
    }

    /// Permission context whose low bits are folded into a page-aligned TLB
    /// tag. SATP/page-table identity remains covered by a full invalidation on
    /// SATP changes and architecturally scoped invalidation on SFENCE.VMA.
    #[inline]
    fn translation_context(&self, access: Access) -> u64 {
        let Some(sys) = self.sys.as_ref() else {
            return 0;
        };
        let mut context = self.eff_mode(access) as u64;
        if access != Access::Fetch && sys.mstatus & MSTATUS_SUM != 0 {
            context |= TLB_CTX_SUM;
        }
        if access == Access::Load && sys.mstatus & MSTATUS_MXR != 0 {
            context |= TLB_CTX_MXR;
        }
        context
    }

    /// Unified load/store context used by the fused JIT rows. Including MXR
    /// for stores merely partitions a few more entries; it lets both rows use
    /// one invocation-local context value without weakening either proof.
    #[inline]
    fn data_tlb_context(&self) -> u64 {
        let Some(sys) = self.sys.as_ref() else {
            return 0;
        };
        let mut context = self.eff_mode(Access::Load) as u64;
        if sys.mstatus & MSTATUS_SUM != 0 {
            context |= TLB_CTX_SUM;
        }
        if sys.mstatus & MSTATUS_MXR != 0 {
            context |= TLB_CTX_MXR;
        }
        context
    }

    #[inline]
    fn translation_tag(&self, va: u64, access: Access) -> u64 {
        (va & TLB_PAGE_MASK) | self.translation_context(access)
    }

    /// Translate a virtual address (full-system mode). Hot path: TLB hit.
    #[inline]
    fn translate<B: Bus>(
        &mut self,
        bus: &mut B,
        va: u64,
        access: Access,
    ) -> Result<u64, Exception> {
        if self.sys.is_none() {
            return Ok(va);
        }
        let idx = ((va >> 12) as usize) & (TLB_SIZE - 1);
        let tag = self.translation_tag(va, access);
        let a = access as usize;
        if self.tlb_tag[a][idx] == tag {
            return Ok(va.wrapping_add(self.tlb_diff[a][idx]));
        }
        self.translate_slow(bus, va, access)
    }

    /// Fetch one halfword through the integrated interpreter's slice-local
    /// executable-page capability. The driver validates mapping generation at
    /// entry and invalidates the local at every boundary that may change
    /// privilege or mappings. Instruction bytes themselves are never cached.
    #[inline(always)]
    fn scalar_fetch16<B: Bus>(
        &mut self,
        bus: &mut B,
        va: u64,
        context: u64,
        capability: &mut ScalarFetchCapability,
    ) -> Result<(u64, u16, Option<usize>), Exception> {
        let tag = (va & TLB_PAGE_MASK) | context;
        if capability.tag == tag {
            let address = va.wrapping_add(capability.linear_off as u64) as usize;
            let instruction = u16::from_le(unsafe {
                // SAFETY: the cached direct-RAM capability covers this whole
                // executable page. The driver invalidates it at every mapping
                // or privilege-changing boundary.
                core::ptr::read_unaligned(core::ptr::with_exposed_provenance::<u16>(address))
            });
            return Ok((
                va.wrapping_add(capability.pa_diff),
                instruction,
                Some(address),
            ));
        }

        self.scalar_fetch16_slow(bus, va, tag, capability)
    }

    /// Authoritative executable-page refill for an integrated scalar fetch
    /// miss. The capability-hit branch remains inline in `scalar_fetch16`;
    /// translation, physical-bus dispatch, and fault propagation stay here.
    #[inline(never)]
    fn scalar_fetch16_slow<B: Bus>(
        &mut self,
        bus: &mut B,
        va: u64,
        tag: u64,
        capability: &mut ScalarFetchCapability,
    ) -> Result<(u64, u16, Option<usize>), Exception> {
        let pa = self.translate(bus, va, Access::Fetch)?;
        if let Some(offset) = bus.jit_fast_off(va, pa, false) {
            let address = va.wrapping_add(offset as u64) as usize;
            let instruction = u16::from_le(unsafe {
                // SAFETY: jit_fast_off proves a live direct-RAM capability
                // for the complete page containing this translated fetch.
                core::ptr::read_unaligned(core::ptr::with_exposed_provenance::<u16>(address))
            });
            capability.tag = tag;
            capability.pa_diff = pa.wrapping_sub(va);
            capability.linear_off = offset;
            Ok((pa, instruction, Some(address)))
        } else {
            bus.fetch16(pa).map(|instruction| (pa, instruction, None))
        }
    }

    /// Page-table walk (sv39/sv48), permission checks, A/D update, TLB fill.
    fn translate_slow<B: Bus>(
        &mut self,
        bus: &mut B,
        va: u64,
        access: Access,
    ) -> Result<u64, Exception> {
        let tag = self.translation_tag(va, access);
        let sys = self.sys.as_ref().unwrap();
        let mode = self.eff_mode(access);
        let satp = sys.satp;
        let vm = satp >> 60;

        // Bare, or M-mode without MPRV redirection: identity.
        if vm == 0 || mode == Mode::Machine {
            return Ok(va);
        }
        let levels: i32 = match vm {
            8 => 3, // sv39
            9 => 4, // sv48
            _ => return Err(Self::fault(access, va)),
        };
        // Canonical check: high bits must equal bit (9*levels + 12 - 1).
        let va_bits = 9 * levels as u32 + 12;
        let ext = (va as i64) >> (va_bits - 1);
        if ext != 0 && ext != -1 {
            return Err(Self::fault(access, va));
        }

        let sum = sys.mstatus & MSTATUS_SUM != 0;
        let mxr = sys.mstatus & MSTATUS_MXR != 0;

        let mut table = (satp & 0xfff_ffff_ffff) << 12; // PPN
        let mut level = levels - 1;
        loop {
            let vpn = (va >> (12 + 9 * level as u32)) & 0x1ff;
            let pte_addr = table + vpn * 8;
            let pte = bus.read64(pte_addr).map_err(|_| Self::fault(access, va))?;
            let v = pte & 1;
            let r = pte >> 1 & 1;
            let w = pte >> 2 & 1;
            let x = pte >> 3 & 1;
            if v == 0 || (r == 0 && w == 1) {
                return Err(Self::fault(access, va));
            }
            if r == 0 && x == 0 {
                // pointer to next level
                if level == 0 {
                    return Err(Self::fault(access, va));
                }
                table = (pte >> 10) << 12;
                level -= 1;
                continue;
            }
            // Leaf. Check alignment of superpages.
            let ppn = pte >> 10;
            if level > 0 && (ppn & ((1 << (9 * level as u32)) - 1)) != 0 {
                return Err(Self::fault(access, va));
            }
            // Permission checks.
            let u = pte >> 4 & 1 != 0;
            match mode {
                Mode::User if !u => return Err(Self::fault(access, va)),
                Mode::Supervisor if u && !(sum && access != Access::Fetch) => {
                    return Err(Self::fault(access, va))
                }
                _ => {}
            }
            let ok = match access {
                Access::Fetch => x == 1,
                Access::Load => r == 1 || (mxr && x == 1),
                Access::Store => w == 1,
            };
            if !ok {
                return Err(Self::fault(access, va));
            }
            // A/D update (hardware-managed, like TinyEMU).
            let mut new_pte = pte | 1 << 6; // A
            if access == Access::Store {
                new_pte |= 1 << 7; // D
            }
            if new_pte != pte {
                bus.write64(pte_addr, new_pte)
                    .map_err(|_| Self::fault(access, va))?;
            }
            // Physical address: superpage low VPN bits come from va.
            let mask = (1u64 << (12 + 9 * level as u32)) - 1;
            let pa = ((ppn << 12) & !mask) | (va & mask);

            // Fill TLB (only 4K granularity; superpages fill one entry).
            // Don't cache Load entries whose D bit isn't set for stores etc.
            let idx = ((va >> 12) as usize) & (TLB_SIZE - 1);
            let a = access as usize;
            self.tlb_tag[a][idx] = tag;
            self.tlb_diff[a][idx] = pa.wrapping_sub(va);
            return Ok(pa);
        }
    }

    // ---- memory accessors (virtual in full-system, direct otherwise) -----

    #[inline(always)]
    fn ld<B: Bus, const N: u32>(&mut self, bus: &mut B, va: u64) -> Result<u64, Exception> {
        if self.sys.is_some() && (va & 0xfff) + N as u64 > 0x1000 {
            return self.ld_slow::<B, N>(bus, va);
        }
        if let Some(address) = self.interpreter_fused_ptr(va, false) {
            // SAFETY: a matching fused load row is published only from
            // `Bus::jit_fast_off`, whose pointer capability covers the whole
            // page and remains live until the same invalidation events that
            // clear this tag.
            return Ok(unsafe { Self::interpreter_direct_load::<N>(address) });
        }
        self.ld_slow::<B, N>(bus, va)
    }

    /// Authoritative load path for page crossing, fused misses, MMIO, and
    /// faults. Keep it out of the integrated scalar body while preserving the
    /// exact translation/refill/bus operations used by the complete decoder.
    #[inline(never)]
    fn ld_slow<B: Bus, const N: u32>(&mut self, bus: &mut B, va: u64) -> Result<u64, Exception> {
        // Split accesses that cross a page boundary (two translations).
        if self.sys.is_some() && (va & 0xfff) + N as u64 > 0x1000 {
            let mut v: u64 = 0;
            for i in 0..N as u64 {
                let pa = self.translate(bus, va + i, Access::Load)?;
                v |= (bus.read8(pa)? as u64) << (8 * i);
            }
            return Ok(v);
        }
        let pa = self.translate(bus, va, Access::Load)?;
        self.fill_jtlb(bus, va, pa, false);
        match N {
            1 => bus.read8(pa).map(|v| v as u64),
            2 => bus.read16(pa).map(|v| v as u64),
            4 => bus.read32(pa).map(|v| v as u64),
            _ => bus.read64(pa),
        }
    }

    #[inline(always)]
    fn st<B: Bus, const N: u32>(
        &mut self,
        bus: &mut B,
        va: u64,
        val: u64,
    ) -> Result<(), Exception> {
        if self.sys.is_some() && (va & 0xfff) + N as u64 > 0x1000 {
            return self.st_slow::<B, N>(bus, va, val);
        }
        if let Some(address) = self.interpreter_fused_ptr(va, true) {
            // SAFETY: store rows additionally prove that the page was not
            // marked as generated code. Every publication that marks a code
            // page clears the entire store row before guest execution resumes.
            unsafe { Self::interpreter_direct_store::<N>(address, val) };
            return Ok(());
        }
        self.st_slow::<B, N>(bus, va, val)
    }

    /// Authoritative store path corresponding to [`Self::ld_slow`].
    #[inline(never)]
    fn st_slow<B: Bus, const N: u32>(
        &mut self,
        bus: &mut B,
        va: u64,
        val: u64,
    ) -> Result<(), Exception> {
        if self.sys.is_some() && (va & 0xfff) + N as u64 > 0x1000 {
            for i in 0..N as u64 {
                let pa = self.translate(bus, va + i, Access::Store)?;
                bus.write8(pa, (val >> (8 * i)) as u8)?;
            }
            return Ok(());
        }
        let pa = self.translate(bus, va, Access::Store)?;
        self.fill_jtlb(bus, va, pa, true);
        match N {
            1 => bus.write8(pa, val as u8),
            2 => bus.write16(pa, val as u16),
            4 => bus.write32(pa, val as u32),
            _ => bus.write64(pa, val),
        }
    }

    // ---- traps ------------------------------------------------------------

    /// Enter the trap handler for an exception or interrupt.
    pub fn take_trap(&mut self, cause: u64, tval: u64, is_interrupt: bool) {
        // Entering a trap changes xIE and mode; re-poll on the next instruction.
        self.irq_poll_cd = 0;
        // A trap between an LR and its SC must invalidate the reservation, so
        // the SC fails and the guest's LR/SC loop retries. Linux's atomics rely
        // on this: without it, an interrupt handler that updates the same word
        // via LR/SC lets the interrupted SC still succeed, silently losing the
        // handler's update — an intermittent source of lost wakeups.
        self.reservation = None;
        // Record user syscalls (ecall from U-mode = cause 8) in a ring buffer.
        if !is_interrupt && cause == 8 {
            let satp = self.sys.as_ref().map_or(0, |s| s.satp);
            self.syscall_log[self.syscall_log_pos] = (self.x[17], satp);
            self.syscall_log_pos = (self.syscall_log_pos + 1) % self.syscall_log.len();
        }
        let c = (cause & 15) as usize;
        if is_interrupt {
            self.irq_counts[c] += 1;
        } else {
            self.exc_counts[c] += 1;
        }
        if is_interrupt && cause == 5 && self.sys.as_ref().is_some_and(|sys| sys.mode == Mode::User)
        {
            let sys = self.sys.as_ref().expect("timer trap requires system mode");
            let rdtime = self
                .insn_count
                .checked_div(sys.time_scale)
                .map(|time| time.wrapping_add(sys.time_offset))
                .unwrap_or(sys.mtime);
            let prior = self.last_timer_trap[6];
            let delta = rdtime.saturating_sub(prior);
            let backwards = self.last_timer_trap[12] + u64::from(prior != 0 && rdtime < prior);
            let min_delta = if prior == 0 {
                0
            } else if self.last_timer_trap[10] == 0 {
                delta
            } else {
                self.last_timer_trap[10].min(delta)
            };
            let max_delta = self.last_timer_trap[11].max(delta);
            self.last_timer_trap = [
                self.pc,
                self.x[4],
                sys.sscratch,
                sys.mstatus,
                sys.satp,
                sys.mode as u64,
                rdtime,
                self.insn_count,
                sys.time_scale,
                sys.time_offset,
                min_delta,
                max_delta,
                backwards,
            ];
        }
        let sys = self.sys.as_mut().unwrap();
        let deleg = if is_interrupt {
            sys.mideleg
        } else {
            sys.medeleg
        };
        let bit = 1u64 << (cause & 63);
        let to_s = sys.mode != Mode::Machine && (deleg & bit) != 0;

        let cause_val = if is_interrupt {
            (1 << 63) | cause
        } else {
            cause
        };
        if to_s {
            sys.scause = cause_val;
            sys.stval = tval;
            sys.sepc = self.pc;
            // SPIE = SIE; SIE = 0; SPP = prev
            let sie = (sys.mstatus >> 1) & 1;
            sys.mstatus = (sys.mstatus & !(MSTATUS_SPIE | MSTATUS_SPP | MSTATUS_SIE))
                | (sie << 5)
                | (if sys.mode == Mode::Supervisor {
                    MSTATUS_SPP
                } else {
                    0
                });
            sys.mode = Mode::Supervisor;
            let base = sys.stvec & !3;
            self.pc = if sys.stvec & 3 == 1 && is_interrupt {
                base + 4 * cause
            } else {
                base
            };
        } else {
            sys.mcause = cause_val;
            sys.mtval = tval;
            sys.mepc = self.pc;
            let mie = (sys.mstatus >> 3) & 1;
            sys.mstatus = (sys.mstatus & !(MSTATUS_MPIE | MSTATUS_MPP | MSTATUS_MIE))
                | (mie << 7)
                | ((sys.mode as u64) << 11);
            sys.mode = Mode::Machine;
            let base = sys.mtvec & !3;
            self.pc = if sys.mtvec & 3 == 1 && is_interrupt {
                base + 4 * cause
            } else {
                base
            };
        }
        // Translation proofs are privilege-tagged. Preserve the unrelated
        // working set across this very frequent U↔S/M boundary.
        self.privilege_changed();
    }

    fn exception_to_trap(&mut self, e: Exception) {
        let (cause, tval) = match e {
            Exception::InstructionAddressMisaligned { addr } => (0, addr),
            Exception::InstructionAccessFault { addr } => (1, addr),
            Exception::IllegalInstruction { insn } => (2, insn as u64),
            Exception::Breakpoint => (3, self.pc),
            Exception::LoadAddressMisaligned { addr } => (4, addr),
            Exception::LoadAccessFault { addr } => (5, addr),
            Exception::StoreAddressMisaligned { addr } => (6, addr),
            Exception::StoreAccessFault { addr } => (7, addr),
            Exception::EnvironmentCallFromUMode => (8, 0),
            Exception::EnvironmentCallFromSMode => (9, 0),
            Exception::EnvironmentCallFromMMode => (11, 0),
            Exception::InstructionPageFault { addr } => (12, addr),
            Exception::LoadPageFault { addr } => (13, addr),
            Exception::StorePageFault { addr } => (15, addr),
        };
        self.take_trap(cause, tval, false);
    }

    /// Check for a deliverable interrupt; take the highest-priority one.
    /// Returns true if a trap was taken. Hardware lines (timer/external)
    /// come live from the bus; only software bits live in sys.mip.
    pub fn check_interrupts<B: Bus>(&mut self, bus: &mut B) -> bool {
        let Some(sys) = self.sys.as_mut() else {
            return false;
        };
        const HW: u64 = IRQ_MTIP | IRQ_MSIP | IRQ_MEIP | IRQ_SEIP;
        sys.mip = (sys.mip & !HW) | (bus.irq_lines() & HW);
        let sys = self.sys.as_ref().unwrap();
        let pending = sys.mip & sys.mie;
        if pending == 0 {
            return false;
        }
        let mideleg = sys.mideleg;
        let m_enabled = sys.mode != Mode::Machine || (sys.mstatus & MSTATUS_MIE) != 0;
        let s_enabled = sys.mode == Mode::User
            || (sys.mode == Mode::Supervisor && (sys.mstatus & MSTATUS_SIE) != 0);

        // Priority: MEI, MSI, MTI, SEI, SSI, STI.
        for &irq in &[11u64, 3, 7, 9, 1, 5] {
            let bit = 1u64 << irq;
            if pending & bit == 0 {
                continue;
            }
            let target_s = mideleg & bit != 0;
            let deliverable = if target_s {
                // S-target: fires when we're below S, or in S with SIE.
                sys.mode == Mode::User || (sys.mode == Mode::Supervisor && s_enabled)
            } else {
                // M-target: fires when below M, or in M with MIE.
                sys.mode != Mode::Machine || m_enabled
            };
            if deliverable {
                self.take_trap(irq, 0, true);
                return true;
            }
        }
        false
    }
    #[inline(always)]
    fn commit_scalar_t0_state(&mut self, pc: u64, retired: u64) {
        self.pc = pc;
        self.insn_count = self.insn_count.wrapping_add(retired);
    }

    #[inline(always)]
    fn publish_scalar_fetch_capability(&mut self, capability: ScalarFetchCapability) {
        self.interpreter_fetch_tag = capability.tag;
        self.interpreter_fetch_map_gen = self.map_gen;
        self.interpreter_fetch_pa_diff = capability.pa_diff;
        self.interpreter_fetch_linear_off = capability.linear_off;
    }

    /// R066 execution driver.  `CHECK_STOP` is a compile-time choice so the
    /// cold-boot `run` path retains no generated-code re-entry callback, while
    /// `run_until` gets the exact existing post-instruction contract.
    fn run_integrated_scalar_t0<B: Bus, F: FnMut(u64) -> bool, const CHECK_STOP: bool>(
        &mut self,
        bus: &mut B,
        budget: u64,
        stop_at: &mut F,
    ) -> StopReason {
        let system = self.sys.is_some();
        let mut pc = self.pc;
        let mut retired = 0u64;
        let mut consumed = 0u64;
        let mut fetch_context = self.sys.as_ref().map_or(0, |sys| sys.mode as u64);
        let mut fetch = if self.interpreter_fetch_map_gen == self.map_gen {
            ScalarFetchCapability {
                tag: self.interpreter_fetch_tag,
                pa_diff: self.interpreter_fetch_pa_diff,
                linear_off: self.interpreter_fetch_linear_off,
            }
        } else {
            ScalarFetchCapability {
                tag: TLB_INVALID,
                pa_diff: 0,
                linear_off: 0,
            }
        };

        while consumed < budget {
            if system {
                if self.irq_poll_cd == 0 {
                    // Interrupt delivery observes architectural PC/minstret.
                    self.commit_scalar_t0_state(pc, retired);
                    retired = 0;
                    self.irq_poll_cd = IRQ_POLL_INTERVAL;
                    if self.check_interrupts(bus) {
                        fetch.tag = TLB_INVALID;
                        fetch_context = self.sys.as_ref().map_or(0, |sys| sys.mode as u64);
                    }
                    pc = self.pc;
                } else {
                    self.irq_poll_cd -= 1;
                }
            }

            match self.scalar_t0_step(bus, pc, fetch_context, &mut fetch) {
                Ok(ScalarT0Step::Retired(next_pc)) => {
                    pc = next_pc;
                    retired = retired.wrapping_add(1);
                    consumed += 1;
                }
                Ok(ScalarT0Step::Stop(next_pc, stop)) => {
                    self.commit_scalar_t0_state(next_pc, retired.wrapping_add(1));
                    self.publish_scalar_fetch_capability(fetch);
                    return stop;
                }
                Ok(ScalarT0Step::Slow) => {
                    consumed += 1;
                    // The complete decoder and every uncommon instruction
                    // family observe fully materialized architectural state.
                    self.commit_scalar_t0_state(pc, retired);
                    retired = 0;
                    fetch.tag = TLB_INVALID;
                    match self.step(bus) {
                        Ok(None) => {}
                        Ok(Some(stop)) => return stop,
                        Err(e) => {
                            if system {
                                self.exception_to_trap(e);
                            } else {
                                return StopReason::Trap(e);
                            }
                        }
                    }
                    pc = self.pc;
                    fetch_context = self.sys.as_ref().map_or(0, |sys| sys.mode as u64);
                }
                Err(e) => {
                    consumed += 1;
                    // Faulting instructions do not retire.  The trap path must
                    // see the faulting PC, not the last committed stretch PC.
                    self.commit_scalar_t0_state(pc, retired);
                    retired = 0;
                    fetch.tag = TLB_INVALID;
                    if system {
                        self.exception_to_trap(e);
                        pc = self.pc;
                        fetch_context = self.sys.as_ref().map_or(0, |sys| sys.mode as u64);
                    } else {
                        return StopReason::Trap(e);
                    }
                }
            }

            if CHECK_STOP && stop_at(pc) {
                self.commit_scalar_t0_state(pc, retired);
                self.publish_scalar_fetch_capability(fetch);
                return StopReason::Budget;
            }
        }

        self.commit_scalar_t0_state(pc, retired);
        self.publish_scalar_fetch_capability(fetch);
        StopReason::Budget
    }

    /// Direct-interpreter driver that keeps ordinary scalar PC and retirement
    /// state in locals. JIT fallback continues to use the authoritative
    /// per-instruction `run` path.
    pub fn run_integrated_scalar<B: Bus>(&mut self, bus: &mut B, budget: u64) -> StopReason {
        self.run_integrated_scalar_t0::<B, _, false>(bus, budget, &mut |_| false)
    }

    /// Run up to `budget` instructions; returns why we stopped.
    pub fn run<B: Bus>(&mut self, bus: &mut B, budget: u64) -> StopReason {
        let system = self.sys.is_some();
        for _ in 0..budget {
            if system {
                if self.irq_poll_cd == 0 {
                    self.irq_poll_cd = IRQ_POLL_INTERVAL;
                    self.check_interrupts(bus);
                } else {
                    self.irq_poll_cd -= 1;
                }
            }
            match self.step(bus) {
                Ok(None) => {}
                Ok(Some(stop)) => return stop,
                Err(e) => {
                    if system {
                        self.exception_to_trap(e);
                    } else {
                        return StopReason::Trap(e);
                    }
                }
            }
        }
        StopReason::Budget
    }

    /// Run until the budget, an architectural stop, or `stop_at(next_pc)`.
    ///
    /// The predicate is checked after each successfully handled instruction
    /// (including a system exception redirected to its guest trap vector), so
    /// callers can hand execution to generated code at an exact PC without
    /// repeatedly invoking `run(..., 1)`. It is deliberately a separate path:
    /// the ordinary cold interpreter pays no callback or branch overhead.
    pub fn run_until<B: Bus>(
        &mut self,
        bus: &mut B,
        budget: u64,
        stop_at: &mut impl FnMut(u64) -> bool,
    ) -> StopReason {
        let system = self.sys.is_some();
        for _ in 0..budget {
            if system {
                if self.irq_poll_cd == 0 {
                    self.irq_poll_cd = IRQ_POLL_INTERVAL;
                    self.check_interrupts(bus);
                } else {
                    self.irq_poll_cd -= 1;
                }
            }
            match self.step(bus) {
                Ok(None) => {}
                Ok(Some(stop)) => return stop,
                Err(e) => {
                    if system {
                        self.exception_to_trap(e);
                    } else {
                        return StopReason::Trap(e);
                    }
                }
            }
            if stop_at(self.pc) {
                return StopReason::Budget;
            }
        }
        StopReason::Budget
    }

    /// Exact generated-code re-entry path with a low-volume control-target
    /// observer. The first non-sequential next PC is reported with its mapping
    /// and architectural context active after the instruction. This is a JIT
    /// policy signal only: direct transfers whose target happens to equal the
    /// ordinary +2/+4 fallthrough may be omitted, but execution semantics and
    /// the exact `stop_at` contract are identical to [`Self::run_until`].
    pub fn run_until_observed<B: Bus>(
        &mut self,
        bus: &mut B,
        budget: u64,
        stop_at: &mut impl FnMut(u64) -> bool,
        observe_target: &mut impl FnMut(u64, u64, u64, u8),
    ) -> StopReason {
        let system = self.sys.is_some();
        let mut observed_target = false;
        for _ in 0..budget {
            if system {
                if self.irq_poll_cd == 0 {
                    self.irq_poll_cd = IRQ_POLL_INTERVAL;
                    self.check_interrupts(bus);
                } else {
                    self.irq_poll_cd -= 1;
                }
            }
            let previous_pc = self.pc;
            match self.step(bus) {
                Ok(None) => {}
                Ok(Some(stop)) => return stop,
                Err(e) => {
                    if system {
                        self.exception_to_trap(e);
                    } else {
                        return StopReason::Trap(e);
                    }
                }
            }
            // A generated destination ends this interpreted stretch and is
            // already known to the policy. Test it before doing the extra fetch
            // probe so the observer reports only genuinely uncovered targets.
            if stop_at(self.pc) {
                return StopReason::Budget;
            }
            let sequential2 = previous_pc.checked_add(2) == Some(self.pc);
            let sequential4 = previous_pc.checked_add(4) == Some(self.pc);
            if !observed_target && !sequential2 && !sequential4 {
                let target = self.pc;
                if let Some(pa) = self.jit_probe_fetch(bus, target) {
                    let (satp, mode) = self
                        .sys
                        .as_ref()
                        .map_or((0, u8::MAX), |sys| (sys.satp, sys.mode as u8));
                    observe_target(target, pa, satp, mode);
                    observed_target = true;
                }
            }
        }
        StopReason::Budget
    }

    /// Interpreter-only instrumentation path for compile-policy research.
    ///
    /// Semantics intentionally mirror [`Cpu::run`]. The extra fetch probe and
    /// per-instruction callback make this unsuitable for performance numbers;
    /// callers use it to collect a workload trace and replay candidate tier-up
    /// policies offline. The normal interpreter remains completely untouched.
    pub fn run_traced<B: Bus>(
        &mut self,
        bus: &mut B,
        budget: u64,
        trace: &mut impl FnMut(InstructionTrace),
    ) -> StopReason {
        let system = self.sys.is_some();
        for _ in 0..budget {
            if system {
                if self.irq_poll_cd == 0 {
                    self.irq_poll_cd = IRQ_POLL_INTERVAL;
                    self.check_interrupts(bus);
                } else {
                    self.irq_poll_cd -= 1;
                }
            }

            let va = self.pc;
            let pa = self.jit_probe_fetch(bus, va);
            let ilen =
                pa.and_then(|pa| bus.fetch16(pa).ok())
                    .map_or(0, |insn| if insn & 3 == 3 { 4 } else { 2 });
            let (satp, mode) = self
                .sys
                .as_ref()
                .map_or((0, u8::MAX), |sys| (sys.satp, sys.mode as u8));
            let before = self.insn_count;
            match self.step(bus) {
                Ok(stop) => {
                    // Architectural exceptions do not retire. Every successful
                    // step does, including a host-serviced ECALL. A successful
                    // step must also have passed the identical fetch translation
                    // probed above; retain the guard rather than inventing a PA.
                    if self.insn_count != before {
                        if let Some(pa) = pa {
                            trace(InstructionTrace {
                                icount: self.insn_count,
                                va,
                                pa,
                                next_va: self.pc,
                                satp,
                                mode,
                                ilen,
                            });
                        }
                    }
                    if let Some(stop) = stop {
                        return stop;
                    }
                }
                Err(e) => {
                    if system {
                        self.exception_to_trap(e);
                    } else {
                        return StopReason::Trap(e);
                    }
                }
            }
        }
        StopReason::Budget
    }

    /// Execute the complete ordinary RV64I/M integer, control, scalar memory,
    /// and RVV families without materializing the per-instruction Cpu state
    /// contract. All other families return `Slow` without architectural
    /// mutation and are re-fetched by the authoritative complete decoder.
    #[inline(always)]
    fn scalar_t0_step<B: Bus>(
        &mut self,
        bus: &mut B,
        pc: u64,
        fetch_context: u64,
        fetch: &mut ScalarFetchCapability,
    ) -> Result<ScalarT0Step, Exception> {
        // Preserve the exact split-fetch semantics of `step`: a compressed
        // instruction can occupy the final halfword of a mapped page.
        let (pa, lo16, linear_fetch) = self.scalar_fetch16(bus, pc, fetch_context, fetch)?;
        let lo = u32::from(lo16);
        if lo & 3 != 3 {
            return self.scalar_t0_compressed(bus, lo as u16, pc);
        }
        let pc2 = pc.wrapping_add(2);
        let hi = if pc2 & 0xfff == 0 {
            self.scalar_fetch16(bus, pc2, fetch_context, fetch)?.1
        } else if let Some(address) = linear_fetch {
            u16::from_le(unsafe {
                // SAFETY: the low-half capability covers the remainder of
                // this page, and the split-page case was handled above.
                core::ptr::read_unaligned(core::ptr::with_exposed_provenance::<u16>(address + 2))
            })
        } else {
            bus.fetch16(pa + 2)?
        };
        let insn = lo | (u32::from(hi) << 16);
        let mut next_pc = pc.wrapping_add(4);

        match opcode(insn) {
            // LUI
            0x37 => self.wr(rd(insn), imm_u(insn) as u64),
            // AUIPC
            0x17 => self.wr(rd(insn), pc.wrapping_add(imm_u(insn) as u64)),
            // JAL
            0x6f => {
                self.wr(rd(insn), next_pc);
                next_pc = pc.wrapping_add(imm_j(insn) as u64);
            }
            // JALR
            0x67 => {
                let target = self.x[rs1(insn)].wrapping_add(imm_i(insn) as u64) & !1;
                self.wr(rd(insn), next_pc);
                next_pc = target;
            }
            // BRANCH
            0x63 => {
                let (a, b) = (self.x[rs1(insn)], self.x[rs2(insn)]);
                let taken = match funct3(insn) {
                    0 => a == b,
                    1 => a != b,
                    4 => (a as i64) < (b as i64),
                    5 => (a as i64) >= (b as i64),
                    6 => a < b,
                    7 => a >= b,
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                if taken {
                    next_pc = pc.wrapping_add(imm_b(insn) as u64);
                }
            }
            // LOAD
            0x03 => {
                let addr = self.x[rs1(insn)].wrapping_add(imm_i(insn) as u64);
                let val = match funct3(insn) {
                    0 => self.ld::<B, 1>(bus, addr)? as i8 as i64 as u64,
                    1 => self.ld::<B, 2>(bus, addr)? as i16 as i64 as u64,
                    2 => self.ld::<B, 4>(bus, addr)? as i32 as i64 as u64,
                    3 => self.ld::<B, 8>(bus, addr)?,
                    4 => self.ld::<B, 1>(bus, addr)?,
                    5 => self.ld::<B, 2>(bus, addr)?,
                    6 => self.ld::<B, 4>(bus, addr)?,
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val);
            }
            // STORE
            0x23 => {
                let addr = self.x[rs1(insn)].wrapping_add(imm_s(insn) as u64);
                let val = self.x[rs2(insn)];
                match funct3(insn) {
                    0 => self.st::<B, 1>(bus, addr, val)?,
                    1 => self.st::<B, 2>(bus, addr, val)?,
                    2 => self.st::<B, 4>(bus, addr, val)?,
                    3 => self.st::<B, 8>(bus, addr, val)?,
                    _ => return Err(Exception::IllegalInstruction { insn }),
                }
            }
            // OP-IMM
            0x13 => {
                let a = self.x[rs1(insn)];
                let imm = imm_i(insn) as u64;
                let shamt = (imm & 0x3f) as u32;
                let val = match funct3(insn) {
                    0 => a.wrapping_add(imm),
                    1 => a << shamt,
                    2 => ((a as i64) < (imm as i64)) as u64,
                    3 => (a < imm) as u64,
                    4 => a ^ imm,
                    5 => {
                        if insn >> 26 == 0x10 {
                            ((a as i64) >> shamt) as u64
                        } else {
                            a >> shamt
                        }
                    }
                    6 => a | imm,
                    7 => a & imm,
                    _ => unreachable!(),
                };
                self.wr(rd(insn), val);
            }
            // OP-IMM-32
            0x1b => {
                let a = self.x[rs1(insn)] as u32;
                let imm = imm_i(insn);
                let shamt = (imm & 0x1f) as u32;
                let val32 = match funct3(insn) {
                    0 => a.wrapping_add(imm as u32),
                    1 => a << shamt,
                    5 => {
                        if funct7(insn) == 0x20 {
                            ((a as i32) >> shamt) as u32
                        } else {
                            a >> shamt
                        }
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val32 as i32 as i64 as u64);
            }
            // OP
            0x33 => {
                let (a, b) = (self.x[rs1(insn)], self.x[rs2(insn)]);
                let shamt = (b & 0x3f) as u32;
                let val = match (funct7(insn), funct3(insn)) {
                    (0x00, 0) => a.wrapping_add(b),
                    (0x20, 0) => a.wrapping_sub(b),
                    (0x00, 1) => a << shamt,
                    (0x00, 2) => ((a as i64) < (b as i64)) as u64,
                    (0x00, 3) => (a < b) as u64,
                    (0x00, 4) => a ^ b,
                    (0x00, 5) => a >> shamt,
                    (0x20, 5) => ((a as i64) >> shamt) as u64,
                    (0x00, 6) => a | b,
                    (0x00, 7) => a & b,
                    (0x01, 0) => a.wrapping_mul(b),
                    (0x01, 1) => (((a as i64 as i128) * (b as i64 as i128)) >> 64) as u64,
                    (0x01, 2) => (((a as i64 as i128) * (b as u128 as i128)) >> 64) as u64,
                    (0x01, 3) => (((a as u128) * (b as u128)) >> 64) as u64,
                    (0x01, 4) => {
                        let (a, b) = (a as i64, b as i64);
                        if b == 0 {
                            u64::MAX
                        } else {
                            a.wrapping_div(b) as u64
                        }
                    }
                    (0x01, 5) => a.checked_div(b).unwrap_or(u64::MAX),
                    (0x01, 6) => {
                        let (a, b) = (a as i64, b as i64);
                        if b == 0 {
                            a as u64
                        } else {
                            a.wrapping_rem(b) as u64
                        }
                    }
                    (0x01, 7) => {
                        if b == 0 {
                            a
                        } else {
                            a % b
                        }
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val);
            }
            // OP-32
            0x3b => {
                let (a, b) = (self.x[rs1(insn)] as u32, self.x[rs2(insn)] as u32);
                let shamt = b & 0x1f;
                let val32 = match (funct7(insn), funct3(insn)) {
                    (0x00, 0) => a.wrapping_add(b),
                    (0x20, 0) => a.wrapping_sub(b),
                    (0x00, 1) => a << shamt,
                    (0x00, 5) => a >> shamt,
                    (0x20, 5) => ((a as i32) >> shamt) as u32,
                    (0x01, 0) => a.wrapping_mul(b),
                    (0x01, 4) => {
                        let (a, b) = (a as i32, b as i32);
                        if b == 0 {
                            u32::MAX
                        } else {
                            a.wrapping_div(b) as u32
                        }
                    }
                    (0x01, 5) => a.checked_div(b).unwrap_or(u32::MAX),
                    (0x01, 6) => {
                        let (a, b) = (a as i32, b as i32);
                        if b == 0 {
                            a as u32
                        } else {
                            a.wrapping_rem(b) as u32
                        }
                    }
                    (0x01, 7) => {
                        if b == 0 {
                            a
                        } else {
                            a % b
                        }
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val32 as i32 as i64 as u64);
            }
            // Vector memory and arithmetic use the same authoritative RVV
            // helpers as `step`, but can retire directly from the integrated
            // interpreter. This avoids a second fetch/decode and a full local
            // PC/retirement-state round trip for every vector instruction.
            0x07 if matches!(funct3(insn), 0 | 5 | 6 | 7) => {
                if vector::unit_stride_memory_encoding(insn) {
                    self.exec_unit_stride_memory_insn(bus, insn, true)?;
                } else {
                    self.exec_vector_memory(bus, insn, true)?;
                }
            }
            0x27 if matches!(funct3(insn), 0 | 5 | 6 | 7) => {
                if vector::unit_stride_memory_encoding(insn) {
                    self.exec_unit_stride_memory_insn(bus, insn, false)?;
                } else {
                    self.exec_vector_memory(bus, insn, false)?;
                }
            }
            0x57 => {
                if funct3(insn) == 7 {
                    self.exec_vector_config(insn)?;
                } else {
                    self.exec_vector_op(insn)?;
                }
            }
            _ => return Ok(ScalarT0Step::Slow),
        }

        Ok(ScalarT0Step::Retired(next_pc))
    }

    /// Execute an integer/scalar-memory RV64C instruction while keeping PC and
    /// retirement local to the integrated Tier-0 driver. Floating-point RVC
    /// forms deliberately return `Slow` to the complete decoder.
    #[inline(always)]
    fn scalar_t0_compressed<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u16,
        pc: u64,
    ) -> Result<ScalarT0Step, Exception> {
        let c = u32::from(instruction);
        if c == 0 {
            return Err(Exception::IllegalInstruction { insn: c });
        }
        let f3 = (c >> 13) & 7;
        let mut next_pc = pc.wrapping_add(2);

        match c & 3 {
            0 => {
                let rd = compressed_register((c >> 2) & 7);
                let rs1 = compressed_register((c >> 7) & 7);
                match f3 {
                    0b000 => {
                        let immediate = (((c >> 11) & 3) << 4)
                            | (((c >> 7) & 0xf) << 6)
                            | (((c >> 6) & 1) << 2)
                            | (((c >> 5) & 1) << 3);
                        if immediate == 0 {
                            return Err(Exception::IllegalInstruction { insn: c });
                        }
                        self.wr(rd, self.x[2].wrapping_add(u64::from(immediate)));
                    }
                    0b001 | 0b101 => return Ok(ScalarT0Step::Slow),
                    0b010 => {
                        let immediate =
                            (((c >> 10) & 7) << 3) | (((c >> 6) & 1) << 2) | (((c >> 5) & 1) << 6);
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        let value = self.ld::<B, 4>(bus, address)? as u32 as i32 as i64 as u64;
                        self.wr(rd, value);
                    }
                    0b011 => {
                        let immediate = (((c >> 10) & 7) << 3) | (((c >> 5) & 3) << 6);
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        let value = self.ld::<B, 8>(bus, address)?;
                        self.wr(rd, value);
                    }
                    0b110 => {
                        let immediate =
                            (((c >> 10) & 7) << 3) | (((c >> 6) & 1) << 2) | (((c >> 5) & 1) << 6);
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        self.st::<B, 4>(bus, address, self.x[rd])?;
                    }
                    0b111 => {
                        let immediate = (((c >> 10) & 7) << 3) | (((c >> 5) & 3) << 6);
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        self.st::<B, 8>(bus, address, self.x[rd])?;
                    }
                    _ => return Err(Exception::IllegalInstruction { insn: c }),
                }
            }
            1 => {
                let rd = ((c >> 7) & 0x1f) as usize;
                let immediate = compressed_sext((((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f), 6);
                match f3 {
                    0b000 => self.wr(rd, self.x[rd].wrapping_add(immediate as u64)),
                    0b001 => {
                        if rd == 0 {
                            return Err(Exception::IllegalInstruction { insn: c });
                        }
                        let value =
                            (self.x[rd] as u32).wrapping_add(immediate as u32) as i32 as i64 as u64;
                        self.wr(rd, value);
                    }
                    0b010 => self.wr(rd, immediate as u64),
                    0b011 => {
                        if rd == 2 {
                            let value = (((c >> 12) & 1) << 9)
                                | (((c >> 6) & 1) << 4)
                                | (((c >> 5) & 1) << 6)
                                | (((c >> 3) & 3) << 7)
                                | (((c >> 2) & 1) << 5);
                            let value = compressed_sext(value, 10);
                            if value == 0 {
                                return Err(Exception::IllegalInstruction { insn: c });
                            }
                            self.x[2] = self.x[2].wrapping_add(value as u64);
                        } else {
                            if immediate == 0 {
                                return Err(Exception::IllegalInstruction { insn: c });
                            }
                            self.wr(rd, (immediate << 12) as u64);
                        }
                    }
                    0b100 => {
                        let rdp = compressed_register((c >> 7) & 7);
                        match (c >> 10) & 3 {
                            0 => {
                                let shift = (((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f);
                                self.x[rdp] >>= shift;
                            }
                            1 => {
                                let shift = (((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f);
                                self.x[rdp] = ((self.x[rdp] as i64) >> shift) as u64;
                            }
                            2 => self.x[rdp] &= immediate as u64,
                            _ => {
                                let rs2 = compressed_register((c >> 2) & 7);
                                match (((c >> 12) & 1) << 2) | ((c >> 5) & 3) {
                                    0b000 => self.x[rdp] = self.x[rdp].wrapping_sub(self.x[rs2]),
                                    0b001 => self.x[rdp] ^= self.x[rs2],
                                    0b010 => self.x[rdp] |= self.x[rs2],
                                    0b011 => self.x[rdp] &= self.x[rs2],
                                    0b100 => {
                                        self.x[rdp] = (self.x[rdp] as u32)
                                            .wrapping_sub(self.x[rs2] as u32)
                                            as i32
                                            as i64
                                            as u64;
                                    }
                                    0b101 => {
                                        self.x[rdp] = (self.x[rdp] as u32)
                                            .wrapping_add(self.x[rs2] as u32)
                                            as i32
                                            as i64
                                            as u64;
                                    }
                                    _ => return Err(Exception::IllegalInstruction { insn: c }),
                                }
                            }
                        }
                    }
                    0b101 => {
                        let immediate = compressed_sext(
                            (((c >> 12) & 1) << 11)
                                | (((c >> 11) & 1) << 4)
                                | (((c >> 9) & 3) << 8)
                                | (((c >> 8) & 1) << 10)
                                | (((c >> 7) & 1) << 6)
                                | (((c >> 6) & 1) << 7)
                                | (((c >> 3) & 7) << 1)
                                | (((c >> 2) & 1) << 5),
                            12,
                        );
                        next_pc = pc.wrapping_add(immediate as u64);
                    }
                    0b110 | 0b111 => {
                        let rs1 = compressed_register((c >> 7) & 7);
                        let immediate = compressed_sext(
                            (((c >> 12) & 1) << 8)
                                | (((c >> 10) & 3) << 3)
                                | (((c >> 5) & 3) << 6)
                                | (((c >> 3) & 3) << 1)
                                | (((c >> 2) & 1) << 5),
                            9,
                        );
                        let taken = if f3 == 0b110 {
                            self.x[rs1] == 0
                        } else {
                            self.x[rs1] != 0
                        };
                        if taken {
                            next_pc = pc.wrapping_add(immediate as u64);
                        }
                    }
                    _ => return Err(Exception::IllegalInstruction { insn: c }),
                }
            }
            2 => {
                let rd = ((c >> 7) & 0x1f) as usize;
                let rs2 = ((c >> 2) & 0x1f) as usize;
                match f3 {
                    0b000 => {
                        let shift = (((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f);
                        self.wr(rd, self.x[rd] << shift);
                    }
                    0b001 | 0b101 => return Ok(ScalarT0Step::Slow),
                    0b010 => {
                        if rd == 0 {
                            return Err(Exception::IllegalInstruction { insn: c });
                        }
                        let immediate =
                            (((c >> 12) & 1) << 5) | (((c >> 4) & 7) << 2) | (((c >> 2) & 3) << 6);
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        let value = self.ld::<B, 4>(bus, address)? as u32 as i32 as i64 as u64;
                        self.wr(rd, value);
                    }
                    0b011 => {
                        if rd == 0 {
                            return Err(Exception::IllegalInstruction { insn: c });
                        }
                        let immediate =
                            (((c >> 12) & 1) << 5) | (((c >> 5) & 3) << 3) | (((c >> 2) & 7) << 6);
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        let value = self.ld::<B, 8>(bus, address)?;
                        self.wr(rd, value);
                    }
                    0b100 => {
                        let bit12 = (c >> 12) & 1;
                        match (bit12, rd, rs2) {
                            (0, 0, _) => return Err(Exception::IllegalInstruction { insn: c }),
                            (0, _, 0) => next_pc = self.x[rd] & !1,
                            (0, _, _) => self.wr(rd, self.x[rs2]),
                            (1, 0, 0) => {
                                if self.sys.is_some() {
                                    return Err(Exception::Breakpoint);
                                }
                                return Ok(ScalarT0Step::Stop(next_pc, StopReason::Break));
                            }
                            (1, _, 0) => {
                                let target = self.x[rd] & !1;
                                self.x[1] = next_pc;
                                next_pc = target;
                            }
                            (1, _, _) => self.wr(rd, self.x[rd].wrapping_add(self.x[rs2])),
                            _ => unreachable!(),
                        }
                    }
                    0b110 => {
                        let immediate = (((c >> 9) & 0xf) << 2) | (((c >> 7) & 3) << 6);
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        self.st::<B, 4>(bus, address, self.x[rs2])?;
                    }
                    0b111 => {
                        let immediate = (((c >> 10) & 7) << 3) | (((c >> 7) & 7) << 6);
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        self.st::<B, 8>(bus, address, self.x[rs2])?;
                    }
                    _ => return Err(Exception::IllegalInstruction { insn: c }),
                }
            }
            _ => unreachable!("compressed path received a 32-bit instruction prefix"),
        }

        Ok(ScalarT0Step::Retired(next_pc))
    }

    /// Execute one RV64C instruction directly. Expanding RVC into a synthetic
    /// 32-bit instruction is an excellent single-source correctness model, but
    /// it makes the interpreter encode fields only to decode them again. This
    /// path keeps identical architectural operations while removing that
    /// intermediate representation from the measured interpreter hot path.
    #[inline(always)]
    fn step_compressed<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u16,
    ) -> Result<Option<StopReason>, Exception> {
        let c = u32::from(instruction);
        if c == 0 {
            return Err(Exception::IllegalInstruction { insn: c });
        }
        let f3 = (c >> 13) & 7;
        let mut next_pc = self.pc.wrapping_add(2);
        let mut stop = None;

        match c & 3 {
            0 => {
                let rd = compressed_register((c >> 2) & 7);
                let rs1 = compressed_register((c >> 7) & 7);
                match f3 {
                    0b000 => {
                        let immediate = (((c >> 11) & 3) << 4)
                            | (((c >> 7) & 0xf) << 6)
                            | (((c >> 6) & 1) << 2)
                            | (((c >> 5) & 1) << 3);
                        if immediate == 0 {
                            return Err(Exception::IllegalInstruction { insn: c });
                        }
                        self.wr(rd, self.x[2].wrapping_add(u64::from(immediate)));
                    }
                    0b001 => {
                        let immediate = (((c >> 10) & 7) << 3) | (((c >> 5) & 3) << 6);
                        let expanded = crate::compressed::expand(instruction)
                            .expect("valid C.FLD must expand");
                        self.fp_check(expanded)?;
                        self.fp_dirty();
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        self.f[rd] = self.ld::<B, 8>(bus, address)?;
                    }
                    0b010 => {
                        let immediate =
                            (((c >> 10) & 7) << 3) | (((c >> 6) & 1) << 2) | (((c >> 5) & 1) << 6);
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        let value = self.ld::<B, 4>(bus, address)? as u32 as i32 as i64 as u64;
                        self.wr(rd, value);
                    }
                    0b011 => {
                        let immediate = (((c >> 10) & 7) << 3) | (((c >> 5) & 3) << 6);
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        let value = self.ld::<B, 8>(bus, address)?;
                        self.wr(rd, value);
                    }
                    0b101 => {
                        let immediate = (((c >> 10) & 7) << 3) | (((c >> 5) & 3) << 6);
                        let expanded = crate::compressed::expand(instruction)
                            .expect("valid C.FSD must expand");
                        self.fp_check(expanded)?;
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        self.st::<B, 8>(bus, address, self.f[rd])?;
                    }
                    0b110 => {
                        let immediate =
                            (((c >> 10) & 7) << 3) | (((c >> 6) & 1) << 2) | (((c >> 5) & 1) << 6);
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        self.st::<B, 4>(bus, address, self.x[rd])?;
                    }
                    0b111 => {
                        let immediate = (((c >> 10) & 7) << 3) | (((c >> 5) & 3) << 6);
                        let address = self.x[rs1].wrapping_add(u64::from(immediate));
                        self.st::<B, 8>(bus, address, self.x[rd])?;
                    }
                    _ => return Err(Exception::IllegalInstruction { insn: c }),
                }
            }
            1 => {
                let rd = ((c >> 7) & 0x1f) as usize;
                let immediate = compressed_sext((((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f), 6);
                match f3 {
                    0b000 => self.wr(rd, self.x[rd].wrapping_add(immediate as u64)),
                    0b001 => {
                        if rd == 0 {
                            return Err(Exception::IllegalInstruction { insn: c });
                        }
                        let value =
                            (self.x[rd] as u32).wrapping_add(immediate as u32) as i32 as i64 as u64;
                        self.wr(rd, value);
                    }
                    0b010 => self.wr(rd, immediate as u64),
                    0b011 => {
                        if rd == 2 {
                            let value = (((c >> 12) & 1) << 9)
                                | (((c >> 6) & 1) << 4)
                                | (((c >> 5) & 1) << 6)
                                | (((c >> 3) & 3) << 7)
                                | (((c >> 2) & 1) << 5);
                            let value = compressed_sext(value, 10);
                            if value == 0 {
                                return Err(Exception::IllegalInstruction { insn: c });
                            }
                            self.x[2] = self.x[2].wrapping_add(value as u64);
                        } else {
                            if immediate == 0 {
                                return Err(Exception::IllegalInstruction { insn: c });
                            }
                            self.wr(rd, (immediate << 12) as u64);
                        }
                    }
                    0b100 => {
                        let rdp = compressed_register((c >> 7) & 7);
                        match (c >> 10) & 3 {
                            0 => {
                                let shift = (((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f);
                                self.x[rdp] >>= shift;
                            }
                            1 => {
                                let shift = (((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f);
                                self.x[rdp] = ((self.x[rdp] as i64) >> shift) as u64;
                            }
                            2 => self.x[rdp] &= immediate as u64,
                            _ => {
                                let rs2 = compressed_register((c >> 2) & 7);
                                match (((c >> 12) & 1) << 2) | ((c >> 5) & 3) {
                                    0b000 => self.x[rdp] = self.x[rdp].wrapping_sub(self.x[rs2]),
                                    0b001 => self.x[rdp] ^= self.x[rs2],
                                    0b010 => self.x[rdp] |= self.x[rs2],
                                    0b011 => self.x[rdp] &= self.x[rs2],
                                    0b100 => {
                                        self.x[rdp] = (self.x[rdp] as u32)
                                            .wrapping_sub(self.x[rs2] as u32)
                                            as i32
                                            as i64
                                            as u64;
                                    }
                                    0b101 => {
                                        self.x[rdp] = (self.x[rdp] as u32)
                                            .wrapping_add(self.x[rs2] as u32)
                                            as i32
                                            as i64
                                            as u64;
                                    }
                                    _ => return Err(Exception::IllegalInstruction { insn: c }),
                                }
                            }
                        }
                    }
                    0b101 => {
                        let immediate = compressed_sext(
                            (((c >> 12) & 1) << 11)
                                | (((c >> 11) & 1) << 4)
                                | (((c >> 9) & 3) << 8)
                                | (((c >> 8) & 1) << 10)
                                | (((c >> 7) & 1) << 6)
                                | (((c >> 6) & 1) << 7)
                                | (((c >> 3) & 7) << 1)
                                | (((c >> 2) & 1) << 5),
                            12,
                        );
                        next_pc = self.pc.wrapping_add(immediate as u64);
                    }
                    0b110 | 0b111 => {
                        let rs1 = compressed_register((c >> 7) & 7);
                        let immediate = compressed_sext(
                            (((c >> 12) & 1) << 8)
                                | (((c >> 10) & 3) << 3)
                                | (((c >> 5) & 3) << 6)
                                | (((c >> 3) & 3) << 1)
                                | (((c >> 2) & 1) << 5),
                            9,
                        );
                        let taken = if f3 == 0b110 {
                            self.x[rs1] == 0
                        } else {
                            self.x[rs1] != 0
                        };
                        if taken {
                            next_pc = self.pc.wrapping_add(immediate as u64);
                        }
                    }
                    _ => return Err(Exception::IllegalInstruction { insn: c }),
                }
            }
            2 => {
                let rd = ((c >> 7) & 0x1f) as usize;
                let rs2 = ((c >> 2) & 0x1f) as usize;
                match f3 {
                    0b000 => {
                        let shift = (((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f);
                        self.wr(rd, self.x[rd] << shift);
                    }
                    0b001 => {
                        let immediate =
                            (((c >> 12) & 1) << 5) | (((c >> 5) & 3) << 3) | (((c >> 2) & 7) << 6);
                        let expanded = crate::compressed::expand(instruction)
                            .expect("valid C.FLDSP must expand");
                        self.fp_check(expanded)?;
                        self.fp_dirty();
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        self.f[rd] = self.ld::<B, 8>(bus, address)?;
                    }
                    0b010 => {
                        if rd == 0 {
                            return Err(Exception::IllegalInstruction { insn: c });
                        }
                        let immediate =
                            (((c >> 12) & 1) << 5) | (((c >> 4) & 7) << 2) | (((c >> 2) & 3) << 6);
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        let value = self.ld::<B, 4>(bus, address)? as u32 as i32 as i64 as u64;
                        self.wr(rd, value);
                    }
                    0b011 => {
                        if rd == 0 {
                            return Err(Exception::IllegalInstruction { insn: c });
                        }
                        let immediate =
                            (((c >> 12) & 1) << 5) | (((c >> 5) & 3) << 3) | (((c >> 2) & 7) << 6);
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        let value = self.ld::<B, 8>(bus, address)?;
                        self.wr(rd, value);
                    }
                    0b100 => {
                        let bit12 = (c >> 12) & 1;
                        match (bit12, rd, rs2) {
                            (0, 0, _) => return Err(Exception::IllegalInstruction { insn: c }),
                            (0, _, 0) => next_pc = self.x[rd] & !1,
                            (0, _, _) => self.wr(rd, self.x[rs2]),
                            (1, 0, 0) => {
                                if self.sys.is_some() {
                                    return Err(Exception::Breakpoint);
                                }
                                stop = Some(StopReason::Break);
                            }
                            (1, _, 0) => {
                                let target = self.x[rd] & !1;
                                self.x[1] = next_pc;
                                next_pc = target;
                            }
                            (1, _, _) => self.wr(rd, self.x[rd].wrapping_add(self.x[rs2])),
                            _ => unreachable!(),
                        }
                    }
                    0b101 => {
                        let immediate = (((c >> 10) & 7) << 3) | (((c >> 7) & 7) << 6);
                        let expanded = crate::compressed::expand(instruction)
                            .expect("valid C.FSDSP must expand");
                        self.fp_check(expanded)?;
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        self.st::<B, 8>(bus, address, self.f[rs2])?;
                    }
                    0b110 => {
                        let immediate = (((c >> 9) & 0xf) << 2) | (((c >> 7) & 3) << 6);
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        self.st::<B, 4>(bus, address, self.x[rs2])?;
                    }
                    0b111 => {
                        let immediate = (((c >> 10) & 7) << 3) | (((c >> 7) & 7) << 6);
                        let address = self.x[2].wrapping_add(u64::from(immediate));
                        self.st::<B, 8>(bus, address, self.x[rs2])?;
                    }
                    _ => return Err(Exception::IllegalInstruction { insn: c }),
                }
            }
            _ => unreachable!("compressed path received a 32-bit instruction prefix"),
        }

        self.pc = next_pc;
        self.insn_count += 1;
        Ok(stop)
    }

    /// Execute one instruction. `Ok(Some(_))` = clean stop (ecall/ebreak),
    /// `Err` = exception. PC already points at the *next* instruction when
    /// Ecall/Break is returned, so the host can service and resume directly.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<Option<StopReason>, Exception> {
        // Fetch 16 bits first: a compressed instruction may sit on the last
        // halfword of a page/region, where a blind 32-bit fetch would fault.
        let pa = self.translate(bus, self.pc, Access::Fetch)?;
        let lo = bus.fetch16(pa)? as u32;
        if lo & 3 != 3 {
            return self.step_compressed(bus, lo as u16);
        }
        let pc2 = self.pc.wrapping_add(2);
        let pa2 = if pc2 & 0xfff == 0 {
            self.translate(bus, pc2, Access::Fetch)?
        } else {
            pa + 2
        };
        let hi = bus.fetch16(pa2)? as u32;
        let insn = lo | (hi << 16);
        let mut next_pc = self.pc.wrapping_add(4);
        let mut stop = None;

        match opcode(insn) {
            // LUI
            0x37 => self.wr(rd(insn), imm_u(insn) as u64),
            // AUIPC
            0x17 => self.wr(rd(insn), self.pc.wrapping_add(imm_u(insn) as u64)),
            // JAL
            0x6f => {
                self.wr(rd(insn), next_pc);
                next_pc = self.pc.wrapping_add(imm_j(insn) as u64);
            }
            // JALR
            0x67 => {
                let target = self.x[rs1(insn)].wrapping_add(imm_i(insn) as u64) & !1;
                self.wr(rd(insn), next_pc);
                next_pc = target;
            }
            // BRANCH
            0x63 => {
                let (a, b) = (self.x[rs1(insn)], self.x[rs2(insn)]);
                let taken = match funct3(insn) {
                    0 => a == b,                   // BEQ
                    1 => a != b,                   // BNE
                    4 => (a as i64) < (b as i64),  // BLT
                    5 => (a as i64) >= (b as i64), // BGE
                    6 => a < b,                    // BLTU
                    7 => a >= b,                   // BGEU
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                if taken {
                    next_pc = self.pc.wrapping_add(imm_b(insn) as u64);
                }
            }
            // LOAD
            0x03 => {
                let addr = self.x[rs1(insn)].wrapping_add(imm_i(insn) as u64);
                let val = match funct3(insn) {
                    0 => self.ld::<B, 1>(bus, addr)? as i8 as i64 as u64, // LB
                    1 => self.ld::<B, 2>(bus, addr)? as i16 as i64 as u64, // LH
                    2 => self.ld::<B, 4>(bus, addr)? as i32 as i64 as u64, // LW
                    3 => self.ld::<B, 8>(bus, addr)?,                     // LD
                    4 => self.ld::<B, 1>(bus, addr)?,                     // LBU
                    5 => self.ld::<B, 2>(bus, addr)?,                     // LHU
                    6 => self.ld::<B, 4>(bus, addr)?,                     // LWU
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val);
            }
            // STORE
            0x23 => {
                let addr = self.x[rs1(insn)].wrapping_add(imm_s(insn) as u64);
                let val = self.x[rs2(insn)];
                match funct3(insn) {
                    0 => self.st::<B, 1>(bus, addr, val)?, // SB
                    1 => self.st::<B, 2>(bus, addr, val)?, // SH
                    2 => self.st::<B, 4>(bus, addr, val)?, // SW
                    3 => self.st::<B, 8>(bus, addr, val)?, // SD
                    _ => return Err(Exception::IllegalInstruction { insn }),
                }
            }
            // OP-IMM
            0x13 => {
                let a = self.x[rs1(insn)];
                let imm = imm_i(insn) as u64;
                let shamt = (imm & 0x3f) as u32;
                let val = match funct3(insn) {
                    0 => a.wrapping_add(imm),                // ADDI
                    1 => a << shamt,                         // SLLI
                    2 => ((a as i64) < (imm as i64)) as u64, // SLTI
                    3 => (a < imm) as u64,                   // SLTIU
                    4 => a ^ imm,                            // XORI
                    5 => {
                        if insn >> 26 == 0x10 {
                            ((a as i64) >> shamt) as u64 // SRAI
                        } else {
                            a >> shamt // SRLI
                        }
                    }
                    6 => a | imm, // ORI
                    7 => a & imm, // ANDI
                    _ => unreachable!(),
                };
                self.wr(rd(insn), val);
            }
            // OP-IMM-32 (ADDIW/SLLIW/SRLIW/SRAIW)
            0x1b => {
                let a = self.x[rs1(insn)] as u32;
                let imm = imm_i(insn);
                let shamt = (imm & 0x1f) as u32;
                let val32 = match funct3(insn) {
                    0 => a.wrapping_add(imm as u32),
                    1 => a << shamt,
                    5 => {
                        if funct7(insn) == 0x20 {
                            ((a as i32) >> shamt) as u32
                        } else {
                            a >> shamt
                        }
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val32 as i32 as i64 as u64);
            }
            // OP
            0x33 => {
                let (a, b) = (self.x[rs1(insn)], self.x[rs2(insn)]);
                let shamt = (b & 0x3f) as u32;
                let val = match (funct7(insn), funct3(insn)) {
                    (0x00, 0) => a.wrapping_add(b),                // ADD
                    (0x20, 0) => a.wrapping_sub(b),                // SUB
                    (0x00, 1) => a << shamt,                       // SLL
                    (0x00, 2) => ((a as i64) < (b as i64)) as u64, // SLT
                    (0x00, 3) => (a < b) as u64,                   // SLTU
                    (0x00, 4) => a ^ b,                            // XOR
                    (0x00, 5) => a >> shamt,                       // SRL
                    (0x20, 5) => ((a as i64) >> shamt) as u64,     // SRA
                    (0x00, 6) => a | b,                            // OR
                    (0x00, 7) => a & b,                            // AND
                    // M extension
                    (0x01, 0) => a.wrapping_mul(b), // MUL
                    (0x01, 1) => {
                        (((a as i64 as i128) * (b as i64 as i128)) >> 64) as u64
                        // MULH
                    }
                    (0x01, 2) => {
                        (((a as i64 as i128) * (b as u128 as i128)) >> 64) as u64
                        // MULHSU
                    }
                    (0x01, 3) => (((a as u128) * (b as u128)) >> 64) as u64, // MULHU
                    (0x01, 4) => {
                        // DIV: div-by-zero -> -1; overflow MIN/-1 -> MIN
                        let (a, b) = (a as i64, b as i64);
                        if b == 0 {
                            u64::MAX
                        } else {
                            a.wrapping_div(b) as u64
                        }
                    }
                    (0x01, 5) => {
                        a.checked_div(b).unwrap_or(u64::MAX) // DIVU
                    }
                    (0x01, 6) => {
                        let (a, b) = (a as i64, b as i64);
                        if b == 0 {
                            a as u64
                        } else {
                            a.wrapping_rem(b) as u64
                        } // REM
                    }
                    (0x01, 7) => {
                        if b == 0 {
                            a
                        } else {
                            a % b
                        } // REMU
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val);
            }
            // OP-32 (ADDW/SUBW/SLLW/SRLW/SRAW)
            0x3b => {
                let (a, b) = (self.x[rs1(insn)] as u32, self.x[rs2(insn)] as u32);
                let shamt = b & 0x1f;
                let val32 = match (funct7(insn), funct3(insn)) {
                    (0x00, 0) => a.wrapping_add(b),
                    (0x20, 0) => a.wrapping_sub(b),
                    (0x00, 1) => a << shamt,
                    (0x00, 5) => a >> shamt,
                    (0x20, 5) => ((a as i32) >> shamt) as u32,
                    // M extension (32-bit forms)
                    (0x01, 0) => a.wrapping_mul(b), // MULW
                    (0x01, 4) => {
                        let (a, b) = (a as i32, b as i32);
                        if b == 0 {
                            u32::MAX
                        } else {
                            a.wrapping_div(b) as u32
                        } // DIVW
                    }
                    (0x01, 5) => {
                        a.checked_div(b).unwrap_or(u32::MAX) // DIVUW
                    }
                    (0x01, 6) => {
                        let (a, b) = (a as i32, b as i32);
                        if b == 0 {
                            a as u32
                        } else {
                            a.wrapping_rem(b) as u32
                        } // REMW
                    }
                    (0x01, 7) => {
                        if b == 0 {
                            a
                        } else {
                            a % b
                        } // REMUW
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val32 as i32 as i64 as u64);
            }
            // AMO (A extension). Single hart: LR sets a reservation, SC
            // succeeds iff it matches; AMOs are read-modify-write.
            0x2f => {
                let addr = self.x[rs1(insn)];
                let src = self.x[rs2(insn)];
                let funct5 = funct7(insn) >> 2;
                let is64 = match funct3(insn) {
                    2 => false,
                    3 => true,
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                macro_rules! aload {
                    () => {
                        if is64 {
                            self.ld::<B, 8>(bus, addr)?
                        } else {
                            self.ld::<B, 4>(bus, addr)? as i32 as i64 as u64
                        }
                    };
                }
                macro_rules! astore {
                    ($v:expr) => {
                        if is64 {
                            self.st::<B, 8>(bus, addr, $v)?
                        } else {
                            self.st::<B, 4>(bus, addr, $v)?
                        }
                    };
                }
                match funct5 {
                    0x02 => {
                        // LR
                        let v = aload!();
                        self.reservation = Some(addr);
                        self.wr(rd(insn), v);
                    }
                    0x03 => {
                        // SC
                        if self.reservation == Some(addr) {
                            astore!(src);
                            self.wr(rd(insn), 0);
                        } else {
                            self.wr(rd(insn), 1);
                        }
                        self.reservation = None;
                    }
                    _ => {
                        let old = aload!();
                        // 32-bit AMOs compare/compute on the low 32 bits
                        // only (the register's high bits are ignored).
                        let (co, cs) = if is64 {
                            (old, src)
                        } else {
                            (old as u32 as u64, src as u32 as u64)
                        };
                        let signed_lt = if is64 {
                            (co as i64) < (cs as i64)
                        } else {
                            (co as u32 as i32) < (cs as u32 as i32)
                        };
                        let new = match funct5 {
                            0x01 => src,                   // AMOSWAP
                            0x00 => old.wrapping_add(src), // AMOADD
                            0x04 => old ^ src,             // AMOXOR
                            0x0c => old & src,             // AMOAND
                            0x08 => old | src,             // AMOOR
                            0x10 => {
                                if signed_lt {
                                    old
                                } else {
                                    src
                                }
                            } // AMOMIN
                            0x14 => {
                                if !signed_lt && co != cs {
                                    old
                                } else {
                                    src
                                }
                            } // AMOMAX
                            0x18 => {
                                if co < cs {
                                    old
                                } else {
                                    src
                                }
                            } // AMOMINU
                            0x1c => {
                                if co > cs {
                                    old
                                } else {
                                    src
                                }
                            } // AMOMAXU
                            _ => return Err(Exception::IllegalInstruction { insn }),
                        };
                        // 32-bit AMOs operate on the sign-extended old value
                        // but store only the low 32 bits.
                        astore!(new);
                        self.wr(rd(insn), old);
                    }
                }
            }
            // LOAD-FP (FLW/FLD) or vector load. Vector memory operations
            // share the major opcode but use the otherwise-unused width
            // encodings 000/101/110/111.
            0x07 => {
                if matches!(funct3(insn), 0 | 5 | 6 | 7) {
                    if vector::unit_stride_memory_encoding(insn) {
                        self.exec_unit_stride_memory_insn(bus, insn, true)?;
                    } else {
                        self.exec_vector_memory(bus, insn, true)?;
                    }
                } else {
                    self.fp_check(insn)?;
                    self.fp_dirty();
                    let addr = self.x[rs1(insn)].wrapping_add(imm_i(insn) as u64);
                    self.f[rd(insn)] = match funct3(insn) {
                        2 => box32(f32::from_bits(self.ld::<B, 4>(bus, addr)? as u32)),
                        3 => self.ld::<B, 8>(bus, addr)?,
                        _ => return Err(Exception::IllegalInstruction { insn }),
                    };
                }
            }
            // STORE-FP (FSW/FSD) or vector store.
            0x27 => {
                if matches!(funct3(insn), 0 | 5 | 6 | 7) {
                    if vector::unit_stride_memory_encoding(insn) {
                        self.exec_unit_stride_memory_insn(bus, insn, false)?;
                    } else {
                        self.exec_vector_memory(bus, insn, false)?;
                    }
                } else {
                    self.fp_check(insn)?;
                    let addr = self.x[rs1(insn)].wrapping_add(imm_s(insn) as u64);
                    let v = self.f[rs2(insn)];
                    match funct3(insn) {
                        2 => self.st::<B, 4>(bus, addr, v)?,
                        3 => self.st::<B, 8>(bus, addr, v)?,
                        _ => return Err(Exception::IllegalInstruction { insn }),
                    }
                }
            }
            // OP-V (RVV 1.0 configuration and vector arithmetic).
            0x57 => {
                if funct3(insn) == 7 {
                    self.exec_vector_config(insn)?;
                } else {
                    self.exec_vector_op(insn)?;
                }
            }
            // FMADD/FMSUB/FNMSUB/FNMADD (softfloat: exact flags)
            0x43 | 0x47 | 0x4b | 0x4f => {
                self.fp_check(insn)?;
                self.fp_dirty();
                use crate::softfp::{sf32, sf64};
                let rs3 = (insn >> 27) as usize;
                let neg_prod = opcode(insn) == 0x4b || opcode(insn) == 0x4f;
                let neg_c = opcode(insn) == 0x47 || opcode(insn) == 0x4f;
                let rm = self
                    .get_rm(funct3(insn))
                    .ok_or(Exception::IllegalInstruction { insn })?;
                let mut fl: u32 = 0;
                match (insn >> 25) & 3 {
                    0 => {
                        let ub = |r: u64| -> u32 {
                            if r >> 32 == 0xffff_ffff {
                                r as u32
                            } else {
                                0x7fc0_0000
                            }
                        };
                        let mut a = ub(self.f[rs1(insn)]);
                        let b = ub(self.f[rs2(insn)]);
                        let mut c = ub(self.f[rs3]);
                        if neg_prod {
                            a ^= 0x8000_0000;
                        }
                        if neg_c {
                            c ^= 0x8000_0000;
                        }
                        let r = sf32::fma(a, b, c, rm, &mut fl);
                        self.f[rd(insn)] = 0xffff_ffff_0000_0000 | r as u64;
                    }
                    1 => {
                        let mut a = self.f[rs1(insn)];
                        let b = self.f[rs2(insn)];
                        let mut c = self.f[rs3];
                        if neg_prod {
                            a ^= 1 << 63;
                        }
                        if neg_c {
                            c ^= 1 << 63;
                        }
                        self.f[rd(insn)] = sf64::fma(a, b, c, rm, &mut fl);
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                }
                self.fcsr |= fl & 0x1f;
            }
            // OP-FP
            0x53 => {
                self.fp_check(insn)?;
                self.fp_dirty();
                self.op_fp(insn)?
            }
            // MISC-MEM: FENCE/FENCE.I — no-ops for a single in-order hart
            0x0f => {}
            // SYSTEM
            0x73 => match (insn, funct3(insn)) {
                (0x0000_0073, _) => {
                    if let Some(sys) = self.sys.as_ref() {
                        if self.host_sbi && sys.mode == Mode::Supervisor {
                            stop = Some(StopReason::Ecall);
                        } else {
                            let cause = match sys.mode {
                                Mode::User => 8,
                                Mode::Supervisor => 9,
                                Mode::Machine => 11,
                            };
                            self.take_trap(cause, 0, false);
                            self.insn_count += 1;
                            return Ok(None); // pc set by take_trap
                        }
                    }
                    if self.sys.is_none() {
                        stop = Some(StopReason::Ecall);
                    }
                }
                (0x0010_0073, _) => {
                    if self.sys.is_some() {
                        return Err(Exception::Breakpoint); // routed to trap
                    }
                    stop = Some(StopReason::Break);
                }
                // MRET
                (0x3020_0073, _) => {
                    self.irq_poll_cd = 0; // MPIE restores interrupt enable
                    let sys = self
                        .sys
                        .as_mut()
                        .ok_or(Exception::IllegalInstruction { insn })?;
                    if sys.mode != Mode::Machine {
                        return Err(Exception::IllegalInstruction { insn });
                    }
                    let mpp = Mode::from_bits((sys.mstatus & MSTATUS_MPP) >> 11);
                    let mpie = (sys.mstatus >> 7) & 1;
                    sys.mstatus = (sys.mstatus & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP))
                        | (mpie << 3)
                        | MSTATUS_MPIE;
                    if mpp != Mode::Machine {
                        sys.mstatus &= !MSTATUS_MPRV;
                    }
                    sys.mode = mpp;
                    next_pc = sys.mepc;
                    self.privilege_changed();
                }
                // SRET
                (0x1020_0073, _) => {
                    self.irq_poll_cd = 0; // SPIE restores interrupt enable
                    let sys = self
                        .sys
                        .as_mut()
                        .ok_or(Exception::IllegalInstruction { insn })?;
                    if sys.mode == Mode::User
                        || (sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TSR != 0)
                    {
                        return Err(Exception::IllegalInstruction { insn });
                    }
                    let spp = if sys.mstatus & MSTATUS_SPP != 0 {
                        Mode::Supervisor
                    } else {
                        Mode::User
                    };
                    let spie = (sys.mstatus >> 5) & 1;
                    sys.mstatus = (sys.mstatus & !(MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP))
                        | (spie << 1)
                        | MSTATUS_SPIE;
                    if spp != Mode::Machine {
                        sys.mstatus &= !MSTATUS_MPRV;
                    }
                    sys.mode = spp;
                    next_pc = sys.sepc;
                    self.privilege_changed();
                }
                // WFI: report to host if nothing pending (host may idle).
                (0x1050_0073, _) => {
                    if let Some(sys) = self.sys.as_ref() {
                        // U-mode WFI is illegal; S-mode WFI traps when TW=1.
                        if sys.mode == Mode::User
                            || (sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TW != 0)
                        {
                            return Err(Exception::IllegalInstruction { insn });
                        }
                        if sys.mip & sys.mie == 0 {
                            stop = Some(StopReason::Wfi);
                        }
                    }
                }
                // SFENCE.VMA (funct7 = 0x09, f3 = 0)
                _ if funct7(insn) == 0x09 && funct3(insn) == 0 => {
                    if let Some(sys) = self.sys.as_ref() {
                        // U-mode always traps; S-mode traps when TVM=1.
                        if sys.mode == Mode::User
                            || (sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TVM != 0)
                        {
                            return Err(Exception::IllegalInstruction { insn });
                        }
                    }
                    let va = (rs1(insn) != 0).then(|| self.x[rs1(insn)]);
                    let asid = (rs2(insn) != 0).then(|| self.x[rs2(insn)] as u16);
                    self.sfence_vma(va, asid);
                    // NOTE: do NOT bump jit_flush_gen here. SFENCE.VMA is
                    // issued on every page-table change — including the
                    // frequent data mmaps of a malloc-heavy process (a
                    // compiler!) — which would flush the whole JIT block
                    // cache and keep coverage at ~0% on realistic workloads.
                    // Stale *code* mappings are instead caught cheaply by
                    // the dispatcher's per-block pa re-verification.
                }
                // Zicsr
                (_, f3 @ 1..=3) | (_, f3 @ 5..=7) => {
                    let csr = insn >> 20;
                    if csr <= 3 {
                        // fflags/frm/fcsr are FP state
                        self.fp_check(insn)?;
                        self.fp_dirty();
                    }
                    let src = if f3 >= 5 {
                        rs1(insn) as u64 // immediate form: uimm5
                    } else {
                        self.x[rs1(insn)]
                    };
                    let old = self
                        .csr_read(csr)
                        .ok_or(Exception::IllegalInstruction { insn })?;
                    // CSRRS/CSRRC with rs1=x0 (or uimm=0) must not write.
                    let src_is_zero = if f3 >= 5 { src == 0 } else { rs1(insn) == 0 };
                    let new = match f3 & 3 {
                        1 => Some(src),                        // CSRRW[I]
                        2 if !src_is_zero => Some(old | src),  // CSRRS[I]
                        3 if !src_is_zero => Some(old & !src), // CSRRC[I]
                        _ => None,
                    };
                    if let Some(v) = new {
                        if !self.csr_write(csr, v) {
                            return Err(Exception::IllegalInstruction { insn });
                        }
                    }
                    self.wr(rd(insn), old);
                }
                _ => return Err(Exception::IllegalInstruction { insn }),
            },
            _ => return Err(Exception::IllegalInstruction { insn }),
        }

        self.pc = next_pc;
        self.insn_count += 1;
        Ok(stop)
    }

    /// Resolve a rounding mode field (0b111 = dynamic via frm).
    /// None = reserved encoding -> illegal instruction.
    fn get_rm(&self, rm_field: u32) -> Option<u32> {
        let rm = if rm_field == 7 {
            (self.fcsr >> 5) & 7
        } else {
            rm_field
        };
        (rm <= 4).then_some(rm)
    }

    /// Native-FP fast path for FADD/FSUB/FMUL/FDIV, valid only when no new
    /// fflags information is possible. Preconditions checked by the caller:
    /// rm == RNE and NX already set (flags are sticky, so once NX is set,
    /// an op whose only possible flag is NX changes nothing architectural).
    /// This function then excludes every operand/result shape that could
    /// raise NV/DZ/OF/UF:
    ///
    ///   - operands must be finite (no NaN/inf -> no NV; nonzero divisor -> no DZ)
    ///   - result must not be inf (no OF)
    ///   - UF: add/sub of finite values never underflows inexactly (a
    ///     subnormal sum is exact — classic IEEE result), so any non-inf
    ///     result is fine; mul/div require a *normal* result, or an exactly
    ///     zero result forced by a zero operand.
    ///
    /// Under those conditions the host op (native FPU, or wasm f32/f64
    /// instructions in the wasm build) is bit-exact IEEE RNE, and no flag
    /// computation is needed at all. Everything else falls to softfp.
    #[inline]
    fn fp_fast64(op: u32, a: u64, b: u64) -> Option<u64> {
        let ea = (a >> 52) & 0x7ff;
        let eb = (b >> 52) & 0x7ff;
        if ea == 0x7ff || eb == 0x7ff {
            return None; // NaN/inf operands
        }
        if op == 3 && b << 1 == 0 {
            return None; // divide by zero
        }
        let (fa, fb) = (f64::from_bits(a), f64::from_bits(b));
        let r = match op {
            0 => fa + fb,
            1 => fa - fb,
            2 => fa * fb,
            _ => fa / fb,
        };
        let rb = r.to_bits();
        let er = (rb >> 52) & 0x7ff;
        let ok = match op {
            0 | 1 => er != 0x7ff,
            2 => (1..=0x7fe).contains(&er) || (rb << 1 == 0 && (a << 1 == 0 || b << 1 == 0)),
            _ => (1..=0x7fe).contains(&er) || (rb << 1 == 0 && a << 1 == 0),
        };
        ok.then_some(rb)
    }

    #[inline]
    fn fp_fast32(op: u32, a: u32, b: u32) -> Option<u32> {
        let ea = (a >> 23) & 0xff;
        let eb = (b >> 23) & 0xff;
        if ea == 0xff || eb == 0xff {
            return None;
        }
        if op == 3 && b << 1 == 0 {
            return None;
        }
        let (fa, fb) = (f32::from_bits(a), f32::from_bits(b));
        let r = match op {
            0 => fa + fb,
            1 => fa - fb,
            2 => fa * fb,
            _ => fa / fb,
        };
        let rb = r.to_bits();
        let er = (rb >> 23) & 0xff;
        let ok = match op {
            0 | 1 => er != 0xff,
            2 => (1..=0xfe).contains(&er) || (rb << 1 == 0 && (a << 1 == 0 || b << 1 == 0)),
            _ => (1..=0xfe).contains(&er) || (rb << 1 == 0 && a << 1 == 0),
        };
        ok.then_some(rb)
    }

    /// OP-FP (opcode 0x53), softfloat implementation (exact fflags).
    /// Ported from TinyEMU's softfp (see softfp.rs).
    fn op_fp(&mut self, insn: u32) -> Result<(), Exception> {
        use crate::softfp::{self as sfp, sf32, sf64};

        /// f32 operand: NaN-boxed reads; improper boxes read as qNaN.
        #[inline]
        fn ub32(r: u64) -> u32 {
            if r >> 32 == 0xffff_ffff {
                r as u32
            } else {
                0x7fc0_0000
            }
        }
        #[inline]
        fn bx32(v: u32) -> u64 {
            0xffff_ffff_0000_0000 | v as u64
        }

        let f7 = funct7(insn);
        let fmt = f7 & 3;
        let op = f7 >> 2;
        let f3 = funct3(insn);
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
        let ill = Exception::IllegalInstruction { insn };
        let mut fl: u32 = 0;

        match (op, fmt) {
            // ---- arithmetic ----
            (0x00..=0x03, 0) => {
                let rm = self.get_rm(f3).ok_or(ill)?;
                let (a, b) = (ub32(self.f[s1]), ub32(self.f[s2]));
                // Fast path: see fp_fast32 — exact, no flag math needed.
                if rm == sfp::RM_RNE && self.fcsr & sfp::FFLAG_INEXACT != 0 {
                    if let Some(r) = Self::fp_fast32(op, a, b) {
                        self.f[d] = bx32(r);
                        return Ok(());
                    }
                }
                let r = match op {
                    0 => sf32::add(a, b, rm, &mut fl),
                    1 => sf32::sub(a, b, rm, &mut fl),
                    2 => sf32::mul(a, b, rm, &mut fl),
                    _ => sf32::div(a, b, rm, &mut fl),
                };
                self.f[d] = bx32(r);
            }
            (0x00..=0x03, 1) => {
                let rm = self.get_rm(f3).ok_or(ill)?;
                let (a, b) = (self.f[s1], self.f[s2]);
                if rm == sfp::RM_RNE && self.fcsr & sfp::FFLAG_INEXACT != 0 {
                    if let Some(r) = Self::fp_fast64(op, a, b) {
                        self.f[d] = r;
                        return Ok(());
                    }
                }
                self.f[d] = match op {
                    0 => sf64::add(a, b, rm, &mut fl),
                    1 => sf64::sub(a, b, rm, &mut fl),
                    2 => sf64::mul(a, b, rm, &mut fl),
                    _ => sf64::div(a, b, rm, &mut fl),
                };
            }
            (0x0b, 0) if s2 == 0 => {
                let rm = self.get_rm(f3).ok_or(ill)?;
                self.f[d] = bx32(sf32::sqrt(ub32(self.f[s1]), rm, &mut fl));
            }
            (0x0b, 1) if s2 == 0 => {
                let rm = self.get_rm(f3).ok_or(ill)?;
                self.f[d] = sf64::sqrt(self.f[s1], rm, &mut fl);
            }

            // ---- sign injection (no flags) ----
            (0x04, 0) => {
                let (a, b) = (ub32(self.f[s1]), ub32(self.f[s2]));
                let r = match f3 {
                    0 => (a & 0x7fff_ffff) | (b & 0x8000_0000),
                    1 => (a & 0x7fff_ffff) | (!b & 0x8000_0000),
                    2 => a ^ (b & 0x8000_0000),
                    _ => return Err(ill),
                };
                self.f[d] = bx32(r);
            }
            (0x04, 1) => {
                let (a, b) = (self.f[s1], self.f[s2]);
                const S: u64 = 1 << 63;
                self.f[d] = match f3 {
                    0 => (a & !S) | (b & S),
                    1 => (a & !S) | (!b & S),
                    2 => a ^ (b & S),
                    _ => return Err(ill),
                };
            }

            // ---- min / max ----
            (0x05, 0) => {
                let (a, b) = (ub32(self.f[s1]), ub32(self.f[s2]));
                let r = match f3 {
                    0 => sf32::min(a, b, &mut fl),
                    1 => sf32::max(a, b, &mut fl),
                    _ => return Err(ill),
                };
                self.f[d] = bx32(r);
            }
            (0x05, 1) => {
                let (a, b) = (self.f[s1], self.f[s2]);
                self.f[d] = match f3 {
                    0 => sf64::min(a, b, &mut fl),
                    1 => sf64::max(a, b, &mut fl),
                    _ => return Err(ill),
                };
            }

            // ---- float <-> float ----
            (0x08, 0) if s2 == 1 => {
                let rm = self.get_rm(f3).ok_or(ill)?;
                self.f[d] = bx32(sfp::cvt_sf64_sf32(self.f[s1], rm, &mut fl)); // FCVT.S.D
            }
            (0x08, 1) if s2 == 0 => {
                self.f[d] = sfp::cvt_sf32_sf64(ub32(self.f[s1]), &mut fl); // FCVT.D.S
            }

            // ---- comparisons ----
            (0x14, 0) => {
                let (a, b) = (ub32(self.f[s1]), ub32(self.f[s2]));
                let r = match f3 {
                    2 => sf32::eq_quiet(a, b, &mut fl),
                    1 => sf32::lt(a, b, &mut fl),
                    0 => sf32::le(a, b, &mut fl),
                    _ => return Err(ill),
                };
                self.wr(d, r as u64);
            }
            (0x14, 1) => {
                let (a, b) = (self.f[s1], self.f[s2]);
                let r = match f3 {
                    2 => sf64::eq_quiet(a, b, &mut fl),
                    1 => sf64::lt(a, b, &mut fl),
                    0 => sf64::le(a, b, &mut fl),
                    _ => return Err(ill),
                };
                self.wr(d, r as u64);
            }

            // ---- float -> int ----
            (0x18, 0) => {
                let rm = self.get_rm(f3).ok_or(ill)?;
                let a = ub32(self.f[s1]);
                let r = match s2 {
                    0 => sf32::cvt_to_i32(a, rm, &mut fl, false) as i32 as i64 as u64,
                    1 => sf32::cvt_to_i32(a, rm, &mut fl, true) as i32 as i64 as u64,
                    2 => sf32::cvt_to_i64(a, rm, &mut fl, false),
                    3 => sf32::cvt_to_i64(a, rm, &mut fl, true),
                    _ => return Err(ill),
                };
                self.wr(d, r);
            }
            (0x18, 1) => {
                let rm = self.get_rm(f3).ok_or(ill)?;
                let a = self.f[s1];
                let r = match s2 {
                    0 => sf64::cvt_to_i32(a, rm, &mut fl, false) as i32 as i64 as u64,
                    1 => sf64::cvt_to_i32(a, rm, &mut fl, true) as i32 as i64 as u64,
                    2 => sf64::cvt_to_i64(a, rm, &mut fl, false),
                    3 => sf64::cvt_to_i64(a, rm, &mut fl, true),
                    _ => return Err(ill),
                };
                self.wr(d, r);
            }

            // ---- int -> float ----
            (0x1a, 0) => {
                let rm = self.get_rm(f3).ok_or(ill)?;
                let x = self.x[s1];
                let r = match s2 {
                    0 => sf32::cvt_from_i32(x as u32, rm, &mut fl, false),
                    1 => sf32::cvt_from_i32(x as u32, rm, &mut fl, true),
                    2 => sf32::cvt_from_i64(x, rm, &mut fl, false),
                    3 => sf32::cvt_from_i64(x, rm, &mut fl, true),
                    _ => return Err(ill),
                };
                self.f[d] = bx32(r);
            }
            (0x1a, 1) => {
                let rm = self.get_rm(f3).ok_or(ill)?;
                let x = self.x[s1];
                self.f[d] = match s2 {
                    0 => sf64::cvt_from_i32(x as u32, rm, &mut fl, false),
                    1 => sf64::cvt_from_i32(x as u32, rm, &mut fl, true),
                    2 => sf64::cvt_from_i64(x, rm, &mut fl, false),
                    3 => sf64::cvt_from_i64(x, rm, &mut fl, true),
                    _ => return Err(ill),
                };
            }

            // ---- moves / classify (no flags) ----
            (0x1c, 0) if f3 == 0 => self.wr(d, self.f[s1] as u32 as i32 as i64 as u64), // FMV.X.W
            (0x1c, 0) if f3 == 1 => self.wr(d, sf32::fclass(ub32(self.f[s1])) as u64),
            (0x1c, 1) if f3 == 0 => self.wr(d, self.f[s1]), // FMV.X.D
            (0x1c, 1) if f3 == 1 => self.wr(d, sf64::fclass(self.f[s1]) as u64),
            (0x1e, 0) if f3 == 0 => self.f[d] = bx32(self.x[s1] as u32), // FMV.W.X
            (0x1e, 1) if f3 == 0 => self.f[d] = self.x[s1],              // FMV.D.X

            _ => return Err(ill),
        }
        self.fcsr |= fl & 0x1f;
        Ok(())
    }

    /// Read a CSR; None = unimplemented (traps as illegal instruction).
    fn csr_read(&self, csr: u32) -> Option<u64> {
        // Privilege check: bits [9:8] of the address encode the minimum mode.
        if let Some(sys) = self.sys.as_ref() {
            if ((csr >> 8) & 3) as u64 > sys.mode as u64 {
                return None;
            }
        }
        match csr {
            FFLAGS => Some((self.fcsr & 0x1f) as u64),
            FRM => Some(((self.fcsr >> 5) & 7) as u64),
            FCSR => Some(self.fcsr as u64),
            VSTART if self.vector_enabled() => Some(self.vector.vstart),
            VXSAT if self.vector_enabled() => Some(self.vector.vxsat as u64),
            VXRM if self.vector_enabled() => Some(self.vector.vxrm as u64),
            VCSR if self.vector_enabled() => {
                Some(((self.vector.vxrm as u64) << 1) | self.vector.vxsat as u64)
            }
            VL if self.vector_enabled() => Some(self.vector.vl),
            VTYPE if self.vector_enabled() => Some(self.vector.vtype),
            VLENB if self.vector_enabled() => Some(vector::VLEN_BYTES as u64),
            CYCLE | INSTRET | MCYCLE | MINSTRET => Some(
                self.insn_count
                    .wrapping_add(self.sys.as_ref().map_or(0, |s| s.minstret_off)),
            ),
            TIME => Some(self.sys.as_ref().map_or(self.insn_count, |s| {
                self.insn_count
                    .checked_div(s.time_scale)
                    .map(|time| time.wrapping_add(s.time_offset))
                    .unwrap_or(s.mtime)
            })),
            // PMP: storage only, no enforcement (single-guest machine).
            0x3a0..=0x3af if csr & 1 == 0 => self
                .sys
                .as_ref()
                .map(|s| s.pmpcfg[((csr - 0x3a0) / 2) as usize]),
            0x3b0..=0x3ef => self.sys.as_ref().map(|s| s.pmpaddr[(csr - 0x3b0) as usize]),
            _ => {
                let sys = self.sys.as_ref()?;
                // SD summarizes dirty extension state (FS or VS).
                let mstatus = if sys.mstatus & MSTATUS_FS == MSTATUS_FS
                    || sys.mstatus & MSTATUS_VS == MSTATUS_VS
                {
                    sys.mstatus | MSTATUS_SD
                } else {
                    sys.mstatus
                };
                Some(match csr {
                    SSTATUS => mstatus & SSTATUS_MASK,
                    SIE => sys.mie & sys.mideleg,
                    STVEC => sys.stvec,
                    SCOUNTEREN => sys.scounteren,
                    SSCRATCH => sys.sscratch,
                    SEPC => sys.sepc,
                    SCAUSE => sys.scause,
                    STVAL => sys.stval,
                    SIP => sys.mip & sys.mideleg,
                    SATP => {
                        // S-mode satp access traps when mstatus.TVM = 1.
                        if sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TVM != 0 {
                            return None;
                        }
                        sys.satp
                    }
                    // Debug triggers: none implemented. tselect reads back
                    // nonzero after writing 0 — the architected "hardwired"
                    // signal riscv-tests uses to skip trigger tests.
                    0x7a0 => 1,
                    0x7a1..=0x7a3 | 0x7a5 => 0,
                    MSTATUS => mstatus,
                    MISA => MISA_VALUE,
                    MEDELEG => sys.medeleg,
                    MIDELEG => sys.mideleg,
                    MIE => sys.mie,
                    MTVEC => sys.mtvec,
                    MCOUNTEREN => sys.mcounteren,
                    MSCRATCH => sys.mscratch,
                    MEPC => sys.mepc,
                    MCAUSE => sys.mcause,
                    MTVAL => sys.mtval,
                    MIP => sys.mip,
                    MVENDORID | MARCHID | MIMPID => 0,
                    MHARTID => sys.mhartid,
                    _ => return None,
                })
            }
        }
    }

    /// Write a CSR; false = unimplemented/read-only.
    fn csr_write(&mut self, csr: u32, v: u64) -> bool {
        // Enabling interrupts (mstatus/sstatus.xIE, mie/sie) or clearing a
        // pending bit must be visible to the very next instruction, so drop the
        // interpreter's interrupt-poll countdown (see irq_poll_cd).
        self.irq_poll_cd = 0;
        if csr >> 10 == 3 {
            return false; // read-only region
        }
        if let Some(sys) = self.sys.as_ref() {
            if ((csr >> 8) & 3) as u64 > sys.mode as u64 {
                return false;
            }
        }
        match csr {
            FFLAGS => self.fcsr = (self.fcsr & !0x1f) | (v as u32 & 0x1f),
            FRM => self.fcsr = (self.fcsr & !0xe0) | ((v as u32 & 7) << 5),
            FCSR => self.fcsr = v as u32 & 0xff,
            VSTART if self.vector_enabled() => {
                self.vector.vstart = v & (vector::VLEN_BITS as u64 - 1);
                self.vector_dirty();
            }
            VXSAT if self.vector_enabled() => {
                self.vector.vxsat = v & 1 != 0;
                self.vector_dirty();
            }
            VXRM if self.vector_enabled() => {
                self.vector.vxrm = (v & 3) as u8;
                self.vector_dirty();
            }
            VCSR if self.vector_enabled() => {
                self.vector.vxsat = v & 1 != 0;
                self.vector.vxrm = ((v >> 1) & 3) as u8;
                self.vector_dirty();
            }
            MCYCLE | MINSTRET => {
                // Writable counters. The writing csrw itself retires after
                // the write takes effect, so bias by insn_count+1: a
                // csrw 0 / csrr pair reads back exactly 0.
                let ic = self.insn_count.wrapping_add(1);
                if let Some(sys) = self.sys.as_mut() {
                    sys.minstret_off = v.wrapping_sub(ic);
                }
            }
            0x3a0..=0x3af if csr & 1 == 0 => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.pmpcfg[((csr - 0x3a0) / 2) as usize] = v;
            }
            0x3b0..=0x3ef => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                // WARL: address bits [53:0]
                sys.pmpaddr[(csr - 0x3b0) as usize] = v & 0x003f_ffff_ffff_ffff;
            }
            SSTATUS => {
                const W: u64 = MSTATUS_SIE
                    | MSTATUS_SPIE
                    | MSTATUS_SPP
                    | MSTATUS_VS
                    | MSTATUS_FS
                    | MSTATUS_SUM
                    | MSTATUS_MXR;
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mstatus = (sys.mstatus & !W) | (v & W);
                // SUM/MXR are already part of every translation tag. Keep
                // proofs from the old context; they cannot hit under the new
                // one, and remain useful if Linux switches the bits back.
                if !self.retain_tlb_on_privilege_change {
                    self.flush_tlb();
                }
                self.sync_jit_tlb_context();
            }
            SIE => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                let mask = sys.mideleg;
                sys.mie = (sys.mie & !mask) | (v & mask);
            }
            STVEC => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.stvec = v & !2;
            }
            SCOUNTEREN => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.scounteren = v & 7;
            }
            SSCRATCH => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.sscratch = v;
            }
            SEPC => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.sepc = v & !1;
            }
            SCAUSE => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.scause = v;
            }
            STVAL => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.stval = v;
            }
            SIP => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                // Only SSIP is directly writable by S-mode.
                let mask = IRQ_SSIP & sys.mideleg;
                sys.mip = (sys.mip & !mask) | (v & mask);
            }
            SATP => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                if sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TVM != 0 {
                    return false; // traps as illegal under TVM
                }
                // Accept bare/sv39/sv48; ignore others (WARL).
                let mode = v >> 60;
                if mode == 0 || mode == 8 || mode == 9 {
                    let changed = sys.satp != v;
                    sys.satp = v;
                    self.flush_tlb();
                    if changed {
                        self.jit_flush_gen += 1; // address space switched
                        self.map_gen += 1;
                    }
                }
            }
            // Debug trigger CSRs: writes ignored (no triggers implemented).
            0x7a0..=0x7a3 | 0x7a5 => {}
            MSTATUS => {
                const W: u64 = MSTATUS_SIE
                    | MSTATUS_MIE
                    | MSTATUS_SPIE
                    | MSTATUS_MPIE
                    | MSTATUS_SPP
                    | MSTATUS_MPP
                    | MSTATUS_VS
                    | MSTATUS_FS
                    | MSTATUS_MPRV
                    | MSTATUS_SUM
                    | MSTATUS_MXR
                    | MSTATUS_TVM
                    | MSTATUS_TW
                    | MSTATUS_TSR;
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mstatus = (sys.mstatus & !W) | (v & W);
                // MPRV/MPP/SUM/MXR all fold into the effective-mode/context
                // tag. Interrupt and FP-state changes do not affect address
                // translation at all, so neither class needs a production
                // flush here.
                if !self.retain_tlb_on_privilege_change {
                    self.flush_tlb();
                }
                self.sync_jit_tlb_context();
            }
            MISA => {}
            MEDELEG => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.medeleg = v;
            }
            MIDELEG => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mideleg = v & (IRQ_SSIP | IRQ_STIP | IRQ_SEIP);
            }
            MIE => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mie = v;
            }
            MTVEC => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mtvec = v & !2;
            }
            MCOUNTEREN => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mcounteren = v & 7;
            }
            MSCRATCH => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mscratch = v;
            }
            MEPC => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mepc = v & !1;
            }
            MCAUSE => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mcause = v;
            }
            MTVAL => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mtval = v;
            }
            MIP => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                // MSIP/MTIP are set by the CLINT; software may write others.
                const W: u64 = IRQ_SSIP | IRQ_STIP | IRQ_SEIP;
                sys.mip = (sys.mip & !W) | (v & W);
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatMemory;

    const BASE: u64 = 0x1000;

    /// Flat physical memory that also admits every translated page to the
    /// fused JIT row. This keeps the context-tag proof independent of the
    /// full virt machine and its device layout.
    struct DirectFlatMemory<'a>(FlatMemory<'a>);

    macro_rules! direct_flat_rw {
        ($read:ident, $write:ident, $ty:ty) => {
            fn $read(&mut self, addr: u64) -> Result<$ty, Exception> {
                self.0.$read(addr)
            }
            fn $write(&mut self, addr: u64, value: $ty) -> Result<(), Exception> {
                self.0.$write(addr, value)
            }
        };
    }

    impl Bus for DirectFlatMemory<'_> {
        direct_flat_rw!(read8, write8, u8);
        direct_flat_rw!(read16, write16, u16);
        direct_flat_rw!(read32, write32, u32);
        direct_flat_rw!(read64, write64, u64);

        fn jit_fast_off(&self, va: u64, pa: u64, _store: bool) -> Option<i64> {
            let offset = pa.checked_sub(self.0.base)? as usize;
            (offset < self.0.mem.len()).then(|| {
                (self.0.mem.as_ptr() as i64)
                    .wrapping_add(offset as i64)
                    .wrapping_sub(va as i64)
            })
        }
    }

    fn run_program(words: &[u32]) -> (Cpu, Vec<u8>) {
        let mut mem = vec![0u8; 0x10000];
        for (i, w) in words.iter().enumerate() {
            mem[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let mut cpu = Cpu::new();
        cpu.pc = BASE;
        let mut bus = FlatMemory::new(BASE, &mut mem);
        let stop = cpu.run(&mut bus, 10_000);
        assert_eq!(stop, StopReason::Ecall, "program should end in ecall");
        (cpu, mem)
    }

    #[test]
    fn folded_jit_tlb_index_separates_direct_map_aliases() {
        const FIRST: u64 = 0x1000;
        const DIRECT_ALIAS: u64 = FIRST + (1u64 << (12 + TLB_BITS));
        let direct = |va: u64| ((va >> 12) as usize) & (TLB_SIZE - 1);

        assert_eq!(direct(FIRST), direct(DIRECT_ALIAS));
        assert_ne!(
            Cpu::jit_tlb_hash_index(FIRST),
            Cpu::jit_tlb_hash_index(DIRECT_ALIAS)
        );

        let mut cpu = Cpu::new();
        cpu.jtlb_tag[0][direct(FIRST)] = 0;
        cpu.set_jit_tlb_hash(true);
        assert!(cpu.jtlb_tag[0].iter().all(|&tag| tag == TLB_INVALID));
        assert_eq!(cpu.jit_tlb_index(FIRST), Cpu::jit_tlb_hash_index(FIRST));
    }

    #[test]
    fn jit_tlb_miss_classification_distinguishes_empty_context_and_collision() {
        const VA: u64 = 0x1234_5000;
        let mut cpu = Cpu::new();
        let index = cpu.jit_tlb_index(VA);
        assert_eq!(cpu.jit_tlb_miss_kind(VA, false), 1);

        cpu.jtlb_tag[0][index] = VA & TLB_PAGE_MASK;
        assert_eq!(cpu.jit_tlb_miss_kind(VA, false), 0);
        cpu.jtlb_tag[0][index] = (VA & TLB_PAGE_MASK) | TLB_CTX_SUM;
        assert_eq!(cpu.jit_tlb_miss_kind(VA, false), 2);
        cpu.jtlb_tag[0][index] = (VA & TLB_PAGE_MASK).wrapping_add(1 << (12 + TLB_BITS));
        assert_eq!(cpu.jit_tlb_miss_kind(VA, false), 3);
    }

    #[test]
    fn sfence_vma_invalidates_only_its_virtual_page_and_current_asid() {
        const VA: u64 = 0x1234_5000;
        const OTHER: u64 = 0x1234_7000;
        const ASID: u16 = 7;

        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        {
            let sys = cpu.sys.as_mut().unwrap();
            sys.mode = Mode::Supervisor;
            sys.satp = (8 << 60) | ((ASID as u64) << 44) | 0x123;
        }
        let context = cpu.data_tlb_context();
        let target_tag = (VA & TLB_PAGE_MASK) | context;
        let other_tag = (OTHER & TLB_PAGE_MASK) | context;
        let target_index = ((VA >> 12) as usize) & (TLB_SIZE - 1);
        let other_index = ((OTHER >> 12) as usize) & (TLB_SIZE - 1);
        let target_jit_index = cpu.jit_tlb_index(VA);
        let other_jit_index = cpu.jit_tlb_index(OTHER);
        assert_ne!(target_index, other_index);
        assert_ne!(target_jit_index, other_jit_index);

        for tags in &mut cpu.tlb_tag {
            tags[target_index] = target_tag;
            tags[other_index] = other_tag;
        }
        for tags in &mut cpu.jtlb_tag {
            tags[target_jit_index] = target_tag;
            tags[other_jit_index] = other_tag;
        }
        cpu.sfence_vma(Some(VA), None);
        assert_eq!(cpu.map_gen, 1);
        assert_eq!(cpu.tlb_flushes, 0, "a page fence must not clear every row");
        assert_eq!(cpu.sfence_page, 1);
        for tags in &cpu.tlb_tag {
            assert_eq!(tags[target_index], TLB_INVALID);
            assert_eq!(tags[other_index], other_tag);
        }
        for tags in &cpu.jtlb_tag {
            assert_eq!(tags[target_jit_index], TLB_INVALID);
            assert_eq!(tags[other_jit_index], other_tag);
        }
        // This hart cannot hold a translation for another ASID: SATP changes
        // perform a full invalidation. A targeted foreign-ASID fence is a no-op.
        cpu.jtlb_tag[0][other_jit_index] = other_tag;
        cpu.sfence_vma(Some(OTHER), Some(ASID + 1));
        assert_eq!(cpu.map_gen, 1);
        assert_eq!(cpu.sfence_foreign_asid, 1);
        assert_eq!(cpu.jtlb_tag[0][other_jit_index], other_tag);
        cpu.sfence_vma(None, Some(ASID));
        assert_eq!(cpu.map_gen, 2);
        assert_eq!(cpu.sfence_all, 1);
        assert_eq!(cpu.tlb_flushes, 1);
        assert!(cpu.jtlb_tag.iter().flatten().all(|&tag| tag == TLB_INVALID));
    }

    #[test]
    fn sfence_vma_instruction_uses_rs1_address_and_rs2_asid() {
        const VA: u64 = 0x1234_5000;
        const ASID: u16 = 7;
        // sfence.vma x5, x6
        const SFENCE: u32 = (0x09 << 25) | (6 << 20) | (5 << 15) | 0x73;

        let mut memory = vec![0u8; 0x1000];
        memory[..4].copy_from_slice(&SFENCE.to_le_bytes());
        let mut bus = FlatMemory::new(BASE, &mut memory);
        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        {
            let sys = cpu.sys.as_mut().unwrap();
            sys.mode = Mode::Supervisor;
            // Bare translation keeps the instruction fetch in FlatMemory;
            // ASID bits still exercise the fence operand comparison.
            sys.satp = (ASID as u64) << 44;
        }
        cpu.pc = BASE;
        cpu.x[5] = VA;
        cpu.x[6] = ASID as u64;
        let index = cpu.jit_tlb_index(VA);
        cpu.jtlb_tag[0][index] = (VA & TLB_PAGE_MASK) | cpu.data_tlb_context();

        assert_eq!(cpu.run(&mut bus, 1), StopReason::Budget);
        assert_eq!(cpu.pc, BASE + 4);
        assert_eq!(cpu.sfence_page, 1);
        assert_eq!(cpu.sfence_all, 0);
        assert_eq!(cpu.jtlb_tag[0][index], TLB_INVALID);
    }

    #[test]
    fn host_sbi_returns_supervisor_ecall_without_machine_trap() {
        let mut mem = vec![0u8; 0x1000];
        mem[..4].copy_from_slice(&0x0000_0073u32.to_le_bytes());
        let mut bus = FlatMemory::new(BASE, &mut mem);
        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        cpu.enable_host_sbi();
        cpu.sys.as_mut().unwrap().mode = Mode::Supervisor;
        cpu.pc = BASE;
        assert_eq!(cpu.run(&mut bus, 1), StopReason::Ecall);
        assert_eq!(cpu.pc, BASE + 4);
        assert_eq!(cpu.sys.as_ref().unwrap().mode, Mode::Supervisor);
        assert_eq!(cpu.insn_count, 1);
    }

    #[test]
    fn traced_run_matches_plain_run_and_reports_retired_instructions() {
        let words = [
            0x00300093u32, // addi x1, x0, 3
            0xfff08093,    // addi x1, x1, -1
            0xfe009ee3,    // bne x1, x0, -4
            0x00000073,    // ecall
        ];
        let mut plain_mem = vec![0u8; 0x1000];
        for (i, word) in words.iter().enumerate() {
            plain_mem[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let mut traced_mem = plain_mem.clone();
        let mut plain = Cpu::new();
        plain.pc = BASE;
        let plain_stop = plain.run(&mut FlatMemory::new(BASE, &mut plain_mem), 100);

        let mut traced = Cpu::new();
        traced.pc = BASE;
        let mut events = Vec::new();
        let traced_stop = traced.run_traced(
            &mut FlatMemory::new(BASE, &mut traced_mem),
            100,
            &mut |event| events.push(event),
        );

        assert_eq!(traced_stop, plain_stop);
        assert_eq!(traced.pc, plain.pc);
        assert_eq!(traced.x, plain.x);
        assert_eq!(traced.insn_count, plain.insn_count);
        assert_eq!(events.len() as u64, traced.insn_count);
        assert_eq!(events.first().unwrap().va, BASE);
        assert_eq!(events.last().unwrap().next_va, traced.pc);
        assert!(events.iter().all(|event| event.ilen == 4));
        for (i, event) in events.iter().enumerate() {
            assert_eq!(event.icount, i as u64 + 1);
            assert_eq!(event.pa, event.va);
        }
    }

    #[test]
    fn run_until_stops_exactly_at_requested_next_pc() {
        let words = [
            0x00100093u32, // addi x1, x0, 1
            0x00108093,    // addi x1, x1, 1
            0x00108093,    // addi x1, x1, 1
            0x00000073,    // ecall
        ];
        let mut mem = vec![0u8; 0x1000];
        for (i, word) in words.iter().enumerate() {
            mem[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let mut cpu = Cpu::new();
        cpu.pc = BASE;
        let mut bus = FlatMemory::new(BASE, &mut mem);
        let mut probes = 0;
        let stop = cpu.run_until(&mut bus, 100, &mut |pc| {
            probes += 1;
            pc == BASE + 8
        });
        assert_eq!(stop, StopReason::Budget);
        assert_eq!(cpu.pc, BASE + 8);
        assert_eq!(cpu.insn_count, 2);
        assert_eq!(cpu.x[1], 2);
        assert_eq!(probes, 2);
        assert_eq!(cpu.run(&mut bus, 100), StopReason::Ecall);
        assert_eq!(cpu.x[1], 3);
    }

    #[test]
    fn run_until_observed_reports_a_mapped_nonsequential_target() {
        let words = [
            0x00100093u32, // addi x1,x0,1
            0x0080006f,    // jal x0,+8
            0x00108093,    // skipped
            0x00000073,    // ecall
        ];
        let mut mem = vec![0u8; 0x1000];
        for (i, word) in words.iter().enumerate() {
            mem[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let mut cpu = Cpu::new();
        cpu.pc = BASE;
        let mut bus = FlatMemory::new(BASE, &mut mem);
        let mut entries = Vec::new();
        let stop =
            cpu.run_until_observed(&mut bus, 2, &mut |_| false, &mut |pc, pa, satp, mode| {
                entries.push((pc, pa, satp, mode))
            });
        assert_eq!(stop, StopReason::Budget);
        assert_eq!(cpu.pc, BASE + 12);
        assert_eq!(cpu.insn_count, 2);
        assert_eq!(entries, vec![(BASE + 12, BASE + 12, 0, u8::MAX)]);
    }

    #[test]
    fn addi_add_sub() {
        // addi x1, x0, 5 ; addi x2, x0, 7 ; add x3, x1, x2 ; sub x4, x2, x1 ; ecall
        let (cpu, _) = run_program(&[0x00500093, 0x00700113, 0x002081b3, 0x40110233, 0x00000073]);
        assert_eq!(cpu.x[3], 12);
        assert_eq!(cpu.x[4], 2);
    }

    #[test]
    fn x0_is_hardwired_zero() {
        // addi x0, x0, 42 ; ecall
        let (cpu, _) = run_program(&[0x02a00013, 0x00000073]);
        assert_eq!(cpu.x[0], 0);
    }

    #[test]
    fn lui_auipc() {
        // lui x1, 0x12345 ; auipc x2, 0 ; ecall
        let (cpu, _) = run_program(&[0x1234_50b7, 0x0000_0117, 0x0000_0073]);
        assert_eq!(cpu.x[1], 0x12345000);
        assert_eq!(cpu.x[2], BASE + 4);
    }

    #[test]
    fn negative_immediate_sign_extends() {
        // addi x1, x0, -1 ; ecall
        let (cpu, _) = run_program(&[0xfff00093, 0x00000073]);
        assert_eq!(cpu.x[1], u64::MAX);
    }

    #[test]
    fn loads_stores_roundtrip() {
        let (cpu, _) = run_program(&[
            0xffe00093, // addi x1, x0, -2
            0x000012b7, // lui x5, 0x1  (x5 = 0x1000 = BASE)
            0x10129023, // sh x1, 0x100(x5)
            0x1012b423, // sd x1, 0x108(x5)
            0x1082b103, // ld x2, 0x108(x5)
            0x1082a183, // lw x3, 0x108(x5)
            0x1082c203, // lbu x4, 0x108(x5)
            0x00000073, // ecall
        ]);
        assert_eq!(cpu.x[2], (-2i64) as u64);
        assert_eq!(cpu.x[3], (-2i64) as u64); // lw sign-extends
        assert_eq!(cpu.x[4], 0xfe); // lbu zero-extends
    }

    #[test]
    fn branch_loop_sums_1_to_10() {
        // x1 = 0 (sum), x2 = 1 (i), x3 = 11 (limit)
        // loop: add x1, x1, x2 ; addi x2, x2, 1 ; bne x2, x3, loop ; ecall
        let (cpu, _) = run_program(&[
            0x00000093, // addi x1, x0, 0
            0x00100113, // addi x2, x0, 1
            0x00b00193, // addi x3, x0, 11
            0x002080b3, // add x1, x1, x2
            0x00110113, // addi x2, x2, 1
            0xfe311ce3, // bne x2, x3, -8
            0x00000073, // ecall
        ]);
        assert_eq!(cpu.x[1], 55);
    }

    #[test]
    fn jal_jalr_link() {
        // jal x1, +8 ; ecall(skipped) ; jalr x0, 0(x1) -> lands on ecall
        let (cpu, _) = run_program(&[
            0x008000ef, // jal x1, +8
            0x00000073, // ecall (return target)
            0x00008067, // jalr x0, 0(x1)
        ]);
        assert_eq!(cpu.x[1], BASE + 4);
        assert_eq!(cpu.pc, BASE + 8); // pc after the ecall at BASE+4
    }

    #[test]
    fn m_extension() {
        // x1 = 7, x2 = -3; mul, mulh, div, rem, divw by zero
        let (cpu, _) = run_program(&[
            0x00700093, // addi x1, x0, 7
            0xffd00113, // addi x2, x0, -3
            0x022081b3, // mul  x3, x1, x2
            0x02209233, // mulh x4, x1, x2
            0x0220c2b3, // div  x5, x1, x2
            0x0220e333, // rem  x6, x1, x2
            0x0200c3bb, // divw x7, x1, x0  (div by zero -> -1)
            0x00000073, // ecall
        ]);
        assert_eq!(cpu.x[3] as i64, -21);
        assert_eq!(cpu.x[4] as i64, -1); // high bits of 7 * -3
        assert_eq!(cpu.x[5] as i64, -2); // 7 / -3 truncates toward zero
        assert_eq!(cpu.x[6] as i64, 1); // 7 rem -3
        assert_eq!(cpu.x[7] as i64, -1); // div by zero
    }

    #[test]
    fn a_extension_lr_sc_amo() {
        // x5 = BASE; store 100 at 0x100(x5); lr.d x1; sc.d x2 (succeeds -> 0);
        // amoadd.d x3 = old(100), mem += 5; ld x4 = 105... build:
        let (cpu, _) = run_program(&[
            0x000012b7, // lui x5, 0x1 (BASE)
            0x10028293, // addi x5, x5, 0x100
            0x06400313, // addi x6, x0, 100
            0x0062b023, // sd x6, 0(x5)
            0x1002b0af, // lr.d x1, (x5)
            0x1862b12f, // sc.d x2, x6, (x5)
            0x00500393, // addi x7, x0, 5
            0x0072b1af, // amoadd.d x3, x7, (x5)
            0x0002b203, // ld x4, 0(x5)
            0x00000073, // ecall
        ]);
        assert_eq!(cpu.x[1], 100); // lr loaded
        assert_eq!(cpu.x[2], 0); // sc succeeded
        assert_eq!(cpu.x[3], 100); // amoadd returned old
        assert_eq!(cpu.x[4], 105); // memory updated
    }

    #[test]
    fn trap_invalidates_lr_reservation() {
        // A trap taken between an LR and its SC must clear the reservation, so
        // the SC fails and the guest's LR/SC loop retries. Without this, an
        // interrupt handler updating the same word via LR/SC lets the
        // interrupted SC still succeed and silently lose the handler's update
        // (an intermittent lost-wakeup source under Linux). Deterministic guard
        // for a bug the full-system smoke test only trips probabilistically.
        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        cpu.reservation = Some(0x8000_0000);
        cpu.take_trap(7, 0, true); // e.g. a timer interrupt
        assert_eq!(cpu.reservation, None, "trap must invalidate LR reservation");

        // Any exception must too (page fault, ecall, ...).
        cpu.reservation = Some(0x8000_1000);
        cpu.take_trap(8, 0, false); // ecall from U-mode
        assert_eq!(
            cpu.reservation, None,
            "exception must invalidate LR reservation"
        );
    }

    #[test]
    fn privilege_tagged_tlbs_survive_traps_without_cross_context_hits() {
        const ROOT: u64 = 0x1000;
        const LEVEL1: u64 = 0x2000;
        const LEVEL0: u64 = 0x3000;
        const VA: u64 = 0x4000;
        const PA: u64 = 0x6000;
        const VALUE: u64 = 0x0123_4567_89ab_cdef;
        const PTE_V: u64 = 1;
        const PTE_R: u64 = 1 << 1;
        const PTE_W: u64 = 1 << 2;
        const PTE_U: u64 = 1 << 4;
        const PTE_A: u64 = 1 << 6;
        const PTE_D: u64 = 1 << 7;

        let mut memory = vec![0u8; 0x10_000];
        let put64 = |memory: &mut [u8], address: u64, value: u64| {
            let address = address as usize;
            memory[address..address + 8].copy_from_slice(&value.to_le_bytes());
        };
        put64(&mut memory, ROOT, ((LEVEL1 >> 12) << 10) | PTE_V);
        put64(&mut memory, LEVEL1, ((LEVEL0 >> 12) << 10) | PTE_V);
        put64(
            &mut memory,
            LEVEL0 + ((VA >> 12) & 0x1ff) * 8,
            ((PA >> 12) << 10) | PTE_V | PTE_R | PTE_W | PTE_U | PTE_A | PTE_D,
        );
        put64(&mut memory, PA, VALUE);
        let mut bus = DirectFlatMemory(FlatMemory::new(0, &mut memory));

        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        {
            let sys = cpu.sys.as_mut().unwrap();
            sys.mode = Mode::User;
            sys.satp = (8 << 60) | (ROOT >> 12);
            sys.medeleg = 1 << 8; // U-mode ECALL enters S mode.
            sys.stvec = 0x8000;
        }
        cpu.sync_jit_tlb_context();
        cpu.set_interpreter_fused_memory(true);

        assert_eq!(cpu.ld::<_, 8>(&mut bus, VA), Ok(VALUE));
        let index = ((VA >> 12) as usize) & (TLB_SIZE - 1);
        let user_tag = cpu.translation_tag(VA, Access::Load);
        assert_eq!(cpu.tlb_tag[Access::Load as usize][index], user_tag);
        assert_eq!(cpu.jtlb_tag[0][index], user_tag);
        assert_eq!(cpu.jit_tlb_context, Mode::User as u64);

        cpu.pc = 0x7000;
        cpu.take_trap(8, 0, false);
        assert_eq!(cpu.sys.as_ref().unwrap().mode, Mode::Supervisor);
        assert_eq!(
            cpu.tlb_tag[Access::Load as usize][index],
            user_tag,
            "a privilege transition must preserve the unrelated TLB working set"
        );
        assert_eq!(cpu.jtlb_tag[0][index], user_tag);
        assert_ne!(cpu.translation_tag(VA, Access::Load), user_tag);
        assert_ne!(cpu.jit_tlb_context, Mode::User as u64);

        // Supervisor mode cannot consume a U-page proof while SUM is clear.
        assert!(matches!(
            cpu.ld::<_, 8>(&mut bus, VA),
            Err(Exception::LoadPageFault { addr: VA })
        ));
        assert_eq!(cpu.jit_fill_tlb(&mut bus, VA, false), None);
        assert_eq!(cpu.jtlb_tag[0][index], user_tag);

        // Changing the permission context admits a fresh proof under a
        // distinct tag; it still cannot alias the retained U-mode proof.
        cpu.sys.as_mut().unwrap().mstatus |= MSTATUS_SUM;
        cpu.sync_jit_tlb_context();
        assert_eq!(cpu.ld::<_, 8>(&mut bus, VA), Ok(VALUE));
        let supervisor_sum_tag = cpu.translation_tag(VA, Access::Load);
        assert_ne!(supervisor_sum_tag, user_tag);
        assert_eq!(
            cpu.tlb_tag[Access::Load as usize][index],
            supervisor_sum_tag
        );
        assert_eq!(cpu.jtlb_tag[0][index], supervisor_sum_tag);
    }

    #[test]
    fn interpreter_fused_memory_bypasses_standard_tlb_after_exact_proof() {
        const ADDRESS: u64 = 0x4000;
        const FIRST: u64 = 0x0123_4567_89ab_cdef;
        const SECOND: u64 = 0xfedc_ba98_7654_3210;

        let mut memory = vec![0u8; 0x10_000];
        memory[ADDRESS as usize..ADDRESS as usize + 8].copy_from_slice(&FIRST.to_le_bytes());
        let mut bus = DirectFlatMemory(FlatMemory::new(0, &mut memory));
        let mut cpu = Cpu::new();
        cpu.set_interpreter_fused_memory(true);

        // The first access follows the ordinary path and publishes both rows.
        assert_eq!(cpu.ld::<_, 8>(&mut bus, ADDRESS), Ok(FIRST));
        let index = ((ADDRESS >> 12) as usize) & (TLB_SIZE - 1);
        assert_eq!(cpu.jtlb_tag[0][index], ADDRESS & TLB_PAGE_MASK);

        // Removing only the standard proof makes a subsequent fused hit
        // observable: the load succeeds without repopulating that row.
        cpu.tlb_tag[Access::Load as usize][index] = TLB_INVALID;
        assert_eq!(cpu.ld::<_, 8>(&mut bus, ADDRESS), Ok(FIRST));
        assert_eq!(cpu.tlb_tag[Access::Load as usize][index], TLB_INVALID);

        assert_eq!(cpu.st::<_, 8>(&mut bus, ADDRESS, SECOND), Ok(()));
        assert_eq!(cpu.jtlb_tag[1][index], ADDRESS & TLB_PAGE_MASK);
        cpu.tlb_tag[Access::Store as usize][index] = TLB_INVALID;
        assert_eq!(cpu.st::<_, 8>(&mut bus, ADDRESS, FIRST), Ok(()));
        assert_eq!(cpu.tlb_tag[Access::Store as usize][index], TLB_INVALID);
        assert_eq!(
            u64::from_le_bytes(
                bus.0.mem[ADDRESS as usize..ADDRESS as usize + 8]
                    .try_into()
                    .unwrap()
            ),
            FIRST
        );

        // Disabling the candidate restores the authoritative fallback.
        cpu.set_interpreter_fused_memory(false);
        cpu.jtlb_tag[0][index] = TLB_INVALID;
        assert_eq!(cpu.ld::<_, 8>(&mut bus, ADDRESS), Ok(FIRST));
        assert_eq!(cpu.jtlb_tag[0][index], ADDRESS & TLB_PAGE_MASK);
    }

    #[test]
    fn interpreter_fused_memory_preserves_all_scalar_widths_and_unaligned_accesses() {
        const ADDRESS: u64 = 0x4103;
        const LOAD_BYTES: [u8; 8] = [0x81, 0x72, 0x63, 0x54, 0x45, 0x36, 0x27, 0x18];

        let mut memory = vec![0u8; 0x10_000];
        memory[ADDRESS as usize..ADDRESS as usize + LOAD_BYTES.len()].copy_from_slice(&LOAD_BYTES);
        let mut bus = DirectFlatMemory(FlatMemory::new(0, &mut memory));
        let mut cpu = Cpu::new();
        cpu.set_interpreter_fused_memory(true);

        // Warm one exact load capability through the ordinary path, then
        // remove its standard-TLB counterpart so every following width must
        // use the fused pointer. ADDRESS is intentionally unaligned.
        assert_eq!(cpu.ld::<_, 8>(&mut bus, ADDRESS), Ok(0x1827_3645_5463_7281));
        let index = ((ADDRESS >> 12) as usize) & (TLB_SIZE - 1);
        cpu.tlb_tag[Access::Load as usize][index] = TLB_INVALID;
        assert_eq!(cpu.ld::<_, 1>(&mut bus, ADDRESS), Ok(0x81));
        assert_eq!(cpu.ld::<_, 2>(&mut bus, ADDRESS), Ok(0x7281));
        assert_eq!(cpu.ld::<_, 4>(&mut bus, ADDRESS), Ok(0x5463_7281));
        assert_eq!(cpu.ld::<_, 8>(&mut bus, ADDRESS), Ok(0x1827_3645_5463_7281));
        assert_eq!(cpu.tlb_tag[Access::Load as usize][index], TLB_INVALID);

        // Warm the store capability once, then exercise non-overlapping,
        // unaligned direct writes at every architectural scalar width.
        assert_eq!(cpu.st::<_, 1>(&mut bus, ADDRESS + 16, 0xaa), Ok(()));
        cpu.tlb_tag[Access::Store as usize][index] = TLB_INVALID;
        assert_eq!(cpu.st::<_, 1>(&mut bus, ADDRESS + 17, 0xbb), Ok(()));
        assert_eq!(cpu.st::<_, 2>(&mut bus, ADDRESS + 19, 0xccdd), Ok(()));
        assert_eq!(cpu.st::<_, 4>(&mut bus, ADDRESS + 23, 0x1122_3344), Ok(()));
        assert_eq!(
            cpu.st::<_, 8>(&mut bus, ADDRESS + 29, 0x8899_aabb_ccdd_eeff),
            Ok(())
        );
        assert_eq!(cpu.tlb_tag[Access::Store as usize][index], TLB_INVALID);
        assert_eq!(bus.0.mem[(ADDRESS + 17) as usize], 0xbb);
        assert_eq!(
            &bus.0.mem[(ADDRESS + 19) as usize..(ADDRESS + 21) as usize],
            &0xccddu16.to_le_bytes()
        );
        assert_eq!(
            &bus.0.mem[(ADDRESS + 23) as usize..(ADDRESS + 27) as usize],
            &0x1122_3344u32.to_le_bytes()
        );
        assert_eq!(
            &bus.0.mem[(ADDRESS + 29) as usize..(ADDRESS + 37) as usize],
            &0x8899_aabb_ccdd_eeffu64.to_le_bytes()
        );
    }

    #[test]
    fn privilege_tlb_retention_switch_restores_the_cold_flush_baseline() {
        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        cpu.set_privilege_tlb_retention(false);
        {
            let sys = cpu.sys.as_mut().unwrap();
            sys.mode = Mode::User;
            sys.medeleg = 1 << 8;
            sys.stvec = 0x8000;
        }
        let index = 7;
        cpu.tlb_tag[Access::Load as usize][index] = 0x7000;
        cpu.jtlb_tag[0][index] = 0x7000;
        cpu.pc = 0x4000;
        cpu.take_trap(8, 0, false);
        assert_eq!(cpu.tlb_tag[Access::Load as usize][index], TLB_INVALID);
        assert_eq!(cpu.jtlb_tag[0][index], TLB_INVALID);
    }

    #[test]
    fn context_tagged_tlbs_survive_non_mapping_status_writes() {
        const VA: u64 = 0x1234_5000;
        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        cpu.sys.as_mut().unwrap().mode = Mode::Supervisor;
        cpu.sync_jit_tlb_context();
        let index = cpu.jit_tlb_index(VA);
        let supervisor_tag = cpu.translation_tag(VA, Access::Load);
        cpu.tlb_tag[Access::Load as usize][index] = supervisor_tag;
        cpu.jtlb_tag[0][index] = supervisor_tag;

        assert!(cpu.csr_write(SSTATUS, MSTATUS_SIE));
        assert_eq!(cpu.tlb_tag[Access::Load as usize][index], supervisor_tag);
        assert_eq!(cpu.jit_tlb_miss_kind(VA, false), 0);

        assert!(cpu.csr_write(SSTATUS, MSTATUS_SIE | MSTATUS_SUM));
        assert_eq!(cpu.tlb_tag[Access::Load as usize][index], supervisor_tag);
        assert_eq!(cpu.jtlb_tag[0][index], supervisor_tag);
        assert_eq!(cpu.jit_tlb_miss_kind(VA, false), 2);

        cpu.set_privilege_tlb_retention(false);
        cpu.jtlb_tag[0][index] = cpu.translation_tag(VA, Access::Load);
        assert!(cpu.csr_write(SSTATUS, MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SUM));
        assert_eq!(cpu.jtlb_tag[0][index], TLB_INVALID);
    }

    #[test]
    fn rdtime_derives_live_from_insn_count() {
        // In full-system mode the machine sets time_scale/time_offset so rdtime
        // advances every instruction (matching the CLINT clock at instruction
        // granularity) instead of only at slice boundaries — kernel busy-wait
        // loops like __delay read rdtime tightly and must see it move.
        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        {
            let sys = cpu.sys.as_mut().unwrap();
            sys.time_scale = 10;
            sys.time_offset = 5;
        }
        cpu.insn_count = 0;
        assert_eq!(cpu.csr_read(TIME), Some(5)); // 0/10 + 5
        cpu.insn_count = 100;
        assert_eq!(cpu.csr_read(TIME), Some(15)); // 100/10 + 5
                                                  // time_scale == 0 falls back to the mirrored mtime (legacy machine).
        {
            let sys = cpu.sys.as_mut().unwrap();
            sys.time_scale = 0;
            sys.mtime = 42;
        }
        assert_eq!(cpu.csr_read(TIME), Some(42));
    }

    #[test]
    fn compressed_instructions_execute() {
        // c.li a0, 21 (0x4555); c.mv a1, a0 (0x85aa); c.add a0, a1 (0x952e); ecall
        let mut mem = vec![0u8; 0x10000];
        let halves: [u16; 3] = [0x4555, 0x85aa, 0x952e];
        for (i, h) in halves.iter().enumerate() {
            mem[i * 2..i * 2 + 2].copy_from_slice(&h.to_le_bytes());
        }
        mem[6..10].copy_from_slice(&0x00000073u32.to_le_bytes());
        let mut cpu = Cpu::new();
        cpu.pc = BASE;
        let mut bus = FlatMemory::new(BASE, &mut mem);
        assert_eq!(cpu.run(&mut bus, 100), StopReason::Ecall);
        assert_eq!(cpu.x[10], 42); // a0 = 21 + 21
        assert_eq!(cpu.x[11], 21); // a1
    }

    #[test]
    fn integrated_fetch_capability_rereads_code_and_flushes_with_tlbs() {
        const PC: usize = 0x100;
        let mut memory = vec![0u8; 0x3000];
        memory[PC..PC + 4].copy_from_slice(&0x0010_0293u32.to_le_bytes()); // addi x5,x0,1
        let mut cpu = Cpu::new();
        cpu.pc = PC as u64;
        assert_eq!(
            cpu.run_integrated_scalar(&mut DirectFlatMemory(FlatMemory::new(0, &mut memory)), 1,),
            StopReason::Budget,
        );
        assert_eq!(cpu.x[5], 1);
        assert_ne!(cpu.interpreter_fetch_tag, TLB_INVALID);

        // The capability caches only translation and direct-page location.
        // Reusing it must observe changed bytes rather than a decoded opcode.
        memory[PC..PC + 4].copy_from_slice(&0x0020_0293u32.to_le_bytes()); // addi x5,x0,2
        cpu.pc = PC as u64;
        cpu.run_integrated_scalar(&mut DirectFlatMemory(FlatMemory::new(0, &mut memory)), 1);
        assert_eq!(cpu.x[5], 2);

        cpu.flush_tlb();
        assert_eq!(cpu.interpreter_fetch_tag, TLB_INVALID);
    }

    #[test]
    fn direct_compressed_execution_matches_expanded_reference() {
        const PC: u64 = 0x100;
        const DATA: u64 = 0x1000;
        const MEMORY_BYTES: usize = 0x3000;

        let mut direct_cpu = Cpu::new();
        let mut reference_cpu = Cpu::new();
        let mut direct_memory = vec![0u8; MEMORY_BYTES];
        let mut reference_memory = vec![0u8; MEMORY_BYTES];

        for raw in 0u32..=u16::MAX as u32 {
            if raw & 3 == 3 {
                continue;
            }
            let compressed = raw as u16;
            let expanded = crate::compressed::expand(compressed);

            for (index, byte) in direct_memory[DATA as usize..DATA as usize + 0x800]
                .iter_mut()
                .enumerate()
            {
                *byte = (index as u8).wrapping_mul(37).wrapping_add(11);
            }
            reference_memory[DATA as usize..DATA as usize + 0x800]
                .copy_from_slice(&direct_memory[DATA as usize..DATA as usize + 0x800]);
            direct_memory[PC as usize..PC as usize + 4].fill(0);
            reference_memory[PC as usize..PC as usize + 4].fill(0);
            direct_memory[PC as usize..PC as usize + 2].copy_from_slice(&compressed.to_le_bytes());

            let reset = |cpu: &mut Cpu| {
                cpu.pc = PC;
                cpu.insn_count = 17;
                cpu.reservation = None;
                cpu.fcsr = 1;
                cpu.sys = None;
                for register in 0..32 {
                    cpu.x[register] = DATA + register as u64 * 8;
                    cpu.f[register] =
                        0x0123_4567_89ab_cdefu64.wrapping_add(register as u64 * 0x0101_0101);
                }
                cpu.x[0] = 0;
                cpu.x[2] = DATA;
            };
            reset(&mut direct_cpu);
            reset(&mut reference_cpu);
            let initial_x = reference_cpu.x;

            let direct_result = {
                let mut bus = FlatMemory::new(0, &mut direct_memory);
                direct_cpu.step(&mut bus)
            };
            let Some(expanded) = expanded else {
                assert_eq!(
                    direct_result,
                    Err(Exception::IllegalInstruction { insn: raw }),
                    "reserved compressed encoding 0x{raw:04x} must remain illegal",
                );
                continue;
            };
            reference_memory[PC as usize..PC as usize + 4].copy_from_slice(&expanded.to_le_bytes());
            let reference_result = {
                let mut bus = FlatMemory::new(0, &mut reference_memory);
                reference_cpu.step(&mut bus)
            };
            assert_eq!(
                direct_result, reference_result,
                "compressed result mismatch for 0x{raw:04x} -> 0x{expanded:08x}",
            );

            let op = opcode(expanded);
            let expected_pc = match op {
                0x6f | 0x67 => reference_cpu.pc,
                0x63 => {
                    let a = initial_x[rs1(expanded)];
                    let b = initial_x[rs2(expanded)];
                    let taken = match funct3(expanded) {
                        0 => a == b,
                        1 => a != b,
                        4 => (a as i64) < (b as i64),
                        5 => (a as i64) >= (b as i64),
                        6 => a < b,
                        7 => a >= b,
                        _ => false,
                    };
                    if taken {
                        reference_cpu.pc
                    } else {
                        PC + 2
                    }
                }
                _ => PC + 2,
            };
            assert_eq!(
                direct_cpu.pc, expected_pc,
                "compressed PC mismatch for 0x{raw:04x} -> 0x{expanded:08x}",
            );

            let mut expected_x = reference_cpu.x;
            if op == 0x67 && rd(expanded) == 1 {
                expected_x[1] = PC + 2;
            }
            assert_eq!(
                direct_cpu.x, expected_x,
                "compressed GPR mismatch for 0x{raw:04x} -> 0x{expanded:08x}",
            );
            assert_eq!(direct_cpu.f, reference_cpu.f);
            assert_eq!(direct_cpu.fcsr, reference_cpu.fcsr);
            assert_eq!(direct_cpu.insn_count, reference_cpu.insn_count);
            assert_eq!(
                &direct_memory[DATA as usize..DATA as usize + 0x800],
                &reference_memory[DATA as usize..DATA as usize + 0x800],
                "compressed memory mismatch for 0x{raw:04x} -> 0x{expanded:08x}",
            );
        }
    }
    #[test]
    fn illegal_instruction_traps() {
        let mut mem = vec![0u8; 0x100];
        let mut cpu = Cpu::new();
        cpu.pc = 0;
        let mut bus = FlatMemory::new(0, &mut mem);
        // all-zero word is defined illegal in RISC-V
        match cpu.run(&mut bus, 10) {
            StopReason::Trap(Exception::IllegalInstruction { .. }) => {}
            other => panic!("expected illegal instruction, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod fp_fastpath_tests {
    use super::Cpu;
    use crate::softfp::{sf32, sf64, FFLAG_INEXACT, RM_RNE};

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// The fast path must be bit-identical to softfp, and softfp must set
    /// no flag beyond NX whenever the fast path considered itself eligible
    /// (that's the entire correctness argument for skipping flag math).
    #[test]
    fn fast64_matches_softfp() {
        let mut rng = Rng(0x9E3779B97F4A7C15);
        let mut hits = 0u32;
        for i in 0..300_000 {
            let (mut a, mut b) = (rng.next(), rng.next());
            if i % 3 == 0 {
                // bias exponents toward mid-range so most samples are
                // eligible normals, not NaN/inf/subnormal rejects
                a = (a & !(0x7ff << 52)) | (((a >> 52) % 0x600 + 0x100) << 52);
                b = (b & !(0x7ff << 52)) | (((b >> 52) % 0x600 + 0x100) << 52);
            }
            let op = (rng.next() % 4) as u32;
            if let Some(fast) = Cpu::fp_fast64(op, a, b) {
                hits += 1;
                let mut fl = 0u32;
                let soft = match op {
                    0 => sf64::add(a, b, RM_RNE, &mut fl),
                    1 => sf64::sub(a, b, RM_RNE, &mut fl),
                    2 => sf64::mul(a, b, RM_RNE, &mut fl),
                    _ => sf64::div(a, b, RM_RNE, &mut fl),
                };
                assert_eq!(fast, soft, "op {op} a={a:#x} b={b:#x}");
                assert_eq!(
                    fl & !FFLAG_INEXACT,
                    0,
                    "op {op} a={a:#x} b={b:#x} flags {fl:#x}"
                );
            }
        }
        assert!(hits > 50_000, "fast path rarely eligible: {hits}");
    }

    #[test]
    fn fast32_matches_softfp() {
        let mut rng = Rng(0xDEADBEEFCAFED00D);
        let mut hits = 0u32;
        for i in 0..300_000 {
            let (mut a, mut b) = (rng.next() as u32, rng.next() as u32);
            if i % 3 == 0 {
                a = (a & !(0xff << 23)) | ((((a >> 23) % 0xc0) + 0x20) << 23);
                b = (b & !(0xff << 23)) | ((((b >> 23) % 0xc0) + 0x20) << 23);
            }
            let op = (rng.next() % 4) as u32;
            if let Some(fast) = Cpu::fp_fast32(op, a, b) {
                hits += 1;
                let mut fl = 0u32;
                let soft = match op {
                    0 => sf32::add(a, b, RM_RNE, &mut fl),
                    1 => sf32::sub(a, b, RM_RNE, &mut fl),
                    2 => sf32::mul(a, b, RM_RNE, &mut fl),
                    _ => sf32::div(a, b, RM_RNE, &mut fl),
                };
                assert_eq!(fast, soft, "op {op} a={a:#x} b={b:#x}");
                assert_eq!(
                    fl & !FFLAG_INEXACT,
                    0,
                    "op {op} a={a:#x} b={b:#x} flags {fl:#x}"
                );
            }
        }
        assert!(hits > 50_000, "fast path rarely eligible: {hits}");
    }
}
