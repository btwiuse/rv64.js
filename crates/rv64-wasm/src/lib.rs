//! wasm export surface for rv64.js.
//!
//! v86-style: a plain `extern "C"` ABI over wasm linear memory — no
//! wasm-bindgen, no JS glue generation. `web/rv64.js` instantiates the
//! module and talks to these exports directly. Guest RAM lives inside the
//! wasm linear memory; JS reaches it via `mem_ptr()`/`mem_size()` over the
//! exported memory, so loading a guest program is a single typed-array copy.
//!
//! Single-instance for now (one emulator per wasm instance, which is also
//! the v86 model); multiple VMs = multiple instantiations of the module.

use rv64_core::{Cpu, FlatMemory, StopReason};

struct Emu {
    cpu: Cpu,
    ram: Vec<u8>,
    ram_base: u64,
}

static mut EMU: Option<Emu> = None;

// Stop-reason codes returned by `run` to JS.
const STOP_BUDGET: i32 = 0;
const STOP_ECALL: i32 = 1;
const STOP_BREAK: i32 = 2;
const STOP_TRAP: i32 = 3;

#[allow(static_mut_refs)]
fn emu() -> &'static mut Emu {
    unsafe { EMU.as_mut().expect("call init() first") }
}

/// Allocate guest RAM (`size` bytes at guest address `base`) and reset the CPU.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn init(base: u64, size: u32) {
    let mut cpu = Cpu::new();
    cpu.pc = base;
    unsafe {
        EMU = Some(Emu { cpu, ram: vec![0; size as usize], ram_base: base });
    }
}

/// Pointer to guest RAM within wasm linear memory (JS: new Uint8Array(wasmMemory.buffer, ptr, size)).
#[no_mangle]
pub extern "C" fn mem_ptr() -> *mut u8 {
    emu().ram.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn mem_size() -> u32 {
    emu().ram.len() as u32
}

#[no_mangle]
pub extern "C" fn get_pc() -> u64 {
    emu().cpu.pc
}

#[no_mangle]
pub extern "C" fn set_pc(pc: u64) {
    emu().cpu.pc = pc;
}

#[no_mangle]
pub extern "C" fn get_reg(i: u32) -> u64 {
    emu().cpu.x[(i & 31) as usize]
}

#[no_mangle]
pub extern "C" fn set_reg(i: u32, val: u64) {
    if i != 0 {
        emu().cpu.x[(i & 31) as usize] = val;
    }
}

#[no_mangle]
pub extern "C" fn insn_count() -> u64 {
    emu().cpu.insn_count
}

/// Run up to `budget` instructions. Returns a STOP_* code; on STOP_TRAP the
/// cause is retrievable via `trap_cause()`.
#[no_mangle]
pub extern "C" fn run(budget: u64) -> i32 {
    let e = emu();
    let mut bus = FlatMemory::new(e.ram_base, &mut e.ram);
    match e.cpu.run(&mut bus, budget) {
        StopReason::Budget => STOP_BUDGET,
        StopReason::Ecall => STOP_ECALL,
        StopReason::Break => STOP_BREAK,
        StopReason::Trap(exc) => {
            unsafe { LAST_TRAP = exc.cause() as i32 };
            STOP_TRAP
        }
    }
}

static mut LAST_TRAP: i32 = -1;

/// mcause code of the last STOP_TRAP, or -1.
#[no_mangle]
pub extern "C" fn trap_cause() -> i32 {
    unsafe { LAST_TRAP }
}
