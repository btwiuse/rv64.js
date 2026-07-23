//! rv64-run: run a static riscv64 Linux ELF natively (qemu-user style).
//! Usage: rv64-run <elf> [args...]

use rv64_linux::{Host, Machine, RunResult, DEFAULT_MEM};
use std::io::Write;
use std::time::Instant;

struct NativeHost {
    t0: Instant,
}

impl Host for NativeHost {
    fn write_out(&mut self, fd: i32, bytes: &[u8]) {
        if fd == 2 {
            std::io::stderr().write_all(bytes).unwrap();
        } else {
            std::io::stdout().write_all(bytes).unwrap();
        }
    }
    fn clock_ns(&mut self) -> u64 {
        self.t0.elapsed().as_nanos() as u64
    }
    fn random(&mut self, buf: &mut [u8]) {
        // Deterministic enough for development; swap for getrandom if needed.
        let mut x: u64 = 0x2545F4914F6CDD1D;
        for b in buf.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: rv64-run <elf> [args...]");
        std::process::exit(2);
    }
    let elf = std::fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("rv64-run: {}: {}", args[1], e);
        std::process::exit(2);
    });

    let guest_argv: Vec<&str> = args[1..].iter().map(String::as_str).collect();
    let envp = ["PATH=/bin", "HOME=/", "TERM=dumb"];

    let mut host = NativeHost { t0: Instant::now() };
    let mut machine = Machine::load(&elf, &guest_argv, &envp, DEFAULT_MEM, &mut host)
        .unwrap_or_else(|e| {
            eprintln!("rv64-run: load error: {e:?}");
            std::process::exit(2);
        });

    match machine.run(&mut host, u64::MAX) {
        RunResult::Exited(code) => {
            let insns = machine.cpu.insn_count;
            if std::env::var_os("RV64_STATS").is_some() {
                let secs = host.t0.elapsed().as_secs_f64();
                eprintln!(
                    "[rv64-run] {} insns in {:.3}s ({:.1} Minsn/s)",
                    insns,
                    secs,
                    insns as f64 / secs / 1e6
                );
            }
            std::process::exit(code & 0xff);
        }
        RunResult::Trap(e) => {
            eprintln!(
                "rv64-run: unhandled trap {:?} at pc={:#x} (insn #{})",
                e, machine.cpu.pc, machine.cpu.insn_count
            );
            std::process::exit(139);
        }
        RunResult::Budget => unreachable!(),
    }
}
