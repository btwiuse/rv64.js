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

    /// Instruction fetch. Separate from data reads so full-system mode can
    /// apply execute permissions and take InstructionPageFault distinctly.
    fn fetch32(&mut self, addr: u64) -> Result<u32, Exception> {
        self.read32(addr)
            .map_err(|_| Exception::InstructionAccessFault { addr })
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
