//! User-mode Linux emulation: run static riscv64 Linux ELF binaries on the
//! rv64-core CPU by servicing their syscalls on the host side.
//!
//! Works native (rv64-run CLI) and in wasm (console I/O via the `Host`
//! trait). This is the qemu-user model: CPU + syscall ABI, no kernel.

pub mod elf;
pub mod syscall;

use rv64_core::{Cpu, FlatMemory, StopReason};

/// Host-side effects the emulated program can cause. Implemented by the CLI
/// (real stdout) and by the wasm layer (buffered console for JS).
pub trait Host {
    fn write_out(&mut self, fd: i32, bytes: &[u8]);
    /// Monotonic-ish clock in nanoseconds (user program observable only).
    fn clock_ns(&mut self) -> u64 {
        0
    }
    /// Fill with entropy (AT_RANDOM, getrandom).
    fn random(&mut self, buf: &mut [u8]) {
        buf.fill(0x5a);
    }
}

pub struct Machine {
    pub cpu: Cpu,
    pub mem: Vec<u8>,
    pub brk: u64,
    pub brk_start: u64,
    /// Bump allocator for anonymous mmap (top-down from below the stack).
    pub mmap_top: u64,
    pub exit_code: Option<i32>,
}

/// Guest virtual address space size (flat, starts at 0).
pub const DEFAULT_MEM: usize = 256 << 20;
/// Stack: 8 MiB at the top of memory.
const STACK_SIZE: u64 = 8 << 20;

pub enum RunResult {
    /// Program called exit/exit_group.
    Exited(i32),
    /// Instruction budget exhausted (call run again).
    Budget,
    /// Unhandled trap.
    Trap(rv64_core::Exception),
}

impl Machine {
    /// Load a static ELF with the given argv/envp; returns a machine ready to run.
    pub fn load(
        elf_bytes: &[u8],
        argv: &[&str],
        envp: &[&str],
        mem_size: usize,
        host: &mut dyn Host,
    ) -> Result<Machine, elf::ElfError> {
        let mut mem = vec![0u8; mem_size];
        let loaded = elf::load(elf_bytes, &mut mem)?;

        let mut cpu = Cpu::new();
        cpu.pc = loaded.entry;

        let stack_top = mem_size as u64 - 16;
        let stack_bottom = mem_size as u64 - STACK_SIZE;
        let mut m = Machine {
            cpu,
            mem,
            brk: loaded.brk_start,
            brk_start: loaded.brk_start,
            mmap_top: stack_bottom,
            exit_code: None,
        };
        m.setup_stack(stack_top, argv, envp, &loaded, host);
        Ok(m)
    }

    /// Build the initial process stack per the Linux ABI:
    /// strings at the top, then auxv, envp, argv, argc at sp (16-aligned).
    fn setup_stack(
        &mut self,
        stack_top: u64,
        argv: &[&str],
        envp: &[&str],
        loaded: &elf::LoadedElf,
        host: &mut dyn Host,
    ) {
        let mut pos = stack_top;
        let push_bytes = |mem: &mut [u8], pos: &mut u64, bytes: &[u8]| -> u64 {
            *pos -= bytes.len() as u64 + 1; // NUL-terminated
            mem[*pos as usize..*pos as usize + bytes.len()].copy_from_slice(bytes);
            mem[*pos as usize + bytes.len()] = 0;
            *pos
        };

        let argv_ptrs: Vec<u64> =
            argv.iter().map(|s| push_bytes(&mut self.mem, &mut pos, s.as_bytes())).collect();
        let envp_ptrs: Vec<u64> =
            envp.iter().map(|s| push_bytes(&mut self.mem, &mut pos, s.as_bytes())).collect();

        // AT_RANDOM: 16 bytes of entropy
        let mut rnd = [0u8; 16];
        host.random(&mut rnd);
        pos -= 16;
        let at_random = pos;
        self.mem[pos as usize..pos as usize + 16].copy_from_slice(&rnd);

        // auxv (pairs), reverse order of final layout is irrelevant — we
        // compute the full block then place it below the strings.
        const AT_PHDR: u64 = 3;
        const AT_PHENT: u64 = 4;
        const AT_PHNUM: u64 = 5;
        const AT_PAGESZ: u64 = 6;
        const AT_BASE: u64 = 7;
        const AT_FLAGS: u64 = 8;
        const AT_ENTRY: u64 = 9;
        const AT_UID: u64 = 11;
        const AT_EUID: u64 = 12;
        const AT_GID: u64 = 13;
        const AT_EGID: u64 = 14;
        const AT_HWCAP: u64 = 16;
        const AT_CLKTCK: u64 = 17;
        const AT_SECURE: u64 = 23;
        const AT_RANDOM: u64 = 25;
        let auxv: Vec<(u64, u64)> = vec![
            (AT_PHDR, loaded.phdr_addr),
            (AT_PHENT, loaded.phent),
            (AT_PHNUM, loaded.phnum),
            (AT_PAGESZ, 4096),
            (AT_BASE, 0),
            (AT_FLAGS, 0),
            (AT_ENTRY, loaded.entry),
            (AT_UID, 1000),
            (AT_EUID, 1000),
            (AT_GID, 1000),
            (AT_EGID, 1000),
            (AT_HWCAP, 0x112d), // imacd (bits: i,m,a,c,d per HWCAP ISA bits)
            (AT_CLKTCK, 100),
            (AT_SECURE, 0),
            (AT_RANDOM, at_random),
            (0, 0), // AT_NULL
        ];

        // Vector area size: argc + argv + NULL + envp + NULL + auxv
        let words = 1 + argv_ptrs.len() + 1 + envp_ptrs.len() + 1 + auxv.len() * 2;
        pos &= !15; // align before laying the block
        let mut sp = pos - (words as u64 * 8);
        sp &= !15; // final sp must be 16-aligned

        let mut w = sp;
        let put = |mem: &mut [u8], w: &mut u64, v: u64| {
            mem[*w as usize..*w as usize + 8].copy_from_slice(&v.to_le_bytes());
            *w += 8;
        };
        put(&mut self.mem, &mut w, argv.len() as u64); // argc
        for p in &argv_ptrs {
            put(&mut self.mem, &mut w, *p);
        }
        put(&mut self.mem, &mut w, 0);
        for p in &envp_ptrs {
            put(&mut self.mem, &mut w, *p);
        }
        put(&mut self.mem, &mut w, 0);
        for (k, v) in &auxv {
            put(&mut self.mem, &mut w, *k);
            put(&mut self.mem, &mut w, *v);
        }

        self.cpu.x[2] = sp; // sp
    }

    /// Run until exit, unhandled trap, or budget exhaustion.
    pub fn run(&mut self, host: &mut dyn Host, budget: u64) -> RunResult {
        let mut remaining = budget;
        loop {
            let slice = remaining.min(1_000_000);
            let stop = {
                let mut bus = FlatMemory::new(0, &mut self.mem);
                self.cpu.run(&mut bus, slice)
            };
            match stop {
                StopReason::Budget => {
                    remaining = remaining.saturating_sub(slice);
                    if remaining == 0 {
                        return RunResult::Budget;
                    }
                }
                StopReason::Ecall => {
                    if let Some(code) = syscall::handle(self, host) {
                        self.exit_code = Some(code);
                        return RunResult::Exited(code);
                    }
                }
                StopReason::Break => {
                    // Treat like SIGTRAP-without-debugger: abort.
                    return RunResult::Exited(133);
                }
                StopReason::Trap(e) => return RunResult::Trap(e),
            }
        }
    }
}
