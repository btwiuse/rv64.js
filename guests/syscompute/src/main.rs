//! System-mode JIT benchmark payload (static riscv64 Linux binary).
//!
//! Two selectable workloads to isolate whether *memory density* is what
//! decides if the system-mode JIT beats the interpreter:
//!   argv[1] == "alu"  : register-only xorshift accumulate (no memory) —
//!                       the JIT's best case (pure decode-elimination win).
//!   argv[1] == "mix"  : repeated transform over a 256 KiB array (loads +
//!                       stores + ALU + branches) — a realistic
//!                       data-processing mix, neither md5-extreme nor
//!                       register-only.
//! Each runs a fixed amount of work, prints a hex checksum, and exits.
//! Deterministic, so JIT-on and JIT-off outputs must match bit-for-bit.

#![no_std]
#![no_main]

use core::arch::asm;

fn sys(nr: u64, a0: u64, a1: u64, a2: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!("ecall", in("a7") nr, inlateout("a0") a0 => ret, in("a1") a1, in("a2") a2);
    }
    ret
}

fn write_hex(v: u64) {
    let mut buf = [0u8; 17];
    for j in 0..16 {
        let d = ((v >> (60 - j * 4)) & 0xf) as u8;
        buf[j] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
    }
    buf[16] = b'\n';
    sys(64, 1, buf.as_ptr() as u64, 17);
}

const ARR_WORDS: usize = 32 * 1024; // 256 KiB
static mut ARR: [u64; ARR_WORDS] = [0; ARR_WORDS];

fn alu_workload() -> u64 {
    // Pure register work: no loads/stores in the hot loop.
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut acc: u64 = 0;
    let mut i: u64 = 0;
    while i < 60_000_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        acc = acc.wrapping_add(x);
        i += 1;
    }
    acc
}

fn mix_workload() -> u64 {
    // Realistic memory+ALU+branch mix: many passes transforming an array,
    // each element mixed with a neighbor (load, load, alu, store).
    unsafe {
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        for i in 0..ARR_WORDS {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            ARR[i] = seed;
        }
        for _pass in 0..400 {
            let mut i = 1;
            while i < ARR_WORDS {
                let a = ARR[i];
                let b = ARR[i - 1];
                // a few ALU ops per element, then store back
                let v = (a.wrapping_mul(6364136223846793005).wrapping_add(b)) ^ (b >> 11);
                ARR[i] = v.rotate_left(7).wrapping_add(a);
                i += 1;
            }
        }
        let mut sum: u64 = 0;
        for i in 0..ARR_WORDS {
            sum = sum.wrapping_add(ARR[i]).rotate_left(1);
        }
        sum
    }
}

#[no_mangle]
extern "C" fn rust_main(sp: *const u64) -> ! {
    let argc = unsafe { *sp } as usize;
    let mut mix = false;
    if argc > 1 {
        let p = unsafe { *sp.add(2) } as *const u8;
        // "mix" starts with 'm'
        mix = unsafe { core::ptr::read_volatile(p) } == b'm';
    }
    let r = if mix { mix_workload() } else { alu_workload() };
    write_hex(r);
    sys(93, 0, 0, 0);
    loop {}
}

core::arch::global_asm!(
    ".global _start",
    "_start:",
    "mv a0, sp",
    "andi sp, sp, -16",
    "tail rust_main"
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sys(93, 101, 0, 0);
    loop {}
}
