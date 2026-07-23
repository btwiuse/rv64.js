//! Privileged-architecture state: privilege modes, CSR addresses, and
//! mstatus bit layout. Scope mirrors what TinyEMU implements (enough to
//! boot mainline Linux): M/S/U, no N extension, no PMP, no hypervisor.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd)]
pub enum Mode {
    User = 0,
    Supervisor = 1,
    Machine = 3,
}

impl Mode {
    pub fn from_bits(b: u64) -> Mode {
        match b & 3 {
            0 => Mode::User,
            1 => Mode::Supervisor,
            _ => Mode::Machine,
        }
    }
}

// CSR addresses
pub const FFLAGS: u32 = 0x001;
pub const FRM: u32 = 0x002;
pub const FCSR: u32 = 0x003;

pub const SSTATUS: u32 = 0x100;
pub const SIE: u32 = 0x104;
pub const STVEC: u32 = 0x105;
pub const SCOUNTEREN: u32 = 0x106;
pub const SSCRATCH: u32 = 0x140;
pub const SEPC: u32 = 0x141;
pub const SCAUSE: u32 = 0x142;
pub const STVAL: u32 = 0x143;
pub const SIP: u32 = 0x144;
pub const SATP: u32 = 0x180;

pub const MSTATUS: u32 = 0x300;
pub const MISA: u32 = 0x301;
pub const MEDELEG: u32 = 0x302;
pub const MIDELEG: u32 = 0x303;
pub const MIE: u32 = 0x304;
pub const MTVEC: u32 = 0x305;
pub const MCOUNTEREN: u32 = 0x306;
pub const MSCRATCH: u32 = 0x340;
pub const MEPC: u32 = 0x341;
pub const MCAUSE: u32 = 0x342;
pub const MTVAL: u32 = 0x343;
pub const MIP: u32 = 0x344;

pub const MCYCLE: u32 = 0xb00;
pub const MINSTRET: u32 = 0xb02;
pub const CYCLE: u32 = 0xc00;
pub const TIME: u32 = 0xc01;
pub const INSTRET: u32 = 0xc02;

pub const MVENDORID: u32 = 0xf11;
pub const MARCHID: u32 = 0xf12;
pub const MIMPID: u32 = 0xf13;
pub const MHARTID: u32 = 0xf14;

// mstatus bits
pub const MSTATUS_SIE: u64 = 1 << 1;
pub const MSTATUS_MIE: u64 = 1 << 3;
pub const MSTATUS_SPIE: u64 = 1 << 5;
pub const MSTATUS_MPIE: u64 = 1 << 7;
pub const MSTATUS_SPP: u64 = 1 << 8;
pub const MSTATUS_MPP: u64 = 3 << 11;
pub const MSTATUS_FS: u64 = 3 << 13;
pub const MSTATUS_MPRV: u64 = 1 << 17;
pub const MSTATUS_SUM: u64 = 1 << 18;
pub const MSTATUS_MXR: u64 = 1 << 19;
pub const MSTATUS_TVM: u64 = 1 << 20;
pub const MSTATUS_TW: u64 = 1 << 21;
pub const MSTATUS_TSR: u64 = 1 << 22;
pub const MSTATUS_UXL: u64 = 3 << 32;
pub const MSTATUS_SXL: u64 = 3 << 34;
pub const MSTATUS_SD: u64 = 1 << 63;

/// Bits of mstatus visible/writable through sstatus.
pub const SSTATUS_MASK: u64 = MSTATUS_SIE
    | MSTATUS_SPIE
    | MSTATUS_SPP
    | MSTATUS_FS
    | MSTATUS_SUM
    | MSTATUS_MXR
    | MSTATUS_UXL
    | MSTATUS_SD;

// Interrupt bits (mip/mie)
pub const IRQ_SSIP: u64 = 1 << 1;
pub const IRQ_MSIP: u64 = 1 << 3;
pub const IRQ_STIP: u64 = 1 << 5;
pub const IRQ_MTIP: u64 = 1 << 7;
pub const IRQ_SEIP: u64 = 1 << 9;
pub const IRQ_MEIP: u64 = 1 << 11;

/// misa: RV64 IMAFDCSU.
pub const MISA_VALUE: u64 = (2 << 62) // MXL=64
    | (1 << 0)  // A
    | (1 << 2)  // C
    | (1 << 3)  // D
    | (1 << 5)  // F
    | (1 << 8)  // I
    | (1 << 12) // M
    | (1 << 18) // S
    | (1 << 20); // U

/// The privileged CSR state added to the hart for full-system mode.
#[derive(Clone)]
pub struct SysCsrs {
    pub mode: Mode,
    pub mstatus: u64,
    pub medeleg: u64,
    pub mideleg: u64,
    pub mie: u64,
    pub mip: u64,
    pub mtvec: u64,
    pub mcounteren: u64,
    pub mscratch: u64,
    pub mepc: u64,
    pub mcause: u64,
    pub mtval: u64,
    pub stvec: u64,
    pub scounteren: u64,
    pub sscratch: u64,
    pub sepc: u64,
    pub scause: u64,
    pub stval: u64,
    pub satp: u64,
    pub mhartid: u64,
    /// mtime mirror, updated by the machine (CLINT owns the real one).
    pub mtime: u64,
    /// Live `time` CSR derivation. When `time_scale != 0`, rdtime returns
    /// `insn_count / time_scale + time_offset`, so it advances every
    /// instruction (matching the CLINT clock but at instruction granularity)
    /// instead of only at slice boundaries — needed by busy-wait loops like
    /// the kernel's __delay that read rdtime tightly. 0 = use `mtime` mirror.
    pub time_scale: u64,
    pub time_offset: u64,
    /// PMP storage (no enforcement — single-guest machine, like TinyEMU).
    pub pmpcfg: [u64; 8],
    pub pmpaddr: [u64; 64],
    /// minstret/mcycle are writable: offset relative to insn_count.
    pub minstret_off: u64,
}

impl Default for SysCsrs {
    fn default() -> Self {
        Self::new()
    }
}

impl SysCsrs {
    pub fn new() -> Self {
        SysCsrs {
            mode: Mode::Machine,
            // UXL/SXL fixed at 64-bit; FS starts Off (firmware enables it)
            mstatus: MSTATUS_UXL & (2 << 32) | MSTATUS_SXL & (2 << 34),
            medeleg: 0,
            mideleg: 0,
            mie: 0,
            mip: 0,
            mtvec: 0,
            mcounteren: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            time_scale: 0,
            time_offset: 0,
            stvec: 0,
            scounteren: 0,
            sscratch: 0,
            sepc: 0,
            scause: 0,
            stval: 0,
            satp: 0,
            mhartid: 0,
            mtime: 0,
            pmpcfg: [0; 8],
            pmpaddr: [0; 64],
            minstret_off: 0,
        }
    }
}
