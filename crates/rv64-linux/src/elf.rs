//! Minimal ELF64 loader — just enough to map static riscv64 executables
//! (ET_EXEC, and ET_DYN/static-pie via a fixed load bias; a static-pie's
//! rcrt1 self-relocates, so we only need to place segments and bias entry).

#[derive(Debug)]
pub enum ElfError {
    NotElf,
    Not64Bit,
    NotRiscv,
    NotExecutable,
    Truncated,
    SegmentOutOfRange { vaddr: u64, size: u64 },
}

pub struct LoadedElf {
    pub entry: u64,
    /// Guest address of the program headers (AT_PHDR).
    pub phdr_addr: u64,
    pub phent: u64,
    pub phnum: u64,
    /// Highest mapped address — initial program break lives above this.
    pub brk_start: u64,
    /// Load bias applied (0 for ET_EXEC).
    pub bias: u64,
}

const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const PT_LOAD: u32 = 1;
const EM_RISCV: u16 = 243;

/// Bias for position-independent executables.
pub const PIE_BASE: u64 = 0x40_0000;

fn r16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn r32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn r64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Map `elf` into `mem` (guest address == index). Returns entry/phdr info.
pub fn load(elf: &[u8], mem: &mut [u8]) -> Result<LoadedElf, ElfError> {
    if elf.len() < 64 {
        return Err(ElfError::Truncated);
    }
    if &elf[0..4] != b"\x7fELF" {
        return Err(ElfError::NotElf);
    }
    if elf[4] != 2 {
        return Err(ElfError::Not64Bit);
    }
    let e_type = r16(elf, 16);
    if r16(elf, 18) != EM_RISCV {
        return Err(ElfError::NotRiscv);
    }
    if e_type != ET_EXEC && e_type != ET_DYN {
        return Err(ElfError::NotExecutable);
    }
    let bias = if e_type == ET_DYN { PIE_BASE } else { 0 };

    let e_entry = r64(elf, 24);
    let e_phoff = r64(elf, 32);
    let e_phentsize = r16(elf, 54) as u64;
    let e_phnum = r16(elf, 56) as u64;

    let mut brk_start = 0u64;
    let mut phdr_addr = 0u64;

    for i in 0..e_phnum {
        let off = (e_phoff + i * e_phentsize) as usize;
        if off + 56 > elf.len() {
            return Err(ElfError::Truncated);
        }
        let p_type = r32(elf, off);
        let p_offset = r64(elf, off + 8);
        let p_vaddr = r64(elf, off + 16).wrapping_add(bias);
        let p_filesz = r64(elf, off + 32);
        let p_memsz = r64(elf, off + 40);
        if p_type != PT_LOAD {
            continue;
        }
        let end = p_vaddr
            .checked_add(p_memsz)
            .ok_or(ElfError::SegmentOutOfRange { vaddr: p_vaddr, size: p_memsz })?;
        if end > mem.len() as u64 {
            return Err(ElfError::SegmentOutOfRange { vaddr: p_vaddr, size: p_memsz });
        }
        let src = p_offset as usize..(p_offset + p_filesz) as usize;
        if src.end > elf.len() {
            return Err(ElfError::Truncated);
        }
        mem[p_vaddr as usize..(p_vaddr + p_filesz) as usize].copy_from_slice(&elf[src]);
        // BSS (memsz > filesz) is already zero — mem starts zeroed.
        brk_start = brk_start.max(end);
        // The program headers live inside the first segment that covers e_phoff.
        if p_offset <= e_phoff && e_phoff < p_offset + p_filesz {
            phdr_addr = p_vaddr + (e_phoff - p_offset);
        }
    }

    // Fallback: copy phdrs somewhere sensible if no segment covered them.
    if phdr_addr == 0 {
        let sz = (e_phentsize * e_phnum) as usize;
        let dst = brk_start as usize;
        mem[dst..dst + sz].copy_from_slice(&elf[e_phoff as usize..e_phoff as usize + sz]);
        phdr_addr = brk_start;
        brk_start += sz as u64;
    }

    Ok(LoadedElf {
        entry: e_entry.wrapping_add(bias),
        phdr_addr,
        phent: e_phentsize,
        phnum: e_phnum,
        brk_start: (brk_start + 0xfff) & !0xfff,
        bias,
    })
}
