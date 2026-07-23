//! Modern "virt"-class riscv64 machine: OpenSBI + a current Linux kernel.
//!
//! Unlike the TinyEMU-compatible [`crate::Machine`] (BBL + Linux 4.15,
//! minimal PLIC, HTIF), this models a QEMU-`virt`-like platform good enough
//! for a modern kernel and a full Debian userland:
//!
//! ```text
//! 0x0010_0000  sifive,test   (poweroff/reboot)
//! 0x0200_0000  CLINT         (msip, mtimecmp, mtime)
//! 0x0c00_0000  PLIC          (full: priorities, enables, thresholds, claim)
//! 0x1000_0000  UART          (ns16550 — OpenSBI + kernel console)
//! 0x1000_1000  virtio-mmio   (0x1000 apart; blk)
//! 0x8000_0000  RAM           (OpenSBI at base, kernel at +2 MiB, DTB above)
//! ```
//!
//! Boot: the hart resets in M-mode at RAM_BASE (OpenSBI fw_jump) with
//! a0=hartid, a1=dtb; OpenSBI sets up the SBI and drops to the S-mode kernel.

use crate::dtb::Fdt;
use crate::virtio::{Backend, VirtioDev};
use rv64_core::csr::{IRQ_MEIP, IRQ_MSIP, IRQ_MTIP, IRQ_SEIP};
use rv64_core::{Bus, Cpu, Exception, StopReason};

fn plic_dbg() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("RV_PLIC_DEBUG").is_ok())
}

pub const RAM_BASE: u64 = 0x8000_0000;
pub const TEST_BASE: u64 = 0x0010_0000;
pub const CLINT_BASE: u64 = 0x0200_0000;
pub const CLINT_SIZE: u64 = 0x1_0000;
pub const PLIC_BASE: u64 = 0x0c00_0000;
pub const PLIC_SIZE: u64 = 0x0400_0000;
pub const UART_BASE: u64 = 0x1000_0000;
pub const UART_SIZE: u64 = 0x100;
pub const VIRTIO_BASE: u64 = 0x1000_1000;
pub const VIRTIO_SIZE: u64 = 0x1000;
pub const VIRTIO_COUNT: u64 = 8;
/// 10 MHz architected timer, matching the DTB timebase.
pub const RTC_FREQ: u64 = 10_000_000;

const KERNEL_OFFSET: u64 = 0x20_0000; // kernel Image at RAM_BASE + 2 MiB

// Interrupt source numbers (PLIC). Source 0 = "no interrupt".
const UART_IRQ: u32 = 10;
const VIRTIO_IRQ_BASE: u32 = 1; // virtio dev i -> source (1 + i)

const PLIC_SOURCES: usize = 32; // one u32 bitmask is enough
const PLIC_CONTEXTS: usize = 2; // ctx0 = hart0 M-ext, ctx1 = hart0 S-ext

/// Full SiFive/QEMU-style PLIC.
struct Plic {
    priority: [u32; PLIC_SOURCES],
    pending: u32,               // level-driven by device lines (recomputed)
    enable: [u32; PLIC_CONTEXTS],
    threshold: [u32; PLIC_CONTEXTS],
    claimed: u32,               // in-service (claimed, awaiting complete)
}

impl Plic {
    fn new() -> Plic {
        Plic { priority: [0; PLIC_SOURCES], pending: 0, enable: [0; PLIC_CONTEXTS], threshold: [0; PLIC_CONTEXTS], claimed: 0 }
    }

    /// Best claimable source for a context (highest priority > threshold,
    /// enabled, pending, not already in-service). 0 if none.
    fn best(&self, ctx: usize) -> u32 {
        let elig = self.pending & self.enable[ctx] & !self.claimed;
        let mut best_id = 0u32;
        let mut best_pri = 0u32;
        let mut m = elig;
        while m != 0 {
            let i = m.trailing_zeros();
            m &= m - 1;
            let pri = self.priority[i as usize];
            if pri > self.threshold[ctx] && pri >= best_pri {
                best_pri = pri;
                best_id = i;
            }
        }
        best_id
    }

    /// Does context `ctx` have a deliverable external interrupt?
    fn pending_ctx(&self, ctx: usize) -> bool {
        self.best(ctx) != 0
    }

    fn read(&mut self, off: u64) -> u32 {
        match off {
            // source priorities: 0x0 + id*4  (id 1..)
            0x0000..=0x0fff => {
                let id = (off / 4) as usize;
                self.priority.get(id).copied().unwrap_or(0)
            }
            // pending bits: 0x1000 (32 sources in first word)
            0x1000 => self.pending,
            0x1004 => 0,
            // enables: 0x2000 + ctx*0x80
            _ if (0x2000..0x2000 + (PLIC_CONTEXTS as u64) * 0x80).contains(&off) => {
                let ctx = ((off - 0x2000) / 0x80) as usize;
                if (off - 0x2000) % 0x80 == 0 { self.enable[ctx] } else { 0 }
            }
            // per-context: threshold at 0x200000 + ctx*0x1000, claim at +4
            _ if off >= 0x20_0000 => {
                let ctx = ((off - 0x20_0000) / 0x1000) as usize;
                let reg = (off - 0x20_0000) % 0x1000;
                if ctx >= PLIC_CONTEXTS { return 0; }
                match reg {
                    0x0 => self.threshold[ctx],
                    0x4 => {
                        // claim: return best, mark in-service, clear pending
                        let id = self.best(ctx);
                        if id != 0 {
                            if plic_dbg() { eprintln!("[plic] claim[ctx{ctx}] -> src={id}"); }
                            self.claimed |= 1 << id;
                        }
                        id
                    }
                    _ => 0,
                }
            }
            _ => 0,
        }
    }

    fn write(&mut self, off: u64, val: u32) {
        match off {
            0x0000..=0x0fff => {
                let id = (off / 4) as usize;
                if id != 0 {
                    if plic_dbg() { eprintln!("[plic] priority[{id}]={val}"); }
                    if let Some(p) = self.priority.get_mut(id) {
                        *p = val;
                    }
                }
            }
            _ if (0x2000..0x2000 + (PLIC_CONTEXTS as u64) * 0x80).contains(&off) => {
                let ctx = ((off - 0x2000) / 0x80) as usize;
                if (off - 0x2000) % 0x80 == 0 {
                    if plic_dbg() { eprintln!("[plic] enable[ctx{ctx}]={val:#x}"); }
                    self.enable[ctx] = val & !1; // source 0 never enabled
                }
            }
            _ if off >= 0x20_0000 => {
                let ctx = ((off - 0x20_0000) / 0x1000) as usize;
                let reg = (off - 0x20_0000) % 0x1000;
                if ctx >= PLIC_CONTEXTS { return; }
                match reg {
                    0x0 => {
                        if plic_dbg() { eprintln!("[plic] threshold[ctx{ctx}]={val}"); }
                        self.threshold[ctx] = val;
                    }
                    0x4 => {
                        // complete: clear in-service for this source
                        if plic_dbg() { eprintln!("[plic] complete[ctx{ctx}] src={val}"); }
                        if (val as usize) < PLIC_SOURCES {
                            self.claimed &= !(1 << val);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Minimal ns16550 (8250) UART: enough for OpenSBI + kernel console.
struct Uart {
    ier: u8,   // interrupt enable
    lcr: u8,
    mcr: u8,
    scr: u8,
    rx: std::collections::VecDeque<u8>,
    tx_out: Vec<u8>,
    /// THR-empty interrupt pending (transmit is instant, so this tracks the
    /// 8250 THRE interrupt: armed when TX ints are enabled or a byte is sent,
    /// cleared when the guest reads IIR).
    thre_ip: bool,
}

impl Uart {
    fn new() -> Uart {
        Uart { ier: 0, lcr: 0, mcr: 0, scr: 0, rx: Default::default(), tx_out: Vec::new(), thre_ip: false }
    }
    // Register offsets (byte): 0 RBR/THR/DLL, 1 IER/DLM, 2 IIR/FCR, 3 LCR,
    // 4 MCR, 5 LSR, 6 MSR, 7 SCR. DLAB (LCR bit7) selects divisor latches.
    fn read(&mut self, off: u64) -> u8 {
        let dlab = self.lcr & 0x80 != 0;
        match off {
            0 if !dlab => self.rx.pop_front().unwrap_or(0), // RBR
            0 => 0,                                          // DLL
            1 if !dlab => self.ier,
            1 => 0, // DLM
            2 => {
                // IIR, highest-priority pending source first (FIFO bits 0xc0):
                // RX data available (0x04) outranks THR-empty (0x02). Reading
                // IIR acknowledges (clears) a pending THRE interrupt.
                if self.ier & 1 != 0 && !self.rx.is_empty() {
                    0xc4
                } else if self.thre_ip {
                    self.thre_ip = false;
                    0xc2
                } else {
                    0xc1
                }
            }
            3 => self.lcr,
            4 => self.mcr,
            5 => {
                // LSR: THR empty + TX empty always; DR if rx data.
                let mut lsr = 0x60;
                if !self.rx.is_empty() {
                    lsr |= 0x01;
                }
                lsr
            }
            6 => 0xb0, // MSR: DCD+DSR+CTS
            7 => self.scr,
            _ => 0,
        }
    }
    fn write(&mut self, off: u64, val: u8) {
        let dlab = self.lcr & 0x80 != 0;
        match off {
            0 if !dlab => {
                self.tx_out.push(val); // THR — transmitted instantly
                // THR is now empty again: re-arm the THR-empty interrupt so
                // interrupt-driven TX keeps flowing.
                if self.ier & 2 != 0 {
                    self.thre_ip = true;
                }
            }
            1 if !dlab => {
                let was = self.ier;
                self.ier = val;
                // Enabling the THR-empty interrupt raises it immediately (our
                // THR is always empty). Without this, the 8250 driver's
                // interrupt-driven TX / tty drain (e.g. bash's tcsetattr on the
                // console) blocks forever waiting for a THRE IRQ.
                if val & 2 != 0 && was & 2 == 0 {
                    self.thre_ip = true;
                }
            }
            3 => self.lcr = val,
            4 => self.mcr = val,
            7 => self.scr = val,
            _ => {}
        }
    }
    /// UART interrupt line: RX-data (if enabled) or THR-empty (if enabled).
    fn irq(&self) -> bool {
        (self.ier & 1 != 0 && !self.rx.is_empty()) || self.thre_ip
    }
}

pub struct VirtBus {
    pub ram: Vec<u8>,
    // CLINT
    pub mtime: u64,
    pub mtimecmp: u64,
    pub msip: bool,
    plic: Plic,
    uart: Uart,
    pub virtio: Vec<VirtioDev>,
    pub power_off: bool,
    // JIT support (mirrors crate::SystemBus)
    pub jit_pages: Vec<u64>,
    pub jit_dirty_pages: Vec<u64>,
}

impl VirtBus {
    fn refresh_plic(&mut self) {
        // Recompute level-triggered pending bits from device lines.
        let mut p = 0u32;
        if self.uart.irq() {
            p |= 1 << UART_IRQ;
        }
        for (i, d) in self.virtio.iter().enumerate() {
            if d.irq_pending() {
                p |= 1 << (VIRTIO_IRQ_BASE + i as u32);
            }
        }
        // Keep already-claimed-but-still-asserted lines out of `pending`
        // (they re-assert after `complete`); level sources naturally re-set.
        self.plic.pending = p;
    }

    /// Drain UART output (host console).
    pub fn uart_take(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.uart.tx_out)
    }
    /// Feed console input to the guest UART.
    pub fn uart_input(&mut self, bytes: &[u8]) {
        self.uart.rx.extend(bytes.iter().copied());
    }

    pub fn jit_mark_page(&mut self, pa: u64) {
        if pa >= RAM_BASE {
            let page = ((pa - RAM_BASE) >> 12) as usize;
            if let Some(w) = self.jit_pages.get_mut(page / 64) {
                *w |= 1 << (page % 64);
            }
        }
    }
    pub fn jit_unmark_page(&mut self, page: u64) {
        if let Some(w) = self.jit_pages.get_mut(page as usize / 64) {
            *w &= !(1 << (page % 64));
        }
    }
    pub fn jit_take_dirty(&mut self) -> Vec<u64> {
        core::mem::take(&mut self.jit_dirty_pages)
    }
    #[inline]
    fn jit_check_store(&mut self, addr: u64) {
        if addr >= RAM_BASE {
            let page = ((addr - RAM_BASE) >> 12) as usize;
            if let Some(w) = self.jit_pages.get(page / 64) {
                if w & (1 << (page % 64)) != 0 {
                    self.jit_dirty_pages.push(page as u64);
                }
            }
        }
    }

    #[inline]
    fn ram_slice(&mut self, addr: u64, len: usize) -> Option<&mut [u8]> {
        if addr >= RAM_BASE {
            let off = (addr - RAM_BASE) as usize;
            if off + len <= self.ram.len() {
                return Some(&mut self.ram[off..off + len]);
            }
        }
        None
    }

    fn mmio_read(&mut self, addr: u64, size: u32) -> Option<u64> {
        match addr {
            _ if (TEST_BASE..TEST_BASE + 0x1000).contains(&addr) => Some(0),
            _ if (CLINT_BASE..CLINT_BASE + CLINT_SIZE).contains(&addr) => Some(match addr - CLINT_BASE {
                0x0 => self.msip as u64,
                0x4000 => self.mtimecmp,
                0xbff8 => self.mtime,
                _ => 0,
            }),
            _ if (PLIC_BASE..PLIC_BASE + PLIC_SIZE).contains(&addr) => {
                Some(self.plic.read(addr - PLIC_BASE) as u64)
            }
            _ if (UART_BASE..UART_BASE + UART_SIZE).contains(&addr) => {
                Some(self.uart.read(addr - UART_BASE) as u64)
            }
            _ if (VIRTIO_BASE..VIRTIO_BASE + VIRTIO_COUNT * VIRTIO_SIZE).contains(&addr) => {
                let i = ((addr - VIRTIO_BASE) / VIRTIO_SIZE) as usize;
                let off = (addr - VIRTIO_BASE) % VIRTIO_SIZE;
                self.virtio.get_mut(i).map(|d| d.read(off) as u64)
            }
            _ => {
                let _ = size;
                None
            }
        }
    }

    fn mmio_write(&mut self, addr: u64, val: u64, _size: u32) -> bool {
        match addr {
            _ if (TEST_BASE..TEST_BASE + 0x1000).contains(&addr) => {
                // sifive,test: 0x5555 poweroff, 0x7777 reboot, 0x3333|.. fail
                let v = val as u32 & 0xffff;
                if v == 0x5555 || v == 0x7777 || (val as u32 & 0xffff) == 0x3333 {
                    self.power_off = true;
                }
                true
            }
            _ if (CLINT_BASE..CLINT_BASE + CLINT_SIZE).contains(&addr) => {
                match addr - CLINT_BASE {
                    0x0 => self.msip = val & 1 != 0,
                    0x4000 => self.mtimecmp = val,
                    _ => {}
                }
                true
            }
            _ if (PLIC_BASE..PLIC_BASE + PLIC_SIZE).contains(&addr) => {
                self.plic.write(addr - PLIC_BASE, val as u32);
                true
            }
            _ if (UART_BASE..UART_BASE + UART_SIZE).contains(&addr) => {
                self.uart.write(addr - UART_BASE, val as u8);
                true
            }
            _ if (VIRTIO_BASE..VIRTIO_BASE + VIRTIO_COUNT * VIRTIO_SIZE).contains(&addr) => {
                let i = ((addr - VIRTIO_BASE) / VIRTIO_SIZE) as usize;
                let off = (addr - VIRTIO_BASE) % VIRTIO_SIZE;
                if i < self.virtio.len() {
                    if let Some(q) = self.virtio[i].write(off, val as u32) {
                        let mut dev = self.virtio.remove(i);
                        dev.process(q as usize, &mut self.ram, RAM_BASE);
                        self.virtio.insert(i, dev);
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Poll every ready virtqueue, servicing any buffers the guest made
    /// available. QueueNotify normally drives this, but polling each slice
    /// recovers from any missed notification (a synchronous device model can
    /// otherwise lose a wakeup and hang the guest waiting on completed I/O).
    fn poll_virtio(&mut self) {
        for i in 0..self.virtio.len() {
            let mut dev = self.virtio.remove(i);
            for qi in 0..2 {
                dev.process(qi, &mut self.ram, RAM_BASE);
            }
            self.virtio.insert(i, dev);
        }
    }
}

macro_rules! virt_rw {
    ($rd:ident, $wr:ident, $ty:ty, $n:expr) => {
        fn $rd(&mut self, addr: u64) -> Result<$ty, Exception> {
            if let Some(s) = self.ram_slice(addr, $n) {
                let b: [u8; $n] = (&*s).try_into().unwrap();
                return Ok(<$ty>::from_le_bytes(b));
            }
            if let Some(v) = self.mmio_read(addr, $n) {
                return Ok(v as $ty);
            }
            Err(Exception::LoadAccessFault { addr })
        }
        fn $wr(&mut self, addr: u64, val: $ty) -> Result<(), Exception> {
            self.jit_check_store(addr);
            if let Some(s) = self.ram_slice(addr, $n) {
                s.copy_from_slice(&val.to_le_bytes());
                return Ok(());
            }
            if self.mmio_write(addr, val as u64, $n) {
                return Ok(());
            }
            Err(Exception::StoreAccessFault { addr })
        }
    };
}

impl Bus for VirtBus {
    virt_rw!(read8, write8, u8, 1);
    virt_rw!(read16, write16, u16, 2);
    virt_rw!(read32, write32, u32, 4);
    virt_rw!(read64, write64, u64, 8);

    fn irq_lines(&mut self) -> u64 {
        self.refresh_plic();
        let mut lines = 0u64;
        if self.mtime >= self.mtimecmp {
            lines |= IRQ_MTIP;
        }
        if self.msip {
            lines |= IRQ_MSIP;
        }
        if self.plic.pending_ctx(0) {
            lines |= IRQ_MEIP;
        }
        if self.plic.pending_ctx(1) {
            lines |= IRQ_SEIP;
        }
        lines
    }
}

pub struct VirtImages<'a> {
    pub opensbi: &'a [u8],
    pub kernel: &'a [u8],
    pub cmdline: &'a str,
    pub initrd: Option<&'a [u8]>,
    pub disk: Option<Vec<u8>>,
}

pub struct VirtMachine {
    pub cpu: Cpu,
    pub bus: VirtBus,
    pub insns_per_tick: u64,
    /// Guest timer ticks accrued while the hart was halted in WFI. Kept
    /// separate from `insn_count` so idle time advances the guest clock
    /// without inflating the (real) retired-instruction count.
    pub idle_ticks: u64,
    pub power_off: bool,
    pub dtb: Vec<u8>,
}

impl VirtMachine {
    pub fn new(ram_bytes: u64, images: VirtImages) -> VirtMachine {
        let ram_size = ram_bytes;
        let mut ram = vec![0u8; ram_size as usize];

        // OpenSBI at RAM base.
        ram[..images.opensbi.len()].copy_from_slice(images.opensbi);
        // Kernel Image at RAM_BASE + 2 MiB.
        let kbase = KERNEL_OFFSET as usize;
        ram[kbase..kbase + images.kernel.len()].copy_from_slice(images.kernel);
        let kend = kbase + images.kernel.len();

        let _ = kend;
        // Place initrd + DTB near the TOP of RAM (as QEMU/U-Boot do) so the
        // kernel's early allocations near the Image don't clobber them.
        // Layout from the top down: [DTB][initrd][fw_dynamic_info], each
        // aligned, leaving a small margin below the very top.
        let ram_top = ram_size as usize;
        let dtb = build_virt_fdt(ram_size, images.cmdline, 0, 0, {
            let mut n = 0;
            if images.disk.is_some() {
                n += 1;
            }
            n
        });
        // Reserve DTB just below the top (2 MiB margin, page aligned).
        let dtb_off = ((ram_top - 0x20_0000).saturating_sub(dtb.len())) & !0xfff;

        // initrd below the DTB (1 MiB aligned).
        let mut initrd_start = 0u64;
        let mut initrd_end = 0u64;
        let mut below = dtb_off;
        if let Some(ir) = images.initrd {
            let s = (below.saturating_sub(ir.len())) & !0xfffff;
            ram[s..s + ir.len()].copy_from_slice(ir);
            initrd_start = RAM_BASE + s as u64;
            initrd_end = initrd_start + ir.len() as u64;
            below = s;
        }

        // fw_dynamic_info struct below the initrd (page aligned).
        let dyn_off = (below.saturating_sub(0x1000)) & !0xfff;

        // Rebuild the DTB now that initrd addresses are known, then place it.
        let dtb = build_virt_fdt(
            ram_size,
            images.cmdline,
            initrd_start,
            initrd_end,
            if images.disk.is_some() { 1 } else { 0 },
        );
        ram[dtb_off..dtb_off + dtb.len()].copy_from_slice(&dtb);
        let dtb_addr = RAM_BASE + dtb_off as u64;

        // Virtio: one blk device if a disk is given (source 1).
        let mut virtio = Vec::new();
        if let Some(disk) = images.disk {
            virtio.push(VirtioDev::new(Backend::Block { disk }));
        }

        // fw_dynamic_info struct (OpenSBI reads it from a2). fw_jump.bin bakes
        // the FDT/next-stage addresses at build time and ignores a1, which
        // makes the kernel fault on a bogus DTB pointer; fw_dynamic forwards the
        // real DTB in a1 and jumps to the address we specify here.
        {
            let info: [u64; 6] = [
                0x4942_534f,                 // magic "OSBI"
                2,                           // version
                RAM_BASE + KERNEL_OFFSET,    // next_addr = kernel Image
                1,                           // next_mode = PRV_S
                0,                           // options
                0,                           // boot_hart = 0
            ];
            for (i, v) in info.iter().enumerate() {
                ram[dyn_off + i * 8..dyn_off + i * 8 + 8].copy_from_slice(&v.to_le_bytes());
            }
        }
        let dyn_addr = RAM_BASE + dyn_off as u64;

        // Enter OpenSBI (fw_dynamic) in M-mode: pc=RAM_BASE, a0=hartid, a1=dtb,
        // a2=&fw_dynamic_info.
        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        cpu.pc = RAM_BASE;
        cpu.x[10] = 0; // a0 = hartid
        cpu.x[11] = dtb_addr; // a1 = dtb
        cpu.x[12] = dyn_addr; // a2 = fw_dynamic_info

        VirtMachine {
            cpu,
            bus: VirtBus {
                ram,
                mtime: 0,
                mtimecmp: u64::MAX,
                msip: false,
                plic: Plic::new(),
                uart: Uart::new(),
                virtio,
                power_off: false,
                jit_pages: vec![0u64; (ram_size as usize >> 12).div_ceil(64)],
                jit_dirty_pages: Vec::new(),
            },
            insns_per_tick: 100,
            idle_ticks: 0,
            power_off: false,
            dtb,
        }
    }

    pub fn console_output(&mut self) -> Vec<u8> {
        self.bus.uart_take()
    }
    pub fn console_input(&mut self, bytes: &[u8]) {
        self.bus.uart_input(bytes);
    }

    pub fn sync_devices(&mut self) {
        self.bus.mtime = self.cpu.insn_count / self.insns_per_tick + self.idle_ticks;
        if let Some(sys) = self.cpu.sys.as_mut() {
            sys.mtime = self.bus.mtime;
            // Let rdtime advance every instruction (not just per slice) so
            // busy-wait loops reading `time` make progress: same clock as the
            // CLINT, derived live from insn_count.
            sys.time_scale = self.insns_per_tick;
            sys.time_offset = self.idle_ticks;
        }
    }

    /// Read a u16 from guest RAM (little-endian), for ring inspection.
    fn ram_u16(&self, pa: u64) -> u16 {
        let off = (pa - RAM_BASE) as usize;
        self.bus
            .ram
            .get(off..off + 2)
            .map(|s| u16::from_le_bytes([s[0], s[1]]))
            .unwrap_or(0)
    }

    /// One-line dump of interrupt-delivery state, for diagnosing idle hangs.
    pub fn debug_irq_state(&self) -> String {
        let p = &self.bus.plic;
        let vio: Vec<String> = self
            .bus
            .virtio
            .iter()
            .enumerate()
            .map(|(i, d)| {
                // For queue 0: avail.idx (guest submitted), my last_avail
                // (serviced), used.idx (device published), all so we can see
                // whether there's outstanding I/O the guest is blocked on.
                let ring = d.queue_debug(0).map(|(_r, _n, avail, used, last_avail)| {
                    let avail_idx = self.ram_u16(avail + 2);
                    let used_idx = self.ram_u16(used + 2);
                    format!(" q0[avail.idx={avail_idx} serviced={last_avail} used.idx={used_idx} outstanding={}]",
                        avail_idx.wrapping_sub(last_avail))
                }).unwrap_or_default();
                format!("v{i}.irq={}{ring}", d.irq_pending())
            })
            .collect();
        let (mip, mie) = self
            .cpu
            .sys
            .as_ref()
            .map(|s| (s.mip, s.mie))
            .unwrap_or((0, 0));
        let (sepc, scause) = self
            .cpu
            .sys
            .as_ref()
            .map(|s| (s.sepc, s.scause))
            .unwrap_or((0, 0));
        format!(
            "plic{{pend={:#x} claimed={:#x} best1={}}} {} mip={:#x} mie={:#x} mtime={} mtcmp={} timer_future={} sepc={:#x} scause={:#x}",
            p.pending, p.claimed, p.best(1),
            vio.join(","), mip, mie, self.bus.mtime, self.bus.mtimecmp,
            self.bus.mtimecmp > self.bus.mtime, sepc, scause,
        )
    }

    pub fn run_slice(&mut self, max_insns: u64) -> u64 {
        let start = self.cpu.insn_count;
        self.bus.poll_virtio();
        self.sync_devices();
        match self.cpu.run(&mut self.bus, max_insns) {
            StopReason::Wfi => {
                // Halted: fast-forward the guest clock to the next timer
                // deadline via `idle_ticks` (not `insn_count`, which must
                // stay a true retired-instruction count for budgets/perf).
                let next = self.bus.mtimecmp;
                if next != u64::MAX && next > self.bus.mtime {
                    self.idle_ticks += next - self.bus.mtime;
                }
            }
            _ => {}
        }
        self.sync_devices();
        self.power_off = self.bus.power_off;
        self.cpu.insn_count - start
    }
}

fn build_virt_fdt(
    ram_size: u64,
    cmdline: &str,
    initrd_start: u64,
    initrd_end: u64,
    n_virtio: usize,
) -> Vec<u8> {
    let mut f = Fdt::new();
    let intc_phandle = 1u32;
    let plic_phandle = 2u32;

    f.begin_node("");
    f.prop_u32("#address-cells", 2);
    f.prop_u32("#size-cells", 2);
    f.prop_str("compatible", "riscv-virtio");
    f.prop_str("model", "riscv-virtio,qemu");

    f.begin_node("chosen");
    f.prop_str("bootargs", cmdline);
    f.prop_str("stdout-path", "/soc/serial@10000000");
    // Seed the kernel CRNG from the DTB (as QEMU/U-Boot do). Without this the
    // guest starves for entropy — jitterentropy can't init on our too-regular
    // cycle counter, so every getrandom() blocks and boot stalls ~30s/step.
    // CONFIG_RANDOM_TRUST_BOOTLOADER credits this as full entropy.
    let mut seed = [0u8; 64];
    for (i, b) in seed.iter_mut().enumerate() {
        // Deterministic but well-mixed; a fixed seed is fine (the kernel only
        // needs unpredictable-to-the-guest bytes to initialize the CRNG).
        *b = ((i as u32).wrapping_mul(0x9e37_79b1) >> 13) as u8 ^ (i as u8).wrapping_mul(31);
    }
    f.prop("rng-seed", &seed);
    if initrd_end > initrd_start {
        f.prop("linux,initrd-start", &initrd_start.to_be_bytes());
        f.prop("linux,initrd-end", &initrd_end.to_be_bytes());
    }
    f.end_node();

    f.begin_node("cpus");
    f.prop_u32("#address-cells", 1);
    f.prop_u32("#size-cells", 0);
    f.prop_u32("timebase-frequency", RTC_FREQ as u32);
    f.begin_node("cpu@0");
    f.prop_str("device_type", "cpu");
    f.prop_u32("reg", 0);
    f.prop_str("status", "okay");
    f.prop_str("compatible", "riscv");
    f.prop_str("riscv,isa", "rv64imafdc");
    f.prop_str("mmu-type", "riscv,sv48");
    f.begin_node("interrupt-controller");
    f.prop_u32("#interrupt-cells", 1);
    f.prop("interrupt-controller", &[]);
    f.prop_str("compatible", "riscv,cpu-intc");
    f.prop_u32("phandle", intc_phandle);
    f.end_node();
    f.end_node(); // cpu@0
    f.end_node(); // cpus

    f.begin_node(&format!("memory@{RAM_BASE:x}"));
    f.prop_str("device_type", "memory");
    f.prop_u64_pair("reg", RAM_BASE, ram_size);
    f.end_node();

    f.begin_node("soc");
    f.prop_u32("#address-cells", 2);
    f.prop_u32("#size-cells", 2);
    f.prop_str("compatible", "simple-bus");
    f.prop("ranges", &[]);

    // test finisher
    f.begin_node(&format!("test@{TEST_BASE:x}"));
    f.prop_strs("compatible", &["sifive,test1", "sifive,test0", "syscon"]);
    f.prop_u64_pair("reg", TEST_BASE, 0x1000);
    let syscon_ph = 5u32;
    f.prop_u32("phandle", syscon_ph);
    f.end_node();
    f.begin_node("poweroff");
    f.prop_str("compatible", "syscon-poweroff");
    f.prop_u32("regmap", syscon_ph);
    f.prop_u32("offset", 0);
    f.prop_u32("value", 0x5555);
    f.end_node();
    f.begin_node("reboot");
    f.prop_str("compatible", "syscon-reboot");
    f.prop_u32("regmap", syscon_ph);
    f.prop_u32("offset", 0);
    f.prop_u32("value", 0x7777);
    f.end_node();

    // UART (ns16550)
    f.begin_node(&format!("serial@{UART_BASE:x}"));
    f.prop_str("compatible", "ns16550a");
    f.prop_u64_pair("reg", UART_BASE, UART_SIZE);
    f.prop_u32("clock-frequency", 3_686_400);
    f.prop_u32s("interrupts-extended", &[plic_phandle, UART_IRQ]);
    f.end_node();

    // CLINT
    f.begin_node(&format!("clint@{CLINT_BASE:x}"));
    f.prop_strs("compatible", &["sifive,clint0", "riscv,clint0"]);
    f.prop_u64_pair("reg", CLINT_BASE, CLINT_SIZE);
    f.prop_u32s("interrupts-extended", &[intc_phandle, 3, intc_phandle, 7]);
    f.end_node();

    // PLIC
    f.begin_node(&format!("plic@{PLIC_BASE:x}"));
    f.prop_strs("compatible", &["sifive,plic-1.0.0", "riscv,plic0"]);
    f.prop_u32("#interrupt-cells", 1);
    f.prop("interrupt-controller", &[]);
    f.prop_u64_pair("reg", PLIC_BASE, PLIC_SIZE);
    f.prop_u32("riscv,ndev", (PLIC_SOURCES - 1) as u32);
    // contexts: hart0 M-ext (11) then hart0 S-ext (9)
    f.prop_u32s("interrupts-extended", &[intc_phandle, 11, intc_phandle, 9]);
    f.prop_u32("phandle", plic_phandle);
    f.end_node();

    // virtio-mmio slots
    for i in 0..n_virtio {
        let base = VIRTIO_BASE + (i as u64) * VIRTIO_SIZE;
        f.begin_node(&format!("virtio_mmio@{base:x}"));
        f.prop_str("compatible", "virtio,mmio");
        f.prop_u64_pair("reg", base, VIRTIO_SIZE);
        f.prop_u32s("interrupts-extended", &[plic_phandle, VIRTIO_IRQ_BASE + i as u32]);
        f.end_node();
    }

    f.end_node(); // soc
    f.end_node(); // root
    f.finish()
}
