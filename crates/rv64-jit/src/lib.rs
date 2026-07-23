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

const MAX_BLOCK: usize = 128;
/// First scratch local (local 0 is the state-pointer parameter).
const SCR: u32 = 1;

/// Where the emitted code finds emulator state in linear memory, and
/// (optionally) guest RAM for direct load/store translation.
#[derive(Clone, Copy)]
pub struct JitLayout {
    /// Linear-memory offset of x[0] (x1.. follow at 8-byte stride).
    pub x_base: u32,
    /// Linear-memory offset of the pc slot.
    pub pc_addr: u32,
    /// Guest RAM window for direct memory ops: (linear offset of guest
    /// address 0, guest size in bytes). None => loads/stores end the block
    /// (interpreter fallback) — used for full-system blocks where accesses
    /// must go through the MMU.
    pub mem: Option<(u32, u64)>,
}

impl JitLayout {
    /// Layout used by the standalone tests: x at 0, pc at 256, no memory.
    pub fn bare() -> JitLayout {
        JitLayout {
            x_base: 0,
            pc_addr: 256,
            mem: None,
        }
    }
}

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

struct Ctx {
    lay: JitLayout,
}

impl Ctx {
    /// Emit `push x[r]` (reads the register slot; x0 is constant 0).
    fn push_reg(&self, m: &mut WasmModule, r: usize) {
        if r == 0 {
            m.i64_const(0);
        } else {
            m.i32_const(0)
                .i64_load(self.lay.x_base as u64 + r as u64 * 8);
        }
    }

    fn store_pre(&self, m: &mut WasmModule, rd: usize) -> bool {
        if rd == 0 {
            return false;
        }
        m.i32_const(0);
        true
    }

    fn store_post(&self, m: &mut WasmModule, rd: usize) {
        m.i64_store(self.lay.x_base as u64 + rd as u64 * 8);
    }

    /// Store the (constant) next pc.
    fn set_pc_const(&self, m: &mut WasmModule, pc: u64) {
        m.i32_const(0)
            .i64_const(pc as i64)
            .i64_store(self.lay.pc_addr as u64);
    }

    /// Guest address (i64) is on the stack. Bounds-check it against guest
    /// RAM and leave the wrapped i32 index on the stack. Traps (wasm
    /// `unreachable`) on out-of-range — a fatal guest fault in user mode.
    fn guest_addr(&self, m: &mut WasmModule, size: u64, len: u64) {
        m.local_set(SCR);
        m.local_get(SCR).i64_const((size - len) as i64).op(I64_GT_U);
        m.op(IF).op(VOID).op(UNREACHABLE).op(END);
        m.local_get(SCR).op(I32_WRAP_I64);
    }
}

/// Translate one basic block starting at `pc`. `code` is the guest code
/// bytes and `base` its guest address. Returns None if the very first
/// instruction isn't translatable (caller interprets it instead).
pub fn translate_block(code: &[u8], base: u64, start_pc: u64, lay: JitLayout) -> Option<Block> {
    let c = Ctx { lay };
    let mut m = WasmModule::new(2); // scratch i64 locals at SCR, SCR+1
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
                if c.store_pre(&mut m, d) {
                    let v = if op == 0x37 {
                        imm_u(insn) as u64
                    } else {
                        pc.wrapping_add(imm_u(insn) as u64)
                    };
                    m.i64_const(v as i64);
                    c.store_post(&mut m, d);
                }
            }
            // OP-IMM
            0x13 => {
                let imm = imm_i(insn);
                let f3 = funct3(insn);
                if c.store_pre(&mut m, d) {
                    c.push_reg(&mut m, s1);
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
                    c.store_post(&mut m, d);
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
                if c.store_pre(&mut m, d) {
                    c.push_reg(&mut m, s1);
                    match (f7, f3) {
                        (0x00, 0) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_ADD);
                        }
                        (0x20, 0) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_SUB);
                        }
                        (0x01, 0) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_MUL);
                        }
                        (0x00, 1) => {
                            c.push_reg(&mut m, s2);
                            m.i64_const(0x3f).op(I64_AND).op(I64_SHL);
                        }
                        (0x00, 2) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_LT_S).op(I64_EXTEND_I32_U);
                        }
                        (0x00, 3) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_LT_U).op(I64_EXTEND_I32_U);
                        }
                        (0x00, 4) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_XOR);
                        }
                        (0x00, 5) => {
                            c.push_reg(&mut m, s2);
                            m.i64_const(0x3f).op(I64_AND).op(I64_SHR_U);
                        }
                        (0x20, 5) => {
                            c.push_reg(&mut m, s2);
                            m.i64_const(0x3f).op(I64_AND).op(I64_SHR_S);
                        }
                        (0x00, 6) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_OR);
                        }
                        (0x00, 7) => {
                            c.push_reg(&mut m, s2);
                            m.op(I64_AND);
                        }
                        _ => unreachable!(),
                    }
                    c.store_post(&mut m, d);
                }
            }
            // OP-IMM-32 (ADDIW/SLLIW/SRLIW/SRAIW): compute in 64, wrap+extend.
            0x1b => {
                let imm = imm_i(insn);
                let f3 = funct3(insn);
                if !matches!(f3, 0 | 1 | 5) {
                    break;
                }
                if c.store_pre(&mut m, d) {
                    c.push_reg(&mut m, s1);
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
                    c.store_post(&mut m, d);
                }
            }
            // OP-32 (ADDW/SUBW/MULW)
            0x3b => {
                let (f7, f3) = (funct7(insn), funct3(insn));
                if !matches!((f7, f3), (0x00, 0) | (0x20, 0) | (0x01, 0)) {
                    break;
                }
                if c.store_pre(&mut m, d) {
                    c.push_reg(&mut m, s1);
                    c.push_reg(&mut m, s2);
                    m.op(match (f7, f3) {
                        (0x00, 0) => I64_ADD,
                        (0x20, 0) => I64_SUB,
                        _ => I64_MUL,
                    });
                    m.op(I32_WRAP_I64).op(I64_EXTEND_I32_S);
                    c.store_post(&mut m, d);
                }
            }
            // LOAD / STORE-FP-less loads: direct guest memory (user-mode only)
            0x03 if lay.mem.is_some() => {
                let (mem_base, size) = lay.mem.unwrap();
                let f3 = funct3(insn);
                let len = match f3 {
                    0 | 4 => 1,
                    1 | 5 => 2,
                    2 | 6 => 4,
                    3 => 8,
                    _ => break,
                };
                c.push_reg(&mut m, s1);
                m.i64_const(imm_i(insn)).op(I64_ADD);
                c.guest_addr(&mut m, size, len); // i32 index on stack
                                                 // value
                m.op(match f3 {
                    0 => I64_LOAD8_S,
                    1 => I64_LOAD16_S,
                    2 => I64_LOAD32_S,
                    3 => I64_LOAD,
                    4 => I64_LOAD8_U,
                    5 => I64_LOAD16_U,
                    _ => I64_LOAD32_U,
                });
                // align hint + offset immediates
                {
                    let a = match len {
                        1 => 0u64,
                        2 => 1,
                        4 => 2,
                        _ => 3,
                    };
                    m.raw_uleb(a).raw_uleb(mem_base as u64);
                }
                if d == 0 {
                    m.op(DROP); // load to x0: access happens, result discarded
                } else {
                    // stash value, then store to x[rd]
                    m.local_set(SCR + 1);
                    m.i32_const(0).local_get(SCR + 1);
                    c.store_post(&mut m, d);
                }
            }
            0x23 if lay.mem.is_some() => {
                let (mem_base, size) = lay.mem.unwrap();
                let f3 = funct3(insn);
                if f3 > 3 {
                    break;
                }
                let len = 1u64 << f3;
                c.push_reg(&mut m, s1);
                m.i64_const(imm_s(insn)).op(I64_ADD);
                c.guest_addr(&mut m, size, len); // i32 index
                c.push_reg(&mut m, s2); // value i64
                m.op(match f3 {
                    0 => I64_STORE8,
                    1 => I64_STORE16,
                    2 => I64_STORE32,
                    _ => I64_STORE,
                });
                let a = match len {
                    1 => 0u64,
                    2 => 1,
                    4 => 2,
                    _ => 3,
                };
                m.raw_uleb(a).raw_uleb(mem_base as u64);
            }
            // JAL: link; follow plain forward jumps (superblock chaining),
            // otherwise end the block with a constant pc.
            0x6f => {
                let target = pc.wrapping_add(imm_j(insn) as u64);
                if c.store_pre(&mut m, d) {
                    m.i64_const(next_pc as i64);
                    c.store_post(&mut m, d);
                }
                let in_window = target > pc && target >= base && target < base + code.len() as u64;
                if d == 0 && in_window {
                    pc = target;
                    n += 1;
                    continue;
                }
                c.set_pc_const(&mut m, target);
                return Some(Block {
                    wasm: m.finish(),
                    len: next_pc - start_pc,
                    n_insns: n + 1,
                });
            }
            // JALR: dynamic target; block ends.
            0x67 => {
                // scratch = (x[rs1] + imm) & ~1  (compute before link write!)
                c.push_reg(&mut m, s1);
                m.i64_const(imm_i(insn))
                    .op(I64_ADD)
                    .i64_const(!1)
                    .op(I64_AND)
                    .local_set(SCR);
                if c.store_pre(&mut m, d) {
                    m.i64_const(next_pc as i64);
                    c.store_post(&mut m, d);
                }
                m.i32_const(0).local_get(SCR).i64_store(lay.pc_addr as u64);
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
                c.push_reg(&mut m, s1);
                c.push_reg(&mut m, s2);
                m.op(cmp);
                m.op(IF).op(VOID);
                c.set_pc_const(&mut m, target);
                m.op(ELSE);
                c.set_pc_const(&mut m, next_pc);
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
    c.set_pc_const(&mut m, pc);
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
        let b = translate_block(&code_bytes(), 0x1000, 0x1000, JitLayout::bare()).unwrap();
        assert_eq!(b.n_insns, 6); // addi,addi,addi,add,addi,bne
        assert!(b.wasm.starts_with(&[0x00, 0x61, 0x73, 0x6d])); // \0asm
    }

    #[test]
    fn loop_body_block() {
        let b = translate_block(&code_bytes(), 0x1000, 0x100c, JitLayout::bare()).unwrap();
        assert_eq!(b.n_insns, 3); // add, addi, bne
    }

    #[test]
    fn ecall_not_translatable() {
        assert!(translate_block(&code_bytes(), 0x1000, 0x1018, JitLayout::bare()).is_none());
    }

    #[test]
    fn compressed_input_translates() {
        // c.li a0, 21 ; c.mv a1, a0 ; c.add a0, a1 ; ecall(32-bit)
        let mut code = Vec::new();
        for h in [0x4555u16, 0x85aa, 0x952e] {
            code.extend_from_slice(&h.to_le_bytes());
        }
        code.extend_from_slice(&0x0000_0073u32.to_le_bytes());
        let b = translate_block(&code, 0, 0, JitLayout::bare()).unwrap();
        assert_eq!(b.n_insns, 3);
        assert_eq!(b.len, 6);
    }
}
