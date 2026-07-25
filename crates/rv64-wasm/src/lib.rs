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
    /// Fill with entropy (crypto.getRandomValues).
    fn host_random(ptr: *mut u8, len: usize);
    /// JIT: instantiate the wasm module currently in JIT_OUT (see
    /// jit_out_ptr/jit_out_len), append its `run` function to this module's
    /// exported function table, and return the table index (-1 on failure).
    fn host_jit_register() -> i32;
    /// One Ethernet frame the guest transmitted, for the page to forward over
    /// its WebSocket relay. Called at quantum granularity, like host_write.
    fn host_net_send(ptr: *const u8, len: usize);
    /// Async variant for large modules (page superblocks): compiles on V8
    /// background threads; sys_sb_ready(ticket, idx) fires between runSystem
    /// calls when the function is in the table (idx -1 = failed).
    fn host_jit_register_async(ticket: u64);
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
    /// Function-table index of the compiled block.
    idx: i32,
    /// Guest instructions it retires.
    n: u32,
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
const DISPATCH_BITS: u32 = 17; // 131072 lines (sys block count can exceed 16k)
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
    /// compile storms that depend on address layout (ISSUES.md P2).
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
    /// Table index -> the (virtual page, physical page) list a MULTI-page
    /// superblock was compiled over. Entries carry their own page's pa (probed
    /// like any block at dispatch); this is the rest of the region, verified on
    /// the same slow path so a region can never execute against a page that was
    /// remapped out from under it.
    regions: std::collections::HashMap<i32, Vec<(u64, u64)>>,
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
            interp_hot: vec![0; DISPATCH_SIZE],
            interp_hot_tag: vec![0; DISPATCH_SIZE],
            page_blocks: Default::default(),
            superblocked: Default::default(),
            regions: Default::default(),
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
        self.sb_gen.clear();
        self.sb_queue.clear();
        self.sb_missed.clear();
        for h in self.interp_hot.iter_mut() {
            *h = 0;
        }
        for t in self.interp_hot_tag.iter_mut() {
            *t = 0;
        }
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

#[allow(static_mut_refs)]
fn retired_addr() -> u32 {
    unsafe { &RETIRED_CELL as *const u64 as u32 }
}

fn fuel_addr() -> u32 {
    unsafe { &FUEL_CELL as *const u64 as u32 }
}

fn copystat_addr() -> u32 {
    unsafe { &COPY_CHUNKS as *const u64 as u32 }
}

/// Wasm function-table entries are unreclaimable (invalidated blocks become
/// unreachable but their slots persist), so unbounded compilation — reboots,
/// address-space churn, self-modifying code — would grow the table forever
/// (ISSUES.md P2). Above this bound the JIT stops COMPILING new blocks
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
    /// (virtual page, physical page) of every page in the compiled region,
    /// ascending and virtually contiguous.
    pages: Vec<(u64, u64)>,
    entries: Vec<u64>,
}

/// Virtual pages a superblock region may span. Loops and functions straddle
/// page boundaries constantly; a page-clamped region turns every crossing into
/// a host dispatch (measured: nbench NUMERIC SORT ran its sift loop as six
/// 2-10 instruction blocks, 5.6 insns per dispatch, because the loop sits
/// across a page boundary).
const MAX_REGION_PAGES: usize = 8;
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

// Dispatch-site profiler (diagnostic, off by default): direct-mapped
// (pc -> dispatches, retired) so a run can be attributed per guest pc —
// the metric that tells small-block/dispatch-bound kernels apart from
// genuinely slow code. One predictable branch per dispatch when off.
const DPROF_N: usize = 8192;
static mut DPROF_PC: [u64; DPROF_N] = [0; DPROF_N];
static mut DPROF_CNT: [u64; DPROF_N] = [0; DPROF_N];
static mut DPROF_RET: [u64; DPROF_N] = [0; DPROF_N];
static mut DPROF_ON: bool = false;

#[no_mangle]
pub extern "C" fn dprof_set(on: u32) {
    unsafe {
        DPROF_ON = on != 0;
        if on != 0 {
            DPROF_PC = [0; DPROF_N];
            DPROF_CNT = [0; DPROF_N];
            DPROF_RET = [0; DPROF_N];
        }
    }
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

#[allow(static_mut_refs)]
fn ihist_slot(insn: u32) -> usize {
    let key = if insn & 3 != 3 {
        insn & 0xffff
    } else {
        insn & 0xfff0_707f
    };
    unsafe {
        let h = ((key ^ (key >> 13)).wrapping_mul(0x9e37_79b9) >> 18) as usize & (IHIST_N - 1);
        if IHIST_KEY[h] != key && IHIST_CNT[h] != 0 {
            return usize::MAX;
        }
        h
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
        // stay honest (ISSUES.md P2). (Wasm function-table entries are not
        // reclaimable, but they become unreachable.)
        if on == 0 {
            if let Some(j) = SYS_JIT.as_mut() {
                j.clear();
            }
            if let Some(j) = USER_JIT.as_mut() {
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
/// Retired instructions that must pass between two superblock compiles.
/// Building one costs a page-wide leader analysis, a register scan, wasm
/// emission and — on the JS side — a module compile and instantiation, which
/// together run into milliseconds. Amortizing them against executed work keeps
/// the cost a fixed small fraction of runtime instead of a fixed number of
/// pages: a long-running kernel gets thousands of compiles, while a short cold
/// workload like `tcc -c` (340M instructions total) can't spend a third of its
/// runtime compiling page functions it will barely execute.
const SB_COMPILE_SPACING: u64 = 16_000_000;
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
            retired_addr: sp + 264,
            f_base: 0,
            fcsr_addr: 0,
            fuel_addr: 0,
            mstatus_addr: 0,
            copystat_addr: 0,
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
                    retired_addr: retired_addr(),
                    f_base: m.cpu.f.as_ptr() as u32,
                    fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
                    fuel_addr: fuel_addr(),
                    mstatus_addr: 0, // user mode: no privileged FP state
                    copystat_addr: 0,
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
                            idx,
                            n: blk.n_insns,
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

/// Give the next-booted machine a virtio-net NIC. Frames the guest sends arrive
/// via the `host_net_send` import; feed inbound frames back with
/// `sys_net_input`. The page supplies the transport (a WebSocket to a relay) —
/// the emulator only moves layer-2 frames.
#[no_mangle]
pub extern "C" fn sys_net_enable(on: u32) {
    unsafe { SYS_NET_ON = on != 0 }
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
        let fs = if SYS_FS_TAR.is_empty() {
            None
        } else {
            let tag = String::from_utf8_lossy(&SYS_FS_TAG).into_owned();
            let tag = if tag.is_empty() { "host".into() } else { tag };
            let mut mem = rv64_system::p9fs::MemFs::new();
            mem.load_tar(&core::mem::take(&mut SYS_FS_TAR));
            Some(rv64_system::p9::Server::new(tag, Box::new(mem)))
        };
        let net = SYS_NET_ON.then(|| {
            <[u8; 6]>::try_from(SYS_NET_MAC.as_slice())
                .unwrap_or(rv64_system::virtio::DEFAULT_MAC)
        });
        let m = rv64_system::Machine::new(
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
        SYS_BIOS = Vec::new();
        SYS_KERNEL = Vec::new();
        SYS = Some(m);
        // A new machine means every compiled block and stat is stale — a
        // second boot in the same wasm instance must never execute code
        // generated from the previous guest (ISSUES.md P2 cache lifecycle).
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

/// Run a slice with JIT tier-up; streams console output through
/// host_write(1, ...). Returns 1 if the guest powered off, else 0.
///
/// Full-system blocks are keyed by virtual pc. Correctness of the va→pa
/// code mapping is guarded cheaply: the whole cache is dropped when
/// `cpu.jit_flush_gen` changes (bumped only on satp write / SFENCE.VMA —
/// the actual remap events), so no per-dispatch pa re-verification is
/// needed. Self-modifying code and recycled pages are caught by per-page
/// store tracking (SystemBus.jit_dirty_pages). The hot path is a
/// direct-mapped dispatch array (one read + compare), not a HashMap.

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
        retired_addr: retired_addr(),
        f_base: m.cpu.f.as_ptr() as u32,
        fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
        fuel_addr: fuel_addr(),
        mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
        copystat_addr: copystat_addr(),
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
    let mut lay = jit_layout(m);
    lay.dispatch_base = jit.dispatch.as_ptr() as u32;
    lay.dispatch_mask = (DISPATCH_SIZE - 1) as u32;
    lay.map_gen_addr = m.cpu.jit_map_gen_ptr() as u32;
    let mut issued = false;
    let n_entries = jit.page_entries.get(&(aspace, vpage)).map_or(0, |e| e.len());
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
                                && ((pa & !0xfff) - rv64_system::RAM_BASE) as usize + 0x1000
                                    <= m.bus.ram.len()
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
                        let mut probe_add =
                            |m: &mut rv64_system::Machine,
                             pages: &mut Vec<(u64, u64)>,
                             va: u64| {
                                if pages.len() >= MAX_REGION_PAGES
                                    || pages.iter().any(|&(v, _)| v == va)
                                {
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
                        if vpage >= 0x1000
                            && seeds.iter().any(|&e| (e & 0xfff) < EDGE)
                        {
                            probe_add(m, &mut pages, vpage - 0x1000);
                        }
                        // Breadth-first over call targets; each accepted page's
                        // own calls extend the frontier.
                        let mut scanned = 0usize;
                        while scanned < pages.len() && pages.len() < MAX_REGION_PAGES {
                            let (va, pp) = pages[scanned];
                            scanned += 1;
                            let o = (pp - rv64_system::RAM_BASE) as usize;
                            let targets = rv64_jit::page_call_targets(
                                &m.bus.ram[o..o + 0x1000],
                                va,
                            );
                            for t in targets {
                                if hot(jit, t & !0xfff) {
                                    probe_add(m, &mut pages, t & !0xfff);
                                }
                            }
                        }
                        // Ascending order keeps virtually contiguous pages
                        // adjacent in the concat, which is what lets bodies
                        // flow across their shared boundary.
                        pages.sort_unstable_by_key(|&(va, _)| va);
                        let mut code = Vec::with_capacity(pages.len() * 0x1000);
                        for &(_, pp) in &pages {
                            let o = (pp - rv64_system::RAM_BASE) as usize;
                            code.extend_from_slice(&m.bus.ram[o..o + 0x1000]);
                        }
                        let vas: Vec<u64> = pages.iter().map(|&(va, _)| va).collect();
                        // Leaders per page (each page's OWN hot pcs seed it),
                        // dropping unemittable pcs and loop headers — those
                        // keep their tight individual loop-region blocks.
                        let mut entries: Vec<u64> = Vec::new();
                        for (k, &(va, _)) in pages.iter().enumerate() {
                            if entries.len() >= MAX_LEADERS {
                                break;
                            }
                            let slice = &code[k * 0x1000..(k + 1) * 0x1000];
                            let pseeds: Vec<u64> = jit
                                .page_entries
                                .get(&(aspace, va))
                                .map(|v| v.clone())
                                .unwrap_or_default();
                            let use_seeds =
                                if va == vpage { &seeds } else { &pseeds };
                            if use_seeds.is_empty() {
                                continue;
                            }
                            let (mut l, back) = rv64_jit::discover_page_leaders_ext(
                                slice,
                                va,
                                va,
                                0x1000,
                                use_seeds,
                                MAX_LEADERS - entries.len(),
                            );
                            l.retain(|&e| {
                                rv64_jit::emittable_at(slice, va, e, lay)
                                    && (!back.contains(&e)
                                        || !rv64_jit::is_loop_at(slice, va, e, lay))
                            });
                            entries.extend(l);
                        }
                        let sb = rv64_jit::translate_superblock_sparse(
                            &code, &vas, &entries, lay,
                        );
                        if sb.is_none() {
                            unsafe { SB_XLATE_FAIL += 1 };
                        }
                        if let Some(blk) = sb {
                            // Large module: compile it ASYNC on V8's
                            // background threads (ISSUES.md/perf: the sync
                            // Module build of a page function stalls the
                            // guest for ms — the cold-compile cost that
                            // kept superblocks gated). Execution continues
                            // on individual blocks; sys_sb_ready repoints
                            // the entries once the function is in the
                            // table, after re-validating page identity.
                            unsafe { JIT_OUT = blk.wasm };
                            for &(_, pp) in &pages {
                                m.bus.jit_mark_page(pp);
                            }
                            // Only the page that reached the threshold is
                            // marked done: a neighbour pulled into this
                            // region still gets to build its own region for
                            // the code this one didn't reach.
                            // Every page the region covers is superblocked,
                            // and this build answers the misses recorded so
                            // far on each — only misses AFTER it argue for a
                            // rebuild.
                            for &(pva, _) in &pages {
                                jit.superblocked.insert((aspace, pva));
                                jit.sb_missed.remove(&(aspace, pva));
                            }
                            // Only a rebuild that covered nothing new counts
                            // against the allowance: a page whose hot set is
                            // still growing must be able to keep up, or code
                            // that gets hot late is stranded on individual
                            // blocks forever. The recorded instruction count
                            // starts this page's rebuild cooldown.
                            let prev = jit
                                .sb_gen
                                .get(&(aspace, vpage))
                                .map_or(0, |&(e, _, _)| e);
                            jit.sb_gen.insert(
                                (aspace, vpage),
                                (
                                    n_entries,
                                    sb_compiles + u32::from(n_entries <= prev),
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
                                    pages,
                                    entries,
                                });
                                host_jit_register_async(ticket);
                                SB_ISSUED += 1;
                                SB_LAST_ICOUNT = m.cpu.insn_count;
                                issued = true;
                            }
                            // The caller still gives this pc an individual
                            // block right now; the superblock repoints its
                            // entries when the module arrives.
                        }
    issued
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_run(max_insns: u64) -> i32 {
    let m = unsafe { SYS.as_mut().expect("call sys_boot() first") };
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
            let due = unsafe {
                ic.wrapping_sub(WALL_LAST_ICOUNT) >= 16384 || WALL_IDLE_ITERS >= 64
            };
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
            unsafe { DIRTY_EVENTS += dirty.len() as u64 };
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
                jit.page_entries.retain(|&(_, vp), _| !dirty_vpages.contains(&vp));
                jit.superblocked.retain(|&(_, vp)| !dirty_vpages.contains(&vp));
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
        while chained < JIT_CHAIN_CAP && retired_sum < round_budget {
            unsafe { FUEL_CELL = round_budget - retired_sum };
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
                        // Multi-page region: every page it was compiled over
                        // must still map where it did (n == 0 marks a
                        // superblock entry; individual blocks retire > 0).
                        let region = if b.n == 0 {
                            jit.regions.get(&b.idx).cloned()
                        } else {
                            None
                        };
                        let self_ok = matches!(
                            m.cpu.jit_probe_fetch(&mut m.bus, pc), Some(pa) if pa == b.pa
                        );
                        let region_ok = region.map_or(true, |pgs| {
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
                        jit.dispatch[slot] = DispatchLine { pc, idx: b.idx, gen: map_gen };
                        b.idx
                    }
                    _ => break, // uncompiled or blacklisted
                }
            };
            if idx < 0 {
                break; // blacklisted (pa-verified for the current mapping)
            }
            call_block(idx, mptr);
            // Sys blocks with inline memory ops may bail mid-block; read the
            // count they actually retired (pc is set by the block either way).
            let retired = unsafe { RETIRED_CELL };
            if unsafe { DPROF_ON } {
                dprof_hit(pc, retired);
            }
            retired_sum += retired;
            chained += 1;
            // A block that retired nothing bailed on its very first instruction
            // (TLB miss / MMIO / FP fast-path). It makes no progress, so stop
            // chaining and let the interpreter handle that instruction — never
            // spin re-calling it.
            if retired == 0 {
                unsafe { ZERO_RETIRE += 1 };
                if unsafe { DPROF_ON } {
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
            // Spend the superblock compile budget on the oldest deferred page
            // that still resolves in the CURRENT address space.
            if m.cpu.insn_count >= unsafe { SB_LAST_ICOUNT }.wrapping_add(SB_COMPILE_SPACING)
                && !jit.sb_queue.is_empty()
            {
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
                            retired_addr: retired_addr(),
                            f_base: m.cpu.f.as_ptr() as u32,
                            fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
                            fuel_addr: fuel_addr(),
                            mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
                            copystat_addr: copystat_addr(),
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
                        let sb_spaced = m.cpu.insn_count
                            >= unsafe { SB_LAST_ICOUNT }.wrapping_add(SB_COMPILE_SPACING);
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
                            let missed =
                                jit.sb_missed.get(&(aspace, vpage)).copied().unwrap_or(0);
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
                        if !is_loop && jit.superblocked.contains(&(aspace, vpage)) {
                            *jit.sb_missed.entry((aspace, vpage)).or_insert(0) += 1;
                            unsafe { SB_INDIV += 1 };
                        }
                        // Individual block (loop or pre-threshold non-loop).
                        // Deliberately NOT deferred while a page function is on
                        // its way: making hot pcs wait for one stalls code
                        // behind an async compile that may never land — that
                        // was measured as 138M retries and a 10x slowdown once
                        // pending builds backed up.
                        let blk = rv64_jit::translate_block(&m.bus.ram[off..end], pc, pc, lay);
                        let entry = blk.and_then(|blk| {
                            unsafe { JIT_OUT = blk.wasm };
                            let idx = unsafe { host_jit_register() };
                            if idx < 0 {
                                return None;
                            }
                            unsafe { JIT_TABLE_ENTRIES += 1 };
                            m.bus.jit_mark_page(pa);
                            m.cpu.clear_store_jtlb(); // this page may now hold code
                            Some(JitBlock { idx, n: blk.n_insns, pa })
                        });
                        match entry {
                            Some(b) => {
                                if jit.cache.insert(pc, Some(b)).is_none() {
                                    jit.page_blocks
                                        .entry((b.pa - rv64_system::RAM_BASE) >> 12)
                                        .or_default()
                                        .push(pc);
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
                                let jb = JitBlock { idx: -1, n: 0, pa };
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
            unsafe { SLICE_CALLS += 1; SLICE_INSNS += ran; }
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
        // surface output comparably; see ISSUES.md P0).
        let out = m.console_output();
        if !out.is_empty() {
            unsafe { host_write(1, out.as_ptr(), out.len()) }
        }
        for frame in m.net_take_output() {
            unsafe { host_net_send(frame.as_ptr(), frame.len()) }
        }
    }

    let out = m.console_output();
    if !out.is_empty() {
        unsafe { host_write(1, out.as_ptr(), out.len()) }
    }
        for frame in m.net_take_output() {
            unsafe { host_net_send(frame.as_ptr(), frame.len()) }
        }
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
        for &e in &p.entries {
            // Sparse regions: find the entry's page by lookup (pages are in
            // dispatch order, not address order).
            let Some(pi) = p.pages.iter().position(|&(va, _)| va == e & !0xfff)
            else {
                continue;
            };
            let epa = p.pages[pi].1 + (e & 0xfff);
            let jb = JitBlock { idx, n: 0, pa: epa };
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
        let Some(m) = SYS.as_mut() else { return u64::MAX };
        let Some(jit) = SYS_JIT.as_ref() else { return u64::MAX };
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
            retired_addr: retired_addr(),
            f_base: m.cpu.f.as_ptr() as u32,
            fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
            fuel_addr: fuel_addr(),
            mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
            copystat_addr: copystat_addr(),
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
            let (rm, wm, fr, fw) = rv64_jit::scan_regs_super_pub(code, vpage, vpage + 0x1000, &keep, &lay);
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
        let Some(m) = SYS.as_mut() else { return u64::MAX };
        let Some(jit) = SYS_JIT.as_ref() else { return u64::MAX };
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
            retired_addr: retired_addr(),
            f_base: m.cpu.f.as_ptr() as u32,
            fcsr_addr: &m.cpu.fcsr as *const u32 as u32,
            fuel_addr: fuel_addr(),
            mstatus_addr: m.cpu.jit_mstatus_ptr() as u32,
            copystat_addr: copystat_addr(),
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
        let Some(jit) = SYS_JIT.as_ref() else { return 0 };
        let Some(mm) = SYS.as_ref() else { return 0 };
        let mut v = 0u64;
        let aspace = mm.cpu.sys.as_ref().map_or(0, |c| c.satp);
        if jit.superblocked.contains(&(aspace, vpage)) {
            v |= 1;
        }
        if PENDING_SB.iter().any(|p| p.pages.iter().any(|&(va, _)| va == vpage)) {
            v |= 2;
        }
        v |= (jit.page_entries.get(&(aspace, vpage)).map_or(0, |e| e.len()) as u64) << 8;
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

/// Enable direct block-to-block tail-call chaining (host feature-detects
/// wasm tail-call support first).
#[no_mangle]
pub extern "C" fn jit_set_tailcall(on: u32) {
    rv64_jit::set_chain(on != 0);
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
            Some(m) => m
                .cpu
                .jit_fill_tlb(&mut m.bus, va, store != 0)
                .unwrap_or(-1),
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
    unsafe { JIT_OUT.len() as u32 }
}
