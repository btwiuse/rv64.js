//! virtio-mmio (version 2, "modern") transport with split virtqueues, plus
//! console and block device backends. Register layout mirrors TinyEMU's
//! virtio.c (which follows the virtio 1.0 spec).

/// Device backends the transport can host.
pub enum Backend {
    /// virtio-console (device id 3). RX = queue 0, TX = queue 1.
    Console { rx_buf: Vec<u8>, tx_out: Vec<u8> },
    /// virtio-blk (device id 2), backed by an in-memory disk image.
    Block { disk: Vec<u8> },
}

const QUEUE_NUM_MAX: u32 = 16;
const MAX_QUEUES: usize = 2;

#[derive(Default, Clone)]
struct Queue {
    ready: u32,
    num: u32,
    desc: u64,
    avail: u64,
    used: u64,
    last_avail_idx: u16,
}

pub struct VirtioDev {
    pub backend: Backend,
    status: u32,
    device_features_sel: u32,
    driver_features_sel: u32,
    queue_sel: u32,
    queues: [Queue; MAX_QUEUES],
    /// bit 0: used-ring update pending
    pub int_status: u32,
}

// virtio-blk request types
const VIRTIO_BLK_T_IN: u32 = 0; // read
const VIRTIO_BLK_T_OUT: u32 = 1; // write

const SECTOR: usize = 512;

impl VirtioDev {
    pub fn new(backend: Backend) -> VirtioDev {
        VirtioDev {
            backend,
            status: 0,
            device_features_sel: 0,
            driver_features_sel: 0,
            queue_sel: 0,
            queues: Default::default(),
            int_status: 0,
        }
    }

    pub fn device_id(&self) -> u32 {
        match self.backend {
            Backend::Console { .. } => 3,
            Backend::Block { .. } => 2,
        }
    }

    /// True when this device's interrupt line should be raised.
    pub fn irq_pending(&self) -> bool {
        self.int_status != 0
    }

    /// Queue console input; delivered to the guest via the RX virtqueue on
    /// the next `process` call.
    pub fn console_input(&mut self, bytes: &[u8]) {
        if let Backend::Console { rx_buf, .. } = &mut self.backend {
            rx_buf.extend_from_slice(bytes);
        }
    }

    /// Drain console output produced by the guest.
    pub fn console_take_output(&mut self) -> Vec<u8> {
        if let Backend::Console { tx_out, .. } = &mut self.backend {
            core::mem::take(tx_out)
        } else {
            Vec::new()
        }
    }

    pub fn read(&mut self, offset: u64) -> u32 {
        match offset {
            0x000 => 0x7472_6976, // magic "virt"
            0x004 => 2,           // version
            0x008 => self.device_id(),
            0x00c => 0xffff, // vendor
            0x010 => {
                // features: VIRTIO_F_VERSION_1 (bit 32) in word 1
                if self.device_features_sel == 1 {
                    1 // bit 32
                } else {
                    0
                }
            }
            0x034 => QUEUE_NUM_MAX,
            0x044 => self.q().ready,
            0x060 => self.int_status,
            0x070 => self.status,
            0x0fc => 0, // config generation
            _ if offset >= 0x100 => self.config_read(offset - 0x100),
            _ => 0,
        }
    }

    /// Returns true if the write requires queue processing (a notify).
    pub fn write(&mut self, offset: u64, val: u32) -> Option<u32> {
        match offset {
            0x014 => self.device_features_sel = val,
            0x024 => self.driver_features_sel = val,
            0x030 => self.queue_sel = val.min(MAX_QUEUES as u32 - 1),
            0x038 => self.qm().num = val.min(QUEUE_NUM_MAX),
            0x044 => self.qm().ready = val & 1,
            0x050 => return Some(val), // QueueNotify -> process queue `val`
            0x064 => self.int_status &= !val,
            0x070 => {
                self.status = val;
                if val == 0 {
                    // reset
                    self.queues = Default::default();
                    self.int_status = 0;
                }
            }
            0x080 => set_lo(&mut self.qm().desc, val),
            0x084 => set_hi(&mut self.qm().desc, val),
            0x090 => set_lo(&mut self.qm().avail, val),
            0x094 => set_hi(&mut self.qm().avail, val),
            0x0a0 => set_lo(&mut self.qm().used, val),
            0x0a4 => set_hi(&mut self.qm().used, val),
            _ => {}
        }
        None
    }

    fn q(&self) -> &Queue {
        &self.queues[self.queue_sel as usize]
    }
    fn qm(&mut self) -> &mut Queue {
        &mut self.queues[self.queue_sel as usize]
    }

    fn config_read(&self, off: u64) -> u32 {
        match &self.backend {
            Backend::Console { .. } => 0, // cols/rows: unused
            Backend::Block { disk } => {
                let sectors = (disk.len() / SECTOR) as u64;
                match off {
                    0 => sectors as u32,
                    4 => (sectors >> 32) as u32,
                    _ => 0,
                }
            }
        }
    }

    // ---- virtqueue processing ------------------------------------------

    /// Process queue `qi` (after a notify, or when console input arrives).
    /// `ram`/`ram_base` give access to guest physical memory.
    pub fn process(&mut self, qi: usize, ram: &mut [u8], ram_base: u64) {
        if qi >= MAX_QUEUES || self.queues[qi].ready == 0 {
            return;
        }
        loop {
            let q = self.queues[qi].clone();
            let avail_idx = read16(ram, ram_base, q.avail + 2);
            if q.last_avail_idx == avail_idx {
                break;
            }
            let slot = (q.last_avail_idx as u64) % (q.num as u64);
            let head = read16(ram, ram_base, q.avail + 4 + slot * 2);

            // Walk the descriptor chain.
            let mut chain: Vec<(u64, u32, bool)> = Vec::new(); // (addr, len, writable)
            let mut di = head as u64;
            for _ in 0..q.num {
                let base = q.desc + di * 16;
                let addr = read64(ram, ram_base, base);
                let len = read32(ram, ram_base, base + 8);
                let flags = read16(ram, ram_base, base + 12);
                chain.push((addr, len, flags & 2 != 0));
                if flags & 1 == 0 {
                    break;
                }
                di = read16(ram, ram_base, base + 14) as u64;
            }

            let written = self.service(qi, &chain, ram, ram_base);
            if written.is_none() {
                // Not serviceable now (e.g. console RX with no input):
                // leave the descriptor for later.
                break;
            }

            // Publish to the used ring.
            let used_idx = read16(ram, ram_base, q.used + 2);
            let uslot = (used_idx as u64) % (q.num as u64);
            write32(ram, ram_base, q.used + 4 + uslot * 8, head as u32);
            write32(ram, ram_base, q.used + 4 + uslot * 8 + 4, written.unwrap());
            write16(ram, ram_base, q.used + 2, used_idx.wrapping_add(1));

            self.queues[qi].last_avail_idx = self.queues[qi].last_avail_idx.wrapping_add(1);
            self.int_status |= 1;
        }
    }

    /// Service one descriptor chain; returns bytes written to guest buffers,
    /// or None if the request can't be serviced yet.
    fn service(
        &mut self,
        qi: usize,
        chain: &[(u64, u32, bool)],
        ram: &mut [u8],
        ram_base: u64,
    ) -> Option<u32> {
        match &mut self.backend {
            Backend::Console { rx_buf, tx_out } => {
                if qi == 0 {
                    // RX: fill writable buffers with pending input.
                    if rx_buf.is_empty() {
                        return None;
                    }
                    let mut written = 0u32;
                    for &(addr, len, writable) in chain {
                        if !writable || rx_buf.is_empty() {
                            continue;
                        }
                        let n = rx_buf.len().min(len as usize);
                        let off = (addr - ram_base) as usize;
                        ram[off..off + n].copy_from_slice(&rx_buf[..n]);
                        rx_buf.drain(..n);
                        written += n as u32;
                        if rx_buf.is_empty() {
                            break;
                        }
                    }
                    Some(written)
                } else {
                    // TX: collect guest output.
                    for &(addr, len, writable) in chain {
                        if writable {
                            continue;
                        }
                        let off = (addr - ram_base) as usize;
                        tx_out.extend_from_slice(&ram[off..off + len as usize]);
                    }
                    Some(0)
                }
            }
            Backend::Block { disk } => {
                // Layout: header (16B, read-only) | data buffers | status (1B, writable)
                let (hdr_addr, ..) = *chain.first()?;
                let hoff = (hdr_addr - ram_base) as usize;
                let req_type = u32::from_le_bytes(ram[hoff..hoff + 4].try_into().unwrap());
                let sector = u64::from_le_bytes(ram[hoff + 8..hoff + 16].try_into().unwrap());
                let mut pos = sector as usize * SECTOR;
                let mut written = 0u32;
                let mut ok = true;

                for &(addr, len, writable) in &chain[1..chain.len() - 1] {
                    let off = (addr - ram_base) as usize;
                    let len = len as usize;
                    match req_type {
                        VIRTIO_BLK_T_IN if writable => {
                            if pos + len <= disk.len() {
                                ram[off..off + len].copy_from_slice(&disk[pos..pos + len]);
                            } else {
                                ok = false;
                            }
                            written += len as u32;
                        }
                        VIRTIO_BLK_T_OUT if !writable => {
                            if pos + len <= disk.len() {
                                disk[pos..pos + len].copy_from_slice(&ram[off..off + len]);
                            } else {
                                ok = false;
                            }
                        }
                        _ => ok = false,
                    }
                    pos += len;
                }

                // status byte in the last descriptor
                if let Some(&(saddr, _, _)) = chain.last() {
                    let soff = (saddr - ram_base) as usize;
                    ram[soff] = if ok { 0 } else { 1 };
                    written += 1;
                }
                Some(written)
            }
        }
    }
}

fn set_lo(v: &mut u64, val: u32) {
    *v = (*v & !0xffff_ffff) | val as u64;
}
fn set_hi(v: &mut u64, val: u32) {
    *v = (*v & 0xffff_ffff) | ((val as u64) << 32);
}

fn read16(ram: &[u8], base: u64, addr: u64) -> u16 {
    let o = (addr - base) as usize;
    u16::from_le_bytes(ram[o..o + 2].try_into().unwrap())
}
fn read32(ram: &[u8], base: u64, addr: u64) -> u32 {
    let o = (addr - base) as usize;
    u32::from_le_bytes(ram[o..o + 4].try_into().unwrap())
}
fn read64(ram: &[u8], base: u64, addr: u64) -> u64 {
    let o = (addr - base) as usize;
    u64::from_le_bytes(ram[o..o + 8].try_into().unwrap())
}
fn write16(ram: &mut [u8], base: u64, addr: u64, v: u16) {
    let o = (addr - base) as usize;
    ram[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn write32(ram: &mut [u8], base: u64, addr: u64, v: u32) {
    let o = (addr - base) as usize;
    ram[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
