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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: rv64-isa-test <test-elf>...");
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
        for _ in 0..200 {
            m.run_slice(1_000_000);
            if m.power_off {
                result = m.bus.htif_exit;
                break;
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
