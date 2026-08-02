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

use rv64_core::{Cpu, FlatMemory, StopReason};
use rv64_linux::{Host, Machine};

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

const NO_PC: u64 = u64::MAX;
// 262144 lines: CPython's ~20k hot pcs collided at ~7% in 131072 under
// guest ASLR — per-boot slot-eviction churn is one suspected source of the
// python row's 3.3-6.7s bimodality. Doubling costs 2MB and no hot-path work.
const DISPATCH_BITS: u32 = 18;
const DISPATCH_SIZE: usize = 1 << DISPATCH_BITS;

struct JitState {
    /// pc -> compiled block; None = tried and not translatable (blacklist).
    /// Authoritative store (iterated for per-page invalidation).
    cache: std::collections::HashMap<u64, Option<JitBlock>>,
    hot: std::collections::HashMap<u64, u32>,
    /// Fast dispatch cache: direct-mapped, populated lazily from `cache`.
    dispatch: Vec<DispatchLine>,
    /// Last observed cpu.jit_flush_gen; a change means the va→pa code
    /// mapping was invalidated (satp/SFENCE) — drop everything.
    flush_gen: u64,
    /// Superblock compilation (v86's function-per-page): the hot block-entry
    /// pcs discovered in each guest code page (keyed by virtual page base). When
    /// a new entry appears the page's superblock is recompiled to cover it, and
    /// every entry's `cache`/`dispatch` slot points at the one superblock.
    page_entries: std::collections::HashMap<(u64, u64), Vec<u64>>,
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
    page_blocks: std::collections::HashMap<u64, Vec<u64>>,
    /// Virtual pages already compiled as a superblock — compile ONCE per page
    /// (with whatever entries were hot then); later hot pcs in the page get
    /// individual blocks. Recompiling the page's big br_table function on every
    /// new entry was a 2x regression on short workloads (the recompile storm).
    superblocked: std::collections::HashSet<(u64, u64)>,
    /// Virtual page -> (hot entries at its last superblock compile, number of
    /// UNPRODUCTIVE compiles — rebuilds that covered no new hot pcs). A page's first superblock is built from whatever handful of
    /// pcs was hot at the threshold; code discovered later — a second function
    /// in the page, a callee reached only by an indirect call — would stay on
    /// individual blocks forever (measured: nbench IDEA ran cipher_idea as 1-15
    /// instruction blocks, 6.4 insns per dispatch, on a page that WAS
    /// superblocked). Recompile once the page has accumulated another
    /// threshold's worth of uncovered hot pcs, bounded so a pathological page
    /// can't loop on it.
    sb_gen: std::collections::HashMap<(u64, u64), (usize, u32, u64)>,
    /// Pages that wanted a superblock while the compile budget was spent.
    /// Drained one per quantum boundary, oldest first.
    sb_queue: Vec<(u64, u64)>,
    /// Hot pcs on a superblocked page that had to get their own block because
    /// the page function does not cover them — the direct measure of a page
    /// function that has fallen behind the code actually running.
    sb_missed: std::collections::HashMap<(u64, u64), u32>,
    /// Observed successor of each dispatched pc, direct-mapped by dispatch
    /// slot (pc, next_pc). This is the NEXT-EXECUTING-TAIL signal that
    /// trace-tree JITs form regions from: batches built from STATIC exit
    /// seeds only kept ~12% of exits in-batch, because a trace's textual
    /// successors are not the ones execution actually takes.
    succ: Vec<(u64, u64, u32)>,
    /// Entry pcs already recompiled once with an inline cache — trace
    /// EXTENSION happens at most once per pc, so a flapping successor
    /// cannot loop the compiler.
    ic_done: std::collections::HashSet<u64>,
    /// Table index -> the (virtual page, physical page) list a MULTI-page
    /// superblock was compiled over. Entries carry their own page's pa (probed
    /// like any block at dispatch); this is the rest of the region, verified on
    /// the same slow path so a region can never execute against a page that was
    /// remapped out from under it.
    regions: std::collections::HashMap<i32, Vec<(u64, u64)>>,
    /// Table index -> live exit profile of a landed region function: which
    /// pages its (sampled) exits transfer control to. This is the measured
    /// signal that drives incremental region EXTENSION — a region grows only
    /// along traffic it demonstrably loses dispatches to, never from
    /// reachability guesses (which glued cold callees into hot regions and
    /// regressed the FP rows; see build_superblock).
    region_exits: std::collections::HashMap<i32, RegionExits>,
    /// Regions whose sampled out-of-region exit count crossed EXT_TRIGGER,
    /// awaiting a build slot at a quantum boundary.
    ext_queue: Vec<i32>,
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
            succ: vec![(NO_PC, 0, 0); DISPATCH_SIZE],
            ic_done: Default::default(),
            interp_hot: vec![0; DISPATCH_SIZE],
            interp_hot_tag: vec![0; DISPATCH_SIZE],
            page_blocks: Default::default(),
            superblocked: Default::default(),
            regions: Default::default(),
            region_exits: Default::default(),
            ext_queue: Vec::new(),
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
            *e = (NO_PC, 0, 0);
        }
        self.ic_done.clear();
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
/// Instruction fuel granted to the CURRENT dispatch (see JitLayout::fuel_addr):
/// compiled loops/superblocks yield once they retire this many instructions,
/// so caller budgets and the interrupt quantum hold to block granularity.
static mut FUEL_CELL: u64 = 0;
/// Diagnostics: emitted copy-loop fast paths bump this cell once per bulk
/// chunk (the emitter receives its address via JitLayout.copystat_addr).
static mut COPY_CHUNKS: u64 = 0;

fn retired_addr() -> u32 {
    (&raw const RETIRED_CELL) as u32
}

fn fuel_addr() -> u32 {
    (&raw const FUEL_CELL) as u32
}

fn copystat_addr() -> u32 {
    (&raw const COPY_CHUNKS) as u32
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
}

/// Virtual pages a superblock region may span. Loops and functions straddle
/// page boundaries constantly; a page-clamped region turns every crossing into
/// a host dispatch (measured: nbench NUMERIC SORT ran its sift loop as six
/// 2-10 instruction blocks, 5.6 insns per dispatch, because the loop sits
/// across a page boundary).
// 3, not 8: the call-graph BFS at 8 pages regressed CPython's eval loop
// 2.4s -> 7s (big glued functions rebuild more and codegen worse); at 3 the
// sparse mechanics keep loop-straddling pages together and pull in ONE hot
// callee, which is where the measured value was.
const MAX_REGION_PAGES: usize = 3;
/// Pages an EXTENDED region may reach (translate_superblock_sparse caps hard
/// at 16). Extension only ever grows along measured exit traffic, so the cap
/// bounds V8 compile cost (~4KB/page of module bytes, 15-40ms per 8-page
/// async build measured), not guesswork about reachability. tcc's hot call
/// graph clusters at ~15 pages — an 8-page cap left its calls crossing
/// region boundaries forever.
const MAX_EXT_REGION_PAGES: usize = 16;
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
const SB_IDX_BIT: i32 = rv64_jit::SB_IDX_BIT;
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
static mut DPROF_TRACE_MIX: [u64; 5] = [0; 5];
static mut DPROF_TRACE_MEM: [u64; 10] = [0; 10];
static mut DPROF_TRACE_CONTROL: [u64; 3] = [0; 3];
static mut DPROF_TRACE_ALU: [u64; 5] = [0; 5];

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
            DPROF_TRACE_MIX = [0; 5];
            DPROF_TRACE_MEM = [0; 10];
            DPROF_TRACE_CONTROL = [0; 3];
            DPROF_TRACE_ALU = [0; 5];
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
            _ => 0,
        }
    }
}

// JIT tier-up threshold. Settable at runtime (jit_set_enabled) so
// benchmarks can compare against the pure wasm interpreter.
/// Compile a block after it is dispatched this many times. High enough
/// that one-shot boot code stays interpreted; low enough that compute
/// loops (dispatched millions of times) tier up quickly.
const JIT_ON_THRESHOLD: u32 = 64;
static mut JIT_THRESHOLD: u32 = 64;
/// Tier-up threshold for the per-EXECUTION interp-stretch counter. Deliberately
/// much higher than JIT_THRESHOLD (which counts block-entry events): blocks and
/// hot-counts persist across context switches now, so a low per-execution bar
/// makes boot synchronously compile ~19k one-shot cold blocks (~0.1ms of
/// WebAssembly.Module each = seconds of boot). Steady-state hot code executes
/// millions of times and crosses 1024 in microseconds.
const INTERP_HOT_THRESHOLD: u16 = 2048;
/// Interpreter fallback slice once JIT blocks exist (tuned below).
const SYS_WARM_SLICE: u64 = 256;

/// Enable/disable JIT tier-up (1/0). Disabling sets the threshold beyond
/// any counter so blocks are never compiled — pure interpreter baseline.
#[no_mangle]
pub extern "C" fn jit_set_enabled(on: u32) {
    unsafe {
        JIT_THRESHOLD = if on == 0 { u32::MAX } else { JIT_ON_THRESHOLD };
        // "Disabled" means EXECUTE NO JIT CODE, not just "stop compiling":
        // drop already-compiled blocks so A/B comparisons and the API name
        // stay honest (PERFORMANCE_PROGRESS.md). (Wasm function-table entries are not
        // reclaimable, but they become unreachable.)
        if on == 0 {
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
/// Leaders per superblock. Every entry into the function loads the register
/// UNION over all its bodies and every exit stores the written union, so a
/// function that covers more of the page pays more on each entry — worth it
/// for code that then stays inside (IDEA), ruinous for code that re-enters
/// constantly (FOURIER's cross-page libm calls). Hot pcs are seeded first, so
/// the cap trims cold reachable code, not the hot core.
const MAX_LEADERS: usize = 512;

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
        let f: extern "C" fn(i32) = core::mem::transmute(idx as usize);
        f(state_ptr as i32);
    }
}

// Standalone superblock-emitter validation: compile the sum-1..10 loop as a
// 2-entry superblock and run it; must return 55 (x1). Exercises the internal
// br_table dispatch, register-in-locals across blocks, loop back-edge and exit.
static mut SBSTATE: [u64; 40] = [0; 40];
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sbtest() -> u64 {
    const PROG: [u32; 7] = [
        0x00000093, 0x00100113, 0x00b00193, 0x002080b3, 0x00110113, 0xfe311ce3, 0x00000073,
    ];
    let code: Vec<u8> = PROG.iter().flat_map(|w| w.to_le_bytes()).collect();
    let base = 0x1000u64;
    unsafe {
        SBSTATE = [0; 40];
        let sp = SBSTATE.as_ptr() as u32;
        SBSTATE[32] = base; // pc
        let lay = rv64_jit::JitLayout {
            x_base: sp,
            pc_addr: sp + 256,
            mem: None,
            sys: None,
            mem_profile: None,
            reg_stress: false,
            reg_profile_base: 0,
            multi_latch: false,
            retired_addr: sp + 264,
            f_base: 0,
            fcsr_addr: 0,
            fuel_addr: 0,
            mstatus_addr: 0,
            copystat_addr: 0,
            chain_off_addr: 0,
            batch_base_addr: 0,
            dispatch_base: 0,
            dispatch_mask: 0,
            map_gen_addr: 0,
        };
        let entries = [0x1000u64, 0x100c];
        let blk = match rv64_jit::translate_superblock(&code, base, 0x1000, 0x40, &entries, lay) {
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

/// Run the loaded program with JIT tier-up. STOP_EXITED on exit,
/// STOP_BUDGET if out of fuel, STOP_TRAP on unhandled trap.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn user_run(budget: u64) -> i32 {
    let e = unsafe { USER.as_mut().expect("call user_load() first") };
    let jit = unsafe { USER_JIT.get_or_insert_with(JitState::new) };
    let mut host = JsHost;
    let m = &mut e.machine;
    let mut remaining = budget;

    loop {
        // --- JIT fast path: direct-mapped dispatch, chain blocks ---
        let mut chained = 0u32;
        while chained < JIT_CHAIN_CAP && remaining > 0 {
            unsafe { FUEL_CELL = remaining };
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
            m.cpu.insn_count += retired;
            unsafe {
                JIT_RETIRED += retired;
                JIT_DISPATCHES += 1;
            }
            remaining = remaining.saturating_sub(retired);
            chained += 1;
            if remaining == 0 {
                return STOP_BUDGET;
            }
        }

        // --- hot counting + compile ---
        let pc = m.cpu.pc;
        if !jit_table_full() && !jit.cache.contains_key(&pc) {
            let c = jit.hot.entry(pc).or_insert(0);
            *c += 1;
            if *c >= unsafe { JIT_THRESHOLD } {
                let lay = rv64_jit::JitLayout {
                    x_base: m.cpu.x.as_ptr() as u32,
                    pc_addr: &m.cpu.pc as *const u64 as u32,
                    mem: Some((m.mem.as_ptr() as u32, m.mem.len() as u64)),
                    sys: None,
                    mem_profile: None,
                    reg_stress: false,
                    reg_profile_base: 0,
                    multi_latch: false,
                    retired_addr: retired_addr(),
                    f_base: m.cpu.f.as_ptr() as u32,
                    fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
                    fuel_addr: fuel_addr(),
                    mstatus_addr: 0, // user mode: no privileged FP state
                    copystat_addr: 0,
                    chain_off_addr: 0,
                    batch_base_addr: 0,
                    dispatch_base: 0,
                    dispatch_mask: 0,
                    map_gen_addr: 0,
                };
                let end = (pc as usize + 1024).min(m.mem.len());
                let entry = rv64_jit::translate_block(&m.mem[pc as usize..end], pc, pc, lay)
                    .and_then(|blk| {
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
        let stop = {
            let mut bus = FlatMemory::new(0, &mut m.mem);
            m.cpu.run(&mut bus, slice)
        };
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
static mut VIRT: Option<rv64_system::virt::VirtMachine> = None;

stage_into!(virt_stage_opensbi, VIRT_OPENSBI);
stage_into!(virt_stage_kernel, VIRT_KERNEL);
stage_into!(virt_stage_initrd, VIRT_INITRD);
stage_into!(virt_stage_disk, VIRT_DISK);
stage_into!(virt_stage_cmdline, VIRT_CMDLINE);
stage_into!(virt_stage_net_mac, VIRT_NET_MAC);

/// Give the next modern virt machine a virtio-net NIC.
#[no_mangle]
pub extern "C" fn virt_net_enable(on: u32) {
    unsafe { VIRT_NET_ON = on != 0 }
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
            net,
        };
        let mut machine = if direct {
            rv64_system::virt::VirtMachine::new_direct(u64::from(ram_mb) << 20, images)
        } else {
            rv64_system::virt::VirtMachine::new(u64::from(ram_mb) << 20, images)
        };
        machine.set_rtc_unix_ns(host_unix_ms() as u64 * 1_000_000);
        VIRT_OPENSBI.clear();
        VIRT_KERNEL.clear();
        VIRT_INITRD.clear();
        VIRT_CMDLINE.clear();
        VIRT = Some(machine);
    }
}

/// Run one modern-machine slice and stream UART output to the host.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_run(max_insns: u64) -> i32 {
    let machine = unsafe { VIRT.as_mut().expect("call virt_boot() first") };
    machine.set_rtc_unix_ns(unsafe { host_unix_ms() } as u64 * 1_000_000);
    machine.run_slice(max_insns);
    let out = machine.console_output();
    if !out.is_empty() {
        unsafe { host_write(1, out.as_ptr(), out.len()) }
    }
    pump_virt_net(machine);
    machine.power_off as i32
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_console_input() {
    let machine = unsafe { VIRT.as_mut().expect("call virt_boot() first") };
    let bytes = unsafe { core::mem::take(&mut STAGING) };
    machine.console_input(&bytes);
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

/// Current modern-machine guest PC (diagnostic: boot and workload profiling).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn virt_pc() -> u64 {
    unsafe { VIRT.as_ref().map_or(0, |m| m.cpu.pc) }
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
        SLICE_CALLS = 0;
        SLICE_INSNS = 0;
    }
}

/// The JIT's view of machine state (register file, fcsr, TLB tables, budget
/// cells) — identical for every translation of the current machine.
fn jit_layout(m: &rv64_system::Machine) -> rv64_jit::JitLayout {
    let (lt, lo, st, so) = m.cpu.jit_ftlb_ptrs();
    rv64_jit::JitLayout {
        x_base: m.cpu.x.as_ptr() as u32,
        pc_addr: &m.cpu.pc as *const u64 as u32,
        mem: None,
        sys: Some(rv64_jit::SysMem {
            ftlb_load_tag: lt as u32,
            ftlb_load_off: lo as u32,
            ftlb_store_tag: st as u32,
            ftlb_store_off: so as u32,
            tlb_mask: (rv64_core::Cpu::jit_tlb_size() - 1) as u32,
        }),
        mem_profile: mem_profile_layout(),
        reg_stress: reg_stress(),
        reg_profile_base: reg_profile_base(),
        multi_latch: unsafe { MULTI_LATCH },
        retired_addr: retired_addr(),
        f_base: m.cpu.f.as_ptr() as u32,
        fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
        fuel_addr: fuel_addr(),
        mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
        copystat_addr: copystat_addr(),
        chain_off_addr: chain_off_addr(),
        batch_base_addr: 0,
        dispatch_base: 0,
        dispatch_mask: 0,
        map_gen_addr: 0,
    }
}

/// Build (asynchronously) the superblock covering `vpage` in address space
/// `aspace`, whose current physical page is `pa_page`. Returns true if a
/// module was issued. Called from the compile path and, when the compile
/// budget deferred one, from the quantum boundary.
#[allow(static_mut_refs)]
fn build_superblock(
    m: &mut rv64_system::Machine,
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
    let ram_ok = |m: &rv64_system::Machine, pa: u64| {
        pa >= rv64_system::RAM_BASE
            && ((pa & !0xfff) - rv64_system::RAM_BASE) as usize + 0x1000 <= m.bus.ram.len()
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
    let mut pages: Vec<(u64, u64)> = vec![(vpage, pa_page)];
    let probe_add = |m: &mut rv64_system::Machine, pages: &mut Vec<(u64, u64)>, va: u64| {
        if pages.len() >= MAX_REGION_PAGES || pages.iter().any(|&(v, _)| v == va) {
            return;
        }
        if let Some(p) = m.cpu.jit_probe_fetch(&mut m.bus, va) {
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
    while pages.len() < MAX_REGION_PAGES && hot(jit, va) {
        let before = pages.len();
        probe_add(m, &mut pages, va);
        if pages.len() == before {
            break;
        }
        va += 0x1000;
    }
    let mut va = vpage.wrapping_sub(0x1000);
    while va < vpage && pages.len() < MAX_REGION_PAGES && hot(jit, va) {
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
        let targets = rv64_jit::page_call_targets(&m.bus.ram[o..o + 0x1000], va);
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
    m: &mut rv64_system::Machine,
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
    m: &mut rv64_system::Machine,
    jit: &mut JitState,
    aspace: u64,
    lead: u64,
    mut pages: Vec<(u64, u64)>,
    sb_compiles: u32,
    n_entries: usize,
    unproductive: bool,
    regs_in_memory: bool,
) -> bool {
    let mut lay = jit_layout(m);
    lay.dispatch_base = jit.dispatch.as_ptr() as u32;
    lay.dispatch_mask = (DISPATCH_SIZE - 1) as u32;
    lay.map_gen_addr = m.cpu.jit_map_gen_ptr() as u32;
    // Ascending order keeps virtually contiguous pages adjacent in the
    // concat, which is what lets bodies flow across their shared boundary.
    pages.sort_unstable_by_key(|&(va, _)| va);
    let mut code = Vec::with_capacity(pages.len() * 0x1000);
    for &(_, pp) in &pages {
        let o = (pp - rv64_system::RAM_BASE) as usize;
        code.extend_from_slice(&m.bus.ram[o..o + 0x1000]);
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
    let mut i = 0usize;
    while i < pages.len() && entries.len() < MAX_LEADERS {
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
            let (mut l, back) = rv64_jit::discover_page_leaders_ext(
                run_slice,
                run_va,
                run_va,
                run_span,
                &rseeds,
                MAX_LEADERS - entries.len(),
            );
            l.retain(|&e| {
                rv64_jit::emittable_at(run_slice, run_va, e, lay)
                    && (!back.contains(&e) || !rv64_jit::is_loop_at(run_slice, run_va, e, lay))
            });
            entries.extend(l);
        }
        i = j + 1;
    }
    let sb = rv64_jit::translate_superblock_sparse(&code, &vas, &entries, lay, regs_in_memory);
    if sb.is_none() {
        unsafe { SB_XLATE_FAIL += 1 };
        return false;
    }
    let blk = sb.unwrap();
    unsafe { JIT_OUT = blk.wasm };
    for &(_, pp) in &pages {
        m.bus.jit_mark_page(pp);
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
            m.cpu.insn_count,
        ),
    );
    m.cpu.clear_store_jtlb(); // pages may now hold code
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
        });
        host_jit_register_async(ticket);
        SB_ISSUED += 1;
        SB_LAST_ICOUNT = m.cpu.insn_count;
    }
    // The caller still gives its pc an individual block right now; the
    // region function repoints the entries when the module arrives.
    true
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
fn drain_ext_queue(m: &mut rv64_system::Machine, jit: &mut JitState) {
    if jit.ext_queue.is_empty()
        || m.cpu.insn_count < unsafe { SB_EXT_NEXT_ICOUNT }
        || !sb_build_allowed(m.cpu.insn_count)
    {
        return;
    }
    unsafe { SB_EXT_DRAIN_VISITS += 1 };
    let aspace = m.cpu.sys.as_ref().map_or(0, |c| c.satp);
    if let Some(i) = jit.ext_queue.iter().position(|idx| {
        jit.region_exits
            .get(idx)
            .is_some_and(|r| r.aspace == aspace)
    }) {
        let idx = jit.ext_queue.remove(i);
        try_extend_region(m, jit, idx);
    } else {
        unsafe { SB_EXT_DRAIN_NOMATCH += 1 };
        // Nothing for this address space: back off before rescanning, and
        // drop entries that no longer resolve at all.
        unsafe { SB_EXT_NEXT_ICOUNT = m.cpu.insn_count + SB_MIN_SPACING };
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
fn try_extend_region(m: &mut rv64_system::Machine, jit: &mut JitState, idx: i32) {
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
    if m.cpu.insn_count < when.wrapping_add(cooldown) || compiles >= SB_RECOMPILE_CAP {
        // Not yet (or allowance spent): let the counters re-arm — the next
        // EXT_TRIGGER crossing re-queues it.
        unsafe { SB_EXT_DEFER_COOL += 1 };
        if let Some(r) = jit.region_exits.get_mut(&idx) {
            r.total = EXT_TRIGGER / 2;
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
        if pages.len() >= MAX_EXT_REGION_PAGES {
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
            && (pa - rv64_system::RAM_BASE) as usize + 0x1000 <= m.bus.ram.len();
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
    let jit = unsafe { SYS_JIT.get_or_insert_with(JitState::new) };
    let mut remaining = max_insns;

    while remaining > 0 && !m.power_off {
        // Refresh the wall-clock time source (opt-in) so the CLINT tracks real
        // host time. host_now_ms is a wasm->JS round-trip (~7% of a dispatch-
        // heavy workload if done per iteration), so gate it: refresh only after
        // ~16k retired insns (~40us at JIT speed, far finer than the 10ms kernel
        // tick) or after 64 iterations without insn progress (WFI idle — time
        // must still advance or timers never fire).
        if unsafe { SYS_WALLCLOCK } {
            let ic = m.cpu.insn_count;
            let due =
                unsafe { ic.wrapping_sub(WALL_LAST_ICOUNT) >= 16384 || WALL_IDLE_ITERS >= 64 };
            if due {
                unsafe {
                    WALL_LAST_ICOUNT = ic;
                    WALL_IDLE_ITERS = 0;
                }
                m.wall_ns = Some(unsafe { host_now_ms() } as u64 * 1_000_000);
                m.wall_anchor_icount = ic;
            } else {
                unsafe { WALL_IDLE_ITERS += 1 };
            }
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
        if m.cpu.jit_flush_gen != jit.flush_gen {
            jit.flush_gen = m.cpu.jit_flush_gen;
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
        if !m.bus.jit_dirty_pages.is_empty() {
            let dirty = m.bus.jit_take_dirty();
            let mut dirty_vpages: std::collections::HashSet<u64> = Default::default();
            unsafe {
                DIRTY_EVENTS += dirty.len() as u64;
                // The trace-window gathers may hold pre-store bytes.
                TRACE_WIN.clear();
            }
            for &ppage in &dirty {
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
                m.bus.jit_unmark_page(ppage);
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
        // --- JIT fast path: direct-mapped dispatch + cheap pa-verify ---
        // Per-dispatch bookkeeping accumulates in LOCALS and flushes once after
        // the chain: at ~200M+ dispatches per second of guest compute, the five
        // read-modify-writes this loop used to do per iteration (insn_count,
        // remaining, two stat counters, chain counter) were a measurable slice
        // of total wall time. map_gen is hoisted too — blocks can't execute
        // satp/SFENCE (SYSTEM never compiles; blocks bail AT it), so it cannot
        // move inside a chain.
        let map_gen = m.cpu.map_gen as u32;
        let mptr = m as *mut rv64_system::Machine as *mut u8;
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
            let pc = m.cpu.pc;
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
            let idx = if line.pc == pc && line.gen == map_gen {
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
                            m.cpu.jit_probe_fetch(&mut m.bus, pc), Some(pa) if pa == b.pa
                        );
                        let region_ok = region.is_none_or(|pgs| {
                            pgs.iter().all(|&(va, pp)| {
                                matches!(
                                    m.cpu.jit_probe_fetch(&mut m.bus, va),
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
            call_block(idx & !SB_IDX_BIT, mptr);
            // Observed successor + stability count (JitState::succ). A
            // trace ends at its first indirect jump, so this records where
            // that jump actually goes. Once the target is proven stable,
            // drop the block ONCE so it recompiles with an inline cache
            // that continues through the edge — trace EXTENSION at a hot
            // side exit, the mechanism that reduces dispatch COUNT for
            // indirect-heavy code. (The oracle is empty at first compile
            // by construction: the pc has never dispatched yet.)
            {
                let sl = JitState::dslot(pc);
                let e = &mut jit.succ[sl];
                if e.0 == pc && e.1 == m.cpu.pc {
                    e.2 = e.2.saturating_add(1);
                } else {
                    *e = (pc, m.cpu.pc, 1);
                }
                if e.2 == unsafe { IC_EXTEND_TRIGGER } && !jit.ic_done.contains(&pc) {
                    jit.ic_done.insert(pc);
                    jit.cache.remove(&pc);
                    jit.dispatch[sl].pc = NO_PC;
                    unsafe { IC_EXTENDS += 1 };
                    break; // recompile on the next pass through tier-up
                }
            }
            // Sampled exit attribution: after a region function returns,
            // cpu.pc holds the pc it exited TO. Out-of-region targets are
            // the measured signal for incremental extension.
            if idx & SB_IDX_BIT != 0 {
                let tick = unsafe {
                    EXIT_TICK = EXIT_TICK.wrapping_add(1);
                    EXIT_TICK
                };
                if tick & ((1 << EXIT_SAMPLE_SHIFT) - 1) == 0 {
                    let stay = unsafe { RETIRED_CELL };
                    record_region_exit(jit, idx & !SB_IDX_BIT, m.cpu.pc, stay);
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
                eprof_hit(pc, m.cpu.pc, retired);
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
                    let fcsr = m.cpu.fcsr;
                    let fs = m.cpu.sys.as_ref().map_or(3, |c| (c.mstatus >> 13) & 3);
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
        m.cpu.insn_count += retired_sum;
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
            if unsafe { SYS_WALLCLOCK } {
                m.wall_ns = Some(unsafe { host_now_ms() } as u64 * 1_000_000);
                m.wall_anchor_icount = m.cpu.insn_count;
                unsafe { WALL_LAST_ICOUNT = m.cpu.insn_count };
            }
            m.sync_devices();
            m.cpu.check_interrupts(&mut m.bus);
            chain_ctl_boundary(m.cpu.insn_count);
            // Extension FIRST: a landed region whose measured exits keep
            // leaving it grows along that traffic. Extensions outrank fresh
            // page builds for the build budget — a fresh 3-page function
            // over call-heavy code exits immediately and holds nothing,
            // while an extension is provably where dispatches are lost
            // (drained behind the page queue, tcc got 3 extensions against
            // 179 page builds and kept its 8-insn dispatches).
            drain_ext_queue(m, jit);
            // Then spend what's left on the oldest deferred page that still
            // resolves in the CURRENT address space (issuing an extension
            // above moved SB_LAST_ICOUNT, so at most one build per boundary).
            if !jit.sb_queue.is_empty() && sb_build_allowed(m.cpu.insn_count) {
                let aspace = m.cpu.sys.as_ref().map_or(0, |c| c.satp);
                if let Some(i) = jit.sb_queue.iter().position(|&(a, _)| a == aspace) {
                    let (_, vpage) = jit.sb_queue.remove(i);
                    let compiles = jit.sb_gen.get(&(aspace, vpage)).map_or(0, |&(_, c, _)| c);
                    if let Some(pa) = m.cpu.jit_probe_fetch(&mut m.bus, vpage) {
                        if pa >= rv64_system::RAM_BASE
                            && ((pa & !0xfff) - rv64_system::RAM_BASE) as usize + 0x1000
                                <= m.bus.ram.len()
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
        drain_ext_queue(m, jit);

        // --- hot counting + compile (from physical code bytes) ---
        let pc = m.cpu.pc;
        // Address space this discovery belongs to (satp; 0 in bare mode).
        let aspace = m.cpu.sys.as_ref().map_or(0, |c| c.satp);
        if unsafe { DPROF_ON } {
            if let Some(pa) = m.cpu.jit_probe_fetch(&mut m.bus, pc) {
                let o = (pa.wrapping_sub(rv64_system::RAM_BASE)) as usize;
                if pa >= rv64_system::RAM_BASE && o + 4 <= m.bus.ram.len() {
                    ihist_hit(u32::from_le_bytes([
                        m.bus.ram[o],
                        m.bus.ram[o + 1],
                        m.bus.ram[o + 2],
                        m.bus.ram[o + 3],
                    ]));
                }
            }
        }
        if !jit_table_full() && !jit.cache.contains_key(&pc) {
            let hot = {
                let c = jit.hot.entry(pc).or_insert(0);
                *c += 1;
                *c
            };
            if hot >= unsafe { JIT_THRESHOLD } {
                if let Some(pa) = m.cpu.jit_probe_fetch(&mut m.bus, pc) {
                    if pa >= rv64_system::RAM_BASE {
                        let (lt, lo, st, so) = m.cpu.jit_ftlb_ptrs();
                        let sysmem = rv64_jit::SysMem {
                            ftlb_load_tag: lt as u32,
                            ftlb_load_off: lo as u32,
                            ftlb_store_tag: st as u32,
                            ftlb_store_off: so as u32,
                            tlb_mask: (rv64_core::Cpu::jit_tlb_size() - 1) as u32,
                        };
                        let lay = rv64_jit::JitLayout {
                            x_base: m.cpu.x.as_ptr() as u32,
                            pc_addr: &m.cpu.pc as *const u64 as u32,
                            mem: None,
                            sys: Some(sysmem),
                            mem_profile: mem_profile_layout(),
                            reg_stress: reg_stress(),
                            reg_profile_base: reg_profile_base(),
                            multi_latch: unsafe { MULTI_LATCH },
                            retired_addr: retired_addr(),
                            f_base: m.cpu.f.as_ptr() as u32,
                            fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
                            fuel_addr: fuel_addr(),
                            mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
                            copystat_addr: copystat_addr(),
                            chain_off_addr: chain_off_addr(),
                            batch_base_addr: 0,
                            dispatch_base: jit.dispatch.as_ptr() as u32,
                            dispatch_mask: (DISPATCH_SIZE - 1) as u32,
                            map_gen_addr: m.cpu.jit_map_gen_ptr() as u32,
                        };
                        let vpage = pc & !0xfff;
                        let pa_page = pa & !0xfff;
                        let pa_page_off = (pa_page - rv64_system::RAM_BASE) as usize;
                        let off = (pa - rv64_system::RAM_BASE) as usize;
                        let end = ((off + 1024).min(off | 0xfff) + 1).min(m.bus.ram.len());
                        // Superblock path (opt-in): loop headers stay individual
                        // (tight wasm loop); non-loop pages accumulate entries and
                        // upgrade to a page superblock once branchy enough.
                        let (is_loop, n_entries) = if unsafe { SYS_SUPERBLOCK } {
                            let il = rv64_jit::is_loop_at(&m.bus.ram[off..end], pc, pc, lay);
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
                        let sb_cool = m.cpu.insn_count >= sb_when.wrapping_add(cooldown);
                        // Recompile on DOUBLING, not on a fixed increment: a
                        // page discovered 6 hot pcs at a time would need 20
                        // recompiles to cover the 120 that nbench IDEA ends up
                        // with, so a fixed cap left most of the page on
                        // individual blocks forever. Doubling covers a page of
                        // any size in a handful of compiles and is
                        // self-amortizing — each one costs at most as much as
                        // all the previous ones together.
                        let sb_spaced = sb_build_allowed(m.cpu.insn_count);
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
                        if !is_loop && sb_want && pa_page_off + 0x1000 <= m.bus.ram.len() {
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
                            _ => rv64_jit::trace_level() == 0,
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
                        if !m.bus.jit_dirty_pages.is_empty() {
                            wins.clear();
                        }
                        let mg = m.cpu.map_gen;
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
                                    if let Some(p) = m.cpu.jit_probe_fetch(&mut m.bus, va) {
                                        let pp = p & !0xfff;
                                        if pp >= rv64_system::RAM_BASE
                                            && (pp - rv64_system::RAM_BASE) as usize + 0x1000
                                                <= m.bus.ram.len()
                                        {
                                            let o = (pp - rv64_system::RAM_BASE) as usize;
                                            let bo = (k * 0x1000) as usize;
                                            w.buf[bo..bo + 0x1000]
                                                .copy_from_slice(&m.bus.ram[o..o + 0x1000]);
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
                                (e.0 == t).then_some(e.1)
                            };
                            rv64_jit::translate_batch_obs(
                                &w.buf,
                                w.first_va,
                                pc,
                                blay,
                                &hot,
                                &want,
                                &next,
                                unsafe { BATCH_CAP },
                            )
                        } else {
                            None
                        };
                        if let Some((wasm, members)) = batch {
                            unsafe {
                                SB_BUILD_MS += host_now_ms() - batch_t0;
                                SB_LAST_ICOUNT = m.cpu.insn_count;
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
                                let gi = (m.cpu.insn_count / 1_000_000_000).max(1);
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
                                            m.bus.jit_mark_page(pp);
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
                                    m.cpu.clear_store_jtlb();
                                    continue; // dispatch the seed member now
                                }
                            }
                        }
                        let blk = {
                            // Hotness oracle for branch-direction bias: a
                            // compiled (non-blacklisted) target is proven-hot.
                            let cache = &jit.cache;
                            let hot = |t: u64| matches!(cache.get(&t), Some(Some(b)) if b.idx >= 0);
                            // Inline-cache oracle: the target this pc's
                            // indirect jump was last observed to take.
                            let succ = &jit.succ;
                            let next = |t: u64| {
                                let e = succ[JitState::dslot(t)];
                                (e.0 == t && e.2 >= unsafe { IC_EXTEND_TRIGGER }).then_some(e.1)
                            };
                            rv64_jit::translate_block_ic(
                                &w.buf,
                                w.first_va,
                                pc,
                                lay,
                                &hot,
                                &|_| None,
                                &next,
                            )
                        };
                        let entry = blk.and_then(|blk| {
                            // Pages the emitted code actually came from
                            // ((0,0) span = wholly within [pc, pc+len)).
                            let (lo, hi) = if blk.span == (0, 0) {
                                (pc, pc + blk.len.max(2))
                            } else {
                                blk.span
                            };
                            let mut spanned: Vec<(u64, u64)> = Vec::new();
                            let mut va = lo & !0xfff;
                            while va <= (hi - 1) & !0xfff {
                                let Some(&(_, pp)) = winpages.iter().find(|&&(v, _)| v == va)
                                else {
                                    return None; // span escaped the window (impossible)
                                };
                                spanned.push((va, pp));
                                va += 0x1000;
                            }
                            unsafe { JIT_OUT = blk.wasm };
                            let idx = unsafe { host_jit_register() };
                            if idx < 0 {
                                return None;
                            }
                            unsafe { JIT_TABLE_ENTRIES += 1 };
                            for &(_, pp) in &spanned {
                                m.bus.jit_mark_page(pp);
                            }
                            m.cpu.clear_store_jtlb(); // these pages may now hold code
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
                                m.bus.jit_mark_page(pa);
                                m.cpu.clear_store_jtlb();
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
        if jit.cache.is_empty() {
            // Cold: no compiled blocks to return to — one big slice avoids
            // dispatch churn before any block exists.
            let ran = m.run_slice(remaining.min(4096));
            unsafe {
                SLICE_CALLS += 1;
                SLICE_INSNS += ran;
            }
            remaining = remaining.saturating_sub(ran.max(1));
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
            let icount_before = m.cpu.insn_count;
            let ran = m.run_slice_until(remaining.min(SYS_WARM_SLICE), |pc| {
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
            });
            unsafe {
                SLICE_CALLS += 1;
                SLICE_INSNS += ran;
                if DPROF_ON && IHIST_LAST != usize::MAX {
                    // Charge the whole interpreted stretch to whatever the JIT
                    // gave up on: one unsupported instruction can drag dozens
                    // of interpreted instructions behind it.
                    IHIST_INSNS[IHIST_LAST] += m.cpu.insn_count - icount_before;
                    IHIST_LAST = usize::MAX;
                }
            }
            remaining = remaining.saturating_sub(ran.max(1));
        }

        // Stream console output at quantum granularity, DURING execution —
        // buffering until sys_run returns skews benchmark timing: a marker
        // printed early in a slice would be timestamped after the whole slice
        // (v86 timestamps serial bytes as they arrive; symmetry demands we
        // surface output comparably; see PERFORMANCE_PROGRESS.md).
        let out = m.console_output();
        if !out.is_empty() {
            unsafe { host_write(1, out.as_ptr(), out.len()) }
        }
        pump_net(m);
    }

    let out = m.console_output();
    if !out.is_empty() {
        unsafe { host_write(1, out.as_ptr(), out.len()) }
    }
    pump_net(m);
    m.power_off as i32
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
        let Some(jit) = SYS_JIT.as_mut() else { return };
        let m = &mut *(state as *mut rv64_system::Machine);
        // Fuel: the cumulative retired cell against this dispatch's grant.
        if RETIRED_CELL >= FUEL_CELL {
            return;
        }
        let pc = m.cpu.pc;
        let line = jit.dispatch[JitState::dslot(pc)];
        if line.pc != pc || line.gen != m.cpu.map_gen as u32 || line.idx < 0 {
            return; // miss/blacklist/stale: the host loop owns the slow path
        }
        CHAIN_DEPTH += 1;
        CHAIN_HOPS += 1;
        let f: extern "C" fn(i32) = core::mem::transmute((line.idx & !SB_IDX_BIT) as usize);
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
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_sb_ready(ticket: u64, idx: i32) {
    unsafe {
        let Some(pos) = PENDING_SB.iter().position(|p| p.ticket == ticket) else {
            return;
        };
        let p = PENDING_SB.swap_remove(pos);
        if idx < 0 || p.boot_gen != BOOT_GEN {
            return;
        }
        let Some(m) = SYS.as_mut() else { return };
        let Some(jit) = SYS_JIT.as_mut() else { return };
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
        let stale = p.pages.iter().any(|&(_, pp)| {
            let ppage = (pp - rv64_system::RAM_BASE) >> 12;
            !m.bus.jit_page_marked(ppage) || m.bus.jit_dirty_pages.contains(&ppage)
        });
        if stale {
            SB_STALE += 1;
            for &(va, _) in &p.pages {
                jit.superblocked.remove(&(p.aspace, va));
            }

            return;
        }
        SB_LANDED += 1;
        JIT_TABLE_ENTRIES += 1;
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
                if b.n != 0 && !b.fp && b.n >= TRACE_KEEP_MIN)
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
            // Invalidate any line still pointing at the old individual block.
            let slot = JitState::dslot(e);
            if jit.dispatch[slot].pc == e {
                jit.dispatch[slot].pc = NO_PC;
            }
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
        let (lt, lo, st, so) = m.cpu.jit_ftlb_ptrs();
        let lay = rv64_jit::JitLayout {
            x_base: m.cpu.x.as_ptr() as u32,
            pc_addr: &m.cpu.pc as *const u64 as u32,
            mem: None,
            sys: Some(rv64_jit::SysMem {
                ftlb_load_tag: lt as u32,
                ftlb_load_off: lo as u32,
                ftlb_store_tag: st as u32,
                ftlb_store_off: so as u32,
                tlb_mask: (rv64_core::Cpu::jit_tlb_size() - 1) as u32,
            }),
            mem_profile: mem_profile_layout(),
            reg_stress: reg_stress(),
            reg_profile_base: reg_profile_base(),
            multi_latch: MULTI_LATCH,
            retired_addr: retired_addr(),
            f_base: m.cpu.f.as_ptr() as u32,
            fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
            fuel_addr: fuel_addr(),
            mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
            copystat_addr: copystat_addr(),
            chain_off_addr: chain_off_addr(),
            batch_base_addr: 0,
            dispatch_base: 0,
            dispatch_mask: 0,
            map_gen_addr: 0,
        };
        let empty = Vec::new();
        let aspace = m.cpu.sys.as_ref().map_or(0, |c| c.satp);
        let seeds = jit.page_entries.get(&(aspace, vpage)).unwrap_or(&empty);
        let leaders = rv64_jit::discover_page_leaders(code, vpage, vpage, 0x1000, seeds, 512);
        let is_loop = |e: u64| rv64_jit::is_loop_at(code, vpage, e, lay);
        if which >= 5 {
            let keep: Vec<u64> = leaders.iter().copied().filter(|&e| !is_loop(e)).collect();
            let (rm, wm, fr, fw) =
                rv64_jit::scan_regs_super_pub(code, vpage, vpage + 0x1000, &keep, &lay);
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
        let (lt, lo, st, so) = m.cpu.jit_ftlb_ptrs();
        let lay = rv64_jit::JitLayout {
            x_base: m.cpu.x.as_ptr() as u32,
            pc_addr: &m.cpu.pc as *const u64 as u32,
            mem: None,
            sys: Some(rv64_jit::SysMem {
                ftlb_load_tag: lt as u32,
                ftlb_load_off: lo as u32,
                ftlb_store_tag: st as u32,
                ftlb_store_off: so as u32,
                tlb_mask: (rv64_core::Cpu::jit_tlb_size() - 1) as u32,
            }),
            mem_profile: mem_profile_layout(),
            reg_stress: reg_stress(),
            reg_profile_base: reg_profile_base(),
            multi_latch: MULTI_LATCH,
            retired_addr: retired_addr(),
            f_base: m.cpu.f.as_ptr() as u32,
            fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
            fuel_addr: fuel_addr(),
            mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
            copystat_addr: copystat_addr(),
            chain_off_addr: chain_off_addr(),
            batch_base_addr: 0,
            dispatch_base: 0,
            dispatch_mask: 0,
            map_gen_addr: 0,
        };
        let code = &m.bus.ram[off..end];
        match which {
            0 => rv64_jit::is_loop_at(code, pc, pc, lay) as u64,
            3 => u32::from_le_bytes([code[0], code[1], code[2], code[3]]) as u64,
            _ => rv64_jit::translate_block(code, pc, pc, lay).map_or(0, |b| b.n_insns as u64),
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
    rv64_jit::set_hw_fma(on != 0);
}

/// Enable direct block-to-block tail-call chaining (host feature-detects
/// wasm tail-call support first). Loop/region chain sites follow this
/// directly (their successor sets are cyclic and stay monomorphic); trace
/// exits additionally require a small block population — flipped per
/// compile from the live cache size (see CHAIN_POP_CAP).
#[no_mangle]
pub extern "C" fn jit_set_tailcall(on: u32) {
    rv64_jit::set_chain(on != 0);
}
/// A/B: trace definedness tracking (rv64_jit::set_defined_track).
#[no_mangle]
pub extern "C" fn jit_set_defined(on: u32) {
    rv64_jit::set_defined_track(on != 0);
}
/// A/B: rotated-nest loop regions (see rv64_jit::set_rotated_nests).
#[no_mangle]
pub extern "C" fn jit_set_rotated_nests(on: u32) {
    rv64_jit::set_rotated_nests(on != 0);
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
/// Batch compilation (see rv64_jit::translate_batch). Members transfer by
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

#[allow(static_mut_refs)]
fn chain_ctl_boundary(retired_total: u64) {
    unsafe {
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

/// Trace aggressiveness for individual blocks (see rv64_jit::set_trace_level):
/// 0 = classic basic blocks, 1 = branch side-exits, 2 = +call following,
/// 3 = +return following (default).
#[no_mangle]
pub extern "C" fn jit_set_trace_level(l: u32) {
    rv64_jit::set_trace_level(l);
}

/// Toggle host-filled TLB misses inside compiled blocks (perf A/B).
#[no_mangle]
pub extern "C" fn jit_set_tlb_fill(on: u32) {
    rv64_jit::set_tlb_fill(on != 0);
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
        match SYS.as_mut() {
            Some(m) => m.cpu.jit_fill_tlb(&mut m.bus, va, store != 0).unwrap_or(-1),
            None => -1,
        }
    }
}

/// Current guest pc (diagnostic: host-side pc sampling for guest profiling).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_pc() -> u64 {
    unsafe { SYS.as_ref().map_or(0, |m| m.cpu.pc) }
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
        match rv64_jit::translate_block(&STAGING, base, pc, rv64_jit::JitLayout::bare()) {
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
