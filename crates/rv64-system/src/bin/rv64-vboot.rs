//! rv64-vboot: boot a modern riscv64 Linux (OpenSBI + kernel + optional
//! rootfs/initrd) on the virt-class machine. Console on stdio (raw tty;
//! Ctrl-A x to quit).
//!
//! Usage:
//!   rv64-vboot <opensbi.bin> <kernel-Image> [--initrd FILE] [--disk FILE]
//!              [--ram GB] [-- <cmdline...>]

use rv64_system::virt::{VirtImages, VirtMachine};
use std::io::{Read, Write};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut cmdline = "console=ttyS0 earlycon=uart8250,mmio,0x10000000".to_string();
    if let Some(pos) = args.iter().position(|a| a == "--") {
        cmdline = args.split_off(pos + 1).join(" ");
        args.pop();
    }
    let mut initrd_path = None;
    let mut disk_path = None;
    let mut ram_gb = 2.0f64;
    let mut positional = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--initrd" => initrd_path = it.next(),
            "--disk" => disk_path = it.next(),
            "--ram" => ram_gb = it.next().and_then(|v| v.parse().ok()).unwrap_or(2.0),
            _ => positional.push(a),
        }
    }
    if positional.len() < 2 {
        eprintln!("usage: rv64-vboot <opensbi.bin> <kernel> [--initrd F] [--disk F] [--ram GB] [-- cmdline]");
        std::process::exit(2);
    }
    let opensbi = std::fs::read(&positional[0]).expect("read opensbi");
    let kernel = std::fs::read(&positional[1]).expect("read kernel");
    let initrd = initrd_path.map(|p| std::fs::read(p).expect("read initrd"));
    let disk = disk_path.map(|p| std::fs::read(p).expect("read disk"));

    let ram_bytes = (ram_gb * (1u64 << 30) as f64) as u64;
    eprintln!(
        "[vboot] opensbi={}B kernel={}B ram={:.1}GB initrd={} disk={} cmdline='{}'",
        opensbi.len(), kernel.len(), ram_gb,
        initrd.as_ref().map_or(0, |v| v.len()),
        disk.as_ref().map_or(0, |v| v.len()),
        cmdline,
    );

    let mut m = VirtMachine::new(
        ram_bytes,
        VirtImages { opensbi: &opensbi, kernel: &kernel, cmdline: &cmdline, initrd: initrd.as_deref(), disk },
    );

    let _raw = RawTerm::enable();
    let mut stdin = { libcish::set_nonblocking(0); std::io::stdin() };
    let mut ctrl_a = false;
    let t0 = std::time::Instant::now();
    let max_insns: Option<u64> = std::env::var("VBOOT_MAX_INSNS").ok().and_then(|v| v.parse().ok());

    loop {
        m.run_slice(4_000_000);
        let out = m.console_output();
        if !out.is_empty() {
            std::io::stdout().write_all(&out).unwrap();
            std::io::stdout().flush().unwrap();
        }
        if m.power_off {
            eprintln!("\r\n[vboot] powered off");
            break;
        }
        let mut buf = [0u8; 256];
        if let Ok(n) = stdin.read(&mut buf) {
            if n > 0 {
                let mut fwd = Vec::new();
                for &b in &buf[..n] {
                    if ctrl_a {
                        ctrl_a = false;
                        if b == b'x' {
                            eprintln!("\r\n[vboot] quit");
                            return;
                        }
                        fwd.push(1);
                        fwd.push(b);
                    } else if b == 1 {
                        ctrl_a = true;
                    } else {
                        fwd.push(b);
                    }
                }
                if !fwd.is_empty() {
                    m.console_input(&fwd);
                }
            }
        }
        if let Some(mx) = max_insns {
            if m.cpu.insn_count > mx {
                eprintln!("\r\n[vboot] insn budget reached: {} insns in {:.1}s, pc={:#x}",
                    m.cpu.insn_count, t0.elapsed().as_secs_f64(), m.cpu.pc);
                break;
            }
        }
    }
}

struct RawTerm {
    orig: libcish::Termios,
}
impl RawTerm {
    fn enable() -> Option<RawTerm> {
        libcish::raw_mode().map(|orig| RawTerm { orig })
    }
}
impl Drop for RawTerm {
    fn drop(&mut self) {
        libcish::restore(&self.orig);
    }
}

mod libcish {
    #[repr(C)]
    #[derive(Clone)]
    pub struct Termios {
        pub c_iflag: u32,
        pub c_oflag: u32,
        pub c_cflag: u32,
        pub c_lflag: u32,
        pub c_line: u8,
        pub c_cc: [u8; 32],
        pub c_ispeed: u32,
        pub c_ospeed: u32,
    }
    extern "C" {
        fn tcgetattr(fd: i32, t: *mut Termios) -> i32;
        fn tcsetattr(fd: i32, act: i32, t: *const Termios) -> i32;
        fn fcntl(fd: i32, cmd: i32, arg: i32) -> i32;
        fn isatty(fd: i32) -> i32;
    }
    const ICANON: u32 = 0o000002;
    const ECHO: u32 = 0o000010;
    const ISIG: u32 = 0o000001;
    pub fn raw_mode() -> Option<Termios> {
        unsafe {
            if isatty(0) == 0 {
                return None;
            }
            let mut t = core::mem::zeroed::<Termios>();
            if tcgetattr(0, &mut t) != 0 {
                return None;
            }
            let orig = t.clone();
            t.c_lflag &= !(ICANON | ECHO | ISIG);
            t.c_cc[6] = 0;
            t.c_cc[5] = 0;
            tcsetattr(0, 0, &t);
            Some(orig)
        }
    }
    pub fn restore(t: &Termios) {
        unsafe {
            tcsetattr(0, 0, t);
        }
    }
    pub fn set_nonblocking(fd: i32) {
        unsafe {
            let fl = fcntl(fd, 3, 0);
            fcntl(fd, 4, fl | 0o4000);
        }
    }
}
