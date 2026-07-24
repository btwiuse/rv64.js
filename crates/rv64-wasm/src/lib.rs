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
    page_entries: std::collections::HashMap<u64, Vec<u64>>,
    /// Cheap direct-mapped hot counter for the interpreter-fallback path: an
    /// interpreted stretch's interior blocks never reach the fast-path hot map
    /// (run_slice_until never returns to the fast path inside a stretch), so
    /// they'd stay interpreted forever (~half of fib's time). Bumping a real
    /// HashMap per interpreted instruction taxes cold boot, so count in a u16
    /// array here and only touch `hot` when a slot actually gets hot.
    interp_hot: Vec<u16>,
    /// Physical page -> pcs of cache entries whose code lives there (blocks
    /// AND pa-stamped blacklist sentinels). Lets dirty-page invalidation drop
    /// exactly the affected entries instead of scanning the whole cache per
    /// page — the cache persists across context switches now, so it's large.
    page_blocks: std::collections::HashMap<u64, Vec<u64>>,
    /// Virtual pages already compiled as a superblock — compile ONCE per page
    /// (with whatever entries were hot then); later hot pcs in the page get
    /// individual blocks. Recompiling the page's big br_table function on every
    /// new entry was a 2x regression on short workloads (the recompile storm).
    superblocked: std::collections::HashSet<u64>,
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
            page_blocks: Default::default(),
            superblocked: Default::default(),
        }
    }
    fn clear(&mut self) {
        self.cache.clear();
        self.hot.clear();
        self.page_entries.clear();
        self.superblocked.clear();
        for h in self.interp_hot.iter_mut() {
            *h = 0;
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

#[allow(static_mut_refs)]
fn retired_addr() -> u32 {
    unsafe { &RETIRED_CELL as *const u64 as u32 }
}

// Perf instrumentation: guest instructions retired inside JIT blocks vs
// total, and dispatch counts (block calls). Exposed via jit_stat().
static mut JIT_RETIRED: u64 = 0;
static mut SLICE_CALLS: u64 = 0;
static mut SLICE_INSNS: u64 = 0;
static mut JIT_DISPATCHES: u64 = 0;


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
    unsafe { JIT_THRESHOLD = if on == 0 { u32::MAX } else { JIT_ON_THRESHOLD } }
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
const SUPERBLOCK_THRESHOLD: usize = 6;

/// Call a compiled block. The state pointer parameter deliberately escapes
/// the emulator state into the opaque call so the compiler reloads CPU
/// fields afterwards instead of caching them in locals.
#[inline]
fn call_block(idx: i32, state_ptr: *mut u8) {
    unsafe {
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
        while chained < JIT_CHAIN_CAP {
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
        if !jit.cache.contains_key(&pc) {
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
            },
        );
        SYS_BIOS = Vec::new();
        SYS_KERNEL = Vec::new();
        SYS = Some(m);
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
            jit.page_entries.clear();
            jit.superblocked.clear();
        }
        // Per-page invalidation: drop only blocks whose physical code page
        // was written (self-modifying code / recycled pages), and clear only
        // their dispatch lines (a full dispatch memset is megabytes per event).
        if !m.bus.jit_dirty_pages.is_empty() {
            let dirty = m.bus.jit_take_dirty();
            for &ppage in &dirty {
                if let Some(pcs) = jit.page_blocks.remove(&ppage) {
                    for pc in pcs {
                        jit.cache.remove(&pc);
                        let slot = JitState::dslot(pc);
                        if jit.dispatch[slot].pc == pc {
                            jit.dispatch[slot].pc = NO_PC;
                        }
                    }
                }
                m.bus.jit_unmark_page(ppage);
            }
            jit.page_entries.clear(); // re-discover superblock entries
            jit.superblocked.clear();
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
        while chained < JIT_CHAIN_CAP {
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
                        match m.cpu.jit_probe_fetch(&mut m.bus, pc) {
                            Some(pa) if pa == b.pa => {}
                            _ => {
                                jit.cache.remove(&pc);
                                jit.dispatch[slot].pc = NO_PC;
                                break;
                            }
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
            retired_sum += retired;
            chained += 1;
            // A block that retired nothing bailed on its very first instruction
            // (TLB miss / MMIO / FP fast-path). It makes no progress, so stop
            // chaining and let the interpreter handle that instruction — never
            // spin re-calling it.
            if retired == 0 {
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
        if chained == JIT_CHAIN_CAP {
            m.sync_devices();
            m.cpu.check_interrupts(&mut m.bus);
            continue;
        }

        // --- hot counting + compile (from physical code bytes) ---
        let pc = m.cpu.pc;
        if !jit.cache.contains_key(&pc) {
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
                                let e = jit.page_entries.entry(vpage).or_default();
                                if let Err(i) = e.binary_search(&pc) {
                                    e.insert(i, pc);
                                }
                                e.len()
                            };
                            (il, ne)
                        } else {
                            (false, 0)
                        };

                        if !is_loop
                            && n_entries >= SUPERBLOCK_THRESHOLD
                            && !jit.superblocked.contains(&vpage)
                            && pa_page_off + 0x1000 <= m.bus.ram.len()
                        {
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
                            let seeds = jit.page_entries[&vpage].clone();
                            let code = &m.bus.ram[pa_page_off..pa_page_off + 0x1000];
                            let mut entries = rv64_jit::discover_page_leaders(
                                code, vpage, vpage, 0x1000, &seeds, 384,
                            );
                            entries.retain(|&e| !rv64_jit::is_loop_at(code, vpage, e, lay));
                            let sb = rv64_jit::translate_superblock(
                                code, vpage, vpage, 0x1000, &entries, lay,
                            );
                            if let Some(blk) = sb {
                                unsafe { JIT_OUT = blk.wasm };
                                let idx = unsafe { host_jit_register() };
                                if idx >= 0 {
                                    m.bus.jit_mark_page(pa);
                                    m.cpu.clear_store_jtlb(); // this page may now hold code
                                    jit.superblocked.insert(vpage); // compile once
                                    for &e in &entries {
                                        let epa = pa_page + (e - vpage);
                                        let jb = JitBlock { idx, n: 0, pa: epa };
                                        if jit.cache.insert(e, Some(jb)).is_none() {
                                            jit.page_blocks
                                                .entry((epa - rv64_system::RAM_BASE) >> 12)
                                                .or_default()
                                                .push(e);
                                        }
                                        jit.dispatch[JitState::dslot(e)] = DispatchLine {
                                            pc: e,
                                            idx,
                                            gen: m.cpu.map_gen as u32,
                                        };
                                    }
                                    continue;
                                }
                            }
                        }

                        // Individual block (loop or pre-threshold non-loop).
                        let blk = rv64_jit::translate_block(&m.bus.ram[off..end], pc, pc, lay);
                        let entry = blk.and_then(|blk| {
                            unsafe { JIT_OUT = blk.wasm };
                            let idx = unsafe { host_jit_register() };
                            if idx < 0 {
                                return None;
                            }
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
            let ran = m.run_slice_until(remaining.min(SYS_WARM_SLICE), |pc| {
                if jit.dispatch[JitState::dslot(pc)].pc == pc {
                    return true;
                }
                let slot = JitState::dslot(pc);
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
            }
            remaining = remaining.saturating_sub(ran.max(1));
        }
    }

    let out = m.console_output();
    if !out.is_empty() {
        unsafe { host_write(1, out.as_ptr(), out.len()) }
    }
    m.power_off as i32
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

#[no_mangle]
#[allow(static_mut_refs)]
/// Current guest pc (diagnostic: host-side pc sampling for guest profiling).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn sys_pc() -> u64 {
    unsafe { SYS.as_ref().map_or(0, |m| m.cpu.pc) }
}

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
