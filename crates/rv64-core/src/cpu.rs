use crate::bus::Bus;
use crate::csr::*;
use crate::decode::*;
use crate::exception::Exception;

/// Why `step`/`run` returned control to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Instruction budget exhausted; just call run() again.
    Budget,
    /// ECALL executed (user-mode emulation only: the host services a
    /// syscall and resumes; full-system routes ecall to the guest kernel).
    Ecall,
    /// EBREAK executed (user-mode emulation only).
    Break,
    /// An exception with no handler configured (user-mode emulation only).
    Trap(Exception),
    /// WFI with no pending interrupt (full-system only): host may idle.
    Wfi,
}

/// Memory access type, for translation and fault selection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Fetch,
    Load,
    Store,
}

const TLB_BITS: u32 = 8;
const TLB_SIZE: usize = 1 << TLB_BITS;
const TLB_INVALID: u64 = !0;

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
    /// LR/SC reservation address (A extension); None = no reservation.
    pub reservation: Option<u64>,
    /// f0..f31 (F/D extensions). f32 values are NaN-boxed in the low bits.
    pub f: [u64; 32],
    /// fcsr: fflags[4:0] | frm[7:5].
    pub fcsr: u32,
    /// Privileged state; None = pure user-mode emulation (no MMU/traps).
    pub sys: Option<SysCsrs>,
    /// Diagnostics: exception counts by cause, interrupt counts by cause.
    pub exc_counts: [u64; 16],
    pub irq_counts: [u64; 16],
    // Direct-mapped TLBs (virtual page tag -> pa-va diff), one per access
    // type so permission bits never need re-checking on a hit.
    tlb_tag: [[u64; TLB_SIZE]; 3],
    tlb_diff: [[u64; TLB_SIZE]; 3],
}

/// NaN-box an f32 into a 64-bit F register (high 32 bits all-ones).
#[inline]
fn box32(v: f32) -> u64 {
    0xffff_ffff_0000_0000 | v.to_bits() as u64
}

/// Unbox an f32; improperly boxed values read as the canonical NaN.
#[inline]
fn unbox32(r: u64) -> f32 {
    if r >> 32 == 0xffff_ffff {
        f32::from_bits(r as u32)
    } else {
        f32::NAN
    }
}

/// FCLASS result bits (same layout for f32/f64).
fn fclass(
    sign: bool,
    is_inf: bool,
    is_nan: bool,
    is_snan: bool,
    is_zero: bool,
    is_sub: bool,
) -> u64 {
    if is_nan {
        return if is_snan { 1 << 8 } else { 1 << 9 };
    }
    match (sign, is_inf, is_zero, is_sub) {
        (true, true, _, _) => 1 << 0,  // -inf
        (true, _, true, _) => 1 << 3,  // -0
        (true, _, _, true) => 1 << 2,  // negative subnormal
        (true, _, _, _) => 1 << 1,     // negative normal
        (false, true, _, _) => 1 << 7, // +inf
        (false, _, true, _) => 1 << 4, // +0
        (false, _, _, true) => 1 << 5, // positive subnormal
        (false, _, _, _) => 1 << 6,    // positive normal
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            x: [0; 32],
            pc: 0,
            insn_count: 0,
            reservation: None,
            f: [0; 32],
            fcsr: 0,
            sys: None,
            exc_counts: [0; 16],
            irq_counts: [0; 16],
            tlb_tag: [[TLB_INVALID; TLB_SIZE]; 3],
            tlb_diff: [[0; TLB_SIZE]; 3],
        }
    }

    /// Enable full-system mode: M/S/U privileges, MMU, traps. The hart
    /// resets to M-mode at `pc` with a0=hartid, a1=dtb (set by caller).
    pub fn enable_system(&mut self, hartid: u64) {
        let mut sys = SysCsrs::new();
        sys.mhartid = hartid;
        self.sys = Some(sys);
    }

    pub fn flush_tlb(&mut self) {
        self.tlb_tag = [[TLB_INVALID; TLB_SIZE]; 3];
    }

    /// Translate a fetch address without raising a fault (JIT support:
    /// verify that a va-keyed compiled block still maps to the same
    /// physical code before dispatching to it).
    pub fn jit_probe_fetch<B: Bus>(&mut self, bus: &mut B, va: u64) -> Option<u64> {
        self.translate(bus, va, Access::Fetch).ok()
    }

    #[inline]
    fn wr(&mut self, rd: usize, val: u64) {
        if rd != 0 {
            self.x[rd] = val;
        }
    }

    /// FP instructions are illegal while mstatus.FS = Off (system mode).
    #[inline]
    fn fp_check(&self, insn: u32) -> Result<(), Exception> {
        if let Some(sys) = &self.sys {
            if sys.mstatus & MSTATUS_FS == 0 {
                return Err(Exception::IllegalInstruction { insn });
            }
        }
        Ok(())
    }

    /// Mark FP state dirty (mstatus.FS = 11) after FP execution.
    #[inline]
    fn fp_dirty(&mut self) {
        if let Some(sys) = &mut self.sys {
            sys.mstatus |= MSTATUS_FS;
        }
    }

    // ---- address translation --------------------------------------------

    #[inline]
    fn fault(access: Access, addr: u64) -> Exception {
        match access {
            Access::Fetch => Exception::InstructionPageFault { addr },
            Access::Load => Exception::LoadPageFault { addr },
            Access::Store => Exception::StorePageFault { addr },
        }
    }

    /// Effective privilege for data accesses (MPRV) or fetch.
    fn eff_mode(&self, access: Access) -> Mode {
        let sys = self.sys.as_ref().unwrap();
        if access != Access::Fetch && sys.mstatus & MSTATUS_MPRV != 0 {
            Mode::from_bits((sys.mstatus & MSTATUS_MPP) >> 11)
        } else {
            sys.mode
        }
    }

    /// Translate a virtual address (full-system mode). Hot path: TLB hit.
    #[inline]
    fn translate<B: Bus>(
        &mut self,
        bus: &mut B,
        va: u64,
        access: Access,
    ) -> Result<u64, Exception> {
        if self.sys.is_none() {
            return Ok(va);
        }
        let idx = ((va >> 12) as usize) & (TLB_SIZE - 1);
        let tag = va >> 12;
        let a = access as usize;
        if self.tlb_tag[a][idx] == tag {
            return Ok(va.wrapping_add(self.tlb_diff[a][idx]));
        }
        self.translate_slow(bus, va, access)
    }

    /// Page-table walk (sv39/sv48), permission checks, A/D update, TLB fill.
    fn translate_slow<B: Bus>(
        &mut self,
        bus: &mut B,
        va: u64,
        access: Access,
    ) -> Result<u64, Exception> {
        let sys = self.sys.as_ref().unwrap();
        let mode = self.eff_mode(access);
        let satp = sys.satp;
        let vm = satp >> 60;

        // Bare, or M-mode without MPRV redirection: identity.
        if vm == 0 || mode == Mode::Machine {
            return Ok(va);
        }
        let levels: i32 = match vm {
            8 => 3, // sv39
            9 => 4, // sv48
            _ => return Err(Self::fault(access, va)),
        };
        // Canonical check: high bits must equal bit (9*levels + 12 - 1).
        let va_bits = 9 * levels as u32 + 12;
        let ext = (va as i64) >> (va_bits - 1);
        if ext != 0 && ext != -1 {
            return Err(Self::fault(access, va));
        }

        let sum = sys.mstatus & MSTATUS_SUM != 0;
        let mxr = sys.mstatus & MSTATUS_MXR != 0;

        let mut table = (satp & 0xfff_ffff_ffff) << 12; // PPN
        let mut level = levels - 1;
        loop {
            let vpn = (va >> (12 + 9 * level as u32)) & 0x1ff;
            let pte_addr = table + vpn * 8;
            let pte = bus.read64(pte_addr).map_err(|_| Self::fault(access, va))?;
            let v = pte & 1;
            let r = pte >> 1 & 1;
            let w = pte >> 2 & 1;
            let x = pte >> 3 & 1;
            if v == 0 || (r == 0 && w == 1) {
                return Err(Self::fault(access, va));
            }
            if r == 0 && x == 0 {
                // pointer to next level
                if level == 0 {
                    return Err(Self::fault(access, va));
                }
                table = (pte >> 10) << 12;
                level -= 1;
                continue;
            }
            // Leaf. Check alignment of superpages.
            let ppn = pte >> 10;
            if level > 0 && (ppn & ((1 << (9 * level as u32)) - 1)) != 0 {
                return Err(Self::fault(access, va));
            }
            // Permission checks.
            let u = pte >> 4 & 1 != 0;
            match mode {
                Mode::User if !u => return Err(Self::fault(access, va)),
                Mode::Supervisor if u && !(sum && access != Access::Fetch) => {
                    return Err(Self::fault(access, va))
                }
                _ => {}
            }
            let ok = match access {
                Access::Fetch => x == 1,
                Access::Load => r == 1 || (mxr && x == 1),
                Access::Store => w == 1,
            };
            if !ok {
                return Err(Self::fault(access, va));
            }
            // A/D update (hardware-managed, like TinyEMU).
            let mut new_pte = pte | 1 << 6; // A
            if access == Access::Store {
                new_pte |= 1 << 7; // D
            }
            if new_pte != pte {
                bus.write64(pte_addr, new_pte)
                    .map_err(|_| Self::fault(access, va))?;
            }
            // Physical address: superpage low VPN bits come from va.
            let mask = (1u64 << (12 + 9 * level as u32)) - 1;
            let pa = ((ppn << 12) & !mask) | (va & mask);

            // Fill TLB (only 4K granularity; superpages fill one entry).
            // Don't cache Load entries whose D bit isn't set for stores etc.
            let idx = ((va >> 12) as usize) & (TLB_SIZE - 1);
            let a = access as usize;
            self.tlb_tag[a][idx] = va >> 12;
            self.tlb_diff[a][idx] = pa.wrapping_sub(va);
            return Ok(pa);
        }
    }

    // ---- memory accessors (virtual in full-system, direct otherwise) -----

    #[inline]
    fn ld<B: Bus, const N: u32>(&mut self, bus: &mut B, va: u64) -> Result<u64, Exception> {
        // Split accesses that cross a page boundary (two translations).
        if self.sys.is_some() && (va & 0xfff) + N as u64 > 0x1000 {
            let mut v: u64 = 0;
            for i in 0..N as u64 {
                let pa = self.translate(bus, va + i, Access::Load)?;
                v |= (bus.read8(pa)? as u64) << (8 * i);
            }
            return Ok(v);
        }
        let pa = self.translate(bus, va, Access::Load)?;
        match N {
            1 => bus.read8(pa).map(|v| v as u64),
            2 => bus.read16(pa).map(|v| v as u64),
            4 => bus.read32(pa).map(|v| v as u64),
            _ => bus.read64(pa),
        }
    }

    #[inline]
    fn st<B: Bus, const N: u32>(
        &mut self,
        bus: &mut B,
        va: u64,
        val: u64,
    ) -> Result<(), Exception> {
        if self.sys.is_some() && (va & 0xfff) + N as u64 > 0x1000 {
            for i in 0..N as u64 {
                let pa = self.translate(bus, va + i, Access::Store)?;
                bus.write8(pa, (val >> (8 * i)) as u8)?;
            }
            return Ok(());
        }
        let pa = self.translate(bus, va, Access::Store)?;
        match N {
            1 => bus.write8(pa, val as u8),
            2 => bus.write16(pa, val as u16),
            4 => bus.write32(pa, val as u32),
            _ => bus.write64(pa, val),
        }
    }

    // ---- traps ------------------------------------------------------------

    /// Enter the trap handler for an exception or interrupt.
    pub fn take_trap(&mut self, cause: u64, tval: u64, is_interrupt: bool) {
        let c = (cause & 15) as usize;
        if is_interrupt {
            self.irq_counts[c] += 1;
        } else {
            self.exc_counts[c] += 1;
        }
        let sys = self.sys.as_mut().unwrap();
        let deleg = if is_interrupt {
            sys.mideleg
        } else {
            sys.medeleg
        };
        let bit = 1u64 << (cause & 63);
        let to_s = sys.mode != Mode::Machine && (deleg & bit) != 0;

        let cause_val = if is_interrupt {
            (1 << 63) | cause
        } else {
            cause
        };
        if to_s {
            sys.scause = cause_val;
            sys.stval = tval;
            sys.sepc = self.pc;
            // SPIE = SIE; SIE = 0; SPP = prev
            let sie = (sys.mstatus >> 1) & 1;
            sys.mstatus = (sys.mstatus & !(MSTATUS_SPIE | MSTATUS_SPP | MSTATUS_SIE))
                | (sie << 5)
                | (if sys.mode == Mode::Supervisor {
                    MSTATUS_SPP
                } else {
                    0
                });
            sys.mode = Mode::Supervisor;
            let base = sys.stvec & !3;
            self.pc = if sys.stvec & 3 == 1 && is_interrupt {
                base + 4 * cause
            } else {
                base
            };
        } else {
            sys.mcause = cause_val;
            sys.mtval = tval;
            sys.mepc = self.pc;
            let mie = (sys.mstatus >> 3) & 1;
            sys.mstatus = (sys.mstatus & !(MSTATUS_MPIE | MSTATUS_MPP | MSTATUS_MIE))
                | (mie << 7)
                | ((sys.mode as u64) << 11);
            sys.mode = Mode::Machine;
            let base = sys.mtvec & !3;
            self.pc = if sys.mtvec & 3 == 1 && is_interrupt {
                base + 4 * cause
            } else {
                base
            };
        }
        self.flush_tlb(); // privilege changed
    }

    fn exception_to_trap(&mut self, e: Exception) {
        let (cause, tval) = match e {
            Exception::InstructionAddressMisaligned { addr } => (0, addr),
            Exception::InstructionAccessFault { addr } => (1, addr),
            Exception::IllegalInstruction { insn } => (2, insn as u64),
            Exception::Breakpoint => (3, self.pc),
            Exception::LoadAddressMisaligned { addr } => (4, addr),
            Exception::LoadAccessFault { addr } => (5, addr),
            Exception::StoreAddressMisaligned { addr } => (6, addr),
            Exception::StoreAccessFault { addr } => (7, addr),
            Exception::EnvironmentCallFromUMode => (8, 0),
            Exception::EnvironmentCallFromSMode => (9, 0),
            Exception::EnvironmentCallFromMMode => (11, 0),
            Exception::InstructionPageFault { addr } => (12, addr),
            Exception::LoadPageFault { addr } => (13, addr),
            Exception::StorePageFault { addr } => (15, addr),
        };
        self.take_trap(cause, tval, false);
    }

    /// Check for a deliverable interrupt; take the highest-priority one.
    /// Returns true if a trap was taken. Hardware lines (timer/external)
    /// come live from the bus; only software bits live in sys.mip.
    pub fn check_interrupts<B: Bus>(&mut self, bus: &mut B) -> bool {
        let Some(sys) = self.sys.as_mut() else {
            return false;
        };
        const HW: u64 = IRQ_MTIP | IRQ_MSIP | IRQ_MEIP | IRQ_SEIP;
        sys.mip = (sys.mip & !HW) | (bus.irq_lines() & HW);
        let sys = self.sys.as_ref().unwrap();
        let pending = sys.mip & sys.mie;
        if pending == 0 {
            return false;
        }
        let mideleg = sys.mideleg;
        let m_enabled = sys.mode != Mode::Machine || (sys.mstatus & MSTATUS_MIE) != 0;
        let s_enabled = sys.mode == Mode::User
            || (sys.mode == Mode::Supervisor && (sys.mstatus & MSTATUS_SIE) != 0);

        // Priority: MEI, MSI, MTI, SEI, SSI, STI.
        for &irq in &[11u64, 3, 7, 9, 1, 5] {
            let bit = 1u64 << irq;
            if pending & bit == 0 {
                continue;
            }
            let target_s = mideleg & bit != 0;
            let deliverable = if target_s {
                // S-target: fires when we're below S, or in S with SIE.
                sys.mode == Mode::User || (sys.mode == Mode::Supervisor && s_enabled)
            } else {
                // M-target: fires when below M, or in M with MIE.
                sys.mode != Mode::Machine || m_enabled
            };
            if deliverable {
                self.take_trap(irq, 0, true);
                return true;
            }
        }
        false
    }

    /// Run up to `budget` instructions; returns why we stopped.
    pub fn run<B: Bus>(&mut self, bus: &mut B, budget: u64) -> StopReason {
        let system = self.sys.is_some();
        for _ in 0..budget {
            if system {
                self.check_interrupts(bus);
            }
            match self.step(bus) {
                Ok(None) => {}
                Ok(Some(stop)) => return stop,
                Err(e) => {
                    if system {
                        self.exception_to_trap(e);
                    } else {
                        return StopReason::Trap(e);
                    }
                }
            }
        }
        StopReason::Budget
    }

    /// Execute one instruction. `Ok(Some(_))` = clean stop (ecall/ebreak),
    /// `Err` = exception. PC already points at the *next* instruction when
    /// Ecall/Break is returned, so the host can service and resume directly.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<Option<StopReason>, Exception> {
        // Fetch 16 bits first: a compressed instruction may sit on the last
        // halfword of a page/region, where a blind 32-bit fetch would fault.
        let pa = self.translate(bus, self.pc, Access::Fetch)?;
        let lo = bus.fetch16(pa)? as u32;
        let (insn, ilen) = if lo & 3 == 3 {
            let pc2 = self.pc.wrapping_add(2);
            let pa2 = if pc2 & 0xfff == 0 {
                self.translate(bus, pc2, Access::Fetch)?
            } else {
                pa + 2
            };
            let hi = bus.fetch16(pa2)? as u32;
            (lo | (hi << 16), 4)
        } else {
            let exp = crate::compressed::expand(lo as u16)
                .ok_or(Exception::IllegalInstruction { insn: lo })?;
            (exp, 2)
        };
        let mut next_pc = self.pc.wrapping_add(ilen);
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
                    0 => a == b,                   // BEQ
                    1 => a != b,                   // BNE
                    4 => (a as i64) < (b as i64),  // BLT
                    5 => (a as i64) >= (b as i64), // BGE
                    6 => a < b,                    // BLTU
                    7 => a >= b,                   // BGEU
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
                    0 => self.ld::<B, 1>(bus, addr)? as i8 as i64 as u64, // LB
                    1 => self.ld::<B, 2>(bus, addr)? as i16 as i64 as u64, // LH
                    2 => self.ld::<B, 4>(bus, addr)? as i32 as i64 as u64, // LW
                    3 => self.ld::<B, 8>(bus, addr)?,                     // LD
                    4 => self.ld::<B, 1>(bus, addr)?,                     // LBU
                    5 => self.ld::<B, 2>(bus, addr)?,                     // LHU
                    6 => self.ld::<B, 4>(bus, addr)?,                     // LWU
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val);
            }
            // STORE
            0x23 => {
                let addr = self.x[rs1(insn)].wrapping_add(imm_s(insn) as u64);
                let val = self.x[rs2(insn)];
                match funct3(insn) {
                    0 => self.st::<B, 1>(bus, addr, val)?, // SB
                    1 => self.st::<B, 2>(bus, addr, val)?, // SH
                    2 => self.st::<B, 4>(bus, addr, val)?, // SW
                    3 => self.st::<B, 8>(bus, addr, val)?, // SD
                    _ => return Err(Exception::IllegalInstruction { insn }),
                }
            }
            // OP-IMM
            0x13 => {
                let a = self.x[rs1(insn)];
                let imm = imm_i(insn) as u64;
                let shamt = (imm & 0x3f) as u32;
                let val = match funct3(insn) {
                    0 => a.wrapping_add(imm),                // ADDI
                    1 => a << shamt,                         // SLLI
                    2 => ((a as i64) < (imm as i64)) as u64, // SLTI
                    3 => (a < imm) as u64,                   // SLTIU
                    4 => a ^ imm,                            // XORI
                    5 => {
                        if insn >> 26 == 0x10 {
                            ((a as i64) >> shamt) as u64 // SRAI
                        } else {
                            a >> shamt // SRLI
                        }
                    }
                    6 => a | imm, // ORI
                    7 => a & imm, // ANDI
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
                    (0x00, 0) => a.wrapping_add(b),                // ADD
                    (0x20, 0) => a.wrapping_sub(b),                // SUB
                    (0x00, 1) => a << shamt,                       // SLL
                    (0x00, 2) => ((a as i64) < (b as i64)) as u64, // SLT
                    (0x00, 3) => (a < b) as u64,                   // SLTU
                    (0x00, 4) => a ^ b,                            // XOR
                    (0x00, 5) => a >> shamt,                       // SRL
                    (0x20, 5) => ((a as i64) >> shamt) as u64,     // SRA
                    (0x00, 6) => a | b,                            // OR
                    (0x00, 7) => a & b,                            // AND
                    // M extension
                    (0x01, 0) => a.wrapping_mul(b), // MUL
                    (0x01, 1) => {
                        (((a as i64 as i128) * (b as i64 as i128)) >> 64) as u64
                        // MULH
                    }
                    (0x01, 2) => {
                        (((a as i64 as i128) * (b as u128 as i128)) >> 64) as u64
                        // MULHSU
                    }
                    (0x01, 3) => (((a as u128) * (b as u128)) >> 64) as u64, // MULHU
                    (0x01, 4) => {
                        // DIV: div-by-zero -> -1; overflow MIN/-1 -> MIN
                        let (a, b) = (a as i64, b as i64);
                        if b == 0 {
                            u64::MAX
                        } else {
                            a.wrapping_div(b) as u64
                        }
                    }
                    (0x01, 5) => {
                        if b == 0 {
                            u64::MAX
                        } else {
                            a / b
                        } // DIVU
                    }
                    (0x01, 6) => {
                        let (a, b) = (a as i64, b as i64);
                        if b == 0 {
                            a as u64
                        } else {
                            a.wrapping_rem(b) as u64
                        } // REM
                    }
                    (0x01, 7) => {
                        if b == 0 {
                            a
                        } else {
                            a % b
                        } // REMU
                    }
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
                    // M extension (32-bit forms)
                    (0x01, 0) => a.wrapping_mul(b), // MULW
                    (0x01, 4) => {
                        let (a, b) = (a as i32, b as i32);
                        if b == 0 {
                            u32::MAX
                        } else {
                            a.wrapping_div(b) as u32
                        } // DIVW
                    }
                    (0x01, 5) => {
                        if b == 0 {
                            u32::MAX
                        } else {
                            a / b
                        } // DIVUW
                    }
                    (0x01, 6) => {
                        let (a, b) = (a as i32, b as i32);
                        if b == 0 {
                            a as u32
                        } else {
                            a.wrapping_rem(b) as u32
                        } // REMW
                    }
                    (0x01, 7) => {
                        if b == 0 {
                            a
                        } else {
                            a % b
                        } // REMUW
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                self.wr(rd(insn), val32 as i32 as i64 as u64);
            }
            // AMO (A extension). Single hart: LR sets a reservation, SC
            // succeeds iff it matches; AMOs are read-modify-write.
            0x2f => {
                let addr = self.x[rs1(insn)];
                let src = self.x[rs2(insn)];
                let funct5 = funct7(insn) >> 2;
                let is64 = match funct3(insn) {
                    2 => false,
                    3 => true,
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
                macro_rules! aload {
                    () => {
                        if is64 {
                            self.ld::<B, 8>(bus, addr)?
                        } else {
                            self.ld::<B, 4>(bus, addr)? as i32 as i64 as u64
                        }
                    };
                }
                macro_rules! astore {
                    ($v:expr) => {
                        if is64 {
                            self.st::<B, 8>(bus, addr, $v)?
                        } else {
                            self.st::<B, 4>(bus, addr, $v)?
                        }
                    };
                }
                match funct5 {
                    0x02 => {
                        // LR
                        let v = aload!();
                        self.reservation = Some(addr);
                        self.wr(rd(insn), v);
                    }
                    0x03 => {
                        // SC
                        if self.reservation == Some(addr) {
                            astore!(src);
                            self.wr(rd(insn), 0);
                        } else {
                            self.wr(rd(insn), 1);
                        }
                        self.reservation = None;
                    }
                    _ => {
                        let old = aload!();
                        // 32-bit AMOs compare/compute on the low 32 bits
                        // only (the register's high bits are ignored).
                        let (co, cs) = if is64 {
                            (old, src)
                        } else {
                            (old as u32 as u64, src as u32 as u64)
                        };
                        let signed_lt = if is64 {
                            (co as i64) < (cs as i64)
                        } else {
                            (co as u32 as i32) < (cs as u32 as i32)
                        };
                        let new = match funct5 {
                            0x01 => src,                   // AMOSWAP
                            0x00 => old.wrapping_add(src), // AMOADD
                            0x04 => old ^ src,             // AMOXOR
                            0x0c => old & src,             // AMOAND
                            0x08 => old | src,             // AMOOR
                            0x10 => {
                                if signed_lt {
                                    old
                                } else {
                                    src
                                }
                            } // AMOMIN
                            0x14 => {
                                if !signed_lt && co != cs {
                                    old
                                } else {
                                    src
                                }
                            } // AMOMAX
                            0x18 => {
                                if co < cs {
                                    old
                                } else {
                                    src
                                }
                            } // AMOMINU
                            0x1c => {
                                if co > cs {
                                    old
                                } else {
                                    src
                                }
                            } // AMOMAXU
                            _ => return Err(Exception::IllegalInstruction { insn }),
                        };
                        // 32-bit AMOs operate on the sign-extended old value
                        // but store only the low 32 bits.
                        astore!(new);
                        self.wr(rd(insn), old);
                    }
                }
            }
            // LOAD-FP (FLW/FLD)
            0x07 => {
                self.fp_check(insn)?;
                self.fp_dirty();
                let addr = self.x[rs1(insn)].wrapping_add(imm_i(insn) as u64);
                self.f[rd(insn)] = match funct3(insn) {
                    2 => box32(f32::from_bits(self.ld::<B, 4>(bus, addr)? as u32)),
                    3 => self.ld::<B, 8>(bus, addr)?,
                    _ => return Err(Exception::IllegalInstruction { insn }),
                };
            }
            // STORE-FP (FSW/FSD)
            0x27 => {
                self.fp_check(insn)?;
                let addr = self.x[rs1(insn)].wrapping_add(imm_s(insn) as u64);
                let v = self.f[rs2(insn)];
                match funct3(insn) {
                    2 => self.st::<B, 4>(bus, addr, v)?,
                    3 => self.st::<B, 8>(bus, addr, v)?,
                    _ => return Err(Exception::IllegalInstruction { insn }),
                }
            }
            // FMADD/FMSUB/FNMSUB/FNMADD
            0x43 | 0x47 | 0x4b | 0x4f => {
                self.fp_check(insn)?;
                self.fp_dirty();
                let rs3 = (insn >> 27) as usize;
                let neg_prod = opcode(insn) == 0x4b || opcode(insn) == 0x4f;
                let neg_c = opcode(insn) == 0x47 || opcode(insn) == 0x4f;
                match (insn >> 25) & 3 {
                    0 => {
                        let (a, b, mut c) = (
                            unbox32(self.f[rs1(insn)]),
                            unbox32(self.f[rs2(insn)]),
                            unbox32(self.f[rs3]),
                        );
                        let a = if neg_prod { -a } else { a };
                        if neg_c {
                            c = -c;
                        }
                        self.f[rd(insn)] = box32(libm::fmaf(a, b, c));
                    }
                    1 => {
                        let (a, b, mut c) = (
                            f64::from_bits(self.f[rs1(insn)]),
                            f64::from_bits(self.f[rs2(insn)]),
                            f64::from_bits(self.f[rs3]),
                        );
                        let a = if neg_prod { -a } else { a };
                        if neg_c {
                            c = -c;
                        }
                        self.f[rd(insn)] = libm::fma(a, b, c).to_bits();
                    }
                    _ => return Err(Exception::IllegalInstruction { insn }),
                }
            }
            // OP-FP
            0x53 => {
                self.fp_check(insn)?;
                self.fp_dirty();
                self.op_fp(insn)?
            }
            // MISC-MEM: FENCE/FENCE.I — no-ops for a single in-order hart
            0x0f => {}
            // SYSTEM
            0x73 => match (insn, funct3(insn)) {
                (0x0000_0073, _) => {
                    if let Some(sys) = self.sys.as_ref() {
                        let cause = match sys.mode {
                            Mode::User => 8,
                            Mode::Supervisor => 9,
                            Mode::Machine => 11,
                        };
                        self.take_trap(cause, 0, false);
                        self.insn_count += 1;
                        return Ok(None); // pc set by take_trap
                    }
                    stop = Some(StopReason::Ecall);
                }
                (0x0010_0073, _) => {
                    if self.sys.is_some() {
                        return Err(Exception::Breakpoint); // routed to trap
                    }
                    stop = Some(StopReason::Break);
                }
                // MRET
                (0x3020_0073, _) => {
                    let sys = self
                        .sys
                        .as_mut()
                        .ok_or(Exception::IllegalInstruction { insn })?;
                    if sys.mode != Mode::Machine {
                        return Err(Exception::IllegalInstruction { insn });
                    }
                    let mpp = Mode::from_bits((sys.mstatus & MSTATUS_MPP) >> 11);
                    let mpie = (sys.mstatus >> 7) & 1;
                    sys.mstatus = (sys.mstatus & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP))
                        | (mpie << 3)
                        | MSTATUS_MPIE;
                    if mpp != Mode::Machine {
                        sys.mstatus &= !MSTATUS_MPRV;
                    }
                    sys.mode = mpp;
                    next_pc = sys.mepc;
                    self.flush_tlb();
                }
                // SRET
                (0x1020_0073, _) => {
                    let sys = self
                        .sys
                        .as_mut()
                        .ok_or(Exception::IllegalInstruction { insn })?;
                    if sys.mode == Mode::User
                        || (sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TSR != 0)
                    {
                        return Err(Exception::IllegalInstruction { insn });
                    }
                    let spp = if sys.mstatus & MSTATUS_SPP != 0 {
                        Mode::Supervisor
                    } else {
                        Mode::User
                    };
                    let spie = (sys.mstatus >> 5) & 1;
                    sys.mstatus = (sys.mstatus & !(MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP))
                        | (spie << 1)
                        | MSTATUS_SPIE;
                    if spp != Mode::Machine {
                        sys.mstatus &= !MSTATUS_MPRV;
                    }
                    sys.mode = spp;
                    next_pc = sys.sepc;
                    self.flush_tlb();
                }
                // WFI: report to host if nothing pending (host may idle).
                (0x1050_0073, _) => {
                    if let Some(sys) = self.sys.as_ref() {
                        // U-mode WFI is illegal; S-mode WFI traps when TW=1.
                        if sys.mode == Mode::User
                            || (sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TW != 0)
                        {
                            return Err(Exception::IllegalInstruction { insn });
                        }
                        if sys.mip & sys.mie == 0 {
                            stop = Some(StopReason::Wfi);
                        }
                    }
                }
                // SFENCE.VMA (funct7 = 0x09, f3 = 0)
                _ if funct7(insn) == 0x09 && funct3(insn) == 0 => {
                    if let Some(sys) = self.sys.as_ref() {
                        // U-mode always traps; S-mode traps when TVM=1.
                        if sys.mode == Mode::User
                            || (sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TVM != 0)
                        {
                            return Err(Exception::IllegalInstruction { insn });
                        }
                    }
                    self.flush_tlb()
                }
                // Zicsr
                (_, f3 @ 1..=3) | (_, f3 @ 5..=7) => {
                    let csr = insn >> 20;
                    if csr <= 3 {
                        // fflags/frm/fcsr are FP state
                        self.fp_check(insn)?;
                        self.fp_dirty();
                    }
                    let src = if f3 >= 5 {
                        rs1(insn) as u64 // immediate form: uimm5
                    } else {
                        self.x[rs1(insn)]
                    };
                    let old = self
                        .csr_read(csr)
                        .ok_or(Exception::IllegalInstruction { insn })?;
                    // CSRRS/CSRRC with rs1=x0 (or uimm=0) must not write.
                    let src_is_zero = if f3 >= 5 { src == 0 } else { rs1(insn) == 0 };
                    let new = match f3 & 3 {
                        1 => Some(src),                        // CSRRW[I]
                        2 if !src_is_zero => Some(old | src),  // CSRRS[I]
                        3 if !src_is_zero => Some(old & !src), // CSRRC[I]
                        _ => None,
                    };
                    if let Some(v) = new {
                        if !self.csr_write(csr, v) {
                            return Err(Exception::IllegalInstruction { insn });
                        }
                    }
                    self.wr(rd(insn), old);
                }
                _ => return Err(Exception::IllegalInstruction { insn }),
            },
            _ => return Err(Exception::IllegalInstruction { insn }),
        }

        self.pc = next_pc;
        self.insn_count += 1;
        Ok(stop)
    }

    /// OP-FP (opcode 0x53). Host-float implementation: results follow IEEE
    /// 754 via the host FPU; fflags are only approximated (documented
    /// deviation until a softfloat pass — TinyEMU's softfp.c is the model).
    fn op_fp(&mut self, insn: u32) -> Result<(), Exception> {
        let f7 = funct7(insn);
        let fmt = f7 & 3;
        let op = f7 >> 2;
        let (d, s1, s2) = (rd(insn), rs1(insn), rs2(insn));
        let ill = Err(Exception::IllegalInstruction { insn });

        // RISC-V float->int conversions: NaN and overflow saturate to the
        // destination's extreme (NaN -> most-positive), unlike Rust's
        // `as` which sends NaN to 0.
        macro_rules! cvt {
            ($v:expr, $ity:ty) => {{
                let v = $v;
                if v.is_nan() {
                    <$ity>::MAX as u64
                } else {
                    (v as $ity) as u64
                }
            }};
        }

        match (op, fmt) {
            // ---- f32 arithmetic ----
            (0x00..=0x03, 0) => {
                let (a, b) = (unbox32(self.f[s1]), unbox32(self.f[s2]));
                let r = match op {
                    0 => a + b,
                    1 => a - b,
                    2 => a * b,
                    _ => a / b,
                };
                self.f[d] = box32(r);
            }
            (0x0b, 0) => self.f[d] = box32(libm::sqrtf(unbox32(self.f[s1]))),
            // ---- f64 arithmetic ----
            (0x00..=0x03, 1) => {
                let (a, b) = (f64::from_bits(self.f[s1]), f64::from_bits(self.f[s2]));
                let r = match op {
                    0 => a + b,
                    1 => a - b,
                    2 => a * b,
                    _ => a / b,
                };
                self.f[d] = r.to_bits();
            }
            (0x0b, 1) => self.f[d] = libm::sqrt(f64::from_bits(self.f[s1])).to_bits(),

            // ---- sign injection ----
            (0x04, 0) => {
                let (a, b) = (unbox32(self.f[s1]).to_bits(), unbox32(self.f[s2]).to_bits());
                let r = match funct3(insn) {
                    0 => (a & 0x7fff_ffff) | (b & 0x8000_0000),
                    1 => (a & 0x7fff_ffff) | (!b & 0x8000_0000),
                    2 => a ^ (b & 0x8000_0000),
                    _ => return ill,
                };
                self.f[d] = box32(f32::from_bits(r));
            }
            (0x04, 1) => {
                let (a, b) = (self.f[s1], self.f[s2]);
                const SIGN: u64 = 1 << 63;
                self.f[d] = match funct3(insn) {
                    0 => (a & !SIGN) | (b & SIGN),
                    1 => (a & !SIGN) | (!b & SIGN),
                    2 => a ^ (b & SIGN),
                    _ => return ill,
                };
            }

            // ---- min/max (RISC-V: -0 < +0, NaN loses unless both NaN) ----
            (0x05, 0) => {
                let (a, b) = (unbox32(self.f[s1]), unbox32(self.f[s2]));
                let r = if a.is_nan() && b.is_nan() {
                    f32::NAN
                } else if a.is_nan() {
                    b
                } else if b.is_nan() {
                    a
                } else if a == b {
                    // break ±0 tie by sign
                    if (funct3(insn) == 0) == a.is_sign_negative() {
                        a
                    } else {
                        b
                    }
                } else if (funct3(insn) == 0) == (a < b) {
                    a
                } else {
                    b
                };
                self.f[d] = box32(r);
            }
            (0x05, 1) => {
                let (a, b) = (f64::from_bits(self.f[s1]), f64::from_bits(self.f[s2]));
                let r = if a.is_nan() && b.is_nan() {
                    f64::NAN
                } else if a.is_nan() {
                    b
                } else if b.is_nan() {
                    a
                } else if a == b {
                    if (funct3(insn) == 0) == a.is_sign_negative() {
                        a
                    } else {
                        b
                    }
                } else if (funct3(insn) == 0) == (a < b) {
                    a
                } else {
                    b
                };
                self.f[d] = r.to_bits();
            }

            // ---- float<->float conversion ----
            (0x08, 0) if s2 == 1 => self.f[d] = box32(f64::from_bits(self.f[s1]) as f32), // FCVT.S.D
            (0x08, 1) if s2 == 0 => self.f[d] = (unbox32(self.f[s1]) as f64).to_bits(), // FCVT.D.S

            // ---- comparisons (write to integer rd) ----
            (0x14, 0) => {
                let (a, b) = (unbox32(self.f[s1]), unbox32(self.f[s2]));
                let r = match funct3(insn) {
                    2 => a == b,
                    1 => a < b,
                    0 => a <= b,
                    _ => return ill,
                };
                self.wr(d, r as u64);
            }
            (0x14, 1) => {
                let (a, b) = (f64::from_bits(self.f[s1]), f64::from_bits(self.f[s2]));
                let r = match funct3(insn) {
                    2 => a == b,
                    1 => a < b,
                    0 => a <= b,
                    _ => return ill,
                };
                self.wr(d, r as u64);
            }

            // ---- float -> int ----
            (0x18, 0) => {
                let v = unbox32(self.f[s1]);
                let r = match s2 {
                    0 => cvt!(v, i32) as i32 as i64 as u64,
                    1 => cvt!(v, u32) as u32 as i32 as i64 as u64,
                    2 => cvt!(v, i64),
                    3 => cvt!(v, u64),
                    _ => return ill,
                };
                self.wr(d, r);
            }
            (0x18, 1) => {
                let v = f64::from_bits(self.f[s1]);
                let r = match s2 {
                    0 => cvt!(v, i32) as i32 as i64 as u64,
                    1 => cvt!(v, u32) as u32 as i32 as i64 as u64,
                    2 => cvt!(v, i64),
                    3 => cvt!(v, u64),
                    _ => return ill,
                };
                self.wr(d, r);
            }

            // ---- int -> float ----
            (0x1a, 0) => {
                let x = self.x[s1];
                let r = match s2 {
                    0 => x as i32 as f32,
                    1 => x as u32 as f32,
                    2 => x as i64 as f32,
                    3 => x as f32,
                    _ => return ill,
                };
                self.f[d] = box32(r);
            }
            (0x1a, 1) => {
                let x = self.x[s1];
                let r = match s2 {
                    0 => x as i32 as f64,
                    1 => x as u32 as f64,
                    2 => x as i64 as f64,
                    3 => x as f64,
                    _ => return ill,
                };
                self.f[d] = r.to_bits();
            }

            // ---- moves / classify ----
            (0x1c, 0) if funct3(insn) == 0 => {
                self.wr(d, self.f[s1] as u32 as i32 as i64 as u64) // FMV.X.W
            }
            (0x1c, 0) => {
                let v = unbox32(self.f[s1]);
                let snan = v.is_nan() && (v.to_bits() & 0x0040_0000) == 0;
                self.wr(
                    d,
                    fclass(
                        v.is_sign_negative(),
                        v.is_infinite(),
                        v.is_nan(),
                        snan,
                        v == 0.0,
                        v.is_subnormal(),
                    ),
                );
            }
            (0x1c, 1) if funct3(insn) == 0 => self.wr(d, self.f[s1]), // FMV.X.D
            (0x1c, 1) => {
                let v = f64::from_bits(self.f[s1]);
                let snan = v.is_nan() && (v.to_bits() & 0x0008_0000_0000_0000) == 0;
                self.wr(
                    d,
                    fclass(
                        v.is_sign_negative(),
                        v.is_infinite(),
                        v.is_nan(),
                        snan,
                        v == 0.0,
                        v.is_subnormal(),
                    ),
                );
            }
            (0x1e, 0) => self.f[d] = box32(f32::from_bits(self.x[s1] as u32)), // FMV.W.X
            (0x1e, 1) => self.f[d] = self.x[s1],                               // FMV.D.X

            _ => return ill,
        }
        Ok(())
    }

    /// Read a CSR; None = unimplemented (traps as illegal instruction).
    fn csr_read(&self, csr: u32) -> Option<u64> {
        // Privilege check: bits [9:8] of the address encode the minimum mode.
        if let Some(sys) = self.sys.as_ref() {
            if ((csr >> 8) & 3) as u64 > sys.mode as u64 {
                return None;
            }
        }
        match csr {
            FFLAGS => Some((self.fcsr & 0x1f) as u64),
            FRM => Some(((self.fcsr >> 5) & 7) as u64),
            FCSR => Some(self.fcsr as u64),
            CYCLE | INSTRET | MCYCLE | MINSTRET => Some(
                self.insn_count
                    .wrapping_add(self.sys.as_ref().map_or(0, |s| s.minstret_off)),
            ),
            TIME => self.sys.as_ref().map(|s| s.mtime).or(Some(self.insn_count)),
            // PMP: storage only, no enforcement (single-guest machine).
            0x3a0..=0x3af if csr & 1 == 0 => self
                .sys
                .as_ref()
                .map(|s| s.pmpcfg[((csr - 0x3a0) / 2) as usize]),
            0x3b0..=0x3ef => self.sys.as_ref().map(|s| s.pmpaddr[(csr - 0x3b0) as usize]),
            _ => {
                let sys = self.sys.as_ref()?;
                // SD summarizes FS: set when FP state is dirty.
                let mstatus = if sys.mstatus & MSTATUS_FS == MSTATUS_FS {
                    sys.mstatus | MSTATUS_SD
                } else {
                    sys.mstatus
                };
                Some(match csr {
                    SSTATUS => mstatus & SSTATUS_MASK,
                    SIE => sys.mie & sys.mideleg,
                    STVEC => sys.stvec,
                    SCOUNTEREN => sys.scounteren,
                    SSCRATCH => sys.sscratch,
                    SEPC => sys.sepc,
                    SCAUSE => sys.scause,
                    STVAL => sys.stval,
                    SIP => sys.mip & sys.mideleg,
                    SATP => {
                        // S-mode satp access traps when mstatus.TVM = 1.
                        if sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TVM != 0 {
                            return None;
                        }
                        sys.satp
                    }
                    // Debug triggers: none implemented. tselect reads back
                    // nonzero after writing 0 — the architected "hardwired"
                    // signal riscv-tests uses to skip trigger tests.
                    0x7a0 => 1,
                    0x7a1..=0x7a3 | 0x7a5 => 0,
                    MSTATUS => mstatus,
                    MISA => MISA_VALUE,
                    MEDELEG => sys.medeleg,
                    MIDELEG => sys.mideleg,
                    MIE => sys.mie,
                    MTVEC => sys.mtvec,
                    MCOUNTEREN => sys.mcounteren,
                    MSCRATCH => sys.mscratch,
                    MEPC => sys.mepc,
                    MCAUSE => sys.mcause,
                    MTVAL => sys.mtval,
                    MIP => sys.mip,
                    MVENDORID | MARCHID | MIMPID => 0,
                    MHARTID => sys.mhartid,
                    _ => return None,
                })
            }
        }
    }

    /// Write a CSR; false = unimplemented/read-only.
    fn csr_write(&mut self, csr: u32, v: u64) -> bool {
        if csr >> 10 == 3 {
            return false; // read-only region
        }
        if let Some(sys) = self.sys.as_ref() {
            if ((csr >> 8) & 3) as u64 > sys.mode as u64 {
                return false;
            }
        }
        match csr {
            FFLAGS => self.fcsr = (self.fcsr & !0x1f) | (v as u32 & 0x1f),
            FRM => self.fcsr = (self.fcsr & !0xe0) | ((v as u32 & 7) << 5),
            FCSR => self.fcsr = v as u32 & 0xff,
            MCYCLE | MINSTRET => {
                // Writable counters. The writing csrw itself retires after
                // the write takes effect, so bias by insn_count+1: a
                // csrw 0 / csrr pair reads back exactly 0.
                let ic = self.insn_count.wrapping_add(1);
                if let Some(sys) = self.sys.as_mut() {
                    sys.minstret_off = v.wrapping_sub(ic);
                }
            }
            0x3a0..=0x3af if csr & 1 == 0 => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.pmpcfg[((csr - 0x3a0) / 2) as usize] = v;
            }
            0x3b0..=0x3ef => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                // WARL: address bits [53:0]
                sys.pmpaddr[(csr - 0x3b0) as usize] = v & 0x003f_ffff_ffff_ffff;
            }
            SSTATUS => {
                const W: u64 = MSTATUS_SIE
                    | MSTATUS_SPIE
                    | MSTATUS_SPP
                    | MSTATUS_FS
                    | MSTATUS_SUM
                    | MSTATUS_MXR;
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mstatus = (sys.mstatus & !W) | (v & W);
                self.flush_tlb(); // SUM/MXR affect translation
            }
            SIE => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                let mask = sys.mideleg;
                sys.mie = (sys.mie & !mask) | (v & mask);
            }
            STVEC => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.stvec = v & !2;
            }
            SCOUNTEREN => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.scounteren = v & 7;
            }
            SSCRATCH => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.sscratch = v;
            }
            SEPC => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.sepc = v & !1;
            }
            SCAUSE => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.scause = v;
            }
            STVAL => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.stval = v;
            }
            SIP => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                // Only SSIP is directly writable by S-mode.
                let mask = IRQ_SSIP & sys.mideleg;
                sys.mip = (sys.mip & !mask) | (v & mask);
            }
            SATP => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                if sys.mode == Mode::Supervisor && sys.mstatus & MSTATUS_TVM != 0 {
                    return false; // traps as illegal under TVM
                }
                // Accept bare/sv39/sv48; ignore others (WARL).
                let mode = v >> 60;
                if mode == 0 || mode == 8 || mode == 9 {
                    sys.satp = v;
                    self.flush_tlb();
                }
            }
            // Debug trigger CSRs: writes ignored (no triggers implemented).
            0x7a0..=0x7a3 | 0x7a5 => {}
            MSTATUS => {
                const W: u64 = MSTATUS_SIE
                    | MSTATUS_MIE
                    | MSTATUS_SPIE
                    | MSTATUS_MPIE
                    | MSTATUS_SPP
                    | MSTATUS_MPP
                    | MSTATUS_FS
                    | MSTATUS_MPRV
                    | MSTATUS_SUM
                    | MSTATUS_MXR
                    | MSTATUS_TVM
                    | MSTATUS_TW
                    | MSTATUS_TSR;
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mstatus = (sys.mstatus & !W) | (v & W);
                self.flush_tlb();
            }
            MISA => {}
            MEDELEG => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.medeleg = v;
            }
            MIDELEG => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mideleg = v & (IRQ_SSIP | IRQ_STIP | IRQ_SEIP);
            }
            MIE => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mie = v;
            }
            MTVEC => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mtvec = v & !2;
            }
            MCOUNTEREN => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mcounteren = v & 7;
            }
            MSCRATCH => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mscratch = v;
            }
            MEPC => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mepc = v & !1;
            }
            MCAUSE => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mcause = v;
            }
            MTVAL => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                sys.mtval = v;
            }
            MIP => {
                let Some(sys) = self.sys.as_mut() else {
                    return false;
                };
                // MSIP/MTIP are set by the CLINT; software may write others.
                const W: u64 = IRQ_SSIP | IRQ_STIP | IRQ_SEIP;
                sys.mip = (sys.mip & !W) | (v & W);
            }
            _ => return false,
        }
        true
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
            0xffe00093, // addi x1, x0, -2
            0x000012b7, // lui x5, 0x1  (x5 = 0x1000 = BASE)
            0x10129023, // sh x1, 0x100(x5)
            0x1012b423, // sd x1, 0x108(x5)
            0x1082b103, // ld x2, 0x108(x5)
            0x1082a183, // lw x3, 0x108(x5)
            0x1082c203, // lbu x4, 0x108(x5)
            0x00000073, // ecall
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
    fn m_extension() {
        // x1 = 7, x2 = -3; mul, mulh, div, rem, divw by zero
        let (cpu, _) = run_program(&[
            0x00700093, // addi x1, x0, 7
            0xffd00113, // addi x2, x0, -3
            0x022081b3, // mul  x3, x1, x2
            0x02209233, // mulh x4, x1, x2
            0x0220c2b3, // div  x5, x1, x2
            0x0220e333, // rem  x6, x1, x2
            0x0200c3bb, // divw x7, x1, x0  (div by zero -> -1)
            0x00000073, // ecall
        ]);
        assert_eq!(cpu.x[3] as i64, -21);
        assert_eq!(cpu.x[4] as i64, -1); // high bits of 7 * -3
        assert_eq!(cpu.x[5] as i64, -2); // 7 / -3 truncates toward zero
        assert_eq!(cpu.x[6] as i64, 1); // 7 rem -3
        assert_eq!(cpu.x[7] as i64, -1); // div by zero
    }

    #[test]
    fn a_extension_lr_sc_amo() {
        // x5 = BASE; store 100 at 0x100(x5); lr.d x1; sc.d x2 (succeeds -> 0);
        // amoadd.d x3 = old(100), mem += 5; ld x4 = 105... build:
        let (cpu, _) = run_program(&[
            0x000012b7, // lui x5, 0x1 (BASE)
            0x10028293, // addi x5, x5, 0x100
            0x06400313, // addi x6, x0, 100
            0x0062b023, // sd x6, 0(x5)
            0x1002b0af, // lr.d x1, (x5)
            0x1862b12f, // sc.d x2, x6, (x5)
            0x00500393, // addi x7, x0, 5
            0x0072b1af, // amoadd.d x3, x7, (x5)
            0x0002b203, // ld x4, 0(x5)
            0x00000073, // ecall
        ]);
        assert_eq!(cpu.x[1], 100); // lr loaded
        assert_eq!(cpu.x[2], 0); // sc succeeded
        assert_eq!(cpu.x[3], 100); // amoadd returned old
        assert_eq!(cpu.x[4], 105); // memory updated
    }

    #[test]
    fn compressed_instructions_execute() {
        // c.li a0, 21 (0x4555); c.mv a1, a0 (0x85aa); c.add a0, a1 (0x952e); ecall
        let mut mem = vec![0u8; 0x10000];
        let halves: [u16; 3] = [0x4555, 0x85aa, 0x952e];
        for (i, h) in halves.iter().enumerate() {
            mem[i * 2..i * 2 + 2].copy_from_slice(&h.to_le_bytes());
        }
        mem[6..10].copy_from_slice(&0x00000073u32.to_le_bytes());
        let mut cpu = Cpu::new();
        cpu.pc = BASE;
        let mut bus = FlatMemory::new(BASE, &mut mem);
        assert_eq!(cpu.run(&mut bus, 100), StopReason::Ecall);
        assert_eq!(cpu.x[10], 42); // a0 = 21 + 21
        assert_eq!(cpu.x[11], 21); // a1
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
