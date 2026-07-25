//! rv64-isa-test: run official riscv-tests ISA binaries against the machine.
//!
//! Usage: rv64-isa-test <test-elf>...
//! Each test is a bare-metal ELF (entry 0x80000000) that reports its result
//! by writing to the `tohost` symbol: 1 = pass, (n<<1)|1 = case n failed.

use rv64_system::{BootImages, Machine, RAM_BASE};

fn r16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn r32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn r64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Load PT_LOAD segments into physical RAM; find the `tohost` symbol.
fn load_test(elf: &[u8], ram: &mut [u8]) -> Result<u64, String> {
    if &elf[0..4] != b"\x7fELF" || elf[4] != 2 {
        return Err("not a 64-bit ELF".into());
    }
    let e_phoff = r64(elf, 32) as usize;
    let e_shoff = r64(elf, 40) as usize;
    let e_phentsize = r16(elf, 54) as usize;
    let e_phnum = r16(elf, 56) as usize;
    let e_shentsize = r16(elf, 58) as usize;
    let e_shnum = r16(elf, 60) as usize;

    for i in 0..e_phnum {
        let off = e_phoff + i * e_phentsize;
        if r32(elf, off) != 1 {
            continue; // PT_LOAD
        }
        let p_offset = r64(elf, off + 8) as usize;
        let p_paddr = r64(elf, off + 24); // physical address
        let p_filesz = r64(elf, off + 32) as usize;
        let dst = p_paddr
            .checked_sub(RAM_BASE)
            .ok_or_else(|| format!("segment below RAM: {p_paddr:#x}"))? as usize;
        if dst + p_filesz > ram.len() {
            return Err("segment beyond RAM".into());
        }
        ram[dst..dst + p_filesz].copy_from_slice(&elf[p_offset..p_offset + p_filesz]);
    }

    // Symbol table scan for `tohost`.
    let mut symtab = None;
    let mut strtab = None;
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        match r32(elf, off + 4) {
            2 => symtab = Some((r64(elf, off + 24) as usize, r64(elf, off + 32) as usize)),
            3 if strtab.is_none() && i != r16(elf, 62) as usize => {
                // first non-shstrtab string table
                strtab = Some(r64(elf, off + 24) as usize)
            }
            _ => {}
        }
    }
    let (sym_off, sym_size) = symtab.ok_or("no symtab")?;
    let str_off = strtab.ok_or("no strtab")?;
    for s in (0..sym_size).step_by(24) {
        let name_off = str_off + r32(elf, sym_off + s) as usize;
        let name_end = elf[name_off..].iter().position(|&b| b == 0).unwrap_or(0) + name_off;
        if &elf[name_off..name_end] == b"tohost" {
            return Ok(r64(elf, sym_off + s + 8)); // st_value
        }
    }
    Err("no tohost symbol".into())
}

/// Look up any symbol's value (for begin_signature/end_signature).
fn find_symbol(elf: &[u8], name: &str) -> Option<u64> {
    let e_shoff = r64(elf, 40) as usize;
    let e_shentsize = r16(elf, 58) as usize;
    let e_shnum = r16(elf, 60) as usize;
    let mut symtab = None;
    let mut strtab = None;
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        match r32(elf, off + 4) {
            2 => symtab = Some((r64(elf, off + 24) as usize, r64(elf, off + 32) as usize)),
            3 if strtab.is_none() && i != r16(elf, 62) as usize => {
                strtab = Some(r64(elf, off + 24) as usize)
            }
            _ => {}
        }
    }
    let (sym_off, sym_size) = symtab?;
    let str_off = strtab?;
    for s in (0..sym_size).step_by(24) {
        let name_off = str_off + r32(elf, sym_off + s) as usize;
        let name_end = elf[name_off..].iter().position(|&b| b == 0).unwrap_or(0) + name_off;
        if &elf[name_off..name_end] == name.as_bytes() {
            return Some(r64(elf, sym_off + s + 8));
        }
    }
    None
}

/// If the instruction at `pc` architecturally writes a non-zero x-register,
/// return that register (best effort: physical read, satp=0 assumption —
/// fine for the bare-metal p-variant tests lockstep runs on).
fn insn_x_dest(m: &Machine, pc: u64) -> Option<usize> {
    use rv64_system::RAM_BASE;
    let off = pc.checked_sub(RAM_BASE)? as usize;
    if off + 4 > m.bus.ram.len() {
        return None;
    }
    let lo = u16::from_le_bytes(m.bus.ram[off..off + 2].try_into().unwrap()) as u32;
    let insn = if lo & 3 == 3 {
        lo | ((u16::from_le_bytes(m.bus.ram[off + 2..off + 4].try_into().unwrap()) as u32) << 16)
    } else {
        rv64_core::compressed::expand(lo as u16)?
    };
    use rv64_core::decode::{funct3, funct7, opcode, rd};
    let writes = match opcode(insn) {
        0x37 | 0x17 | 0x6f | 0x67 | 0x03 | 0x13 | 0x33 | 0x1b | 0x3b | 0x2f => true,
        0x73 => funct3(insn) != 0 && funct3(insn) != 4, // CSR ops
        // OP-FP forms whose destination is an x-register:
        // fcmp (0x50/0x51), fcvt.int.fmt (0x60/0x61), fmv.x/fclass (0x70/0x71)
        0x53 => matches!(funct7(insn), 0x50 | 0x51 | 0x60 | 0x61 | 0x70 | 0x71),
        _ => false,
    };
    (writes && rd(insn) != 0).then(|| rd(insn))
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // --signature FILE : dump begin_signature..end_signature after the run
    //                    (RISCOF DUT protocol; single test only)
    // --trace FILE     : per-instruction commit log of x-register writes,
    //                    normalizable against `spike --log-commits`
    let mut sig_path = None;
    let mut trace_path = None;
    while args.len() >= 2 {
        match args[0].as_str() {
            "--signature" => {
                sig_path = Some(args[1].clone());
                args.drain(..2);
            }
            "--trace" => {
                trace_path = Some(args[1].clone());
                args.drain(..2);
            }
            _ => break,
        }
    }
    if args.is_empty() {
        eprintln!("usage: rv64-isa-test [--signature FILE] [--trace FILE] <test-elf>...");
        std::process::exit(2);
    }

    let (mut passed, mut failed) = (0u32, 0u32);
    for path in &args {
        let elf = std::fs::read(path).expect("read test");
        let name = path.rsplit('/').next().unwrap();

        let mut m = Machine::new(
            64,
            BootImages {
                bios: &[],
                kernel: None,
                cmdline: "",
                disk: None,
                fs: None,
            },
        );
        match load_test(&elf, &mut m.bus.ram) {
            Ok(tohost) => m.bus.htif_base = tohost,
            Err(e) => {
                println!("ERROR {name}: {e}");
                failed += 1;
                continue;
            }
        }

        let mut result = None;
        if let Some(tp) = &trace_path {
            // Lockstep trace: single-step; for every instruction that
            // architecturally writes an x-register, emit "pc reg value" —
            // the same event stream `spike --log-commits` produces.
            use std::io::Write;
            let mut w = std::io::BufWriter::new(std::fs::File::create(tp).expect("trace file"));
            for _ in 0..60_000_000u64 {
                let pc = m.cpu.pc;
                let dest = insn_x_dest(&m, pc);
                let excs_before: u64 = m.cpu.exc_counts.iter().sum();
                m.run_slice(1);
                let trapped = m.cpu.exc_counts.iter().sum::<u64>() != excs_before;
                if let Some(rd) = dest {
                    if !trapped {
                        writeln!(w, "{pc:#x} x{rd} {:#x}", m.cpu.x[rd]).unwrap();
                    }
                }
                if m.power_off {
                    result = m.bus.htif_exit;
                    break;
                }
            }
        } else {
            for _ in 0..200 {
                m.run_slice(1_000_000);
                if m.power_off {
                    result = m.bus.htif_exit;
                    break;
                }
            }
        }

        // RISCOF signature dump (4 bytes per line, lowercase hex).
        if let Some(sp) = &sig_path {
            use std::io::Write;
            let (b, e) = (
                find_symbol(&elf, "begin_signature"),
                find_symbol(&elf, "end_signature"),
            );
            if let (Some(b), Some(e)) = (b, e) {
                let mut w =
                    std::io::BufWriter::new(std::fs::File::create(sp).expect("signature file"));
                let mut a = b;
                while a < e {
                    let off = (a - RAM_BASE) as usize;
                    let word = u32::from_le_bytes(m.bus.ram[off..off + 4].try_into().unwrap());
                    writeln!(w, "{word:08x}").unwrap();
                    a += 4;
                }
            } else {
                eprintln!("warning: no begin/end_signature symbols in {name}");
            }
        }
        match result {
            Some(0) => {
                passed += 1;
                println!("PASS {name}");
            }
            Some(n) => {
                failed += 1;
                println!("FAIL {name} (test case {n}) pc={:#x}", m.cpu.pc);
            }
            None => {
                failed += 1;
                println!(
                    "TIMEOUT {name} pc={:#x} insns={}",
                    m.cpu.pc, m.cpu.insn_count
                );
            }
        }
    }
    println!("--- {passed} passed, {failed} failed");
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
