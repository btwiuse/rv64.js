//! RVC (compressed) expander: each 16-bit instruction maps onto exactly one
//! 32-bit base instruction, so we decompress in front of the one decoder —
//! there is no second execute path.
//!
//! Returns `None` for reserved/illegal encodings (including the all-zero
//! halfword, which the spec defines as illegal).

// ---- 32-bit encoders ----------------------------------------------------

fn enc_r(op: u32, rd: u32, f3: u32, rs1: u32, rs2: u32, f7: u32) -> u32 {
    op | (rd << 7) | (f3 << 12) | (rs1 << 15) | (rs2 << 20) | (f7 << 25)
}

fn enc_i(op: u32, rd: u32, f3: u32, rs1: u32, imm: i32) -> u32 {
    op | (rd << 7) | (f3 << 12) | (rs1 << 15) | (((imm as u32) & 0xfff) << 20)
}

fn enc_s(op: u32, f3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    op | (((imm & 0x1f) << 7) | (f3 << 12) | (rs1 << 15) | (rs2 << 20) | ((imm & 0xfe0) << 20))
}

fn enc_b(op: u32, f3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    op | (((imm >> 11) & 1) << 7)
        | (((imm >> 1) & 0xf) << 8)
        | (f3 << 12)
        | (rs1 << 15)
        | (rs2 << 20)
        | (((imm >> 5) & 0x3f) << 25)
        | (((imm >> 12) & 1) << 31)
}

fn enc_u(op: u32, rd: u32, imm20: u32) -> u32 {
    op | (rd << 7) | (imm20 << 12)
}

fn enc_j(op: u32, rd: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    op | (rd << 7)
        | (imm & 0xff000)
        | (((imm >> 11) & 1) << 20)
        | (((imm >> 1) & 0x3ff) << 21)
        | (((imm >> 20) & 1) << 31)
}

// ---- field helpers ------------------------------------------------------

#[inline]
fn creg(f: u32) -> u32 {
    f + 8 // x8..x15
}

#[inline]
fn sext(val: u32, bits: u32) -> i32 {
    ((val << (32 - bits)) as i32) >> (32 - bits)
}

const OP: u32 = 0x33;
const OP32: u32 = 0x3b;
const OPIMM: u32 = 0x13;
const OPIMM32: u32 = 0x1b;
const LOAD: u32 = 0x03;
const STORE: u32 = 0x23;
const LUI: u32 = 0x37;
const JAL: u32 = 0x6f;
const JALR: u32 = 0x67;
const BRANCH: u32 = 0x63;

/// Expand a 16-bit RVC instruction to its 32-bit equivalent (RV64C).
pub fn expand(c: u16) -> Option<u32> {
    let c = c as u32;
    if c == 0 {
        return None; // defined illegal
    }
    let f3 = (c >> 13) & 7;
    match c & 3 {
        // ---- Quadrant 0 ----
        0 => {
            let rd = creg((c >> 2) & 7);
            let rs1 = creg((c >> 7) & 7);
            match f3 {
                0b000 => {
                    // C.ADDI4SPN: addi rd', x2, nzuimm
                    let imm = (((c >> 11) & 3) << 4)
                        | (((c >> 7) & 0xf) << 6)
                        | (((c >> 6) & 1) << 2)
                        | (((c >> 5) & 1) << 3);
                    if imm == 0 {
                        return None; // reserved
                    }
                    Some(enc_i(OPIMM, rd, 0, 2, imm as i32))
                }
                0b010 => {
                    // C.LW: lw rd', uimm(rs1')
                    let imm = (((c >> 10) & 7) << 3) | (((c >> 6) & 1) << 2) | (((c >> 5) & 1) << 6);
                    Some(enc_i(LOAD, rd, 2, rs1, imm as i32))
                }
                0b011 => {
                    // C.LD: ld rd', uimm(rs1')
                    let imm = (((c >> 10) & 7) << 3) | (((c >> 5) & 3) << 6);
                    Some(enc_i(LOAD, rd, 3, rs1, imm as i32))
                }
                0b110 => {
                    // C.SW
                    let imm = (((c >> 10) & 7) << 3) | (((c >> 6) & 1) << 2) | (((c >> 5) & 1) << 6);
                    Some(enc_s(STORE, 2, rs1, rd, imm as i32))
                }
                0b111 => {
                    // C.SD
                    let imm = (((c >> 10) & 7) << 3) | (((c >> 5) & 3) << 6);
                    Some(enc_s(STORE, 3, rs1, rd, imm as i32))
                }
                // 001/101 = C.FLD/C.FSD — arrive with the D extension
                _ => None,
            }
        }
        // ---- Quadrant 1 ----
        1 => {
            let rd = (c >> 7) & 0x1f;
            let imm6 = sext((((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f), 6);
            match f3 {
                0b000 => Some(enc_i(OPIMM, rd, 0, rd, imm6)), // C.ADDI (incl. nop)
                0b001 => {
                    if rd == 0 {
                        return None; // reserved (RV64: C.ADDIW requires rd != 0)
                    }
                    Some(enc_i(OPIMM32, rd, 0, rd, imm6)) // C.ADDIW
                }
                0b010 => Some(enc_i(OPIMM, rd, 0, 0, imm6)), // C.LI
                0b011 => {
                    if rd == 2 {
                        // C.ADDI16SP
                        let imm = sext(
                            (((c >> 12) & 1) << 9)
                                | (((c >> 6) & 1) << 4)
                                | (((c >> 5) & 1) << 6)
                                | (((c >> 3) & 3) << 7)
                                | (((c >> 2) & 1) << 5),
                            10,
                        );
                        if imm == 0 {
                            return None;
                        }
                        Some(enc_i(OPIMM, 2, 0, 2, imm))
                    } else {
                        // C.LUI
                        if imm6 == 0 {
                            return None;
                        }
                        // imm6 is the sign-extended value of imm[17:12]
                        Some(enc_u(LUI, rd, (imm6 as u32) & 0xfffff))
                    }
                }
                0b100 => {
                    let rdp = creg((c >> 7) & 7);
                    match (c >> 10) & 3 {
                        0 => {
                            // C.SRLI
                            let shamt = (((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f);
                            Some(enc_i(OPIMM, rdp, 5, rdp, shamt as i32))
                        }
                        1 => {
                            // C.SRAI
                            let shamt = (((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f);
                            Some(enc_i(OPIMM, rdp, 5, rdp, (shamt | 0x400) as i32))
                        }
                        2 => Some(enc_i(OPIMM, rdp, 7, rdp, imm6)), // C.ANDI
                        _ => {
                            let rs2 = creg((c >> 2) & 7);
                            match (((c >> 12) & 1) << 2) | ((c >> 5) & 3) {
                                0b000 => Some(enc_r(OP, rdp, 0, rdp, rs2, 0x20)), // C.SUB
                                0b001 => Some(enc_r(OP, rdp, 4, rdp, rs2, 0)),    // C.XOR
                                0b010 => Some(enc_r(OP, rdp, 6, rdp, rs2, 0)),    // C.OR
                                0b011 => Some(enc_r(OP, rdp, 7, rdp, rs2, 0)),    // C.AND
                                0b100 => Some(enc_r(OP32, rdp, 0, rdp, rs2, 0x20)), // C.SUBW
                                0b101 => Some(enc_r(OP32, rdp, 0, rdp, rs2, 0)),  // C.ADDW
                                _ => None,
                            }
                        }
                    }
                }
                0b101 => {
                    // C.J
                    let imm = sext(
                        (((c >> 12) & 1) << 11)
                            | (((c >> 11) & 1) << 4)
                            | (((c >> 9) & 3) << 8)
                            | (((c >> 8) & 1) << 10)
                            | (((c >> 7) & 1) << 6)
                            | (((c >> 6) & 1) << 7)
                            | (((c >> 3) & 7) << 1)
                            | (((c >> 2) & 1) << 5),
                        12,
                    );
                    Some(enc_j(JAL, 0, imm))
                }
                0b110 | 0b111 => {
                    // C.BEQZ / C.BNEZ
                    let rs1 = creg((c >> 7) & 7);
                    let imm = sext(
                        (((c >> 12) & 1) << 8)
                            | (((c >> 10) & 3) << 3)
                            | (((c >> 5) & 3) << 6)
                            | (((c >> 3) & 3) << 1)
                            | (((c >> 2) & 1) << 5),
                        9,
                    );
                    Some(enc_b(BRANCH, if f3 == 0b110 { 0 } else { 1 }, rs1, 0, imm))
                }
                _ => None,
            }
        }
        // ---- Quadrant 2 ----
        2 => {
            let rd = (c >> 7) & 0x1f;
            let rs2 = (c >> 2) & 0x1f;
            match f3 {
                0b000 => {
                    // C.SLLI
                    let shamt = (((c >> 12) & 1) << 5) | ((c >> 2) & 0x1f);
                    Some(enc_i(OPIMM, rd, 1, rd, shamt as i32))
                }
                0b010 => {
                    // C.LWSP
                    if rd == 0 {
                        return None;
                    }
                    let imm =
                        (((c >> 12) & 1) << 5) | (((c >> 4) & 7) << 2) | (((c >> 2) & 3) << 6);
                    Some(enc_i(LOAD, rd, 2, 2, imm as i32))
                }
                0b011 => {
                    // C.LDSP
                    if rd == 0 {
                        return None;
                    }
                    let imm =
                        (((c >> 12) & 1) << 5) | (((c >> 5) & 3) << 3) | (((c >> 2) & 7) << 6);
                    Some(enc_i(LOAD, rd, 3, 2, imm as i32))
                }
                0b100 => {
                    let bit12 = (c >> 12) & 1;
                    match (bit12, rd, rs2) {
                        (0, 0, _) => None,
                        (0, _, 0) => Some(enc_i(JALR, 0, 0, rd, 0)), // C.JR
                        (0, _, _) => Some(enc_r(OP, rd, 0, 0, rs2, 0)), // C.MV
                        (1, 0, 0) => Some(0x0010_0073),              // C.EBREAK
                        (1, _, 0) => Some(enc_i(JALR, 1, 0, rd, 0)), // C.JALR
                        (1, _, _) => Some(enc_r(OP, rd, 0, rd, rs2, 0)), // C.ADD
                        _ => None,
                    }
                }
                0b110 => {
                    // C.SWSP
                    let imm = (((c >> 9) & 0xf) << 2) | (((c >> 7) & 3) << 6);
                    Some(enc_s(STORE, 2, 2, rs2, imm as i32))
                }
                0b111 => {
                    // C.SDSP
                    let imm = (((c >> 10) & 7) << 3) | (((c >> 7) & 7) << 6);
                    Some(enc_s(STORE, 3, 2, rs2, imm as i32))
                }
                // 001/101 = C.FLDSP/C.FSDSP — arrive with the D extension
                _ => None,
            }
        }
        _ => unreachable!(), // (c & 3) == 3 is a 32-bit instruction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cross-checked against `riscv64-unknown-elf-objdump -d` conventions.
    #[test]
    fn expands_common_forms() {
        // c.addi sp, -32  (0x1101) -> addi x2, x2, -32
        assert_eq!(expand(0x1101), Some(enc_i(OPIMM, 2, 0, 2, -32)));
        // c.li a0, 1 (0x4505) -> addi x10, x0, 1
        assert_eq!(expand(0x4505), Some(enc_i(OPIMM, 10, 0, 0, 1)));
        // c.mv a0, a1 (0x852e) -> add x10, x0, x11
        assert_eq!(expand(0x852e), Some(enc_r(OP, 10, 0, 0, 11, 0)));
        // c.add a0, a1 (0x952e) -> add x10, x10, x11
        assert_eq!(expand(0x952e), Some(enc_r(OP, 10, 0, 10, 11, 0)));
        // c.jr ra (0x8082) -> jalr x0, 0(x1)
        assert_eq!(expand(0x8082), Some(enc_i(JALR, 0, 0, 1, 0)));
        // c.ebreak (0x9002)
        assert_eq!(expand(0x9002), Some(0x0010_0073));
        // c.sdsp ra, 8(sp) (0xe406) -> sd x1, 8(x2)
        assert_eq!(expand(0xe406), Some(enc_s(STORE, 3, 2, 1, 8)));
        // c.ldsp ra, 8(sp) (0x60a2) -> ld x1, 8(x2)
        assert_eq!(expand(0x60a2), Some(enc_i(LOAD, 1, 3, 2, 8)));
        // all-zero halfword is illegal
        assert_eq!(expand(0x0000), None);
    }

    #[test]
    fn addi4spn() {
        // c.addi4spn a0, sp, 16 (0x0808) -> addi x10, x2, 16
        assert_eq!(expand(0x0808), Some(enc_i(OPIMM, 10, 0, 2, 16)));
    }

    #[test]
    fn lui_negative() {
        // c.lui a1, 0xfffe1 (i.e. -31 << 12): imm6 = -31 -> lui x11, 0xfffe1
        // encoding: f3=011, rd=11, imm[17]=1 (bit12), imm[16:12]=00001 -> 0x75c5? build manually:
        // c = 011 1 01011 00001 01 = 0b0111010110000101 = 0x7585
        let got = expand(0x7585).unwrap();
        assert_eq!(got & 0x7f, LUI);
        assert_eq!((got >> 7) & 0x1f, 11);
        assert_eq!(got >> 12, 0xfffe1);
    }
}
