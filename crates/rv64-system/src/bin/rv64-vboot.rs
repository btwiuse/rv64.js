//! rv64-vboot: boot a modern riscv64 Linux (OpenSBI + kernel + optional
//! rootfs/initrd) on the virt-class machine. Console on stdio (raw tty;
//! Ctrl-A x to quit).
//!
//! Usage:
//!   rv64-vboot <opensbi.bin> <kernel-Image> [--initrd FILE] [--disk FILE]
//!              [--9p DIR] [--9p-tag TAG] [--net ws://HOST:PORT] [--net-mac MAC]
//!              [--ram GB] [-- <cmdline...>]
//!
//! `--9p DIR` exports a host directory over virtio-9p. Note that the stock
//! Debian/nixpkgs riscv64 kernels ship 9p as modules (`9pnet_virtio`, `9p`),
//! so the guest must load them (or use a kernel with them built in) before:
//!   mount -t 9p -o trans=virtio,version=9p2000.L host /mnt

use rv64_system::virt::{VirtImages, VirtMachine};
use rv64_system::{p9, p9fs, ws};
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
    let mut share = None;
    let mut tag = "host".to_string();
    let mut relay_url = None;
    let mut mac = rv64_system::virtio::DEFAULT_MAC;
    let mut ram_gb = 2.0f64;
    let mut positional = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--initrd" => initrd_path = it.next(),
            "--disk" => disk_path = it.next(),
            "--9p" => share = it.next(),
            "--9p-tag" => tag = it.next().unwrap_or(tag),
            "--net" => relay_url = it.next(),
            "--net-mac" => {
                mac = it
                    .next()
                    .as_deref()
                    .and_then(parse_mac)
                    .unwrap_or_else(|| { eprintln!("bad --net-mac"); std::process::exit(2) })
            }
            "--ram" => ram_gb = it.next().and_then(|v| v.parse().ok()).unwrap_or(2.0),
            _ => positional.push(a),
        }
    }
    if positional.len() < 2 {
        eprintln!("usage: rv64-vboot <opensbi.bin> <kernel> [--initrd F] [--disk F] [--9p DIR] [--9p-tag TAG] [--net ws://HOST:PORT] [--net-mac MAC] [--ram GB] [-- cmdline]");
        std::process::exit(2);
    }
    let opensbi = std::fs::read(&positional[0]).expect("read opensbi");
    let kernel = std::fs::read(&positional[1]).expect("read kernel");
    let initrd = initrd_path.map(|p| std::fs::read(p).expect("read initrd"));
    let disk = disk_path.map(|p| std::fs::read(p).expect("read disk"));
    let fs = share.map(|dir| {
        eprintln!("[vboot] 9p: exporting {dir} as tag '{tag}'");
        p9::Server::new(tag, Box::new(p9fs::HostFs::new(dir)))
    });

    // Connect the relay before booting: a NIC the guest can see but that has
    // nowhere to send is worse than no NIC at all.
    let mut relay = match &relay_url {
        Some(url) => match ws::Relay::connect(url) {
            Ok(r) => {
                eprintln!("[vboot] net: relay {url}, mac {}", fmt_mac(&mac));
                Some(r)
            }
            Err(e) => {
                eprintln!("[vboot] net: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };
    let net = relay.is_some().then_some(mac);

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
        VirtImages { opensbi: &opensbi, kernel: &kernel, cmdline: &cmdline, initrd: initrd.as_deref(), disk, fs, net },
    );

    let _raw = RawTerm::enable();
    let mut stdin = { libcish::set_nonblocking(0); std::io::stdin() };
    let mut ctrl_a = false;
    let t0 = std::time::Instant::now();
    let max_insns: Option<u64> = std::env::var("VBOOT_MAX_INSNS").ok().and_then(|v| v.parse().ok());
    // Optional heartbeat: every N wall-seconds print retired-insn count,
    // throughput, and current guest PC (privilege) — for diagnosing hangs.
    let hb_secs: Option<u64> = std::env::var("VBOOT_HEARTBEAT").ok().and_then(|v| v.parse().ok());
    let mut hb_last = std::time::Instant::now();
    let mut hb_insns = 0u64;

    loop {
        m.run_slice(2_000_000);
        if let Some(hb) = hb_secs {
            if hb_last.elapsed().as_secs() >= hb {
                let now_insns = m.cpu.insn_count;
                let dt = hb_last.elapsed().as_secs_f64();
                let mips = (now_insns - hb_insns) as f64 / dt / 1e6;
                let mode = m.cpu.sys.as_ref().map(|s| format!("{:?}", s.mode)).unwrap_or_default();
                eprintln!("\r[hb] t={:.0}s insns={} {:.0} MIPS pc={:#x} mode={} {}",
                    t0.elapsed().as_secs_f64(), now_insns, mips, m.cpu.pc, mode,
                    m.debug_irq_state());
                eprintln!("[hb] last syscalls (a7@satp): {}", fmt_syscalls(&m.cpu));
                hb_last = std::time::Instant::now();
                hb_insns = now_insns;
            }
        }
        let out = m.console_output();
        if !out.is_empty() {
            std::io::stdout().write_all(&out).unwrap();
            std::io::stdout().flush().unwrap();
        }
        if m.power_off {
            eprintln!("\r\n[vboot] powered off");
            break;
        }
        // Pump the relay both ways once per slice.
        if let Some(r) = relay.as_mut() {
            for frame in m.net_take_output() {
                r.send(&frame);
            }
            for frame in r.recv() {
                m.net_input(&frame);
            }
            if r.is_closed() {
                eprintln!("\r\n[vboot] net: relay closed");
                relay = None;
            }
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

/// Parse `52:54:00:12:34:56`.
fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let mut parts = s.split(':');
    for byte in out.iter_mut() {
        *byte = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    parts.next().is_none().then_some(out)
}

fn fmt_mac(mac: &[u8; 6]) -> String {
    mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

/// Format the last ~20 user syscalls from the CPU ring buffer, decoding
/// common riscv64 syscall numbers, tagging the address space by satp.
fn fmt_syscalls(cpu: &rv64_core::Cpu) -> String {
    fn name(n: u64) -> String {
        match n {
            17 => "getcwd".into(), 23 => "dup".into(), 25 => "fcntl".into(),
            29 => "ioctl".into(), 48 => "faccessat".into(), 56 => "openat".into(),
            57 => "close".into(), 61 => "getdents".into(), 62 => "lseek".into(),
            63 => "read".into(), 64 => "write".into(), 66 => "writev".into(),
            72 => "pselect6".into(), 73 => "ppoll".into(), 78 => "readlinkat".into(),
            79 => "newfstatat".into(), 80 => "fstat".into(), 93 => "exit".into(),
            94 => "exit_group".into(), 96 => "set_tid_address".into(),
            98 => "futex".into(), 99 => "set_robust_list".into(),
            101 => "nanosleep".into(), 113 => "clock_gettime".into(),
            124 => "sched_yield".into(), 129 => "kill".into(), 134 => "sigaction".into(),
            135 => "sigprocmask".into(), 172 => "getpid".into(), 173 => "getppid".into(),
            174 => "getuid".into(), 178 => "gettid".into(), 214 => "brk".into(),
            215 => "munmap".into(), 220 => "clone".into(), 221 => "execve".into(),
            222 => "mmap".into(), 226 => "mprotect".into(), 233 => "madvise".into(),
            260 => "wait4".into(), 261 => "prlimit64".into(), 278 => "getrandom".into(),
            435 => "clone3".into(),
            u64::MAX => "-".into(),
            other => format!("sys{other}"),
        }
    }
    let log = &cpu.syscall_log;
    let n = log.len();
    let mut out = String::new();
    for i in 0..20.min(n) {
        let idx = (cpu.syscall_log_pos + n - 1 - i) % n;
        let (a7, satp) = log[idx];
        if a7 == u64::MAX {
            break;
        }
        // last 5 hex of satp identifies the address space (process)
        out = format!("{}({}@{:x}) ", out, name(a7), satp & 0xfffff);
    }
    out
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
