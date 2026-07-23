// Compute-bound guest: integer hot loop for JIT speedup measurement.
#![no_std]
#![no_main]
use core::arch::asm;

fn sys(nr: u64, a0: u64, a1: u64, a2: u64) -> i64 {
    let ret: i64;
    unsafe { asm!("ecall", in("a7") nr, inlateout("a0") a0 => ret, in("a1") a1, in("a2") a2) }
    ret
}

fn set_frm(frm: u64) {
    unsafe { asm!("csrw frm, {0}", in(reg) frm) }
}

#[no_mangle]
extern "C" fn rust_main(sp: *const u64) -> ! {
    // argv[1] == "soft" forces frm=RDN: the interpreter's native-FP fast
    // path requires RNE, so this benchmarks the softfp fallback instead.
    let argc = unsafe { *sp } as usize;
    let mut soft = false;
    if argc > 1 {
        let p = unsafe { *sp.add(2) } as *const u8;
        soft = unsafe { *p == b's' };
    }
    set_frm(if soft { 2 } else { 0 }); // RDN vs RNE

    // FP phase: mul/add/div mix on normal values
    let mut y: f64 = 1.5;
    let mut facc: f64 = 0.0;
    let mut i: u64 = 0;
    while i < 5_000_000 {
        y = y * 1.0000001 + 0.0625;
        facc += y / 3.0;
        if y > 1e300 {
            y = 1.5;
        }
        i += 1;
    }
    let fb = facc.to_bits();
    let mut fbuf = [0u8; 17];
    for j in 0..16 {
        let dd = ((fb >> (60 - j * 4)) & 0xf) as u8;
        fbuf[j] = if dd < 10 { b'0' + dd } else { b'a' + dd - 10 };
    }
    fbuf[16] = b'\n';
    sys(64, 1, fbuf.as_ptr() as u64, 17);

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
