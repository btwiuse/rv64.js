//! Minimal riscv64 Linux guest: raw syscalls, no libc, no std.
//! Prints argv and a message, exits 7. Exercises write/exit + stack setup.

#![no_std]
#![no_main]

use core::arch::asm;

fn sys(nr: u64, a0: u64, a1: u64, a2: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "ecall",
            in("a7") nr,
            inlateout("a0") a0 => ret,
            in("a1") a1,
            in("a2") a2,
        );
    }
    ret
}

fn write(fd: u64, buf: &[u8]) {
    sys(64, fd, buf.as_ptr() as u64, buf.len() as u64);
}

fn strlen(mut p: *const u8) -> usize {
    let mut n = 0;
    unsafe {
        // read_volatile keeps LLVM's loop-idiom pass from lowering this
        // into a call to libc strlen (which doesn't exist here).
        while core::ptr::read_volatile(p) != 0 {
            n += 1;
            p = p.add(1);
        }
    }
    n
}

#[no_mangle]
extern "C" fn rust_main(sp: *const u64) -> ! {
    write(1, b"hello from rv64 guest!\n");

    // Walk argc/argv straight off the initial stack.
    unsafe {
        let argc = *sp as usize;
        write(1, b"argc: ");
        let d = [b'0' + (argc % 10) as u8, b'\n'];
        write(1, &d);
        for i in 0..argc {
            let p = *sp.add(1 + i) as *const u8;
            write(1, b"argv: ");
            write(1, core::slice::from_raw_parts(p, strlen(p)));
            write(1, b"\n");
        }
    }

    sys(93, 7, 0, 0); // exit(7)
    loop {}
}

core::arch::global_asm!(
    ".global _start",
    "_start:",
    "mv a0, sp",
    "andi sp, sp, -16",
    "tail rust_main",
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sys(93, 101, 0, 0);
    loop {}
}
