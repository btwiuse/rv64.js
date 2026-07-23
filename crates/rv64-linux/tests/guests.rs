//! Integration tests: run the real riscv64 guest binaries under emulation.
//! Guests are built separately (see guests/*/); tests skip if not built.

use rv64_linux::{Host, Machine, RunResult};

struct TestHost {
    stdout: Vec<u8>,
}

impl Host for TestHost {
    fn write_out(&mut self, _fd: i32, bytes: &[u8]) {
        self.stdout.extend_from_slice(bytes);
    }
}

fn run_guest(rel_path: &str, argv: &[&str]) -> Option<(i32, String)> {
    let root = env!("CARGO_MANIFEST_DIR");
    let path = format!("{root}/../../{rel_path}");
    let elf = std::fs::read(&path).ok()?;
    let mut host = TestHost { stdout: Vec::new() };
    let mut m = Machine::load(&elf, argv, &["TERM=dumb"], 256 << 20, &mut host).ok()?;
    match m.run(&mut host, 10_000_000_000) {
        RunResult::Exited(code) => Some((code, String::from_utf8_lossy(&host.stdout).into_owned())),
        other => panic!(
            "guest did not exit cleanly: {:?} at pc={:#x}",
            match other {
                RunResult::Trap(e) => format!("{e:?}"),
                _ => "budget".into(),
            },
            m.cpu.pc
        ),
    }
}

const NOSTD: &str =
    "guests/hello-nostd/target/riscv64gc-unknown-linux-musl/release/hello-nostd";
const STD: &str = "guests/hello-std/target/riscv64gc-unknown-linux-musl/release/hello-std";
const FPU: &str = "guests/fpu-test/target/riscv64gc-unknown-linux-musl/release/fpu-test";

#[test]
fn nostd_guest() {
    let Some((code, out)) = run_guest(NOSTD, &["hello", "abc"]) else {
        eprintln!("skipped: build guests/hello-nostd first");
        return;
    };
    assert_eq!(code, 7);
    assert!(out.contains("hello from rv64 guest!"));
    assert!(out.contains("argc: 2"));
    assert!(out.contains("argv: abc"));
}

#[test]
fn std_guest() {
    let Some((code, out)) = run_guest(STD, &["hello-std", "x"]) else {
        eprintln!("skipped: build guests/hello-std first");
        return;
    };
    assert_eq!(code, 0);
    assert!(out.contains("sum of squares 1..10 = 385"));
    assert!(out.contains("'the' appears 3 times"));
}

#[test]
fn fpu_guest() {
    let Some((code, out)) = run_guest(FPU, &["fpu-test"]) else {
        eprintln!("skipped: build guests/fpu-test first");
        return;
    };
    assert!(out.contains("--- 0 failures"), "output:\n{out}");
    assert_eq!(code, 0);
}
