//! Phase-6 JIT, v1: translate straight-line RV64 basic blocks into wasm
//! modules (v86's endgame architecture).
//!
//! State contract with the host: the guest register file lives in the
//! module's imported linear memory —
//!
//! ```text
//! offset   0..256   x0..x31 (u64 LE)
//! offset 256        pc      (u64 LE)
//! ```
//!
//! A compiled block updates registers in place, stores the next pc, and
//! returns. The dispatcher (interpreter loop) looks up the next block by pc.
//!
//! v1 scope: OP-IMM/OP/OP-IMM-32/OP-32 (I+M subset), LUI/AUIPC, JAL/JALR,
//! conditional branches. Loads/stores/system/FP end the block and fall back
//! to the interpreter — the tiering seam v86 uses. Compressed instructions
//! are expanded through the same rv64-core expander before translation.

pub mod wasm_emit;

use rv64_core::compressed::expand;
use rv64_core::decode::*;
use wasm_emit::*;

const PC_OFF: u64 = 256;
const MAX_BLOCK: usize = 128;

/// Result of translating one block.
pub struct Block {
    pub wasm: Vec<u8>,
    /// Guest byte length of code consumed.
    pub len: u64,
    /// Number of instructions translated.
    pub n_insns: u32,
}

/// Fetch helper over a code slice starting at `base` (guest address).
fn fetch(code: &[u8], base: u64, pc: u64) -> Option<(u32, u64)> {
    let off = pc.checked_sub(base)? as usize;
    let lo = u16::from_le_bytes(code.get(off..off + 2)?.try_into().ok()?) as u32;
    if lo & 3 == 3 {
        let hi = u16::from_le_bytes(code.get(off + 2..off + 4)?.try_into().ok()?) as u32;
        Some((lo | (hi << 16), 4))
    } else {
        expand(lo as u16).map(|e| (e, 2))
    }
}

/// Emit `push x[r]` (reads the register slot; x0 is constant 0).
fn push_reg(m: &mut WasmModule, r: usize) {
    if r == 0 {
        m.i64_const(0);
    } else {
        m.i32_const(0).i64_load(r as u64 * 8);
    }
}

/// Emit `x[rd] = <top of stack>`. The value must be on the stack *after*
/// the address operand — so callers emit `i32_const(0)` first via this
/// helper pair: `store_reg_pre` ... value ... `store_reg_post`.
fn store_pre(m: &mut WasmModule, rd: usize) -> bool {
    if rd == 0 {
        return false; // caller must still balance the stack (skip entirely)
    }
    m.i32_const(0);
    true
}

fn store_post(m: &mut WasmModule, rd: usize) {
    m.i64_store(rd as u64 * 8);
}

/// Store the (constant) next pc and return.
fn set_pc_const(m: &mut WasmModule, pc: u64) {
    m.i32_const(0).i64_const(pc as i64).i64_store(PC_OFF);
}

/// Translate one basic block starting at `pc`. `code` is the guest code
/// bytes and `base` its guest address. Returns None if the very first
/// instruction isn't translatable (caller interprets it instead).
pub fn translate_block(code: &[u8], base: u64, start_pc: u64) -> Option<Block> {
    let mut m = WasmModule::new(2); // locals: 0 = scratch a, 1 = scratch b
    let mut pc = start_pc;
    let mut n = 0u32;

    while n < MAX_BLOCK as u32 {
        let Some((insn, ilen)) = fetch(code, base, pc) else {
            break;
        };
        let next_pc = pc.wrapping_add(ilen);
        let op = opcode(insn);
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));

        match op {
            // LUI / AUIPC: constants at translation time.
            0x37 | 0x17 => {
                if store_pre(&mut m, d) {
                    let v = if op == 0x37 {
                        imm_u(insn) as u64
                    } else {
                        pc.wrapping_add(imm_u(insn) as u64)
                    };
                    m.i64_const(v as i64);
                    store_post(&mut m, d);
                }
            }
            // OP-IMM
            0x13 => {
                let imm = imm_i(insn);
                let f3 = funct3(insn);
                if store_pre(&mut m, d) {
                    push_reg(&mut m, s1);
                    match f3 {
                        0 => {
                            m.i64_const(imm).op(I64_ADD);
                        }
                        1 => {
                            m.i64_const(imm & 0x3f).op(I64_SHL);
                        }
                        2 => {
                            m.i64_const(imm).op(I64_LT_S).op(I64_EXTEND_I32_U);
                        }
                        3 => {
                            m.i64_const(imm).op(I64_LT_U).op(I64_EXTEND_I32_U);
                        }
                        4 => {
                            m.i64_const(imm).op(I64_XOR);
                        }
                        5 => {
                            if insn >> 26 == 0x10 {
                                m.i64_const(imm & 0x3f).op(I64_SHR_S);
                            } else {
                                m.i64_const(imm & 0x3f).op(I64_SHR_U);
                            }
                        }
                        6 => {
                            m.i64_const(imm).op(I64_OR);
                        }
                        _ => {
                            m.i64_const(imm).op(I64_AND);
                        }
                    }
                    store_post(&mut m, d);
                }
            }
            // OP (I and M-mul only; div falls back)
            0x33 => {
                let f7 = funct7(insn);
                let f3 = funct3(insn);
                let supported = matches!((f7, f3), (0x00, _) | (0x20, 0) | (0x20, 5) | (0x01, 0));
                if !supported {
                    break;
                }
                if store_pre(&mut m, d) {
                    push_reg(&mut m, s1);
                    match (f7, f3) {
                        (0x00, 0) => {
                            push_reg(&mut m, s2);
                            m.op(I64_ADD);
                        }
                        (0x20, 0) => {
                            push_reg(&mut m, s2);
                            m.op(I64_SUB);
                        }
                        (0x01, 0) => {
                            push_reg(&mut m, s2);
                            m.op(I64_MUL);
                        }
                        (0x00, 1) => {
                            push_reg(&mut m, s2);
                            m.i64_const(0x3f).op(I64_AND).op(I64_SHL);
                        }
                        (0x00, 2) => {
                            push_reg(&mut m, s2);
                            m.op(I64_LT_S).op(I64_EXTEND_I32_U);
                        }
                        (0x00, 3) => {
                            push_reg(&mut m, s2);
                            m.op(I64_LT_U).op(I64_EXTEND_I32_U);
                        }
                        (0x00, 4) => {
                            push_reg(&mut m, s2);
                            m.op(I64_XOR);
                        }
                        (0x00, 5) => {
                            push_reg(&mut m, s2);
                            m.i64_const(0x3f).op(I64_AND).op(I64_SHR_U);
                        }
                        (0x20, 5) => {
                            push_reg(&mut m, s2);
                            m.i64_const(0x3f).op(I64_AND).op(I64_SHR_S);
                        }
                        (0x00, 6) => {
                            push_reg(&mut m, s2);
                            m.op(I64_OR);
                        }
                        (0x00, 7) => {
                            push_reg(&mut m, s2);
                            m.op(I64_AND);
                        }
                        _ => unreachable!(),
                    }
                    store_post(&mut m, d);
                }
            }
            // OP-IMM-32 (ADDIW/SLLIW/SRLIW/SRAIW): compute in 64, wrap+extend.
            0x1b => {
                let imm = imm_i(insn);
                let f3 = funct3(insn);
                if !matches!(f3, 0 | 1 | 5) {
                    break;
                }
                if store_pre(&mut m, d) {
                    push_reg(&mut m, s1);
                    match f3 {
                        0 => {
                            m.i64_const(imm).op(I64_ADD);
                        }
                        1 => {
                            m.i64_const(imm & 0x1f).op(I64_SHL);
                        }
                        _ => {
                            // shift on the 32-bit value
                            m.op(I32_WRAP_I64).op(I64_EXTEND_I32_U);
                            if funct7(insn) == 0x20 {
                                m.op(I32_WRAP_I64)
                                    .op(I64_EXTEND_I32_S)
                                    .i64_const(imm & 0x1f)
                                    .op(I64_SHR_S);
                            } else {
                                m.i64_const(0xffff_ffff)
                                    .op(I64_AND)
                                    .i64_const(imm & 0x1f)
                                    .op(I64_SHR_U);
                            }
                        }
                    }
                    // sign-extend low 32 bits
                    m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                    store_post(&mut m, d);
                }
            }
            // OP-32 (ADDW/SUBW/MULW)
            0x3b => {
                let (f7, f3) = (funct7(insn), funct3(insn));
                if !matches!((f7, f3), (0x00, 0) | (0x20, 0) | (0x01, 0)) {
                    break;
                }
                if store_pre(&mut m, d) {
                    push_reg(&mut m, s1);
                    push_reg(&mut m, s2);
                    m.op(match (f7, f3) {
                        (0x00, 0) => I64_ADD,
                        (0x20, 0) => I64_SUB,
                        _ => I64_MUL,
                    });
                    m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                    store_post(&mut m, d);
                }
            }
            // JAL: link + constant jump; block ends.
            0x6f => {
                if store_pre(&mut m, d) {
                    m.i64_const(next_pc as i64);
                    store_post(&mut m, d);
                }
                set_pc_const(&mut m, pc.wrapping_add(imm_j(insn) as u64));
                return Some(Block {
                    wasm: m.finish(),
                    len: next_pc - start_pc,
                    n_insns: n + 1,
                });
            }
            // JALR: dynamic target; block ends.
            0x67 => {
                // scratch = (x[rs1] + imm) & ~1  (compute before link write!)
                push_reg(&mut m, s1);
                m.i64_const(imm_i(insn))
                    .op(I64_ADD)
                    .i64_const(!1)
                    .op(I64_AND)
                    .local_set(0);
                if store_pre(&mut m, d) {
                    m.i64_const(next_pc as i64);
                    store_post(&mut m, d);
                }
                m.i32_const(0).local_get(0).i64_store(PC_OFF);
                return Some(Block {
                    wasm: m.finish(),
                    len: next_pc - start_pc,
                    n_insns: n + 1,
                });
            }
            // BRANCH: conditional pc select; block ends.
            0x63 => {
                let target = pc.wrapping_add(imm_b(insn) as u64);
                let cmp = match funct3(insn) {
                    0 => I64_EQ,
                    1 => I64_NE,
                    4 => I64_LT_S,
                    5 => I64_GE_S,
                    6 => I64_LT_U,
                    7 => I64_GE_U,
                    _ => break, // before any stack pushes — stream stays balanced
                };
                push_reg(&mut m, s1);
                push_reg(&mut m, s2);
                m.op(cmp);
                m.op(IF).op(VOID);
                set_pc_const(&mut m, target);
                m.op(ELSE);
                set_pc_const(&mut m, next_pc);
                m.op(END);
                return Some(Block {
                    wasm: m.finish(),
                    len: next_pc - start_pc,
                    n_insns: n + 1,
                });
            }
            // Anything else (loads/stores/AMO/FP/SYSTEM): end the block here;
            // the interpreter takes over at this pc.
            _ => break,
        }

        pc = next_pc;
        n += 1;
    }

    if n == 0 {
        return None;
    }
    set_pc_const(&mut m, pc);
    Some(Block {
        wasm: m.finish(),
        len: pc - start_pc,
        n_insns: n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // sum 1..10 program from the core tests
    const PROG: [u32; 7] = [
        0x00000093, 0x00100113, 0x00b00193, 0x002080b3, 0x00110113, 0xfe311ce3, 0x00000073,
    ];

    fn code_bytes() -> Vec<u8> {
        PROG.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn translates_leading_block() {
        // Block 1: three addis then falls into the loop body... the block
        // actually extends through the branch (bne terminates it).
        let b = translate_block(&code_bytes(), 0x1000, 0x1000).unwrap();
        assert_eq!(b.n_insns, 6); // addi,addi,addi,add,addi,bne
        assert!(b.wasm.starts_with(&[0x00, 0x61, 0x73, 0x6d])); // \0asm
    }

    #[test]
    fn loop_body_block() {
        let b = translate_block(&code_bytes(), 0x1000, 0x100c).unwrap();
        assert_eq!(b.n_insns, 3); // add, addi, bne
    }

    #[test]
    fn ecall_not_translatable() {
        assert!(translate_block(&code_bytes(), 0x1000, 0x1018).is_none());
    }

    #[test]
    fn compressed_input_translates() {
        // c.li a0, 21 ; c.mv a1, a0 ; c.add a0, a1 ; ecall(32-bit)
        let mut code = Vec::new();
        for h in [0x4555u16, 0x85aa, 0x952e] {
            code.extend_from_slice(&h.to_le_bytes());
        }
        code.extend_from_slice(&0x0000_0073u32.to_le_bytes());
        let b = translate_block(&code, 0, 0).unwrap();
        assert_eq!(b.n_insns, 3);
        assert_eq!(b.len, 6);
    }
}
