//! Instruction field extraction for the base 32-bit encoding.
//!
//! RV64GC adds the compressed (C) extension: 16-bit forms on 2-byte
//! boundaries, distinguished by `(insn & 3) != 3`. Phase 1 targets rv64i
//! (32-bit forms only); C lands as an expander that maps each 16-bit form
//! onto its 32-bit equivalent before this decoder — no second execute path.

#[inline]
pub fn opcode(insn: u32) -> u32 {
    insn & 0x7f
}

#[inline]
pub fn rd(insn: u32) -> usize {
    ((insn >> 7) & 0x1f) as usize
}

#[inline]
pub fn rs1(insn: u32) -> usize {
    ((insn >> 15) & 0x1f) as usize
}

#[inline]
pub fn rs2(insn: u32) -> usize {
    ((insn >> 20) & 0x1f) as usize
}

#[inline]
pub fn funct3(insn: u32) -> u32 {
    (insn >> 12) & 7
}

#[inline]
pub fn funct7(insn: u32) -> u32 {
    insn >> 25
}

/// I-type immediate: insn[31:20], sign-extended.
#[inline]
pub fn imm_i(insn: u32) -> i64 {
    (insn as i32 as i64) >> 20
}

/// S-type immediate: insn[31:25] ++ insn[11:7], sign-extended.
#[inline]
pub fn imm_s(insn: u32) -> i64 {
    (((insn & 0xfe00_0000) as i32 as i64) >> 20) | (((insn >> 7) & 0x1f) as i64)
}

/// B-type immediate: bit-swizzled branch offset, sign-extended, even.
#[inline]
pub fn imm_b(insn: u32) -> i64 {
    (((insn & 0x8000_0000) as i32 as i64) >> 19)
        | (((insn & 0x80) as i64) << 4)
        | (((insn >> 20) & 0x7e0) as i64)
        | (((insn >> 7) & 0x1e) as i64)
}

/// U-type immediate: insn[31:12] << 12, sign-extended.
#[inline]
pub fn imm_u(insn: u32) -> i64 {
    (insn & 0xffff_f000) as i32 as i64
}

/// J-type immediate: bit-swizzled JAL offset, sign-extended, even.
#[inline]
pub fn imm_j(insn: u32) -> i64 {
    (((insn & 0x8000_0000) as i32 as i64) >> 11)
        | ((insn & 0xff000) as i64)
        | (((insn >> 9) & 0x800) as i64)
        | (((insn >> 20) & 0x7fe) as i64)
}
