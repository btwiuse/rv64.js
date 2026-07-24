use crate::exception::Exception;

/// Memory/MMIO access interface the CPU executes against.
///
/// This is the seam between user-mode and full-system emulation:
/// - user-mode: [`FlatMemory`] — bounds-checked flat buffer, no translation
/// - full-system (later): sv39/sv48 page-table walk + TLB, then RAM or MMIO
///
/// Addresses are guest virtual addresses; the implementation decides what
/// translation means.
pub trait Bus {
    fn read8(&mut self, addr: u64) -> Result<u8, Exception>;
    fn read16(&mut self, addr: u64) -> Result<u16, Exception>;
    fn read32(&mut self, addr: u64) -> Result<u32, Exception>;
    fn read64(&mut self, addr: u64) -> Result<u64, Exception>;
    fn write8(&mut self, addr: u64, val: u8) -> Result<(), Exception>;
    fn write16(&mut self, addr: u64, val: u16) -> Result<(), Exception>;
    fn write32(&mut self, addr: u64, val: u32) -> Result<(), Exception>;
    fn write64(&mut self, addr: u64, val: u64) -> Result<(), Exception>;

    /// Fused JIT-TLB support: if the page holding physical address `pa` (which
    /// `va` translates to) is directly JIT-accessible — in guest RAM, and for a
    /// store also writable and not holding compiled code — return the linear
    /// offset such that `linear_index = va + off`; else `None`. Lets a JIT block
    /// access memory with just a tag match and one add. Default: not accessible.
    fn jit_fast_off(&self, _va: u64, _pa: u64, _store: bool) -> Option<i64> {
        None
    }

    /// Instruction fetch. Separate from data reads so full-system mode can
    /// apply execute permissions and take InstructionPageFault distinctly.
    fn fetch32(&mut self, addr: u64) -> Result<u32, Exception> {
        self.read32(addr)
            .map_err(|_| Exception::InstructionAccessFault { addr })
    }

    /// Halfword fetch — the CPU fetches 16 bits at a time so compressed
    /// instructions on a page's last halfword don't over-fetch.
    fn fetch16(&mut self, addr: u64) -> Result<u16, Exception> {
        self.read16(addr)
            .map_err(|_| Exception::InstructionAccessFault { addr })
    }

    /// Hardware interrupt lines (MTIP/MSIP/MEIP/SEIP bit positions), sampled
    /// by the CPU before each instruction in full-system mode. Level-
    /// triggered: the device recomputes from its own state, so a cleared
    /// condition (e.g. mtimecmp rewritten) drops the line immediately —
    /// no stale-mip interrupt storms.
    fn irq_lines(&mut self) -> u64 {
        0
    }
}

/// Flat guest memory starting at `base` — the user-mode Bus.
pub struct FlatMemory<'a> {
    pub base: u64,
    pub mem: &'a mut [u8],
}

impl<'a> FlatMemory<'a> {
    pub fn new(base: u64, mem: &'a mut [u8]) -> Self {
        Self { base, mem }
    }

    #[inline]
    fn offset(&self, addr: u64, len: u64) -> Option<usize> {
        let off = addr.checked_sub(self.base)?;
        if off + len <= self.mem.len() as u64 {
            Some(off as usize)
        } else {
            None
        }
    }
}

macro_rules! flat_rw {
    ($rd:ident, $wr:ident, $ty:ty, $n:expr) => {
        fn $rd(&mut self, addr: u64) -> Result<$ty, Exception> {
            let off = self
                .offset(addr, $n)
                .ok_or(Exception::LoadAccessFault { addr })?;
            let bytes: [u8; $n] = self.mem[off..off + $n].try_into().unwrap();
            Ok(<$ty>::from_le_bytes(bytes))
        }
        fn $wr(&mut self, addr: u64, val: $ty) -> Result<(), Exception> {
            let off = self
                .offset(addr, $n)
                .ok_or(Exception::StoreAccessFault { addr })?;
            self.mem[off..off + $n].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
    };
}

impl Bus for FlatMemory<'_> {
    flat_rw!(read8, write8, u8, 1);
    flat_rw!(read16, write16, u16, 2);
    flat_rw!(read32, write32, u32, 4);
    flat_rw!(read64, write64, u64, 8);
}
