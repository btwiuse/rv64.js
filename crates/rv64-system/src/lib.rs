//! Full-system riscv64 machine, TinyEMU-compatible layout:
//!
//! ```text
//! 0x0000_1000  boot trampoline + DTB (low RAM, 64 KiB)
//! 0x0200_0000  CLINT  (msip, mtimecmp, mtime)
//! 0x4001_0000  virtio-mmio slots (0x1000 apart; console, blk)
//! 0x4010_0000  PLIC   (TinyEMU's minimal claim/complete interface)
//! 0x8000_0000  RAM    (BBL at base, kernel at +2 MiB)
//! ```
//!
//! Boots the same bbl64.bin/kernel/rootfs images TinyEMU ships.

pub mod dtb;
/// Native egress for the HTTP proxy: real sockets, so absent on wasm — the
/// browser uses `fetch()` via `web/rv64.js` instead.
#[cfg(not(target_arch = "wasm32"))]
pub mod egress;
pub mod httpproxy;
pub mod netstack;
pub mod p9;
pub mod p9fs;
pub mod rtc;
pub mod tlsproxy;
pub mod virt;
pub mod virtio;
/// WebSocket relay transport for virtio-net. Host-side networking, so absent on
/// wasm — the browser uses its own `WebSocket` via `web/rv64.js` instead.
#[cfg(not(target_arch = "wasm32"))]
pub mod ws;

use rv64_core::csr::{IRQ_MEIP, IRQ_MSIP, IRQ_MTIP, IRQ_SEIP};
use rv64_core::{Bus, Cpu, Exception, StopReason};
use virtio::{Backend, VirtioDev};

pub const RAM_BASE: u64 = 0x8000_0000;
pub const LOW_RAM_SIZE: usize = 0x1_0000;
pub const CLINT_BASE: u64 = 0x0200_0000;
pub const CLINT_SIZE: u64 = 0xc_0000;
pub const VIRTIO_BASE: u64 = 0x4001_0000;
pub const VIRTIO_SIZE: u64 = 0x1000;
pub const PLIC_BASE: u64 = 0x4010_0000;
pub const PLIC_SIZE: u64 = 0x40_0000;
pub const HTIF_BASE: u64 = 0x4000_8000;
pub const GOLDFISH_RTC_BASE: u64 = 0x0010_1000;
pub const GOLDFISH_RTC_SIZE: u64 = 0x1000;
pub const GOLDFISH_RTC_IRQ: u32 = 11;
/// 10 MHz timebase, like TinyEMU's RTC_FREQ.
pub const RTC_FREQ: u64 = 10_000_000;

/// Current Unix-epoch time for native machine embeddings. Browser embeddings
/// provide the equivalent value through their host ABI.
#[cfg(not(target_arch = "wasm32"))]
pub fn host_unix_time_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

/// Guest-physical bus: RAM + devices. The CPU hands us *physical*
/// addresses (its MMU translated already).
pub struct SystemBus {
    pub ram: Vec<u8>,
    pub low_ram: Vec<u8>,
    // CLINT
    pub mtime: u64,
    pub mtimecmp: u64,
    pub msip: bool,
    pub rtc: rtc::GoldfishRtc,
    // PLIC (TinyEMU-style: pending & served masks, claim/complete only)
    pub plic_pending: u32,
    pub plic_served: u32,
    // virtio devices (irq = index + 1)
    pub virtio: Vec<VirtioDev>,
    // HTIF (BBL early console + power off; riscv-tests result channel)
    pub htif_base: u64,
    pub htif_tohost: u64,
    pub htif_fromhost: u64,
    pub htif_console: Vec<u8>,
    pub power_off: bool,
    /// Set when the guest writes an odd value to tohost: value >> 1
    /// (riscv-tests: 0 = pass, n = failing test case number).
    pub htif_exit: Option<u64>,
    // JIT support: bitset of RAM pages holding compiled code. A store to a
    // marked page records that page in jit_dirty_pages, so the dispatcher
    // can drop just the affected blocks (self-modifying code / page reuse).
    pub jit_pages: Vec<u64>,
    /// Physical pages (page number = (pa-RAM_BASE)>>12) written since the
    /// last drain that held compiled code — the dispatcher invalidates just
    /// those blocks instead of the whole cache.
    pub jit_dirty_pages: Vec<u64>,
}

impl SystemBus {
    fn mmio_read32(&mut self, addr: u64) -> Option<u32> {
        match addr {
            _ if (CLINT_BASE..CLINT_BASE + CLINT_SIZE).contains(&addr) => {
                Some(match addr - CLINT_BASE {
                    0x0 => self.msip as u32,
                    0x4000 => self.mtimecmp as u32,
                    0x4004 => (self.mtimecmp >> 32) as u32,
                    0xbff8 => self.mtime as u32,
                    0xbffc => (self.mtime >> 32) as u32,
                    _ => 0,
                })
            }
            _ if (PLIC_BASE..PLIC_BASE + PLIC_SIZE).contains(&addr) => {
                Some(match addr - PLIC_BASE {
                    // hart0 claim register
                    0x20_0004 => {
                        let mask = self.plic_pending & !self.plic_served;
                        if mask != 0 {
                            let i = mask.trailing_zeros();
                            self.plic_served |= 1 << i;
                            i + 1
                        } else {
                            0
                        }
                    }
                    _ => 0,
                })
            }
            _ if (GOLDFISH_RTC_BASE..GOLDFISH_RTC_BASE + GOLDFISH_RTC_SIZE).contains(&addr) => {
                Some(self.rtc.read(addr - GOLDFISH_RTC_BASE))
            }
            _ if (self.htif_base..self.htif_base + 16).contains(&addr) => {
                Some(match addr - self.htif_base {
                    0 => self.htif_tohost as u32,
                    4 => (self.htif_tohost >> 32) as u32,
                    8 => self.htif_fromhost as u32,
                    12 => (self.htif_fromhost >> 32) as u32,
                    _ => 0,
                })
            }
            _ if (VIRTIO_BASE..VIRTIO_BASE + 8 * VIRTIO_SIZE).contains(&addr) => {
                let i = ((addr - VIRTIO_BASE) / VIRTIO_SIZE) as usize;
                let off = (addr - VIRTIO_BASE) % VIRTIO_SIZE;
                self.virtio.get_mut(i).map(|d| d.read(off))
            }
            _ => None,
        }
    }

    /// Sub-word MMIO read. Only the virtio config window answers below 32 bits,
    /// and only because it must: Linux reads a 9p mount tag one byte at a time
    /// (`virtio_cread_bytes`), so refusing narrow reads means never mounting.
    fn mmio_read_narrow(&mut self, addr: u64, size: usize) -> Option<u32> {
        if (VIRTIO_BASE..VIRTIO_BASE + 8 * VIRTIO_SIZE).contains(&addr) {
            let i = ((addr - VIRTIO_BASE) / VIRTIO_SIZE) as usize;
            let off = (addr - VIRTIO_BASE) % VIRTIO_SIZE;
            return self
                .virtio
                .get_mut(i)
                .map(|d| d.read_sized(off, size as u32));
        }
        None
    }

    fn htif_handle_cmd(&mut self) {
        let device = self.htif_tohost >> 56;
        let cmd = (self.htif_tohost >> 48) & 0xff;
        if self.htif_tohost & 1 == 1 && device == 0 {
            // Test/shutdown exit: 1 = clean poweroff (exit 0); odd value
            // (n<<1)|1 = riscv-tests failure in case n.
            self.htif_exit = Some(self.htif_tohost >> 1);
            self.power_off = true;
        } else if device == 1 && cmd == 1 {
            self.htif_console.push(self.htif_tohost as u8);
            self.htif_tohost = 0;
            self.htif_fromhost = (device << 56) | (cmd << 48);
        } else if device == 1 && cmd == 0 {
            self.htif_tohost = 0; // keyboard irq request: ignore
        } else {
            self.htif_tohost = 0;
        }
    }

    fn mmio_write32(&mut self, addr: u64, val: u32) -> bool {
        match addr {
            _ if (CLINT_BASE..CLINT_BASE + CLINT_SIZE).contains(&addr) => {
                match addr - CLINT_BASE {
                    0x0 => self.msip = val & 1 != 0,
                    0x4000 => self.mtimecmp = (self.mtimecmp & !0xffff_ffff) | val as u64,
                    0x4004 => self.mtimecmp = (self.mtimecmp & 0xffff_ffff) | ((val as u64) << 32),
                    _ => {}
                }
                true
            }
            _ if (PLIC_BASE..PLIC_BASE + PLIC_SIZE).contains(&addr) => {
                if addr - PLIC_BASE == 0x20_0004 {
                    // complete
                    let irq = val.wrapping_sub(1);
                    if irq < 32 {
                        self.plic_served &= !(1 << irq);
                    }
                }
                true
            }
            _ if (GOLDFISH_RTC_BASE..GOLDFISH_RTC_BASE + GOLDFISH_RTC_SIZE).contains(&addr) => {
                self.rtc.write(addr - GOLDFISH_RTC_BASE, val);
                self.refresh_plic();
                true
            }
            _ if (self.htif_base..self.htif_base + 16).contains(&addr) => {
                match addr - self.htif_base {
                    0 => {
                        self.htif_tohost = (self.htif_tohost & !0xffff_ffff) | val as u64;
                        // command fires on the high-word write (TinyEMU-compatible)
                    }
                    4 => {
                        self.htif_tohost = (self.htif_tohost & 0xffff_ffff) | ((val as u64) << 32);
                        self.htif_handle_cmd();
                    }
                    8 => self.htif_fromhost = (self.htif_fromhost & !0xffff_ffff) | val as u64,
                    12 => {
                        self.htif_fromhost =
                            (self.htif_fromhost & 0xffff_ffff) | ((val as u64) << 32)
                    }
                    _ => {}
                }
                true
            }
            _ if (VIRTIO_BASE..VIRTIO_BASE + 8 * VIRTIO_SIZE).contains(&addr) => {
                let i = ((addr - VIRTIO_BASE) / VIRTIO_SIZE) as usize;
                let off = (addr - VIRTIO_BASE) % VIRTIO_SIZE;
                if i < self.virtio.len() {
                    if let Some(q) = self.virtio[i].write(off, val) {
                        // QueueNotify: run the virtqueue against RAM.
                        let mut dev = self.virtio.remove(i);
                        dev.process(q as usize, &mut self.ram, RAM_BASE);
                        self.virtio.insert(i, dev);
                    }
                    self.refresh_plic();
                }
                true
            }
            _ => false,
        }
    }

    /// The virtio-net device, if this machine has one.
    fn net_dev(&mut self) -> Option<&mut VirtioDev> {
        self.virtio.iter_mut().find(|d| d.device_id() == 1)
    }

    /// Deliver any inbound frames the guest now has RX buffers for.
    ///
    /// QueueNotify does not cover this case: frames arrive from the host
    /// *between* notifications, so without a poll an inbound frame would wait
    /// until the guest happened to touch the device for some other reason.
    pub fn poll_net_rx(&mut self) {
        let mut delivered = false;
        for i in 0..self.virtio.len() {
            if !self.virtio[i].net_rx_pending() {
                continue;
            }
            let mut dev = self.virtio.remove(i);
            dev.process(0, &mut self.ram, RAM_BASE);
            self.virtio.insert(i, dev);
            delivered = true;
        }
        if delivered {
            self.refresh_plic();
        }
    }

    /// Recompute PLIC pending bits from device interrupt lines.
    pub fn refresh_plic(&mut self) {
        let mut pending = 0u32;
        for (i, d) in self.virtio.iter().enumerate() {
            if d.irq_pending() {
                pending |= 1 << i; // irq number = i+1 -> bit i
            }
        }
        if self.rtc.irq() {
            pending |= 1 << (GOLDFISH_RTC_IRQ - 1);
        }
        self.plic_pending = pending;
    }

    /// External interrupt line state for mip (MEIP|SEIP), TinyEMU-style.
    pub fn plic_irq_active(&self) -> bool {
        self.plic_pending & !self.plic_served != 0
    }

    /// Mark a RAM page as containing JIT-compiled code.
    pub fn jit_mark_page(&mut self, pa: u64) {
        if pa >= RAM_BASE {
            let page = ((pa - RAM_BASE) >> 12) as usize;
            if let Some(w) = self.jit_pages.get_mut(page / 64) {
                *w |= 1 << (page % 64);
            }
        }
    }

    /// Clear a single compiled-code page's bit (after its blocks are dropped).
    /// Is this physical page still marked as holding compiled code? Used by
    /// async superblock registration to reject a compile whose source page
    /// was written (and therefore invalidated) while compiling.
    pub fn jit_page_marked(&self, page: u64) -> bool {
        self.jit_pages
            .get(page as usize / 64)
            .is_some_and(|w| w & (1 << (page % 64)) != 0)
    }

    pub fn jit_unmark_page(&mut self, page: u64) {
        if let Some(w) = self.jit_pages.get_mut(page as usize / 64) {
            *w &= !(1 << (page % 64));
        }
    }

    /// Take the list of dirtied compiled-code pages (drains it).
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
        } else if addr >= 0x1000 && (addr as usize) + len <= LOW_RAM_SIZE {
            let off = addr as usize;
            return Some(&mut self.low_ram[off..off + len]);
        }
        None
    }
}

impl SystemBus {
    /// riscv-tests link `tohost` inside RAM; the HTIF window must win over
    /// plain memory dispatch.
    #[inline]
    fn htif_hit(&self, addr: u64) -> bool {
        addr.wrapping_sub(self.htif_base) < 16
    }
}

macro_rules! sys_rw {
    ($rd:ident, $wr:ident, $ty:ty, $n:expr) => {
        fn $rd(&mut self, addr: u64) -> Result<$ty, Exception> {
            if !self.htif_hit(addr) {
                if let Some(s) = self.ram_slice(addr, $n) {
                    let b: [u8; $n] = (&*s).try_into().unwrap();
                    return Ok(<$ty>::from_le_bytes(b));
                }
            }
            // Sub-word MMIO (virtio config space only).
            if $n == 1 || $n == 2 {
                if let Some(v) = self.mmio_read_narrow(addr, $n) {
                    return Ok(v as $ty);
                }
            }
            // MMIO: otherwise only aligned 32-bit accesses are meaningful.
            if $n == 4 {
                if let Some(v) = self.mmio_read32(addr) {
                    return Ok(v as $ty);
                }
            }
            if $n == 8 {
                // allow 64-bit mtime reads
                if let (Some(lo), Some(hi)) = (self.mmio_read32(addr), self.mmio_read32(addr + 4)) {
                    return Ok((((hi as u64) << 32) | lo as u64) as $ty);
                }
            }
            Err(Exception::LoadAccessFault { addr })
        }
        fn $wr(&mut self, addr: u64, val: $ty) -> Result<(), Exception> {
            self.jit_check_store(addr);
            if !self.htif_hit(addr) {
                if let Some(s) = self.ram_slice(addr, $n) {
                    s.copy_from_slice(&val.to_le_bytes());
                    return Ok(());
                }
            }
            if $n == 4 && self.mmio_write32(addr, val as u32) {
                return Ok(());
            }
            if $n == 8 {
                let v = val as u64;
                if self.mmio_write32(addr, v as u32)
                    && self.mmio_write32(addr + 4, (v >> 32) as u32)
                {
                    return Ok(());
                }
            }
            Err(Exception::StoreAccessFault { addr })
        }
    };
}

impl Bus for SystemBus {
    sys_rw!(read8, write8, u8, 1);
    sys_rw!(read16, write16, u16, 2);
    sys_rw!(read32, write32, u32, 4);
    sys_rw!(read64, write64, u64, 8);

    fn irq_lines(&mut self) -> u64 {
        let mut lines = 0u64;
        if self.mtime >= self.mtimecmp {
            lines |= IRQ_MTIP;
        }
        if self.msip {
            lines |= IRQ_MSIP;
        }
        if self.plic_irq_active() {
            lines |= IRQ_MEIP | IRQ_SEIP;
        }
        lines
    }

    fn jit_fast_off(&self, va: u64, pa: u64, store: bool) -> Option<i64> {
        // The whole 4K page must lie in guest RAM (not MMIO / device space).
        if pa < RAM_BASE || (pa | 0xfff) >= RAM_BASE + self.ram.len() as u64 {
            return None;
        }
        if store {
            // A store to a page holding compiled code must bail so the block is
            // invalidated — so don't fast-path such pages.
            let ppage = ((pa - RAM_BASE) >> 12) as usize;
            if let Some(w) = self.jit_pages.get(ppage / 64) {
                if (w >> (ppage & 63)) & 1 != 0 {
                    return None;
                }
            }
        }
        // linear_index = ram.as_ptr() + (pa - RAM_BASE); store off = linear - va.
        Some(self.ram.as_ptr() as i64 + (pa as i64 - RAM_BASE as i64) - va as i64)
    }
}

/// The assembled machine: hart + bus + boot state.
pub struct Machine {
    pub cpu: Cpu,
    pub bus: SystemBus,
    /// Instructions per mtime tick (insn rate / RTC_FREQ).
    pub insns_per_tick: u64,
    pub power_off: bool,
    /// Opt-in wall-clock time source (host nanoseconds). `None` (default) =
    /// deterministic instruction-counted time (mtime = insn_count/insns_per_tick),
    /// which our lockstep/differential testing relies on. When `Some(ns)` the
    /// CLINT tracks real host time instead, so guest `gettimeofday`/`clock` and
    /// self-timing benchmarks (nbench) reflect real throughput — the host layer
    /// refreshes it each slice. Monotonic-clamped so it never runs backward.
    pub wall_ns: Option<u64>,
    /// insn_count at the moment `wall_ns` was last refreshed. Between (rate-
    /// limited) refreshes, mtime advances by insns-since-anchor so guest-visible
    /// time never freezes — code that spins on rdtime (kernel __delay, seqlock
    /// retries) must always observe progress or it degenerates into millions of
    /// single-instruction interpreter round-trips.
    pub wall_anchor_icount: u64,
}

pub struct BootImages<'a> {
    pub bios: &'a [u8],
    pub kernel: Option<&'a [u8]>,
    pub cmdline: &'a str,
    pub disk: Option<Vec<u8>>,
    /// Host filesystems to export over virtio-9p. The guest mounts each by its
    /// server tag (`mount -t 9p -o trans=virtio <tag> /mnt`), or can boot from
    /// one directly with `rootfstype=9p` when its tag is `/dev/root`.
    pub fs: Vec<p9::Server>,
    /// MAC address for a virtio-net device, or `None` for no networking. The
    /// device only moves Ethernet frames; the host layer decides where they go
    /// (see [`ws`] for the WebSocket relay).
    pub net: Option<[u8; 6]>,
}

impl Machine {
    pub fn new(ram_mb: usize, images: BootImages) -> Machine {
        let ram_size = ram_mb << 20;
        let mut ram = vec![0u8; ram_size];

        // BBL at RAM base.
        ram[..images.bios.len()].copy_from_slice(images.bios);
        // Kernel at +2 MiB (TinyEMU's rv64 alignment).
        let mut kernel_base = 0usize;
        let mut kernel_len = 0usize;
        if let Some(k) = images.kernel {
            let align = 2 << 20;
            kernel_base = (images.bios.len() + align - 1) & !(align - 1);
            ram[kernel_base..kernel_base + k.len()].copy_from_slice(k);
            kernel_len = k.len();
        }

        // Devices: console is virtio slot 0 (irq 1), then blk, then 9p — each
        // slot's irq is its index + 1.
        let mut virtio = vec![VirtioDev::new(Backend::Console {
            rx_buf: Vec::new(),
            tx_out: Vec::new(),
        })];
        if let Some(disk) = images.disk {
            virtio.push(VirtioDev::new(Backend::Block { disk }));
        }
        for srv in images.fs {
            virtio.push(VirtioDev::new(Backend::Fs { srv }));
        }
        if let Some(mac) = images.net {
            virtio.push(VirtioDev::new(Backend::Net {
                mac,
                inbox: Vec::new(),
                outbox: Vec::new(),
            }));
        }
        let ndevs = virtio.len();

        // DTB, matching TinyEMU's riscv_build_fdt.
        let dtb = build_fdt(
            ram_size as u64,
            RAM_BASE + kernel_base as u64,
            kernel_len as u64,
            images.cmdline,
            ndevs,
        );

        // Low RAM: trampoline at 0x1000, DTB after it (same as TinyEMU).
        let mut low_ram = vec![0u8; LOW_RAM_SIZE];
        let fdt_addr = 0x1000u64 + 8 * 8;
        low_ram[fdt_addr as usize..fdt_addr as usize + dtb.len()].copy_from_slice(&dtb);
        let tramp: [u32; 5] = [
            0x297 + (RAM_BASE - 0x1000) as u32, // auipc t0, jump_addr
            0x597,                              // auipc a1, 0
            0x58593 + (((fdt_addr - 0x1004) as u32) << 20), // addi a1, a1, dtb-4
            0xf140_2573,                        // csrr a0, mhartid
            0x0002_8067,                        // jalr zero, t0, 0
        ];
        for (i, w) in tramp.iter().enumerate() {
            low_ram[0x1000 + i * 4..0x1000 + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }

        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        cpu.pc = 0x1000;

        Machine {
            cpu,
            bus: SystemBus {
                ram,
                low_ram,
                mtime: 0,
                mtimecmp: u64::MAX,
                msip: false,
                rtc: rtc::GoldfishRtc::new(),
                plic_pending: 0,
                plic_served: 0,
                virtio,
                htif_base: HTIF_BASE,
                htif_tohost: 0,
                htif_fromhost: 0,
                htif_console: Vec::new(),
                power_off: false,
                htif_exit: None,
                jit_pages: vec![0u64; (ram_size >> 12).div_ceil(64)],
                jit_dirty_pages: Vec::new(),
            },
            insns_per_tick: 10, // pretend 100 Minsn/s against the 10 MHz clock
            power_off: false,
            wall_ns: None, // deterministic instruction-counted time by default
            wall_anchor_icount: 0,
        }
    }

    /// Feed console input (keyboard) to the guest.
    pub fn console_input(&mut self, bytes: &[u8]) {
        if let Some(dev) = self.bus.virtio.first_mut() {
            dev.console_input(bytes);
            // Try to deliver immediately via the RX queue.
            let mut d = self.bus.virtio.remove(0);
            d.process(0, &mut self.bus.ram, RAM_BASE);
            self.bus.virtio.insert(0, d);
            self.bus.refresh_plic();
        }
    }

    /// Collect console output produced by the guest (HTIF early console +
    /// virtio-console, in arrival order approximation).
    pub fn console_output(&mut self) -> Vec<u8> {
        let mut out = core::mem::take(&mut self.bus.htif_console);
        if let Some(d) = self.bus.virtio.first_mut() {
            out.extend(d.console_take_output());
        }
        out
    }

    /// Deliver an inbound Ethernet frame to the guest's NIC. Silently ignored
    /// when the machine has no network device.
    pub fn net_input(&mut self, frame: &[u8]) {
        if let Some(dev) = self.bus.net_dev() {
            dev.net_input(frame);
        }
        self.bus.poll_net_rx();
    }

    /// Collect the Ethernet frames the guest has transmitted, for the host to
    /// forward (to a relay, a tap device, wherever).
    pub fn net_take_output(&mut self) -> Vec<Vec<u8>> {
        self.bus
            .net_dev()
            .map(|d| d.net_take_output())
            .unwrap_or_default()
    }

    /// Supply the RTC's Unix-epoch time from the embedding host.
    pub fn set_rtc_unix_ns(&mut self, ns: u64) {
        self.bus.rtc.set_host_time_ns(ns);
        self.bus.refresh_plic();
    }

    /// Run one slice; returns instructions retired.
    pub fn run_slice(&mut self, max_insns: u64) -> u64 {
        let start = self.cpu.insn_count;

        // Update timers + interrupt lines before entering the CPU.
        self.sync_devices();
        // Frames the host handed us since the last slice may now have buffers.
        self.bus.poll_net_rx();

        match self.cpu.run(&mut self.bus, max_insns) {
            StopReason::Wfi => {
                // Idle: skip time forward to the next timer event.
                let next = self.bus.mtimecmp;
                if next != u64::MAX && next > self.bus.mtime {
                    self.cpu.insn_count += (next - self.bus.mtime) * self.insns_per_tick;
                }
            }
            StopReason::Budget => {}
            // Full-system mode routes everything else internally.
            _ => {}
        }
        self.sync_devices();
        self.power_off = self.bus.power_off;
        self.cpu.insn_count - start
    }

    /// Direct JIT-off slice using the integrated scalar interpreter driver.
    /// Generated-code fallback deliberately continues to call [`Self::run_slice`].
    pub fn run_slice_integrated(&mut self, max_insns: u64) -> u64 {
        let start = self.cpu.insn_count;

        self.sync_devices();
        self.bus.poll_net_rx();

        match self.cpu.run_integrated_scalar(&mut self.bus, max_insns) {
            StopReason::Wfi => {
                let next = self.bus.mtimecmp;
                if next != u64::MAX && next > self.bus.mtime {
                    self.cpu.insn_count += (next - self.bus.mtime) * self.insns_per_tick;
                }
            }
            StopReason::Budget => {}
            _ => {}
        }
        self.sync_devices();
        self.power_off = self.bus.power_off;
        self.cpu.insn_count - start
    }

    /// Interpret up to `max_insns`, but stop as soon as `compiled(pc)` reports
    /// the pc has reached a JIT-compiled block — so the interpreter never
    /// overshoots into compiled code (which the JIT should run instead). Used by
    /// the system JIT dispatcher's warm-interp fallback. Runs one instruction at
    /// a time (interrupts/exceptions handled by `cpu.run`), refreshing the clock
    /// only periodically so the per-instruction cost stays near the interpreter's.
    pub fn run_slice_until(
        &mut self,
        max_insns: u64,
        mut compiled: impl FnMut(u64) -> bool,
    ) -> (u64, bool) {
        let start = self.cpu.insn_count;
        self.sync_devices();
        self.bus.poll_net_rx();
        let mut i = 0u64;
        let mut yielded = false;
        while i < max_insns {
            if let StopReason::Wfi = self.cpu.run(&mut self.bus, 1) {
                yielded = true;
                let next = self.bus.mtimecmp;
                if next != u64::MAX && next > self.bus.mtime {
                    self.cpu.insn_count += (next - self.bus.mtime) * self.insns_per_tick;
                }
                break;
            }
            i = self.cpu.insn_count - start;
            if compiled(self.cpu.pc) {
                break; // reached compiled code — hand back to the JIT
            }
            if i & 63 == 0 {
                self.sync_devices();
            }
        }
        self.sync_devices();
        self.power_off = self.bus.power_off;
        (self.cpu.insn_count - start, yielded)
    }

    /// Advance the CLINT clock (interrupt lines are sampled live by the CPU
    /// via Bus::irq_lines; nothing else to propagate).
    pub fn sync_devices(&mut self) {
        let next = match self.wall_ns {
            // ns → 10 MHz ticks (1e9 / RTC_FREQ = 100 ns/tick), plus insn-count
            // interpolation since the last (rate-limited) host clock read so
            // time visibly advances between refreshes (see wall_anchor_icount).
            // The interpolation rate deliberately ASSUMES a fast host (500
            // Minsn/s, 50 insns/tick): undershooting real time is safe (the
            // next refresh jumps mtime forward), but overshooting makes the
            // refresh clamp mtime into a frozen plateau — and frozen guest
            // time turns any rdtime spin (kernel __delay) into an interpreter
            // round-trip storm.
            Some(ns) => {
                // Clamp the interpolated advance: bulk-copy fast paths retire
                // guest instructions far faster than the assumed 500 Minsn/s,
                // and an unclamped extrapolation makes guest time run FAST —
                // deflating every self-timed benchmark score. The host layer
                // refreshes the real clock at quantum boundaries, so the
                // clamp bounds the error to ~33us of guest time per refresh.
                ns / (1_000_000_000 / RTC_FREQ)
                    + self
                        .cpu
                        .insn_count
                        .wrapping_sub(self.wall_anchor_icount)
                        .min(16384)
                        / 50
            }
            None => self.cpu.insn_count / self.insns_per_tick,
        };
        // Never let the clock run backward (host wall-clock can be non-monotonic).
        self.bus.mtime = next.max(self.bus.mtime);
        let sys = self.cpu.sys.as_mut().unwrap();
        sys.mtime = self.bus.mtime;
    }
}

fn build_fdt(
    ram_size: u64,
    kernel_start: u64,
    kernel_len: u64,
    cmdline: &str,
    ndevs: usize,
) -> Vec<u8> {
    let mut f = dtb::Fdt::new();
    let intc_phandle = 1u32;
    let plic_phandle = 2u32;

    f.begin_node("");
    f.prop_u32("#address-cells", 2);
    f.prop_u32("#size-cells", 2);
    f.prop_str("compatible", "ucbbar,riscvemu-bar_dev");
    f.prop_str("model", "ucbbar,riscvemu-bare");

    f.begin_node("cpus");
    f.prop_u32("#address-cells", 1);
    f.prop_u32("#size-cells", 0);
    f.prop_u32("timebase-frequency", RTC_FREQ as u32);
    f.begin_node("cpu@0");
    f.prop_str("device_type", "cpu");
    f.prop_u32("reg", 0);
    f.prop_str("status", "okay");
    f.prop_str("compatible", "riscv");
    f.prop_str("riscv,isa-base", "rv64i");
    f.prop_strs(
        "riscv,isa-extensions",
        &["i", "m", "a", "f", "d", "c", "zicntr", "zicsr", "zifencei"],
    );
    // The legacy-tinyemu board still boots the bundled Linux 4.15 image,
    // which predates the structured ISA binding. Do not copy this deprecated
    // compatibility property to the release riscv-virt machine.
    f.prop_str("riscv,isa", "rv64imafdcsu");
    f.prop_str("mmu-type", "riscv,sv48");
    f.prop_u32("clock-frequency", 2_000_000_000);
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

    f.begin_node("htif");
    f.prop_str("compatible", "ucb,htif0");
    f.end_node();

    f.begin_node("soc");
    f.prop_u32("#address-cells", 2);
    f.prop_u32("#size-cells", 2);
    f.prop_strs("compatible", &["ucbbar,riscvemu-bar-soc", "simple-bus"]);
    f.prop("ranges", &[]);

    f.begin_node(&format!("clint@{CLINT_BASE:x}"));
    f.prop_str("compatible", "riscv,clint0");
    f.prop_u32s("interrupts-extended", &[intc_phandle, 3, intc_phandle, 7]);
    f.prop_u64_pair("reg", CLINT_BASE, CLINT_SIZE);
    f.end_node();

    f.begin_node(&format!("plic@{PLIC_BASE:x}"));
    f.prop_u32("#interrupt-cells", 1);
    f.prop("interrupt-controller", &[]);
    f.prop_str("compatible", "riscv,plic0");
    f.prop_u32("riscv,ndev", 31);
    f.prop_u64_pair("reg", PLIC_BASE, PLIC_SIZE);
    f.prop_u32s("interrupts-extended", &[intc_phandle, 9, intc_phandle, 11]);
    f.prop_u32("phandle", plic_phandle);
    f.end_node();

    f.begin_node(&format!("rtc@{GOLDFISH_RTC_BASE:x}"));
    f.prop_str("compatible", "google,goldfish-rtc");
    f.prop_u64_pair("reg", GOLDFISH_RTC_BASE, GOLDFISH_RTC_SIZE);
    f.prop_u32s("interrupts-extended", &[plic_phandle, GOLDFISH_RTC_IRQ]);
    f.end_node();

    for i in 0..ndevs {
        let base = VIRTIO_BASE + (i as u64) * VIRTIO_SIZE;
        f.begin_node(&format!("virtio@{base:x}"));
        f.prop_str("compatible", "virtio,mmio");
        f.prop_u64_pair("reg", base, VIRTIO_SIZE);
        f.prop_u32s("interrupts-extended", &[plic_phandle, (i + 1) as u32]);
        f.end_node();
    }
    f.end_node(); // soc

    f.begin_node("chosen");
    f.prop_str("bootargs", cmdline);
    if kernel_len > 0 {
        f.prop("riscv,kernel-start", &kernel_start.to_be_bytes());
        f.prop(
            "riscv,kernel-end",
            &(kernel_start + kernel_len).to_be_bytes(),
        );
    }
    f.end_node();

    f.end_node(); // root
    f.finish()
}
