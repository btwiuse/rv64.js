//! virtio-mmio (version 2, "modern") transport with split virtqueues, plus
//! console, block and 9p filesystem backends. Register layout mirrors TinyEMU's
//! virtio.c (which follows the virtio 1.0 spec).

/// Device backends the transport can host.
pub enum Backend {
    /// virtio-console (device id 3). RX = queue 0, TX = queue 1.
    Console { rx_buf: Vec<u8>, tx_out: Vec<u8> },
    /// virtio-blk (device id 2), backed by an in-memory disk image.
    Block { disk: Vec<u8> },
    /// virtio-9p (device id 9): host filesystem sharing. One queue, carrying
    /// 9P2000.L messages that `p9::Server` answers.
    Fs { srv: crate::p9::Server },
}

const MAX_QUEUES: usize = 2;

fn vio_dbg() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("RV_PLIC_DEBUG").is_ok())
}

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
            Backend::Fs { .. } => 9,
        }
    }

    /// Feature bits for word `sel` (0 = bits 0-31, 1 = bits 32-63).
    fn device_features(&self, sel: u32) -> u32 {
        match sel {
            // bit 32: VIRTIO_F_VERSION_1 — modern queue layout.
            1 => 1,
            0 => match self.backend {
                // bit 0: VIRTIO_9P_MOUNT_TAG — config space carries the tag
                // the guest mounts by. Without it the driver refuses to probe.
                Backend::Fs { .. } => 1,
                _ => 0,
            },
            _ => 0,
        }
    }

    /// Ring depth advertised to the driver.
    fn queue_num_max(&self) -> u32 {
        match self.backend {
            // A 9P message up to p9::MAX_MSIZE must fit in ONE descriptor
            // chain, and the client scatters payload across page-sized
            // descriptors — so the ring needs MAX_MSIZE/4096 entries plus
            // headroom. 128 is also the driver's own VIRTQUEUE_NUM.
            Backend::Fs { .. } => 128,
            _ => 16,
        }
    }

    /// True when this device's interrupt line should be raised.
    pub fn irq_pending(&self) -> bool {
        self.int_status != 0
    }

    /// (ready, num, avail_addr, used_addr, my last_avail_idx) for queue `qi`.
    pub fn queue_debug(&self, qi: usize) -> Option<(u32, u32, u64, u64, u16)> {
        self.queues
            .get(qi)
            .filter(|q| q.ready != 0)
            .map(|q| (q.ready, q.num, q.avail, q.used, q.last_avail_idx))
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
            0x010 => self.device_features(self.device_features_sel),
            0x034 => self.queue_num_max(),
            0x044 => self.q().ready,
            0x060 => self.int_status,
            0x070 => self.status,
            0x0fc => 0, // config generation
            _ if offset >= 0x100 => {
                let o = offset - 0x100;
                u32::from_le_bytes([
                    self.config_u8(o),
                    self.config_u8(o + 1),
                    self.config_u8(o + 2),
                    self.config_u8(o + 3),
                ])
            }
            _ => 0,
        }
    }

    /// MMIO read of `size` bytes (1, 2 or 4).
    ///
    /// Only config space is meaningful below word width, and it is not
    /// optional: Linux reads the 9p mount tag one byte at a time
    /// (`virtio_cread_bytes` → `vm_get(len=1)`), so a device that only answers
    /// aligned 32-bit reads never gets mounted.
    pub fn read_sized(&mut self, offset: u64, size: u32) -> u32 {
        if offset >= 0x100 && size < 4 {
            let o = offset - 0x100;
            let mut v = 0u32;
            for i in 0..size as u64 {
                v |= (self.config_u8(o + i) as u32) << (8 * i);
            }
            return v;
        }
        self.read(offset)
    }

    /// Returns true if the write requires queue processing (a notify).
    pub fn write(&mut self, offset: u64, val: u32) -> Option<u32> {
        match offset {
            0x014 => self.device_features_sel = val,
            0x024 => self.driver_features_sel = val,
            0x030 => self.queue_sel = val.min(MAX_QUEUES as u32 - 1),
            0x038 => {
                let max = self.queue_num_max();
                self.qm().num = val.min(max);
            }
            0x044 => self.qm().ready = val & 1,
            0x050 => return Some(val), // QueueNotify -> process queue `val`
            0x064 => {
                if vio_dbg() {
                    eprintln!("[vio] ACK int_status {:#x} &= !{:#x}", self.int_status, val);
                }
                self.int_status &= !val;
            }
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

    /// One byte of device-specific config space (`off` is relative to 0x100).
    fn config_u8(&self, off: u64) -> u8 {
        match &self.backend {
            Backend::Console { .. } => 0, // cols/rows: unused
            Backend::Block { disk } => {
                // struct virtio_blk_config { le64 capacity; ... } in sectors.
                let sectors = (disk.len() / SECTOR) as u64;
                sectors.to_le_bytes().get(off as usize).copied().unwrap_or(0)
            }
            Backend::Fs { srv } => {
                // struct virtio_9p_config { le16 tag_len; u8 tag[tag_len]; }
                let tag = srv.tag().as_bytes();
                match off {
                    0 => tag.len() as u8,
                    1 => (tag.len() >> 8) as u8,
                    _ => tag.get(off as usize - 2).copied().unwrap_or(0),
                }
            }
        }
    }

    // ---- virtqueue processing ------------------------------------------

    /// Process queue `qi` (after a notify, or when console input arrives).
    /// `ram`/`ram_base` give access to guest physical memory.
    pub fn process(&mut self, qi: usize, ram: &mut [u8], ram_base: u64) {
        if qi >= MAX_QUEUES || self.queues[qi].ready == 0 {
            if vio_dbg() {
                eprintln!("[vio] notify q{qi} BAILED ready={}",
                    self.queues.get(qi).map_or(0, |q| q.ready));
            }
            return;
        }
        let mut serviced = 0u32;
        loop {
            let q = self.queues[qi].clone();
            let avail_idx = read16(ram, ram_base, q.avail + 2);
            if q.last_avail_idx == avail_idx {
                if vio_dbg() {
                    eprintln!("[vio] notify q{qi} done serviced={serviced} last_avail={} avail_idx={avail_idx}",
                        q.last_avail_idx);
                }
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
            serviced += 1;
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
            Backend::Fs { srv } => {
                // One chain carries the whole exchange: the device-readable
                // descriptors hold the T-message, the device-writable ones are
                // the reply buffer the client sized to msize.
                let mut req = Vec::new();
                for &(addr, len, writable) in chain {
                    if writable {
                        continue;
                    }
                    match guest_slice(ram, ram_base, addr, len) {
                        Some(s) => req.extend_from_slice(s),
                        // A descriptor pointing outside RAM is a broken guest,
                        // not something to panic over: consume it unanswered.
                        None => return Some(0),
                    }
                }
                if req.is_empty() {
                    return Some(0);
                }
                let reply = srv.handle(&req);
                let mut pos = 0usize;
                for &(addr, len, writable) in chain {
                    if !writable || pos >= reply.len() {
                        continue;
                    }
                    let n = (reply.len() - pos).min(len as usize);
                    match guest_slice_mut(ram, ram_base, addr, n as u32) {
                        Some(d) => d.copy_from_slice(&reply[pos..pos + n]),
                        None => break,
                    }
                    pos += n;
                }
                Some(pos as u32)
            }
        }
    }
}

/// Guest-physical view of a descriptor, or `None` if it does not lie entirely
/// within RAM.
fn guest_slice(ram: &[u8], ram_base: u64, addr: u64, len: u32) -> Option<&[u8]> {
    let off = addr.checked_sub(ram_base)? as usize;
    ram.get(off..off.checked_add(len as usize)?)
}

fn guest_slice_mut(ram: &mut [u8], ram_base: u64, addr: u64, len: u32) -> Option<&mut [u8]> {
    let off = addr.checked_sub(ram_base)? as usize;
    ram.get_mut(off..off.checked_add(len as usize)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p9;
    use crate::p9fs::MemFs;

    // A small guest RAM with a hand-built split virtqueue, so these tests
    // exercise the same path a real driver takes: descriptor chain in, reply
    // scattered into the writable descriptors, entry on the used ring.
    const BASE: u64 = 0x8000_0000;
    const DESC: usize = 0x1000;
    const AVAIL: usize = 0x2000;
    const USED: usize = 0x3000;
    const REQ: usize = 0x4000;
    const REPLY: usize = 0x5000;
    const NUM: u32 = 8;

    fn fs_device() -> (VirtioDev, Vec<u8>) {
        let mut fs = MemFs::new();
        fs.add_file("/greeting", b"from-the-host", 0o644);
        let dev = VirtioDev::new(Backend::Fs {
            srv: p9::Server::new("hostshare", Box::new(fs)),
        });
        (dev, vec![0u8; 64 * 1024])
    }

    fn setup_queue(dev: &mut VirtioDev) {
        dev.write(0x030, 0); // queue_sel = 0
        dev.write(0x038, NUM); // queue_num
        dev.write(0x080, (BASE + DESC as u64) as u32);
        dev.write(0x084, ((BASE + DESC as u64) >> 32) as u32);
        dev.write(0x090, (BASE + AVAIL as u64) as u32);
        dev.write(0x094, ((BASE + AVAIL as u64) >> 32) as u32);
        dev.write(0x0a0, (BASE + USED as u64) as u32);
        dev.write(0x0a4, ((BASE + USED as u64) >> 32) as u32);
        dev.write(0x044, 1); // queue_ready
    }

    fn put_desc(ram: &mut [u8], i: usize, addr: usize, len: u32, flags: u16, next: u16) {
        let o = DESC + i * 16;
        ram[o..o + 8].copy_from_slice(&(BASE + addr as u64).to_le_bytes());
        ram[o + 8..o + 12].copy_from_slice(&len.to_le_bytes());
        ram[o + 12..o + 14].copy_from_slice(&flags.to_le_bytes());
        ram[o + 14..o + 16].copy_from_slice(&next.to_le_bytes());
    }

    fn u16_at(ram: &[u8], off: usize) -> u16 {
        u16::from_le_bytes(ram[off..off + 2].try_into().unwrap())
    }
    fn u32_at(ram: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(ram[off..off + 4].try_into().unwrap())
    }

    /// Body builder for T-messages.
    #[derive(Default)]
    struct B(Vec<u8>);
    impl B {
        fn u16(mut self, v: u16) -> B {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn u32(mut self, v: u32) -> B {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn u64(mut self, v: u64) -> B {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn str(mut self, s: &str) -> B {
            self.0.extend_from_slice(&(s.len() as u16).to_le_bytes());
            self.0.extend_from_slice(s.as_bytes());
            self
        }
    }

    /// Submit one T-message on the ring, notify, and return the R-message the
    /// device wrote back. `n` is the 1-based submission count.
    fn round_trip(dev: &mut VirtioDev, ram: &mut [u8], id: u8, body: &B, n: u16) -> Vec<u8> {
        let mut msg = ((body.0.len() + 7) as u32).to_le_bytes().to_vec();
        msg.push(id);
        msg.extend_from_slice(&0u16.to_le_bytes()); // tag
        msg.extend_from_slice(&body.0);
        ram[REQ..REQ + msg.len()].copy_from_slice(&msg);
        put_desc(ram, 0, REQ, msg.len() as u32, 1 /* NEXT */, 1);
        put_desc(ram, 1, REPLY, 4096, 2 /* WRITE */, 0);
        // avail.ring[slot] = chain head, then publish the new index.
        let slot = (n as usize - 1) % NUM as usize;
        ram[AVAIL + 4 + slot * 2..AVAIL + 6 + slot * 2].copy_from_slice(&0u16.to_le_bytes());
        ram[AVAIL + 2..AVAIL + 4].copy_from_slice(&n.to_le_bytes());

        assert_eq!(dev.write(0x050, 0), Some(0), "QueueNotify selects queue 0");
        dev.process(0, ram, BASE);

        // The device must have published exactly one more used entry.
        assert_eq!(u16_at(ram, USED + 2), n, "used.idx after request {n}");
        let uslot = (n as usize - 1) % NUM as usize;
        assert_eq!(u32_at(ram, USED + 4 + uslot * 8), 0, "used entry is chain 0");
        let len = u32_at(ram, USED + 8 + uslot * 8) as usize;
        let reply = ram[REPLY..REPLY + len].to_vec();
        assert_eq!(u32_at(&reply, 0) as usize, len, "reply size field");
        assert_ne!(reply[4], 7, "unexpected Rlerror: {:?}", &reply[7..11]);
        assert_eq!(reply[4], id + 1, "reply id");
        reply
    }

    #[test]
    fn advertises_a_9p_device_and_its_mount_tag() {
        let (mut dev, _) = fs_device();
        assert_eq!(dev.read(0x008), 9); // virtio device id 9 = 9p
        dev.write(0x014, 0);
        assert_eq!(dev.read(0x010), 1, "VIRTIO_9P_MOUNT_TAG in feature word 0");
        dev.write(0x014, 1);
        assert_eq!(dev.read(0x010), 1, "VIRTIO_F_VERSION_1 in feature word 1");
        assert_eq!(dev.read(0x034), 128, "ring must hold a whole msize message");
        // Read the tag exactly as Linux does: a 16-bit length, then one byte at
        // a time. This is what a 32-bit-only config space would break.
        assert_eq!(dev.read_sized(0x100, 2), 9);
        let tag: Vec<u8> = (0..9)
            .map(|i| dev.read_sized(0x102 + i, 1) as u8)
            .collect();
        assert_eq!(&tag, b"hostshare");
    }

    #[test]
    fn console_and_block_config_still_read_as_words() {
        // Assembling config space from bytes must not disturb the existing
        // devices: virtio-blk's capacity is a le64 sector count at offset 0.
        let mut dev = VirtioDev::new(Backend::Block {
            disk: vec![0u8; 8 * 512],
        });
        assert_eq!(dev.read(0x100), 8);
        assert_eq!(dev.read(0x104), 0);
        assert_eq!(dev.read(0x034), 16, "unchanged ring depth for blk");
        let mut dev = VirtioDev::new(Backend::Console {
            rx_buf: Vec::new(),
            tx_out: Vec::new(),
        });
        assert_eq!(dev.read(0x008), 3);
        dev.write(0x014, 0);
        assert_eq!(dev.read(0x010), 0, "console offers no feature bits");
    }

    #[test]
    fn services_a_mount_and_read_over_the_virtqueue() {
        let (mut dev, mut ram) = fs_device();
        setup_queue(&mut dev);

        // Tversion — negotiate msize.
        let r = round_trip(&mut dev, &mut ram, 100, &B::default().u32(8192).str("9P2000.L"), 1);
        assert_eq!(u32_at(&r, 7), 8192);
        assert!(dev.irq_pending(), "used-ring update must raise the irq");

        // Tattach fid 0 -> the export root.
        round_trip(
            &mut dev,
            &mut ram,
            104,
            &B::default().u32(0).u32(!0).str("root").str("").u32(0),
            2,
        );

        // Twalk fid 0 -> fid 1 = "greeting", then Tlopen and Tread it.
        let r = round_trip(
            &mut dev,
            &mut ram,
            110,
            &B::default().u32(0).u32(1).u16(1).str("greeting"),
            3,
        );
        assert_eq!(u16_at(&r, 7), 1, "one qid walked");
        round_trip(&mut dev, &mut ram, 12, &B::default().u32(1).u32(0), 4);
        let r = round_trip(
            &mut dev,
            &mut ram,
            116,
            &B::default().u32(1).u64(0).u32(4096),
            5,
        );
        let n = u32_at(&r, 7) as usize;
        assert_eq!(&r[11..11 + n], b"from-the-host");
    }

    #[test]
    fn a_descriptor_outside_ram_is_consumed_not_panicked_on() {
        let (mut dev, mut ram) = fs_device();
        setup_queue(&mut dev);
        // A buggy guest can point a descriptor anywhere; the device must not
        // fault the host process over it.
        put_desc(&mut ram, 0, 0, 64, 1, 1);
        let o = DESC;
        ram[o..o + 8].copy_from_slice(&0xdead_0000u64.to_le_bytes());
        put_desc(&mut ram, 1, REPLY, 4096, 2, 0);
        ram[AVAIL + 4..AVAIL + 6].copy_from_slice(&0u16.to_le_bytes());
        ram[AVAIL + 2..AVAIL + 4].copy_from_slice(&1u16.to_le_bytes());
        dev.process(0, &mut ram, BASE);
        assert_eq!(u16_at(&ram, USED + 2), 1, "chain consumed");
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
