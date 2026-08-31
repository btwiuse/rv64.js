//! virtio-mmio (version 2, "modern") transport with split virtqueues, plus
//! console, block, 9p filesystem and network backends. Register layout mirrors
//! TinyEMU's virtio.c (which follows the virtio 1.0 spec).

use std::collections::{BTreeMap, VecDeque};

type Descriptor = (u64, u32, bool);

const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;
const VIRTQ_DESC_F_INDIRECT: u16 = 4;
const DESCRIPTOR_BYTES: u64 = 16;

/// Guest descriptor state retained while an external 9P server handles a
/// request. 9P tags, rather than submission order, identify completions.
pub struct ExternalP9Pending {
    head: u16,
    chain: Vec<Descriptor>,
}

/// Device backends the transport can host.
pub enum Backend {
    /// virtio-console (device id 3). RX = queue 0, TX = queue 1.
    Console {
        rx_buf: Vec<u8>,
        tx_out: Vec<u8>,
        cols: u16,
        rows: u16,
    },
    /// virtio-blk (device id 2), backed by an in-memory disk image.
    Block { disk: Vec<u8> },
    /// virtio-9p (device id 9): host filesystem sharing. One queue, carrying
    /// 9P2000.L messages that `p9::Server` answers.
    Fs { srv: crate::p9::Server },
    /// virtio-9p whose server lives in the JavaScript host. Available chains
    /// are detached from the virtqueue and may complete out of order by 9P tag.
    FsExternal {
        tag: String,
        requests: VecDeque<Vec<u8>>,
        pending: BTreeMap<u16, ExternalP9Pending>,
    },
    /// virtio-net (device id 1). RX = queue 0, TX = queue 1.
    ///
    /// The device works purely at layer 2: it moves whole Ethernet frames
    /// between the guest's queues and these two mailboxes, and knows nothing
    /// about ARP, IP or TCP. Where the frames actually go is the host layer's
    /// problem — see `ws.rs` and `web/rv64.js` for the WebSocket relay.
    Net {
        mac: [u8; 6],
        /// Frames from the host, awaiting an RX buffer from the guest.
        inbox: VecDeque<Vec<u8>>,
        /// Frames the guest has sent, awaiting collection by the host.
        outbox: Vec<Vec<u8>>,
    },
}

const MAX_QUEUES: usize = 2;

/// Bytes of `struct virtio_net_hdr_v1` in front of every frame in both
/// directions. 12 rather than 10 because we negotiate `VIRTIO_F_VERSION_1`,
/// which makes the header carry `num_buffers` — Linux sizes `vi->hdr_len` on
/// exactly that condition, so a 10-byte header would desynchronise every frame.
const NET_HDR_LEN: usize = 12;

/// Largest frame we will move: Ethernet MTU + header + VLAN slack.
const NET_MAX_FRAME: usize = 1600;

/// MAC handed to the guest unless the caller picks one. Locally administered
/// (bit 1 of the first octet), unicast. Two guests sharing one relay must NOT
/// share this — give the second a different address.
pub const DEFAULT_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

/// How many frames may queue up in either direction before we drop. A guest
/// that stops posting RX buffers, or a host that stops collecting, must not
/// grow our memory without bound.
const NET_QUEUE_LIMIT: usize = 256;

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

enum ServiceResult {
    /// The chain completed synchronously and can enter the used ring now.
    Complete(u32),
    /// The chain was retained by an asynchronous backend. Advance avail only.
    Deferred,
    /// The backend cannot consume this chain yet; leave it on the avail ring.
    Blocked,
}

pub struct VirtioDev {
    pub backend: Backend,
    status: u32,
    device_features_sel: u32,
    driver_features_sel: u32,
    queue_sel: u32,
    queues: [Queue; MAX_QUEUES],
    config_generation: u32,
    /// bit 0: used-ring update pending, bit 1: configuration change.
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
            config_generation: 0,
            int_status: 0,
        }
    }

    pub fn device_id(&self) -> u32 {
        match self.backend {
            Backend::Console { .. } => 3,
            Backend::Block { .. } => 2,
            Backend::Fs { .. } | Backend::FsExternal { .. } => 9,
            Backend::Net { .. } => 1,
        }
    }

    /// Feature bits for word `sel` (0 = bits 0-31, 1 = bits 32-63).
    fn device_features(&self, sel: u32) -> u32 {
        match sel {
            // bit 32: VIRTIO_F_VERSION_1 — modern queue layout.
            1 => 1,
            0 => match self.backend {
                Backend::Console { .. } => 1,
                // bit 0: VIRTIO_9P_MOUNT_TAG — config space carries the tag
                // the guest mounts by. Without it the driver refuses to probe.
                Backend::Fs { .. } | Backend::FsExternal { .. } => {
                    // bit 0: VIRTIO_9P_MOUNT_TAG
                    // bit 28: VIRTIO_RING_F_INDIRECT_DESC. A maximum-sized
                    // 9P request scatters across roughly 33 guest pages. With
                    // direct descriptors alone that limited a 128-entry ring
                    // to three in-flight requests. Indirect tables consume one
                    // ring entry per request and let Linux pipeline the I/O.
                    1 | (1 << 28)
                }
                // bit 5: VIRTIO_NET_F_MAC — the MAC in config space is ours to
                // give. Offering nothing else keeps the driver off checksum
                // offload, GSO and mergeable RX buffers, none of which a
                // frame-shuffling device benefits from.
                Backend::Net { .. } => 1 << 5,
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
            Backend::Fs { .. } | Backend::FsExternal { .. } => 128,
            // Modern virtio_net stops a TX queue unless it has enough free
            // descriptors for a maximally fragmented skb plus its header.
            // A 16-entry ring can transmit once and then remain stopped even
            // after we publish the used entry; 256 is the conventional size.
            Backend::Net { .. } => 256,
            // Console output can arrive as one burst (a full-screen tmux
            // redraw). A 16-entry ring fills in a single render, after which
            // the hvc driver only retries on its periodic khvcd poll — far too
            // slow on this emulator and easily mistaken for a hang. Give the
            // driver its full VIRTQUEUE_NUM (128) so bursts fit.
            Backend::Console { .. } => 128,
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

    pub fn console_resize(&mut self, cols: u16, rows: u16) {
        if let Backend::Console {
            cols: old_cols,
            rows: old_rows,
            ..
        } = &mut self.backend
        {
            *old_cols = cols;
            *old_rows = rows;
            self.config_generation = self.config_generation.wrapping_add(1);
            self.int_status |= 2;
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
            0x0fc => self.config_generation, // config generation
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
                let driver_ok = self.status & 0x04 != 0;
                self.status = val;
                if val == 0 {
                    // reset
                    self.queues = Default::default();
                    self.int_status = 0;
                    if let Backend::FsExternal {
                        requests, pending, ..
                    } = &mut self.backend
                    {
                        requests.clear();
                        pending.clear();
                    }
                } else if !driver_ok && val & 0x04 != 0 {
                    // The virtio_console driver only learns the console size
                    // from a config-change interrupt (its non-multiport
                    // config_work handler). Without an initial one, hvc0 keeps
                    // a zero winsize and stty reports no size information.
                    if matches!(self.backend, Backend::Console { .. }) {
                        self.int_status |= 2;
                    }
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
            Backend::Console { cols, rows, .. } => match off {
                0 => cols.to_le_bytes()[0],
                1 => cols.to_le_bytes()[1],
                2 => rows.to_le_bytes()[0],
                3 => rows.to_le_bytes()[1],
                _ => 0,
            },
            Backend::Block { disk } => {
                // struct virtio_blk_config { le64 capacity; ... } in sectors.
                let sectors = (disk.len() / SECTOR) as u64;
                sectors
                    .to_le_bytes()
                    .get(off as usize)
                    .copied()
                    .unwrap_or(0)
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
            Backend::FsExternal { tag, .. } => {
                let tag = tag.as_bytes();
                match off {
                    0 => tag.len() as u8,
                    1 => (tag.len() >> 8) as u8,
                    _ => tag.get(off as usize - 2).copied().unwrap_or(0),
                }
            }
            Backend::Net { mac, .. } => {
                // struct virtio_net_config { u8 mac[6]; le16 status; ... }
                // status stays 0: VIRTIO_NET_F_STATUS is not offered, so the
                // driver assumes the link is up and never reads it.
                mac.get(off as usize).copied().unwrap_or(0)
            }
        }
    }

    /// Queue a frame from the host for delivery to the guest. Returns false
    /// without consuming ownership when the bounded inbox has no room.
    pub fn net_input(&mut self, frame: &[u8]) -> bool {
        if let Backend::Net { inbox, .. } = &mut self.backend {
            if frame.len() <= NET_MAX_FRAME && inbox.len() < NET_QUEUE_LIMIT {
                inbox.push_back(frame.to_vec());
                return true;
            }
        }
        false
    }

    /// Number of frames the host can admit without dropping any.
    pub fn net_input_capacity(&self) -> usize {
        match &self.backend {
            Backend::Net { inbox, .. } => NET_QUEUE_LIMIT.saturating_sub(inbox.len()),
            _ => 0,
        }
    }

    /// Collect the frames the guest has transmitted.
    pub fn net_take_output(&mut self) -> Vec<Vec<u8>> {
        if let Backend::Net { outbox, .. } = &mut self.backend {
            core::mem::take(outbox)
        } else {
            Vec::new()
        }
    }

    /// True when this device has inbound frames waiting for an RX buffer.
    pub fn net_rx_pending(&self) -> bool {
        matches!(&self.backend, Backend::Net { inbox, .. } if !inbox.is_empty())
    }

    /// Take the next external 9P request waiting for the JavaScript host.
    pub fn fs_external_take_request(&mut self) -> Option<Vec<u8>> {
        match &mut self.backend {
            Backend::FsExternal { requests, .. } => requests.pop_front(),
            _ => None,
        }
    }

    /// Supply an external 9P reply and complete the request having its tag.
    /// Replies may arrive in any order. Completion is published immediately:
    /// deferring it until another CPU slice can deadlock a hart waiting in WFI.
    pub fn fs_external_reply(&mut self, bytes: Vec<u8>, ram: &mut [u8], ram_base: u64) -> bool {
        let Some(tag) = p9_tag(&bytes) else {
            return false;
        };
        let pending = match &mut self.backend {
            Backend::FsExternal { pending, .. } => pending.remove(&tag),
            _ => None,
        };
        let Some(pending) = pending else {
            return false;
        };

        let mut pos = 0usize;
        for &(addr, len, writable) in &pending.chain {
            if !writable || pos >= bytes.len() {
                continue;
            }
            let n = (bytes.len() - pos).min(len as usize);
            match guest_slice_mut(ram, ram_base, addr, n as u32) {
                Some(dst) => dst.copy_from_slice(&bytes[pos..pos + n]),
                None => break,
            }
            pos += n;
        }

        let q = self.queues[0].clone();
        publish_used(&q, pending.head, pos as u32, ram, ram_base);
        self.int_status |= 1;
        true
    }

    // ---- virtqueue processing ------------------------------------------

    /// Process queue `qi` (after a notify, or when console input arrives).
    /// `ram`/`ram_base` give access to guest physical memory.
    pub fn process(&mut self, qi: usize, ram: &mut [u8], ram_base: u64) {
        if qi >= MAX_QUEUES || self.queues[qi].ready == 0 {
            if vio_dbg() {
                eprintln!(
                    "[vio] notify q{qi} BAILED ready={}",
                    self.queues.get(qi).map_or(0, |q| q.ready)
                );
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

            // Walk either the split-ring descriptor chain or the indirect
            // table named by its head. A malformed chain is consumed with a
            // zero-length completion instead of panicking in the emulator.
            let Some(chain) = descriptor_chain(&q, head, ram, ram_base) else {
                publish_used(&q, head, 0, ram, ram_base);
                self.queues[qi].last_avail_idx = self.queues[qi].last_avail_idx.wrapping_add(1);
                self.int_status |= 1;
                serviced += 1;
                continue;
            };

            match self.service(qi, head, &chain, ram, ram_base) {
                ServiceResult::Complete(written) => {
                    publish_used(&q, head, written, ram, ram_base);
                    self.queues[qi].last_avail_idx = self.queues[qi].last_avail_idx.wrapping_add(1);
                    self.int_status |= 1;
                    serviced += 1;
                }
                ServiceResult::Deferred => {
                    // The asynchronous backend owns this chain now. Keep
                    // consuming avail entries so Linux can overlap requests.
                    self.queues[qi].last_avail_idx = self.queues[qi].last_avail_idx.wrapping_add(1);
                }
                ServiceResult::Blocked => {
                    // Not serviceable now (for example console RX without
                    // input): leave this descriptor for a future poll.
                    break;
                }
            }
        }
    }

    /// Service one descriptor chain synchronously, retain it for asynchronous
    /// completion, or leave it available when the backend is temporarily idle.
    fn service(
        &mut self,
        qi: usize,
        head: u16,
        chain: &[Descriptor],
        ram: &mut [u8],
        ram_base: u64,
    ) -> ServiceResult {
        match &mut self.backend {
            Backend::Console { rx_buf, tx_out, .. } => {
                if qi == 0 {
                    // RX: fill writable buffers with pending input.
                    if rx_buf.is_empty() {
                        return ServiceResult::Blocked;
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
                    ServiceResult::Complete(written)
                } else {
                    // TX: collect guest output.
                    for &(addr, len, writable) in chain {
                        if writable {
                            continue;
                        }
                        let off = (addr - ram_base) as usize;
                        tx_out.extend_from_slice(&ram[off..off + len as usize]);
                    }
                    ServiceResult::Complete(0)
                }
            }
            Backend::Block { disk } => {
                // Layout: header (16B, read-only) | data buffers | status (1B, writable)
                let Some(&(hdr_addr, ..)) = chain.first() else {
                    return ServiceResult::Complete(0);
                };
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
                ServiceResult::Complete(written)
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
                        None => return ServiceResult::Complete(0),
                    }
                }
                if req.is_empty() {
                    return ServiceResult::Complete(0);
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
                ServiceResult::Complete(pos as u32)
            }
            Backend::FsExternal {
                requests, pending, ..
            } => {
                let mut bytes = Vec::new();
                for &(addr, len, writable) in chain {
                    if writable {
                        continue;
                    }
                    let Some(src) = guest_slice(ram, ram_base, addr, len) else {
                        return ServiceResult::Complete(0);
                    };
                    bytes.extend_from_slice(src);
                }
                let Some(tag) = p9_tag(&bytes) else {
                    return ServiceResult::Complete(0);
                };
                if pending.contains_key(&tag) {
                    // Tags must remain unique until their reply. Consume a
                    // malformed duplicate rather than corrupting the original.
                    return ServiceResult::Complete(0);
                }
                pending.insert(
                    tag,
                    ExternalP9Pending {
                        head,
                        chain: chain.to_vec(),
                    },
                );
                requests.push_back(bytes);
                ServiceResult::Deferred
            }
            Backend::Net { inbox, outbox, .. } => {
                if qi == 0 {
                    // RX: hand the oldest inbound frame to the guest, prefixed
                    // by a zeroed virtio_net_hdr_v1. No frame means we leave the
                    // buffer on the ring for later rather than consuming it.
                    if inbox.is_empty() {
                        return ServiceResult::Blocked;
                    }
                    let frame = inbox.front().expect("checked non-empty inbox");
                    let need = NET_HDR_LEN + frame.len();
                    let capacity: usize = chain
                        .iter()
                        .filter(|&&(_, _, writable)| writable)
                        .map(|&(_, len, _)| len as usize)
                        .sum();
                    if capacity < need {
                        // The frame cannot fit the buffer the guest offered.
                        // Drop it: keeping it would wedge the queue forever.
                        inbox.pop_front();
                        return ServiceResult::Blocked;
                    }
                    let mut hdr = [0u8; NET_HDR_LEN];
                    // num_buffers = 1: this frame occupies exactly one chain.
                    hdr[10] = 1;
                    let mut written = 0usize;
                    let mut src: Vec<u8> = hdr.to_vec();
                    src.extend_from_slice(frame);
                    for &(addr, len, writable) in chain {
                        if !writable || written >= src.len() {
                            continue;
                        }
                        let n = (src.len() - written).min(len as usize);
                        match guest_slice_mut(ram, ram_base, addr, n as u32) {
                            Some(d) => d.copy_from_slice(&src[written..written + n]),
                            None => break,
                        }
                        written += n;
                    }
                    inbox.pop_front();
                    ServiceResult::Complete(written as u32)
                } else {
                    // TX: reassemble the frame from the readable descriptors and
                    // strip the header the guest prepended.
                    let mut frame = Vec::new();
                    for &(addr, len, writable) in chain {
                        if writable {
                            continue;
                        }
                        match guest_slice(ram, ram_base, addr, len) {
                            Some(s) => frame.extend_from_slice(s),
                            None => return ServiceResult::Complete(0),
                        }
                    }
                    if frame.len() > NET_HDR_LEN
                        && frame.len() - NET_HDR_LEN <= NET_MAX_FRAME
                        && outbox.len() < NET_QUEUE_LIMIT
                    {
                        outbox.push(frame[NET_HDR_LEN..].to_vec());
                    }
                    ServiceResult::Complete(0)
                }
            }
        }
    }
}

/// Resolve a split-ring descriptor chain, including the indirect-table form
/// negotiated through VIRTIO_RING_F_INDIRECT_DESC. The returned descriptors
/// contain only guest buffers; the outer indirect descriptor is transport
/// metadata and never participates in the device request itself.
fn descriptor_chain(q: &Queue, head: u16, ram: &[u8], ram_base: u64) -> Option<Vec<Descriptor>> {
    let (addr, len, flags, _) = read_descriptor(ram, ram_base, q.desc, q.num, head as u32)?;
    if flags & VIRTQ_DESC_F_INDIRECT == 0 {
        return walk_descriptor_table(ram, ram_base, q.desc, q.num, head as u32);
    }

    // Virtio 1.0 section 2.4.5.3: an indirect head must not also carry NEXT,
    // its table is a non-empty array of complete descriptors, and indirect
    // descriptors cannot be nested.
    if flags & VIRTQ_DESC_F_NEXT != 0 || len == 0 || u64::from(len) % DESCRIPTOR_BYTES != 0 {
        return None;
    }
    let count = len / DESCRIPTOR_BYTES as u32;
    walk_descriptor_table(ram, ram_base, addr, count, 0)
}

fn walk_descriptor_table(
    ram: &[u8],
    ram_base: u64,
    table: u64,
    count: u32,
    first: u32,
) -> Option<Vec<Descriptor>> {
    if count == 0 || first >= count {
        return None;
    }
    let mut chain = Vec::new();
    let mut index = first;
    for _ in 0..count {
        let (addr, len, flags, next) = read_descriptor(ram, ram_base, table, count, index)?;
        if flags & VIRTQ_DESC_F_INDIRECT != 0 {
            return None;
        }
        chain.push((addr, len, flags & VIRTQ_DESC_F_WRITE != 0));
        if flags & VIRTQ_DESC_F_NEXT == 0 {
            return Some(chain);
        }
        index = u32::from(next);
    }
    // More NEXT links than table entries implies a cycle.
    None
}

fn read_descriptor(
    ram: &[u8],
    ram_base: u64,
    table: u64,
    count: u32,
    index: u32,
) -> Option<(u64, u32, u16, u16)> {
    if index >= count {
        return None;
    }
    let address = table.checked_add(u64::from(index).checked_mul(DESCRIPTOR_BYTES)?)?;
    let bytes = guest_slice(ram, ram_base, address, DESCRIPTOR_BYTES as u32)?;
    Some((
        u64::from_le_bytes(bytes[0..8].try_into().ok()?),
        u32::from_le_bytes(bytes[8..12].try_into().ok()?),
        u16::from_le_bytes(bytes[12..14].try_into().ok()?),
        u16::from_le_bytes(bytes[14..16].try_into().ok()?),
    ))
}

fn p9_tag(message: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes([*message.get(5)?, *message.get(6)?]))
}

fn publish_used(q: &Queue, head: u16, written: u32, ram: &mut [u8], ram_base: u64) {
    let used_idx = read16(ram, ram_base, q.used + 2);
    let slot = (used_idx as u64) % (q.num as u64);
    write32(ram, ram_base, q.used + 4 + slot * 8, head as u32);
    write32(ram, ram_base, q.used + 4 + slot * 8 + 4, written);
    write16(ram, ram_base, q.used + 2, used_idx.wrapping_add(1));
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
fn write16(ram: &mut [u8], base: u64, addr: u64, v: u16) {
    let o = (addr - base) as usize;
    ram[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn write32(ram: &mut [u8], base: u64, addr: u64, v: u32) {
    let o = (addr - base) as usize;
    ram[o..o + 4].copy_from_slice(&v.to_le_bytes());
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
    const INDIRECT: usize = 0x9000;
    const NUM: u32 = 8;

    fn fs_device() -> (VirtioDev, Vec<u8>) {
        let mut fs = MemFs::new();
        fs.add_file("/greeting", b"from-the-host", 0o644);
        let dev = VirtioDev::new(Backend::Fs {
            srv: p9::Server::new("hostshare", Box::new(fs)),
        });
        (dev, vec![0u8; 64 * 1024])
    }

    /// Ring addresses for one queue. Queue 0 lives at the low set; a
    /// two-queue device (net) puts queue 1 at the high set.
    struct Ring {
        desc: usize,
        avail: usize,
        used: usize,
    }
    const RING0: Ring = Ring {
        desc: DESC,
        avail: AVAIL,
        used: USED,
    };
    const RING1: Ring = Ring {
        desc: 0x6000,
        avail: 0x7000,
        used: 0x8000,
    };

    fn setup_queue(dev: &mut VirtioDev) {
        setup_ring(dev, 0, &RING0);
    }

    fn setup_ring(dev: &mut VirtioDev, qi: u32, r: &Ring) {
        dev.write(0x030, qi); // queue_sel
        dev.write(0x038, NUM); // queue_num
        dev.write(0x080, (BASE + r.desc as u64) as u32);
        dev.write(0x084, ((BASE + r.desc as u64) >> 32) as u32);
        dev.write(0x090, (BASE + r.avail as u64) as u32);
        dev.write(0x094, ((BASE + r.avail as u64) >> 32) as u32);
        dev.write(0x0a0, (BASE + r.used as u64) as u32);
        dev.write(0x0a4, ((BASE + r.used as u64) >> 32) as u32);
        dev.write(0x044, 1); // queue_ready
    }

    fn put_desc(ram: &mut [u8], i: usize, addr: usize, len: u32, flags: u16, next: u16) {
        put_desc_in(ram, &RING0, i, addr, len, flags, next);
    }

    fn put_desc_in(
        ram: &mut [u8],
        r: &Ring,
        i: usize,
        addr: usize,
        len: u32,
        flags: u16,
        next: u16,
    ) {
        let o = r.desc + i * 16;
        ram[o..o + 8].copy_from_slice(&(BASE + addr as u64).to_le_bytes());
        ram[o + 8..o + 12].copy_from_slice(&len.to_le_bytes());
        ram[o + 12..o + 14].copy_from_slice(&flags.to_le_bytes());
        ram[o + 14..o + 16].copy_from_slice(&next.to_le_bytes());
    }

    fn put_indirect_desc(ram: &mut [u8], i: usize, addr: usize, len: u32, flags: u16, next: u16) {
        let o = INDIRECT + i * DESCRIPTOR_BYTES as usize;
        ram[o..o + 8].copy_from_slice(&(BASE + addr as u64).to_le_bytes());
        ram[o + 8..o + 12].copy_from_slice(&len.to_le_bytes());
        ram[o + 12..o + 14].copy_from_slice(&flags.to_le_bytes());
        ram[o + 14..o + 16].copy_from_slice(&next.to_le_bytes());
    }

    /// Put chain head `head` on the avail ring and publish index `n`.
    fn publish(ram: &mut [u8], r: &Ring, head: u16, n: u16) {
        let slot = (n as usize - 1) % NUM as usize;
        ram[r.avail + 4 + slot * 2..r.avail + 6 + slot * 2].copy_from_slice(&head.to_le_bytes());
        ram[r.avail + 2..r.avail + 4].copy_from_slice(&n.to_le_bytes());
    }

    /// (chain head, bytes written) from used-ring entry `n` (1-based).
    fn used_entry(ram: &[u8], r: &Ring, n: u16) -> (u32, u32) {
        let slot = (n as usize - 1) % NUM as usize;
        (
            u32_at(ram, r.used + 4 + slot * 8),
            u32_at(ram, r.used + 8 + slot * 8),
        )
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
        publish(ram, &RING0, 0, n);

        assert_eq!(dev.write(0x050, 0), Some(0), "QueueNotify selects queue 0");
        dev.process(0, ram, BASE);

        // The device must have published exactly one more used entry.
        assert_eq!(u16_at(ram, USED + 2), n, "used.idx after request {n}");
        let (head, len) = used_entry(ram, &RING0, n);
        assert_eq!(head, 0, "used entry is chain 0");
        let len = len as usize;
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
        assert_eq!(
            dev.read(0x010),
            1 | (1 << 28),
            "mount-tag and indirect-descriptor features"
        );
        dev.write(0x014, 1);
        assert_eq!(dev.read(0x010), 1, "VIRTIO_F_VERSION_1 in feature word 1");
        assert_eq!(dev.read(0x034), 128, "ring must hold a whole msize message");
        // Read the tag exactly as Linux does: a 16-bit length, then one byte at
        // a time. This is what a 32-bit-only config space would break.
        assert_eq!(dev.read_sized(0x100, 2), 9);
        let tag: Vec<u8> = (0..9).map(|i| dev.read_sized(0x102 + i, 1) as u8).collect();
        assert_eq!(&tag, b"hostshare");
    }

    #[test]
    fn external_9p_completes_the_chain_when_the_host_replies() {
        let mut dev = VirtioDev::new(Backend::FsExternal {
            tag: "host9p".into(),
            requests: Default::default(),
            pending: Default::default(),
        });
        let mut ram = vec![0u8; 64 * 1024];
        setup_queue(&mut dev);
        let request = [7, 0, 0, 0, 100, 3, 0];
        ram[REQ..REQ + request.len()].copy_from_slice(&request);
        put_desc(&mut ram, 0, REQ, request.len() as u32, 1, 1);
        put_desc(&mut ram, 1, REPLY, 4096, 2, 0);
        publish(&mut ram, &RING0, 0, 1);

        dev.process(0, &mut ram, BASE);
        assert_eq!(u16_at(&ram, USED + 2), 0);
        assert_eq!(
            dev.fs_external_take_request().as_deref(),
            Some(request.as_slice())
        );
        assert_eq!(
            dev.fs_external_take_request(),
            None,
            "request is emitted once"
        );
        dev.process(0, &mut ram, BASE);
        assert_eq!(
            dev.fs_external_take_request(),
            None,
            "in-flight request is not re-emitted"
        );

        let reply = [7, 0, 0, 0, 101, 3, 0];
        assert!(dev.fs_external_reply(reply.to_vec(), &mut ram, BASE));
        assert_eq!(u16_at(&ram, USED + 2), 1);
        assert_eq!(&ram[REPLY..REPLY + reply.len()], &reply);
        assert!(dev.irq_pending());
    }

    #[test]
    fn external_9p_pipelines_and_completes_out_of_order_by_tag() {
        let mut dev = VirtioDev::new(Backend::FsExternal {
            tag: "host9p".into(),
            requests: Default::default(),
            pending: Default::default(),
        });
        let mut ram = vec![0u8; 64 * 1024];
        setup_queue(&mut dev);

        const REQ2: usize = REQ + 0x100;
        const REPLY2: usize = REPLY + 0x100;
        let request1 = [7, 0, 0, 0, 100, 3, 0];
        let request2 = [7, 0, 0, 0, 116, 9, 0];
        ram[REQ..REQ + request1.len()].copy_from_slice(&request1);
        ram[REQ2..REQ2 + request2.len()].copy_from_slice(&request2);
        put_desc(&mut ram, 0, REQ, request1.len() as u32, 1, 1);
        put_desc(&mut ram, 1, REPLY, 128, 2, 0);
        put_desc(&mut ram, 2, REQ2, request2.len() as u32, 1, 3);
        put_desc(&mut ram, 3, REPLY2, 128, 2, 0);
        publish(&mut ram, &RING0, 0, 1);
        publish(&mut ram, &RING0, 2, 2);

        dev.process(0, &mut ram, BASE);
        assert_eq!(
            dev.queue_debug(0).unwrap().4,
            2,
            "both avail entries consumed"
        );
        assert_eq!(u16_at(&ram, USED + 2), 0, "neither request has replied");
        assert_eq!(dev.fs_external_take_request(), Some(request1.to_vec()));
        assert_eq!(dev.fs_external_take_request(), Some(request2.to_vec()));
        assert_eq!(dev.fs_external_take_request(), None);

        let reply2 = [7, 0, 0, 0, 117, 9, 0];
        assert!(dev.fs_external_reply(reply2.to_vec(), &mut ram, BASE));
        assert_eq!(u16_at(&ram, USED + 2), 1);
        assert_eq!(used_entry(&ram, &RING0, 1), (2, reply2.len() as u32));
        assert_eq!(&ram[REPLY2..REPLY2 + reply2.len()], &reply2);

        let reply1 = [7, 0, 0, 0, 101, 3, 0];
        assert!(dev.fs_external_reply(reply1.to_vec(), &mut ram, BASE));
        assert_eq!(u16_at(&ram, USED + 2), 2);
        assert_eq!(used_entry(&ram, &RING0, 2), (0, reply1.len() as u32));
        assert_eq!(&ram[REPLY..REPLY + reply1.len()], &reply1);
        assert!(dev.irq_pending());
    }

    #[test]
    fn external_9p_accepts_a_scattered_indirect_descriptor_chain() {
        let mut dev = VirtioDev::new(Backend::FsExternal {
            tag: "host9p".into(),
            requests: Default::default(),
            pending: Default::default(),
        });
        let mut ram = vec![0u8; 64 * 1024];
        setup_queue(&mut dev);

        let request = [7, 0, 0, 0, 100, 0x34, 0x12];
        ram[REQ..REQ + request.len()].copy_from_slice(&request);
        // The split-ring head consumes one descriptor. Its three-entry table
        // scatters the request across two readable buffers and names a writable
        // reply buffer, matching Linux's large-message virtio-9P shape.
        put_desc(
            &mut ram,
            0,
            INDIRECT,
            (3 * DESCRIPTOR_BYTES) as u32,
            VIRTQ_DESC_F_INDIRECT,
            7, // ignored without NEXT, as real free-list values must be
        );
        put_indirect_desc(&mut ram, 0, REQ, 4, VIRTQ_DESC_F_NEXT, 1);
        put_indirect_desc(
            &mut ram,
            1,
            REQ + 4,
            (request.len() - 4) as u32,
            VIRTQ_DESC_F_NEXT,
            2,
        );
        put_indirect_desc(&mut ram, 2, REPLY, 128, VIRTQ_DESC_F_WRITE, 0);
        publish(&mut ram, &RING0, 0, 1);

        dev.process(0, &mut ram, BASE);
        assert_eq!(u16_at(&ram, USED + 2), 0, "request remains in flight");
        assert_eq!(dev.fs_external_take_request(), Some(request.to_vec()));

        let reply = [7, 0, 0, 0, 101, 0x34, 0x12];
        assert!(dev.fs_external_reply(reply.to_vec(), &mut ram, BASE));
        assert_eq!(used_entry(&ram, &RING0, 1), (0, reply.len() as u32));
        assert_eq!(&ram[REPLY..REPLY + reply.len()], &reply);
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
            cols: 80,
            rows: 24,
        });
        assert_eq!(dev.read(0x008), 3);
        dev.write(0x014, 0);
        assert_eq!(dev.read(0x010), 1, "console offers the size feature");
        assert_eq!(dev.read_sized(0x100, 2), 80);
        assert_eq!(dev.read_sized(0x102, 2), 24);
        dev.console_resize(120, 40);
        assert_eq!(dev.read_sized(0x100, 2), 120);
        assert_eq!(dev.read_sized(0x102, 2), 40);
        assert_eq!(dev.read(0x0fc), 1);
        assert!(dev.irq_pending());
    }

    #[test]
    fn services_a_mount_and_read_over_the_virtqueue() {
        let (mut dev, mut ram) = fs_device();
        setup_queue(&mut dev);

        // Tversion — negotiate msize.
        let r = round_trip(
            &mut dev,
            &mut ram,
            100,
            &B::default().u32(8192).str("9P2000.L"),
            1,
        );
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
        publish(&mut ram, &RING0, 0, 1);
        dev.process(0, &mut ram, BASE);
        assert_eq!(u16_at(&ram, USED + 2), 1, "chain consumed");
    }

    // ---- virtio-net ----

    const FRAME: usize = 0x9000; // guest TX frame buffer
    const RXBUF: usize = 0xa000; // guest RX buffer

    const TEST_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0xab, 0xcd, 0xef];

    fn net_device() -> (VirtioDev, Vec<u8>) {
        let dev = VirtioDev::new(Backend::Net {
            mac: TEST_MAC,
            inbox: Default::default(),
            outbox: Vec::new(),
        });
        (dev, vec![0u8; 64 * 1024])
    }

    #[test]
    fn advertises_a_net_device_and_its_mac() {
        let (mut dev, _) = net_device();
        assert_eq!(dev.read(0x008), 1); // virtio device id 1 = net
        dev.write(0x014, 0);
        assert_eq!(dev.read(0x010), 1 << 5, "VIRTIO_NET_F_MAC only");
        dev.write(0x014, 1);
        assert_eq!(dev.read(0x010), 1, "VIRTIO_F_VERSION_1");
        // The MAC is read byte-wise out of config space; without F_MAC (and a
        // readable MAC) the guest would invent a random address instead.
        let mac: Vec<u8> = (0..6).map(|i| dev.read_sized(0x100 + i, 1) as u8).collect();
        assert_eq!(&mac, &TEST_MAC);
        // status is only meaningful with VIRTIO_NET_F_STATUS, which we do not
        // offer — the guest treats the link as up unconditionally.
        assert_eq!(dev.read_sized(0x106, 2), 0);
        assert_eq!(dev.read(0x034), 256, "modern TX needs a non-tiny ring");
    }

    #[test]
    fn transmits_a_guest_frame_stripped_of_its_header() {
        let (mut dev, mut ram) = net_device();
        setup_ring(&mut dev, 1, &RING1);

        // The guest writes virtio_net_hdr_v1 then the Ethernet frame.
        let payload: Vec<u8> = (0..60u8).collect();
        ram[FRAME..FRAME + NET_HDR_LEN].fill(0);
        ram[FRAME + NET_HDR_LEN..FRAME + NET_HDR_LEN + payload.len()].copy_from_slice(&payload);
        put_desc_in(
            &mut ram,
            &RING1,
            0,
            FRAME,
            (NET_HDR_LEN + payload.len()) as u32,
            0,
            0,
        );
        publish(&mut ram, &RING1, 0, 1);

        assert_eq!(dev.write(0x050, 1), Some(1), "notify selects the TX queue");
        dev.process(1, &mut ram, BASE);

        assert_eq!(u16_at(&ram, RING1.used + 2), 1, "TX chain consumed");
        let out = dev.net_take_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], payload, "header must be stripped, frame intact");
        assert!(dev.net_take_output().is_empty(), "drained once");
    }

    #[test]
    fn transmits_a_frame_split_across_descriptors() {
        let (mut dev, mut ram) = net_device();
        setup_ring(&mut dev, 1, &RING1);
        // Linux hands the header and the payload as separate sg entries.
        let payload: Vec<u8> = (0..100u8).map(|i| i ^ 0x5a).collect();
        ram[FRAME..FRAME + NET_HDR_LEN].fill(0);
        ram[RXBUF..RXBUF + payload.len()].copy_from_slice(&payload);
        put_desc_in(&mut ram, &RING1, 0, FRAME, NET_HDR_LEN as u32, 1, 1);
        put_desc_in(&mut ram, &RING1, 1, RXBUF, payload.len() as u32, 0, 0);
        publish(&mut ram, &RING1, 0, 1);
        dev.process(1, &mut ram, BASE);
        assert_eq!(dev.net_take_output()[0], payload);
    }

    #[test]
    fn receives_a_frame_into_a_guest_buffer() {
        let (mut dev, mut ram) = net_device();
        setup_ring(&mut dev, 0, &RING0);

        let frame: Vec<u8> = (0..74u8).map(|i| i.wrapping_mul(3)).collect();
        assert!(dev.net_input(&frame));
        assert!(dev.net_rx_pending());

        put_desc_in(&mut ram, &RING0, 0, RXBUF, 2048, 2 /* WRITE */, 0);
        publish(&mut ram, &RING0, 0, 1);
        dev.process(0, &mut ram, BASE);

        let (head, len) = used_entry(&ram, &RING0, 1);
        assert_eq!(head, 0);
        assert_eq!(len as usize, NET_HDR_LEN + frame.len());
        // num_buffers = 1 at offset 10. Linux sizes hdr_len from VERSION_1, so
        // a 10-byte header here would shift every frame by two bytes.
        assert_eq!(&ram[RXBUF..RXBUF + 10], &[0u8; 10]);
        assert_eq!(u16_at(&ram, RXBUF + 10), 1, "num_buffers");
        assert_eq!(
            &ram[RXBUF + NET_HDR_LEN..RXBUF + NET_HDR_LEN + frame.len()],
            &frame[..]
        );
        assert!(!dev.net_rx_pending(), "frame consumed");
        assert!(dev.irq_pending(), "RX must raise the interrupt");
    }

    #[test]
    fn an_rx_buffer_with_no_frame_is_left_on_the_ring() {
        let (mut dev, mut ram) = net_device();
        setup_ring(&mut dev, 0, &RING0);
        put_desc_in(&mut ram, &RING0, 0, RXBUF, 2048, 2, 0);
        publish(&mut ram, &RING0, 0, 1);
        dev.process(0, &mut ram, BASE);
        // Nothing to deliver: the buffer stays available for the next frame
        // rather than being consumed empty.
        assert_eq!(u16_at(&ram, RING0.used + 2), 0);
        assert!(!dev.irq_pending());
        // It is still there when a frame turns up.
        assert!(dev.net_input(b"late frame"));
        dev.process(0, &mut ram, BASE);
        assert_eq!(u16_at(&ram, RING0.used + 2), 1);
    }

    #[test]
    fn an_undersized_rx_buffer_drops_the_frame_instead_of_wedging() {
        let (mut dev, mut ram) = net_device();
        setup_ring(&mut dev, 0, &RING0);
        assert!(dev.net_input(&vec![7u8; 1500]));
        put_desc_in(&mut ram, &RING0, 0, RXBUF, 64, 2, 0);
        publish(&mut ram, &RING0, 0, 1);
        dev.process(0, &mut ram, BASE);
        // Keeping an unsendable frame at the head would stall the queue forever.
        assert!(!dev.net_rx_pending(), "oversized frame dropped");
        assert_eq!(u16_at(&ram, RING0.used + 2), 0, "buffer not consumed");
    }

    #[test]
    fn mailboxes_are_bounded() {
        let (mut dev, mut ram) = net_device();
        setup_ring(&mut dev, 1, &RING1);
        // A guest that never posts RX buffers must not grow host memory.
        for _ in 0..NET_QUEUE_LIMIT {
            assert!(dev.net_input(b"flood"));
        }
        assert_eq!(dev.net_input_capacity(), 0);
        for _ in 0..50 {
            assert!(!dev.net_input(b"flood"));
        }
        let Backend::Net { inbox, .. } = &dev.backend else {
            unreachable!()
        };
        assert_eq!(inbox.len(), NET_QUEUE_LIMIT);
        // Nor must a host that never collects.
        ram[FRAME..FRAME + NET_HDR_LEN + 4].fill(1);
        for n in 1..=(NET_QUEUE_LIMIT as u16 + 50) {
            put_desc_in(&mut ram, &RING1, 0, FRAME, (NET_HDR_LEN + 4) as u32, 0, 0);
            publish(&mut ram, &RING1, 0, n);
            dev.process(1, &mut ram, BASE);
        }
        assert_eq!(dev.net_take_output().len(), NET_QUEUE_LIMIT);
    }

    #[test]
    fn an_over_long_frame_is_refused() {
        let (mut dev, _) = net_device();
        assert!(!dev.net_input(&vec![0u8; NET_MAX_FRAME + 1]));
        assert!(!dev.net_rx_pending(), "frame larger than NET_MAX_FRAME");
        assert!(dev.net_input(&vec![0u8; NET_MAX_FRAME]));
        assert!(dev.net_rx_pending(), "a frame at the limit is accepted");
    }
}
