//! wasm export surface for rv64.js.
//!
//! v86-style: a plain `extern "C"` ABI over wasm linear memory — no
//! wasm-bindgen. `web/rv64.js` instantiates the module and talks to these
//! exports directly.
//!
//! Two APIs:
//! - **raw CPU** (`init`/`run`/`get_reg`/...): bare hart + flat RAM, used by
//!   the phase-0 demo and tests.
//! - **user-mode Linux** (`user_*`): load a static riscv64 ELF and run it,
//!   syscalls serviced by rv64-linux. Console output and clock/entropy go
//!   through imported host functions (see `extern "C"` imports below).
//!
//! Single-instance (v86's model): one emulator per wasm instantiation.

use rv64_core::{csr::Mode, Bus, Cpu, FlatMemory, StopReason};
use rv64_linux::{Host, Machine};
use std::cell::Cell;
use std::hash::{BuildHasher, Hasher};

// ---- host imports (provided by web/rv64.js) -----------------------------

#[link(wasm_import_module = "env")]
extern "C" {
    /// Console output from the guest (fd 1 = stdout, 2 = stderr).
    fn host_write(fd: i32, ptr: *const u8, len: usize);
    /// Milliseconds since an arbitrary epoch (performance.now()).
    fn host_now_ms() -> f64;
    /// Milliseconds since the Unix epoch (Date.now()), for the guest RTC.
    fn host_unix_ms() -> f64;
    /// Fill with entropy (crypto.getRandomValues).
    fn host_random(ptr: *mut u8, len: usize);
    /// JIT: instantiate the wasm module currently in JIT_OUT (see
    /// jit_out_ptr/jit_out_len), append its `run` function to this module's
    /// exported function table, and return the table index (-1 on failure).
    fn host_jit_register() -> i32;
    /// Instantiate the BATCH module in JIT_OUT and append its `r0`..`r{n-1}`
    /// exports to the function table CONTIGUOUSLY; returns the base index
    /// (-1 on failure). Members transfer to each other by direct tail call
    /// inside the module, so nothing imports the table and registration
    /// stays O(1) per batch.
    fn host_jit_register_batch(n: u32) -> i32;
    /// One Ethernet frame the guest transmitted, for the page to forward over
    /// its WebSocket relay. Called at quantum granularity, like host_write.
    fn host_net_send(ptr: *const u8, len: usize);
    /// One HTTP request the in-process proxy wants performed, encoded by
    /// `httpproxy::Request::encode`. The page performs it with `fetch()` and
    /// calls `sys_http_response` when it completes — asynchronously, so this
    /// returns immediately and the guest's TCP connection stays open meanwhile.
    fn host_http_request(id: u64, ptr: *const u8, len: usize);
    /// Transparent guest TCP stream events for a WISP transport.
    fn host_wisp_open(id: u64, address: *const u8, port: u32);
    fn host_wisp_data(id: u64, ptr: *const u8, len: usize);
    fn host_wisp_close(id: u64);
    fn host_wisp_datagram(id: u64, address: *const u8, port: u32, ptr: *const u8, len: usize);
    /// Async variant for large modules (page superblocks): compiles on V8
    /// background threads; sys_sb_ready(ticket, idx) fires between runSystem
    /// calls when the function is in the table (idx -1 = failed).
    fn host_jit_register_async(ticket: u64);
}

fn tls_random(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    unsafe { host_random(buf.as_mut_ptr(), buf.len()) }
    Ok(())
}

getrandom::register_custom_getrandom!(tls_random);

// JIT state is keyed almost exclusively by PCs and pairs of virtual/physical
// page addresses. Rust's DoS-resistant SipHash is disproportionately expensive
// for those one- and two-word keys in wasm32. Keep randomized map-local seeds,
// but use a compact avalanche permutation tailored to integer state tables.
const JIT_HASH_MIX: u64 = 0xd6e8_feb8_6659_fd93;
const JIT_HASH_STEP: u64 = 0x9e37_79b9_7f4a_7c15;

#[inline(always)]
fn jit_hash_avalanche(mut value: u64) -> u64 {
    value ^= value >> 32;
    value = value.wrapping_mul(JIT_HASH_MIX);
    value ^= value >> 32;
    value = value.wrapping_mul(JIT_HASH_MIX);
    value ^ (value >> 32)
}

fn initial_jit_hash_seed() -> u64 {
    let mut seed = 0_u64;
    unsafe { host_random((&mut seed as *mut u64).cast(), core::mem::size_of::<u64>()) };
    let mixed = jit_hash_avalanche(seed ^ JIT_HASH_STEP);
    if mixed == 0 {
        JIT_HASH_MIX
    } else {
        mixed
    }
}

thread_local! {
    /// One crypto-seeded sequence per Wasm thread. Map construction advances
    /// it; lookup and insertion never cross the Wasm/host boundary.
    static JIT_HASH_SEED: Cell<u64> = Cell::new(initial_jit_hash_seed());
}

#[derive(Clone, Copy)]
struct FastBuildHasher {
    seed: u64,
}

impl FastBuildHasher {
    #[cfg(test)]
    const fn with_seed(seed: u64) -> Self {
        Self { seed }
    }
}

impl Default for FastBuildHasher {
    fn default() -> Self {
        JIT_HASH_SEED.with(|sequence| {
            let next = jit_hash_avalanche(sequence.get().wrapping_add(JIT_HASH_STEP));
            sequence.set(next);
            Self { seed: next }
        })
    }
}

struct FastHasher {
    state: u64,
}

impl BuildHasher for FastBuildHasher {
    type Hasher = FastHasher;

    #[inline(always)]
    fn build_hasher(&self) -> Self::Hasher {
        FastHasher { state: self.seed }
    }
}

impl FastHasher {
    #[inline(always)]
    fn word(&mut self, value: u64) {
        self.state = jit_hash_avalanche(self.state ^ value.wrapping_add(JIT_HASH_STEP));
    }
}

impl Hasher for FastHasher {
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.state
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.word(u64::from_ne_bytes(chunk.try_into().unwrap()));
        }
        let tail = chunks.remainder();
        if !tail.is_empty() {
            let mut word = [0_u8; 8];
            word[..tail.len()].copy_from_slice(tail);
            self.word(u64::from_ne_bytes(word) ^ ((tail.len() as u64) << 56));
        }
    }

    #[inline(always)]
    fn write_u8(&mut self, value: u8) {
        self.word(value.into());
    }
    #[inline(always)]
    fn write_u16(&mut self, value: u16) {
        self.word(value.into());
    }
    #[inline(always)]
    fn write_u32(&mut self, value: u32) {
        self.word(value.into());
    }
    #[inline(always)]
    fn write_u64(&mut self, value: u64) {
        self.word(value);
    }
    #[inline(always)]
    fn write_usize(&mut self, value: usize) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_i8(&mut self, value: i8) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_i16(&mut self, value: i16) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_i32(&mut self, value: i32) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_i64(&mut self, value: i64) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_isize(&mut self, value: isize) {
        self.word(value as u64);
    }
}

type FastHashMap<K, V> = std::collections::HashMap<K, V, FastBuildHasher>;
type FastHashSet<K> = std::collections::HashSet<K, FastBuildHasher>;

#[cfg(test)]
mod fast_hash_tests {
    use super::{FastBuildHasher, FastHashMap};
    use std::hash::{BuildHasher, Hash};

    fn hash<T: Hash>(builder: FastBuildHasher, value: T) -> u64 {
        builder.hash_one(value)
    }

    #[test]
    fn same_seed_is_stable_and_different_seeds_diverge() {
        let first = FastBuildHasher::with_seed(0x1234_5678_9abc_def0);
        let same = FastBuildHasher::with_seed(0x1234_5678_9abc_def0);
        let other = FastBuildHasher::with_seed(0xfedc_ba98_7654_3210);
        let key = (0xffff_ffc0_1234_5000_u64, 0x9234_5000_u64);
        assert_eq!(hash(first, key), hash(same, key));
        assert_ne!(hash(first, key), hash(other, key));
        assert_ne!(hash(first, key), hash(first, (key.1, key.0)));
    }

    #[test]
    fn one_word_hash_has_no_sequential_or_page_shaped_collisions() {
        const KEYS: u64 = 1_000_000;
        let builder = FastBuildHasher::with_seed(0x6a09_e667_f3bc_c909);
        let mut hashes = std::collections::HashSet::with_capacity(KEYS as usize);
        for key in 0..KEYS {
            assert!(
                hashes.insert(hash(builder, key)),
                "sequential collision at {key}"
            );
        }
        hashes.clear();
        for index in 0..KEYS {
            let page = 0xffff_ffc0_0000_0000_u64.wrapping_add(index << 12);
            assert!(
                hashes.insert(hash(builder, page)),
                "page-shaped collision at {index}"
            );
        }
    }

    #[test]
    fn integer_map_semantics_match_std_hash_map() {
        let builder = FastBuildHasher::with_seed(0xbb67_ae85_84ca_a73b);
        let mut fast = FastHashMap::with_hasher(builder);
        let mut standard = std::collections::HashMap::new();
        for index in 0..65_536_u64 {
            let key = (
                0xffff_ffc0_0000_0000_u64.wrapping_add(index << 12),
                0x8000_0000_u64.wrapping_add(index.wrapping_mul(0x9e37) << 12),
            );
            fast.insert(key, index.wrapping_mul(17));
            standard.insert(key, index.wrapping_mul(17));
        }
        assert_eq!(fast.len(), standard.len());
        for (key, expected) in standard {
            assert_eq!(fast.get(&key), Some(&expected));
        }
    }
}

struct JsHost;

impl Host for JsHost {
    fn write_out(&mut self, fd: i32, bytes: &[u8]) {
        unsafe { host_write(fd, bytes.as_ptr(), bytes.len()) }
    }
    fn clock_ns(&mut self) -> u64 {
        (unsafe { host_now_ms() } * 1e6) as u64
    }
    fn random(&mut self, buf: &mut [u8]) {
        unsafe { host_random(buf.as_mut_ptr(), buf.len()) }
    }
}

// ---- shared staging buffer (JS -> wasm data transfer) --------------------

static mut STAGING: Vec<u8> = Vec::new();

/// Resize the staging buffer and return its pointer; JS copies data in.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn staging_alloc(len: usize) -> *mut u8 {
    unsafe {
        STAGING.clear();
        STAGING.resize(len, 0);
        STAGING.as_mut_ptr()
    }
}

/// Pointer to the staging buffer WITHOUT resizing or clearing it, for reading
/// data the core placed there. `staging_alloc` is the write path and empties the
/// buffer, so it cannot be used to read a result back out.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn staging_ptr() -> *const u8 {
    unsafe { STAGING.as_ptr() }
}

// ---- raw CPU API ---------------------------------------------------------

struct RawEmu {
    cpu: Cpu,
    ram: Vec<u8>,
    ram_base: u64,
}

static mut RAW: Option<RawEmu> = None;

const STOP_BUDGET: i32 = 0;
const STOP_ECALL: i32 = 1;
const STOP_BREAK: i32 = 2;
const STOP_TRAP: i32 = 3;
const STOP_EXITED: i32 = 4;

static mut LAST_TRAP: i32 = -1;

#[allow(static_mut_refs)]
fn raw() -> &'static mut RawEmu {
    unsafe { RAW.as_mut().expect("call init() first") }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn init(base: u64, size: u32) {
    let mut cpu = Cpu::new();
    cpu.pc = base;
    unsafe {
        RAW = Some(RawEmu {
            cpu,
            ram: vec![0; size as usize],
            ram_base: base,
        })
    }
}

#[no_mangle]
pub extern "C" fn mem_ptr() -> *mut u8 {
    raw().ram.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn mem_size() -> u32 {
    raw().ram.len() as u32
}

#[no_mangle]
pub extern "C" fn get_pc() -> u64 {
    raw().cpu.pc
}

#[no_mangle]
pub extern "C" fn set_pc(pc: u64) {
    raw().cpu.pc = pc;
}

#[no_mangle]
pub extern "C" fn get_reg(i: u32) -> u64 {
    raw().cpu.x[(i & 31) as usize]
}

#[no_mangle]
pub extern "C" fn set_reg(i: u32, val: u64) {
    if i != 0 {
        raw().cpu.x[(i & 31) as usize] = val;
    }
}

#[no_mangle]
pub extern "C" fn insn_count() -> u64 {
    raw().cpu.insn_count
}

#[no_mangle]
pub extern "C" fn run(budget: u64) -> i32 {
    let e = raw();
    let mut bus = FlatMemory::new(e.ram_base, &mut e.ram);
    match e.cpu.run(&mut bus, budget) {
        StopReason::Budget => STOP_BUDGET,
        StopReason::Ecall => STOP_ECALL,
        StopReason::Break => STOP_BREAK,
        StopReason::Wfi => STOP_BUDGET, // raw API has no system mode yet
        StopReason::Trap(exc) => {
            unsafe { LAST_TRAP = exc.cause() as i32 };
            STOP_TRAP
        }
    }
}

#[no_mangle]
pub extern "C" fn trap_cause() -> i32 {
    unsafe { LAST_TRAP }
}

// ---- user-mode Linux API --------------------------------------------------

struct UserEmu {
    machine: Machine,
    exit_code: i32,
}

static mut USER: Option<UserEmu> = None;
static mut USER_ARGS: Vec<String> = Vec::new();

/// Append one argv string (staged via staging_alloc + copy).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_arg_push() {
    unsafe {
        let s = String::from_utf8_lossy(&STAGING).into_owned();
        USER_ARGS.push(s);
    }
}

/// Load the ELF currently in the staging buffer with the pushed argv.
/// Returns 0 on success, negative on load error.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_load(mem_size: u32) -> i32 {
    let mut host = JsHost;
    unsafe {
        let argv: Vec<&str> = USER_ARGS.iter().map(String::as_str).collect();
        let argv: &[&str] = if argv.is_empty() { &["guest"] } else { &argv };
        let envp = ["PATH=/bin", "HOME=/", "TERM=dumb"];
        match Machine::load(&STAGING, argv, &envp, mem_size as usize, &mut host) {
            Ok(machine) => {
                USER = Some(UserEmu {
                    machine,
                    exit_code: 0,
                });
                // New address space: any compiled blocks are stale.
                if let Some(j) = USER_JIT.as_mut() {
                    j.clear();
                }
                USER_TRANSLATE_NS = 0;
                USER_TRANSLATE_ATTEMPTS = 0;
                USER_EMITTED_BYTES = 0;
                STAGING.clear();
                USER_ARGS.clear();
                0
            }
            Err(_) => -1,
        }
    }
}

// ---- JIT dispatch state ---------------------------------------------------

#[derive(Clone, Copy)]
struct JitBlock {
    /// Trace touches the FP file (claim policy: see TRACE_KEEP_MIN).
    fp: bool,
    /// Function-table index of the compiled block.
    idx: i32,
    /// Guest instructions it retires.
    n: u32,
    /// Static ordinary-trace mix (ALU/load/store/control/FP).
    mix: [u16; 5],
    mem: [u16; 10],
    control: [u16; 3],
    alu: [u16; 5],
    /// Physical address of the code (full-system: verified per dispatch).
    pa: u64,
}

/// Direct-mapped dispatch line: `pc` is the full key. A slot with
/// `pc == NO_PC` is empty. Indexed by low pc bits — a single array read +
/// compare replaces the HashMap+SipHash lookup on the hot path. Deliberately
/// PACKED to 16 bytes: at 200M+ dispatches/s the line load is the loop's main
/// memory traffic, and 16B lines double how many fit in cache versus carrying
/// the whole JitBlock (pa/n live in `cache` and are only needed on the rare
/// verify path).
#[derive(Clone, Copy)]
#[repr(C)] // compiled blocks read lines directly: pc @0, idx @8, gen @12
struct DispatchLine {
    pc: u64,
    /// Function-table index (JitBlock.idx; < 0 = blacklisted sentinel).
    idx: i32,
    /// Low 32 bits of cpu.map_gen at last successful pa-verify. The fast path
    /// re-verifies via the authoritative cache only when the generation moved
    /// (SFENCE.VMA / satp write); a wraparound false-mismatch just re-probes.
    gen: u32,
}

/// A landed async entry is visible to the interpreter before its first
/// address-space-specific PA verification. The host dispatch path must never
/// execute such a line directly; it resolves the authoritative `JitBlock`,
/// probes the mapping, and replaces this generation with the live map_gen.
const UNVERIFIED_DISPATCH_GEN: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct SuccessorProfile {
    owner: u64,
    targets: [u64; 2],
    counts: [u32; 2],
}

impl SuccessorProfile {
    const EMPTY: Self = Self {
        owner: NO_PC,
        targets: [0; 2],
        counts: [0; 2],
    };

    fn observe(&mut self, owner: u64, target: u64) {
        if self.owner != owner {
            *self = Self {
                owner,
                targets: [target, 0],
                counts: [1, 0],
            };
            return;
        }
        if let Some(index) =
            (0..2).find(|&index| self.counts[index] != 0 && self.targets[index] == target)
        {
            self.counts[index] = self.counts[index].saturating_add(1);
            return;
        }
        if let Some(index) = self.counts.iter().position(|&count| count == 0) {
            self.targets[index] = target;
            self.counts[index] = 1;
            return;
        }

        // Bounded Misra-Gries replacement: preserve two genuinely frequent
        // targets without allocating on the dispatch path. A third occasional
        // exit decays both candidates instead of immediately evicting either.
        for count in &mut self.counts {
            *count -= 1;
        }
    }

    fn ranked_targets(self, owner: u64, minimum: u32) -> Vec<u64> {
        if self.owner != owner {
            return Vec::new();
        }
        let mut candidates: Vec<(u64, u32)> = (0..2)
            .filter(|&index| self.counts[index] >= minimum)
            .map(|index| (self.targets[index], self.counts[index]))
            .collect();
        candidates.sort_unstable_by_key(|&(_, count)| core::cmp::Reverse(count));
        candidates.into_iter().map(|(target, _)| target).collect()
    }

    fn hottest(self, owner: u64, minimum: u32) -> Option<u64> {
        self.ranked_targets(owner, minimum).into_iter().next()
    }

    fn target_count(self, owner: u64, target: u64) -> u32 {
        if self.owner != owner {
            return 0;
        }
        (0..2)
            .find(|&index| self.counts[index] != 0 && self.targets[index] == target)
            .map_or(0, |index| self.counts[index])
    }

    fn is_dominant(self, owner: u64, target: u64, factor: u32) -> bool {
        let target_count = self.target_count(owner, target);
        let other = (0..2)
            .filter(|&index| self.counts[index] != 0 && self.targets[index] != target)
            .map(|index| self.counts[index])
            .max()
            .unwrap_or(0);
        other == 0 || target_count >= other.saturating_mul(factor)
    }
}

const NO_PC: u64 = u64::MAX;
// 262144 lines: CPython's ~20k hot pcs collided at ~7% in 131072 under
// guest ASLR — per-boot slot-eviction churn is one suspected source of the
// python row's 3.3-6.7s bimodality. Doubling costs 2MB and no hot-path work.
const DISPATCH_BITS: u32 = 18;
const DISPATCH_SIZE: usize = 1 << DISPATCH_BITS;

#[derive(Clone, Copy)]
struct PagePolicyCandidate {
    aspace: u64,
    vpage: u64,
    pa_page: u64,
}

struct JitState {
    /// pc -> compiled block; None = tried and not translatable (blacklist).
    /// Authoritative store (iterated for per-page invalidation).
    cache: FastHashMap<u64, Option<JitBlock>>,
    hot: FastHashMap<u64, u32>,
    /// Fast dispatch cache: direct-mapped, populated lazily from `cache`.
    dispatch: Vec<DispatchLine>,
    /// Last observed cpu.jit_flush_gen; a change means the va→pa code
    /// mapping was invalidated (satp/SFENCE) — drop everything.
    flush_gen: u64,
    /// Superblock compilation (v86's function-per-page): the hot block-entry
    /// pcs discovered in each guest code page (keyed by virtual page base). When
    /// a new entry appears the page's superblock is recompiled to cover it, and
    /// every entry's `cache`/`dispatch` slot points at the one superblock.
    page_entries: FastHashMap<(u64, u64), Vec<u64>>,
    /// Cheap direct-mapped hot counter for the interpreter-fallback path: an
    /// interpreted stretch's interior blocks never reach the fast-path hot map
    /// (run_slice_until never returns to the fast path inside a stretch), so
    /// they'd stay interpreted forever (~half of fib's time). Bumping a real
    /// HashMap per interpreted instruction taxes cold boot, so count in a u16
    /// array here and only touch `hot` when a slot actually gets hot.
    interp_hot: Vec<u16>,
    /// Full-pc tag for each interp_hot slot (low 32 bits of pc>>1). Untagged
    /// direct-mapped counters let unrelated pcs (or another address space)
    /// inherit a slot's heat and get compiled on their first execution —
    /// compile storms that depend on address layout (PERFORMANCE_PROGRESS.md).
    interp_hot_tag: Vec<u32>,
    /// Physical page -> pcs of cache entries whose code lives there (blocks
    /// AND pa-stamped blacklist sentinels). Lets dirty-page invalidation drop
    /// exactly the affected entries instead of scanning the whole cache per
    /// page — the cache persists across context switches now, so it's large.
    page_blocks: FastHashMap<u64, Vec<u64>>,
    /// Virtual pages already compiled as a superblock — compile ONCE per page
    /// (with whatever entries were hot then); later hot pcs in the page get
    /// individual blocks. Recompiling the page's big br_table function on every
    /// new entry was a 2x regression on short workloads (the recompile storm).
    superblocked: FastHashSet<(u64, u64)>,
    /// Virtual page -> (hot entries at its last superblock compile, number of
    /// UNPRODUCTIVE compiles — rebuilds that covered no new hot pcs). A page's first superblock is built from whatever handful of
    /// pcs was hot at the threshold; code discovered later — a second function
    /// in the page, a callee reached only by an indirect call — would stay on
    /// individual blocks forever (measured: nbench IDEA ran cipher_idea as 1-15
    /// instruction blocks, 6.4 insns per dispatch, on a page that WAS
    /// superblocked). Recompile once the page has accumulated another
    /// threshold's worth of uncovered hot pcs, bounded so a pathological page
    /// can't loop on it.
    sb_gen: FastHashMap<(u64, u64), (usize, u32, u64)>,
    /// Pages that wanted a superblock while the compile budget was spent.
    /// Drained one per quantum boundary, oldest first.
    sb_queue: Vec<(u64, u64)>,
    /// Hot pcs on a superblocked page that had to get their own block because
    /// the page function does not cover them — the direct measure of a page
    /// function that has fallen behind the code actually running.
    sb_missed: FastHashMap<(u64, u64), u32>,
    /// Observed successor of each dispatched pc, direct-mapped by dispatch
    /// slot (pc, next_pc). This is the NEXT-EXECUTING-TAIL signal that
    /// trace-tree JITs form regions from: batches built from STATIC exit
    /// seeds only kept ~12% of exits in-batch, because a trace's textual
    /// successors are not the ones execution actually takes.
    succ: Vec<SuccessorProfile>,
    /// Entry PC -> target set already embedded in its indirect specialization.
    /// The set grows at most once, from monomorphic to two-way polymorphic, so
    /// a megamorphic site cannot drive unbounded recompilation.
    ic_targets: FastHashMap<u64, Vec<u64>>,
    /// Table index -> the (virtual page, physical page) list a MULTI-page
    /// superblock was compiled over. Entries carry their own page's pa (probed
    /// like any block at dispatch); this is the rest of the region, verified on
    /// the same slow path so a region can never execute against a page that was
    /// remapped out from under it.
    regions: FastHashMap<i32, Vec<(u64, u64)>>,
    /// Table index -> live exit profile of a landed region function: which
    /// pages its (sampled) exits transfer control to. This is the measured
    /// signal that drives incremental region EXTENSION — a region grows only
    /// along traffic it demonstrably loses dispatches to, never from
    /// reachability guesses (which glued cold callees into hot regions and
    /// regressed the FP rows; see build_superblock).
    region_exits: FastHashMap<i32, RegionExits>,
    /// Regions whose sampled out-of-region exit count crossed EXT_TRIGGER,
    /// awaiting a build slot at a quantum boundary.
    ext_queue: Vec<i32>,
    /// Experimental v86-style page heat keyed by the shareable code mapping:
    /// virtual page + physical page, deliberately not SATP. Dispatch rechecks
    /// the VA->PA mapping before generated code can run.
    policy_heat: FastHashMap<(u64, u64), u64>,
    policy_last_sample: FastHashMap<(u64, u64), u64>,
    policy_entries: FastHashMap<(u64, u64), Vec<u64>>,
    /// Per-mapping sampling mix used to distinguish compact direct-control
    /// code from computed-dispatch pages before choosing multi-page geometry.
    policy_observations: FastHashMap<(u64, u64), u64>,
    policy_control_observations: FastHashMap<(u64, u64), u64>,
    /// Last physical code page observed for `(satp, virtual page)`. The page
    /// policy's reusable heat/entry identity deliberately remains `(VA, PA)`,
    /// but assembling a multi-page module also needs proof that each neighbour
    /// was observed in the SAME address space as the lead. First dispatch
    /// still revalidates every recorded VA -> PA mapping before execution.
    policy_mappings: FastHashMap<(u64, u64), u64>,
    policy_attempted: FastHashMap<(u64, u64), FastHashSet<u64>>,
    policy_installed: FastHashMap<(u64, u64), FastHashSet<u64>>,
    policy_queue: std::collections::VecDeque<PagePolicyCandidate>,
    policy_queued: FastHashSet<(u64, u64)>,
    policy_suppressed: FastHashSet<(u64, u64)>,
    policy_pending: FastHashSet<(u64, u64)>,
    policy_compiled: FastHashSet<(u64, u64)>,
    policy_rejected: FastHashSet<(u64, u64)>,
    /// Diagnostic inventory of already-landed single-page functions.  The
    /// key is semantic (exact code bytes, virtual address, layout, emission
    /// mode), not a benchmark or process identity.  It currently only counts
    /// safe reuse opportunities; execution remains unchanged until an A/B
    /// proves that the opportunity is both real and worthwhile.
    page_templates: std::collections::VecDeque<PageTemplate>,
    /// Physical code page -> entry offsets present in at least one retained
    /// position-independent host module. This is an acceleration index only:
    /// the exact byte/layout/state comparison in `page_template_plan` remains
    /// authoritative before a module can be instantiated or published.
    page_template_cached_offsets: FastHashMap<u64, FastHashSet<u16>>,
}

/// Sampled exit profile of one landed region function (JitState::region_exits).
struct RegionExits {
    /// satp the region was discovered in.
    aspace: u64,
    /// The page whose threshold crossing originally built this region — the
    /// stable identity that keys build cooldowns across rebuilds/extensions.
    lead: u64,
    /// (virtual page, physical page) the function was compiled over,
    /// ascending. Extension reuses these RECORDED pas rather than re-probing:
    /// the build slot fires at an arbitrary guest moment (usually inside the
    /// kernel), where a fetch probe of a user va fails on privilege — the
    /// same trap that once dropped 96% of finished page functions in
    /// sys_sb_ready. Landing validation plus first-dispatch pa-verify carry
    /// the correctness burden, exactly as for every other region install.
    pages: Vec<(u64, u64)>,
    /// Sampled exits to a page OUTSIDE the region since landing.
    total: u32,
    /// Those exits per target page, first-come bounded (EXT_TARGET_CAP).
    targets: Vec<(u64, u32)>,
    /// Every sampled exit (in- or out-of-region), and the instructions those
    /// stays retired — the measured average stay length that picks the
    /// extension's register mode (locals for long stays, memory for short)
    /// and triggers DEMOTION when the function demonstrably doesn't hold.
    samples: u32,
    stay_sum: u64,
    /// Global sampled-exit tick of this region's latest observation. The
    /// extension queue is prioritized by recency so a phase change cannot
    /// spend scarce async build slots on a formerly hot, now dormant region.
    last_tick: u64,
    /// The entry pcs installed at landing (needed to un-claim on demotion).
    entries: Vec<u64>,
}

impl JitState {
    fn new() -> JitState {
        JitState {
            cache: Default::default(),
            hot: Default::default(),
            dispatch: vec![
                DispatchLine {
                    pc: NO_PC,
                    idx: 0,
                    gen: 0,
                };
                DISPATCH_SIZE
            ],
            flush_gen: 0,
            page_entries: Default::default(),
            succ: vec![SuccessorProfile::EMPTY; DISPATCH_SIZE],
            ic_targets: Default::default(),
            interp_hot: vec![0; DISPATCH_SIZE],
            interp_hot_tag: vec![0; DISPATCH_SIZE],
            page_blocks: Default::default(),
            superblocked: Default::default(),
            regions: Default::default(),
            region_exits: Default::default(),
            ext_queue: Vec::new(),
            policy_heat: Default::default(),
            policy_last_sample: Default::default(),
            policy_entries: Default::default(),
            policy_observations: Default::default(),
            policy_control_observations: Default::default(),
            policy_mappings: Default::default(),
            policy_attempted: Default::default(),
            policy_installed: Default::default(),
            policy_queue: Default::default(),
            policy_queued: Default::default(),
            policy_suppressed: Default::default(),
            policy_pending: Default::default(),
            policy_compiled: Default::default(),
            policy_rejected: Default::default(),
            page_templates: Default::default(),
            page_template_cached_offsets: Default::default(),
            sb_gen: Default::default(),
            sb_queue: Vec::new(),
            sb_missed: Default::default(),
        }
    }
    fn clear(&mut self) {
        self.cache.clear();
        self.hot.clear();
        self.page_entries.clear();
        self.superblocked.clear();
        self.regions.clear();
        self.region_exits.clear();
        self.ext_queue.clear();
        self.policy_heat.clear();
        self.policy_last_sample.clear();
        self.policy_entries.clear();
        self.policy_observations.clear();
        self.policy_control_observations.clear();
        self.policy_mappings.clear();
        self.policy_attempted.clear();
        self.policy_installed.clear();
        self.policy_queue.clear();
        self.policy_queued.clear();
        self.policy_suppressed.clear();
        self.policy_pending.clear();
        self.policy_compiled.clear();
        self.policy_rejected.clear();
        self.page_templates.clear();
        self.page_template_cached_offsets.clear();
        self.sb_gen.clear();
        self.sb_queue.clear();
        self.sb_missed.clear();
        for h in self.interp_hot.iter_mut() {
            *h = 0;
        }
        for t in self.interp_hot_tag.iter_mut() {
            *t = 0;
        }
        for e in self.succ.iter_mut() {
            *e = SuccessorProfile::EMPTY;
        }
        self.ic_targets.clear();
        self.page_blocks.clear();
        self.clear_dispatch();
    }
    #[inline]
    fn clear_dispatch(&mut self) {
        for l in self.dispatch.iter_mut() {
            l.pc = NO_PC;
        }
    }
    #[inline]
    fn dslot(pc: u64) -> usize {
        ((pc >> 1) as usize) & (DISPATCH_SIZE - 1)
    }
}

static mut USER_JIT: Option<JitState> = None;
static mut SYS_JIT: Option<JitState> = None;

// Cell every compiled block writes with the number of guest instructions
// it actually retired before returning (sys blocks with inline memory ops
// can bail mid-block, so the count is dynamic — the dispatcher reads this
// rather than assuming full block length).
static mut RETIRED_CELL: u64 = 0;
/// Cold feedback from a failed monomorphic target guard. `OWNER == NO_PC`
/// means the invocation had no miss; generated code writes TARGET first and
/// OWNER last.
static mut IC_MISS_OWNER_CELL: u64 = NO_PC;
static mut IC_MISS_TARGET_CELL: u64 = 0;
/// Instruction fuel granted to the CURRENT dispatch (see JitLayout::fuel_addr):
/// compiled loops/superblocks yield once they retire this many instructions,
/// so caller budgets and the interrupt quantum hold to block granularity.
static mut FUEL_CELL: u64 = 0;
/// Maximum guest instructions one user-mode compiled loop may retire in one
/// Wasm invocation. Re-entry gives engines without Wasm OSR an opportunity to
/// install optimized code after background tier-up. `u64::MAX` disables the
/// cap; policy is benchmarked independently from the compiler.
static mut USER_LOOP_QUANTUM: u64 = u64::MAX;
/// Diagnostics: emitted copy-loop fast paths bump this cell once per bulk
/// chunk (the emitter receives its address via JitLayout.copystat_addr).
static mut COPY_CHUNKS: u64 = 0;

fn retired_addr() -> u32 {
    (&raw const RETIRED_CELL) as u32
}

fn fuel_addr() -> u32 {
    (&raw const FUEL_CELL) as u32
}

fn ic_miss_owner_addr() -> u32 {
    (&raw const IC_MISS_OWNER_CELL) as u32
}

fn ic_miss_target_addr() -> u32 {
    (&raw const IC_MISS_TARGET_CELL) as u32
}

#[no_mangle]
pub extern "C" fn jit_set_user_loop_quantum(guest_insns: u64) {
    unsafe {
        USER_LOOP_QUANTUM = if guest_insns == 0 {
            u64::MAX
        } else {
            guest_insns
        };
    }
}

fn copystat_addr() -> u32 {
    (&raw const COPY_CHUNKS) as u32
}

/// User-mode LR/SC reservation capability imported by generated modules.
///
/// Generated code passes the same opaque state pointer it receives as its
/// first parameter. This helper is enabled only for the flat Linux-user
/// layout, where that pointer is known to address an `rv64_linux::Machine`.
/// Keeping reservation state in the canonical CPU object preserves exact
/// behavior across compiled/interpreted transitions without exposing Rust
/// layout offsets to the DBT emitter.
#[no_mangle]
pub extern "C" fn jit_user_reservation(op: u32, state_ptr: u32, address: u64) -> u32 {
    // SAFETY: user_run passes its live `rv64_linux::Machine` pointer to every
    // generated user block. System/raw layouts never enable this import.
    let machine = unsafe { &mut *(state_ptr as usize as *mut rv64_linux::Machine) };
    match op {
        // LR records the byte address after its memory load has succeeded.
        0 => {
            machine.cpu.reservation = Some(address);
            1
        }
        // Probe only. Generated SC code conditionally validates/performs the
        // store, then calls op 2 to clear the reservation. A precise side exit
        // during store-address validation must leave the reservation intact so
        // the interpreter can re-execute the faulting SC architecturally.
        1 => {
            let success = machine.cpu.reservation == Some(address);
            u32::from(success)
        }
        // Every completed SC attempt clears the reservation, successful or
        // not. The address remains in the ABI to keep one helper signature.
        2 => {
            machine.cpu.reservation = None;
            1
        }
        _ => 0,
    }
}

/// Full-system counterpart of `jit_user_reservation`. Generated modules import
/// a distinct symbol, so the opaque state pointer is cast only to the machine
/// type declared by their compile-time reservation capability.
#[no_mangle]
pub extern "C" fn jit_system_reservation(op: u32, state_ptr: u32, address: u64) -> u32 {
    // SAFETY: the active full-system dispatcher records which concrete board
    // owns `state_ptr` before calling generated code. Both boards keep the
    // canonical reservation in their Cpu.
    let cpu = unsafe {
        match ACTIVE_SYSTEM_KIND {
            kind if kind == SystemMachineKind::Legacy as u8 => {
                &mut (*(state_ptr as usize as *mut rv64_system::Machine)).cpu
            }
            kind if kind == SystemMachineKind::Virt as u8 => {
                &mut (*(state_ptr as usize as *mut rv64_system::virt::VirtMachine)).cpu
            }
            _ => return 0,
        }
    };
    match op {
        0 => {
            cpu.reservation = Some(address);
            1
        }
        1 => {
            let success = cpu.reservation == Some(address);
            u32::from(success)
        }
        2 => {
            cpu.reservation = None;
            1
        }
        _ => 0,
    }
}

/// Exact scalar floating-point helper imported by generated modules. One ABI
/// covers unary, binary, ternary, comparison, classification, and conversion
/// operations so each generated module needs at most one function import. The
/// result is an i64 carrying either raw FP bits or an integer result; accrued
/// flags are ORed into the shared fcsr cell. Calls stay Wasm-to-Wasm and never
/// enter JavaScript code.
#[no_mangle]
pub extern "C" fn jit_fp_exec(
    op: u32,
    lhs: u64,
    rhs: u64,
    third: u64,
    rm: u32,
    fcsr_addr: u32,
) -> u64 {
    use rv64_core::softfp::{self as sfp, sf32, sf64};

    #[inline]
    fn unbox32(value: u64) -> u32 {
        if value >> 32 == 0xffff_ffff {
            value as u32
        } else {
            0x7fc0_0000
        }
    }

    // Wasm scalar FP is exact IEEE round-to-nearest/even, but exposes no
    // exception flags. Once NX is already sticky, it is safe to use native
    // arithmetic when finite operands/result prove that no *other* flag can
    // arise. Add/sub cannot produce an inexact subnormal result; mul/div must
    // produce a normal result or an exact zero forced by a zero operand.
    #[inline]
    fn fast32(op: u32, a: u32, b: u32) -> Option<u32> {
        let ea = (a >> 23) & 0xff;
        let eb = (b >> 23) & 0xff;
        if ea == 0xff || eb == 0xff || (op == 3 && b << 1 == 0) {
            return None;
        }
        let result = match op {
            0 => f32::from_bits(a) + f32::from_bits(b),
            1 => f32::from_bits(a) - f32::from_bits(b),
            2 => f32::from_bits(a) * f32::from_bits(b),
            _ => f32::from_bits(a) / f32::from_bits(b),
        }
        .to_bits();
        let exp = (result >> 23) & 0xff;
        let safe = match op {
            0 | 1 => exp != 0xff,
            2 => (1..=0xfe).contains(&exp) || (result << 1 == 0 && (a << 1 == 0 || b << 1 == 0)),
            _ => (1..=0xfe).contains(&exp) || (result << 1 == 0 && a << 1 == 0),
        };
        safe.then_some(result)
    }

    #[inline]
    fn fast64(op: u32, a: u64, b: u64) -> Option<u64> {
        let ea = (a >> 52) & 0x7ff;
        let eb = (b >> 52) & 0x7ff;
        if ea == 0x7ff || eb == 0x7ff || (op == 3 && b << 1 == 0) {
            return None;
        }
        let result = match op {
            0 => f64::from_bits(a) + f64::from_bits(b),
            1 => f64::from_bits(a) - f64::from_bits(b),
            2 => f64::from_bits(a) * f64::from_bits(b),
            _ => f64::from_bits(a) / f64::from_bits(b),
        }
        .to_bits();
        let exp = (result >> 52) & 0x7ff;
        let safe = match op {
            0 | 1 => exp != 0x7ff,
            2 => (1..=0x7fe).contains(&exp) || (result << 1 == 0 && (a << 1 == 0 || b << 1 == 0)),
            _ => (1..=0x7fe).contains(&exp) || (result << 1 == 0 && a << 1 == 0),
        };
        safe.then_some(result)
    }

    let accrued_flags = unsafe { *(fcsr_addr as *const u32) };
    if rm == sfp::RM_RNE && accrued_flags & sfp::FFLAG_INEXACT != 0 {
        if op < 4 {
            if let Some(result) = fast32(op, unbox32(lhs), unbox32(rhs)) {
                return 0xffff_ffff_0000_0000 | u64::from(result);
            }
        } else if op < 8 {
            if let Some(result) = fast64(op - 4, lhs, rhs) {
                return result;
            }
        }
    }

    let mut flags = 0;
    let result = match op {
        0 => sf32::add(unbox32(lhs), unbox32(rhs), rm, &mut flags) as u64,
        1 => sf32::sub(unbox32(lhs), unbox32(rhs), rm, &mut flags) as u64,
        2 => sf32::mul(unbox32(lhs), unbox32(rhs), rm, &mut flags) as u64,
        3 => sf32::div(unbox32(lhs), unbox32(rhs), rm, &mut flags) as u64,
        4 => sf64::add(lhs, rhs, rm, &mut flags),
        5 => sf64::sub(lhs, rhs, rm, &mut flags),
        6 => sf64::mul(lhs, rhs, rm, &mut flags),
        7 => sf64::div(lhs, rhs, rm, &mut flags),
        8 => sf32::eq_quiet(unbox32(lhs), unbox32(rhs), &mut flags) as u64,
        9 => sf32::lt(unbox32(lhs), unbox32(rhs), &mut flags) as u64,
        10 => sf32::le(unbox32(lhs), unbox32(rhs), &mut flags) as u64,
        11 => sf64::eq_quiet(lhs, rhs, &mut flags) as u64,
        12 => sf64::lt(lhs, rhs, &mut flags) as u64,
        13 => sf64::le(lhs, rhs, &mut flags) as u64,
        14 => sf32::sqrt(unbox32(lhs), rm, &mut flags) as u64,
        15 => sf64::sqrt(lhs, rm, &mut flags),
        16 => sf32::min(unbox32(lhs), unbox32(rhs), &mut flags) as u64,
        17 => sf32::max(unbox32(lhs), unbox32(rhs), &mut flags) as u64,
        18 => sf64::min(lhs, rhs, &mut flags),
        19 => sf64::max(lhs, rhs, &mut flags),
        20 => sfp::cvt_sf64_sf32(lhs, rm, &mut flags) as u64,
        21 => sfp::cvt_sf32_sf64(unbox32(lhs), &mut flags),
        22 => sf32::cvt_to_i32(unbox32(lhs), rm, &mut flags, false) as i32 as i64 as u64,
        23 => sf32::cvt_to_i32(unbox32(lhs), rm, &mut flags, true) as i32 as i64 as u64,
        24 => sf32::cvt_to_i64(unbox32(lhs), rm, &mut flags, false),
        25 => sf32::cvt_to_i64(unbox32(lhs), rm, &mut flags, true),
        26 => sf64::cvt_to_i32(lhs, rm, &mut flags, false) as i32 as i64 as u64,
        27 => sf64::cvt_to_i32(lhs, rm, &mut flags, true) as i32 as i64 as u64,
        28 => sf64::cvt_to_i64(lhs, rm, &mut flags, false),
        29 => sf64::cvt_to_i64(lhs, rm, &mut flags, true),
        30 => sf32::cvt_from_i32(lhs as u32, rm, &mut flags, false) as u64,
        31 => sf32::cvt_from_i32(lhs as u32, rm, &mut flags, true) as u64,
        32 => sf32::cvt_from_i64(lhs, rm, &mut flags, false) as u64,
        33 => sf32::cvt_from_i64(lhs, rm, &mut flags, true) as u64,
        34 => sf64::cvt_from_i32(lhs as u32, rm, &mut flags, false),
        35 => sf64::cvt_from_i32(lhs as u32, rm, &mut flags, true),
        36 => sf64::cvt_from_i64(lhs, rm, &mut flags, false),
        37 => sf64::cvt_from_i64(lhs, rm, &mut flags, true),
        38 => sf32::fclass(unbox32(lhs)) as u64,
        39 => sf64::fclass(lhs) as u64,
        40 => sf32::fma(unbox32(lhs), unbox32(rhs), unbox32(third), rm, &mut flags) as u64,
        41 => sf64::fma(lhs, rhs, third, rm, &mut flags),
        _ => {
            flags |= rv64_core::softfp::FFLAG_INVALID_OP;
            0x7ff8_0000_0000_0000
        }
    };
    unsafe {
        let fcsr = &mut *(fcsr_addr as *mut u32);
        *fcsr |= flags & 0x1f;
    }
    if matches!(op, 0..=3 | 14 | 16 | 17 | 20 | 30..=33 | 40) {
        0xffff_ffff_0000_0000 | result
    } else {
        result
    }
}

/// Wasm function-table entries are unreclaimable (invalidated blocks become
/// unreachable but their slots persist), so unbounded compilation — reboots,
/// address-space churn, self-modifying code — would grow the table forever
/// (PERFORMANCE_PROGRESS.md). Above this bound the JIT stops COMPILING new blocks
/// (existing blocks keep running; the interpreter covers the rest). 1M
/// entries ~= a few hundred MB of compiled code, far beyond any observed
/// workload (fib ~20k, boot ~2k, tcc ~15k) but a hard stop for runaways.
const JIT_TABLE_CAP: u64 = 1_000_000;
static mut JIT_TABLE_ENTRIES: u64 = 0;
/// User-mode compiler lifecycle accounting. Translation excludes JavaScript's
/// WebAssembly validation/compilation, instantiation, and table publication;
/// the host records those phases separately.
static mut USER_TRANSLATE_NS: u64 = 0;
static mut USER_TRANSLATE_ATTEMPTS: u64 = 0;
static mut USER_EMITTED_BYTES: u64 = 0;
/// Full-system translation accounting, including individual blocks, observed
/// batches, and multi-entry regions. Host Wasm frontend phases remain separate
/// in `web/rv64.js`.
static mut SYS_TRANSLATE_NS: u64 = 0;
static mut SYS_TRANSLATE_ATTEMPTS: u64 = 0;
static mut SYS_EMITTED_BYTES: u64 = 0;
/// Members for which the DBT actually emitted the range-checked dense-copy
/// lowering. This is translation-time instrumentation: it proves selection
/// in real guest code without perturbing generated hot paths.
static mut SYS_DENSE_COPY_MEMBERS: u64 = 0;
static mut SYS_DENSE_STORE_MEMBERS: u64 = 0;
static mut SYS_BULK_COPY_MEMBERS: u64 = 0;
/// Bulk-copy helper diagnostics: calls, stale-state rejection, sub-iteration
/// page boundary, source translation rejection, destination translation
/// rejection. They are cold observability only; generated code reads none.
static mut SYS_BULK_COPY_DIAG: [u64; 6] = [0; 6];

fn jit_table_full() -> bool {
    unsafe { JIT_TABLE_ENTRIES >= JIT_TABLE_CAP }
}

/// Longest compiled-code residency between interrupt/device checks, in guest
/// instructions: ~2.5ms at JIT speed. Loops yield at this bound even when the
/// caller's budget is larger (P0 interrupt-latency contract).
const INTERRUPT_QUANTUM: u64 = 1 << 20;

/// A page superblock compiling asynchronously on V8's background threads.
/// Guest execution continues on individual blocks meanwhile; when JS calls
/// sys_sb_ready the entries are repointed — after validating that the boot
/// generation, the va→pa mapping, and the (dirty-tracked) code page are all
/// still the ones the compile was issued against.
struct PendingSb {
    ticket: u64,
    boot_gen: u64,
    /// satp of the address space the region was discovered in.
    aspace: u64,
    /// The page whose threshold crossing owns this region (cooldown identity
    /// across rebuilds and extensions).
    lead: u64,
    /// (virtual page, physical page) of every page in the compiled region,
    /// ascending (sparse regions need not be virtually contiguous).
    pages: Vec<(u64, u64)>,
    entries: Vec<u64>,
    template_key: Option<PageTemplateKey>,
    /// Nonzero only for a position-independent module stored in the host's
    /// per-VM compiled-module cache.
    template_cache_id: u64,
    template_wasm_bytes: u64,
}

#[derive(Clone)]
struct PageTemplateKey {
    vpage: u64,
    fingerprint: u64,
    code: Vec<u8>,
    entries: Vec<u64>,
    layout: rv64_dbt::JitLayout,
    state: rv64_dbt::MultiEntryState,
    emission_config: u64,
}

struct PageTemplate {
    key: PageTemplateKey,
    physical_page: u64,
    template_cache_id: u64,
    wasm_bytes: u64,
    #[allow(dead_code)]
    table_index: i32,
}

struct PageTemplateReuse {
    template_cache_id: u64,
    covered_entries: Vec<u64>,
    template_wasm_bytes: u64,
}

struct PageTemplatePlan {
    key: Option<PageTemplateKey>,
    reuse: Option<PageTemplateReuse>,
    compile_position_independent: bool,
}

impl PageTemplatePlan {
    const NONE: Self = Self {
        key: None,
        reuse: None,
        compile_position_independent: false,
    };
}

const PAGE_TEMPLATE_PROBE_CAP: usize = 256;

/// Virtual pages a superblock region may span. Loops and functions straddle
/// page boundaries constantly; a page-clamped region turns every crossing into
/// a host dispatch (measured: nbench NUMERIC SORT ran its sift loop as six
/// 2-10 instruction blocks, 5.6 insns per dispatch, because the loop sits
/// across a page boundary).
// The hard cap remains three for controlled experiments. Matched CPython and
// SHA measurements select two: a control-entry ratio gate keeps CPython's
// computed-dispatch pages single-page while allowing SHA's adjacent hot page.
const HARD_MAX_REGION_PAGES: usize = 3;
static mut REGION_PAGE_CAP: usize = 2;

#[no_mangle]
pub extern "C" fn jit_set_region_page_cap(pages: u32) {
    unsafe { REGION_PAGE_CAP = (pages as usize).clamp(1, HARD_MAX_REGION_PAGES) }
}

fn region_page_cap() -> usize {
    unsafe { REGION_PAGE_CAP }
}
/// Pages an EXTENDED region may reach (translate_superblock_sparse caps hard
/// at 16). Extension only ever grows along measured exit traffic, so the cap
/// bounds V8 compile cost (~4KB/page of module bytes, 15-40ms per 8-page
/// async build measured), not guesswork about reachability. tcc's hot call
/// graph clusters at ~15 pages — an 8-page cap left its calls crossing
/// region boundaries forever.
const MAX_EXT_REGION_PAGES: usize = 16;
static mut REGION_EXTENSION_PAGE_CAP: usize = MAX_EXT_REGION_PAGES;

#[no_mangle]
pub extern "C" fn jit_set_region_extension_page_cap(pages: u32) {
    unsafe { REGION_EXTENSION_PAGE_CAP = (pages as usize).clamp(1, MAX_EXT_REGION_PAGES) }
}

fn region_extension_page_cap() -> usize {
    unsafe { REGION_EXTENSION_PAGE_CAP }
}
/// Attribute 1 of every 2^N region-function exits (full attribution is a
/// HashMap probe per dispatch — measurable on region-heavy code).
const EXIT_SAMPLE_SHIFT: u32 = 5;
/// Sampled out-of-region exits before a region asks for extension
/// (~EXT_TRIGGER << EXIT_SAMPLE_SHIFT real exits).
const EXT_TRIGGER: u32 = 16;
/// Distinct out-of-region target pages tracked per region.
const EXT_TARGET_CAP: usize = 8;
/// Dispatch-line idx bit marking a region function, so the chain loop can
/// attribute the following exit without a cache probe. Table indices stay
/// far below this bit (JIT_TABLE_CAP = 1M); blacklist (-1) keeps its sign.
/// Shared with the emitter, whose chain transfers mask it off.
const SB_IDX_BIT: i32 = rv64_dbt::SB_IDX_BIT;
/// Measured average stay (retired insns per dispatch of a region function)
/// below which an EXTENDED region is built with registers in MEMORY instead
/// of locals: short stays pay the union load/store at every entry/exit,
/// which for call-shaped code exceeds the work itself. Long-stay regions
/// (FP EMULATION holds ~444 insns) keep locals.
const EXT_MEMORY_MODE_STAY: u64 = 48;
/// Measured average stay below which a landed region function is DEMOTED:
/// its entries return to individual trace blocks and the lead page stops
/// rebuilding. A function whose visits run ~a dozen instructions pays its
/// per-entry cost for nothing — call-shaped code (tcc measured 8-20-insn
/// stays and ran 2x slower under page functions) wants traces, while
/// genuinely holding functions (FP EMULATION ~444-insn stays) never come
/// near this bar. The signal is per-region and measured, so one guest can
/// have both kinds of code and each page gets the winner.
const DEMOTE_STAY: u64 = 24;
/// Runtime toggle for the demotion pass (A/B diagnostics).
static mut DEMOTE_ON: bool = true;
#[no_mangle]
pub extern "C" fn jit_set_demote(on: u32) {
    unsafe { DEMOTE_ON = on != 0 }
}
/// Sampled exits before the demotion verdict is trusted. Zero-retire
/// samples (entry bails: FP gate, first-instruction TLB miss) are excluded
/// from the average — they are refusals, not stays, and a legitimate
/// long-stay FP region bails exactly like that while FS is off. 64 (~2K
/// real exits): the 16-sample verdict condemned regions on WARM-UP stays —
/// NUMERIC SORT's straddling-loop region measured 320-467 iter/s across
/// identical boots (a coin flip against v86's ~400) because whether it
/// survived demotion depended on how cold its first sampled exits were.
const DEMOTE_MIN_SAMPLES: u32 = 64;
static mut SB_DEMOTED: u64 = 0;
/// Legacy wide trace-window support: a 64-page (256KB) aligned VA region
/// around a hot pc, gathered into one contiguous buffer so traces can follow
/// calls across page boundaries. The measured default is now one page, but
/// the wide mode remains available for diagnostics and uses this cache.
const TRACE_WIN_PAGES: u64 = 64;
const TRACE_WIN_MASK: u64 = TRACE_WIN_PAGES * 0x1000 - 1;
struct TraceWin {
    aspace: u64,
    map_gen: u64,
    boot_gen: u64,
    first_va: u64,
    /// (va, physical page) of every RAM-backed mapped page in the window;
    /// unmapped holes stay zero-filled in `buf` (invalid instructions, so
    /// a trace walking into one simply ends there).
    pages: Vec<(u64, u64)>,
    buf: Vec<u8>,
}

/// Ephemeral sparse code snapshot for a rare two-way indirect-cache upgrade.
/// Unlike `TraceWin`, this copies only the source/target pages and is never
/// retained globally; ordinary T1 compilation therefore keeps its measured
/// one-page capture cost.
struct PicCapture {
    page_vas: Vec<u64>,
    pages: Vec<(u64, u64)>,
    buf: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum SystemMachineKind {
    Legacy = 1,
    Virt = 2,
}

static mut ACTIVE_SYSTEM_KIND: u8 = 0;
static mut ACTIVE_SYSTEM_STATE: u32 = 0;

#[derive(Clone, Copy)]
struct PagePolicySample {
    pc: u64,
    pa: u64,
    aspace: u64,
    mode: u8,
    retired: u64,
    control_entry: bool,
}

/// The compiler/runtime contract shared by the legacy TinyEMU-compatible
/// board and the current riscv-virt board. Device models stay in their owning
/// machine; only CPU, RAM, invalidation, and bounded interpreter operations
/// are exposed to the DBT dispatcher.
trait SystemJitMachine {
    fn kind(&self) -> SystemMachineKind;
    fn state_ptr(&mut self) -> u32;
    fn cpu(&self) -> &rv64_core::Cpu;
    fn cpu_mut(&mut self) -> &mut rv64_core::Cpu;
    fn ram(&self) -> &[u8];
    fn probe_fetch(&mut self, va: u64) -> Option<u64>;
    fn dirty_pages_pending(&self) -> bool;
    fn take_dirty_pages(&mut self) -> Vec<u64>;
    fn mark_jit_page(&mut self, pa: u64);
    fn unmark_jit_page(&mut self, page: u64);
    fn jit_page_marked(&self, page: u64) -> bool;
    fn jit_page_dirty(&self, page: u64) -> bool;
    fn run_interpreter(&mut self, max_insns: u64) -> u64;
    fn run_direct_interpreter(&mut self, max_insns: u64) -> u64;
    fn run_interpreter_until(
        &mut self,
        max_insns: u64,
        compiled: &mut dyn FnMut(u64) -> bool,
    ) -> (u64, bool);
    fn run_policy_interpreter(
        &mut self,
        max_insns: u64,
        sample: &mut dyn FnMut(PagePolicySample),
        compiled: &mut dyn FnMut(u64) -> bool,
        precise_stop: bool,
        control_entries: bool,
    ) -> (u64, bool);
    fn sync_and_check_interrupts(&mut self);
    fn power_off(&self) -> bool;
    fn flush_host_io(&mut self);
    fn wallclock_iteration(&mut self);
    fn wallclock_quantum_boundary(&mut self);

    fn activate(&mut self) {
        unsafe {
            ACTIVE_SYSTEM_KIND = self.kind() as u8;
            ACTIVE_SYSTEM_STATE = self.state_ptr();
        }
    }
}

impl SystemJitMachine for rv64_system::Machine {
    fn kind(&self) -> SystemMachineKind {
        SystemMachineKind::Legacy
    }
    fn state_ptr(&mut self) -> u32 {
        self as *mut Self as usize as u32
    }
    fn cpu(&self) -> &rv64_core::Cpu {
        &self.cpu
    }
    fn cpu_mut(&mut self) -> &mut rv64_core::Cpu {
        &mut self.cpu
    }
    fn ram(&self) -> &[u8] {
        &self.bus.ram
    }
    fn probe_fetch(&mut self, va: u64) -> Option<u64> {
        self.cpu.jit_probe_fetch(&mut self.bus, va)
    }
    fn dirty_pages_pending(&self) -> bool {
        !self.bus.jit_dirty_pages.is_empty()
    }
    fn take_dirty_pages(&mut self) -> Vec<u64> {
        self.bus.jit_take_dirty()
    }
    fn mark_jit_page(&mut self, pa: u64) {
        self.bus.jit_mark_page(pa);
    }
    fn unmark_jit_page(&mut self, page: u64) {
        self.bus.jit_unmark_page(page);
    }
    fn jit_page_marked(&self, page: u64) -> bool {
        self.bus.jit_page_marked(page)
    }
    fn jit_page_dirty(&self, page: u64) -> bool {
        self.bus.jit_dirty_pages.contains(&page)
    }
    fn run_interpreter(&mut self, max_insns: u64) -> u64 {
        self.run_slice(max_insns)
    }
    fn run_direct_interpreter(&mut self, max_insns: u64) -> u64 {
        self.run_slice_integrated(max_insns)
    }
    fn run_interpreter_until(
        &mut self,
        max_insns: u64,
        compiled: &mut dyn FnMut(u64) -> bool,
    ) -> (u64, bool) {
        self.run_slice_until(max_insns, compiled)
    }
    fn run_policy_interpreter(
        &mut self,
        max_insns: u64,
        sample: &mut dyn FnMut(PagePolicySample),
        compiled: &mut dyn FnMut(u64) -> bool,
        precise_stop: bool,
        _control_entries: bool,
    ) -> (u64, bool) {
        // The page policy targets the modern virt machine. Keep the legacy
        // board functional behind the same switch with bounded samples; its
        // run_slice owns the exact device/SBI semantics.
        let start = self.cpu.insn_count;
        let mut first = true;
        let mut yielded = false;
        while self.cpu.insn_count - start < max_insns && !self.power_off {
            if !first && compiled(self.cpu.pc) {
                break;
            }
            first = false;
            let pc = self.cpu.pc;
            let aspace = self.cpu.sys.as_ref().map_or(0, |sys| sys.satp);
            let pa = self.probe_fetch(pc);
            let chunk = if precise_stop { 1 } else { 32 };
            let (ran, did_yield) = self.run_slice_until(
                (max_insns - (self.cpu.insn_count - start)).min(chunk),
                &mut |_| false,
            );
            if let Some(pa) = pa {
                if ran != 0 {
                    sample(PagePolicySample {
                        pc,
                        pa,
                        aspace,
                        mode: self.cpu.sys.as_ref().map_or(u8::MAX, |sys| sys.mode as u8),
                        retired: ran,
                        control_entry: false,
                    });
                }
            }
            if ran == 0 {
                break;
            }
            if did_yield {
                yielded = true;
                break;
            }
        }
        (self.cpu.insn_count - start, yielded)
    }
    fn sync_and_check_interrupts(&mut self) {
        let origin = self.cpu.sys.as_ref().map_or(0, |sys| sys.mode as usize);
        let timer_before = self.cpu.irq_counts[5];
        self.sync_devices();
        self.cpu.check_interrupts(&mut self.bus);
        let delivered = self.cpu.irq_counts[5].wrapping_sub(timer_before);
        if delivered != 0 {
            unsafe { JIT_TIMER_IRQ_ORIGIN[origin] += delivered }
        }
    }
    fn power_off(&self) -> bool {
        self.power_off
    }
    fn flush_host_io(&mut self) {
        let out = self.console_output();
        if !out.is_empty() {
            unsafe { host_write(1, out.as_ptr(), out.len()) }
        }
        pump_net(self);
    }
    fn wallclock_iteration(&mut self) {
        if !unsafe { SYS_WALLCLOCK } {
            return;
        }
        let ic = self.cpu.insn_count;
        let due = unsafe { ic.wrapping_sub(WALL_LAST_ICOUNT) >= 16384 || WALL_IDLE_ITERS >= 64 };
        if due {
            unsafe {
                WALL_LAST_ICOUNT = ic;
                WALL_IDLE_ITERS = 0;
            }
            self.wall_ns = Some(unsafe { host_now_ms() } as u64 * 1_000_000);
            self.wall_anchor_icount = ic;
        } else {
            unsafe { WALL_IDLE_ITERS += 1 };
        }
    }
    fn wallclock_quantum_boundary(&mut self) {
        if unsafe { SYS_WALLCLOCK } {
            self.wall_ns = Some(unsafe { host_now_ms() } as u64 * 1_000_000);
            self.wall_anchor_icount = self.cpu.insn_count;
            unsafe { WALL_LAST_ICOUNT = self.cpu.insn_count };
        }
    }
}

impl SystemJitMachine for rv64_system::virt::VirtMachine {
    fn kind(&self) -> SystemMachineKind {
        SystemMachineKind::Virt
    }
    fn state_ptr(&mut self) -> u32 {
        self as *mut Self as usize as u32
    }
    fn cpu(&self) -> &rv64_core::Cpu {
        &self.cpu
    }
    fn cpu_mut(&mut self) -> &mut rv64_core::Cpu {
        &mut self.cpu
    }
    fn ram(&self) -> &[u8] {
        &self.bus.ram
    }
    fn probe_fetch(&mut self, va: u64) -> Option<u64> {
        self.cpu.jit_probe_fetch(&mut self.bus, va)
    }
    fn dirty_pages_pending(&self) -> bool {
        !self.bus.jit_dirty_pages.is_empty()
    }
    fn take_dirty_pages(&mut self) -> Vec<u64> {
        self.bus.jit_take_dirty()
    }
    fn mark_jit_page(&mut self, pa: u64) {
        self.bus.jit_mark_page(pa);
    }
    fn unmark_jit_page(&mut self, page: u64) {
        self.bus.jit_unmark_page(page);
    }
    fn jit_page_marked(&self, page: u64) -> bool {
        self.bus.jit_page_marked(page)
    }
    fn jit_page_dirty(&self, page: u64) -> bool {
        self.bus.jit_dirty_pages.contains(&page)
    }
    fn run_interpreter(&mut self, max_insns: u64) -> u64 {
        self.run_slice(max_insns)
    }
    fn run_direct_interpreter(&mut self, max_insns: u64) -> u64 {
        self.run_slice_integrated(max_insns)
    }
    fn run_interpreter_until(
        &mut self,
        max_insns: u64,
        compiled: &mut dyn FnMut(u64) -> bool,
    ) -> (u64, bool) {
        self.run_slice_until(max_insns, compiled)
    }
    fn run_policy_interpreter(
        &mut self,
        max_insns: u64,
        sample: &mut dyn FnMut(PagePolicySample),
        compiled: &mut dyn FnMut(u64) -> bool,
        precise_stop: bool,
        control_entries: bool,
    ) -> (u64, bool) {
        let control_entries = control_entries
            && (unsafe { PAGE_POLICY_PRIVILEGED_CONTROL_ENTRIES }
                || self
                    .cpu
                    .sys
                    .as_ref()
                    .is_some_and(|sys| sys.mode == rv64_core::csr::Mode::User));
        let mut observe = |pc, pa, satp, mode, retired, control_entry| {
            sample(PagePolicySample {
                pc,
                pa,
                aspace: satp,
                mode,
                retired,
                control_entry,
            });
        };
        if precise_stop || control_entries {
            self.run_slice_sampled_until(
                max_insns,
                unsafe { PAGE_POLICY_QUANTUM },
                &mut observe,
                compiled,
                control_entries,
            )
        } else {
            self.run_slice_sampled(
                max_insns,
                unsafe { PAGE_POLICY_QUANTUM },
                &mut observe,
                compiled,
            )
        }
    }
    fn sync_and_check_interrupts(&mut self) {
        let origin = self.cpu.sys.as_ref().map_or(0, |sys| sys.mode as usize);
        let timer_before = self.cpu.irq_counts[5];
        self.sync_devices();
        self.cpu.check_interrupts(&mut self.bus);
        let delivered = self.cpu.irq_counts[5].wrapping_sub(timer_before);
        if delivered != 0 {
            unsafe { JIT_TIMER_IRQ_ORIGIN[origin] += delivered }
        }
    }
    fn power_off(&self) -> bool {
        self.power_off
    }
    fn flush_host_io(&mut self) {
        let out = self.console_output();
        if !out.is_empty() {
            unsafe { host_write(1, out.as_ptr(), out.len()) }
        }
        let export = self.virtio_console_take_output();
        if !export.is_empty() {
            unsafe { host_write(3, export.as_ptr(), export.len()) }
        }
        pump_virt_net(self);
    }
    fn wallclock_iteration(&mut self) {}
    fn wallclock_quantum_boundary(&mut self) {
        advance_virt_realtime(self);
    }
}

fn capture_pic_pages(
    m: &mut impl SystemJitMachine,
    source: u64,
    targets: &[u64],
) -> Option<PicCapture> {
    let source_page = source & !0xfff;
    let mut wanted = Vec::with_capacity(3);
    wanted.push(source_page);
    for &target in targets.iter().take(2) {
        wanted.push(target & !0xfff);
    }
    wanted.sort_unstable();
    wanted.dedup();

    let mut page_vas = Vec::with_capacity(wanted.len());
    let mut pages = Vec::with_capacity(wanted.len());
    let mut buf = Vec::with_capacity(wanted.len() * 4096);
    for va in wanted {
        let Some(translated) = m.probe_fetch(va) else {
            continue;
        };
        let pa = translated & !0xfff;
        if pa < rv64_system::RAM_BASE {
            continue;
        }
        let offset = usize::try_from(pa - rv64_system::RAM_BASE).ok()?;
        let Some(end) = offset.checked_add(4096) else {
            continue;
        };
        if end > m.ram().len() {
            continue;
        }
        page_vas.push(va);
        pages.push((va, pa));
        buf.extend_from_slice(&m.ram()[offset..end]);
    }
    page_vas.contains(&source_page).then_some(PicCapture {
        page_vas,
        pages,
        buf,
    })
}
/// Small LRU of gathered windows: one entry thrashed when a workload's hot
/// code alternates across several 256KB regions (CPython spans many — each
/// miss re-copies 256KB, which dwarfed the compile itself).
const TRACE_WIN_CACHE: usize = 8;
static mut TRACE_WIN: Vec<TraceWin> = Vec::new();
/// Translation-window control: 1 (the measured default) uses one page, 2
/// forces the legacy 64-page window, and 0 follows TRACE_LEVEL (level 0 is one
/// page, higher levels are wide). This keeps the old combinations available
/// for diagnostics while making the reproducibly faster narrow window the
/// normal policy.
static mut TRACE_WINDOW_MODE: u32 = 1;
#[no_mangle]
pub extern "C" fn jit_set_trace_window(mode: u32) {
    // Benchmark/runtime knobs are configured before boot, while TRACE_WIN is
    // still empty (the same pre-boot contract as the other JIT setters).
    unsafe { TRACE_WINDOW_MODE = mode.min(2) }
}
/// A landed region function does NOT claim a pc whose individual block is a
/// trace at least this long. 0 = functions claim everything: mixed claiming
/// fragments execution into function/trace ping-pong. Measured medians favor
/// claim-all (compile 3.3s vs 3.6-4.0s, HUFFMAN ~1010 vs ~920), but note the
/// boot-to-boot coverage races swing NUMERIC/HUFFMAN/ASSIGNMENT +/-20% in
/// EVERY configuration — single draws cannot distinguish 0 from 24 (both
/// were sampled at NUMERIC ~320 and ~445 on identical binaries). Runtime-
/// settable for A/B.
static mut TRACE_KEEP_MIN: u32 = 0;
#[no_mangle]
pub extern "C" fn jit_set_trace_keep_min(v: u32) {
    unsafe { TRACE_KEEP_MIN = v }
}
/// After a drain visit finds no matching aspace, don't rescan until this
/// many instructions pass (the fall-through drain otherwise scans the
/// queue on every chain break — measured 1.1M scans on one tcc run).
static mut SB_EXT_NEXT_ICOUNT: u64 = 0;
static mut EXIT_TICK: u64 = 0;
static mut SB_EXT_ISSUED: u64 = 0;
static mut SB_EXIT_SAMPLED: u64 = 0;
/// Diagnostic split of SB_EXIT_SAMPLED (jit_stat 35-38).
static mut SB_EXIT_NOMAP: u64 = 0;
static mut SB_EXIT_INREGION: u64 = 0;
static mut SB_EXT_DEFER_COOL: u64 = 0;
static mut SB_EXT_NO_TARGET: u64 = 0;
static mut SB_EXT_PUSHED: u64 = 0;
static mut SB_EXT_DRAIN_VISITS: u64 = 0;
static mut SB_EXT_DRAIN_NOMATCH: u64 = 0;
static mut PENDING_SB: Vec<PendingSb> = Vec::new();
static mut NEXT_SB_TICKET: u64 = 1;
// Superblock lifecycle counters (diagnostic, jit_stat 10..14).
static mut SB_TRIGGER: u64 = 0;
static mut SB_XLATE_FAIL: u64 = 0;
static mut SB_ISSUED: u64 = 0;
static mut SB_LANDED: u64 = 0;
static mut SB_STALE: u64 = 0;
/// Dispatches that entered a compiled block and retired NOTHING (entry bail:
/// FP gate, first-instruction TLB miss, or a br_table slot the function
/// doesn't own). Each one costs a call plus a single interpreted instruction.
static mut ZERO_RETIRE: u64 = 0;
/// Individual blocks compiled for a pc on a page that is already superblocked
/// (i.e. code the page's superblock does not cover), and superblock compiles
/// still awaiting their async module.
static mut SB_INDIV: u64 = 0;
/// Why an entry retired nothing (sampled under DPROF_ON): the FP gate's three
/// conditions, checked host-side at the moment of the bail.
static mut ZR_NX: u64 = 0;
static mut ZR_FRM: u64 = 0;
static mut ZR_FS: u64 = 0;
/// Compiled entries evicted at dispatch because their code page no longer maps
/// where it did (split: the entry's own page vs another page of its region).
static mut DROP_SELF: u64 = 0;
static mut DROP_REGION: u64 = 0;
/// Dirty-code-page events and the compiled entries they dropped.
static mut DIRTY_EVENTS: u64 = 0;
static mut DIRTY_DROPPED: u64 = 0;
/// Entries installed by landed superblocks, and how many of those installs
/// replaced an individual block (i.e. code that had already fallen back).
static mut SB_ENTRIES_IN: u64 = 0;
static mut SB_REPLACED: u64 = 0;
/// Trace one pc through the compile pipeline (diagnostic).
static mut TRACE_PC: u64 = 0;
static mut TRACE_SB_INSTALL: u64 = 0;
static mut TRACE_INDIV: u64 = 0;
static mut TRACE_SEED: u64 = 0;
static mut TRACE_ENTRY: u64 = 0;
/// Bumped by sys_boot: async results from a previous machine must be dropped.
static mut BOOT_GEN: u64 = 0;

// Perf instrumentation: guest instructions retired inside JIT blocks vs
// total, and dispatch counts (block calls). Exposed via jit_stat().
static mut JIT_RETIRED: u64 = 0;
static mut SLICE_CALLS: u64 = 0;
static mut SLICE_INSNS: u64 = 0;
static mut JIT_DISPATCHES: u64 = 0;
// Timer interrupts delivered explicitly at generated-code boundaries, split
// by interrupted privilege mode (U=0, S=1, M=3). Interpreter-internal polls
// are intentionally excluded so this diagnoses the JIT scheduler contract.
static mut JIT_TIMER_IRQ_ORIGIN: [u64; 4] = [0; 4];
/// Diagnostic mode bits: 1=memory paths, 2=register boundaries, 4=size only.
static mut MEMPROF_MODE: u32 = 0;
static mut MULTI_LATCH: bool = true;
static mut MEMPROF: [u64; 89] = [0; 89];

#[no_mangle]
pub extern "C" fn memprof_set(on: u32) {
    unsafe {
        MEMPROF_MODE = on;
        MEMPROF = [0; 89];
    }
}

#[no_mangle]
pub extern "C" fn memprof_get(index: u32) -> u64 {
    unsafe { MEMPROF[index as usize % 89] }
}

#[no_mangle]
pub extern "C" fn jit_set_multi_latch(on: u32) {
    unsafe { MULTI_LATCH = on != 0 }
}

#[allow(static_mut_refs)]
fn mem_profile_layout() -> Option<[u32; 17]> {
    unsafe {
        if MEMPROF_MODE & 3 == 0 {
            return None;
        }
        let base = MEMPROF.as_ptr() as u32;
        let mem = MEMPROF_MODE & 1 != 0;
        let regs = MEMPROF_MODE & 2 != 0;
        Some([
            if mem { base } else { 0 },
            if mem { base + 8 } else { 0 },
            if mem { base + 16 } else { 0 },
            if mem { base + 24 } else { 0 },
            if mem { base + 32 } else { 0 },
            if regs { base + 40 } else { 0 },
            if regs { base + 48 } else { 0 },
            if regs { base + 56 } else { 0 },
            if regs { base + 64 } else { 0 },
            if regs { base + 152 } else { 0 },
            if regs { base + 160 } else { 0 },
            if regs { base + 168 } else { 0 },
            if regs { base + 176 } else { 0 },
            if regs { base + 184 } else { 0 },
            if regs { base + 192 } else { 0 },
            if regs { base + 200 } else { 0 },
            if regs { base + 208 } else { 0 },
        ])
    }
}

fn reg_stress() -> bool {
    unsafe { MEMPROF_MODE & 8 != 0 }
}

#[allow(static_mut_refs)]
fn reg_profile_base() -> u32 {
    unsafe {
        if MEMPROF_MODE & 2 != 0 {
            MEMPROF.as_ptr().add(27) as u32
        } else {
            0
        }
    }
}

// Dispatch-site profiler (diagnostic, off by default): direct-mapped
// (pc -> dispatches, retired) so a run can be attributed per guest pc —
// the metric that tells small-block/dispatch-bound kernels apart from
// genuinely slow code. One predictable branch per dispatch when off.
const DPROF_N: usize = 8192;
static mut DPROF_PC: [u64; DPROF_N] = [0; DPROF_N];
static mut DPROF_CNT: [u64; DPROF_N] = [0; DPROF_N];
static mut DPROF_RET: [u64; DPROF_N] = [0; DPROF_N];
static mut EPROF_SRC: [u64; DPROF_N] = [0; DPROF_N];
static mut EPROF_DST: [u64; DPROF_N] = [0; DPROF_N];
static mut EPROF_CNT: [u64; DPROF_N] = [0; DPROF_N];
static mut EPROF_RET: [u64; DPROF_N] = [0; DPROF_N];
static mut DPROF_ON: bool = false;
/// Profile one of every 2^N dispatches. Full attribution materially perturbs
/// dispatch-heavy nbench kernels, so diagnostics default to sampling in the
/// JS worker while shift=0 preserves the original exact profiler.
static mut DPROF_SAMPLE_SHIFT: u32 = 0;
static mut DPROF_TICK: u64 = 0;
static mut DPROF_BLOCK_CALLS: u64 = 0;
static mut DPROF_BLOCK_INSNS: u64 = 0;
static mut DPROF_REGION_CALLS: u64 = 0;
static mut DPROF_REGION_INSNS: u64 = 0;
static mut DPROF_STRUCTURED_MEMBERS: u64 = 0;
static mut DPROF_STRUCTURED_INSNS: u64 = 0;
static mut DPROF_STRUCTURED_X2_WRITE_MEMBERS: u64 = 0;
static mut DPROF_STRUCTURED_STACK_MEMORY: u64 = 0;
static mut DPROF_TRACE_MIX: [u64; 5] = [0; 5];
static mut DPROF_TRACE_MEM: [u64; 10] = [0; 10];
static mut DPROF_TRACE_CONTROL: [u64; 3] = [0; 3];
static mut DPROF_TRACE_ALU: [u64; 5] = [0; 5];

#[allow(static_mut_refs)]
fn structured_profile_layout() -> Option<[u32; 9]> {
    unsafe {
        if !DPROF_ON {
            return None;
        }
        Some([
            (&raw const DPROF_STRUCTURED_MEMBERS) as u32,
            (&raw const DPROF_STRUCTURED_INSNS) as u32,
            DPROF_TRACE_MIX.as_ptr().add(0) as u32,
            DPROF_TRACE_MIX.as_ptr().add(1) as u32,
            DPROF_TRACE_MIX.as_ptr().add(2) as u32,
            DPROF_TRACE_MIX.as_ptr().add(3) as u32,
            DPROF_TRACE_MIX.as_ptr().add(4) as u32,
            (&raw const DPROF_STRUCTURED_X2_WRITE_MEMBERS) as u32,
            (&raw const DPROF_STRUCTURED_STACK_MEMORY) as u32,
        ])
    }
}

#[no_mangle]
pub extern "C" fn dprof_set(on: u32) {
    unsafe {
        DPROF_ON = on != 0;
        if on != 0 {
            DPROF_TICK = 0;
            DPROF_PC = [0; DPROF_N];
            DPROF_CNT = [0; DPROF_N];
            DPROF_RET = [0; DPROF_N];
            EPROF_SRC = [0; DPROF_N];
            EPROF_DST = [0; DPROF_N];
            EPROF_CNT = [0; DPROF_N];
            EPROF_RET = [0; DPROF_N];
            DPROF_BLOCK_CALLS = 0;
            DPROF_BLOCK_INSNS = 0;
            DPROF_REGION_CALLS = 0;
            DPROF_REGION_INSNS = 0;
            DPROF_STRUCTURED_MEMBERS = 0;
            DPROF_STRUCTURED_INSNS = 0;
            DPROF_STRUCTURED_X2_WRITE_MEMBERS = 0;
            DPROF_STRUCTURED_STACK_MEMORY = 0;
            DPROF_TRACE_MIX = [0; 5];
            DPROF_TRACE_MEM = [0; 10];
            DPROF_TRACE_CONTROL = [0; 3];
            DPROF_TRACE_ALU = [0; 5];
            IHIST_KEY = [0; IHIST_N];
            IHIST_CNT = [0; IHIST_N];
            IHIST_INSNS = [0; IHIST_N];
            IHIST_LAST = usize::MAX;
        }
    }
}

/// which: 0 = source pc, 1 = target pc, 2 = transitions, 3 = retired insns.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn eprof_get(which: u32, i: u32) -> u64 {
    let i = i as usize % DPROF_N;
    unsafe {
        match which {
            0 => EPROF_SRC[i],
            1 => EPROF_DST[i],
            2 => EPROF_CNT[i],
            _ => EPROF_RET[i],
        }
    }
}

#[no_mangle]
pub extern "C" fn dprof_set_sample_shift(shift: u32) {
    unsafe { DPROF_SAMPLE_SHIFT = shift.min(20) }
}

/// which: 0 = pc, 1 = dispatches, 2 = retired insns.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn dprof_get(which: u32, i: u32) -> u64 {
    let i = i as usize % DPROF_N;
    unsafe {
        match which {
            0 => DPROF_PC[i],
            1 => DPROF_CNT[i],
            _ => DPROF_RET[i],
        }
    }
}

/// Histogram of the INSTRUCTION the JIT gave up on (diagnostic, DPROF_ON):
/// keyed by the encoding fields that select an emitter path. Interpreted
/// instructions cost ~300x a compiled one, so a handful of missing encodings
/// can dominate a kernel's wall time.
const IHIST_N: usize = 1024;
static mut IHIST_KEY: [u32; IHIST_N] = [0; IHIST_N];
static mut IHIST_CNT: [u64; IHIST_N] = [0; IHIST_N];

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn ihist_get(which: u32, i: u32) -> u64 {
    let i = i as usize % IHIST_N;
    unsafe {
        match which {
            0 => IHIST_KEY[i] as u64,
            1 => IHIST_CNT[i],
            _ => IHIST_INSNS[i],
        }
    }
}

/// Interpreted instructions charged to the fallback that started the stretch.
static mut IHIST_INSNS: [u64; IHIST_N] = [0; IHIST_N];
static mut IHIST_LAST: usize = usize::MAX;

#[allow(static_mut_refs)]
fn ihist_hit(insn: u32) {
    // opcode + funct3 + funct7 (and the rs2 field, which selects FCVT variants)
    let key = if insn & 3 != 3 {
        insn & 0xffff // compressed: whole halfword
    } else {
        insn & 0xfff0_707f
    };
    unsafe {
        let h = ((key ^ (key >> 13)).wrapping_mul(0x9e37_79b9) >> 18) as usize & (IHIST_N - 1);
        if IHIST_KEY[h] != key {
            if IHIST_CNT[h] != 0 {
                return;
            }
            IHIST_KEY[h] = key;
        }
        IHIST_CNT[h] += 1;
        IHIST_LAST = h;
    }
}

#[inline(always)]
#[allow(static_mut_refs)]
fn dprof_hit(pc: u64, retired: u64) {
    unsafe {
        let h = ((pc >> 1) ^ (pc >> 13)) as usize & (DPROF_N - 1);
        if DPROF_PC[h] != pc {
            if DPROF_CNT[h] != 0 {
                return; // collision: first hot pc keeps the slot
            }
            DPROF_PC[h] = pc;
        }
        DPROF_CNT[h] += 1;
        DPROF_RET[h] += retired;
    }
}

#[inline(always)]
#[allow(static_mut_refs)]
fn eprof_hit(src: u64, dst: u64, retired: u64) {
    unsafe {
        let h = ((src >> 1) ^ (src >> 13) ^ (dst >> 3) ^ (dst >> 17)) as usize & (DPROF_N - 1);
        if EPROF_SRC[h] != src || EPROF_DST[h] != dst {
            if EPROF_CNT[h] != 0 {
                return; // collision: first hot edge keeps the slot
            }
            EPROF_SRC[h] = src;
            EPROF_DST[h] = dst;
        }
        EPROF_CNT[h] += 1;
        EPROF_RET[h] += retired;
    }
}

/// jit_stat(0) = insns retired in JIT blocks, (1) = block dispatches,
/// (2) = compiled blocks (user), (3) = compiled blocks (sys).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_stat(which: u32) -> u64 {
    unsafe {
        match which {
            0 => JIT_RETIRED,
            1 => JIT_DISPATCHES,
            2 => USER_JIT.as_ref().map_or(0, |j| j.cache.len() as u64),
            3 => SYS_JIT.as_ref().map_or(0, |j| j.cache.len() as u64),
            4 => SLICE_CALLS,
            5 => SLICE_INSNS,
            8 => COPY_CHUNKS,
            10 => SB_TRIGGER,
            11 => SB_XLATE_FAIL,
            12 => SB_ISSUED,
            13 => SB_LANDED,
            14 => SB_STALE,
            15 => ZERO_RETIRE,
            16 => SB_INDIV,
            17 => PENDING_SB.len() as u64,
            18 => ZR_NX,
            19 => ZR_FRM,
            20 => ZR_FS,
            21 => DROP_SELF,
            22 => DROP_REGION,
            31 => TLB_FILLS,
            23 => DIRTY_EVENTS,
            24 => DIRTY_DROPPED,
            25 => SB_ENTRIES_IN,
            26 => SB_REPLACED,
            27 => TRACE_SB_INSTALL,
            28 => TRACE_INDIV,
            29 => TRACE_SEED,
            30 => TRACE_ENTRY,
            32 => SB_EXT_ISSUED,
            33 => SB_EXIT_SAMPLED,
            34 => SB_BUILD_MS as u64,
            35 => SB_EXIT_NOMAP,
            36 => SB_EXIT_INREGION,
            37 => SB_EXT_DEFER_COOL,
            38 => SB_EXT_NO_TARGET,
            39 => SB_EXT_PUSHED,
            40 => SB_EXT_DRAIN_VISITS,
            41 => SB_EXT_DRAIN_NOMATCH,
            42 => SB_DEMOTED,
            43 => BATCHES,
            44 => BATCH_MEMBERS,
            45 => IC_EXTENDS,
            46 => DPROF_BLOCK_CALLS,
            47 => DPROF_BLOCK_INSNS,
            48 => DPROF_REGION_CALLS,
            49 => DPROF_REGION_INSNS,
            50..=54 => DPROF_TRACE_MIX[(which - 50) as usize],
            55..=64 => DPROF_TRACE_MEM[(which - 55) as usize],
            65..=67 => DPROF_TRACE_CONTROL[(which - 65) as usize],
            68..=72 => DPROF_TRACE_ALU[(which - 68) as usize],
            73 => USER_TRANSLATE_NS,
            74 => USER_TRANSLATE_ATTEMPTS,
            75 => USER_EMITTED_BYTES,
            76 => SYS_TRANSLATE_NS,
            77 => SYS_TRANSLATE_ATTEMPTS,
            78 => SYS_EMITTED_BYTES,
            79 => IC_PIC_EXTENDS,
            80 => CHAIN_HOPS,
            81..=88 => TLB_FILL_KIND[(which - 81) as usize],
            // MMU/JIT-TLB invalidation attribution. map_gen counts changed
            // SATP plus SFENCE.VMA; jit_flush_gen counts changed SATP only.
            // The remaining counters expose physical row clears directly.
            89 => system_cpu_stat(0),
            90 => system_cpu_stat(1),
            91 => system_cpu_stat(2),
            92 => system_cpu_stat(3),
            93 => system_cpu_stat(4),
            94 => system_cpu_stat(5),
            95 => system_cpu_stat(6),
            96 => JIT_TIMER_IRQ_ORIGIN[0],
            97 => JIT_TIMER_IRQ_ORIGIN[1],
            98 => JIT_TIMER_IRQ_ORIGIN[3],
            99 => SYS_DENSE_COPY_MEMBERS,
            100 => SYS_DENSE_STORE_MEMBERS,
            101 => SYS_BULK_COPY_MEMBERS,
            102..=107 => SYS_BULK_COPY_DIAG[(which - 102) as usize],
            108 => PAGE_TEMPLATE_PROBE as u64,
            109 => PAGE_TEMPLATE_ELIGIBLE,
            110 => PAGE_TEMPLATE_CODE_MATCH,
            111 => PAGE_TEMPLATE_REUSABLE,
            112 => PAGE_TEMPLATE_CROSS_PHYSICAL,
            113 => SYS_JIT
                .as_ref()
                .map_or(0, |jit| jit.page_templates.len() as u64),
            114 => PAGE_TEMPLATE_MATCH_REQUESTED_ENTRIES,
            115 => PAGE_TEMPLATE_MATCH_COVERED_ENTRIES,
            116 => PAGE_TEMPLATE_MATCH_MISSING_ENTRIES,
            117 => PAGE_TEMPLATE_UNION_COVERED_ENTRIES,
            118 => PAGE_TEMPLATE_UNION_MISSING_ENTRIES,
            119 => PAGE_TEMPLATE_RELOCATED_MATCH,
            120 => PAGE_TEMPLATE_RELOCATED_REQUESTED_ENTRIES,
            121 => PAGE_TEMPLATE_RELOCATED_COVERED_ENTRIES,
            122 => PAGE_TEMPLATE_RELOCATED_MISSING_ENTRIES,
            123 => PAGE_TEMPLATE_REUSE as u64,
            124 => PAGE_TEMPLATE_PIC_COMPILES,
            125 => PAGE_TEMPLATE_REUSE_HITS,
            126 => PAGE_TEMPLATE_REUSE_COVERED_ENTRIES,
            127 => PAGE_TEMPLATE_REUSE_MISSING_ENTRIES,
            128 => PAGE_TEMPLATE_REUSE_SAVED_WASM_BYTES,
            129 => PAGE_TEMPLATE_EAGER_PHYSICAL_CANDIDATES,
            130 => DPROF_STRUCTURED_MEMBERS,
            131 => DPROF_STRUCTURED_INSNS,
            132 => DPROF_STRUCTURED_X2_WRITE_MEMBERS,
            133 => DPROF_STRUCTURED_STACK_MEMORY,
            134 => system_cpu_stat(7),
            135 => system_cpu_stat(8),
            136 => system_cpu_stat(9),
            137 => system_cpu_stat(10),
            138 => system_cpu_stat(11),
            139 => system_cpu_stat(12),
            140 => system_cpu_stat(13),
            141 => system_cpu_stat(14),
            142 => system_cpu_stat(15),
            143 => system_cpu_stat(16),
            144 => system_cpu_stat(17),
            145 => system_cpu_stat(18),
            146 => system_cpu_stat(19),
            147 => system_cpu_stat(20),
            148 => system_cpu_stat(21),
            149 => system_cpu_stat(22),
            150 => system_cpu_stat(23),
            151 => system_cpu_stat(24),
            _ => 0,
        }
    }
}

#[allow(static_mut_refs)]
unsafe fn system_cpu_stat(which: u32) -> u64 {
    let cpu = if let Some(machine) = VIRT.as_ref() {
        Some(&machine.cpu)
    } else {
        SYS.as_ref().map(|machine| &machine.cpu)
    };
    cpu.map_or(0, |cpu| match which {
        0 => cpu.map_gen,
        1 => cpu.jit_flush_gen,
        2 => cpu.tlb_flushes,
        3 => cpu.store_jtlb_clears,
        4 => cpu.sfence_all,
        5 => cpu.sfence_page,
        6 => cpu.sfence_foreign_asid,
        // Reserved former direct-interpreter recognizer counters. The
        // benchmark-derived recognizers were removed; keep the public stat
        // numbers stable for older diagnostic clients.
        7..=24 => 0,
        _ => 0,
    })
}

// JIT tier-up policy. Settable at runtime (jit_set_enabled) so benchmarks can
// compare against the pure wasm interpreter. JIT_ENABLED is authoritative:
// disabled execution bypasses the dispatcher entirely. The threshold remains
// a defensive compile gate for code already inside a JIT runner.
/// Compile a block after it is dispatched this many times. High enough
/// that one-shot boot code stays interpreted; low enough that compute
/// loops (dispatched millions of times) tier up quickly.
const JIT_ON_THRESHOLD: u32 = 64;
static mut JIT_ENABLED: bool = true;
static mut JIT_THRESHOLD: u32 = 64;
static mut JIT_SUPERVISOR_ENABLED: bool = true;

/// Diagnostic policy switch: retain generated user execution while routing
/// supervisor mode through the exact interpreter scheduler. This is used to
/// isolate kernel-only translation faults without conflating them with user
/// code generation or Linux timer delivery.
#[no_mangle]
pub extern "C" fn jit_set_supervisor_enabled(on: u32) {
    unsafe { JIT_SUPERVISOR_ENABLED = on != 0 }
}

// Async page policy selected by trace simulation plus process-isolated Node
// and Chromium A/B gates. The low-level debug core leaves the selector
// explicit for differential tests; the stable web loader enables it for
// ordinary system emulation.
const PAGE_POLICY_QUEUE_CAP: usize = 64;
const PAGE_POLICY_STALE_INSNS: u64 = 2_097_152;
static mut PAGE_POLICY_ENABLED: bool = false;
static mut PAGE_POLICY_THRESHOLD: u64 = 131_072;
// Privileged code encountered during kernel/firmware work is generally much
// more transient than a user process's hot loop. Require 32x as much S/M-mode
// heat while retaining the ordinary threshold for user code. A 8/16/32/64
// fresh-process sweep found a broad boot-time plateau, and 32 passed the full
// unrelated-row guard; keeping the setter explicit preserves controlled A/Bs.
static mut PAGE_POLICY_PRIVILEGED_THRESHOLD_MULTIPLIER: u64 = 32;
static mut PAGE_POLICY_QUANTUM: u64 = 1_024;
static mut PAGE_POLICY_REBUILD: bool = false;
static mut PAGE_POLICY_CONTROL_ENTRIES: bool = true;
// Exact non-sequential-target observation is useful for branchy user code but
// pure overhead across transient supervisor/firmware stretches. Keep it off
// for privileged starts; user control entries remain governed independently
// by PAGE_POLICY_CONTROL_ENTRIES.
static mut PAGE_POLICY_PRIVILEGED_CONTROL_ENTRIES: bool = false;
// Observe non-sequential interpreter targets for region classification without
// necessarily making every target a compilation entry. Keeping observation
// separate from seeding lets the classifier measure the historical sampler
// without changing the regions it is trying to classify.
static mut PAGE_POLICY_CONTROL_PROFILE: bool = false;
static mut PAGE_POLICY_INFLIGHT_LIMIT: usize = 2;
// Whether direct calls may pull a callee on another page into the caller's
// page-policy region. Kept explicit until browser A/B establishes whether
// call graph closure or only page-straddling CFG closure is profitable.
static mut PAGE_POLICY_CROSS_PAGE_CALLS: bool = true;
// Allow the page policy's initially bounded regions to grow only after
// sampled exits prove that execution repeatedly crosses into another hot
// code page. This reuses the adaptive policy's extension and short-stay
// demotion machinery behind an explicit browser A/B switch.
static mut PAGE_POLICY_MEASURED_REGIONS: bool = false;
static mut PAGE_POLICY_EXTENSION_MIN_STAY: u64 = EXT_MEMORY_MODE_STAY;
static mut PAGE_POLICY_EXTENSION_SHORT_BLOCKED: u64 = 0;
// Cross-page regions help compact functions that straddle page boundaries,
// but hurt computed-dispatch pages with many independently observed entries.
// Keep the historical unlimited behaviour until the measured production cap
// is selected; the setter makes that discriminator independently sweepable.
static mut PAGE_POLICY_MULTIPAGE_ENTRY_CAP: usize = 512;
// Maximum fraction of sampled entries that may be non-sequential for a page
// participating in a multi-page region, expressed in parts per thousand.
// 1000 preserves the unrestricted baseline; browser sweeps select the value.
static mut PAGE_POLICY_MULTIPAGE_CONTROL_PERMILLE: u64 = 100;
static mut PAGE_POLICY_SAMPLES: u64 = 0;
static mut PAGE_POLICY_RETIRED: u64 = 0;
static mut PAGE_POLICY_CANDIDATES: u64 = 0;
static mut PAGE_POLICY_QUEUE_DROPS: u64 = 0;
static mut PAGE_POLICY_ISSUED: u64 = 0;
static mut PAGE_POLICY_LANDED: u64 = 0;
static mut PAGE_POLICY_FAILED: u64 = 0;
static mut PAGE_POLICY_QUEUE_MAX: u64 = 0;
static mut PAGE_POLICY_STALE_DROPS: u64 = 0;
static mut PAGE_POLICY_ISSUED_PAGES: u64 = 0;
static mut PAGE_POLICY_MULTI_PAGE_ISSUED: u64 = 0;
static mut PAGE_POLICY_REBUILDS: u64 = 0;
static mut PAGE_POLICY_CONTROL_ENTRY_SAMPLES: u64 = 0;
static mut PAGE_POLICY_MULTIPAGE_ENTRY_ELIGIBLE: u64 = 0;
static mut PAGE_POLICY_MULTIPAGE_ENTRY_BLOCKED: u64 = 0;
static mut PAGE_POLICY_MULTIPAGE_CONTROL_ELIGIBLE: u64 = 0;
static mut PAGE_POLICY_MULTIPAGE_CONTROL_BLOCKED: u64 = 0;
static mut PAGE_POLICY_FETCH_STRADDLE_FORCED: u64 = 0;
static mut PAGE_POLICY_FETCH_STRADDLE_DEFERRED: u64 = 0;
static mut PAGE_POLICY_USER_RETIRED: u64 = 0;
static mut PAGE_POLICY_PRIVILEGED_RETIRED: u64 = 0;
static mut PAGE_POLICY_USER_CANDIDATES: u64 = 0;
static mut PAGE_POLICY_PRIVILEGED_CANDIDATES: u64 = 0;
/// Exact, single-page compiled-function reuse opportunity probe.  This is
/// opt-in and observational: it hashes only at region-build time and never
/// changes which module is translated, compiled, installed, or executed.
static mut PAGE_TEMPLATE_PROBE: bool = false;
static mut PAGE_TEMPLATE_ELIGIBLE: u64 = 0;
static mut PAGE_TEMPLATE_CODE_MATCH: u64 = 0;
static mut PAGE_TEMPLATE_REUSABLE: u64 = 0;
static mut PAGE_TEMPLATE_CROSS_PHYSICAL: u64 = 0;
static mut PAGE_TEMPLATE_MATCH_REQUESTED_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_MATCH_COVERED_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_MATCH_MISSING_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_UNION_COVERED_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_UNION_MISSING_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_RELOCATED_MATCH: u64 = 0;
static mut PAGE_TEMPLATE_RELOCATED_REQUESTED_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_RELOCATED_COVERED_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_RELOCATED_MISSING_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_RELOCATED_PAIRS: Vec<[u64; 6]> = Vec::new();
/// Controlled candidate: cache only position-independent modules whose code
/// page has already been observed at another virtual address. Disabled by
/// default until correctness and browser A/B gates promote it.
static mut PAGE_TEMPLATE_REUSE: bool = false;
static mut PAGE_TEMPLATE_PIC_COMPILES: u64 = 0;
static mut PAGE_TEMPLATE_REUSE_HITS: u64 = 0;
static mut PAGE_TEMPLATE_REUSE_COVERED_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_REUSE_MISSING_ENTRIES: u64 = 0;
static mut PAGE_TEMPLATE_REUSE_SAVED_WASM_BYTES: u64 = 0;
static mut PAGE_TEMPLATE_EAGER_PHYSICAL_CANDIDATES: u64 = 0;
static mut NEXT_PAGE_TEMPLATE_CACHE_ID: u64 = 1;
/// Description of the page-template opportunity for the module currently in
/// `JIT_OUT`.  The async host import reads this synchronously, before another
/// region can replace either buffer.  Fields are: match class (0 probe off,
/// 1 eligible/no match, 2 same-VA match, 3 relocated match, 4 ineligible),
/// current VA page, current PA page, template VA page, template PA page,
/// requested entries, covered entries, emitted Wasm bytes.
static mut JIT_OUT_PAGE_TEMPLATE_DIAG: [u64; 8] = [0; 8];
/// Host compiled-module-cache action for the async module in `JIT_OUT`:
/// action 0 = ordinary compile, 1 = compile and retain under id, 2 = instantiate
/// retained id without compiling. Field 2 is the alias's guest page base.
static mut JIT_OUT_TEMPLATE_CACHE: [u64; 3] = [0; 3];

#[allow(static_mut_refs)]
fn reset_page_template_probe_stats() {
    unsafe {
        PAGE_TEMPLATE_ELIGIBLE = 0;
        PAGE_TEMPLATE_CODE_MATCH = 0;
        PAGE_TEMPLATE_REUSABLE = 0;
        PAGE_TEMPLATE_CROSS_PHYSICAL = 0;
        PAGE_TEMPLATE_MATCH_REQUESTED_ENTRIES = 0;
        PAGE_TEMPLATE_MATCH_COVERED_ENTRIES = 0;
        PAGE_TEMPLATE_MATCH_MISSING_ENTRIES = 0;
        PAGE_TEMPLATE_UNION_COVERED_ENTRIES = 0;
        PAGE_TEMPLATE_UNION_MISSING_ENTRIES = 0;
        PAGE_TEMPLATE_RELOCATED_MATCH = 0;
        PAGE_TEMPLATE_RELOCATED_REQUESTED_ENTRIES = 0;
        PAGE_TEMPLATE_RELOCATED_COVERED_ENTRIES = 0;
        PAGE_TEMPLATE_RELOCATED_MISSING_ENTRIES = 0;
        PAGE_TEMPLATE_RELOCATED_PAIRS.clear();
        JIT_OUT_PAGE_TEMPLATE_DIAG = [0; 8];
        PAGE_TEMPLATE_PIC_COMPILES = 0;
        PAGE_TEMPLATE_REUSE_HITS = 0;
        PAGE_TEMPLATE_REUSE_COVERED_ENTRIES = 0;
        PAGE_TEMPLATE_REUSE_MISSING_ENTRIES = 0;
        PAGE_TEMPLATE_REUSE_SAVED_WASM_BYTES = 0;
        PAGE_TEMPLATE_EAGER_PHYSICAL_CANDIDATES = 0;
        JIT_OUT_TEMPLATE_CACHE = [0; 3];
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_set_page_template_probe(on: u32) {
    unsafe {
        PAGE_TEMPLATE_PROBE = on != 0;
        reset_page_template_probe_stats();
        if let Some(jit) = SYS_JIT.as_mut() {
            jit.page_templates.clear();
            jit.page_template_cached_offsets.clear();
        }
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_set_page_template_reuse(on: u32) {
    unsafe {
        PAGE_TEMPLATE_REUSE = on != 0;
        reset_page_template_probe_stats();
        if let Some(jit) = SYS_JIT.as_mut() {
            jit.page_templates.clear();
            jit.page_template_cached_offsets.clear();
        }
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_page_template_pair_count() -> u32 {
    unsafe { PAGE_TEMPLATE_RELOCATED_PAIRS.len() as u32 }
}

/// Relocation-probe pair fields: current/template virtual page,
/// current/template physical page, requested entries, covered entries.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_page_template_pair(index: u32, field: u32) -> u64 {
    unsafe {
        PAGE_TEMPLATE_RELOCATED_PAIRS
            .get(index as usize)
            .and_then(|pair| pair.get(field as usize))
            .copied()
            .unwrap_or(0)
    }
}

/// Page-template diagnostic fields for the async module currently staged in
/// `JIT_OUT`; see `JIT_OUT_PAGE_TEMPLATE_DIAG`.  This is observational and is
/// all zero when the opt-in probe is disabled.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_out_page_template_diag(field: u32) -> u64 {
    unsafe {
        JIT_OUT_PAGE_TEMPLATE_DIAG
            .get(field as usize)
            .copied()
            .unwrap_or(0)
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_out_template_cache(field: u32) -> u64 {
    unsafe {
        JIT_OUT_TEMPLATE_CACHE
            .get(field as usize)
            .copied()
            .unwrap_or(0)
    }
}

#[allow(static_mut_refs)]
fn reset_page_policy_stats() {
    unsafe {
        PAGE_POLICY_SAMPLES = 0;
        PAGE_POLICY_RETIRED = 0;
        PAGE_POLICY_CANDIDATES = 0;
        PAGE_POLICY_QUEUE_DROPS = 0;
        PAGE_POLICY_ISSUED = 0;
        PAGE_POLICY_LANDED = 0;
        PAGE_POLICY_FAILED = 0;
        PAGE_POLICY_QUEUE_MAX = 0;
        PAGE_POLICY_STALE_DROPS = 0;
        PAGE_POLICY_ISSUED_PAGES = 0;
        PAGE_POLICY_MULTI_PAGE_ISSUED = 0;
        PAGE_POLICY_REBUILDS = 0;
        PAGE_POLICY_CONTROL_ENTRY_SAMPLES = 0;
        PAGE_POLICY_MULTIPAGE_ENTRY_ELIGIBLE = 0;
        PAGE_POLICY_MULTIPAGE_ENTRY_BLOCKED = 0;
        PAGE_POLICY_MULTIPAGE_CONTROL_ELIGIBLE = 0;
        PAGE_POLICY_MULTIPAGE_CONTROL_BLOCKED = 0;
        PAGE_POLICY_FETCH_STRADDLE_FORCED = 0;
        PAGE_POLICY_FETCH_STRADDLE_DEFERRED = 0;
        PAGE_POLICY_USER_RETIRED = 0;
        PAGE_POLICY_PRIVILEGED_RETIRED = 0;
        PAGE_POLICY_USER_CANDIDATES = 0;
        PAGE_POLICY_PRIVILEGED_CANDIDATES = 0;
        PAGE_POLICY_EXTENSION_SHORT_BLOCKED = 0;
        reset_page_template_probe_stats();
    }
}

/// Select the page-heat/async-only compiler policy. The switch is intended to
/// be set before boot; changing it also drops policy-incompatible generated
/// state so an A/B run cannot accidentally execute the other policy's code.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_set_page_policy(on: u32) {
    unsafe {
        let enabled = on != 0;
        if enabled == PAGE_POLICY_ENABLED {
            return;
        }
        PAGE_POLICY_ENABLED = enabled;
        PENDING_SB.clear();
        if let Some(jit) = SYS_JIT.as_mut() {
            jit.clear();
        }
        reset_page_policy_stats();
    }
}

/// Candidate mapping heat required before its first asynchronous page build.
#[no_mangle]
pub extern "C" fn jit_set_page_threshold(instructions: u32) {
    unsafe { PAGE_POLICY_THRESHOLD = u64::from(instructions.max(1)) }
}

#[no_mangle]
pub extern "C" fn jit_set_privileged_page_threshold_multiplier(multiplier: u32) {
    unsafe {
        PAGE_POLICY_PRIVILEGED_THRESHOLD_MULTIPLIER = u64::from(multiplier.clamp(1, 1024));
    }
}

/// Select the interpreted sampling interval used by the experimental page
/// policy. Larger intervals reduce policy overhead but may attribute a short
/// cross-page stretch to its starting mapping and may interpret past a newly
/// compiled entry by at most this many instructions.
#[no_mangle]
pub extern "C" fn jit_set_page_quantum(instructions: u32) {
    unsafe { PAGE_POLICY_QUANTUM = u64::from(instructions.clamp(1, 4096)) }
}

/// Rebuild a hot page from all accumulated entry seeds when a new entry pays
/// the threshold. Disabling retains the historical new-entry-only fragments
/// for controlled browser A/B measurements.
#[no_mangle]
pub extern "C" fn jit_set_page_rebuild(on: u32) {
    unsafe { PAGE_POLICY_REBUILD = on != 0 }
}

/// Record one mapped non-sequential interpreter target per sampled chunk as an
/// entry seed. This is an experimental CPython/computed-dispatch policy input;
/// disabling selects the exact historical sampler for A/B measurements.
#[no_mangle]
pub extern "C" fn jit_set_page_control_entries(on: u32) {
    unsafe { PAGE_POLICY_CONTROL_ENTRIES = on != 0 }
}

#[no_mangle]
pub extern "C" fn jit_set_page_privileged_control_entries(on: u32) {
    unsafe { PAGE_POLICY_PRIVILEGED_CONTROL_ENTRIES = on != 0 }
}

/// Observe mapped non-sequential interpreter targets as a page-shape signal,
/// but leave entry seeding under the independent `control_entries` switch.
#[no_mangle]
pub extern "C" fn jit_set_page_control_profile(on: u32) {
    unsafe { PAGE_POLICY_CONTROL_PROFILE = on != 0 }
}

/// Maximum page modules concurrently handed to the embedding engine's
/// asynchronous WebAssembly compiler. Keep this bounded so hot code cannot
/// create an unbounded browser compile queue.
#[no_mangle]
pub extern "C" fn jit_set_page_inflight_limit(limit: u32) {
    unsafe { PAGE_POLICY_INFLIGHT_LIMIT = (limit as usize).clamp(1, 8) }
}

/// Permit a page-policy candidate to grow into adjacent reachable pages only
/// while its lead page has at most this many observed entry PCs. Zero disables
/// page growth without changing the independently useful region page cap.
#[no_mangle]
pub extern "C" fn jit_set_page_multipage_entry_cap(entries: u32) {
    unsafe { PAGE_POLICY_MULTIPAGE_ENTRY_CAP = (entries as usize).min(512) }
}

#[no_mangle]
pub extern "C" fn jit_set_page_multipage_control_permille(permille: u32) {
    unsafe { PAGE_POLICY_MULTIPAGE_CONTROL_PERMILLE = u64::from(permille.min(1000)) }
}

#[no_mangle]
pub extern "C" fn jit_set_page_cross_page_calls(on: u32) {
    unsafe { PAGE_POLICY_CROSS_PAGE_CALLS = on != 0 }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_set_page_measured_regions(on: u32) {
    unsafe {
        let enabled = on != 0;
        if enabled == PAGE_POLICY_MEASURED_REGIONS {
            return;
        }
        PAGE_POLICY_MEASURED_REGIONS = enabled;
        EXIT_TICK = 0;
        if let Some(jit) = SYS_JIT.as_mut() {
            jit.ext_queue.clear();
            for profile in jit.region_exits.values_mut() {
                profile.total = 0;
                profile.targets.clear();
                profile.samples = 0;
                profile.stay_sum = 0;
                profile.last_tick = EXIT_TICK;
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn jit_set_page_extension_min_stay(instructions: u32) {
    unsafe { PAGE_POLICY_EXTENSION_MIN_STAY = u64::from(instructions.clamp(1, 4096)) }
}

/// Page-policy diagnostics: enabled, threshold, quantum, sampled callbacks,
/// sampled retired instructions, live mappings, candidates, queued/pending,
/// queue drops/max, issued/landed/failed builds, and compiled mappings.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_page_policy_stat(which: u32) -> u64 {
    unsafe {
        let jit = SYS_JIT.as_ref();
        match which {
            0 => PAGE_POLICY_ENABLED as u64,
            1 => PAGE_POLICY_THRESHOLD,
            2 => PAGE_POLICY_QUANTUM,
            3 => PAGE_POLICY_SAMPLES,
            4 => PAGE_POLICY_RETIRED,
            5 => jit.map_or(0, |state| state.policy_heat.len() as u64),
            6 => PAGE_POLICY_CANDIDATES,
            7 => jit.map_or(0, |state| state.policy_queue.len() as u64),
            8 => jit.map_or(0, |state| state.policy_pending.len() as u64),
            9 => PAGE_POLICY_QUEUE_DROPS,
            10 => PAGE_POLICY_QUEUE_MAX,
            11 => PAGE_POLICY_ISSUED,
            12 => PAGE_POLICY_LANDED,
            13 => PAGE_POLICY_FAILED,
            14 => jit.map_or(0, |state| state.policy_compiled.len() as u64),
            15 => PAGE_POLICY_QUEUE_CAP as u64,
            16 => jit.map_or(0, |state| state.policy_rejected.len() as u64),
            17 => jit.map_or(0, |state| state.policy_suppressed.len() as u64),
            18 => PAGE_POLICY_STALE_DROPS,
            19 => PAGE_POLICY_ISSUED_PAGES,
            20 => PAGE_POLICY_MULTI_PAGE_ISSUED,
            21 => PAGE_POLICY_REBUILDS,
            22 => PAGE_POLICY_REBUILD as u64,
            23 => PAGE_POLICY_CONTROL_ENTRIES as u64,
            24 => PAGE_POLICY_CONTROL_ENTRY_SAMPLES,
            25 => PAGE_POLICY_INFLIGHT_LIMIT as u64,
            26 => PAGE_POLICY_MULTIPAGE_ENTRY_CAP as u64,
            27 => PAGE_POLICY_MULTIPAGE_ENTRY_ELIGIBLE,
            28 => PAGE_POLICY_MULTIPAGE_ENTRY_BLOCKED,
            29 => PAGE_POLICY_CROSS_PAGE_CALLS as u64,
            30 => PAGE_POLICY_MEASURED_REGIONS as u64,
            31 => region_extension_page_cap() as u64,
            32 => PAGE_POLICY_EXTENSION_MIN_STAY,
            33 => PAGE_POLICY_EXTENSION_SHORT_BLOCKED,
            34 => PAGE_POLICY_MULTIPAGE_CONTROL_PERMILLE,
            35 => PAGE_POLICY_MULTIPAGE_CONTROL_ELIGIBLE,
            36 => PAGE_POLICY_MULTIPAGE_CONTROL_BLOCKED,
            37 => PAGE_POLICY_CONTROL_PROFILE as u64,
            38 => region_page_cap() as u64,
            39 => region_leader_cap() as u64,
            40 => rv64_dbt::region_tail_chain_enabled() as u64,
            41 => PAGE_POLICY_FETCH_STRADDLE_FORCED,
            42 => PAGE_POLICY_FETCH_STRADDLE_DEFERRED,
            43 => REGION_TLB_CACHE as u64,
            44 => REGION_TLB_CACHE_MIN_ACCESSES as u64,
            45 => PAGE_POLICY_PRIVILEGED_THRESHOLD_MULTIPLIER,
            46 => PAGE_POLICY_USER_RETIRED,
            47 => PAGE_POLICY_PRIVILEGED_RETIRED,
            48 => PAGE_POLICY_USER_CANDIDATES,
            49 => PAGE_POLICY_PRIVILEGED_CANDIDATES,
            50 => PAGE_POLICY_PRIVILEGED_CONTROL_ENTRIES as u64,
            51 => PAGE_POLICY_STABLE_CHAIN as u64,
            _ => 0,
        }
    }
}

#[inline]
fn measured_region_policy_enabled() -> bool {
    unsafe { !PAGE_POLICY_ENABLED || PAGE_POLICY_MEASURED_REGIONS }
}
/// Tier-up threshold for the per-EXECUTION interp-stretch counter. Deliberately
/// much higher than JIT_THRESHOLD (which counts block-entry events): blocks and
/// hot-counts persist across context switches now, so a low per-execution bar
/// makes boot synchronously compile ~19k one-shot cold blocks (~0.1ms of
/// WebAssembly.Module each = seconds of boot). Steady-state hot code executes
/// millions of times and crosses 1024 in microseconds.
const INTERP_HOT_THRESHOLD: u16 = 2048;
/// Interpreter fallback slice once JIT blocks exist (tuned below).
const SYS_WARM_SLICE: u64 = 256;

/// Enable/disable JIT execution and tier-up (1/0). Disabled runs take the
/// machine's direct interpreter path without cache probes, hot counters,
/// invalidation checks, or bounded JIT fallback slices.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_set_enabled(on: u32) {
    unsafe {
        JIT_ENABLED = on != 0;
        JIT_THRESHOLD = if JIT_ENABLED {
            JIT_ON_THRESHOLD
        } else {
            u32::MAX
        };
        // "Disabled" means EXECUTE NO JIT CODE, not just "stop compiling":
        // drop already-compiled blocks so A/B comparisons and the API name
        // stay honest (PERFORMANCE_PROGRESS.md). (Wasm function-table entries are not
        // reclaimable, but they become unreachable.)
        if !JIT_ENABLED {
            // Ignore any asynchronous region completion still in flight. Its
            // callback will find no matching ticket and cannot repopulate the
            // cleared dispatch state.
            PENDING_SB.clear();
            #[allow(clippy::deref_addrof)]
            if let Some(j) = (*(&raw mut SYS_JIT)).as_mut() {
                j.clear();
            }
            #[allow(clippy::deref_addrof)]
            if let Some(j) = (*(&raw mut USER_JIT)).as_mut() {
                j.clear();
            }
        }
    }
}
/// Opt-in: drive the guest CLINT from real host wall-clock instead of the
/// default deterministic instruction-counted time. For benchmarks that self-
/// time via the guest clock (nbench) and realistic `date`/timeouts. Off by
/// default so lockstep/differential testing stays reproducible.
static mut SYS_WALLCLOCK: bool = false;
static mut WALL_LAST_ICOUNT: u64 = 0;
static mut WALL_IDLE_ITERS: u32 = 0;
#[no_mangle]
pub extern "C" fn sys_set_wallclock(on: u32) {
    unsafe { SYS_WALLCLOCK = on != 0 }
}
/// Opt-in: fold branchy code pages into one superblock (function-per-page with
/// an internal br_table dispatch). Correct and validated, but per-page
/// granularity doesn't capture CPython's multi-page eval loop and the
/// recompile-on-new-entry cost regresses short warmups — off until whole-
/// function superblocks + incremental compilation land. See translate_superblock.
static mut SYS_SUPERBLOCK: bool = false;
#[no_mangle]
pub extern "C" fn sys_set_superblock(on: u32) {
    unsafe { SYS_SUPERBLOCK = on != 0 }
}
/// Max chained block dispatches before returning to the interpreter (keeps
/// interrupt/budget latency bounded in fully-jitted loops).
const JIT_CHAIN_CAP: u32 = 1024;
/// Once a code page accumulates this many hot NON-loop block entries it is
/// branchy enough (e.g. an interpreter's dispatch loop) to compile as one
/// superblock — one wasm function covering the whole page with an internal
/// br_table dispatch and registers cached in locals across all blocks.
/// Hot pcs on a page before it is compiled as one function. Low on purpose:
/// every individual block is its own WebAssembly module — a Module build, an
/// Instance, and a table growth each — so a page's worth of them costs far
/// more than the single page function that covers the same code (an in-guest
/// `tcc -c` built 8517 block modules against 54 page functions).
const SUPERBLOCK_THRESHOLD: usize = 6;
/// How many times one page may be recompiled as a superblock as more of it
/// turns out to be hot (see JitState::sb_gen).
const SB_RECOMPILE_CAP: u32 = 16;
/// Distinct (address space, page) discovery records kept before the whole
/// table is dropped — address spaces die and their pages go with them.
const SB_SPACE_CAP: usize = 16384;
/// Superblock/region builds are paced by their MEASURED host cost, not a
/// flat instruction gap: cumulative host-side build time (leader analysis,
/// register scan, wasm emission — the V8 module compile itself is async and
/// off-thread) may not exceed this fraction of wall time since the machine
/// started. The old flat 16M-insn spacing allowed ~20 builds over an entire
/// `tcc -c`, so the hot call graph's ~50 pages never got covered while the
/// workload still ran (measured: 34 landed, 8.0 insns/dispatch, extension
/// starved); a fraction-of-runtime budget lets a cold workload take a fast
/// burst of coverage while still bounding total compile cost on any run.
const SB_BUILD_BUDGET: f64 = 0.08;
/// Floor between two builds, in retired instructions. This is the old flat
/// spacing: at 1M the build/rebuild rate went up ~16x and python fib ran
/// 8.5s against 3.6s — rebuild churn discards V8-optimized page functions
/// (the measured FP EMULATION 3x cliff), so the wall-time budget alone is
/// NOT a sufficient pacing signal. The budget still caps pathological
/// translate storms below this floor's rate.
static mut SB_MIN_SPACING: u64 = 16_000_000;
#[no_mangle]
pub extern "C" fn jit_set_sb_spacing(m_insns: u32) {
    unsafe { SB_MIN_SPACING = m_insns as u64 * 1_000_000 }
}
static mut SB_BUILD_MS: f64 = 0.0;
static mut SB_ANCHOR_MS: f64 = -1.0;

/// May another region build be issued now? (Measured-cost budget above.)
#[allow(static_mut_refs)]
fn sb_build_allowed(insn_count: u64) -> bool {
    unsafe {
        if insn_count < SB_LAST_ICOUNT.wrapping_add(SB_MIN_SPACING) {
            return false;
        }
        if SB_ANCHOR_MS < 0.0 {
            SB_ANCHOR_MS = host_now_ms();
        }
        let elapsed = host_now_ms() - SB_ANCHOR_MS;
        // The +2ms grace admits the first builds while elapsed is still ~0.
        SB_BUILD_MS <= elapsed * SB_BUILD_BUDGET + 2.0
    }
}
/// Deferred superblock requests kept before new ones are dropped.
const SB_QUEUE_CAP: usize = 64;
/// Individually-compiled hot pcs on a superblocked page before its page
/// function is rebuilt to cover them.
const SB_MISSED_TRIGGER: u32 = 8;
/// Retired instructions a page must run before its FIRST rebuild; each further
/// rebuild doubles the wait (see the cooldown comment at the trigger).
const SB_PAGE_COOLDOWN: u64 = 8_000_000;
static mut SB_LAST_ICOUNT: u64 = 0;
/// Guest TLB misses served inside compiled code (jit_stat 31). In-block TLB
/// fill is off by default: it costs register pressure in every memory-op block
/// and buys ~10% on an in-guest `tcc -c` (whose symbol tables thrash the TLB)
/// while costing ~15% on CPython's eval loop, whose working set never misses.
/// Switching it from a measured miss rate was tried and made the CPython row
/// worse without flipping the tcc row, so the policy stays explicit:
/// jit_set_tlb_fill(1) for guests with a working set past the 4096-entry TLB.
static mut TLB_FILLS: u64 = 0;
// Refill attribution by [load/store][hit, empty, context, page collision].
// The helper is already a cold path; these counters distinguish invalidation
// from capacity/conflict misses without perturbing generated probe shape.
static mut TLB_FILL_KIND: [u64; 8] = [0; 8];
/// Select a folded VPN index for the fused generated-code TLB. This is a code
/// shape choice: modules compiled under the direct and folded policies probe
/// different slots. The setter therefore retires the current generated cache
/// before changing the CPU's publication policy; callers can deliberately set
/// either value to obtain an equally cold A/B baseline.
static mut JIT_TLB_HASH: bool = false;

/// Let the interpreter consume an already permission/context-checked fused row
/// as a direct RAM pointer. R054 promoted this after exact differentials and
/// same-Wasm plus accepted-artifact modern-Linux A/B gates. Generated code and
/// row publication/invalidation remain unchanged; the setter retains an exact
/// disabled baseline for diagnostics.
static mut INTERPRETER_FUSED_MEMORY: bool = true;

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_set_interpreter_fused_memory(on: u32) {
    let enabled = on != 0;
    unsafe {
        INTERPRETER_FUSED_MEMORY = enabled;
        if let Some(machine) = SYS.as_mut() {
            machine.cpu.set_interpreter_fused_memory(enabled);
        }
        if let Some(machine) = VIRT.as_mut() {
            machine.cpu.set_interpreter_fused_memory(enabled);
        }
    }
}

/// Diagnostic control for an exact A/B against the former policy that cleared
/// every translation on every trap/xRET. Generated module shape is unchanged;
/// only retention of already context-tagged proofs differs.
static mut PRIVILEGE_TLB_RETENTION: bool = true;

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_set_privilege_tlb_retention(on: u32) {
    let retain = on != 0;
    unsafe {
        PRIVILEGE_TLB_RETENTION = retain;
        if let Some(machine) = SYS.as_mut() {
            machine.cpu.set_privilege_tlb_retention(retain);
        }
        if let Some(machine) = VIRT.as_mut() {
            machine.cpu.set_privilege_tlb_retention(retain);
        }
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_set_tlb_hash(on: u32) {
    let hashed = on != 0;
    unsafe {
        JIT_TLB_HASH = hashed;
        match ACTIVE_SYSTEM_KIND {
            kind if kind == SystemMachineKind::Legacy as u8 => {
                if let Some(machine) = SYS.as_mut() {
                    reset_system_jit_for_tlb_hash(machine, hashed);
                }
            }
            kind if kind == SystemMachineKind::Virt as u8 => {
                if let Some(machine) = VIRT.as_mut() {
                    reset_system_jit_for_tlb_hash(machine, hashed);
                }
            }
            _ => {
                if let Some(jit) = SYS_JIT.as_mut() {
                    jit.clear();
                }
            }
        }
        // Finished modules can arrive asynchronously after configuration. A
        // new generation plus an empty ticket set makes those old-layout
        // callbacks harmless and forces subsequent discovery to start clean.
        BOOT_GEN += 1;
        PENDING_SB.clear();
        reset_chain_runtime();
    }
}

#[allow(static_mut_refs)]
fn reset_system_jit_for_tlb_hash(machine: &mut impl SystemJitMachine, hashed: bool) {
    let pages = unsafe {
        SYS_JIT
            .as_ref()
            .map(|jit| jit.page_blocks.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    for page in pages {
        machine.unmark_jit_page(page);
    }
    unsafe {
        if let Some(jit) = SYS_JIT.as_mut() {
            jit.clear();
        }
    }
    machine.cpu_mut().set_jit_tlb_hash(hashed);
    // The explicit configuration call defines the cold point for both A/B
    // arms, even when the requested policy equals the default.
    machine.cpu_mut().flush_tlb();
}

/// Retain one proven load/store translation in dense direct-copy members.
/// The structural filter avoids the CPython and floating-point regressions of
/// the earlier all-memory experiment. Frozen full-size-TLB corpora show the
/// eight-word copy form winning in V8/Chromium/Firefox; four pairs is the
/// production break-even guard against the two-pair Chromium loss.
static mut REGION_TLB_CACHE: bool = true;
static mut REGION_TLB_CACHE_MIN_ACCESSES: u16 = 4;
#[no_mangle]
pub extern "C" fn jit_set_region_tlb_cache(on: u32) {
    unsafe { REGION_TLB_CACHE = on != 0 }
}

#[no_mangle]
pub extern "C" fn jit_set_region_tlb_cache_min_accesses(accesses: u32) {
    unsafe { REGION_TLB_CACHE_MIN_ACCESSES = accesses.clamp(1, 64) as u16 }
}

/// Load a region's architectural register union on first dynamic use instead
/// of eagerly at function entry. Kept opt-in until captured real-region and
/// cross-engine samples show that the saved memory traffic exceeds the added
/// validity branches/local pressure.
static mut REGION_LAZY_STATE: bool = false;

#[no_mangle]
pub extern "C" fn jit_set_region_lazy_state(on: u32) {
    unsafe { REGION_LAZY_STATE = on != 0 }
}
/// Structured dense-switch dispatcher for known same-module member edges.
/// This is a portable core-Wasm experiment; the balanced PC tree remains the
/// default until real-region, multi-engine results justify the larger body.
static mut REGION_DIRECT_DISPATCH: bool = true;

#[no_mangle]
pub extern "C" fn jit_set_region_direct_dispatch(on: u32) {
    unsafe { REGION_DIRECT_DISPATCH = on != 0 }
}
/// Basic-block CFG members with both covered branch successors kept inside the
/// dense structured dispatcher. Opt-in while differential and browser A/B
/// gates compare it with the guarded-trace region form.
static mut REGION_CFG_BLOCKS: bool = false;

#[no_mangle]
pub extern "C" fn jit_set_region_cfg_blocks(on: u32) {
    unsafe { REGION_CFG_BLOCKS = on != 0 }
}
/// Stackifier-style structured lowering for basic-block CFG regions. This is
/// separate from `REGION_CFG_BLOCKS` while the dense-dispatch bridge remains a
/// controlled A/B baseline.
static mut REGION_STRUCTURED_CFG: bool = true;

#[no_mangle]
pub extern "C" fn jit_set_region_structured_cfg(on: u32) {
    unsafe { REGION_STRUCTURED_CFG = on != 0 }
}

/// Leaders per superblock. Every entry into the function loads the register
/// UNION over all its bodies and every exit stores the written union, so a
/// function that covers more of the page pays more on each entry — worth it
/// for code that then stays inside (IDEA), ruinous for code that re-enters
/// constantly (FOURIER's cross-page libm calls). Hot pcs are seeded first, so
/// the cap trims cold reachable code, not the hot core.
const HARD_MAX_REGION_LEADERS: usize = 512;
// Matched Linux 6.12 / CPython / SHA sweeps found that 128 leaders stranded
// cross-page entries (about 95% generated coverage). 512 restores ~99% while
// the control-entry gate prevents large computed-dispatch regions.
static mut REGION_LEADER_CAP: usize = HARD_MAX_REGION_LEADERS;

#[no_mangle]
pub extern "C" fn jit_set_region_leader_cap(leaders: u32) {
    unsafe { REGION_LEADER_CAP = (leaders as usize).clamp(2, HARD_MAX_REGION_LEADERS) }
}

fn region_leader_cap() -> usize {
    unsafe { REGION_LEADER_CAP }
}

/// Call a compiled block. The state pointer parameter deliberately escapes
/// the emulator state into the opaque call so the compiler reloads CPU
/// fields afterwards instead of caching them in locals.
#[inline]
fn call_block(idx: i32, state_ptr: *mut u8) {
    unsafe {
        // The retirement cell is CUMULATIVE across one host dispatch: blocks
        // ADD what they retire (tail-call transfers keep accumulating without
        // returning here), so it must start each chain at zero.
        RETIRED_CELL = 0;
        IC_MISS_OWNER_CELL = NO_PC;
        let f: extern "C" fn(i32) = core::mem::transmute(idx as usize);
        f(state_ptr as i32);
    }
}

/// Page/CFG regions do not carry the single-trace indirect-target guard whose
/// miss is reported through `IC_MISS_OWNER_CELL`. Avoid clearing and probing
/// that feedback channel on every region invocation.
#[inline]
fn call_region(idx: i32, state_ptr: *mut u8) {
    unsafe {
        RETIRED_CELL = 0;
        let f: extern "C" fn(i32) = core::mem::transmute(idx as usize);
        f(state_ptr as i32);
    }
}

// Standalone superblock-emitter validation state. The first 40 words retain the
// original GPR/PC/retirement layout; words 40..72 hold FP registers and word 72
// begins the fcsr cell used by the mixed-state test.
static mut SBSTATE: [u64; 80] = [0; 80];
static mut SBTAIL_DISPATCH: [DispatchLine; 16] = [DispatchLine {
    pc: NO_PC,
    idx: -1,
    gen: 0,
}; 16];
static mut SBTAIL_MAP_GEN: u32 = 0;
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sbtest() -> u64 {
    const PROG: [u32; 7] = [
        0x00000093, 0x00100113, 0x00b00193, 0x002080b3, 0x00110113, 0xfe311ce3, 0x00000073,
    ];
    let code: Vec<u8> = PROG.iter().flat_map(|w| w.to_le_bytes()).collect();
    let base = 0x1000u64;
    unsafe {
        SBSTATE = [0; 80];
        let sp = SBSTATE.as_ptr() as u32;
        SBSTATE[32] = base; // pc
        let lay = rv64_dbt::JitLayout {
            x_base: sp,
            pc_addr: sp + 256,
            mem: None,
            sys: None,
            mem_profile: None,
            reg_stress: false,
            reg_profile_base: 0,
            structured_profile: None,
            multi_latch: false,
            retired_addr: sp + 264,
            f_base: 0,
            fcsr_addr: 0,
            reservation: None,
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
        };
        let entries = [0x1000u64, 0x100c];
        let blk = match rv64_dbt::translate_superblock(&code, base, 0x1000, 0x40, &entries, lay) {
            Some(b) => b,
            None => return 0xDEAD_0001,
        };
        JIT_OUT = blk.wasm;
        let idx = host_jit_register();
        if idx < 0 {
            return 0xDEAD_0002;
        }
        call_block(idx, sp as *mut u8);
        SBSTATE[1] // x1 == 55 if correct
    }
}

/// Compile the same conditional loop as `sbtest` through the basic-block CFG
/// and Stackifier path. This directly executes a reducible loop with two
/// callable entries, rather than merely validating the generated module.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sbtest_structured() -> u64 {
    const PROG: [u32; 7] = [
        0x00000093, 0x00100113, 0x00b00193, 0x002080b3, 0x00110113, 0xfe311ce3, 0x00000073,
    ];
    let mut code: Vec<u8> = PROG.iter().flat_map(|word| word.to_le_bytes()).collect();
    code.resize(0x1000, 0);
    unsafe {
        SBSTATE = [0; 80];
        let state = SBSTATE.as_ptr() as u32;
        SBSTATE[32] = 0x1000;
        let mut layout = rv64_dbt::JitLayout::bare();
        layout.x_base = state;
        layout.pc_addr = state + 256;
        layout.retired_addr = state + 264;
        let Some(block) = rv64_dbt::translate_superblock_sparse_state(
            &code,
            &[0x1000],
            &[0x1000, 0x100c, 0x1018],
            layout,
            rv64_dbt::MultiEntryState::RegisterStructured,
        ) else {
            return 0xDEAD_1001;
        };
        JIT_OUT = block.wasm;
        let index = host_jit_register();
        if index < 0 {
            return 0xDEAD_1002;
        }
        call_block(index, state as *mut u8);
        if SBSTATE[32] != 0x1018 {
            return 0xDEAD_1003;
        }
        SBSTATE[1]
    }
}

/// Execute a four-block cycle split across two independently compiled
/// structured modules. Every block exits its own module, so reaching the fuel
/// limit with nineteen recorded hops proves that `return_call_indirect`
/// transfers preserve state, use the shared table, and do not escape the
/// cumulative invocation budget.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sbtest_structured_tail_chain() -> u64 {
    let jal = |offset: i32| {
        let value = offset as u32;
        0x6f | (((value >> 12) & 0xff) << 12)
            | (((value >> 11) & 1) << 20)
            | (((value >> 1) & 0x3ff) << 21)
            | (((value >> 20) & 1) << 31)
    };
    let words = [
        0x0010_8093, // 0x1000 A0: addi x1,x1,1
        jal(4),      // -> 0x1008 B0
        0x0011_0113, // 0x1008 B0: addi x2,x2,1
        jal(4),      // -> 0x1010 A1
        0x0010_8093, // 0x1010 A1: addi x1,x1,1
        jal(4),      // -> 0x1018 B1
        0x0011_0113, // 0x1018 B1: addi x2,x2,1
        jal(-28),    // -> 0x1000 A0
    ];
    let mut code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    code.resize(0x1000, 0);

    unsafe {
        SBSTATE = [0; 80];
        SBTAIL_DISPATCH.fill(DispatchLine {
            pc: NO_PC,
            idx: -1,
            gen: 0,
        });
        SBTAIL_MAP_GEN = 7;
        CHAIN_HOPS = 0;
        RETIRED_CELL = 0;
        FUEL_CELL = 40;
        let state = SBSTATE.as_ptr() as u32;
        SBSTATE[32] = 0x1000;

        let mut layout = rv64_dbt::JitLayout::bare();
        layout.x_base = state;
        layout.pc_addr = state + 256;
        layout.retired_addr = retired_addr();
        layout.fuel_addr = fuel_addr();
        layout.dispatch_base = SBTAIL_DISPATCH.as_ptr() as u32;
        layout.dispatch_mask = 15;
        layout.map_gen_addr = (&raw const SBTAIL_MAP_GEN) as u32;
        layout.chain_hops_addr = chain_hops_addr();

        rv64_dbt::set_region_tail_chain(true);
        let first = rv64_dbt::translate_superblock_sparse_state(
            &code,
            &[0x1000],
            &[0x1000, 0x1010],
            layout,
            rv64_dbt::MultiEntryState::RegisterStructured,
        );
        let Some(first) = first else {
            rv64_dbt::set_region_tail_chain(false);
            return 0xDEAD_3001;
        };
        JIT_OUT = first.wasm;
        let first_index = host_jit_register();
        if first_index < 0 {
            rv64_dbt::set_region_tail_chain(false);
            return 0xDEAD_3002;
        }

        let second = rv64_dbt::translate_superblock_sparse_state(
            &code,
            &[0x1000],
            &[0x1008, 0x1018],
            layout,
            rv64_dbt::MultiEntryState::RegisterStructured,
        );
        let Some(second) = second else {
            rv64_dbt::set_region_tail_chain(false);
            return 0xDEAD_3003;
        };
        JIT_OUT = second.wasm;
        let second_index = host_jit_register();
        rv64_dbt::set_region_tail_chain(false);
        if second_index < 0 {
            return 0xDEAD_3004;
        }

        for (pc, index) in [
            (0x1000, first_index),
            (0x1010, first_index),
            (0x1008, second_index),
            (0x1018, second_index),
        ] {
            let slot = ((pc >> 1) & 15) as usize;
            SBTAIL_DISPATCH[slot] = DispatchLine {
                pc,
                idx: index | SB_IDX_BIT,
                gen: SBTAIL_MAP_GEN,
            };
        }

        call_region(first_index, state as *mut u8);
        if SBSTATE[32] != 0x1000 || RETIRED_CELL != 40 || CHAIN_HOPS != 19 {
            return 0xDEAD_3005;
        }
        (SBSTATE[1] << 48) | (SBSTATE[2] << 32) | (RETIRED_CELL << 16) | CHAIN_HOPS
    }
}

/// Execute a two-member direct cycle through same-module `return_call`
/// wrappers. This is deliberately separate from the portable register-
/// resident backend: engines that do not implement the tail-call proposal can
/// reject the optional module without affecting the default JIT.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sbtest_tail_call() -> u64 {
    let jal = |offset: i32| {
        let value = offset as u32;
        0x6f | (((value >> 12) & 0xff) << 12)
            | (((value >> 11) & 1) << 20)
            | (((value >> 1) & 0x3ff) << 21)
            | (((value >> 20) & 1) << 31)
    };
    let words = [
        0x0010_8093, // 0x1000: addi x1,x1,1
        jal(0x0c),   // jal x0,0x1010
        0x0000_0013,
        0x0000_0013,
        0x0011_0113, // 0x1010: addi x2,x2,1
        jal(-0x14),  // jal x0,0x1000
    ];
    let mut code: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    code.resize(0x1000, 0);
    unsafe {
        SBSTATE = [0; 80];
        let state = SBSTATE.as_ptr() as u32;
        SBSTATE[32] = 0x1000;
        RETIRED_CELL = 0;
        FUEL_CELL = 40;
        let mut layout = rv64_dbt::JitLayout::bare();
        layout.x_base = state;
        layout.pc_addr = state + 256;
        layout.retired_addr = retired_addr();
        layout.fuel_addr = fuel_addr();
        let Some(block) = rv64_dbt::translate_superblock_sparse_state(
            &code,
            &[0x1000],
            &[0x1000, 0x1010],
            layout,
            rv64_dbt::MultiEntryState::MemoryTailCall,
        ) else {
            return 0xDEAD_2001;
        };
        JIT_OUT = block.wasm;
        let index = host_jit_register();
        if index < 0 {
            return 0xDEAD_2002;
        }
        call_block(index, state as *mut u8);
        if RETIRED_CELL != 40 || SBSTATE[32] != 0x1000 {
            return 0xDEAD_2003;
        }
        (SBSTATE[1] << 32) | SBSTATE[2]
    }
}

/// Compile and execute a two-entry FP/control cycle. This is a direct runtime
/// proof that the shared multi-entry backend carries GPRs, raw FP registers,
/// fcsr, exact-helper effects, and retirement coherently across internal edges.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sbtest_fp() -> u64 {
    sbtest_fp_state(rv64_dbt::MultiEntryState::RegisterEager)
}

/// The same mixed GPR/FP/helper proof with first-use architectural loads.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sbtest_fp_lazy() -> u64 {
    sbtest_fp_state(rv64_dbt::MultiEntryState::RegisterLazy)
}

#[allow(static_mut_refs)]
fn sbtest_fp_state(state_mode: rv64_dbt::MultiEntryState) -> u64 {
    const PROG: [u32; 7] = [
        0x0220_80d3, // 0x1000: fadd.d f1,f1,f2
        0x0010_8093, // addi x1,x1,1
        0x0080_006f, // jal x0,0x1010
        0x0000_0013,
        0x0011_0113, // 0x1010: addi x2,x2,1
        0xfe30_c6e3, // blt x1,x3,0x1000
        0x0010_0073, // ebreak: precise interpreter exit
    ];
    let mut code: Vec<u8> = PROG.iter().flat_map(|word| word.to_le_bytes()).collect();
    code.resize(0x1000, 0);
    let base = 0x1000u64;
    unsafe {
        SBSTATE = [0; 80];
        let state = SBSTATE.as_ptr() as u32;
        SBSTATE[3] = 10;
        SBSTATE[32] = base;
        SBSTATE[41] = 1.0f64.to_bits();
        SBSTATE[42] = 0.5f64.to_bits();
        let layout = rv64_dbt::JitLayout {
            x_base: state,
            pc_addr: state + 256,
            // User FP mode is selected through the flat-memory capability;
            // this program performs no memory access, so an empty range is
            // sufficient while still exercising the real user helper path.
            mem: Some((0, 0)),
            sys: None,
            mem_profile: None,
            reg_stress: false,
            reg_profile_base: 0,
            structured_profile: None,
            multi_latch: false,
            retired_addr: state + 264,
            f_base: state + 40 * 8,
            fcsr_addr: state + 72 * 8,
            reservation: None,
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
        };
        let entries = [0x1000u64, 0x1010];
        let Some(block) = rv64_dbt::translate_superblock_sparse_state(
            &code,
            &[base],
            &entries,
            layout,
            state_mode,
        ) else {
            return 0xDEAD_1001;
        };
        JIT_OUT = block.wasm;
        let index = host_jit_register();
        if index < 0 {
            return 0xDEAD_1002;
        }
        call_block(index, state as *mut u8);
        if SBSTATE[1] != 10 || SBSTATE[2] != 10 {
            return 0xDEAD_1003_0000_0000 | (SBSTATE[1] << 16) | SBSTATE[2];
        }
        if SBSTATE[33] != 50 {
            return 0xDEAD_1004;
        }
        if SBSTATE[72] as u32 != 0 {
            return 0xDEAD_1005;
        }
        SBSTATE[41]
    }
}

/// Run the loaded program with JIT tier-up. STOP_EXITED on exit,
/// STOP_BUDGET if out of fuel, STOP_TRAP on unhandled trap.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_run(budget: u64) -> i32 {
    let e = unsafe { USER.as_mut().expect("call user_load() first") };
    if !unsafe { JIT_ENABLED } {
        let mut host = JsHost;
        return match e.machine.run(&mut host, budget) {
            rv64_linux::RunResult::Exited(code) => {
                e.exit_code = code;
                STOP_EXITED
            }
            rv64_linux::RunResult::Budget => STOP_BUDGET,
            rv64_linux::RunResult::Trap(exc) => {
                unsafe { LAST_TRAP = exc.cause() as i32 };
                STOP_TRAP
            }
        };
    }
    let jit = unsafe { USER_JIT.get_or_insert_with(JitState::new) };
    let mut host = JsHost;
    let m = &mut e.machine;
    let mut remaining = budget;

    loop {
        // --- JIT fast path: direct-mapped dispatch, chain blocks ---
        let mut chained = 0u32;
        while chained < JIT_CHAIN_CAP && remaining > 0 {
            unsafe { FUEL_CELL = remaining.min(USER_LOOP_QUANTUM) };
            let pc = m.cpu.pc;
            let line = jit.dispatch[JitState::dslot(pc)];
            let idx = if line.pc == pc {
                line.idx
            } else {
                match jit.cache.get(&pc) {
                    Some(Some(b)) => {
                        let idx = b.idx;
                        jit.dispatch[JitState::dslot(pc)] = DispatchLine { pc, idx, gen: 0 };
                        idx
                    }
                    _ => break,
                }
            };
            if idx < 0 {
                break; // blacklisted (user mode never blacklists with pa, but keep the invariant)
            }
            call_block(idx, m as *mut _ as *mut u8);
            // Read the dynamic retired count the block wrote: self-loop blocks
            // (Phase 3) run a runtime-variable number of iterations, so their
            // length is not the static b.n.
            let retired = unsafe { RETIRED_CELL };
            let dprof_sample = unsafe {
                if DPROF_ON {
                    DPROF_TICK = DPROF_TICK.wrapping_add(1);
                    DPROF_TICK & ((1u64 << DPROF_SAMPLE_SHIFT) - 1) == 0
                } else {
                    false
                }
            };
            if dprof_sample {
                dprof_hit(pc, retired);
                eprof_hit(pc, m.cpu.pc, retired);
            }
            if unsafe { DPROF_ON } {
                unsafe {
                    DPROF_BLOCK_CALLS += 1;
                    DPROF_BLOCK_INSNS += retired;
                }
            }
            m.cpu.insn_count += retired;
            unsafe {
                JIT_RETIRED += retired;
                JIT_DISPATCHES += 1;
            }
            // A precise memory side exit at the first instruction retires
            // nothing and leaves that instruction for the interpreter. Do not
            // spin through the chain cap repeatedly at the same PC.
            if retired == 0 {
                break;
            }
            remaining = remaining.saturating_sub(retired);
            chained += 1;
            if remaining == 0 {
                return STOP_BUDGET;
            }
        }

        // This is a scheduling boundary, not an interpreter fallback. Going
        // through T0 here would execute already-compiled code in 512-insn
        // slices whenever a deliberately short loop quantum reaches the chain
        // cap.
        if chained == JIT_CHAIN_CAP {
            continue;
        }

        // --- hot counting + compile ---
        let pc = m.cpu.pc;
        if unsafe { DPROF_ON } && (pc as usize).saturating_add(4) <= m.mem.len() {
            let o = pc as usize;
            ihist_hit(u32::from_le_bytes([
                m.mem[o],
                m.mem[o + 1],
                m.mem[o + 2],
                m.mem[o + 3],
            ]));
        }
        if !jit_table_full() && !jit.cache.contains_key(&pc) {
            let c = jit.hot.entry(pc).or_insert(0);
            *c += 1;
            if *c >= unsafe { JIT_THRESHOLD } {
                let lay = rv64_dbt::JitLayout {
                    x_base: m.cpu.x.as_ptr() as u32,
                    pc_addr: &m.cpu.pc as *const u64 as u32,
                    mem: Some((m.mem.as_ptr() as u32, m.mem.len() as u64)),
                    sys: None,
                    mem_profile: None,
                    reg_stress: false,
                    reg_profile_base: 0,
                    structured_profile: structured_profile_layout(),
                    multi_latch: false,
                    retired_addr: retired_addr(),
                    f_base: m.cpu.f.as_ptr() as u32,
                    fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
                    reservation: Some(rv64_dbt::ReservationCapability::User),
                    fuel_addr: fuel_addr(),
                    mstatus_addr: 0, // user mode: no privileged FP state
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
                };
                let end = (pc as usize + 1024).min(m.mem.len());
                let translate_start = unsafe { host_now_ms() };
                let translated = rv64_dbt::translate_block(&m.mem[pc as usize..end], pc, pc, lay);
                unsafe {
                    USER_TRANSLATE_ATTEMPTS += 1;
                    USER_TRANSLATE_NS +=
                        ((host_now_ms() - translate_start).max(0.0) * 1_000_000.0) as u64;
                    if let Some(blk) = &translated {
                        USER_EMITTED_BYTES += blk.wasm.len() as u64;
                    }
                }
                let entry = translated.and_then(|blk| {
                    unsafe { JIT_OUT = blk.wasm };
                    let idx = unsafe { host_jit_register() };
                    (idx >= 0).then_some(JitBlock {
                        fp: false,
                        idx,
                        n: blk.n_insns,
                        mix: blk.trace_mix,
                        mem: blk.trace_mem,
                        control: blk.trace_control,
                        alu: blk.trace_alu,
                        pa: pc,
                    })
                });
                jit.cache.insert(pc, entry);
                if entry.is_some() {
                    continue; // dispatch it immediately
                }
            }
        }

        // --- interpreter slice ---
        let slice = remaining.min(512);
        let icount_before = m.cpu.insn_count;
        let stop = {
            let mut bus = FlatMemory::new(0, &mut m.mem);
            m.cpu.run(&mut bus, slice)
        };
        unsafe {
            if DPROF_ON && IHIST_LAST != usize::MAX {
                IHIST_INSNS[IHIST_LAST] += m.cpu.insn_count - icount_before;
                IHIST_LAST = usize::MAX;
            }
        }
        match stop {
            StopReason::Budget => {
                remaining = remaining.saturating_sub(slice);
                if remaining == 0 {
                    return STOP_BUDGET;
                }
            }
            StopReason::Ecall => {
                if let Some(code) = rv64_linux::syscall::handle(m, &mut host) {
                    m.exit_code = Some(code);
                    e.exit_code = code;
                    return STOP_EXITED;
                }
                if m.icache_flush_pending {
                    m.icache_flush_pending = false;
                    jit.clear(); // architectural code-change signal
                }
            }
            StopReason::Break => {
                e.exit_code = 133;
                return STOP_EXITED;
            }
            StopReason::Trap(exc) => {
                unsafe { LAST_TRAP = exc.cause() as i32 };
                return STOP_TRAP;
            }
            StopReason::Wfi => unreachable!(),
        }
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_exit_code() -> i32 {
    unsafe { USER.as_ref().map(|e| e.exit_code).unwrap_or(-1) }
}

/// Read a user-machine GPR (differential testing: full architectural-state
/// comparison between JIT and interpreter runs).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_reg(i: u32) -> u64 {
    unsafe {
        USER.as_ref()
            .map(|e| e.machine.cpu.x[(i & 31) as usize])
            .unwrap_or(0)
    }
}

/// Read a user-machine FP register (raw bits).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_freg(i: u32) -> u64 {
    unsafe {
        USER.as_ref()
            .map(|e| e.machine.cpu.f[(i & 31) as usize])
            .unwrap_or(0)
    }
}

/// User-machine fcsr (flags + rounding mode).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_fcsr() -> u32 {
    unsafe { USER.as_ref().map(|e| e.machine.cpu.fcsr).unwrap_or(0) }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_pc() -> u64 {
    unsafe { USER.as_ref().map(|e| e.machine.cpu.pc).unwrap_or(0) }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_insn_count() -> u64 {
    unsafe { USER.as_ref().map(|e| e.machine.cpu.insn_count).unwrap_or(0) }
}

// ---- full-system API (boot Linux in the browser) --------------------------

static mut SYS_BIOS: Vec<u8> = Vec::new();
static mut SYS_KERNEL: Vec<u8> = Vec::new();
static mut SYS_DISK: Vec<u8> = Vec::new();
static mut SYS_CMDLINE: Vec<u8> = Vec::new();
/// In-process HTTP proxy: the guest's NIC talks to this instead of a relay, and
/// egress happens through the page's `fetch()`. This is the only configuration
/// that reaches the network with no external infrastructure at all.
static mut SYS_NETSTACK: Option<rv64_system::netstack::NetStack> = None;
static mut SYS_PROXY: Option<rv64_system::httpproxy::Proxy> = None;
static mut SYS_WISP: bool = false;
static mut SYS_EGRESS: FetchEgress = FetchEgress { done: Vec::new() };

/// Hands requests to the page and collects what the `sys_http_*` exports
/// deliver. Responses arrive as a head then body chunks, so a streaming
/// response (SSE, a long download) reaches the guest as it arrives.
struct FetchEgress {
    done: Vec<rv64_system::httpproxy::Completion>,
}

impl rv64_system::httpproxy::Egress for FetchEgress {
    fn submit(&mut self, id: rv64_system::httpproxy::ReqId, req: rv64_system::httpproxy::Request) {
        let bytes = req.encode();
        unsafe { host_http_request(id, bytes.as_ptr(), bytes.len()) }
    }
    fn poll(&mut self) -> Vec<rv64_system::httpproxy::Completion> {
        core::mem::take(&mut self.done)
    }
}

/// Optional 6-byte MAC for the NIC; empty means use the crate default.
static mut SYS_NET_MAC: Vec<u8> = Vec::new();
/// Whether sys_boot should give the machine a virtio-net device.
static mut SYS_NET_ON: bool = false;
/// tar archive staged for the virtio-9p export (see `sys_stage_fs_tar`).
static mut SYS_FS_TAR: Vec<u8> = Vec::new();
/// Mount tag the 9p export answers to; the guest mounts this name.
static mut SYS_FS_TAG: Vec<u8> = Vec::new();
static mut SYS: Option<rv64_system::Machine> = None;

macro_rules! stage_into {
    ($name:ident, $slot:ident) => {
        #[no_mangle]
        #[allow(static_mut_refs)]
        pub extern "C" fn $name() {
            unsafe {
                $slot = core::mem::take(&mut STAGING);
            }
        }
    };
}

stage_into!(sys_stage_bios, SYS_BIOS);
stage_into!(sys_stage_kernel, SYS_KERNEL);
stage_into!(sys_stage_disk, SYS_DISK);
stage_into!(sys_stage_cmdline, SYS_CMDLINE);
// Stage a tar archive to export over virtio-9p. There is no host filesystem in
// the browser, so the export is an in-memory tree built from a tarball the page
// fetched — mount it in the guest with
// `mount -t 9p -o trans=virtio,version=9p2000.L <tag> /mnt`.
stage_into!(sys_stage_fs_tar, SYS_FS_TAR);
stage_into!(sys_stage_fs_tag, SYS_FS_TAG);
// Optional 6-byte MAC override for the NIC.
stage_into!(sys_stage_net_mac, SYS_NET_MAC);

// ---- modern virt-machine API (OpenSBI + current Linux) -------------------

static mut VIRT_OPENSBI: Vec<u8> = Vec::new();
static mut VIRT_KERNEL: Vec<u8> = Vec::new();
static mut VIRT_INITRD: Vec<u8> = Vec::new();
static mut VIRT_DISK: Vec<u8> = Vec::new();
static mut VIRT_CMDLINE: Vec<u8> = Vec::new();
static mut VIRT_NET_ON: bool = false;
static mut VIRT_NET_MAC: Vec<u8> = Vec::new();
static mut VIRT_FS_EXTERNAL_TAG: Vec<u8> = Vec::new();
static mut VIRT_CONSOLE_ON: bool = false;
static mut VIRT_LAST_MONOTONIC_MS: f64 = 0.0;

/// Charge elapsed host-monotonic time to the modern Virt clock. Generated
/// execution can service more than one interrupt quantum inside a single
/// public `virt_run` call, so this must run at those internal boundaries too;
/// otherwise `sync_devices` repeatedly re-anchors `rdtime` to one stale host
/// sample and Linux's scheduler clock can move backwards.
fn advance_virt_realtime(machine: &mut rv64_system::virt::VirtMachine) {
    let now = unsafe { host_now_ms() };
    let elapsed_ms = unsafe { (now - VIRT_LAST_MONOTONIC_MS).max(0.0) };
    unsafe { VIRT_LAST_MONOTONIC_MS = now };
    machine.advance_realtime_ns((elapsed_ms * 1_000_000.0) as u64);
}
static mut VIRT: Option<rv64_system::virt::VirtMachine> = None;

// ---- opt-in interpreter trace for offline JIT policy research -----------

/// Sparse heat-event resolution. Per-page totals remain exact; only event
/// timing is quantized, keeping a full Linux trace compact enough to export.
const POLICY_TRACE_QUANTUM: u64 = 16_384;
const POLICY_TRACE_EVENT_CAP: usize = 1_000_000;
const POLICY_TRACE_SCHEMA: u64 = 2;

#[derive(Default)]
struct PolicyTraceContext {
    vpage: u64,
    satp: u64,
    mode: u8,
    total: u64,
    first: u64,
    last: u64,
    unique_pcs: u32,
    unique_entries: u32,
    executed: Option<Box<[u64; 32]>>,
    entries: Option<Box<[u64; 32]>>,
}

#[derive(Default)]
struct PolicyTracePage {
    total: u64,
    first: u64,
    last: u64,
    unique_pcs: u32,
    unique_entries: u32,
    transfers: u64,
    backedges: u64,
    cross_page_exits: u64,
    executed: Option<Box<[u64; 32]>>,
    entries: Option<Box<[u64; 32]>>,
    contexts: Vec<PolicyTraceContext>,
}

#[derive(Clone, Copy)]
struct PolicyTraceEvent {
    at: u64,
    page: u32,
    page_heat: u64,
    context_heat: u64,
    va: u64,
    satp: u64,
    mode: u8,
    kind: u8,
}

#[derive(Clone, Copy)]
struct PolicyTracePrevious {
    va: u64,
    next_va: u64,
    satp: u64,
    mode: u8,
    ilen: u8,
}

struct PolicyTrace {
    pages: Vec<PolicyTracePage>,
    touched: Vec<u32>,
    contexts: Vec<(u32, u32)>,
    events: Vec<PolicyTraceEvent>,
    origin: u64,
    last: u64,
    observed: u64,
    outside_ram: u64,
    dropped_events: u64,
    started: bool,
    previous: Option<PolicyTracePrevious>,
}

impl PolicyTrace {
    fn new(ram_pages: usize) -> Self {
        let mut pages = Vec::with_capacity(ram_pages);
        pages.resize_with(ram_pages, PolicyTracePage::default);
        Self {
            pages,
            touched: Vec::new(),
            contexts: Vec::new(),
            events: Vec::new(),
            origin: 0,
            last: 0,
            observed: 0,
            outside_ram: 0,
            dropped_events: 0,
            started: false,
            previous: None,
        }
    }

    fn mark(bitmap: &mut Option<Box<[u64; 32]>>, slot: usize) -> bool {
        let words = bitmap.get_or_insert_with(|| Box::new([0; 32]));
        let word = slot >> 6;
        let bit = 1u64 << (slot & 63);
        let fresh = words[word] & bit == 0;
        words[word] |= bit;
        fresh
    }

    fn record(&mut self, insn: rv64_core::InstructionTrace) {
        if !self.started {
            self.origin = insn.icount.saturating_sub(1);
            self.started = true;
        }
        self.last = insn.icount;
        self.observed += 1;
        let at = insn.icount.wrapping_sub(self.origin);

        // A new entry is the first observed instruction, the destination of a
        // non-fallthrough transfer, an exception discontinuity, or execution
        // after an address-space/privilege transition.
        let is_entry = self.previous.map_or(true, |prev| {
            prev.satp != insn.satp
                || prev.mode != insn.mode
                || prev.next_va != insn.va
                || prev.ilen == 0
                || prev.next_va != prev.va.wrapping_add(u64::from(prev.ilen))
        });
        self.previous = Some(PolicyTracePrevious {
            va: insn.va,
            next_va: insn.next_va,
            satp: insn.satp,
            mode: insn.mode,
            ilen: insn.ilen,
        });

        const BASE: u64 = rv64_system::virt::RAM_BASE;
        if insn.pa < BASE {
            self.outside_ram += 1;
            return;
        }
        let page_index = ((insn.pa - BASE) >> 12) as usize;
        if page_index >= self.pages.len() {
            self.outside_ram += 1;
            return;
        }

        if self.pages[page_index].total == 0 {
            self.touched.push(page_index as u32);
        }
        let pc_slot = ((insn.pa & 0xfff) >> 1) as usize;
        let vpage = insn.va & !0xfff;
        let (new_context, local_context, event) = {
            let page = &mut self.pages[page_index];
            page.total += 1;
            if page.first == 0 {
                page.first = at;
            }
            page.last = at;

            if Self::mark(&mut page.executed, pc_slot) {
                page.unique_pcs += 1;
            }
            if is_entry && Self::mark(&mut page.entries, pc_slot) {
                page.unique_entries += 1;
            }

            let fallthrough = insn.va.wrapping_add(u64::from(insn.ilen));
            if insn.ilen != 0 && insn.next_va != fallthrough {
                page.transfers += 1;
                if insn.next_va <= insn.va && (insn.next_va >> 12) == (insn.va >> 12) {
                    page.backedges += 1;
                }
            }
            if (insn.next_va >> 12) != (insn.va >> 12) {
                page.cross_page_exits += 1;
            }

            let (new_context, local_context) = match page.contexts.iter().position(|context| {
                context.vpage == vpage && context.satp == insn.satp && context.mode == insn.mode
            }) {
                Some(index) => (false, index),
                None => {
                    page.contexts.push(PolicyTraceContext {
                        vpage,
                        satp: insn.satp,
                        mode: insn.mode,
                        ..PolicyTraceContext::default()
                    });
                    (true, page.contexts.len() - 1)
                }
            };
            let context = &mut page.contexts[local_context];
            context.total += 1;
            if context.first == 0 {
                context.first = at;
            }
            context.last = at;
            if Self::mark(&mut context.executed, pc_slot) {
                context.unique_pcs += 1;
            }
            if is_entry && Self::mark(&mut context.entries, pc_slot) {
                context.unique_entries += 1;
            }

            let page_due = page.total % POLICY_TRACE_QUANTUM == 0;
            let context_due = context.total % POLICY_TRACE_QUANTUM == 0;
            let event = (page_due || context_due).then_some(PolicyTraceEvent {
                at,
                page: page_index as u32,
                page_heat: page.total,
                context_heat: context.total,
                va: insn.va,
                satp: insn.satp,
                mode: insn.mode,
                kind: page_due as u8 | ((context_due as u8) << 1),
            });
            (new_context, local_context, event)
        };
        if new_context {
            self.contexts
                .push((page_index as u32, local_context as u32));
        }
        if let Some(event) = event {
            if self.events.len() < POLICY_TRACE_EVENT_CAP {
                self.events.push(event);
            } else {
                self.dropped_events += 1;
            }
        }
    }
}

static mut POLICY_TRACE_ENABLED: bool = false;
static mut POLICY_TRACE: Option<PolicyTrace> = None;

/// Enable collection on the JIT-disabled modern virt interpreter. Enabling
/// starts a fresh capture; disabling retains it for extraction.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn policy_trace_set_enabled(on: u32) {
    unsafe {
        let enabled = on != 0;
        if enabled && !POLICY_TRACE_ENABLED {
            let pages = VIRT.as_ref().map_or(0, |m| m.bus.ram.len() >> 12);
            POLICY_TRACE = Some(PolicyTrace::new(pages));
        }
        POLICY_TRACE_ENABLED = enabled;
    }
}

/// Clear captured data without changing whether collection is enabled.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn policy_trace_reset() {
    unsafe {
        let pages = VIRT.as_ref().map_or(0, |m| m.bus.ram.len() >> 12);
        POLICY_TRACE = Some(PolicyTrace::new(pages));
    }
}

/// Trace metadata by field: schema, enabled, origin/last absolute icount,
/// observed instructions, touched pages, events, dropped events, out-of-RAM
/// instructions, event quantum, RAM base/pages, event cap, and contexts.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn policy_trace_meta(which: u32) -> u64 {
    unsafe {
        let trace = POLICY_TRACE.as_ref();
        match which {
            0 => POLICY_TRACE_SCHEMA,
            1 => POLICY_TRACE_ENABLED as u64,
            2 => trace.map_or(0, |t| t.origin),
            3 => trace.map_or(0, |t| t.last),
            4 => trace.map_or(0, |t| t.observed),
            5 => trace.map_or(0, |t| t.touched.len() as u64),
            6 => trace.map_or(0, |t| t.events.len() as u64),
            7 => trace.map_or(0, |t| t.dropped_events),
            8 => trace.map_or(0, |t| t.outside_ram),
            9 => POLICY_TRACE_QUANTUM,
            10 => rv64_system::virt::RAM_BASE,
            11 => trace.map_or(0, |t| t.pages.len() as u64),
            12 => POLICY_TRACE_EVENT_CAP as u64,
            13 => trace.map_or(0, |t| t.contexts.len() as u64),
            _ => 0,
        }
    }
}

/// Per execution-context fields: physical address/page, virtual page, SATP,
/// mode, exact heat, first/last relative icount, unique PCs/entries, and heat
/// event count. A context is the architecture-correct unit for studying how
/// much SATP keying fragments otherwise shareable physical code.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn policy_trace_context_stat(index: u32, which: u32) -> u64 {
    unsafe {
        let Some(trace) = POLICY_TRACE.as_ref() else {
            return 0;
        };
        let Some(&(dense, local)) = trace.contexts.get(index as usize) else {
            return 0;
        };
        let context = &trace.pages[dense as usize].contexts[local as usize];
        match which {
            0 => rv64_system::virt::RAM_BASE + (u64::from(dense) << 12),
            1 => u64::from(dense),
            2 => context.vpage,
            3 => context.satp,
            4 => u64::from(context.mode),
            5 => context.total,
            6 => context.first,
            7 => context.last,
            8 => u64::from(context.unique_pcs),
            9 => u64::from(context.unique_entries),
            10 => context.total / POLICY_TRACE_QUANTUM,
            _ => 0,
        }
    }
}

/// Per-touched-page fields: physical address, dense page index, exact heat,
/// first/last relative icount, unique PCs/entries, transfers, backedges,
/// cross-page exits, and emitted heat-event count.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn policy_trace_page_stat(index: u32, which: u32) -> u64 {
    unsafe {
        let Some(trace) = POLICY_TRACE.as_ref() else {
            return 0;
        };
        let Some(&dense) = trace.touched.get(index as usize) else {
            return 0;
        };
        let page = &trace.pages[dense as usize];
        match which {
            0 => rv64_system::virt::RAM_BASE + (u64::from(dense) << 12),
            1 => u64::from(dense),
            2 => page.total,
            3 => page.first,
            4 => page.last,
            5 => u64::from(page.unique_pcs),
            6 => u64::from(page.unique_entries),
            7 => page.transfers,
            8 => page.backedges,
            9 => page.cross_page_exits,
            10 => page.total / POLICY_TRACE_QUANTUM,
            _ => 0,
        }
    }
}

/// Ordered heat-event fields: relative global icount, physical address/page,
/// page/context heat, VA/context, and event-kind bits (1=physical page heat
/// quantum, 2=full execution-context heat quantum).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn policy_trace_event_stat(index: u32, which: u32) -> u64 {
    unsafe {
        let Some(event) = POLICY_TRACE
            .as_ref()
            .and_then(|trace| trace.events.get(index as usize))
        else {
            return 0;
        };
        match which {
            0 => event.at,
            1 => rv64_system::virt::RAM_BASE + (u64::from(event.page) << 12),
            2 => u64::from(event.page),
            3 => event.page_heat,
            4 => event.context_heat,
            5 => event.va,
            6 => event.va & !0xfff,
            7 => event.satp,
            8 => u64::from(event.mode),
            9 => u64::from(event.kind),
            _ => 0,
        }
    }
}

stage_into!(virt_stage_opensbi, VIRT_OPENSBI);
stage_into!(virt_stage_kernel, VIRT_KERNEL);
stage_into!(virt_stage_initrd, VIRT_INITRD);
stage_into!(virt_stage_disk, VIRT_DISK);
stage_into!(virt_stage_cmdline, VIRT_CMDLINE);
stage_into!(virt_stage_net_mac, VIRT_NET_MAC);
stage_into!(virt_stage_fs_external_tag, VIRT_FS_EXTERNAL_TAG);

/// Give the next modern virt machine a virtio-net NIC.
#[no_mangle]
pub extern "C" fn virt_net_enable(on: u32) {
    unsafe { VIRT_NET_ON = on != 0 }
}

#[no_mangle]
pub extern "C" fn virt_console_enable(on: u32) {
    unsafe { VIRT_CONSOLE_ON = on != 0 }
}

/// Assemble and boot the modern virt machine from staged images.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_boot(ram_mb: u32) {
    boot_virt(ram_mb, false);
}

/// Assemble the modern virt machine and enter Linux directly in S-mode.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_boot_direct(ram_mb: u32) {
    boot_virt(ram_mb, true);
}

#[allow(static_mut_refs)]
fn boot_virt(ram_mb: u32, direct: bool) {
    unsafe {
        let cmdline = String::from_utf8_lossy(&VIRT_CMDLINE).into_owned();
        let cmdline = if cmdline.is_empty() {
            "console=ttyS0 root=/dev/vda rw"
        } else {
            &cmdline
        };
        let mut fs = Vec::new();
        if let Some(proxy) = SYS_PROXY.as_mut() {
            if let Ok(ca_fs) = proxy.ca_9p_server() {
                fs.push(ca_fs);
            }
        }
        let net = VIRT_NET_ON.then(|| {
            <[u8; 6]>::try_from(VIRT_NET_MAC.as_slice()).unwrap_or(rv64_system::virtio::DEFAULT_MAC)
        });
        let images = rv64_system::virt::VirtImages {
            opensbi: &VIRT_OPENSBI,
            kernel: &VIRT_KERNEL,
            cmdline,
            initrd: (!VIRT_INITRD.is_empty()).then_some(VIRT_INITRD.as_slice()),
            disk: (!VIRT_DISK.is_empty()).then(|| core::mem::take(&mut VIRT_DISK)),
            fs,
            external_fs: (!VIRT_FS_EXTERNAL_TAG.is_empty())
                .then(|| core::str::from_utf8(&VIRT_FS_EXTERNAL_TAG).unwrap_or("host")),
            virtio_console: VIRT_CONSOLE_ON,
            net,
        };
        let mut machine = if direct {
            rv64_system::virt::VirtMachine::new_direct(u64::from(ram_mb) << 20, images)
        } else {
            rv64_system::virt::VirtMachine::new(u64::from(ram_mb) << 20, images)
        };
        machine.set_rtc_unix_ns(host_unix_ms() as u64 * 1_000_000);
        if POLICY_TRACE_ENABLED {
            POLICY_TRACE = Some(PolicyTrace::new(machine.bus.ram.len() >> 12));
        }
        VIRT_OPENSBI.clear();
        VIRT_KERNEL.clear();
        VIRT_INITRD.clear();
        VIRT_CMDLINE.clear();
        VIRT_FS_EXTERNAL_TAG.clear();
        VIRT = Some(machine);
        VIRT_LAST_MONOTONIC_MS = host_now_ms();
        ACTIVE_SYSTEM_KIND = SystemMachineKind::Virt as u8;
        ACTIVE_SYSTEM_STATE = VIRT
            .as_mut()
            .map_or(0, |machine| machine as *mut _ as usize as u32);
        BOOT_GEN += 1;
        PENDING_SB.clear();
        if let Some(jit) = SYS_JIT.as_mut() {
            jit.clear();
        }
        JIT_RETIRED = 0;
        JIT_DISPATCHES = 0;
        JIT_TIMER_IRQ_ORIGIN = [0; 4];
        CHAIN_DEPTH = 0;
        CHAIN_HOPS = 0;
        SLICE_CALLS = 0;
        SLICE_INSNS = 0;
        SYS_TRANSLATE_NS = 0;
        SYS_TRANSLATE_ATTEMPTS = 0;
        SYS_EMITTED_BYTES = 0;
        SYS_DENSE_COPY_MEMBERS = 0;
        SYS_DENSE_STORE_MEMBERS = 0;
        SYS_BULK_COPY_MEMBERS = 0;
        SYS_BULK_COPY_DIAG = [0; 6];
        reset_chain_runtime();
        reset_page_policy_stats();
    }
}

/// Run one modern-machine slice and stream UART output to the host.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_run(max_insns: u64) -> i32 {
    let machine = unsafe { VIRT.as_mut().expect("call virt_boot() first") };
    advance_virt_realtime(machine);
    machine.set_rtc_unix_ns(unsafe { host_unix_ms() } as u64 * 1_000_000);
    if !unsafe { JIT_ENABLED } {
        if unsafe { POLICY_TRACE_ENABLED } {
            return run_virt_interpreter_traced(machine, max_insns);
        }
        return run_system_interpreter(machine, max_insns);
    }
    let origin = machine.cpu.sys.as_ref().map_or(0, |sys| sys.mode as usize);
    let timer_before = machine.cpu.irq_counts[5];
    machine.sync_devices();
    machine.cpu.check_interrupts(&mut machine.bus);
    let delivered = machine.cpu.irq_counts[5].wrapping_sub(timer_before);
    if delivered != 0 {
        unsafe { JIT_TIMER_IRQ_ORIGIN[origin] += delivered }
    }
    run_system_jit(machine, max_insns)
}

#[allow(static_mut_refs)]
fn run_virt_interpreter_traced(
    machine: &mut rv64_system::virt::VirtMachine,
    max_insns: u64,
) -> i32 {
    let mut record = |insn| unsafe {
        if let Some(trace) = POLICY_TRACE.as_mut() {
            trace.record(insn);
        }
    };
    machine.run_slice_traced(max_insns, &mut record);
    machine.flush_host_io();
    machine.power_off as i32
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_console_input() {
    let machine = unsafe { VIRT.as_mut().expect("call virt_boot() first") };
    let bytes = unsafe { core::mem::take(&mut STAGING) };
    machine.console_input(&bytes);
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_export_input() {
    let machine = unsafe { VIRT.as_mut().expect("call virt_boot() first") };
    let bytes = unsafe { core::mem::take(&mut STAGING) };
    machine.virtio_console_input(&bytes);
}

/// Move the next external 9P request into STAGING, returning its byte length.
/// Zero means that no request is waiting for the host.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_p9_take_request() -> u32 {
    unsafe {
        let Some(machine) = VIRT.as_mut() else {
            return 0;
        };
        let Some(request) = machine.fs_external_take_request() else {
            return 0;
        };
        STAGING = request;
        STAGING.len() as u32
    }
}

/// Deliver the staged reply to the external virtio-9P device.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_p9_reply() -> u32 {
    unsafe {
        let reply = core::mem::take(&mut STAGING);
        VIRT.as_mut()
            .is_some_and(|machine| machine.fs_external_reply(reply)) as u32
    }
}

/// Deliver one inbound Ethernet frame to the modern machine's NIC.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_net_input() {
    let machine = unsafe { VIRT.as_mut().expect("call virt_boot() first") };
    let frame = unsafe { core::mem::take(&mut STAGING) };
    machine.net_input(&frame);
}

#[allow(static_mut_refs)]
fn pump_virt_net(machine: &mut rv64_system::virt::VirtMachine) {
    unsafe {
        match SYS_NETSTACK.as_mut() {
            Some(stack) => {
                for frame in machine.net_take_output() {
                    stack.input(&frame);
                }
                if let Some(proxy) = SYS_PROXY.as_mut() {
                    proxy.pump(stack, &mut SYS_EGRESS);
                } else if SYS_WISP {
                    pump_wisp(stack);
                }
                for frame in stack.take_output() {
                    machine.net_input(&frame);
                }
            }
            None => {
                for frame in machine.net_take_output() {
                    host_net_send(frame.as_ptr(), frame.len())
                }
            }
        }
    }
}

#[allow(static_mut_refs)]
fn pump_wisp(stack: &mut rv64_system::netstack::NetStack) {
    for event in stack.take_events() {
        match event {
            rv64_system::netstack::Event::Opened { id, address, port } => unsafe {
                host_wisp_open(id, address.as_ptr(), u32::from(port));
            },
            rv64_system::netstack::Event::Data(id, bytes) => unsafe {
                host_wisp_data(id, bytes.as_ptr(), bytes.len());
            },
            rv64_system::netstack::Event::Closed(id) => unsafe { host_wisp_close(id) },
            rv64_system::netstack::Event::Datagram {
                id,
                address,
                port,
                bytes,
            } => unsafe {
                host_wisp_datagram(
                    id,
                    address.as_ptr(),
                    u32::from(port),
                    bytes.as_ptr(),
                    bytes.len(),
                );
            },
        }
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_insn_count() -> u64 {
    unsafe { VIRT.as_ref().map(|m| m.cpu.insn_count).unwrap_or(0) }
}

/// Diagnostic direct-SBI call counter. Indexes are total, BASE, TIME, IPI,
/// RFENCE, HSM, SRST, and legacy/other.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_sbi_call_count(index: u32) -> u64 {
    unsafe {
        VIRT.as_ref()
            .and_then(|m| m.sbi_calls.get(index as usize))
            .copied()
            .unwrap_or(0)
    }
}

/// Current modern-machine guest PC (diagnostic: boot and workload profiling).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_pc() -> u64 {
    unsafe { VIRT.as_ref().map_or(0, |m| m.cpu.pc) }
}

/// Read-only modern-machine architectural state for differential harnesses.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_reg(index: u32) -> u64 {
    unsafe {
        VIRT.as_ref()
            .and_then(|machine| machine.cpu.x.get(index as usize).copied())
            .unwrap_or(0)
    }
}

/// Read a little-endian u64 from modern-machine physical RAM. Invalid and
/// MMIO addresses return zero; this export is diagnostic-only.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_ram_u64(physical_address: u64) -> u64 {
    unsafe {
        let Some(machine) = VIRT.as_ref() else {
            return 0;
        };
        let Some(offset) = physical_address
            .checked_sub(rv64_system::virt::RAM_BASE)
            .and_then(|offset| usize::try_from(offset).ok())
        else {
            return 0;
        };
        let Some(bytes) = machine
            .bus
            .ram
            .get(offset..offset.saturating_add(core::mem::size_of::<u64>()))
        else {
            return 0;
        };
        let Ok(bytes) = <[u8; 8]>::try_from(bytes) else {
            return 0;
        };
        u64::from_le_bytes(bytes)
    }
}

/// Architectural state captured immediately before the latest supervisor
/// timer trap delivered from user mode. See `Cpu::last_timer_trap` for fields.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_last_timer_trap(index: u32) -> u64 {
    unsafe {
        VIRT.as_ref()
            .map_or(0, |m| m.cpu.last_timer_trap[index as usize % 13])
    }
}

/// Unsupported direct-boot SBI extension/function, or zero when none.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_unsupported_sbi_ext() -> u64 {
    unsafe {
        VIRT.as_ref()
            .and_then(|m| m.unsupported_sbi)
            .map_or(0, |v| v.0)
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_unsupported_sbi_function() -> u64 {
    unsafe {
        VIRT.as_ref()
            .and_then(|m| m.unsupported_sbi)
            .map_or(0, |v| v.1)
    }
}

/// Give the next-booted machine a virtio-net NIC. Frames the guest sends arrive
/// via the `host_net_send` import; feed inbound frames back with
/// `sys_net_input`. The page supplies the transport (a WebSocket to a relay) —
/// the emulator only moves layer-2 frames.
#[no_mangle]
pub extern "C" fn sys_net_enable(on: u32) {
    unsafe { SYS_NET_ON = on != 0 }
}

/// Run the in-process HTTP proxy behind the NIC (implies `sys_net_enable`).
/// Frames then go to the built-in netstack rather than out `host_net_send`.
///
/// `upgrade_https` rewrites the guest's `http://` targets to `https://` on
/// egress, which a page served over https requires — it cannot fetch http:// at
/// all. Pass 0 only when egress genuinely wants plaintext (a localhost server,
/// or a page served over http).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_proxy_enable(on: u32, upgrade_https: u32) {
    unsafe {
        if on != 0 {
            SYS_WISP = false;
            SYS_NET_ON = true;
            VIRT_NET_ON = true;
            SYS_NETSTACK = Some(rv64_system::netstack::NetStack::new(
                rv64_system::netstack::NetConfig::default(),
            ));
            let proxy = rv64_system::httpproxy::Proxy::new();
            SYS_PROXY = Some(if upgrade_https != 0 {
                proxy
            } else {
                proxy.keep_scheme()
            });
        } else {
            SYS_NETSTACK = None;
            SYS_PROXY = None;
        }
    }
}

/// Run a transparent TCP stack behind the NIC for the JavaScript WISP client.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_wisp_enable(on: u32) {
    unsafe {
        SYS_WISP = on != 0;
        if SYS_WISP {
            SYS_NET_ON = true;
            VIRT_NET_ON = true;
            let cfg = rv64_system::netstack::NetConfig {
                transparent: true,
                ..rv64_system::netstack::NetConfig::default()
            };
            SYS_NETSTACK = Some(rv64_system::netstack::NetStack::new(cfg));
            SYS_PROXY = None;
        } else if SYS_PROXY.is_none() {
            SYS_NETSTACK = None;
        }
    }
}

/// Deliver bytes received from a WISP stream (bytes staged first).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_wisp_data(id: u64) {
    unsafe {
        if let Some(stack) = SYS_NETSTACK.as_mut() {
            let bytes = core::mem::take(&mut STAGING);
            stack.send(id, &bytes);
        }
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_wisp_close(id: u64) {
    unsafe {
        if let Some(stack) = SYS_NETSTACK.as_mut() {
            stack.close(id);
        }
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_wisp_datagram(id: u64) {
    unsafe {
        if let Some(stack) = SYS_NETSTACK.as_mut() {
            let bytes = core::mem::take(&mut STAGING);
            stack.send_udp(id, &bytes);
        }
    }
}

/// Deliver a response head (staged via staging_alloc) for request `id`.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_http_head(id: u64) {
    unsafe {
        let bytes = core::mem::take(&mut STAGING);
        match rv64_system::httpproxy::decode_head(&bytes) {
            Some((status, headers)) => {
                SYS_EGRESS
                    .done
                    .push(rv64_system::httpproxy::Completion::Head {
                        id,
                        status,
                        headers,
                    })
            }
            None => SYS_EGRESS
                .done
                .push(rv64_system::httpproxy::Completion::Failed {
                    id,
                    error: "malformed response head from host".into(),
                }),
        }
    }
}

/// Deliver a chunk of response body (staged via staging_alloc) for `id`.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_http_body(id: u64) {
    unsafe {
        let bytes = core::mem::take(&mut STAGING);
        SYS_EGRESS
            .done
            .push(rv64_system::httpproxy::Completion::Body { id, bytes });
    }
}

/// The response for `id` is complete.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_http_end(id: u64) {
    unsafe {
        SYS_EGRESS
            .done
            .push(rv64_system::httpproxy::Completion::End { id });
    }
}

/// The request `id` could not be performed; STAGING holds why.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_http_fail(id: u64) {
    unsafe {
        let bytes = core::mem::take(&mut STAGING);
        SYS_EGRESS
            .done
            .push(rv64_system::httpproxy::Completion::Failed {
                id,
                error: String::from_utf8_lossy(&bytes).into_owned(),
            });
    }
}

/// The `http_proxy` URL the guest should use, written into STAGING; returns its
/// length so the page can show it without hardcoding the address.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_proxy_url() -> u32 {
    unsafe {
        let url = SYS_NETSTACK
            .as_ref()
            .map(|s| s.proxy_url())
            .unwrap_or_default();
        STAGING = url.into_bytes();
        STAGING.len() as u32
    }
}

/// Assemble and boot the machine from the staged images.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_boot(ram_mb: u32) {
    unsafe {
        let cmdline = String::from_utf8_lossy(&SYS_CMDLINE).into_owned();
        let cmdline = if cmdline.is_empty() {
            "console=hvc0 root=/dev/vda rw"
        } else {
            &cmdline
        };
        let mut fs = Vec::new();
        if !SYS_FS_TAR.is_empty() {
            let tag = String::from_utf8_lossy(&SYS_FS_TAG).into_owned();
            let tag = if tag.is_empty() { "host".into() } else { tag };
            let mut mem = rv64_system::p9fs::MemFs::new();
            mem.load_tar(&core::mem::take(&mut SYS_FS_TAR));
            fs.push(rv64_system::p9::Server::new(tag, Box::new(mem)));
        }
        // The guest can trust the exact ephemeral authority owned by this
        // proxy without fetching it over the network. Only its public
        // certificate is exposed; private signing material stays in Rust.
        if let Some(proxy) = SYS_PROXY.as_mut() {
            if let Ok(ca_fs) = proxy.ca_9p_server() {
                fs.push(ca_fs);
            }
        }
        let net = SYS_NET_ON.then(|| {
            <[u8; 6]>::try_from(SYS_NET_MAC.as_slice()).unwrap_or(rv64_system::virtio::DEFAULT_MAC)
        });
        let mut m = rv64_system::Machine::new(
            ram_mb as usize,
            rv64_system::BootImages {
                bios: &SYS_BIOS,
                kernel: if SYS_KERNEL.is_empty() {
                    None
                } else {
                    Some(&SYS_KERNEL)
                },
                cmdline,
                disk: if SYS_DISK.is_empty() {
                    None
                } else {
                    Some(core::mem::take(&mut SYS_DISK))
                },
                fs,
                net,
            },
        );
        m.set_rtc_unix_ns(host_unix_ms() as u64 * 1_000_000);
        SYS_BIOS = Vec::new();
        SYS_KERNEL = Vec::new();
        SYS = Some(m);
        ACTIVE_SYSTEM_KIND = SystemMachineKind::Legacy as u8;
        ACTIVE_SYSTEM_STATE = SYS
            .as_mut()
            .map_or(0, |machine| machine as *mut _ as usize as u32);
        // A new machine means every compiled block and stat is stale — a
        // second boot in the same wasm instance must never execute code
        // generated from the previous guest (PERFORMANCE_PROGRESS.md cache lifecycle).
        BOOT_GEN += 1;
        PENDING_SB.clear();
        if let Some(j) = SYS_JIT.as_mut() {
            j.clear();
        }
        JIT_RETIRED = 0;
        JIT_DISPATCHES = 0;
        JIT_TIMER_IRQ_ORIGIN = [0; 4];
        CHAIN_DEPTH = 0;
        CHAIN_HOPS = 0;
        SLICE_CALLS = 0;
        SLICE_INSNS = 0;
        SYS_TRANSLATE_NS = 0;
        SYS_TRANSLATE_ATTEMPTS = 0;
        SYS_EMITTED_BYTES = 0;
        SYS_DENSE_COPY_MEMBERS = 0;
        SYS_DENSE_STORE_MEMBERS = 0;
        SYS_BULK_COPY_MEMBERS = 0;
        SYS_BULK_COPY_DIAG = [0; 6];
        reset_chain_runtime();
    }
}

/// The JIT's view of machine state (register file, fcsr, TLB tables, budget
/// cells) — identical for every translation of the current machine.
fn jit_system_memory(cpu: &rv64_core::Cpu) -> rv64_dbt::SystemMemory {
    let hashed_tlb = unsafe { JIT_TLB_HASH };
    let (lt, lo, st, so) = cpu.jit_ftlb_ptrs(hashed_tlb);
    rv64_dbt::SystemMemory::fused_4k(
        lt as u32,
        lo as u32,
        st as u32,
        so as u32,
        cpu.jit_tlb_context_ptr() as u32,
        (rv64_core::Cpu::jit_tlb_size() - 1) as u32,
        rv64_dbt::tlb_fill_enabled(),
    )
    .with_index_hash_shift(if hashed_tlb {
        rv64_core::Cpu::jit_tlb_hash_shift()
    } else {
        0
    })
}

fn jit_layout(m: &impl SystemJitMachine) -> rv64_dbt::JitLayout {
    let cpu = m.cpu();
    let memory = jit_system_memory(cpu)
        .with_invocation_cache(unsafe { REGION_TLB_CACHE })
        .with_invocation_cache_min_accesses(unsafe { REGION_TLB_CACHE_MIN_ACCESSES });
    rv64_dbt::JitLayout {
        x_base: cpu.x.as_ptr() as u32,
        pc_addr: &cpu.pc as *const u64 as u32,
        mem: None,
        sys: Some(memory),
        mem_profile: mem_profile_layout(),
        reg_stress: reg_stress(),
        reg_profile_base: reg_profile_base(),
        structured_profile: structured_profile_layout(),
        multi_latch: unsafe { MULTI_LATCH },
        retired_addr: retired_addr(),
        f_base: cpu.f.as_ptr() as u32,
        fcsr_addr: &cpu.fcsr as *const u32 as u32,
        reservation: Some(rv64_dbt::ReservationCapability::System),
        fuel_addr: fuel_addr(),
        mstatus_addr: cpu.jit_mstatus_ptr() as u32,
        copystat_addr: copystat_addr(),
        chain_off_addr: chain_off_addr(),
        batch_base_addr: 0,
        dispatch_base: 0,
        dispatch_mask: 0,
        map_gen_addr: 0,
        chain_hops_addr: chain_hops_addr(),
        ic_miss_owner_addr: ic_miss_owner_addr(),
        ic_miss_target_addr: ic_miss_target_addr(),
        pic_code_base: None,
    }
}

/// Build (asynchronously) the superblock covering `vpage` in address space
/// `aspace`, whose current physical page is `pa_page`. Returns true if a
/// module was issued. Called from the compile path and, when the compile
/// budget deferred one, from the quantum boundary.
#[allow(static_mut_refs)]
fn build_superblock<M: SystemJitMachine>(
    m: &mut M,
    jit: &mut JitState,
    aspace: u64,
    vpage: u64,
    pa_page: u64,
    sb_compiles: u32,
) -> bool {
    let n_entries = jit
        .page_entries
        .get(&(aspace, vpage))
        .map_or(0, |e| e.len());
    // Enough individually-hot pcs: compile the page as
    // ONE function — but over the FULL statically
    // discovered leader set (v86's page analysis), not
    // just the hot seeds. That keeps intra-page control
    // flow inside the function (any discovered target
    // hits its br_table slot) without recompiling per
    // newly-hot entry. Loop headers are EXCLUDED: their
    // br_table slots fall to the exit default, so the
    // tight individual loop-region blocks keep owning
    // them.
    unsafe { SB_TRIGGER += 1 };
    // Assemble the region: the hot page plus its
    // virtually contiguous, RAM-backed neighbours, so
    // control flow that leaves the page still lands
    // inside the same wasm function.
    let ram_ok = |m: &M, pa: u64| {
        pa >= rv64_system::RAM_BASE
            && ((pa & !0xfff) - rv64_system::RAM_BASE) as usize + 0x1000 <= m.ram().len()
    };
    // Assemble the region from the CALL GRAPH, not from
    // address adjacency: pages join when the code already
    // in the region calls into them and they are hot
    // (page_entries non-empty), plus the contiguous next/
    // previous page when hot code sits within a block's
    // reach of the shared edge (loops straddle page
    // boundaries; calls do not care about distance). The
    // sparse translator resolves a target to (page, slot)
    // with one compare per page, so a caller and a callee
    // hundreds of KB apart still transfer inside one
    // function — the compile row's 9 insns per host
    // dispatch were exactly these cross-page calls.
    const EDGE: u64 = 0x80;
    let seeds = jit.page_entries[&(aspace, vpage)].clone();
    let hot = |jit: &JitState, va: u64| {
        jit.page_entries
            .get(&(aspace, va))
            .is_some_and(|e| !e.is_empty())
    };
    let page_cap = region_page_cap();
    let mut pages: Vec<(u64, u64)> = vec![(vpage, pa_page)];
    let probe_add = |m: &mut M, pages: &mut Vec<(u64, u64)>, va: u64| {
        if pages.len() >= page_cap || pages.iter().any(|&(v, _)| v == va) {
            return;
        }
        if let Some(p) = m.probe_fetch(va) {
            if ram_ok(m, p) {
                pages.push((va, p & !0xfff));
            }
        }
    };
    if seeds.iter().any(|&e| (e & 0xfff) >= 0x1000 - EDGE) {
        probe_add(m, &mut pages, vpage + 0x1000);
    }
    if vpage >= 0x1000 && seeds.iter().any(|&e| (e & 0xfff) < EDGE) {
        probe_add(m, &mut pages, vpage - 0x1000);
    }
    // Contiguous hot neighbours first (the configuration
    // that measured 11/13), THEN up to two call-graph
    // joins — cross-page calls only pay when the callee
    // is genuinely hot, and gluing more than a couple of
    // far pages regressed CPython 3x.
    let mut va = vpage + 0x1000;
    while pages.len() < page_cap && hot(jit, va) {
        let before = pages.len();
        probe_add(m, &mut pages, va);
        if pages.len() == before {
            break;
        }
        va += 0x1000;
    }
    let mut va = vpage.wrapping_sub(0x1000);
    while va < vpage && pages.len() < page_cap && hot(jit, va) {
        let before = pages.len();
        probe_add(m, &mut pages, va);
        if pages.len() == before {
            break;
        }
        va = va.wrapping_sub(0x1000);
    }
    // Far (call-graph) pages join only under MEASURED
    // pressure: a first build stays contiguous — exactly
    // the configuration that held 11/13 — and rebuilds
    // pull in call targets once this page's misses prove
    // cross-page traffic. Reachability alone glued cold
    // callees into hot regions and regressed the FP rows.
    let missed_now = jit.sb_missed.get(&(aspace, vpage)).copied().unwrap_or(0);
    // With regions capped at 3 pages the far joins are
    // cheap and a19ea3b-measured; the miss gate was
    // compensating for the (now reverted) 8-page growth.
    // Far (call-graph) joins are DISABLED: they have never
    // demonstrated a win, and in a back-to-back sample on
    // an identically loaded host, regions without them ran
    // FP EMULATION at 1971 MIPS against 896 for the
    // baseline JIT. Gluing a callee page in costs every
    // entry a bigger register union and V8 a bigger
    // function; the compile row's cross-page calls need
    // regions that EXTEND on measured misses (see the
    // historical incremental-extension design (see PERFORMANCE_PROGRESS.md), not
    // regions that guess from reachability. The selection
    // code stays — it is one predicate away from being
    // re-enabled behind that signal.
    let _ = missed_now;
    let far_cap = pages.len();
    let mut scanned = 0usize;
    while scanned < pages.len() && pages.len() < far_cap {
        let (va, pp) = pages[scanned];
        scanned += 1;
        let o = (pp - rv64_system::RAM_BASE) as usize;
        let targets = rv64_dbt::page_call_targets(&m.ram()[o..o + 0x1000], va);
        for t in targets {
            if hot(jit, t & !0xfff) {
                probe_add(m, &mut pages, t & !0xfff);
            }
        }
    }
    // Only a rebuild that covered nothing new counts against the allowance:
    // a page whose hot set is still growing must be able to keep up, or code
    // that gets hot late is stranded on individual blocks forever.
    let prev = jit.sb_gen.get(&(aspace, vpage)).map_or(0, |&(e, _, _)| e);
    issue_region(
        m,
        jit,
        aspace,
        vpage,
        pages,
        sb_compiles,
        n_entries,
        n_entries <= prev,
        false,
    )
}

/// Translate `pages` as one sparse region function and issue it for ASYNC
/// compilation on V8's background threads (the sync Module build of a page
/// function stalls the guest for ms — the cold-compile cost that kept
/// superblocks gated). Execution continues on whatever is installed NOW —
/// individual blocks or a previous region function — and sys_sb_ready
/// repoints the entries only once the new function is in the table, after
/// re-validating page identity. Never uninstalls anything early: the gap
/// between issue and landing running on individual blocks was the measured
/// FP EMULATION 2568 -> 550 MIPS rebuild cliff.
///
/// `lead` keys the build cooldown (sb_gen): the page whose threshold crossing
/// owns this region, across rebuilds AND extensions.
#[allow(clippy::too_many_arguments)]
#[allow(static_mut_refs)]
fn issue_region(
    m: &mut impl SystemJitMachine,
    jit: &mut JitState,
    aspace: u64,
    lead: u64,
    pages: Vec<(u64, u64)>,
    sb_compiles: u32,
    n_entries: usize,
    unproductive: bool,
    regs_in_memory: bool,
) -> bool {
    // The build budget (sb_build_allowed) charges every issue attempt its
    // real host cost, translate failures included.
    let t0 = unsafe { host_now_ms() };
    let r = issue_region_inner(
        m,
        jit,
        aspace,
        lead,
        pages,
        sb_compiles,
        n_entries,
        unproductive,
        regs_in_memory,
    );
    unsafe { SB_BUILD_MS += host_now_ms() - t0 };
    r
}

#[allow(clippy::too_many_arguments)]
#[allow(static_mut_refs)]
fn issue_region_inner(
    m: &mut impl SystemJitMachine,
    jit: &mut JitState,
    aspace: u64,
    lead: u64,
    mut pages: Vec<(u64, u64)>,
    sb_compiles: u32,
    n_entries: usize,
    unproductive: bool,
    regs_in_memory: bool,
) -> bool {
    // The host reads this during host_jit_register_async.  Clear it before
    // every build attempt so a failed or ineligible build can never inherit
    // the previous module's attribution.
    unsafe {
        JIT_OUT_PAGE_TEMPLATE_DIAG = [0; 8];
        JIT_OUT_TEMPLATE_CACHE = [0; 3];
    };
    let mut lay = jit_layout(m);
    lay.dispatch_base = jit.dispatch.as_ptr() as u32;
    lay.dispatch_mask = (DISPATCH_SIZE - 1) as u32;
    lay.map_gen_addr = m.cpu().jit_map_gen_ptr() as u32;
    // Ascending order keeps virtually contiguous pages adjacent in the
    // concat, which is what lets bodies flow across their shared boundary.
    pages.sort_unstable_by_key(|&(va, _)| va);
    let mut code = Vec::with_capacity(pages.len() * 0x1000);
    for &(_, pp) in &pages {
        let o = (pp - rv64_system::RAM_BASE) as usize;
        code.extend_from_slice(&m.ram()[o..o + 0x1000]);
    }
    let vas: Vec<u64> = pages.iter().map(|&(va, _)| va).collect();
    // Leader discovery per CONTIGUOUS RUN of pages, from the union of the
    // run's recorded hot pcs: static reachability crosses page boundaries,
    // so a seed on one page discovers the leaders of its neighbours. The
    // per-page variant that briefly lived here (a bisect configuration from
    // the invalidated 2026-07-25 session, labeled SUB-BISECT(i)) silently
    // skipped every page with no seeds of its own — regions covered
    // fragments, exits dominated, and the branchy-int kernels lost a third
    // to half their throughput (ASSIGNMENT 12.6 -> 8.0, HUFFMAN 1525 -> 950
    // against the 11/13-era JIT).
    let mut entries: Vec<u64> = Vec::new();
    let leader_cap = region_leader_cap();
    let mut i = 0usize;
    while i < pages.len() && entries.len() < leader_cap {
        let mut j = i;
        while j + 1 < pages.len() && pages[j + 1].0 == pages[j].0 + 0x1000 {
            j += 1;
        }
        let run_slice = &code[i * 0x1000..(j + 1) * 0x1000];
        let run_va = pages[i].0;
        let run_span = ((j - i + 1) * 0x1000) as u64;
        let mut rseeds: Vec<u64> = Vec::new();
        for &(va, _) in &pages[i..=j] {
            if let Some(v) = jit.page_entries.get(&(aspace, va)) {
                rseeds.extend_from_slice(v);
            }
        }
        if !rseeds.is_empty() {
            let (mut l, back) = rv64_dbt::discover_page_leaders_ext(
                run_slice,
                run_va,
                run_va,
                run_span,
                &rseeds,
                leader_cap - entries.len(),
            );
            l.retain(|&e| {
                rv64_dbt::emittable_at(run_slice, run_va, e, lay)
                    && (unsafe { PAGE_POLICY_ENABLED }
                        || !back.contains(&e)
                        || !rv64_dbt::is_loop_at(run_slice, run_va, e, lay))
            });
            entries.extend(l);
        }
        i = j + 1;
    }
    let state = if regs_in_memory {
        rv64_dbt::MultiEntryState::Memory
    } else if unsafe { REGION_STRUCTURED_CFG } {
        rv64_dbt::MultiEntryState::RegisterStructured
    } else if unsafe { REGION_CFG_BLOCKS } {
        rv64_dbt::MultiEntryState::RegisterCfg
    } else if unsafe { REGION_DIRECT_DISPATCH } {
        rv64_dbt::MultiEntryState::RegisterDirect
    } else if unsafe { REGION_LAZY_STATE } {
        rv64_dbt::MultiEntryState::RegisterLazy
    } else {
        rv64_dbt::MultiEntryState::RegisterEager
    };
    unsafe {
        if PAGE_TEMPLATE_PROBE {
            // Class 4 accounts for every async module that is outside the
            // deliberately narrow single-page-template experiment.  Keeping
            // these modules in the diagnostic supplies an honest byte and
            // latency denominator rather than reporting only matches.
            JIT_OUT_PAGE_TEMPLATE_DIAG[0] = 4;
            if pages.len() == 1 {
                JIT_OUT_PAGE_TEMPLATE_DIAG[1] = pages[0].0;
                JIT_OUT_PAGE_TEMPLATE_DIAG[2] = pages[0].1;
            }
            JIT_OUT_PAGE_TEMPLATE_DIAG[5] = entries.len() as u64;
        }
    }
    let mut template_plan = page_template_plan(jit, &pages, &code, &entries, lay, state);
    let template_key;
    let template_cache_id;
    let template_wasm_bytes;
    if let Some(reuse) = template_plan.reuse.take() {
        // The retained module's internal dispatcher contains exactly these
        // offsets. Leave the small uncovered suffix on its existing T1 blocks
        // rather than publishing entries the module cannot execute.
        entries = reuse.covered_entries;
        template_key = None;
        template_cache_id = reuse.template_cache_id;
        template_wasm_bytes = reuse.template_wasm_bytes;
        unsafe {
            JIT_OUT.clear();
            JIT_OUT_TEMPLATE_CACHE = [2, template_cache_id, pages[0].0];
        }
    } else {
        if template_plan.compile_position_independent {
            lay.pic_code_base = Some(pages[0].0);
        }
        let translate_t0 = unsafe { host_now_ms() };
        let sb = rv64_dbt::translate_superblock_sparse_state(&code, &vas, &entries, lay, state);
        unsafe {
            SYS_TRANSLATE_ATTEMPTS += 1;
            SYS_TRANSLATE_NS += ((host_now_ms() - translate_t0).max(0.0) * 1_000_000.0) as u64;
            if let Some(region) = &sb {
                SYS_EMITTED_BYTES += region.wasm.len() as u64;
                SYS_DENSE_COPY_MEMBERS += u64::from(region.dense_copy_members);
                SYS_DENSE_STORE_MEMBERS += u64::from(region.dense_store_members);
                SYS_BULK_COPY_MEMBERS += u64::from(region.bulk_copy_members);
            }
        }
        let Some(blk) = sb else {
            unsafe { SB_XLATE_FAIL += 1 };
            return false;
        };
        entries = blk.entries.clone();
        if let Some(key) = template_plan.key.as_mut() {
            key.entries.clone_from(&entries);
        }
        template_wasm_bytes = blk.wasm.len() as u64;
        template_cache_id = if template_plan.compile_position_independent {
            unsafe {
                let id = NEXT_PAGE_TEMPLATE_CACHE_ID;
                NEXT_PAGE_TEMPLATE_CACHE_ID += 1;
                PAGE_TEMPLATE_PIC_COMPILES += 1;
                JIT_OUT_TEMPLATE_CACHE = [1, id, pages[0].0];
                id
            }
        } else {
            0
        };
        unsafe {
            if PAGE_TEMPLATE_PROBE {
                JIT_OUT_PAGE_TEMPLATE_DIAG[7] = template_wasm_bytes;
            }
            JIT_OUT = blk.wasm;
        }
        template_key = template_plan.key;
    }
    for &(_, pp) in &pages {
        m.mark_jit_page(pp);
    }
    // Every page the region covers is superblocked, and this build answers
    // the misses recorded so far on each — only misses AFTER it argue for a
    // rebuild. A neighbour pulled into this region still gets to build its
    // own region later for code this one didn't reach.
    for &(pva, _) in &pages {
        jit.superblocked.insert((aspace, pva));
        jit.sb_missed.remove(&(aspace, pva));
    }
    // The recorded instruction count starts the lead page's build cooldown.
    jit.sb_gen.insert(
        (aspace, lead),
        (
            n_entries,
            sb_compiles + u32::from(unproductive),
            m.cpu().insn_count,
        ),
    );
    m.cpu_mut().clear_store_jtlb(); // pages may now hold code
    unsafe {
        let ticket = NEXT_SB_TICKET;
        NEXT_SB_TICKET += 1;
        PENDING_SB.push(PendingSb {
            ticket,
            boot_gen: BOOT_GEN,
            aspace,
            lead,
            pages,
            entries,
            template_key,
            template_cache_id,
            template_wasm_bytes,
        });
        host_jit_register_async(ticket);
        SB_ISSUED += 1;
        SB_LAST_ICOUNT = m.cpu().insn_count;
    }
    // The caller still gives its pc an individual block right now; the
    // region function repoints the entries when the module arrives.
    true
}

/// Find a landed single-page function with identical code and emission
/// semantics. The byte comparison makes fingerprint collisions harmless.
/// Position-independent cached functions compare entries by page offset; a
/// hit requires the requested entries to be covered, then publishes every
/// entry the retained module actually exports through its internal dispatcher.
/// Missing requested entries remain on their already-generated T1 paths
/// instead of making reuse unsound.
#[allow(static_mut_refs)]
fn page_template_plan(
    jit: &JitState,
    pages: &[(u64, u64)],
    code: &[u8],
    entries: &[u64],
    layout: rv64_dbt::JitLayout,
    state: rv64_dbt::MultiEntryState,
) -> PageTemplatePlan {
    if !unsafe { PAGE_TEMPLATE_PROBE || PAGE_TEMPLATE_REUSE }
        || !unsafe { PAGE_POLICY_ENABLED }
        || unsafe { PAGE_POLICY_MEASURED_REGIONS }
        || pages.len() != 1
        || code.len() != 0x1000
        || entries.is_empty()
    {
        return PageTemplatePlan::NONE;
    }
    let fingerprint = page_template_fingerprint(code);
    let emission_config = rv64_dbt::emission_config_signature();
    // The actual alias base changes the imported immutable global but not the
    // generated module bytes or any other machine-layout capability.
    let mut key_layout = layout;
    key_layout.pic_code_base = None;
    unsafe {
        PAGE_TEMPLATE_ELIGIBLE += 1;
        if PAGE_TEMPLATE_PROBE {
            JIT_OUT_PAGE_TEMPLATE_DIAG[0] = 1;
            JIT_OUT_PAGE_TEMPLATE_DIAG[1] = pages[0].0;
            JIT_OUT_PAGE_TEMPLATE_DIAG[2] = pages[0].1;
            JIT_OUT_PAGE_TEMPLATE_DIAG[3] = 0;
            JIT_OUT_PAGE_TEMPLATE_DIAG[4] = 0;
            JIT_OUT_PAGE_TEMPLATE_DIAG[5] = entries.len() as u64;
            JIT_OUT_PAGE_TEMPLATE_DIAG[6] = 0;
        }
    };
    let mut best: Option<&PageTemplate> = None;
    let mut best_covered = 0usize;
    let mut union_covered = vec![false; entries.len()];
    let mut relocated_best: Option<&PageTemplate> = None;
    let mut relocated_best_covered = 0usize;
    let mut cached_best: Option<&PageTemplate> = None;
    let mut cached_best_covered = 0usize;
    for template in jit.page_templates.iter().rev() {
        if template.key.fingerprint != fingerprint
            || template.key.layout != key_layout
            || template.key.state != state
            || template.key.emission_config != emission_config
            || template.key.code != code
        {
            continue;
        }
        let relocated = template.key.vpage != pages[0].0;
        let covered = entries
            .iter()
            .filter(|entry| {
                if relocated {
                    template
                        .key
                        .entries
                        .iter()
                        .any(|cached| cached & 0xfff == **entry & 0xfff)
                } else {
                    template.key.entries.contains(entry)
                }
            })
            .count();
        if template.template_cache_id != 0
            && (cached_best.is_none() || covered > cached_best_covered)
        {
            cached_best = Some(template);
            cached_best_covered = covered;
        }
        if relocated {
            if relocated_best.is_none() || covered > relocated_best_covered {
                relocated_best = Some(template);
                relocated_best_covered = covered;
            }
            continue;
        }
        for (index, entry) in entries.iter().enumerate() {
            if template.key.entries.contains(entry) {
                union_covered[index] = true;
            }
        }
        if best.is_none() || covered > best_covered {
            best = Some(template);
            best_covered = covered;
        }
    }
    if let Some(template) = best {
        unsafe {
            PAGE_TEMPLATE_CODE_MATCH += 1;
            PAGE_TEMPLATE_MATCH_REQUESTED_ENTRIES += entries.len() as u64;
            PAGE_TEMPLATE_MATCH_COVERED_ENTRIES += best_covered as u64;
            PAGE_TEMPLATE_MATCH_MISSING_ENTRIES += (entries.len() - best_covered) as u64;
            let union_count = union_covered.iter().filter(|covered| **covered).count();
            PAGE_TEMPLATE_UNION_COVERED_ENTRIES += union_count as u64;
            PAGE_TEMPLATE_UNION_MISSING_ENTRIES += (entries.len() - union_count) as u64;
        }
        if best_covered == entries.len() {
            unsafe {
                PAGE_TEMPLATE_REUSABLE += 1;
                PAGE_TEMPLATE_CROSS_PHYSICAL += u64::from(template.physical_page != pages[0].1);
            }
        }
    }
    if let Some(template) = relocated_best {
        unsafe {
            PAGE_TEMPLATE_RELOCATED_MATCH += 1;
            PAGE_TEMPLATE_RELOCATED_REQUESTED_ENTRIES += entries.len() as u64;
            PAGE_TEMPLATE_RELOCATED_COVERED_ENTRIES += relocated_best_covered as u64;
            PAGE_TEMPLATE_RELOCATED_MISSING_ENTRIES +=
                (entries.len() - relocated_best_covered) as u64;
            if PAGE_TEMPLATE_RELOCATED_PAIRS.len() < 64 {
                PAGE_TEMPLATE_RELOCATED_PAIRS.push([
                    pages[0].0,
                    template.key.vpage,
                    pages[0].1,
                    template.physical_page,
                    entries.len() as u64,
                    relocated_best_covered as u64,
                ]);
            }
        }
    }
    // Attribute this module to the best representation that could avoid its
    // compile.  A fully reusable same-VA function takes precedence.  When the
    // same-VA match is incomplete, retain the relocated candidate so its
    // coverage bounds the relocation design's actual opportunity.
    if let Some(template) = best {
        unsafe {
            if PAGE_TEMPLATE_PROBE {
                JIT_OUT_PAGE_TEMPLATE_DIAG[0] = 2;
                JIT_OUT_PAGE_TEMPLATE_DIAG[3] = template.key.vpage;
                JIT_OUT_PAGE_TEMPLATE_DIAG[4] = template.physical_page;
                JIT_OUT_PAGE_TEMPLATE_DIAG[6] = best_covered as u64;
            }
        }
    }
    if best_covered != entries.len() {
        if let Some(template) = relocated_best {
            unsafe {
                if PAGE_TEMPLATE_PROBE {
                    JIT_OUT_PAGE_TEMPLATE_DIAG[0] = 3;
                    JIT_OUT_PAGE_TEMPLATE_DIAG[3] = template.key.vpage;
                    JIT_OUT_PAGE_TEMPLATE_DIAG[4] = template.physical_page;
                    JIT_OUT_PAGE_TEMPLATE_DIAG[6] = relocated_best_covered as u64;
                }
            }
        }
    }
    const REUSE_COVERAGE_PERMILLE: usize = 950;
    let reuse = cached_best
        .filter(|_| {
            cached_best_covered != 0
                && cached_best_covered * 1000 >= entries.len() * REUSE_COVERAGE_PERMILLE
        })
        .map(|template| {
            let relocated = template.key.vpage != pages[0].0;
            let requested_covered = entries
                .iter()
                .filter(|entry| {
                    if relocated {
                        template
                            .key
                            .entries
                            .iter()
                            .any(|cached| cached & 0xfff == *entry & 0xfff)
                    } else {
                        template.key.entries.contains(entry)
                    }
                })
                .count();
            // One instantiated module already contains every retained entry,
            // not merely the seed(s) that triggered this alias. Publishing all
            // exact page-offset equivalents is the physical-page ownership
            // model used by v86: later control-flow entries can dispatch to the
            // same instance without waiting through another heat cycle and
            // instantiating the same compiled module again.
            let covered_entries = if relocated {
                relocate_page_template_entries(&template.key.entries, pages[0].0)
            } else {
                template.key.entries.clone()
            };
            unsafe {
                PAGE_TEMPLATE_REUSE_HITS += 1;
                PAGE_TEMPLATE_REUSE_COVERED_ENTRIES += covered_entries.len() as u64;
                PAGE_TEMPLATE_REUSE_MISSING_ENTRIES += (entries.len() - requested_covered) as u64;
                PAGE_TEMPLATE_REUSE_SAVED_WASM_BYTES += template.wasm_bytes;
            }
            PageTemplateReuse {
                template_cache_id: template.template_cache_id,
                covered_entries,
                template_wasm_bytes: template.wasm_bytes,
            }
        });
    PageTemplatePlan {
        key: Some(PageTemplateKey {
            vpage: pages[0].0,
            fingerprint,
            code: code.to_vec(),
            entries: entries.to_vec(),
            layout: key_layout,
            state,
            emission_config,
        }),
        // Under the physical-page ownership candidate, the first eligible
        // module is position-independent and retained. Waiting until a second
        // virtual alias appears merely replaces one absolute compilation with
        // a second PIC compilation; v86 avoids that conversion by making the
        // physical page's first generated representation shareable.
        compile_position_independent: unsafe { PAGE_TEMPLATE_REUSE && reuse.is_none() },
        reuse,
    }
}

#[inline]
fn page_template_fingerprint(code: &[u8]) -> u64 {
    let mut hash = 0x9e37_79b9_7f4a_7c15u64;
    let mut chunks = code.chunks_exact(8);
    for chunk in &mut chunks {
        hash ^= u64::from_le_bytes(chunk.try_into().unwrap());
        hash = hash
            .rotate_left(27)
            .wrapping_mul(0x3c79_ac49_2ba7_b653)
            .wrapping_add(0x1c69_b3f7_4ac4_ae35);
    }
    for &byte in chunks.remainder() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash ^ code.len() as u64
}

#[inline]
fn page_template_entry_offset(entry: u64) -> u16 {
    (entry & 0xfff) as u16
}

fn relocate_page_template_entries(entries: &[u64], alias_vpage: u64) -> Vec<u64> {
    let mut relocated = Vec::with_capacity(entries.len());
    for &entry in entries {
        let alias = alias_vpage.wrapping_add(u64::from(page_template_entry_offset(entry)));
        if !relocated.contains(&alias) {
            relocated.push(alias);
        }
    }
    relocated
}

fn page_template_has_cached_physical_entry(jit: &JitState, physical_page: u64, entry: u64) -> bool {
    jit.page_template_cached_offsets
        .get(&physical_page)
        .is_some_and(|offsets| offsets.contains(&page_template_entry_offset(entry)))
}

fn rebuild_page_template_cached_offsets(jit: &mut JitState, physical_page: u64) {
    let offsets: FastHashSet<u16> = jit
        .page_templates
        .iter()
        .filter(|template| {
            template.physical_page == physical_page && template.template_cache_id != 0
        })
        .flat_map(|template| {
            template
                .key
                .entries
                .iter()
                .copied()
                .map(page_template_entry_offset)
        })
        .collect();
    if offsets.is_empty() {
        jit.page_template_cached_offsets.remove(&physical_page);
    } else {
        jit.page_template_cached_offsets
            .insert(physical_page, offsets);
    }
}

#[cfg(test)]
mod page_template_tests {
    use super::relocate_page_template_entries;

    #[test]
    fn relocates_every_cached_entry_by_page_offset_without_duplicates() {
        let entries = [0x1000, 0x102a, 0x1ffe, 0x102a];
        assert_eq!(
            relocate_page_template_entries(&entries, 0x7fff_8123_4000),
            [0x7fff_8123_4000, 0x7fff_8123_402a, 0x7fff_8123_4ffe,]
        );
    }
}

/// Record one sampled exit of a landed region function: `target` is the pc
/// the function published on its way out. Out-of-region targets accumulate
/// per page; crossing EXT_TRIGGER queues the region for measured extension.
#[inline(never)]
fn record_region_exit(jit: &mut JitState, idx: i32, target: u64, stay: u64) {
    unsafe { SB_EXIT_SAMPLED += 1 };
    let mut queue = false;
    let mut demote = false;
    if let Some(r) = jit.region_exits.get_mut(&idx) {
        r.last_tick = unsafe { EXIT_TICK };
        if stay > 0 {
            r.samples = r.samples.saturating_add(1);
            r.stay_sum = r.stay_sum.saturating_add(stay);
        }
        // The demotion verdict: enough evidence, and the function's visits
        // are too short to pay for their entries.
        if r.samples == DEMOTE_MIN_SAMPLES
            && r.stay_sum / (r.samples as u64) < DEMOTE_STAY
            && unsafe { DEMOTE_ON }
        {
            demote = true;
        }
        let tp = target & !0xfff;
        if !demote {
            if r.pages.iter().any(|&(va, _)| va == tp) {
                unsafe { SB_EXIT_INREGION += 1 };
                return; // in-region uncovered pc: sb_missed/rebuild owns that
            }
            r.total = r.total.saturating_add(1);
            if let Some(t) = r.targets.iter_mut().find(|t| t.0 == tp) {
                t.1 = t.1.saturating_add(1);
            } else if r.targets.len() < EXT_TARGET_CAP {
                r.targets.push((tp, 1));
            }
            queue = r.total == EXT_TRIGGER;
            if queue && unsafe { PAGE_POLICY_ENABLED && PAGE_POLICY_MEASURED_REGIONS } {
                let average_stay = r.stay_sum / u64::from(r.samples.max(1));
                if average_stay < unsafe { PAGE_POLICY_EXTENSION_MIN_STAY } {
                    queue = false;
                    unsafe { PAGE_POLICY_EXTENSION_SHORT_BLOCKED += 1 };
                }
            }
        }
    } else {
        unsafe { SB_EXIT_NOMAP += 1 };
    }
    if demote {
        demote_region(jit, idx);
        return;
    }
    if queue && !jit.ext_queue.contains(&idx) && jit.ext_queue.len() < SB_QUEUE_CAP {
        jit.ext_queue.push(idx);
        unsafe { SB_EXT_PUSHED += 1 };
    }
}

/// Un-claim a region function that measurably does not hold execution: its
/// entry pcs go back to individual (trace) blocks — they are hot, so the
/// interp-stretch counters re-tier them within microseconds — and the lead
/// page's build allowance is spent so the page function does not come back.
#[allow(static_mut_refs)]
fn demote_region(jit: &mut JitState, idx: i32) {
    let Some(r) = jit.region_exits.remove(&idx) else {
        return;
    };
    unsafe { SB_DEMOTED += 1 };
    for &e in &r.entries {
        if matches!(jit.cache.get(&e), Some(Some(b)) if b.idx == idx) {
            jit.cache.remove(&e);
            let slot = JitState::dslot(e);
            if jit.dispatch[slot].pc == e {
                jit.dispatch[slot].pc = NO_PC;
            }
        }
    }
    jit.regions.remove(&idx);
    jit.ext_queue.retain(|&i| i != idx);
    // Spend the allowance for every page the region covered: rebuilds check
    // `n_entries > sb_last || compiles < CAP`, so a huge sb_last plus a
    // capped compile count keeps both arms false.
    for &(va, _) in &r.pages {
        jit.sb_gen
            .insert((r.aspace, va), (usize::MAX / 2, SB_RECOMPILE_CAP, 0));
        jit.sb_missed.remove(&(r.aspace, va));
    }
}

/// Pop and build one queued extension whose region belongs to the CURRENT
/// address space. Called from the quantum boundary AND from the chain-break
/// fall-through: the boundary alone almost never runs during dispatch-miss-
/// heavy code (chains break long before the cap) and usually lands in kernel
/// moments where no queued aspace matches — extension starved at 5 builds
/// against 158k measured out-of-region exits until the fall-through call.
#[allow(static_mut_refs)]
fn drain_ext_queue(m: &mut impl SystemJitMachine, jit: &mut JitState) {
    if jit.ext_queue.is_empty()
        || (unsafe { PAGE_POLICY_ENABLED && PENDING_SB.len() >= PAGE_POLICY_INFLIGHT_LIMIT })
        || m.cpu().insn_count < unsafe { SB_EXT_NEXT_ICOUNT }
        || !sb_build_allowed(m.cpu().insn_count)
    {
        return;
    }
    unsafe { SB_EXT_DRAIN_VISITS += 1 };
    let aspace = m.cpu().sys.as_ref().map_or(0, |c| c.satp);
    let best = jit
        .ext_queue
        .iter()
        .enumerate()
        .filter_map(|(index, idx)| {
            jit.region_exits
                .get(idx)
                .filter(|region| region.aspace == aspace)
                .map(|region| (index, region.last_tick, region.total))
        })
        .max_by_key(|&(_, last_tick, total)| (last_tick, total))
        .map(|(index, _, _)| index);
    if let Some(i) = best {
        let idx = jit.ext_queue.remove(i);
        try_extend_region(m, jit, idx);
    } else {
        unsafe { SB_EXT_DRAIN_NOMATCH += 1 };
        // Nothing for this address space: back off before rescanning, and
        // drop entries that no longer resolve at all.
        unsafe { SB_EXT_NEXT_ICOUNT = m.cpu().insn_count + SB_MIN_SPACING };
        let JitState {
            ext_queue,
            region_exits,
            ..
        } = jit;
        ext_queue.retain(|idx| region_exits.contains_key(idx));
    }
}

/// Grow a region along its measured exit traffic: rebuild it over the old
/// page set plus the hottest out-of-region exit-target pages, asynchronously;
/// the old function keeps running until the superset lands. This is what a
/// build-time selection could never do for tcc-shaped code — a caller and a
/// callee 16KB apart join only when dispatches actually flow between them.
fn try_extend_region(m: &mut impl SystemJitMachine, jit: &mut JitState, idx: i32) {
    let Some(r) = jit.region_exits.get(&idx) else {
        return;
    };
    let (aspace, lead) = (r.aspace, r.lead);
    let old_pages = r.pages.clone();
    let mut targets = r.targets.clone();
    // The measured average stay picks the register mode for the superset:
    // short stays (call-shaped code) go memory-direct so entries cost
    // nothing; long stays keep the locals that make loops fast.
    let avg_stay = r.stay_sum / r.samples.max(1) as u64;
    let regs_in_memory = avg_stay < EXT_MEMORY_MODE_STAY;
    // Build cooldown keyed to the REGION (its lead page), not each member.
    let (_, compiles, when) = jit
        .sb_gen
        .get(&(aspace, lead))
        .copied()
        .unwrap_or((0, 0, 0));
    let cooldown = SB_PAGE_COOLDOWN << compiles.min(6);
    if m.cpu().insn_count < when.wrapping_add(cooldown) || compiles >= SB_RECOMPILE_CAP {
        // Not yet (or allowance spent): let the counters re-arm — the next
        // EXT_TRIGGER crossing re-queues it.
        unsafe { SB_EXT_DEFER_COOL += 1 };
        if let Some(r) = jit.region_exits.get_mut(&idx) {
            r.total = EXT_TRIGGER / 2;
        }
        return;
    }
    if unsafe { PAGE_POLICY_ENABLED && PAGE_POLICY_MEASURED_REGIONS } {
        // The page policy promotes only into the same adjacent, statically
        // reachable geometry that wins its controlled multi-page A/B. The
        // sampled exits and long-stay gate authorize promotion; arbitrary
        // exit-target call-graph pages do not become region members.
        let pages = grow_page_policy_pages(
            m,
            jit,
            aspace,
            old_pages.clone(),
            region_extension_page_cap(),
        );
        if pages.len() == old_pages.len() {
            unsafe { SB_EXT_NO_TARGET += 1 };
            if let Some(r) = jit.region_exits.get_mut(&idx) {
                r.total = EXT_TRIGGER / 2;
            }
            return;
        }
        jit.region_exits.remove(&idx);
        let n_entries = jit.page_entries.get(&(aspace, lead)).map_or(0, Vec::len);
        if issue_region(
            m, jit, aspace, lead, pages, compiles, n_entries, false, false,
        ) {
            unsafe { SB_EXT_ISSUED += 1 };
        }
        return;
    }
    // The old pages carry their recorded pas; a page remapped since then is
    // caught at landing (marked/dirty) and at first dispatch (pa-verify), so
    // no probe — a probe here would fail on privilege whenever the build slot
    // lands inside the kernel, which is most of the time.
    let mut pages: Vec<(u64, u64)> = old_pages;
    // Hottest measured targets first; only pages with recorded hot code join
    // (a target with no page_entries has nothing to discover leaders from).
    // A target's pa comes from any of its already-compiled blocks — again no
    // probe; a target with no pa-carrying cache entry is skipped.
    targets.sort_unstable_by_key(|&(_, c)| core::cmp::Reverse(c));
    let mut added = 0usize;
    for &(tp, _) in &targets {
        if pages.len() >= region_extension_page_cap() {
            break;
        }
        if pages.iter().any(|&(v, _)| v == tp) {
            continue;
        }
        let Some(entries) = jit.page_entries.get(&(aspace, tp)) else {
            continue;
        };
        let Some(pa) = entries.iter().find_map(|e| {
            jit.cache
                .get(e)
                .and_then(|b| b.as_ref())
                .filter(|b| b.idx >= 0)
                .map(|b| b.pa & !0xfff)
        }) else {
            continue;
        };
        let ram_ok = pa >= rv64_system::RAM_BASE
            && (pa - rv64_system::RAM_BASE) as usize + 0x1000 <= m.ram().len();
        if ram_ok {
            pages.push((tp, pa));
            added += 1;
        }
    }
    if added == 0 {
        unsafe { SB_EXT_NO_TARGET += 1 };
        if let Some(r) = jit.region_exits.get_mut(&idx) {
            r.total = EXT_TRIGGER / 2;
        }
        return;
    }
    // Consume the profile: the old function keeps running (and keeps its
    // regions pa-verify entry) but stops sampling; the superset starts a
    // fresh profile when it lands.
    jit.region_exits.remove(&idx);
    let n_entries = jit.page_entries.get(&(aspace, lead)).map_or(0, |e| e.len());
    if issue_region(
        m,
        jit,
        aspace,
        lead,
        pages,
        compiles,
        n_entries,
        false,
        regs_in_memory,
    ) {
        unsafe { SB_EXT_ISSUED += 1 };
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
#[allow(clippy::needless_range_loop)] // avoids references to mutable profiling statics
pub extern "C" fn sys_run(max_insns: u64) -> i32 {
    let m = unsafe { SYS.as_mut().expect("call sys_boot() first") };
    m.set_rtc_unix_ns(unsafe { host_unix_ms() } as u64 * 1_000_000);
    if !unsafe { JIT_ENABLED } {
        return run_system_interpreter(m, max_insns);
    }
    run_system_jit(m, max_insns)
}

/// Exact non-JIT driver: one machine interpreter slice for the caller's full
/// budget, followed by the same host-I/O drain used by generated execution.
/// Deliberately does not initialize or inspect JitState.
fn run_system_interpreter(m: &mut impl SystemJitMachine, max_insns: u64) -> i32 {
    m.cpu_mut()
        .set_interpreter_fused_memory(unsafe { INTERPRETER_FUSED_MEMORY });
    m.run_direct_interpreter(max_insns);
    m.flush_host_io();
    m.power_off() as i32
}

#[allow(static_mut_refs)]
fn page_policy_observe(jit: &mut JitState, sample: PagePolicySample) {
    if sample.pa < rv64_system::RAM_BASE {
        return;
    }
    let vpage = sample.pc & !0xfff;
    let pa_page = sample.pa & !0xfff;
    let key = (vpage, pa_page);
    if sample.control_entry {
        unsafe { PAGE_POLICY_CONTROL_ENTRY_SAMPLES += 1 };
    }
    let observations = jit.policy_observations.entry(key).or_insert(0);
    *observations = observations.saturating_add(1);
    if sample.control_entry {
        let controls = jit.policy_control_observations.entry(key).or_insert(0);
        *controls = controls.saturating_add(1);
    }
    let retired_tick = unsafe {
        PAGE_POLICY_SAMPLES += 1;
        PAGE_POLICY_RETIRED += sample.retired;
        if sample.mode == rv64_core::csr::Mode::User as u8 {
            PAGE_POLICY_USER_RETIRED += sample.retired;
        } else {
            PAGE_POLICY_PRIVILEGED_RETIRED += sample.retired;
        }
        PAGE_POLICY_RETIRED
    };

    // A profile-only control target is classification evidence, not an entry
    // seed. Returning here is deliberately stronger than merely adding zero
    // heat: it cannot refresh queue recency, trigger a rebuild, or alter the
    // member set of the page it is measuring.
    if sample.control_entry && !unsafe { PAGE_POLICY_CONTROL_ENTRIES } {
        return;
    }

    // This is evidence, not an execution permission: a region assembled from
    // it carries the recorded PA for every page and the dispatcher probes all
    // of those mappings in the address space that eventually enters it. A
    // profile-only target returns above so classification cannot invent region
    // geometry that the ordinary sampler never observed.
    jit.policy_mappings.insert((sample.aspace, vpage), pa_page);

    if jit.policy_rejected.contains(&key)
        || jit
            .policy_attempted
            .get(&key)
            .is_some_and(|entries| entries.contains(&sample.pc))
        || jit
            .policy_installed
            .get(&key)
            .is_some_and(|entries| entries.contains(&sample.pc))
    {
        return;
    }
    jit.policy_last_sample.insert(key, retired_tick);

    // A retained PIC module for this exact physical code page has already
    // paid both translation and V8 compilation. Do not make a new virtual
    // alias interpret 131k instructions merely to discover that fact. Dirty
    // page processing removes this physical index before observation can use
    // it; `page_template_plan` still performs the exact code/layout/state and
    // requested-entry comparison before reuse is authorized.
    let eager_physical_template = unsafe { PAGE_TEMPLATE_REUSE }
        && page_template_has_cached_physical_entry(jit, pa_page, sample.pc);

    let entries = jit.policy_entries.entry(key).or_default();
    if let Err(index) = entries.binary_search(&sample.pc) {
        entries.insert(index, sample.pc);
    }
    let heat = jit.policy_heat.entry(key).or_insert(0);
    *heat = heat.saturating_add(sample.retired);
    let threshold = unsafe {
        if sample.mode == rv64_core::csr::Mode::User as u8 {
            PAGE_POLICY_THRESHOLD
        } else {
            PAGE_POLICY_THRESHOLD.saturating_mul(PAGE_POLICY_PRIVILEGED_THRESHOLD_MULTIPLIER)
        }
    };
    if eager_physical_template {
        *heat = (*heat).max(threshold);
    }
    if *heat < threshold
        || jit.policy_queued.contains(&key)
        || jit.policy_suppressed.contains(&key)
        || jit.policy_pending.contains(&key)
    {
        return;
    }
    if jit.policy_queue.len() >= PAGE_POLICY_QUEUE_CAP {
        if jit.policy_suppressed.insert(key) {
            unsafe { PAGE_POLICY_QUEUE_DROPS += 1 };
        }
        return;
    }
    jit.policy_queue.push_back(PagePolicyCandidate {
        aspace: sample.aspace,
        vpage,
        pa_page,
    });
    jit.policy_queued.insert(key);
    unsafe {
        PAGE_TEMPLATE_EAGER_PHYSICAL_CANDIDATES += u64::from(eager_physical_template);
        PAGE_POLICY_CANDIDATES += 1;
        if sample.mode == rv64_core::csr::Mode::User as u8 {
            PAGE_POLICY_USER_CANDIDATES += 1;
        } else {
            PAGE_POLICY_PRIVILEGED_CANDIDATES += 1;
        }
        PAGE_POLICY_QUEUE_MAX = PAGE_POLICY_QUEUE_MAX.max(jit.policy_queue.len() as u64);
    }
}

#[inline]
fn page_policy_control_allows_multipage(jit: &JitState, key: (u64, u64)) -> bool {
    if !unsafe { PAGE_POLICY_CONTROL_ENTRIES || PAGE_POLICY_CONTROL_PROFILE } {
        return true;
    }
    let observations = jit.policy_observations.get(&key).copied().unwrap_or(0);
    if observations == 0 {
        return true;
    }
    let controls = jit
        .policy_control_observations
        .get(&key)
        .copied()
        .unwrap_or(0);
    controls.saturating_mul(1000)
        <= observations.saturating_mul(unsafe { PAGE_POLICY_MULTIPAGE_CONTROL_PERMILLE })
}

/// Extend an already selected contiguous page-policy region with adjacent
/// mappings that the compiler's bounded direct-CFG walk can actually reach.
/// Candidate pages never donate their own seeds to the reachability test: an
/// edge from the existing region must justify each addition.
fn grow_page_policy_pages(
    m: &impl SystemJitMachine,
    jit: &JitState,
    aspace: u64,
    mut selected: Vec<(u64, u64)>,
    page_cap: usize,
) -> Vec<(u64, u64)> {
    if selected.is_empty() {
        return selected;
    }
    let mut low = selected.iter().map(|&(va, _)| va).min().unwrap();
    let mut high = selected.iter().map(|&(va, _)| va).max().unwrap();
    while selected.len() < page_cap {
        let mut neighbours = Vec::with_capacity(2);
        for va in [low.checked_sub(0x1000), high.checked_add(0x1000)]
            .into_iter()
            .flatten()
        {
            if selected.iter().any(|&(selected_va, _)| selected_va == va) {
                continue;
            }
            let Some(pa) = jit.policy_mappings.get(&(aspace, va)).copied() else {
                continue;
            };
            let neighbour_key = (va, pa);
            let in_ram = pa >= rv64_system::RAM_BASE
                && (pa - rv64_system::RAM_BASE) as usize + 0x1000 <= m.ram().len();
            if !in_ram
                || jit.policy_rejected.contains(&neighbour_key)
                || !page_policy_control_allows_multipage(jit, neighbour_key)
            {
                continue;
            }
            if jit
                .policy_entries
                .get(&neighbour_key)
                .is_none_or(Vec::is_empty)
            {
                continue;
            }

            let mut trial_pages = selected.clone();
            trial_pages.push((va, pa));
            trial_pages.sort_unstable_by_key(|&(trial_va, _)| trial_va);
            let trial_base = trial_pages[0].0;
            let mut trial_code = Vec::with_capacity(trial_pages.len() * 0x1000);
            for &(_, trial_pa) in &trial_pages {
                let offset = (trial_pa - rv64_system::RAM_BASE) as usize;
                trial_code.extend_from_slice(&m.ram()[offset..offset + 0x1000]);
            }
            let trial_seeds: Vec<u64> = selected
                .iter()
                .flat_map(|&(selected_va, selected_pa)| {
                    jit.policy_entries
                        .get(&(selected_va, selected_pa))
                        .into_iter()
                        .flatten()
                        .copied()
                })
                .collect();
            let reachable = if unsafe { PAGE_POLICY_CROSS_PAGE_CALLS } {
                rv64_dbt::discover_page_reachable_pages(
                    &trial_code,
                    trial_base,
                    trial_base,
                    trial_code.len() as u64,
                    &trial_seeds,
                    region_leader_cap(),
                )
            } else {
                rv64_dbt::discover_page_reachable_noncall_pages(
                    &trial_code,
                    trial_base,
                    trial_base,
                    trial_code.len() as u64,
                    &trial_seeds,
                    region_leader_cap(),
                )
            };
            if reachable.contains(&va) {
                neighbours.push((
                    jit.policy_heat.get(&neighbour_key).copied().unwrap_or(0),
                    va,
                    pa,
                ));
            }
        }
        let Some((_, va, pa)) = neighbours.into_iter().max_by_key(|item| item.0) else {
            break;
        };
        low = low.min(va);
        high = high.max(va);
        selected.push((va, pa));
    }
    selected
}

/// Issue at most one page function per drain call. Up to the measured global
/// in-flight limit may await WebAssembly.compile; later candidates remain in
/// the bounded Wasm-side queue until `sys_sb_ready` publishes a result.
#[allow(static_mut_refs)]
fn page_policy_issue(m: &mut impl SystemJitMachine, jit: &mut JitState) {
    if !unsafe { PAGE_POLICY_ENABLED } || jit_table_full() {
        return;
    }
    if jit.policy_queue.len() < PAGE_POLICY_QUEUE_CAP / 2 {
        // Queue pressure has subsided. Previously suppressed hot mappings may
        // offer themselves again on their next interpreted sample, avoiding a
        // per-32-instruction retry storm while preserving eventual progress.
        jit.policy_suppressed.clear();
    }
    let now = unsafe { PAGE_POLICY_RETIRED };
    let mut index = 0usize;
    while index < jit.policy_queue.len() {
        let candidate = jit.policy_queue[index];
        let key = (candidate.vpage, candidate.pa_page);
        let last = jit.policy_last_sample.get(&key).copied().unwrap_or(0);
        if now.saturating_sub(last) > PAGE_POLICY_STALE_INSNS {
            jit.policy_queue.remove(index);
            jit.policy_queued.remove(&key);
            unsafe { PAGE_POLICY_STALE_DROPS += 1 };
        } else {
            index += 1;
        }
    }
    if unsafe { PENDING_SB.len() >= PAGE_POLICY_INFLIGHT_LIMIT } {
        return;
    }
    while !jit.policy_queue.is_empty() {
        let best = jit
            .policy_queue
            .iter()
            .enumerate()
            .max_by_key(|(_, candidate)| {
                let key = (candidate.vpage, candidate.pa_page);
                (
                    jit.policy_heat.get(&key).copied().unwrap_or(0),
                    jit.policy_last_sample.get(&key).copied().unwrap_or(0),
                )
            })
            .map(|(index, _)| index)
            .unwrap();
        let candidate = jit.policy_queue.remove(best).unwrap();
        let key = (candidate.vpage, candidate.pa_page);
        jit.policy_queued.remove(&key);
        if jit.policy_pending.contains(&key) || jit.policy_rejected.contains(&key) {
            continue;
        }
        let ram_ok = candidate.pa_page >= rv64_system::RAM_BASE
            && (candidate.pa_page - rv64_system::RAM_BASE) as usize + 0x1000 <= m.ram().len();
        let all_entries: Vec<u64> = jit
            .policy_entries
            .get(&key)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let attempted = jit.policy_attempted.get(&key);
        let installed = jit.policy_installed.get(&key);
        let new_entries: Vec<u64> = all_entries
            .iter()
            .copied()
            .filter(|entry| attempted.is_none_or(|set| !set.contains(entry)))
            .filter(|entry| installed.is_none_or(|set| !set.contains(entry)))
            .collect();
        if !ram_ok {
            jit.policy_rejected.insert(key);
            unsafe { PAGE_POLICY_FAILED += 1 };
            continue;
        }
        if new_entries.is_empty() {
            jit.policy_heat.insert(key, 0);
            continue;
        }
        // A late entry rebuilds the page from the accumulated entry set, as in
        // v86, rather than creating a permanently disconnected fragment. Put
        // new seeds first so the bounded leader walk cannot strand the event
        // that paid for this rebuild; the remaining established entries then
        // recover same-module indirect dispatch wherever the cap permits.
        let lead_entry_count = all_entries.len();
        let mut entries = new_entries.clone();
        if unsafe { PAGE_POLICY_REBUILD } {
            let mut included: FastHashSet<u64> = entries.iter().copied().collect();
            entries.extend(
                all_entries
                    .into_iter()
                    .filter(|entry| included.insert(*entry)),
            );
        }
        // Grow only through virtually contiguous pages that were actually
        // observed in the lead candidate's address space. This is the RV64
        // counterpart of v86's bounded cross-page CFG closure: it lets direct
        // fallthroughs/branches remain in one Wasm function without guessing
        // that a VA from another process has the same mapping. The sparse
        // translator performs the actual reachability discovery across the
        // concatenated pages; a cold neighbour contributes only its observed
        // entry seeds and the configured cap bounds compile cost.
        let configured_page_cap = region_page_cap();
        // RV64 permits a 32-bit instruction at page offset 0xffe. Its low
        // halfword is on the lead page and its high halfword is on the next
        // executable page. Ordinary region-growth heuristics are about CFG
        // profitability; they must not strand the instruction that is the
        // compilation entry itself. Carry the adjacent mapping as a required
        // fetch page (and therefore as an invalidation/dispatch proof), even
        // when entry-count or control-flow gates reject optional expansion.
        let fetch_straddles = entries.iter().any(|entry| {
            if entry & 0xfff != 0xffe {
                return false;
            }
            let offset = (candidate.pa_page - rv64_system::RAM_BASE) as usize + 0xffe;
            let low = u16::from_le_bytes([m.ram()[offset], m.ram()[offset + 1]]);
            low & 3 == 3
        });
        let required_fetch_page = if fetch_straddles && configured_page_cap > 1 {
            let next_va = candidate.vpage + 0x1000;
            let recorded = jit
                .policy_mappings
                .get(&(candidate.aspace, next_va))
                .copied();
            let current_aspace = m.cpu().sys.as_ref().map_or(0, |sys| sys.satp);
            let mapped = recorded.or_else(|| {
                (current_aspace == candidate.aspace)
                    .then(|| m.probe_fetch(next_va).map(|pa| pa & !0xfff))
                    .flatten()
            });
            let mapped = mapped.filter(|pa| {
                *pa >= rv64_system::RAM_BASE
                    && (*pa - rv64_system::RAM_BASE) as usize + 0x1000 <= m.ram().len()
            });
            if let Some(pa) = mapped {
                jit.policy_mappings.insert((candidate.aspace, next_va), pa);
                unsafe { PAGE_POLICY_FETCH_STRADDLE_FORCED += 1 };
                Some((next_va, pa))
            } else {
                // Leave an incomplete byte snapshot eligible to reheat.
                jit.policy_heat.insert(key, 0);
                unsafe { PAGE_POLICY_FETCH_STRADDLE_DEFERRED += 1 };
                continue;
            }
        } else {
            None
        };
        let entry_eligible = lead_entry_count <= unsafe { PAGE_POLICY_MULTIPAGE_ENTRY_CAP };
        let control_eligible = page_policy_control_allows_multipage(jit, key);
        if configured_page_cap > 1 {
            unsafe {
                if entry_eligible {
                    PAGE_POLICY_MULTIPAGE_ENTRY_ELIGIBLE += 1;
                } else {
                    PAGE_POLICY_MULTIPAGE_ENTRY_BLOCKED += 1;
                }
                if control_eligible {
                    PAGE_POLICY_MULTIPAGE_CONTROL_ELIGIBLE += 1;
                } else {
                    PAGE_POLICY_MULTIPAGE_CONTROL_BLOCKED += 1;
                }
            }
        }
        let page_cap = if required_fetch_page.is_some() {
            configured_page_cap
        } else if entry_eligible && control_eligible {
            configured_page_cap
        } else {
            1
        };
        let mut initial_pages = vec![(candidate.vpage, candidate.pa_page)];
        if let Some(required) = required_fetch_page {
            initial_pages.push(required);
        }
        let selected_pages =
            grow_page_policy_pages(m, jit, candidate.aspace, initial_pages, page_cap);
        let selected: Vec<(u64, u64, Vec<u64>, Vec<u64>)> = selected_pages
            .into_iter()
            .map(|(va, pa)| {
                if va == candidate.vpage && pa == candidate.pa_page {
                    (va, pa, entries.clone(), new_entries.clone())
                } else {
                    (
                        va,
                        pa,
                        jit.policy_entries
                            .get(&(va, pa))
                            .cloned()
                            .unwrap_or_default(),
                        Vec::new(),
                    )
                }
            })
            .collect();

        let mut pages = Vec::with_capacity(selected.len());
        let mut policy_keys = Vec::with_capacity(selected.len());
        for (va, pa, seeds, attempted_seeds) in selected {
            let selected_key = (va, pa);
            jit.policy_attempted
                .entry(selected_key)
                .or_default()
                .extend(attempted_seeds);
            jit.policy_heat.insert(selected_key, 0);
            jit.page_entries.insert((candidate.aspace, va), seeds);
            // A queued neighbour is satisfied by this combined module. Remove
            // it now so landing cannot immediately issue a duplicate function.
            if selected_key != key {
                jit.policy_queue
                    .retain(|queued| (queued.vpage, queued.pa_page) != selected_key);
                jit.policy_queued.remove(&selected_key);
            }
            pages.push((va, pa));
            policy_keys.push(selected_key);
        }
        let issued_pages = pages.len() as u64;
        let rebuild = jit.policy_compiled.contains(&key);
        let n_entries: usize = policy_keys
            .iter()
            .map(|key| jit.policy_entries.get(key).map_or(0, Vec::len))
            .sum();
        let issued = issue_region(
            m,
            jit,
            candidate.aspace,
            candidate.vpage,
            pages,
            0,
            n_entries,
            false,
            false,
        );
        if issued {
            jit.policy_pending.extend(policy_keys);
            unsafe {
                PAGE_POLICY_ISSUED += 1;
                PAGE_POLICY_ISSUED_PAGES += issued_pages;
                PAGE_POLICY_MULTI_PAGE_ISSUED += u64::from(issued_pages > 1);
                PAGE_POLICY_REBUILDS += u64::from(rebuild);
            };
        } else {
            unsafe { PAGE_POLICY_FAILED += 1 };
        }
        break;
    }
}

#[allow(static_mut_refs)]
#[allow(clippy::needless_range_loop)]
fn run_system_jit(m: &mut impl SystemJitMachine, max_insns: u64) -> i32 {
    m.activate();
    m.cpu_mut()
        .set_interpreter_fused_memory(unsafe { INTERPRETER_FUSED_MEMORY });
    let jit = unsafe { SYS_JIT.get_or_insert_with(JitState::new) };
    let mut remaining = max_insns;

    while remaining > 0 && !m.power_off() {
        // Generated memory probes snapshot this cell once per invocation.
        // SYSTEM/CSR instructions never execute in generated code, so the
        // effective permission context is stable for the complete chain.
        m.cpu_mut().set_jit_tlb_hash(unsafe { JIT_TLB_HASH });
        m.cpu_mut()
            .set_privilege_tlb_retention(unsafe { PRIVILEGE_TLB_RETENTION });
        m.cpu_mut().sync_jit_tlb_context();
        // Refresh the wall-clock time source (opt-in) so the CLINT tracks real
        // host time. host_now_ms is a wasm->JS round-trip (~7% of a dispatch-
        // heavy workload if done per iteration), so gate it: refresh only after
        // ~16k retired insns (~40us at JIT speed, far finer than the 10ms kernel
        // tick) or after 64 iterations without insn progress (WFI idle — time
        // must still advance or timers never fire).
        m.wallclock_iteration();
        if unsafe { !JIT_SUPERVISOR_ENABLED }
            && m.cpu()
                .sys
                .as_ref()
                .is_some_and(|sys| sys.mode != Mode::User)
        {
            let ran = m.run_interpreter(remaining.min(4096));
            unsafe {
                SLICE_CALLS += 1;
                SLICE_INSNS += ran;
            }
            if ran == 0 {
                break;
            }
            remaining = remaining.saturating_sub(ran);
            m.flush_host_io();
            continue;
        }
        // Address-space switch (satp write): compiled blocks SURVIVE — they're
        // va-keyed and every dispatch lazily re-verifies its va→pa mapping when
        // cpu.map_gen moved, so a block whose va now maps elsewhere (or nowhere)
        // is dropped at dispatch, and kernel/global mappings keep their blocks
        // across every context switch (recompiling the working set per switch
        // was a large fraction of boot time). Only va-keyed state that could
        // POISON a different address space at the same va must go: blacklist
        // entries (a va untranslatable in space A may be hot compilable code in
        // space B) and superblock page-entry lists (block starts from A are
        // arbitrary byte offsets in B).
        if m.cpu().jit_flush_gen != jit.flush_gen {
            jit.flush_gen = m.cpu().jit_flush_gen;
            // Superblock discovery state is keyed by (satp, virtual page), so a
            // context switch no longer throws it away. It used to: every satp
            // write cleared every page's hot-pc list, and since those pcs were
            // by then compiled (and so never re-registered), a page could never
            // grow its superblock again — whether a page ended up covered came
            // down to how many switches happened while it was warming up, which
            // is why nbench IDEA measured 1538 or 4594 iter/s from identical
            // runs. Bound the map instead: address spaces come and go.
            if jit.page_entries.len() > SB_SPACE_CAP {
                jit.page_entries.clear();
                jit.superblocked.clear();
                jit.sb_gen.clear();
            }
        }
        // Per-page invalidation: drop only blocks whose physical code page
        // was written (self-modifying code / recycled pages), and clear only
        // their dispatch lines (a full dispatch memset is megabytes per event).
        if m.dirty_pages_pending() {
            let dirty = m.take_dirty_pages();
            let mut dirty_vpages: FastHashSet<u64> = Default::default();
            unsafe {
                DIRTY_EVENTS += dirty.len() as u64;
                // The trace-window gathers may hold pre-store bytes.
                TRACE_WIN.clear();
            }
            for &ppage in &dirty {
                let physical_page = rv64_system::RAM_BASE + (ppage << 12);
                if let Some(pcs) = jit.page_blocks.remove(&ppage) {
                    for pc in pcs {
                        unsafe { DIRTY_DROPPED += 1 };
                        dirty_vpages.insert(pc & !0xfff);
                        jit.cache.remove(&pc);
                        let slot = JitState::dslot(pc);
                        if jit.dispatch[slot].pc == pc {
                            jit.dispatch[slot].pc = NO_PC;
                        }
                    }
                }
                jit.policy_heat.retain(|&(_, pa), _| pa != physical_page);
                jit.policy_last_sample
                    .retain(|&(_, pa), _| pa != physical_page);
                jit.policy_entries.retain(|&(_, pa), _| pa != physical_page);
                jit.policy_observations
                    .retain(|&(_, pa), _| pa != physical_page);
                jit.policy_control_observations
                    .retain(|&(_, pa), _| pa != physical_page);
                jit.policy_mappings.retain(|_, pa| *pa != physical_page);
                jit.policy_attempted
                    .retain(|&(_, pa), _| pa != physical_page);
                jit.policy_installed
                    .retain(|&(_, pa), _| pa != physical_page);
                jit.policy_queue
                    .retain(|candidate| candidate.pa_page != physical_page);
                jit.policy_queued.retain(|&(_, pa)| pa != physical_page);
                jit.policy_suppressed.retain(|&(_, pa)| pa != physical_page);
                jit.policy_compiled.retain(|&(_, pa)| pa != physical_page);
                jit.policy_rejected.retain(|&(_, pa)| pa != physical_page);
                // A physical-page template is valid only while dirty tracking
                // proves its exact code snapshot remains current. The ordinary
                // reuse path also compares bytes, but the eager acceleration
                // index deliberately avoids rereading RAM on every sample.
                jit.page_templates
                    .retain(|template| template.physical_page != physical_page);
                jit.page_template_cached_offsets.remove(&physical_page);
                m.unmark_jit_page(ppage);
            }
            // Re-discover superblock entries for the pages whose code bytes
            // changed (any address space that mapped them), not globally.
            if !dirty_vpages.is_empty() {
                jit.page_entries
                    .retain(|&(_, vp), _| !dirty_vpages.contains(&vp));
                jit.superblocked
                    .retain(|&(_, vp)| !dirty_vpages.contains(&vp));
                jit.sb_gen.retain(|&(_, vp), _| !dirty_vpages.contains(&vp));
            }
        }
        page_policy_issue(m, jit);
        // --- JIT fast path: direct-mapped dispatch + cheap pa-verify ---
        // Per-dispatch bookkeeping accumulates in LOCALS and flushes once after
        // the chain: at ~200M+ dispatches per second of guest compute, the five
        // read-modify-writes this loop used to do per iteration (insn_count,
        // remaining, two stat counters, chain counter) were a measurable slice
        // of total wall time. map_gen is hoisted too — blocks can't execute
        // satp/SFENCE (SYSTEM never compiles; blocks bail AT it), so it cannot
        // move inside a chain.
        let map_gen = m.cpu().map_gen as u32;
        let mptr = m.state_ptr() as *mut u8;
        let mut chained = 0u32;
        let mut retired_sum = 0u64;
        // Budget/interrupt contract: this round may retire at most
        // min(remaining, INTERRUPT_QUANTUM) instructions (to block/iteration
        // granularity); each dispatch is granted the leftover as loop fuel.
        let round_budget = remaining.min(INTERRUPT_QUANTUM);
        // The fuel cell is only consulted by loop/region blocks; refreshing
        // it on every dispatch is a store per ~13-insn block. Refresh every
        // 8 dispatches or 4K retired — staleness overshoots the round by at
        // most that, within the documented block-granularity tolerance
        // (user_run keeps its exact per-dispatch store).
        unsafe { FUEL_CELL = round_budget };
        let mut fuel_stored_at = 0u64;
        while chained < JIT_CHAIN_CAP && retired_sum < round_budget {
            if chained & 7 == 0 || retired_sum.wrapping_sub(fuel_stored_at) > 4096 {
                unsafe { FUEL_CELL = round_budget - retired_sum };
                fuel_stored_at = retired_sum;
            }
            let pc = m.cpu().pc;
            let slot = JitState::dslot(pc);
            // Fast path: line hit AND no mapping event since it verified —
            // one 16-byte load and two compares, then straight to the call.
            // Any other case (miss, or SFENCE.VMA/satp moved cpu.map_gen)
            // resolves through the authoritative cache with a fetch-TLB
            // probe and refills the line stamped with the current generation
            // — so the cache survives the frequent data-page SFENCEs of
            // malloc-heavy processes (one re-probe per block per event, not
            // a flush).
            let line = jit.dispatch[slot];
            let idx = if line.pc == pc && line.gen == map_gen && line.gen != UNVERIFIED_DISPATCH_GEN
            {
                line.idx
            } else {
                match jit.cache.get(&pc) {
                    Some(Some(b)) => {
                        let b = *b;
                        // Multi-page code: every page it was compiled over
                        // must still map where it did. Region functions AND
                        // page-crossing trace blocks both record their page
                        // sets here; single-page blocks miss (fast).
                        let region = jit.regions.get(&b.idx).cloned();
                        let self_ok = matches!(
                            m.probe_fetch(pc), Some(pa) if pa == b.pa
                        );
                        let region_ok = region.is_none_or(|pgs| {
                            pgs.iter().all(|&(va, pp)| {
                                matches!(
                                    m.probe_fetch(va),
                                    Some(q) if q & !0xfff == pp
                                )
                            })
                        });
                        let mapped = self_ok && region_ok;
                        if !mapped {
                            unsafe {
                                if !self_ok {
                                    DROP_SELF += 1;
                                } else {
                                    DROP_REGION += 1;
                                }
                            }
                            jit.cache.remove(&pc);
                            jit.dispatch[slot].pc = NO_PC;
                            break;
                        }
                        // Region functions (n == 0) carry SB_IDX_BIT in their
                        // dispatch line so the exit below can be attributed
                        // without a cache probe (blacklist -1 keeps its sign).
                        let tagged = if b.n == 0 && b.idx >= 0 {
                            b.idx | SB_IDX_BIT
                        } else {
                            b.idx
                        };
                        jit.dispatch[slot] = DispatchLine {
                            pc,
                            idx: tagged,
                            gen: map_gen,
                        };
                        tagged
                    }
                    _ => break, // uncompiled or blacklisted
                }
            };
            if idx < 0 {
                break; // blacklisted (pa-verified for the current mapping)
            }
            let is_region = idx & SB_IDX_BIT != 0;
            if is_region {
                call_region(idx & !SB_IDX_BIT, mptr);
            } else {
                call_block(idx, mptr);
            }
            // Before specialization, a one-body return exposes the exact
            // successor of its terminating indirect edge. After monomorphic
            // fusion, only an explicit cold GuardTarget miss is feedback: a
            // successful fused invocation returns from some later edge, which
            // must not be mistaken for a second target of the original site.
            let feedback = if is_region {
                None
            } else {
                unsafe {
                    if IC_MISS_OWNER_CELL != NO_PC {
                        Some((IC_MISS_OWNER_CELL, IC_MISS_TARGET_CELL))
                    } else {
                        let one_body = jit.cache.get(&pc).is_some_and(|entry| {
                            entry.as_ref().is_some_and(|block| {
                                block.n != 0 && RETIRED_CELL <= u64::from(block.n)
                            })
                        });
                        (!jit.ic_targets.contains_key(&pc) && one_body).then_some((pc, m.cpu().pc))
                    }
                }
            };
            if let Some((owner, target)) = feedback {
                let sl = JitState::dslot(owner);
                jit.succ[sl].observe(owner, target);
                let profile = jit.succ[sl];
                let mature = profile.ranked_targets(owner, unsafe { IC_EXTEND_TRIGGER });
                let embedded = jit.ic_targets.get(&owner).map_or(0, Vec::len);
                let desired = match (embedded, mature.len()) {
                    (0, 1) if profile.is_dominant(owner, mature[0], 8) => Some(mature),
                    (0, 2..) => Some(mature),
                    (1, 2..) => Some(mature),
                    _ => None,
                };
                if let Some(mut targets) = desired {
                    targets.truncate(2);
                    jit.ic_targets.insert(owner, targets.clone());
                    jit.cache.remove(&owner);
                    jit.dispatch[sl].pc = NO_PC;
                    unsafe {
                        IC_EXTENDS += 1;
                        if targets.len() == 2 {
                            IC_PIC_EXTENDS += 1;
                        }
                    }
                }
            }
            // Sampled exit attribution: after a region function returns,
            // cpu.pc holds the exit target, the measured extension signal.
            if idx & SB_IDX_BIT != 0 && measured_region_policy_enabled() {
                let tick = unsafe {
                    EXIT_TICK = EXIT_TICK.wrapping_add(1);
                    EXIT_TICK
                };
                if tick & ((1 << EXIT_SAMPLE_SHIFT) - 1) == 0 {
                    let stay = unsafe { RETIRED_CELL };
                    record_region_exit(jit, idx & !SB_IDX_BIT, m.cpu().pc, stay);
                }
            }
            // Sys blocks with inline memory ops may bail mid-block; read the
            // count they actually retired (pc is set by the block either way).
            let retired = unsafe { RETIRED_CELL };
            let dprof_sample = unsafe {
                if DPROF_ON {
                    DPROF_TICK = DPROF_TICK.wrapping_add(1);
                    DPROF_TICK & ((1u64 << DPROF_SAMPLE_SHIFT) - 1) == 0
                } else {
                    false
                }
            };
            if dprof_sample {
                dprof_hit(pc, retired);
                eprof_hit(pc, m.cpu().pc, retired);
            }
            if unsafe { DPROF_ON } {
                unsafe {
                    if idx & SB_IDX_BIT != 0 {
                        DPROF_REGION_CALLS += 1;
                        DPROF_REGION_INSNS += retired;
                    } else {
                        DPROF_BLOCK_CALLS += 1;
                        DPROF_BLOCK_INSNS += retired;
                        if let Some(Some(b)) = jit.cache.get(&pc) {
                            if b.n != 0 {
                                let mut attributed = 0u64;
                                for i in 1..5 {
                                    let count = retired * b.mix[i] as u64 / b.n as u64;
                                    DPROF_TRACE_MIX[i] += count;
                                    attributed += count;
                                }
                                DPROF_TRACE_MIX[0] += retired.saturating_sub(attributed);
                                for i in 0..10 {
                                    DPROF_TRACE_MEM[i] += retired * b.mem[i] as u64 / b.n as u64;
                                }
                                for i in 0..3 {
                                    DPROF_TRACE_CONTROL[i] +=
                                        retired * b.control[i] as u64 / b.n as u64;
                                }
                                for i in 0..5 {
                                    DPROF_TRACE_ALU[i] += retired * b.alu[i] as u64 / b.n as u64;
                                }
                            }
                        }
                    }
                }
            }
            retired_sum += retired;
            chained += 1;
            // A block that retired nothing bailed on its very first instruction
            // (TLB miss / MMIO / FP fast-path). It makes no progress, so stop
            // chaining and let the interpreter handle that instruction — never
            // spin re-calling it.
            if retired == 0 {
                unsafe { ZERO_RETIRE += 1 };
                if dprof_sample {
                    let fcsr = m.cpu().fcsr;
                    let fs = m.cpu().sys.as_ref().map_or(3, |c| (c.mstatus >> 13) & 3);
                    unsafe {
                        if fcsr & 1 == 0 {
                            ZR_NX += 1;
                        }
                        if (fcsr >> 5) & 7 != 0 {
                            ZR_FRM += 1;
                        }
                        if fs != 3 {
                            ZR_FS += 1;
                        }
                    }
                }
                break;
            }
        }
        m.cpu_mut().insn_count += retired_sum;
        unsafe {
            JIT_RETIRED += retired_sum;
            JIT_DISPATCHES += chained as u64;
        }
        remaining = remaining.saturating_sub(retired_sum);

        // If we stopped only because we hit the chain cap (the next pc is still
        // compiled and making progress), keep running in the JIT: advance the
        // clock and service interrupts here — the interrupt/timer work the
        // interpreter slice below used to do — instead of dropping to a wasteful
        // ~256-insn interp slice. This is the difference between ~50% and ~95%
        // JIT coverage on branchy, deeply-chained workloads (the CPython eval
        // loop). (`chained == CAP` can only be reached when every block in the
        // batch retired > 0, since a zero-retire block breaks above.)
        if remaining == 0 {
            break;
        }
        if chained == JIT_CHAIN_CAP || retired_sum >= round_budget {
            // Quantum boundary: re-anchor the wall clock BEFORE advancing
            // devices — a full quantum (1M insns) can pass in well under the
            // interpolation model's assumptions when bulk fast paths run,
            // and mtime must track real time, not extrapolated time.
            m.wallclock_quantum_boundary();
            m.sync_and_check_interrupts();
            chain_ctl_boundary(m.cpu().insn_count);
            // Extension FIRST: a landed region whose measured exits keep
            // leaving it grows along that traffic. Extensions outrank fresh
            // page builds for the build budget — a fresh 3-page function
            // over call-heavy code exits immediately and holds nothing,
            // while an extension is provably where dispatches are lost
            // (drained behind the page queue, tcc got 3 extensions against
            // 179 page builds and kept its 8-insn dispatches).
            if measured_region_policy_enabled() {
                drain_ext_queue(m, jit);
            }
            // Then spend what's left on the oldest deferred page that still
            // resolves in the CURRENT address space (issuing an extension
            // above moved SB_LAST_ICOUNT, so at most one build per boundary).
            if !jit.sb_queue.is_empty() && sb_build_allowed(m.cpu().insn_count) {
                let aspace = m.cpu().sys.as_ref().map_or(0, |c| c.satp);
                if let Some(i) = jit.sb_queue.iter().position(|&(a, _)| a == aspace) {
                    let (_, vpage) = jit.sb_queue.remove(i);
                    let compiles = jit.sb_gen.get(&(aspace, vpage)).map_or(0, |&(_, c, _)| c);
                    if let Some(pa) = m.probe_fetch(vpage) {
                        if pa >= rv64_system::RAM_BASE
                            && ((pa & !0xfff) - rv64_system::RAM_BASE) as usize + 0x1000
                                <= m.ram().len()
                        {
                            build_superblock(m, jit, aspace, vpage, pa & !0xfff, compiles);
                        }
                    }
                }
            }
            continue;
        }

        // Extension drain in USER context: the chain just broke while the
        // guest code that queued the work is the one running (the quantum
        // boundary above misses dispatch-heavy phases entirely).
        if measured_region_policy_enabled() {
            drain_ext_queue(m, jit);
        }

        // --- hot counting + compile (from physical code bytes) ---
        let pc = m.cpu().pc;
        // Address space this discovery belongs to (satp; 0 in bare mode).
        let aspace = m.cpu().sys.as_ref().map_or(0, |c| c.satp);
        if unsafe { DPROF_ON } {
            let pa = m.probe_fetch(pc);
            if let Some(pa) = pa {
                let o = (pa.wrapping_sub(rv64_system::RAM_BASE)) as usize;
                if pa >= rv64_system::RAM_BASE && o + 4 <= m.ram().len() {
                    ihist_hit(u32::from_le_bytes([
                        m.ram()[o],
                        m.ram()[o + 1],
                        m.ram()[o + 2],
                        m.ram()[o + 3],
                    ]));
                }
            }
        }
        if !unsafe { PAGE_POLICY_ENABLED } && !jit_table_full() && !jit.cache.contains_key(&pc) {
            let hot = {
                let c = jit.hot.entry(pc).or_insert(0);
                *c += 1;
                *c
            };
            if hot >= unsafe { JIT_THRESHOLD } {
                if let Some(pa) = m.probe_fetch(pc) {
                    if pa >= rv64_system::RAM_BASE {
                        let cpu = m.cpu();
                        let sysmem = jit_system_memory(cpu);
                        let lay = rv64_dbt::JitLayout {
                            x_base: cpu.x.as_ptr() as u32,
                            pc_addr: &cpu.pc as *const u64 as u32,
                            mem: None,
                            sys: Some(sysmem),
                            mem_profile: mem_profile_layout(),
                            reg_stress: reg_stress(),
                            reg_profile_base: reg_profile_base(),
                            structured_profile: structured_profile_layout(),
                            multi_latch: unsafe { MULTI_LATCH },
                            retired_addr: retired_addr(),
                            f_base: cpu.f.as_ptr() as u32,
                            fcsr_addr: &cpu.fcsr as *const u32 as u32,
                            reservation: Some(rv64_dbt::ReservationCapability::System),
                            fuel_addr: fuel_addr(),
                            mstatus_addr: cpu.jit_mstatus_ptr() as u32,
                            copystat_addr: copystat_addr(),
                            chain_off_addr: chain_off_addr(),
                            batch_base_addr: 0,
                            dispatch_base: jit.dispatch.as_ptr() as u32,
                            dispatch_mask: (DISPATCH_SIZE - 1) as u32,
                            map_gen_addr: cpu.jit_map_gen_ptr() as u32,
                            chain_hops_addr: chain_hops_addr(),
                            ic_miss_owner_addr: ic_miss_owner_addr(),
                            ic_miss_target_addr: ic_miss_target_addr(),
                            pic_code_base: None,
                        };
                        let vpage = pc & !0xfff;
                        let pa_page = pa & !0xfff;
                        let pa_page_off = (pa_page - rv64_system::RAM_BASE) as usize;
                        let off = (pa - rv64_system::RAM_BASE) as usize;
                        let end = ((off + 1024).min(off | 0xfff) + 1).min(m.ram().len());
                        // Superblock path (opt-in): loop headers stay individual
                        // (tight wasm loop); non-loop pages accumulate entries and
                        // upgrade to a page superblock once branchy enough.
                        let (is_loop, n_entries) = if unsafe { SYS_SUPERBLOCK } {
                            let il = rv64_dbt::is_loop_at(&m.ram()[off..end], pc, pc, lay);
                            let ne = if il {
                                0
                            } else {
                                let e = jit.page_entries.entry((aspace, vpage)).or_default();
                                if let Err(i) = e.binary_search(&pc) {
                                    e.insert(i, pc);
                                }
                                e.len()
                            };
                            (il, ne)
                        } else {
                            (false, 0)
                        };

                        let (sb_last, sb_compiles, sb_when) = jit
                            .sb_gen
                            .get(&(aspace, vpage))
                            .copied()
                            .unwrap_or((0, 0, 0));
                        // Rebuilding a page function DISCARDS the optimized
                        // code V8 built for it: the replacement module starts
                        // in the baseline compiler again. Measured, a page that
                        // kept rebuilding every couple of seconds ran ~3x
                        // slower with identical coverage (nbench FP EMULATION:
                        // 444 insns/dispatch either way, 2568 -> 820 MIPS), and
                        // nbench itself flagged the result as statistically
                        // uncertain. So each rebuild costs the page an
                        // exponentially longer quiet period.
                        let cooldown = SB_PAGE_COOLDOWN << sb_compiles.min(6);
                        let sb_cool = m.cpu().insn_count >= sb_when.wrapping_add(cooldown);
                        // Recompile on DOUBLING, not on a fixed increment: a
                        // page discovered 6 hot pcs at a time would need 20
                        // recompiles to cover the 120 that nbench IDEA ends up
                        // with, so a fixed cap left most of the page on
                        // individual blocks forever. Doubling covers a page of
                        // any size in a handful of compiles and is
                        // self-amortizing — each one costs at most as much as
                        // all the previous ones together.
                        let sb_spaced = sb_build_allowed(m.cpu().insn_count);
                        let sb_want = if jit.superblocked.contains(&(aspace, vpage)) {
                            // Recompile when the page has grown by half again,
                            // OR as soon as the page function has visibly
                            // fallen behind: SB_MISSED_TRIGGER hot pcs on it
                            // needed their own blocks. Growth alone raced —
                            // a page that ends at 120 hot pcs after compiling
                            // at 96 never doubles again, and which of its
                            // functions the page function happened to cover
                            // decided whether nbench IDEA scored 1600 or 4400
                            // iter/s from identical runs.
                            // Rebuild when the page function has visibly
                            // fallen behind: enough hot pcs needed their own
                            // blocks, scaled to what it already covers. A flat
                            // trigger burned the whole recompile allowance
                            // during warmup (16 rebuilds while barely a handful
                            // of pcs were hot), after which the bulk of
                            // cipher_idea — which only gets hot later — could
                            // never be covered: nbench IDEA scored 1600 instead
                            // of 4400 iter/s depending on that race.
                            let missed = jit.sb_missed.get(&(aspace, vpage)).copied().unwrap_or(0);
                            // Rebuild when enough hot pcs have had to build
                            // their own blocks, scaled to what the page
                            // function already covers. The counter is reset by
                            // each build, so this converges: a page only keeps
                            // rebuilding while it keeps discovering hot code.
                            sb_cool
                                && missed >= SB_MISSED_TRIGGER.max(sb_last as u32 / 4)
                                && (n_entries > sb_last || sb_compiles < SB_RECOMPILE_CAP)
                        } else {
                            n_entries >= SUPERBLOCK_THRESHOLD
                        };
                        if !is_loop && sb_want && pa_page_off + 0x1000 <= m.ram().len() {
                            if sb_spaced {
                                build_superblock(m, jit, aspace, vpage, pa_page, sb_compiles);
                            } else {
                                // Budget says not yet: remember the page and
                                // build it at a later quantum boundary rather
                                // than dropping the request — a page whose hot
                                // pcs all appear inside one budget window would
                                // otherwise never be revisited (nbench IDEA
                                // fell back to 6.4 insns/dispatch).
                                if jit.sb_queue.len() < SB_QUEUE_CAP
                                    && !jit.sb_queue.contains(&(aspace, vpage))
                                {
                                    jit.sb_queue.push((aspace, vpage));
                                }
                            }
                        }
                        if pc == unsafe { TRACE_PC } {
                            unsafe { TRACE_INDIV += 1 };
                        }
                        // The rebuild-pressure count moves BELOW, after the
                        // block exists: only SHORT blocks count as misses.
                        // Long traces would not be claimed by a page function
                        // anyway (TRACE_KEEP_MIN), and in the trace world new
                        // hot pcs are minted continuously (side-exit
                        // targets), so counting every one drove PERPETUAL
                        // rebuilds that discarded V8-optimized functions —
                        // the measured 3x churn cliff, back from the dead.
                        let missed_here = !is_loop && jit.superblocked.contains(&(aspace, vpage));
                        // Individual block (loop or pre-threshold non-loop).
                        // Deliberately NOT deferred while a page function is on
                        // its way: making hot pcs wait for one stalls code
                        // behind an async compile that may never land — that
                        // was measured as 138M retries and a 10x slowdown once
                        // pending builds backed up.
                        // The measured default supplies the current 4KB page.
                        // TRACE_WINDOW_MODE can instead select the legacy
                        // aligned 64-page gather (see TraceWin), in which case
                        // a trace may follow calls within 256KB and registers
                        // every page its final span covers. Multi-page spans
                        // ride the regions pa-verify.
                        let single_page = match unsafe { TRACE_WINDOW_MODE } {
                            1 => true,
                            2 => false,
                            _ => rv64_dbt::trace_level() == 0,
                        };
                        let first_va = if single_page {
                            vpage
                        } else {
                            vpage & !TRACE_WIN_MASK
                        };
                        let wins = unsafe { &mut TRACE_WIN };
                        // Unprocessed dirty pages force a re-gather: the
                        // buffer may predate the store (a fresh gather reads
                        // current RAM, so it is always safe to rebuild).
                        if m.dirty_pages_pending() {
                            wins.clear();
                        }
                        let mg = m.cpu().map_gen;
                        let bg = unsafe { BOOT_GEN };
                        let hit = wins.iter().position(|w| {
                            w.aspace == aspace
                                && w.map_gen == mg
                                && w.boot_gen == bg
                                && w.first_va == first_va
                        });
                        let npages = if single_page { 1 } else { TRACE_WIN_PAGES };
                        let wi = match hit {
                            Some(i) => i,
                            None => {
                                let mut w = TraceWin {
                                    aspace,
                                    map_gen: mg,
                                    boot_gen: bg,
                                    first_va,
                                    pages: Vec::new(),
                                    buf: vec![0u8; (npages * 0x1000) as usize],
                                };
                                for k in 0..npages {
                                    let va = first_va + k * 0x1000;
                                    if let Some(p) = m.probe_fetch(va) {
                                        let pp = p & !0xfff;
                                        if pp >= rv64_system::RAM_BASE
                                            && (pp - rv64_system::RAM_BASE) as usize + 0x1000
                                                <= m.ram().len()
                                        {
                                            let o = (pp - rv64_system::RAM_BASE) as usize;
                                            let bo = (k * 0x1000) as usize;
                                            w.buf[bo..bo + 0x1000]
                                                .copy_from_slice(&m.ram()[o..o + 0x1000]);
                                            w.pages.push((va, pp));
                                        }
                                    }
                                }
                                if wins.len() >= TRACE_WIN_CACHE {
                                    wins.remove(0);
                                }
                                wins.push(w);
                                wins.len() - 1
                            }
                        };
                        let w = &wins[wi];
                        let winpages = &w.pages;
                        unsafe { COMPILES_TICK += 1 };
                        // BATCH: compile this pc together with its fixed-
                        // target successors as one module whose members
                        // tail-call each other directly (~2ns/hop, no table
                        // import, O(1) registration). Falls back to the
                        // single-block path whenever a batch can't form.
                        let batch_t0 = unsafe { host_now_ms() };
                        let cell = unsafe {
                            let c = BATCH_CELL_NEXT;
                            BATCH_CELL_NEXT = (c + 1) % BATCH_CELLS;
                            c
                        };
                        let batch = if unsafe { BATCH_ON }
                            && jit.cache.len() < unsafe { BATCH_POP_CAP }
                            && !w.pages.is_empty()
                        {
                            let mut blay = lay;
                            blay.batch_base_addr = batch_cell_addr(cell);
                            let cache = &jit.cache;
                            let hotmap = &jit.hot;
                            let hot = |t: u64| matches!(cache.get(&t), Some(Some(b)) if b.idx >= 0);
                            let wlo = w.first_va;
                            let whi = w.first_va + (TRACE_WIN_PAGES * 0x1000);
                            let pages = &w.pages;
                            // Members must be PROVEN hot: taking every exit
                            // target compiled ~24 blocks per tier-up, most
                            // never executed — a compile storm that ran
                            // python fib 35x slower (173s). Warm pcs only
                            // (half the tier-up threshold) keeps a batch to
                            // the successor set actually being executed.
                            let bar = unsafe { JIT_THRESHOLD >> BATCH_BAR_SHIFT };
                            // Already-compiled successors DO join (the batch
                            // supersedes them in the cache; the old block just
                            // becomes unreachable): requiring uncompiled pcs
                            // meant batches almost never formed with 2+
                            // members, since a hot pc's successors are
                            // normally compiled before it. Loop headers are
                            // excluded — their tight wasm regions beat any
                            // trace — as are superblock entries (n == 0).
                            // BATCH_PAGE: co-locate the hot pcs of the seed's
                            // OWN page. Successor-seeded batches only reached
                            // ~12% in-batch exits; if the per-dispatch cost is
                            // dominated by V8 instance switches (each block
                            // module is its own instance), packing a page's
                            // blocks into one instance pays on EVERY dispatch
                            // between them, links or not.
                            let seedpage = pc & !0xfff;
                            let page_mode = unsafe { BATCH_PAGE };
                            let want = |t: u64| {
                                t >= wlo
                                    && t < whi
                                    && pages.iter().any(|&(va, _)| va == t & !0xfff)
                                    && hotmap.get(&t).is_some_and(|&c| c >= bar)
                                    && !matches!(cache.get(&t), Some(Some(b)) if b.n == 0)
                                    && (!page_mode || t & !0xfff == seedpage)
                            };
                            let succ = &jit.succ;
                            // Observed successor of a pc, when we have one.
                            let next = |t: u64| {
                                let e = succ[JitState::dslot(t)];
                                e.hottest(t, 1)
                            };
                            let translated = rv64_dbt::translate_batch_obs(
                                &w.buf,
                                w.first_va,
                                pc,
                                blay,
                                &hot,
                                &want,
                                &next,
                                unsafe { BATCH_CAP },
                            );
                            unsafe {
                                SYS_TRANSLATE_ATTEMPTS += 1;
                                SYS_TRANSLATE_NS +=
                                    ((host_now_ms() - batch_t0).max(0.0) * 1_000_000.0) as u64;
                                if let Some((bytes, _)) = &translated {
                                    SYS_EMITTED_BYTES += bytes.len() as u64;
                                }
                                if let Some((_, members)) = &translated {
                                    SYS_DENSE_COPY_MEMBERS +=
                                        members.iter().filter(|member| member.dense_copy).count()
                                            as u64;
                                    SYS_DENSE_STORE_MEMBERS +=
                                        members.iter().filter(|member| member.dense_store).count()
                                            as u64;
                                    SYS_BULK_COPY_MEMBERS +=
                                        members.iter().filter(|member| member.bulk_copy).count()
                                            as u64;
                                }
                            }
                            translated
                        } else {
                            None
                        };
                        if let Some((wasm, members)) = batch {
                            unsafe {
                                SB_BUILD_MS += host_now_ms() - batch_t0;
                                SB_LAST_ICOUNT = m.cpu().insn_count;
                            }
                            // RATE GOVERNOR. The gates that separate a
                            // workload batching PAYS for from one it does
                            // not are neither population nor footprint
                            // (both accumulate kernel/boot code and fire
                            // for everyone) — it is how FAST batches are
                            // demanded. nbench ASSIGNMENT wants a few dozen
                            // over tens of billions of instructions; CPython
                            // wants thousands inside its first second, and
                            // pays a batch compile per tier-up for code it
                            // never re-enters (python fib 3.7s -> 180s).
                            // Once the observed rate proves that shape,
                            // batching switches off for the rest of the run.
                            unsafe {
                                // Deferring this verdict until the guest is
                                // warm was tried and is WORSE: python's
                                // storm resumes unchecked (all runs time
                                // out) while ASSIGNMENT still gains nothing.
                                // Judging from the first batch on is what
                                // produced python's MATCH.
                                let gi = (m.cpu().insn_count / 1_000_000_000).max(1);
                                if BATCHES > 64 && BATCHES / gi > BATCH_RATE_CAP {
                                    BATCH_ON = false;
                                }
                            }
                            if members.len() >= 2 {
                                let n = members.len() as u32;
                                unsafe { JIT_OUT = wasm };
                                let bbase = unsafe { host_jit_register_batch(n) };
                                if bbase >= 0 {
                                    unsafe {
                                        BATCH_BASE_POOL[cell] = bbase as u32;
                                        JIT_TABLE_ENTRIES += n as u64;
                                        BATCHES += 1;
                                        BATCH_MEMBERS += n as u64;
                                    }
                                    for (j, mb) in members.iter().enumerate() {
                                        let (lo, hi) = if mb.span == (0, 0) {
                                            (mb.pc, mb.pc + 2)
                                        } else {
                                            mb.span
                                        };
                                        let mut mpa = 0u64;
                                        let mut spanned: Vec<(u64, u64)> = Vec::new();
                                        let mut va = lo & !0xfff;
                                        let mut okp = true;
                                        while va <= (hi - 1) & !0xfff {
                                            match w.pages.iter().find(|&&(v, _)| v == va) {
                                                Some(&(_, pp)) => {
                                                    if va == mb.pc & !0xfff {
                                                        mpa = pp + (mb.pc & 0xfff);
                                                    }
                                                    spanned.push((va, pp));
                                                }
                                                None => {
                                                    okp = false;
                                                    break;
                                                }
                                            }
                                            va += 0x1000;
                                        }
                                        if !okp || mpa == 0 {
                                            continue;
                                        }
                                        for &(_, pp) in &spanned {
                                            m.mark_jit_page(pp);
                                        }
                                        let idx = bbase + j as i32;
                                        if spanned.len() > 1 {
                                            jit.regions.insert(idx, spanned.clone());
                                        }
                                        let b = JitBlock {
                                            fp: mb.uses_fp,
                                            idx,
                                            n: mb.n_insns,
                                            mix: mb.trace_mix,
                                            mem: mb.trace_mem,
                                            control: mb.trace_control,
                                            alu: mb.trace_alu,
                                            pa: mpa,
                                        };
                                        if jit.cache.insert(mb.pc, Some(b)).is_none() {
                                            for &(_, pp) in &spanned {
                                                jit.page_blocks
                                                    .entry((pp - rv64_system::RAM_BASE) >> 12)
                                                    .or_default()
                                                    .push(mb.pc);
                                            }
                                        }
                                        if unsafe { SYS_SUPERBLOCK } {
                                            for &sd in &mb.seeds {
                                                let e = jit
                                                    .page_entries
                                                    .entry((aspace, sd & !0xfff))
                                                    .or_default();
                                                if let Err(i) = e.binary_search(&sd) {
                                                    e.insert(i, sd);
                                                }
                                            }
                                        }
                                    }
                                    m.cpu_mut().clear_store_jtlb();
                                    continue; // dispatch the seed member now
                                }
                            }
                        }
                        let pic_targets = jit
                            .ic_targets
                            .get(&pc)
                            .filter(|targets| targets.len() >= 2)
                            .cloned();
                        let pic_capture = pic_targets
                            .as_deref()
                            .and_then(|targets| capture_pic_pages(m, pc, targets));
                        let translate_t0 = unsafe { host_now_ms() };
                        let blk = {
                            // Hotness oracle for branch-direction bias: a
                            // compiled (non-blacklisted) target is proven-hot.
                            let cache = &jit.cache;
                            let hot = |t: u64| matches!(cache.get(&t), Some(Some(b)) if b.idx >= 0);
                            // Inline-cache oracle: the target this pc's
                            // indirect jump was last observed to take.
                            let succ = &jit.succ;
                            let ic_targets = &jit.ic_targets;
                            let next = |t: u64| {
                                let e = succ[JitState::dslot(t)];
                                ic_targets
                                    .get(&t)
                                    .and_then(|targets| targets.first().copied())
                                    .or_else(|| e.hottest(t, unsafe { IC_EXTEND_TRIGGER }))
                            };
                            match (pic_targets.as_deref(), pic_capture.as_ref()) {
                                (Some(targets), Some(capture)) => {
                                    rv64_dbt::translate_block_pic_sparse(
                                        &capture.buf,
                                        &capture.page_vas,
                                        pc,
                                        lay,
                                        targets,
                                    )
                                }
                                _ => rv64_dbt::translate_block_ic(
                                    &w.buf,
                                    w.first_va,
                                    pc,
                                    lay,
                                    &hot,
                                    &|_| None,
                                    &next,
                                ),
                            }
                        };
                        unsafe {
                            SYS_TRANSLATE_ATTEMPTS += 1;
                            SYS_TRANSLATE_NS +=
                                ((host_now_ms() - translate_t0).max(0.0) * 1_000_000.0) as u64;
                            if let Some(region) = &blk {
                                SYS_EMITTED_BYTES += region.wasm.len() as u64;
                                SYS_DENSE_COPY_MEMBERS += u64::from(region.dense_copy_members);
                                SYS_DENSE_STORE_MEMBERS += u64::from(region.dense_store_members);
                                SYS_BULK_COPY_MEMBERS += u64::from(region.bulk_copy_members);
                            }
                        }
                        let pic_pages = pic_capture.as_ref().map(|capture| capture.pages.clone());
                        let entry = blk.and_then(|blk| {
                            // Pages the emitted code actually came from
                            // ((0,0) span = wholly within [pc, pc+len)).
                            let spanned = if let Some(pages) = &pic_pages {
                                // Sparse targets can be arbitrarily far apart;
                                // retain their explicit mapping rather than
                                // treating the min/max span as contiguous.
                                pages.clone()
                            } else {
                                let (lo, hi) = if blk.span == (0, 0) {
                                    (pc, pc + blk.len.max(2))
                                } else {
                                    blk.span
                                };
                                let mut pages = Vec::new();
                                let mut va = lo & !0xfff;
                                while va <= (hi - 1) & !0xfff {
                                    let Some(&(_, pp)) = winpages.iter().find(|&&(v, _)| v == va)
                                    else {
                                        return None; // span escaped the window (impossible)
                                    };
                                    pages.push((va, pp));
                                    va += 0x1000;
                                }
                                pages
                            };
                            unsafe { JIT_OUT = blk.wasm };
                            let idx = unsafe { host_jit_register() };
                            if idx < 0 {
                                return None;
                            }
                            unsafe { JIT_TABLE_ENTRIES += 1 };
                            for &(_, pp) in &spanned {
                                m.mark_jit_page(pp);
                            }
                            m.cpu_mut().clear_store_jtlb(); // these pages may now hold code
                            if spanned.len() > 1 {
                                jit.regions.insert(idx, spanned.clone());
                            }
                            Some((
                                JitBlock {
                                    fp: blk.uses_fp,
                                    idx,
                                    n: blk.n_insns,
                                    mix: blk.trace_mix,
                                    mem: blk.trace_mem,
                                    control: blk.trace_control,
                                    alu: blk.trace_alu,
                                    pa,
                                },
                                spanned,
                                blk.seeds,
                            ))
                        });
                        if missed_here {
                            let short = match &entry {
                                Some((b, _, _)) => b.n < unsafe { TRACE_KEEP_MIN },
                                None => true, // untranslatable: function coverage wanted
                            };
                            if short {
                                *jit.sb_missed.entry((aspace, vpage)).or_insert(0) += 1;
                                unsafe { SB_INDIV += 1 };
                            }
                        }
                        match entry {
                            Some((b, spanned, seeds)) => {
                                // Trace exit targets are hot-path block
                                // leaders: feed them to superblock discovery,
                                // which trace compilation otherwise starves
                                // (interior pcs never tier up on their own,
                                // so page functions built from a handful of
                                // seeds covered fragments and measured
                                // catastrophically without the demotion
                                // safety valve).
                                if unsafe { SYS_SUPERBLOCK } {
                                    for &sd in &seeds {
                                        let e = jit
                                            .page_entries
                                            .entry((aspace, sd & !0xfff))
                                            .or_default();
                                        if let Err(i) = e.binary_search(&sd) {
                                            e.insert(i, sd);
                                        }
                                    }
                                }
                                if jit.cache.insert(pc, Some(b)).is_none() {
                                    for &(_, pp) in &spanned {
                                        jit.page_blocks
                                            .entry((pp - rv64_system::RAM_BASE) >> 12)
                                            .or_default()
                                            .push(pc);
                                    }
                                }
                                continue;
                            }
                            // Untranslatable at THESE code bytes: blacklist with
                            // a pa-stamped sentinel (idx = -1). It's re-verified
                            // like a real block (map_gen / dispatch probe) so it
                            // survives context switches without poisoning a
                            // different address space at the same va, and the
                            // dirty-page tracker naturally drops it if the code
                            // bytes are overwritten.
                            None => {
                                m.mark_jit_page(pa);
                                m.cpu_mut().clear_store_jtlb();
                                let jb = JitBlock {
                                    fp: false,
                                    idx: -1,
                                    n: 0,
                                    mix: [0; 5],
                                    mem: [0; 10],
                                    control: [0; 3],
                                    alu: [0; 5],
                                    pa,
                                };
                                if jit.cache.insert(pc, Some(jb)).is_none() {
                                    jit.page_blocks
                                        .entry((pa - rv64_system::RAM_BASE) >> 12)
                                        .or_default()
                                        .push(pc);
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- interpreter + devices ---
        if unsafe { PAGE_POLICY_ENABLED } {
            let icount_before = m.cpu().insn_count;
            let precise_stop = !jit.cache.is_empty();
            // Once this exact entry on this VA->PA mapping has already been
            // attempted or installed, page_policy_observe will return before
            // adding heat or changing the candidate set. Privileged CSR and
            // other deliberate side-exit PCs can be visited hundreds of
            // thousands of times by Linux; running the control-entry sampler
            // on every such visit is pure profiling overhead. Retain exact
            // stop-at-compiled behavior, but use the unsampled interpreter for
            // entries whose policy outcome is already final.
            let policy_entry = {
                let pc = m.cpu().pc;
                m.probe_fetch(pc).map(|pa| (pc, pc & !0xfff, pa & !0xfff))
            };
            let policy_sample_needed = policy_entry.is_none_or(|(pc, vpage, pa_page)| {
                if pa_page < rv64_system::RAM_BASE {
                    return true;
                }
                let key = (vpage, pa_page);
                !jit.policy_attempted
                    .get(&key)
                    .is_some_and(|entries| entries.contains(&pc))
                    && !jit
                        .policy_installed
                        .get(&key)
                        .is_some_and(|entries| entries.contains(&pc))
            });
            // The pointer stays stable during the sampled call: observation
            // only updates page-policy maps/queues, and async callbacks cannot
            // run until this Wasm invocation returns.
            let dispatch = jit.dispatch.as_ptr();
            let mut stop_at_compiled =
                |pc| unsafe { (*dispatch.add(JitState::dslot(pc))).pc == pc };
            let (ran, yielded) = if policy_sample_needed {
                let mut observe = |sample| page_policy_observe(jit, sample);
                m.run_policy_interpreter(
                    remaining.min(4096),
                    &mut observe,
                    &mut stop_at_compiled,
                    precise_stop,
                    unsafe { PAGE_POLICY_CONTROL_ENTRIES || PAGE_POLICY_CONTROL_PROFILE },
                )
            } else {
                m.run_interpreter_until(remaining.min(4096), &mut stop_at_compiled)
            };
            unsafe {
                SLICE_CALLS += 1;
                SLICE_INSNS += ran;
                if DPROF_ON && IHIST_LAST != usize::MAX {
                    IHIST_INSNS[IHIST_LAST] += m.cpu().insn_count - icount_before;
                    IHIST_LAST = usize::MAX;
                }
            }
            page_policy_issue(m, jit);
            if ran == 0 {
                // A precise guest exception can redirect to a compiled trap
                // handler without retiring the faulting instruction. That is
                // architectural progress, not a host yield. Bound repeated
                // non-retiring traps by one driver-budget unit each while
                // leaving Cpu::insn_count exact. WFI has its own explicit
                // stop flag and must return to JavaScript immediately.
                if yielded {
                    break;
                }
                remaining = remaining.saturating_sub(1);
                continue;
            }
            remaining = remaining.saturating_sub(ran);
            // Match the exact interpreter scheduler contract: WFI returns to
            // JavaScript even when the slice retired instructions before it.
            // Without this signal the JIT driver consumed the caller's entire
            // budget across idle wakeups, adding millions of unrelated shell
            // instructions to short workload timings and delaying host I/O.
            if yielded {
                break;
            }
        } else if jit.cache.is_empty() {
            // Cold: no compiled blocks to return to — one big slice avoids
            // dispatch churn before any block exists.
            let ran = m.run_interpreter(remaining.min(4096));
            unsafe {
                SLICE_CALLS += 1;
                SLICE_INSNS += ran;
            }
            if ran == 0 {
                // WFI made no architectural progress. Return to JavaScript so
                // asynchronous devices (notably an external 9P backend) can
                // deliver the event the guest is waiting for. Spending the
                // budget one synthetic instruction at a time spins millions
                // of idle interpreter calls without giving the host a turn.
                break;
            }
            remaining = remaining.saturating_sub(ran);
        } else {
            // Warm: interpret ONLY the uncompiled stretch — stop the moment pc
            // reaches a compiled block again. A fixed warm slice overshoots into
            // compiled code and runs it in the interpreter; on the CPython eval
            // loop that overshoot was ~half of all instructions (2.8M slices ×
            // 256 insns). Run in small chunks, checking the (cheap, direct-
            // mapped) dispatch cache between them.
            // Interpret only the uncompiled stretch, stopping the instant pc
            // reaches a hot compiled block — a fixed slice would overshoot into
            // compiled code and run it in the interpreter (on the CPython eval
            // loop that overshoot was ~half of all instructions). The first
            // instruction always runs (pc may be a block that just bailed here),
            // so no spin.
            // Stop when pc reaches a compiled block; ALSO hot-count each
            // uncompiled pc and stop once it's hot enough, so the interior of an
            // interpreted stretch actually reaches the compile threshold (else
            // run_slice_until would interpret the whole stretch forever without
            // any of its blocks ever tiering up — that residual is ~half of
            // fib's wall time).
            let icount_before = m.cpu().insn_count;
            let mut stop_at_compiled = |pc| {
                if jit.dispatch[JitState::dslot(pc)].pc == pc {
                    return true;
                }
                let slot = JitState::dslot(pc);
                let tag = (pc >> 1) as u32;
                if jit.interp_hot_tag[slot] != tag {
                    // different pc aliased here: heat belongs to someone else
                    jit.interp_hot_tag[slot] = tag;
                    jit.interp_hot[slot] = 0;
                }
                let cnt = &mut jit.interp_hot[slot];
                *cnt = cnt.saturating_add(1);
                if *cnt < INTERP_HOT_THRESHOLD {
                    return false; // cold: cheap array bump only, no HashMap
                }
                // Hot stretch interior: force it onto the fast-path hot map so
                // the compile step tiers it up, and stop interpreting here.
                jit.hot.insert(pc, unsafe { JIT_THRESHOLD });
                true
            };
            let (ran, yielded) =
                m.run_interpreter_until(remaining.min(SYS_WARM_SLICE), &mut stop_at_compiled);
            unsafe {
                SLICE_CALLS += 1;
                SLICE_INSNS += ran;
                if DPROF_ON && IHIST_LAST != usize::MAX {
                    // Charge the whole interpreted stretch to whatever the JIT
                    // gave up on: one unsupported instruction can drag dozens
                    // of interpreted instructions behind it.
                    IHIST_INSNS[IHIST_LAST] += m.cpu().insn_count - icount_before;
                    IHIST_LAST = usize::MAX;
                }
            }
            if ran == 0 {
                // An exception redirected to a compiled trap handler retires
                // no instruction but has changed architectural state. Do not
                // confuse it with WFI: charge one driver-budget attempt and
                // let generated handler code run. This also prevents a guest
                // with an endlessly faulting zero-retirement path from
                // monopolizing the host call.
                if yielded {
                    break;
                }
                remaining = remaining.saturating_sub(1);
                continue;
            }
            remaining = remaining.saturating_sub(ran);
            // WFI may retire after a non-empty interpreted stretch. Preserve
            // that stop reason instead of continuing within this host call and
            // executing whatever bytes follow the halted instruction.
            if yielded {
                break;
            }
        }

        // Stream console output at quantum granularity, DURING execution —
        // buffering until sys_run returns skews benchmark timing: a marker
        // printed early in a slice would be timestamped after the whole slice
        // (v86 timestamps serial bytes as they arrive; symmetry demands we
        // surface output comparably; see PERFORMANCE_PROGRESS.md).
        m.flush_host_io();
    }

    m.flush_host_io();
    m.power_off() as i32
}

/// Deliver one inbound Ethernet frame (staged via staging_alloc) to the NIC.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_net_input() {
    let m = unsafe { SYS.as_mut().expect("call sys_boot() first") };
    unsafe {
        let frame = core::mem::take(&mut STAGING);
        m.net_input(&frame);
    }
}

/// Move the guest's frames one step: into the in-process proxy when one is
/// running, otherwise out to the page's relay.
#[allow(static_mut_refs)]
fn pump_net(m: &mut rv64_system::Machine) {
    unsafe {
        match SYS_NETSTACK.as_mut() {
            Some(stack) => {
                for frame in m.net_take_output() {
                    stack.input(&frame);
                }
                if let Some(proxy) = SYS_PROXY.as_mut() {
                    proxy.pump(stack, &mut SYS_EGRESS);
                } else if SYS_WISP {
                    pump_wisp(stack);
                }
                for frame in stack.take_output() {
                    m.net_input(&frame);
                }
            }
            None => {
                for frame in m.net_take_output() {
                    host_net_send(frame.as_ptr(), frame.len())
                }
            }
        }
    }
}

/// Send keyboard bytes (staged via staging_alloc) to the guest console.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_console_input() {
    let m = unsafe { SYS.as_mut().expect("call sys_boot() first") };
    unsafe {
        let bytes = core::mem::take(&mut STAGING);
        m.console_input(&bytes);
    }
}

/// Nested chain-transfer depth (see chain_next) and its bound: each hop
/// holds two wasm frames (the calling block + chain_next) until the chain
/// unwinds, so the cap bounds stack use; a chain that reaches it simply
/// returns to the host loop, which re-dispatches seamlessly.
static mut CHAIN_DEPTH: u32 = 0;
const CHAIN_DEPTH_CAP: u32 = 64;
static mut CHAIN_HOPS: u64 = 0;
static mut REGION_CHAIN_ENABLED: bool = false;

fn chain_hops_addr() -> u32 {
    (&raw const CHAIN_HOPS) as u32
}

/// Block-to-block transfer WITHOUT the shared-table import: generated trace
/// blocks call this main-module export as a FUNCTION import (env.chain_next
/// — the same wasm-to-wasm shape as env.tlb_fill, which thousands of block
/// modules already import with no penalty). Importing the function table
/// instead made every table.set O(importing instances) on this V8 —
/// quadratic registration across tcc's 7.5k blocks — which is why emitted
/// return_call_indirect chaining is off. Here the dispatch-line fast path
/// (pc match under the current map generation, blacklist, fuel) runs in
/// ONE place in Rust and the transfer is a plain indirect call through the
/// table the main module owns.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn chain_next(state: i32) {
    unsafe {
        if CHAIN_DEPTH >= CHAIN_DEPTH_CAP {
            return;
        }
        // Fuel: the cumulative retired cell against this dispatch's grant.
        if RETIRED_CELL >= FUEL_CELL {
            return;
        }
        let (pc, map_gen) = match ACTIVE_SYSTEM_KIND {
            kind if kind == SystemMachineKind::Legacy as u8 => {
                let machine = &*(state as *const rv64_system::Machine);
                (machine.cpu.pc, machine.cpu.map_gen as u32)
            }
            kind if kind == SystemMachineKind::Virt as u8 => {
                let machine = &*(state as *const rv64_system::virt::VirtMachine);
                (machine.cpu.pc, machine.cpu.map_gen as u32)
            }
            _ => return,
        };
        // End the mutable JitState borrow before calling generated code: the
        // target may itself call chain_next recursively.
        let idx = {
            let Some(jit) = SYS_JIT.as_mut() else { return };
            let line = jit.dispatch[JitState::dslot(pc)];
            if line.pc != pc
                || line.gen != map_gen
                || line.gen == UNVERIFIED_DISPATCH_GEN
                || line.idx < 0
            {
                return; // miss/blacklist/stale: the host loop owns the slow path
            }
            line.idx & !SB_IDX_BIT
        };
        CHAIN_DEPTH += 1;
        CHAIN_HOPS += 1;
        let f: extern "C" fn(i32) = core::mem::transmute(idx as usize);
        f(state);
        CHAIN_DEPTH -= 1;
    }
}

/// Region-function modules issued but not yet landed. The host's run loop
/// should yield to its event loop when this is nonzero: module compilation
/// resolves on the microtask queue, and a loop that never yields leaves
/// finished code waiting tens of millions of instructions (v86's runner is
/// event-driven per slice, so its codegen lands immediately — symmetric
/// scheduling requires giving our compiles the same chance).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_pending_builds() -> u32 {
    unsafe { PENDING_SB.len() as u32 }
}

/// Async superblock completion (called by JS between runSystem calls, never
/// during wasm execution). Validates that the machine, the code page, and
/// the va→pa mapping are still the ones the compile was issued against
/// before repointing the page's entries at the new function.
fn pending_region_stale(machine: &impl SystemJitMachine, pages: &[(u64, u64)]) -> bool {
    pages.iter().any(|&(_, physical_page)| {
        let page = (physical_page - rv64_system::RAM_BASE) >> 12;
        !machine.jit_page_marked(page) || machine.jit_page_dirty(page)
    })
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_sb_ready(ticket: u64, idx: i32) {
    unsafe {
        let Some(pos) = PENDING_SB.iter().position(|p| p.ticket == ticket) else {
            return;
        };
        let p = PENDING_SB.swap_remove(pos);
        if p.boot_gen != BOOT_GEN {
            return;
        }
        let Some(jit) = SYS_JIT.as_mut() else { return };
        let policy_keys: Vec<(u64, u64)> = p
            .pages
            .iter()
            .map(|&(va, pa)| (va, pa))
            .filter(|key| jit.policy_pending.remove(key))
            .collect();
        if idx < 0 {
            for key in policy_keys {
                jit.policy_rejected.insert(key);
                PAGE_POLICY_FAILED += 1;
            }
            return;
        }
        // Page written (dirtied/unmarked) while compiling → drop; un-superblock
        // so the region can retry with fresh bytes. `marked && !dirty` on every
        // page means no store has touched these code bytes since the compile
        // was issued.
        //
        // The va→pa mappings are deliberately NOT re-probed here: this callback
        // fires on the microtask queue at an arbitrary guest moment — usually
        // inside the kernel or another process, where a fetch probe of a user
        // va fails on privilege or resolves in the wrong address space. That
        // made 96% of finished superblocks drop on the floor (measured:
        // landed=4 of 127 on nbench). Instead the entries go into the
        // authoritative cache carrying their recorded pa, with NO dispatch
        // line: the first dispatch of each entry takes the cache path, which
        // probes the fetch mapping (and, for a multi-page region, every other
        // page too) before it runs or caches the line — the same verification
        // every block gets after a mapping event, deferred to a point where the
        // current address space is the one asking for the block.
        let stale = match ACTIVE_SYSTEM_KIND {
            kind if kind == SystemMachineKind::Legacy as u8 => SYS
                .as_ref()
                .is_none_or(|machine| pending_region_stale(machine, &p.pages)),
            kind if kind == SystemMachineKind::Virt as u8 => VIRT
                .as_ref()
                .is_none_or(|machine| pending_region_stale(machine, &p.pages)),
            _ => true,
        };
        if stale {
            SB_STALE += 1;
            PAGE_POLICY_FAILED += policy_keys.len() as u64;
            for &(va, _) in &p.pages {
                jit.superblocked.remove(&(p.aspace, va));
            }

            return;
        }
        for key in policy_keys {
            jit.policy_installed.entry(key).or_default().extend(
                p.entries
                    .iter()
                    .copied()
                    .filter(|entry| entry & !0xfff == key.0),
            );
            // Work interpreted while this build was in flight may already be
            // covered by the landed static closure; re-arm from zero and let a
            // genuinely uncovered entry accumulate fresh evidence.
            jit.policy_heat.insert(key, 0);
            jit.policy_compiled.insert(key);
            PAGE_POLICY_LANDED += 1;
        }
        SB_LANDED += 1;
        JIT_TABLE_ENTRIES += 1;
        if let Some(key) = p.template_key.clone() {
            let evicted_physical = (jit.page_templates.len() == PAGE_TEMPLATE_PROBE_CAP)
                .then(|| {
                    jit.page_templates
                        .pop_front()
                        .map(|template| template.physical_page)
                })
                .flatten();
            let physical_page = p.pages[0].1;
            jit.page_templates.push_back(PageTemplate {
                key,
                physical_page,
                template_cache_id: p.template_cache_id,
                wasm_bytes: p.template_wasm_bytes,
                table_index: idx,
            });
            if let Some(evicted) = evicted_physical.filter(|evicted| *evicted != physical_page) {
                rebuild_page_template_cached_offsets(jit, evicted);
            }
            rebuild_page_template_cached_offsets(jit, physical_page);
        }
        if p.pages.len() > 1 {
            jit.regions.insert(idx, p.pages.clone());
        }
        // Start the exit profile that drives measured extension/demotion.
        jit.region_exits.insert(
            idx,
            RegionExits {
                aspace: p.aspace,
                lead: p.lead,
                pages: p.pages.clone(),
                total: 0,
                targets: Vec::new(),
                samples: 0,
                stay_sum: 0,
                last_tick: EXIT_TICK,
                entries: p.entries.clone(),
            },
        );
        for &e in &p.entries {
            // Sparse regions: find the entry's page by lookup (pages are in
            // dispatch order, not address order).
            let Some(pi) = p.pages.iter().position(|&(va, _)| va == e & !0xfff) else {
                continue;
            };
            // A long trace block keeps its pc: it already amortizes its
            // dispatch, and the function entry would trade that for a
            // register-union load per visit (see TRACE_KEEP_MIN).
            if matches!(jit.cache.get(&e), Some(Some(b))
                if TRACE_KEEP_MIN != 0 && b.n != 0 && !b.fp && b.n >= TRACE_KEEP_MIN)
            {
                continue;
            }
            let epa = p.pages[pi].1 + (e & 0xfff);
            let jb = JitBlock {
                fp: false,
                idx,
                n: 0,
                mix: [0; 5],
                mem: [0; 10],
                control: [0; 3],
                alu: [0; 5],
                pa: epa,
            };
            let prev = jit.cache.insert(e, Some(jb));
            SB_ENTRIES_IN += 1;
            if e == TRACE_PC {
                TRACE_SB_INSTALL += 1;
            }
            if matches!(prev, Some(Some(b)) if b.n != 0) {
                SB_REPLACED += 1;
            }
            let fresh = prev.is_none();
            for (k, &(_, pp)) in p.pages.iter().enumerate() {
                if k == pi && !fresh {
                    continue; // already registered under its own page
                }
                jit.page_blocks
                    .entry((pp - rv64_system::RAM_BASE) >> 12)
                    .or_default()
                    .push(e);
            }
            // Publish an unverified direct-map candidate. The precise
            // interpreter re-entry predicate can now see the landed PC; the
            // next outer dispatch deliberately misses the generation check,
            // validates VA→PA through `cache`, and installs a verified line.
            // Without this candidate, sampled interpretation can run past the
            // cache-only entry forever unless a host slice happens to end on
            // that exact PC (measured CPython generated coverage: 0.3%).
            let slot = JitState::dslot(e);
            jit.dispatch[slot] = DispatchLine {
                pc: e,
                idx: idx | SB_IDX_BIT,
                gen: UNVERIFIED_DISPATCH_GEN,
            };
        }
    }
}

/// Diagnostic: re-run the superblock leader analysis for a page in the CURRENT
/// address space. which: 0 = leaders discovered, 1 = leaders dropped as loop
/// headers, 2 = hot pcs recorded, 3 = hot pcs that survive as leaders,
/// 4 = hot pcs dropped as loop headers. u64::MAX = page not resolvable now.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sb_analyze(vpage: u64, which: u32) -> u64 {
    unsafe {
        let Some(m) = SYS.as_mut() else {
            return u64::MAX;
        };
        let Some(jit) = SYS_JIT.as_ref() else {
            return u64::MAX;
        };
        let Some(pa) = m.cpu.jit_probe_fetch(&mut m.bus, vpage) else {
            return u64::MAX;
        };
        let off = ((pa & !0xfff) - rv64_system::RAM_BASE) as usize;
        if pa < rv64_system::RAM_BASE || off + 0x1000 > m.bus.ram.len() {
            return u64::MAX;
        }
        let code = &m.bus.ram[off..off + 0x1000];
        let lay = rv64_dbt::JitLayout {
            x_base: m.cpu.x.as_ptr() as u32,
            pc_addr: &m.cpu.pc as *const u64 as u32,
            mem: None,
            sys: Some(jit_system_memory(&m.cpu)),
            mem_profile: mem_profile_layout(),
            reg_stress: reg_stress(),
            reg_profile_base: reg_profile_base(),
            structured_profile: None,
            multi_latch: MULTI_LATCH,
            retired_addr: retired_addr(),
            f_base: m.cpu.f.as_ptr() as u32,
            fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
            reservation: Some(rv64_dbt::ReservationCapability::System),
            fuel_addr: fuel_addr(),
            mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
            copystat_addr: copystat_addr(),
            chain_off_addr: chain_off_addr(),
            batch_base_addr: 0,
            dispatch_base: 0,
            dispatch_mask: 0,
            map_gen_addr: 0,
            chain_hops_addr: 0,
            ic_miss_owner_addr: 0,
            ic_miss_target_addr: 0,
            pic_code_base: None,
        };
        let empty = Vec::new();
        let aspace = m.cpu.sys.as_ref().map_or(0, |c| c.satp);
        let seeds = jit.page_entries.get(&(aspace, vpage)).unwrap_or(&empty);
        let leaders = rv64_dbt::discover_page_leaders(code, vpage, vpage, 0x1000, seeds, 512);
        let is_loop = |e: u64| rv64_dbt::is_loop_at(code, vpage, e, lay);
        if which >= 5 {
            let keep: Vec<u64> = leaders.iter().copied().filter(|&e| !is_loop(e)).collect();
            let (rm, wm, fr, fw) =
                rv64_dbt::scan_regs_super_pub(code, vpage, vpage + 0x1000, &keep, &lay);
            return match which {
                5 => ((rm | wm) & !1).count_ones() as u64,
                _ => (fr | fw).count_ones() as u64,
            };
        }
        match which {
            0 => leaders.len() as u64,
            1 => leaders.iter().filter(|&&e| is_loop(e)).count() as u64,
            2 => seeds.len() as u64,
            3 => seeds
                .iter()
                .filter(|&&e| leaders.contains(&e) && !is_loop(e))
                .count() as u64,
            _ => seeds.iter().filter(|&&e| is_loop(e)).count() as u64,
        }
    }
}

/// Diagnostic for ONE pc: 0 = is_loop_at (excluded from superblocks),
/// 1 = instructions an individual block covers, 2 = cached (1) / blacklisted
/// (2) / absent (0).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sb_analyze_pc(pc: u64, which: u32) -> u64 {
    unsafe {
        let Some(m) = SYS.as_mut() else {
            return u64::MAX;
        };
        let Some(jit) = SYS_JIT.as_ref() else {
            return u64::MAX;
        };
        if which == 2 {
            return match jit.cache.get(&pc) {
                Some(Some(b)) => {
                    if b.n == 0 {
                        3 // superblock entry
                    } else {
                        1
                    }
                }
                Some(None) => 2,
                None => 0,
            };
        }
        let Some(pa) = m.cpu.jit_probe_fetch(&mut m.bus, pc) else {
            return u64::MAX;
        };
        if pa < rv64_system::RAM_BASE {
            return u64::MAX;
        }
        let off = (pa - rv64_system::RAM_BASE) as usize;
        let end = ((off + 1024).min(off | 0xfff) + 1).min(m.bus.ram.len());
        let lay = rv64_dbt::JitLayout {
            x_base: m.cpu.x.as_ptr() as u32,
            pc_addr: &m.cpu.pc as *const u64 as u32,
            mem: None,
            sys: Some(jit_system_memory(&m.cpu)),
            mem_profile: mem_profile_layout(),
            reg_stress: reg_stress(),
            reg_profile_base: reg_profile_base(),
            structured_profile: None,
            multi_latch: MULTI_LATCH,
            retired_addr: retired_addr(),
            f_base: m.cpu.f.as_ptr() as u32,
            fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
            reservation: Some(rv64_dbt::ReservationCapability::System),
            fuel_addr: fuel_addr(),
            mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
            copystat_addr: copystat_addr(),
            chain_off_addr: chain_off_addr(),
            batch_base_addr: 0,
            dispatch_base: 0,
            dispatch_mask: 0,
            map_gen_addr: 0,
            chain_hops_addr: 0,
            ic_miss_owner_addr: 0,
            ic_miss_target_addr: 0,
            pic_code_base: None,
        };
        let code = &m.bus.ram[off..end];
        match which {
            0 => rv64_dbt::is_loop_at(code, pc, pc, lay) as u64,
            3 => u32::from_le_bytes([code[0], code[1], code[2], code[3]]) as u64,
            _ => rv64_dbt::translate_block(code, pc, pc, lay).map_or(0, |b| b.n_insns as u64),
        }
    }
}

/// Diagnostic: superblock state of a code page — bit0 superblocked,
/// bit1 pending-async, bits 8.. = discovered hot entry count.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sb_debug(vpage: u64) -> u64 {
    unsafe {
        let Some(jit) = SYS_JIT.as_ref() else {
            return 0;
        };
        let Some(mm) = SYS.as_ref() else { return 0 };
        let mut v = 0u64;
        let aspace = mm.cpu.sys.as_ref().map_or(0, |c| c.satp);
        if jit.superblocked.contains(&(aspace, vpage)) {
            v |= 1;
        }
        if PENDING_SB
            .iter()
            .any(|p| p.pages.iter().any(|&(va, _)| va == vpage))
        {
            v |= 2;
        }
        v |= (jit
            .page_entries
            .get(&(aspace, vpage))
            .map_or(0, |e| e.len()) as u64)
            << 8;
        // bits 24..31 = superblock compiles, 32..39 = uncovered hot pcs since
        v |= (jit.sb_gen.get(&(aspace, vpage)).map_or(0, |&(_, c, _)| c) as u64 & 0xff) << 24;
        v |= (jit.sb_missed.get(&(aspace, vpage)).copied().unwrap_or(0) as u64 & 0xff) << 32;
        v
    }
}

/// Trace one guest pc through the compile pipeline: jit_stat 27 = times it was
/// installed as a superblock entry, 28 = individual blocks built for it,
/// 29 = times it was a superblock seed, 30 = times it survived leader retain.
#[no_mangle]
pub extern "C" fn sb_trace_pc(pc: u64) {
    unsafe { TRACE_PC = pc };
}

/// Enable the hardware fused-madd FMADD path (host proves fusedness first).
#[no_mangle]
pub extern "C" fn jit_set_hw_fma(on: u32) {
    rv64_dbt::set_hw_fma(on != 0);
}

/// Enable direct block-to-block tail-call chaining (host feature-detects
/// wasm tail-call support first). Loop/region chain sites follow this
/// directly (their successor sets are cyclic and stay monomorphic); trace
/// exits additionally require a small block population — flipped per
/// compile from the live cache size (see CHAIN_POP_CAP).
#[no_mangle]
pub extern "C" fn jit_set_tailcall(on: u32) {
    rv64_dbt::set_chain(on != 0);
}

/// Enable structured-region continuation through the main module's typed
/// `chain_next` function import. This does not require the Wasm tail-call
/// proposal or make generated modules import the shared function table.
#[no_mangle]
pub extern "C" fn jit_set_region_chain(on: u32) {
    unsafe {
        REGION_CHAIN_ENABLED = on != 0;
        CHAIN_OFF_CELL = 0;
    }
    rv64_dbt::set_chain(on != 0);
}

/// Enable frame-free continuation between independently compiled structured
/// regions. The browser must first prove `return_call_indirect` support; only
/// modules compiled while this flag is enabled import the shared function
/// table and emit tail-transfer sites.
#[no_mangle]
pub extern "C" fn jit_set_region_tail_chain(on: u32) {
    rv64_dbt::set_region_tail_chain(on != 0);
}
/// A/B: trace definedness tracking (rv64_dbt::set_defined_track).
#[no_mangle]
pub extern "C" fn jit_set_defined(on: u32) {
    rv64_dbt::set_defined_track(on != 0);
}
/// A/B: rotated-nest loop regions (see rv64_dbt::set_rotated_nests).
#[no_mangle]
pub extern "C" fn jit_set_rotated_nests(on: u32) {
    rv64_dbt::set_rotated_nests(on != 0);
}
/// Base table index of the most recently REGISTERED batch (see
/// JitLayout::batch_base_addr): the emitted intra-batch link checks
/// `line.idx == base + j` against this cell, which the host writes right
/// after the batch's functions land in the table.
/// Per-batch base cells. A single global cell was wrong: every batch's
/// emitted links read the same address, so as soon as a second batch
/// registered, the first batch's freshness checks compared against the
/// wrong base — silently defeating the check they exist to perform. Each
/// batch gets its own slot (address baked into its emitted code), rotating
/// through a fixed pool so addresses are stable for the module's lifetime.
const BATCH_CELLS: usize = 4096;
static mut BATCH_BASE_POOL: [u32; BATCH_CELLS] = [0; BATCH_CELLS];
static mut BATCH_CELL_NEXT: usize = 0;
/// Batch compilation (see rv64_dbt::translate_batch). Members transfer by
/// DIRECT tail call inside one module — the only chaining shape that avoids
/// both historical blockers (no table import, so registration stays O(1);
/// no host round-trip per hop). It WORKS: 766 batches / 2965 members on an
/// in-guest `tcc -c`, host dispatches down 12%.
///
/// DEFAULT OFF anyway, on an interleaved 3-round A/B (the only method that
/// survives this host's boot lottery): compile 4356/4075/4069 without vs
/// 4558/4169/4336 with — batching loses every round. Reason, measured: only
/// ~12% of exits stay inside a batch, so the other ~88% pay the link's
/// guard (dispatch-line pc + map-generation + fuel) for nothing, and that
/// costs slightly more than the saved host round-trips return. Raising the
/// cap makes it worse (bigger modules, no better hit rate: 32 -> 4195,
/// 64 -> 4546). What would flip it: hit rate, not hop cost — batches formed
/// from OBSERVED dispatch sequences (trace trees) rather than static exit
/// seeds. Machinery is fully tested and one flag away.
/// Batching is ON, but only while the code cache is SMALL. Measured:
/// batching lifts nbench ASSIGNMENT from 8.37 to 9.14-10.13 iter/s
/// (uncontended), and destroys CPython — python fib went from 3.7s to 180s,
/// because a workload with a five-figure block population pays a batch
/// compile per tier-up for code it never re-enters. The separating property
/// is the population itself, not the workload: below the cap a batch's
/// members are a large fraction of all hot code, above it they are noise.
/// DEFAULT OFF on the evidence: batching's only beneficiary is nbench
/// ASSIGNMENT (+9..21%), which remains a LOSS either way (best draw 0.944x),
/// while it costs NUMERIC SORT ~25% (285.6 vs 358.5 iter/s under identical
/// contention) — enough to drop that row from MATCH to LOSS on the
/// authoritative scorecard. A mechanism that cannot flip the row it helps
/// and does flip a row it hurts is not worth shipping on. The machinery,
/// the IC composition and the rate governor stay behind jit_set_batch(1).
static mut BATCH_ON: bool = false;
/// Blocks in the cache beyond which batching stops (see BATCH_ON).
/// NOTE: population alone is NOT a sufficient gate — python fib still ran
/// 180s with a 4096-block cap, because the storm happens during warm-up
/// while the cache is still small. Batch builds are therefore ALSO charged
/// to the measured build-time budget (SB_BUILD_MS), which is what actually
/// bounds a workload that wants a batch per tier-up.
static mut BATCH_POP_CAP: usize = 4096;
/// Distinct hot code PAGES beyond which batching stops. This is the
/// footprint signal that actually separates the two behaviours: nbench
/// ASSIGNMENT's hot code is a handful of pages and its batches cover most
/// of what runs, while CPython spreads over dozens and its batches are
/// noise it pays for per tier-up. A build-time budget was tried and is too
/// blunt — tight enough to save python (180s -> 3.7s) also erased
/// ASSIGNMENT's gain (10.1 -> 8.3).
static mut BATCH_PAGE_CAP: usize = 64;
/// Batches per billion retired instructions above which batching is judged
/// unprofitable and switched off for the run (see the rate governor).
static mut BATCH_RATE_CAP: u64 = 200;
#[no_mangle]
pub extern "C" fn jit_set_batch_rate_cap(v: u32) {
    unsafe { BATCH_RATE_CAP = v as u64 }
}
#[no_mangle]
pub extern "C" fn jit_set_batch_page_cap(v: u32) {
    unsafe { BATCH_PAGE_CAP = v as usize }
}
#[no_mangle]
pub extern "C" fn jit_set_batch_pop_cap(v: u32) {
    unsafe { BATCH_POP_CAP = v as usize }
}
/// Consecutive identical successors before a trace is recompiled with an
/// inline cache through its terminating indirect jump. 256 measured best:
/// the extension costs one recompile per pc, so a higher bar spends that
/// only on genuinely stable edges (python fib 3520 -> 3356ms, compile
/// 4505 -> 4468ms against trigger 64).
static mut IC_EXTEND_TRIGGER: u32 = 256;
static mut IC_EXTENDS: u64 = 0;
static mut IC_PIC_EXTENDS: u64 = 0;
#[no_mangle]
pub extern "C" fn jit_set_ic_trigger(v: u32) {
    unsafe { IC_EXTEND_TRIGGER = if v == 0 { u32::MAX } else { v } }
}
static mut BATCH_CAP: usize = 32;
static mut BATCH_PAGE: bool = false;
#[no_mangle]
pub extern "C" fn jit_set_batch_page(on: u32) {
    unsafe { BATCH_PAGE = on != 0 }
}
static mut BATCH_BAR_SHIFT: u32 = 1;
#[no_mangle]
pub extern "C" fn jit_set_batch_cap(v: u32) {
    unsafe { BATCH_CAP = v as usize }
}
#[no_mangle]
pub extern "C" fn jit_set_batch_bar_shift(v: u32) {
    unsafe { BATCH_BAR_SHIFT = v }
}
static mut BATCHES: u64 = 0;
static mut BATCH_MEMBERS: u64 = 0;
#[no_mangle]
pub extern "C" fn jit_set_batch(on: u32) {
    unsafe { BATCH_ON = on != 0 }
}
#[allow(static_mut_refs)]
fn batch_cell_addr(i: usize) -> u32 {
    unsafe { &BATCH_BASE_POOL[i % BATCH_CELLS] as *const u32 as u32 }
}

/// Live chain kill switch (see JitLayout::chain_off_addr): nonzero disables
/// every emitted chain transfer. Driven by CODE-CHURN RATE at quantum
/// boundaries: a workload still compiling new blocks (tcc churns ~7.5k
/// blocks across its entire run; CPython warms for seconds) has an
/// unstable, megamorphic chain graph that V8's ICs cannot serve — measured
/// 2-2.9x slower chained. A warmed workload (nbench kernels self-time for
/// minutes after a burst of compiles) has a stable graph where chained
/// hops cost ~2ns and measured up to +23%. Population caps cannot separate
/// the two (cumulative counts overlap); churn does.
static mut CHAIN_OFF_CELL: u32 = 0;
static mut COMPILES_TICK: u64 = 0;
// Async page regions have a bounded, mapping-validated tail-call topology.
// Keep chaining deterministic for that policy: adjacent host-time probe
// windows previously made identical Huffman runs differ by 19x. The legacy
// adaptive policy retains its historical online controller.
static mut PAGE_POLICY_STABLE_CHAIN: bool = true;

#[no_mangle]
pub extern "C" fn jit_set_page_stable_chain(on: u32) {
    unsafe {
        PAGE_POLICY_STABLE_CHAIN = on != 0;
        if PAGE_POLICY_ENABLED && PAGE_POLICY_STABLE_CHAIN {
            CHAIN_OFF_CELL = 0;
        }
    }
}

fn chain_off_addr() -> u32 {
    (&raw const CHAIN_OFF_CELL) as u32
}

/// Online chain controller: no static rule separates workloads whose chain
/// graph V8 serves at ~2ns/hop (warm nbench kernels: +23% and an
/// ASSIGNMENT row that flips to a WIN) from those it cannot (tcc's 7.5k-
/// block soup: 2-2.9x slower; population, churn and per-site target-kind
/// gates all failed to split them). So MEASURE: alternate ON/OFF probe
/// windows of PROBE_QUANTA boundaries, compare wall-ns per retired
/// instruction, lock the faster setting for LOCK_QUANTA, then re-probe
/// (workloads change phases). Compiling a new block unlocks immediately.
struct ChainCtl {
    state: u8, // 0 = probing ON, 1 = probing OFF, 2 = locked
    quanta: u32,
    t0_ms: f64,
    retired0: u64,
    ns_per_insn: [f64; 2],
    locked_off: u32,
}
static mut CHAIN_CTL: ChainCtl = ChainCtl {
    state: 0,
    quanta: 0,
    t0_ms: 0.0,
    retired0: 0,
    ns_per_insn: [0.0; 2],
    locked_off: 0,
};
const PROBE_QUANTA: u32 = 8;
const LOCK_QUANTA: u32 = 256;

fn reset_chain_runtime() {
    unsafe {
        CHAIN_DEPTH = 0;
        CHAIN_HOPS = 0;
        CHAIN_OFF_CELL = 0;
        COMPILES_TICK = 0;
        CHAIN_CTL = ChainCtl {
            state: 0,
            quanta: 0,
            t0_ms: 0.0,
            retired0: 0,
            ns_per_insn: [0.0; 2],
            locked_off: 0,
        };
    }
}

#[allow(static_mut_refs)]
fn chain_ctl_boundary(retired_total: u64) {
    unsafe {
        if REGION_CHAIN_ENABLED || (PAGE_POLICY_ENABLED && PAGE_POLICY_STABLE_CHAIN) {
            CHAIN_OFF_CELL = 0;
            return;
        }
        let ctl = &mut CHAIN_CTL;
        ctl.quanta += 1;
        let now = host_now_ms();
        match ctl.state {
            0 | 1 => {
                if ctl.quanta >= PROBE_QUANTA {
                    let insns = retired_total.wrapping_sub(ctl.retired0).max(1);
                    ctl.ns_per_insn[ctl.state as usize] = (now - ctl.t0_ms) * 1e6 / insns as f64;
                    if ctl.state == 0 {
                        ctl.state = 1;
                        CHAIN_OFF_CELL = 1;
                    } else {
                        // Verdict: lock the faster setting.
                        ctl.locked_off = (ctl.ns_per_insn[1] < ctl.ns_per_insn[0]) as u32;
                        CHAIN_OFF_CELL = ctl.locked_off;
                        ctl.state = 2;
                    }
                    ctl.quanta = 0;
                    ctl.t0_ms = now;
                    ctl.retired0 = retired_total;
                }
            }
            _ => {
                if ctl.quanta >= LOCK_QUANTA {
                    ctl.state = 0;
                    CHAIN_OFF_CELL = 0;
                    ctl.quanta = 0;
                    ctl.t0_ms = now;
                    ctl.retired0 = retired_total;
                }
            }
        }
    }
}

/// Trace aggressiveness for individual blocks (see rv64_dbt::set_trace_level):
/// 0 = classic basic blocks, 1 = branch side-exits, 2 = +call following,
/// 3 = +return following (default).
#[no_mangle]
pub extern "C" fn jit_set_trace_level(l: u32) {
    rv64_dbt::set_trace_level(l);
}

/// Toggle host-filled TLB misses inside compiled blocks (perf A/B).
#[no_mangle]
pub extern "C" fn jit_set_tlb_fill(on: u32) {
    rv64_dbt::set_tlb_fill(on != 0);
}

const BULK_COPY_PAGE_BYTES: u64 = 4096;
const BULK_COPY_PAGE_MASK: u64 = BULK_COPY_PAGE_BYTES - 1;

#[derive(Clone, Copy)]
struct BulkCopyPage {
    guest_page: u64,
    linear_offset: i64,
}

#[derive(Clone, Copy)]
struct BulkCopyRange {
    first: BulkCopyPage,
    second: Option<BulkCopyPage>,
}

impl BulkCopyRange {
    #[inline]
    fn linear(self, address: u64) -> usize {
        let page = address & !BULK_COPY_PAGE_MASK;
        let mapping = if self.first.guest_page == page {
            self.first
        } else {
            self.second
                .expect("validated bulk range has its second page")
        };
        address.wrapping_add(mapping.linear_offset as u64) as usize
    }
}

#[inline]
fn bulk_copy_page<B: Bus>(
    cpu: &mut Cpu,
    bus: &mut B,
    guest_page: u64,
    store: bool,
) -> Option<BulkCopyPage> {
    let linear_offset = cpu
        .jit_probe_tlb(guest_page, store)
        .or_else(|| cpu.jit_fill_tlb(bus, guest_page, store))?;
    Some(BulkCopyPage {
        guest_page,
        linear_offset,
    })
}

/// Validate the one or two guest pages occupied by a short range without
/// modifying guest data. The whole range is proved before the boundary replay
/// starts, so a rejected page leaves the original scalar iteration untouched.
fn bulk_copy_range<B: Bus>(
    cpu: &mut Cpu,
    bus: &mut B,
    start: u64,
    bytes: u64,
    store: bool,
) -> Option<BulkCopyRange> {
    let last = start.checked_add(bytes.checked_sub(1)?)?;
    let first_page = start & !BULK_COPY_PAGE_MASK;
    let last_page = last & !BULK_COPY_PAGE_MASK;
    let first = bulk_copy_page(cpu, bus, first_page, store)?;
    let second = if last_page != first_page {
        Some(bulk_copy_page(cpu, bus, last_page, store)?)
    } else {
        None
    };
    Some(BulkCopyRange { first, second })
}

/// Replay one proved copy-loop iteration whose source or destination range
/// straddles a guest page. Loads and stores remain interleaved in the guest's
/// original direction, preserving overlap behavior; each virtual page may map
/// to an unrelated linear-memory page.
fn bulk_copy_boundary_iteration<B: Bus>(
    cpu: &mut Cpu,
    bus: &mut B,
    source: u64,
    destination: u64,
    bytes: u64,
    backward: bool,
) -> Result<u64, u8> {
    let source_map = bulk_copy_range(cpu, bus, source, bytes, false).ok_or(2)?;
    let destination_map = bulk_copy_range(cpu, bus, destination, bytes, true).ok_or(3)?;
    let words = usize::try_from(bytes / 8).map_err(|_| 1)?;
    let mut last_value = 0u64;
    for ordinal in 0..words {
        let slot = if backward {
            words - 1 - ordinal
        } else {
            ordinal
        };
        let offset = (slot * 8) as u64;
        let source_address = source.wrapping_add(offset);
        let destination_address = destination.wrapping_add(offset);
        let source_crosses = source_address & BULK_COPY_PAGE_MASK > BULK_COPY_PAGE_BYTES - 8;
        let destination_crosses =
            destination_address & BULK_COPY_PAGE_MASK > BULK_COPY_PAGE_BYTES - 8;

        // SAFETY: both short ranges were permission-checked above. A word
        // wholly inside one guest page is contiguous in linear memory. The
        // byte path selects the independently validated mapping on each side
        // of a non-contiguous guest-page boundary.
        let value = unsafe {
            if source_crosses {
                let mut bytes = [0u8; 8];
                for (index, byte) in bytes.iter_mut().enumerate() {
                    *byte = core::ptr::read(
                        source_map.linear(source_address + index as u64) as *const u8
                    );
                }
                u64::from_le_bytes(bytes)
            } else {
                core::ptr::read_unaligned(source_map.linear(source_address) as *const u64)
            }
        };
        unsafe {
            if destination_crosses {
                for (index, byte) in value.to_le_bytes().into_iter().enumerate() {
                    core::ptr::write(
                        destination_map.linear(destination_address + index as u64) as *mut u8,
                        byte,
                    );
                }
            } else {
                core::ptr::write_unaligned(
                    destination_map.linear(destination_address) as *mut u64,
                    value,
                );
            }
        }
        last_value = value;
    }
    Ok(last_value)
}

/// Execute page-contained portions of a proved compiler-generated memmove
/// loop.  Each chunk validates both guest translations before modifying RAM;
/// a zero/partial return therefore lets generated code resume the exact scalar
/// loop at the first unprocessed 64-byte iteration.
fn system_bulk_copy<B: Bus>(
    cpu: &mut Cpu,
    bus: &mut B,
    source: u64,
    destination: u64,
    requested: u64,
    iteration_bytes: u64,
    backward: bool,
    value_reg: usize,
) -> (u64, u64, u8) {
    if !matches!(iteration_bytes, 8 | 64)
        || requested < iteration_bytes
        || requested & (iteration_bytes - 1) != 0
        || value_reg == 0
        || value_reg >= cpu.x.len()
    {
        return (0, 0, 1);
    }

    let mut copied = 0u64;
    let mut chunks = 0u64;
    let mut last_value = 0u64;
    let mut stop_reason = 0u8;
    while copied < requested {
        let remaining = requested - copied;
        let (source_start, destination_start, available) = if backward {
            let source_end = source.wrapping_sub(copied);
            let destination_end = destination.wrapping_sub(copied);
            let source_page_bytes = match source_end & (BULK_COPY_PAGE_BYTES - 1) {
                0 => BULK_COPY_PAGE_BYTES,
                bytes => bytes,
            };
            let destination_page_bytes = match destination_end & (BULK_COPY_PAGE_BYTES - 1) {
                0 => BULK_COPY_PAGE_BYTES,
                bytes => bytes,
            };
            let available = remaining.min(source_page_bytes).min(destination_page_bytes)
                & !(iteration_bytes - 1);
            (
                source_end.wrapping_sub(available),
                destination_end.wrapping_sub(available),
                available,
            )
        } else {
            let source_start = source.wrapping_add(copied);
            let destination_start = destination.wrapping_add(copied);
            let source_page_bytes =
                BULK_COPY_PAGE_BYTES - (source_start & (BULK_COPY_PAGE_BYTES - 1));
            let destination_page_bytes =
                BULK_COPY_PAGE_BYTES - (destination_start & (BULK_COPY_PAGE_BYTES - 1));
            let available = remaining.min(source_page_bytes).min(destination_page_bytes)
                & !(iteration_bytes - 1);
            (source_start, destination_start, available)
        };
        if available == 0 {
            let (boundary_source, boundary_destination) = if backward {
                (
                    source.wrapping_sub(copied).wrapping_sub(iteration_bytes),
                    destination
                        .wrapping_sub(copied)
                        .wrapping_sub(iteration_bytes),
                )
            } else {
                (source_start, destination_start)
            };
            match bulk_copy_boundary_iteration(
                cpu,
                bus,
                boundary_source,
                boundary_destination,
                iteration_bytes,
                backward,
            ) {
                Ok(value) => {
                    last_value = value;
                    copied += iteration_bytes;
                    chunks += 1;
                    continue;
                }
                Err(reason) => {
                    stop_reason = reason;
                    break;
                }
            }
        }

        // The ranges are page-contained, so one permission-checked refill per
        // side proves every byte the ensuing memory.copy may touch.  Store
        // refill also rejects pages containing generated guest code.
        let source_offset = cpu
            .jit_probe_tlb(source_start, false)
            .or_else(|| cpu.jit_fill_tlb(bus, source_start, false));
        let Some(source_offset) = source_offset else {
            stop_reason = 2;
            break;
        };
        let destination_offset = cpu
            .jit_probe_tlb(destination_start, true)
            .or_else(|| cpu.jit_fill_tlb(bus, destination_start, true));
        let Some(destination_offset) = destination_offset else {
            stop_reason = 3;
            break;
        };
        let source_linear = source_start.wrapping_add(source_offset as u64) as usize;
        let destination_linear = destination_start.wrapping_add(destination_offset as u64) as usize;
        let Ok(length) = usize::try_from(available) else {
            stop_reason = 1;
            break;
        };

        // SAFETY: jit_fill_tlb returned direct linear-memory offsets and the
        // page-boundary calculation keeps both ranges within those validated
        // pages. ptr::copy deliberately supplies memmove overlap semantics.
        unsafe {
            core::ptr::copy(
                source_linear as *const u8,
                destination_linear as *mut u8,
                length,
            );
            let final_word = if backward {
                destination_linear
            } else {
                destination_linear + length - 8
            };
            last_value = core::ptr::read_unaligned(final_word as *const u64);
        }
        copied += available;
        chunks += 1;
    }

    if copied != 0 {
        cpu.x[value_reg] = last_value;
    }
    (copied, chunks, stop_reason)
}

/// Whole-loop copy helper imported by dynamic JIT modules. This is a
/// Wasm-to-Wasm call: JavaScript participates only while instantiating the
/// module, never on the hot copy path.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_system_bulk_copy(
    state_ptr: u32,
    source: u64,
    destination: u64,
    requested: u64,
    iteration_bytes: u32,
    backward: u32,
    value_reg: u32,
) -> u64 {
    unsafe {
        SYS_BULK_COPY_DIAG[0] = SYS_BULK_COPY_DIAG[0].wrapping_add(1);
        if state_ptr == 0 || state_ptr != ACTIVE_SYSTEM_STATE {
            SYS_BULK_COPY_DIAG[1] = SYS_BULK_COPY_DIAG[1].wrapping_add(1);
            return 0;
        }
        let result = match ACTIVE_SYSTEM_KIND {
            kind if kind == SystemMachineKind::Legacy as u8 => {
                let machine = &mut *(state_ptr as usize as *mut rv64_system::Machine);
                system_bulk_copy(
                    &mut machine.cpu,
                    &mut machine.bus,
                    source,
                    destination,
                    requested,
                    u64::from(iteration_bytes),
                    backward != 0,
                    value_reg as usize,
                )
            }
            kind if kind == SystemMachineKind::Virt as u8 => {
                let machine = &mut *(state_ptr as usize as *mut rv64_system::virt::VirtMachine);
                system_bulk_copy(
                    &mut machine.cpu,
                    &mut machine.bus,
                    source,
                    destination,
                    requested,
                    u64::from(iteration_bytes),
                    backward != 0,
                    value_reg as usize,
                )
            }
            _ => (0, 0, 0),
        };
        COPY_CHUNKS = COPY_CHUNKS.wrapping_add(result.1);
        SYS_BULK_COPY_DIAG[5] = SYS_BULK_COPY_DIAG[5].wrapping_add(result.0);
        if result.2 != 0 {
            let index = 1 + result.2 as usize;
            SYS_BULK_COPY_DIAG[index] = SYS_BULK_COPY_DIAG[index].wrapping_add(1);
        }
        result.0
    }
}

/// Fused-TLB refill for compiled blocks: called from generated code (a
/// wasm->wasm call through the module's `env.tlb_fill` import) when an inline
/// probe misses. Returns the offset such that `linear = va + off`, or -1 when
/// the access can't be served inline (unmapped, permission fault, MMIO, or a
/// page holding compiled code) — the block then bails and the interpreter
/// re-executes the instruction, raising the exact architectural fault.
///
/// Reentrancy: this runs inside a `call_block` that sys_run made while it
/// holds the machine. That is the same contract compiled code already has —
/// blocks write the register file and TLB rows through raw addresses, and the
/// block call is opaque to the compiler (the machine pointer is passed in
/// precisely so nothing is cached across it).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_tlb_fill(va: u64, store: u32) -> i64 {
    unsafe {
        TLB_FILLS += 1;
        let state = ACTIVE_SYSTEM_STATE as usize;
        match ACTIVE_SYSTEM_KIND {
            kind if kind == SystemMachineKind::Legacy as u8 && state != 0 => {
                let machine = &mut *(state as *mut rv64_system::Machine);
                let kind = machine.cpu.jit_tlb_miss_kind(va, store != 0);
                TLB_FILL_KIND[store.min(1) as usize * 4 + kind as usize] += 1;
                machine
                    .cpu
                    .jit_fill_tlb(&mut machine.bus, va, store != 0)
                    .unwrap_or(-1)
            }
            kind if kind == SystemMachineKind::Virt as u8 && state != 0 => {
                let machine = &mut *(state as *mut rv64_system::virt::VirtMachine);
                let kind = machine.cpu.jit_tlb_miss_kind(va, store != 0);
                TLB_FILL_KIND[store.min(1) as usize * 4 + kind as usize] += 1;
                machine
                    .cpu
                    .jit_fill_tlb(&mut machine.bus, va, store != 0)
                    .unwrap_or(-1)
            }
            _ => -1,
        }
    }
}

/// Current guest pc (diagnostic: host-side pc sampling for guest profiling).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_pc() -> u64 {
    unsafe { SYS.as_ref().map_or(0, |m| m.cpu.pc) }
}

/// Read-only full-system architectural state for deterministic differential
/// harnesses. These diagnostics do not participate in generated-code ABIs.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_reg(index: u32) -> u64 {
    unsafe {
        SYS.as_ref()
            .and_then(|m| m.cpu.x.get(index as usize).copied())
            .unwrap_or(0)
    }
}

/// Read a little-endian u64 from guest physical RAM for differential tests.
/// Invalid, low-RAM, and MMIO addresses return zero.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_ram_u64(physical_address: u64) -> u64 {
    unsafe {
        let Some(machine) = SYS.as_ref() else {
            return 0;
        };
        let Some(offset) = physical_address
            .checked_sub(rv64_system::RAM_BASE)
            .and_then(|offset| usize::try_from(offset).ok())
        else {
            return 0;
        };
        let Some(bytes) = machine.bus.ram.get(offset..offset.saturating_add(8)) else {
            return 0;
        };
        let Ok(bytes) = <[u8; 8]>::try_from(bytes) else {
            return 0;
        };
        u64::from_le_bytes(bytes)
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_insn_count() -> u64 {
    unsafe { SYS.as_ref().map(|m| m.cpu.insn_count).unwrap_or(0) }
}

// ---- JIT API (phase 6, v1) -------------------------------------------------

static mut JIT_OUT: Vec<u8> = Vec::new();

/// Translate a basic block: guest code bytes staged via staging_alloc,
/// `base` = guest address of the staged bytes, `pc` = block entry.
/// Returns number of guest instructions translated (0 = not translatable).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_translate(base: u64, pc: u64) -> u32 {
    unsafe {
        match rv64_dbt::translate_block(&STAGING, base, pc, rv64_dbt::JitLayout::bare()) {
            Some(b) => {
                JIT_OUT = b.wasm;
                b.n_insns
            }
            None => {
                JIT_OUT.clear();
                0
            }
        }
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_out_ptr() -> *const u8 {
    unsafe { JIT_OUT.as_ptr() }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn jit_out_len() -> u32 {
    unsafe {
        if MEMPROF_MODE & 4 != 0 {
            let len = JIT_OUT.len() as u64;
            MEMPROF[9] += len;
            MEMPROF[10] += 1;
            let bucket = if len <= 1024 {
                0
            } else if len <= 4096 {
                1
            } else if len <= 16384 {
                2
            } else {
                3
            };
            MEMPROF[11 + bucket] += 1;
            MEMPROF[15 + bucket] += len;
        }
        JIT_OUT.len() as u32
    }
}
