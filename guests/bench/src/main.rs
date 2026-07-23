// Compute-bound guest: integer hot loop for JIT speedup measurement.
#![no_std]
#![no_main]
use core::arch::asm;

fn sys(nr: u64, a0: u64, a1: u64, a2: u64) -> i64 {
    let ret: i64;
    unsafe { asm!("ecall", in("a7") nr, inlateout("a0") a0 => ret, in("a1") a1, in("a2") a2) }
    ret
}

#[no_mangle]
extern "C" fn rust_main(_sp: *const u64) -> ! {
    // xorshift + accumulate: pure ALU/branch, JIT-friendly
    let mut x: u64 = 0x2545F4914F6CDD1D;
    let mut acc: u64 = 0;
    let mut i: u64 = 0;
    while i < 20_000_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        acc = acc.wrapping_add(x);
        i += 1;
    }
    // print acc as hex
    let mut buf = [0u8; 17];
    for j in 0..16 {
        let d = ((acc >> (60 - j * 4)) & 0xf) as u8;
        buf[j] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
    }
    buf[16] = b'\n';
    sys(64, 1, buf.as_ptr() as u64, 17);
    sys(93, 0, 0, 0);
    loop {}
}

core::arch::global_asm!(".global _start", "_start:", "mv a0, sp", "andi sp, sp, -16", "tail rust_main");

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sys(93, 101, 0, 0);
    loop {}
}
