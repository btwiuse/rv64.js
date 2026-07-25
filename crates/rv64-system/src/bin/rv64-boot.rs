//! rv64-boot: boot a riscv64 Linux system natively (development harness).
//!
//! Usage: rv64-boot <bbl64.bin> [kernel.bin] [rootfs.bin] [--9p DIR]
//!                  [--9p-tag TAG] [--net ws://HOST:PORT] [--net-mac MAC]
//!                  [-- cmdline]
//! Console on stdio. Ctrl-A x to exit (screen/QEMU style).
//!
//! `--9p DIR` exports a host directory over virtio-9p; in the guest:
//!   mount -t 9p -o trans=virtio,version=9p2000.L host /mnt
//!
//! `--net ws://HOST:PORT` attaches a virtio-net NIC to a WebSocket relay (one
//! binary message per Ethernet frame — websockproxy/v86's protocol). In the
//! guest, configure it however the relay's network expects, e.g.
//!   udhcpc -i eth0
//!
//! `--proxy` instead runs an in-process HTTP proxy behind the NIC: no relay, no
//! external anything. The guest configures the NIC (DHCP or static) and points
//! http_proxy at the printed URL. Natively the proxy speaks plaintext http only
//! (see egress.rs); the browser build gets https because fetch does the TLS.

use rv64_system::egress::NativeEgress;
use rv64_system::httpproxy::Proxy;
use rv64_system::netstack::{NetConfig, NetStack};
use rv64_system::{p9, p9fs, ws, BootImages, Machine};
use std::io::{Read, Write};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut cmdline = "console=hvc0 root=/dev/vda rw".to_string();
    if let Some(pos) = args.iter().position(|a| a == "--") {
        cmdline = args.split_off(pos + 1).join(" ");
        args.pop();
    }
    let mut share = None;
    let mut tag = "host".to_string();
    let mut relay_url = None;
    let mut proxy_mode = false;
    let mut mac = rv64_system::virtio::DEFAULT_MAC;
    let mut positional = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--9p" => share = it.next(),
            "--9p-tag" => tag = it.next().unwrap_or(tag),
            "--net" => relay_url = it.next(),
            "--proxy" => proxy_mode = true,
            "--net-mac" => {
                mac = it
                    .next()
                    .as_deref()
                    .and_then(parse_mac)
                    .unwrap_or_else(|| { eprintln!("bad --net-mac"); std::process::exit(2) })
            }
            _ => positional.push(a),
        }
    }
    if positional.is_empty() {
        eprintln!("usage: rv64-boot <bios> [kernel] [disk] [--9p DIR] [--9p-tag TAG] [--net ws://HOST:PORT | --proxy] [--net-mac MAC] [-- cmdline]");
        std::process::exit(2);
    }
    let bios = std::fs::read(&positional[0]).expect("read bios");
    let kernel = positional
        .get(1)
        .map(|p| std::fs::read(p).expect("read kernel"));
    let disk = positional
        .get(2)
        .map(|p| std::fs::read(p).expect("read disk"));
    let fs = share.map(|dir| {
        eprintln!("[rv64-boot] 9p: exporting {dir} as tag '{tag}'");
        p9::Server::new(tag, Box::new(p9fs::HostFs::new(dir)))
    });
    // Connect the relay before booting: a NIC the guest can see but that has
    // nowhere to send is worse than no NIC at all.
    let mut relay = match &relay_url {
        Some(url) => match ws::Relay::connect(url) {
            Ok(r) => {
                eprintln!("[rv64-boot] net: relay {url}, mac {}", fmt_mac(&mac));
                Some(r)
            }
            Err(e) => {
                eprintln!("[rv64-boot] net: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };
    // Either transport needs the NIC; the proxy needs the host-side stack too.
    let mut proxy_stack = proxy_mode.then(|| {
        let stack = NetStack::new(NetConfig::default());
        eprintln!(
            "[rv64-boot] proxy: {} — in the guest:\n\
             [rv64-boot]   ifconfig eth0 {} netmask {} up\n\
             [rv64-boot]   export http_proxy={}",
            stack.proxy_url(),
            fmt_ip(&stack.config().guest_ip),
            fmt_ip(&stack.config().netmask),
            stack.proxy_url(),
        );
        // Native egress is plaintext-only, so leave the guest's scheme alone.
        (stack, Proxy::new().keep_scheme(), NativeEgress::new())
    });
    let net = (relay.is_some() || proxy_mode).then_some(mac);

    let mut m = Machine::new(
        128,
        BootImages {
            bios: &bios,
            kernel: kernel.as_deref(),
            cmdline: &cmdline,
            disk,
            fs,
            net,
        },
    );

    // Raw terminal so the guest gets keystrokes directly.
    let _raw = RawTerm::enable();
    let mut stdin = nonblocking_stdin();
    let mut ctrl_a = false;

    let t0 = std::time::Instant::now();
    loop {
        m.run_slice(200_000);
        if m.power_off {
            let out = m.console_output();
            std::io::stdout().write_all(&out).ok();
            eprintln!("\r\n[rv64-boot] guest powered off");
            report(&m, t0);
            return;
        }

        let out = m.console_output();
        if !out.is_empty() {
            std::io::stdout().write_all(&out).unwrap();
            std::io::stdout().flush().unwrap();
        }

        // Pump the in-process proxy both ways once per slice.
        if let Some((stack, proxy, egress)) = proxy_stack.as_mut() {
            for frame in m.net_take_output() {
                stack.input(&frame);
            }
            proxy.pump(stack, egress);
            for frame in stack.take_output() {
                m.net_input(&frame);
            }
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
                eprintln!("\r\n[rv64-boot] net: relay closed");
                relay = None;
            }
        }

        let mut buf = [0u8; 256];
        if let Ok(n) = stdin.read(&mut buf) {
            if n > 0 {
                // Ctrl-A x -> quit
                let mut fwd = Vec::new();
                for &b in &buf[..n] {
                    if ctrl_a {
                        ctrl_a = false;
                        if b == b'x' {
                            eprintln!("\r\n[rv64-boot] exit");
                            report(&m, t0);
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

        if std::env::var_os("RV64_MAX_INSNS")
            .map(|v| m.cpu.insn_count > v.to_string_lossy().parse::<u64>().unwrap_or(u64::MAX))
            .unwrap_or(false)
        {
            eprintln!("\r\n[rv64-boot] insn budget reached");
            report(&m, t0);
            if std::env::var_os("RV64_DUMP_LOG").is_some() {
                dump_kernel_log(&m);
            }
            return;
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

fn fmt_ip(o: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3])
}

fn fmt_mac(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Scrape printk text out of guest RAM (development aid): find
/// "Linux version" and print the surrounding log bytes.
fn dump_kernel_log(m: &Machine) {
    let ram = &m.bus.ram;
    let needle = b"Linux version";
    let mut hits = 0;
    let mut i = 0;
    while i + needle.len() < ram.len() && hits < 3 {
        if &ram[i..i + needle.len()] == needle {
            hits += 1;
            let end = (i + 32768).min(ram.len());
            let chunk: String = ram[i..end]
                .iter()
                .map(|&b| {
                    if (32..127).contains(&b) || b == b'\n' {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            eprintln!("--- log candidate at ram+{i:#x}:\n{chunk}\n---");
        }
        i += 1;
    }
    if hits == 0 {
        eprintln!("[rv64-boot] no 'Linux version' string found in RAM");
    }
}

fn report(m: &Machine, t0: std::time::Instant) {
    let secs = t0.elapsed().as_secs_f64();
    eprintln!(
        "[rv64-boot] {} insns in {:.1}s ({:.1} Minsn/s), pc={:#x}",
        m.cpu.insn_count,
        secs,
        m.cpu.insn_count as f64 / secs / 1e6,
        m.cpu.pc
    );
    eprintln!(
        "[rv64-boot] excs {:?}\r\n[rv64-boot] irqs {:?}\r\n[rv64-boot] mtime={} mtimecmp={} mip={:#x} mie={:#x} mode={:?}",
        m.cpu.exc_counts,
        m.cpu.irq_counts,
        m.bus.mtime,
        m.bus.mtimecmp,
        m.cpu.sys.as_ref().unwrap().mip,
        m.cpu.sys.as_ref().unwrap().mie,
        m.cpu.sys.as_ref().unwrap().mode,
    );
}

// ---- tiny raw-terminal helpers (no external crates) ----------------------

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

fn nonblocking_stdin() -> impl Read {
    libcish::set_nonblocking(0);
    std::io::stdin()
}

/// Minimal libc via syscalls — avoids a libc crate dependency.
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
    const TCSANOW: i32 = 0;
    const F_GETFL: i32 = 3;
    const F_SETFL: i32 = 4;
    const O_NONBLOCK: i32 = 0o4000;

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
            t.c_cc[6] = 0; // VMIN
            t.c_cc[5] = 0; // VTIME
            tcsetattr(0, TCSANOW, &t);
            Some(orig)
        }
    }

    pub fn restore(t: &Termios) {
        unsafe {
            tcsetattr(0, TCSANOW, t);
        }
    }

    pub fn set_nonblocking(fd: i32) {
        unsafe {
            let fl = fcntl(fd, F_GETFL, 0);
            fcntl(fd, F_SETFL, fl | O_NONBLOCK);
        }
    }
}
