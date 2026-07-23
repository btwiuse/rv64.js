use crate::bus::Bus;
use crate::decode::*;
use crate::exception::Exception;

/// Why `step`/`run` returned control to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Instruction budget exhausted; just call run() again.
    Budget,
    /// ECALL executed. User-mode: the host services a syscall and resumes.
    /// Full-system (later): routed to the trap vector instead of returned.
    Ecall,
    /// EBREAK executed.
    Break,
    /// An exception with no handler configured (phase 1: all of them).
    Trap(Exception),
}

/// RV64I hart state + interpreter.
///
/// Generic over [`Bus`] — the same execute code runs user-mode (flat memory)
/// and, later, full-system (MMU + MMIO). See /DESIGN.md.
pub struct Cpu {
    /// x0..x31; x0 reads as zero (enforced at write sites).
    pub x: [u64; 32],
    pub pc: u64,
    /// Retired instruction count (minstret / rdinstret).
    pub insn_count: u64,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self { x: [0; 32], pc: 0, insn_count: 0 }
    }

    #[inline]
    fn wr(&mut self, rd: usize, val: u64) {
        if rd != 0 {
            self.x[rd] = val;
        }
    }

    /// Run up to `budget` instructions; returns why we stopped.
    pub fn run<B: Bus>(&mut self, bus: &mut B, budget: u64) -> StopReason {
        for _ in 0..budget {
            match self.step(bus) {
                Ok(None) => {}
                Ok(Some(stop)) => return stop,
                Err(e) => return StopReason::Trap(e),
            }
        }
        StopReason::Budget
    }

    /// Execute one instruction. `Ok(Some(_))` = clean stop (ecall/ebreak),
    /// `Err` = exception. PC already points at the *next* instruction when
    /// Ecall/Break is returned, so the host can service and resume directly.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<Option<StopReason>, Exception> {
        let insn = bus.fetch32(self.pc)?;
        if insn & 3 != 3 {
            // Compressed (C extension) — phase 3. See decode.rs.
            return Err(Exception::IllegalInstruction { insn });
        }
        let mut next_pc = self.pc.wrapping_add(4);
        let mut stop = None;

        match opcode(insn) {
            // LUI
            0x37 => self.wr(rd(insn), imm_u(insn) as u64),
            // AUIPC
            0x17 => self.wr(rd(insn), self.pc.wrapping_add(imm_u(insn) as u64)),
            // JAL
            0x6f => {
                self.wr(rd(insn), next_pc);
                next_pc = self.pc.wrapping_add(imm_j(insn) as u64);
            }
            // JALR
            0x67 => {
                let target = self.x[rs1(insn)].wrapping_add(imm_i(insn) as u64) & !1;
                self.wr(rd(insn), next_pc);
                next_pc = target;
            }
            // BRANCH
            0x63 => {
                let (a, b) = (self.x[rs1(insn)], self.x[rs2(insn)]);
                let taken = match funct3(insn) {
                    0 => a == b,                     // BEQ
                    1 => a != b,                     // BNE
                    4 => (a as i64) < (b as i64),    // BLT
                    5 => (a as i64) >= (b as i64),   // BGE
                    6 => a < b,                      // BLTU
                    7 => a >= b,                     // BGEU
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                if taken {
                    next_pc = self.pc.wrapping_add(imm_b(insn) as u64);
                }
            }
            // LOAD
            0x03 => {
                let addr = self.x[rs1(insn)].wrapping_add(imm_i(insn) as u64);
                let val = match funct3(insn) {
                    0 => bus.read8(addr)? as i8 as i64 as u64,   // LB
                    1 => bus.read16(addr)? as i16 as i64 as u64, // LH
                    2 => bus.read32(addr)? as i32 as i64 as u64, // LW
                    3 => bus.read64(addr)?,                      // LD
                    4 => bus.read8(addr)? as u64,                // LBU
                    5 => bus.read16(addr)? as u64,               // LHU
                    6 => bus.read32(addr)? as u64,               // LWU
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val);
            }
            // STORE
            0x23 => {
                let addr = self.x[rs1(insn)].wrapping_add(imm_s(insn) as u64);
                let val = self.x[rs2(insn)];
                match funct3(insn) {
                    0 => bus.write8(addr, val as u8)?,   // SB
                    1 => bus.write16(addr, val as u16)?, // SH
                    2 => bus.write32(addr, val as u32)?, // SW
                    3 => bus.write64(addr, val)?,        // SD
                    _ => return Err(Exception::IllegalInstruction { insn }),
                }
            }
            // OP-IMM
            0x13 => {
                let a = self.x[rs1(insn)];
                let imm = imm_i(insn) as u64;
                let shamt = (imm & 0x3f) as u32;
                let val = match funct3(insn) {
                    0 => a.wrapping_add(imm),                              // ADDI
                    1 => a << shamt,                                       // SLLI
                    2 => ((a as i64) < (imm as i64)) as u64,               // SLTI
                    3 => (a < imm) as u64,                                 // SLTIU
                    4 => a ^ imm,                                          // XORI
                    5 => {
                        if insn >> 26 == 0x10 {
                            ((a as i64) >> shamt) as u64                   // SRAI
                        } else {
                            a >> shamt                                     // SRLI
                        }
                    }
                    6 => a | imm,                                          // ORI
                    7 => a & imm,                                          // ANDI
                    _ => unreachable!(),
                };
                self.wr(rd(insn), val);
            }
            // OP-IMM-32 (ADDIW/SLLIW/SRLIW/SRAIW)
            0x1b => {
                let a = self.x[rs1(insn)] as u32;
                let imm = imm_i(insn);
                let shamt = (imm & 0x1f) as u32;
                let val32 = match funct3(insn) {
                    0 => a.wrapping_add(imm as u32),
                    1 => a << shamt,
                    5 => {
                        if funct7(insn) == 0x20 {
                            ((a as i32) >> shamt) as u32
                        } else {
                            a >> shamt
                        }
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val32 as i32 as i64 as u64);
            }
            // OP
            0x33 => {
                let (a, b) = (self.x[rs1(insn)], self.x[rs2(insn)]);
                let shamt = (b & 0x3f) as u32;
                let val = match (funct7(insn), funct3(insn)) {
                    (0x00, 0) => a.wrapping_add(b),                  // ADD
                    (0x20, 0) => a.wrapping_sub(b),                  // SUB
                    (0x00, 1) => a << shamt,                         // SLL
                    (0x00, 2) => ((a as i64) < (b as i64)) as u64,   // SLT
                    (0x00, 3) => (a < b) as u64,                     // SLTU
                    (0x00, 4) => a ^ b,                              // XOR
                    (0x00, 5) => a >> shamt,                         // SRL
                    (0x20, 5) => ((a as i64) >> shamt) as u64,       // SRA
                    (0x00, 6) => a | b,                              // OR
                    (0x00, 7) => a & b,                              // AND
                    // funct7=0x01 is the M extension (phase 3)
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val);
            }
            // OP-32 (ADDW/SUBW/SLLW/SRLW/SRAW)
            0x3b => {
                let (a, b) = (self.x[rs1(insn)] as u32, self.x[rs2(insn)] as u32);
                let shamt = b & 0x1f;
                let val32 = match (funct7(insn), funct3(insn)) {
                    (0x00, 0) => a.wrapping_add(b),
                    (0x20, 0) => a.wrapping_sub(b),
                    (0x00, 1) => a << shamt,
                    (0x00, 5) => a >> shamt,
                    (0x20, 5) => ((a as i32) >> shamt) as u32,
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val32 as i32 as i64 as u64);
            }
            // MISC-MEM: FENCE/FENCE.I — no-ops for a single in-order hart
            0x0f => {}
            // SYSTEM
            0x73 => match insn {
                0x0000_0073 => stop = Some(StopReason::Ecall),
                0x0010_0073 => stop = Some(StopReason::Break),
                // CSR instructions arrive with the privileged arch (phase 4)
                _ => return Err(Exception::IllegalInstruction { insn }),
            },
            _ => return Err(Exception::IllegalInstruction { insn }),
        }

        self.pc = next_pc;
        self.insn_count += 1;
        Ok(stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatMemory;

    const BASE: u64 = 0x1000;

    fn run_program(words: &[u32]) -> (Cpu, Vec<u8>) {
        let mut mem = vec![0u8; 0x10000];
        for (i, w) in words.iter().enumerate() {
            mem[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let mut cpu = Cpu::new();
        cpu.pc = BASE;
        let mut bus = FlatMemory::new(BASE, &mut mem);
        let stop = cpu.run(&mut bus, 10_000);
        assert_eq!(stop, StopReason::Ecall, "program should end in ecall");
        (cpu, mem)
    }

    #[test]
    fn addi_add_sub() {
        // addi x1, x0, 5 ; addi x2, x0, 7 ; add x3, x1, x2 ; sub x4, x2, x1 ; ecall
        let (cpu, _) = run_program(&[0x00500093, 0x00700113, 0x002081b3, 0x40110233, 0x00000073]);
        assert_eq!(cpu.x[3], 12);
        assert_eq!(cpu.x[4], 2);
    }

    #[test]
    fn x0_is_hardwired_zero() {
        // addi x0, x0, 42 ; ecall
        let (cpu, _) = run_program(&[0x02a00013, 0x00000073]);
        assert_eq!(cpu.x[0], 0);
    }

    #[test]
    fn lui_auipc() {
        // lui x1, 0x12345 ; auipc x2, 0 ; ecall
        let (cpu, _) = run_program(&[0x12345_0b7, 0x0000_0117, 0x00000073]);
        assert_eq!(cpu.x[1], 0x12345000);
        assert_eq!(cpu.x[2], BASE + 4);
    }

    #[test]
    fn negative_immediate_sign_extends() {
        // addi x1, x0, -1 ; ecall
        let (cpu, _) = run_program(&[0xfff00093, 0x00000073]);
        assert_eq!(cpu.x[1], u64::MAX);
    }

    #[test]
    fn loads_stores_roundtrip() {
        let (cpu, _) = run_program(&[
            0xffe00093,  // addi x1, x0, -2
            0x000012b7,  // lui x5, 0x1  (x5 = 0x1000 = BASE)
            0x10129023,  // sh x1, 0x100(x5)
            0x1012b423,  // sd x1, 0x108(x5)
            0x1082b103,  // ld x2, 0x108(x5)
            0x1082a183,  // lw x3, 0x108(x5)
            0x1082c203,  // lbu x4, 0x108(x5)
            0x00000073,  // ecall
        ]);
        assert_eq!(cpu.x[2], (-2i64) as u64);
        assert_eq!(cpu.x[3], (-2i64) as u64); // lw sign-extends
        assert_eq!(cpu.x[4], 0xfe); // lbu zero-extends
    }

    #[test]
    fn branch_loop_sums_1_to_10() {
        // x1 = 0 (sum), x2 = 1 (i), x3 = 11 (limit)
        // loop: add x1, x1, x2 ; addi x2, x2, 1 ; bne x2, x3, loop ; ecall
        let (cpu, _) = run_program(&[
            0x00000093, // addi x1, x0, 0
            0x00100113, // addi x2, x0, 1
            0x00b00193, // addi x3, x0, 11
            0x002080b3, // add x1, x1, x2
            0x00110113, // addi x2, x2, 1
            0xfe311ce3, // bne x2, x3, -8
            0x00000073, // ecall
        ]);
        assert_eq!(cpu.x[1], 55);
    }

    #[test]
    fn jal_jalr_link() {
        // jal x1, +8 ; ecall(skipped) ; jalr x0, 0(x1) -> lands on ecall
        let (cpu, _) = run_program(&[
            0x008000ef, // jal x1, +8
            0x00000073, // ecall (return target)
            0x00008067, // jalr x0, 0(x1)
        ]);
        assert_eq!(cpu.x[1], BASE + 4);
        assert_eq!(cpu.pc, BASE + 8); // pc after the ecall at BASE+4
    }

    #[test]
    fn illegal_instruction_traps() {
        let mut mem = vec![0u8; 0x100];
        let mut cpu = Cpu::new();
        cpu.pc = 0;
        let mut bus = FlatMemory::new(0, &mut mem);
        // all-zero word is defined illegal in RISC-V
        match cpu.run(&mut bus, 10) {
            StopReason::Trap(Exception::IllegalInstruction { .. }) => {}
            other => panic!("expected illegal instruction, got {other:?}"),
        }
    }
}
