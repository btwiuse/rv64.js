/// RISC-V exception causes (privileged spec, mcause values).
///
/// Phase 0/1 only raises a handful of these; the full set is listed now so
/// trap plumbing never needs a schema change when full-system mode lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exception {
    InstructionAddressMisaligned { addr: u64 },
    InstructionAccessFault { addr: u64 },
    IllegalInstruction { insn: u32 },
    Breakpoint,
    LoadAddressMisaligned { addr: u64 },
    LoadAccessFault { addr: u64 },
    StoreAddressMisaligned { addr: u64 },
    StoreAccessFault { addr: u64 },
    EnvironmentCallFromUMode,
    EnvironmentCallFromSMode,
    EnvironmentCallFromMMode,
    InstructionPageFault { addr: u64 },
    LoadPageFault { addr: u64 },
    StorePageFault { addr: u64 },
}

impl Exception {
    /// mcause code per the privileged spec.
    pub fn cause(&self) -> u64 {
        use Exception::*;
        match self {
            InstructionAddressMisaligned { .. } => 0,
            InstructionAccessFault { .. } => 1,
            IllegalInstruction { .. } => 2,
            Breakpoint => 3,
            LoadAddressMisaligned { .. } => 4,
            LoadAccessFault { .. } => 5,
            StoreAddressMisaligned { .. } => 6,
            StoreAccessFault { .. } => 7,
            EnvironmentCallFromUMode => 8,
            EnvironmentCallFromSMode => 9,
            EnvironmentCallFromMMode => 11,
            InstructionPageFault { .. } => 12,
            LoadPageFault { .. } => 13,
            StorePageFault { .. } => 15,
        }
    }
}
