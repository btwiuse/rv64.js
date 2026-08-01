//! Minimal host-side network stack, sitting behind virtio-net.
//!
//! This is *not* a NAT or a router. Its entire job is to let a guest open TCP
//! connections to **one** address — the proxy — and hand those connections up as
//! byte streams. Everything a general slirp needs and this does not: no DNS (the
//! guest passes hostnames to the proxy, so nothing here resolves anything), no
//! forwarding, no UDP beyond DHCP, no connection tracking per destination.
//!
//! That reduction is the whole reason the proxy design is cheap. Compare
//! TinyEMU's `slirp/` at ~8.5k lines.
//!
//! The stack is pure logic: frames in, frames out, events up. The host loop
//! drives it, so it is equally usable natively and in wasm, and testable with no
//! emulator at all:
//!
//! ```text
//! m.net_take_output() ──▶ stack.input(frame)
//! m.net_input(frame)  ◀── stack.take_output()
//!                         stack.take_events() ──▶ proxy
//!                         stack.send/close    ◀── proxy
//! ```
//!
//! ## What the TCP is and is not
//!
//! Both endpoints live in one process, so the wire is effectively lossless and
//! there is no congestion to control. What remains genuinely necessary: correct
//! SYN/FIN sequencing, acknowledging exactly what arrived, and respecting the
//! guest's advertised window. Frames *can* still be dropped — the device bounds
//! its mailboxes, and a guest whose RX ring backs up will lose one — so unacked
//! data is retained and retransmitted after a stall rather than assumed
//! delivered.

use std::collections::VecDeque;

// ---- wire constants -------------------------------------------------------

const ETH_HDR: usize = 14;
const ETHERTYPE_IP: u16 = 0x0800;
const ETHERTYPE_ARP: u16 = 0x0806;
const BROADCAST: [u8; 6] = [0xff; 6];

const IP_PROTO_ICMP: u8 = 1;
const IP_PROTO_TCP: u8 = 6;
const IP_PROTO_UDP: u8 = 17;

const TCP_FIN: u8 = 0x01;
const TCP_SYN: u8 = 0x02;
const TCP_RST: u8 = 0x04;
const TCP_PSH: u8 = 0x08;
const TCP_ACK: u8 = 0x10;

/// Payload bytes per segment. 1460 = 1500 MTU - 20 IP - 20 TCP.
const MSS: usize = 1460;

/// Window we advertise to the guest. Bounded because everything the guest sends
/// is buffered in `rx` until the proxy consumes it.
const RCV_WINDOW: u16 = 32768;

/// `take_output` calls with unacked data and no progress before we retransmit.
/// The host loop calls once per slice, so this is a handful of slices — long
/// enough not to duplicate normal traffic, short enough that a dropped frame
/// does not stall a transfer visibly.
const RETRANSMIT_STALL: u32 = 8;

// ---- configuration --------------------------------------------------------

/// Addresses the stack presents. Defaults mirror the 10.0.2.0/24 layout QEMU's
/// user networking uses, so guest instructions look familiar.
#[derive(Clone, Copy, Debug)]
pub struct NetConfig {
    /// MAC of the gateway/proxy host (us).
    pub host_mac: [u8; 6],
    /// Our IP — the address the guest points `http_proxy` at.
    pub host_ip: [u8; 4],
    /// Address we hand the guest over DHCP.
    pub guest_ip: [u8; 4],
    pub netmask: [u8; 4],
    /// TCP port the proxy listens on.
    pub proxy_port: u16,
}

impl Default for NetConfig {
    fn default() -> NetConfig {
        NetConfig {
            host_mac: [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02],
            host_ip: [10, 0, 2, 2],
            guest_ip: [10, 0, 2, 15],
            netmask: [255, 255, 255, 0],
            proxy_port: 8080,
        }
    }
}

/// Connection identifier handed to the layer above.
pub type ConnId = u64;

/// What happened on the guest side, for the proxy to act on.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A guest connection to the proxy port completed its handshake.
    Opened(ConnId),
    /// Bytes arrived from the guest.
    Data(ConnId, Vec<u8>),
    /// The guest finished sending (FIN) or reset the connection.
    Closed(ConnId),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    SynReceived,
    Established,
    /// Guest sent FIN; we may still be sending.
    CloseWait,
    /// We sent FIN and are waiting for its ACK.
    LastAck,
}

struct Conn {
    id: ConnId,
    guest_port: u16,
    state: State,
    /// Next sequence number we expect from the guest.
    rcv_nxt: u32,
    /// Oldest byte we have sent and not had acknowledged.
    snd_una: u32,
    /// Next sequence number we will send.
    snd_nxt: u32,
    /// Guest's advertised receive window.
    snd_wnd: u16,
    /// Sent but unacknowledged payload, retained for retransmission.
    unacked: Vec<u8>,
    /// Queued by the proxy, not yet sent.
    tx: VecDeque<u8>,
    /// Proxy asked to close: send FIN once `tx` drains.
    fin_pending: bool,
    /// `take_output` calls since `snd_una` last advanced.
    stalled: u32,
}

pub struct NetStack {
    cfg: NetConfig,
    /// Learned from the guest's own frames; until then we cannot address it.
    guest_mac: Option<[u8; 6]>,
    conns: Vec<Conn>,
    next_id: ConnId,
    out: Vec<Vec<u8>>,
    events: Vec<Event>,
}

impl NetStack {
    pub fn new(cfg: NetConfig) -> NetStack {
        NetStack {
            cfg,
            guest_mac: None,
            conns: Vec::new(),
            next_id: 1,
            out: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn config(&self) -> &NetConfig {
        &self.cfg
    }

    /// `http_proxy` value the guest should use.
    pub fn proxy_url(&self) -> String {
        format!(
            "http://{}:{}",
            fmt_ip(&self.cfg.host_ip),
            self.cfg.proxy_port
        )
    }

    /// Frames to hand to the guest's NIC.
    pub fn take_output(&mut self) -> Vec<Vec<u8>> {
        self.pump_tx();
        core::mem::take(&mut self.out)
    }

    /// Connection events since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.events)
    }

    /// Queue bytes to send to the guest on `id`.
    pub fn send(&mut self, id: ConnId, data: &[u8]) {
        if let Some(c) = self.conns.iter_mut().find(|c| c.id == id) {
            c.tx.extend(data.iter().copied());
        }
    }

    /// Half-close `id`: send FIN once queued data has gone out.
    pub fn close(&mut self, id: ConnId) {
        if let Some(c) = self.conns.iter_mut().find(|c| c.id == id) {
            c.fin_pending = true;
        }
    }

    // ---- inbound ----------------------------------------------------------

    /// Process one Ethernet frame from the guest.
    pub fn input(&mut self, frame: &[u8]) {
        if frame.len() < ETH_HDR {
            return;
        }
        // Learn the guest's MAC from anything it sends; we have no other way to
        // address it, and it is not the MAC the device was configured with
        // unless the guest chose to keep it.
        let src: [u8; 6] = frame[6..12].try_into().unwrap();
        if src != BROADCAST && src != self.cfg.host_mac {
            self.guest_mac = Some(src);
        }
        match be16(&frame[12..14]) {
            ETHERTYPE_ARP => self.on_arp(frame),
            ETHERTYPE_IP => self.on_ip(frame),
            _ => {}
        }
    }

    fn on_arp(&mut self, frame: &[u8]) {
        let Some(arp) = frame.get(ETH_HDR..ETH_HDR + 28) else {
            return;
        };
        // Only requests for our address, and only IPv4-over-Ethernet.
        if be16(&arp[0..2]) != 1 || be16(&arp[2..4]) != ETHERTYPE_IP || be16(&arp[6..8]) != 1 {
            return;
        }
        if arp[24..28] != self.cfg.host_ip {
            return;
        }
        let (sender_mac, sender_ip) = (&arp[8..14], &arp[14..18]);
        let mut out = Vec::with_capacity(42);
        out.extend_from_slice(sender_mac);
        out.extend_from_slice(&self.cfg.host_mac);
        out.extend_from_slice(&ETHERTYPE_ARP.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes()); // htype: ethernet
        out.extend_from_slice(&ETHERTYPE_IP.to_be_bytes()); // ptype: ipv4
        out.extend_from_slice(&[6, 4]); // hlen, plen
        out.extend_from_slice(&2u16.to_be_bytes()); // oper: reply
        out.extend_from_slice(&self.cfg.host_mac);
        out.extend_from_slice(&self.cfg.host_ip);
        out.extend_from_slice(sender_mac);
        out.extend_from_slice(sender_ip);
        self.out.push(out);
    }

    fn on_ip(&mut self, frame: &[u8]) {
        let Some(ip) = frame.get(ETH_HDR..) else {
            return;
        };
        if ip.len() < 20 || ip[0] >> 4 != 4 {
            return;
        }
        let ihl = (ip[0] & 0x0f) as usize * 4;
        let total = be16(&ip[2..4]) as usize;
        if ihl < 20 || total < ihl || ip.len() < total {
            return;
        }
        // A fragment we would have to reassemble: nothing the guest sends to a
        // local proxy should be fragmented, and guessing would corrupt streams.
        if be16(&ip[6..8]) & 0x1fff != 0 {
            return;
        }
        let proto = ip[9];
        let src: [u8; 4] = ip[12..16].try_into().unwrap();
        let payload = &ip[ihl..total];
        match proto {
            IP_PROTO_ICMP => self.on_icmp(src, payload),
            IP_PROTO_UDP => self.on_udp(payload),
            IP_PROTO_TCP => self.on_tcp(src, payload),
            _ => {}
        }
    }

    fn on_icmp(&mut self, src: [u8; 4], icmp: &[u8]) {
        if icmp.len() < 8 || icmp[0] != 8 {
            return; // only echo requests
        }
        let mut reply = icmp.to_vec();
        reply[0] = 0; // echo reply
        reply[2..4].copy_from_slice(&[0, 0]);
        let sum = checksum(&reply);
        reply[2..4].copy_from_slice(&sum.to_be_bytes());
        self.emit_ip(IP_PROTO_ICMP, src, &reply);
    }

    // ---- DHCP -------------------------------------------------------------

    fn on_udp(&mut self, udp: &[u8]) {
        if udp.len() < 8 {
            return;
        }
        let (sport, dport) = (be16(&udp[0..2]), be16(&udp[2..4]));
        // DHCP is the only UDP service: it is what makes the guest configure
        // itself instead of needing a hand-written ifconfig.
        if (sport, dport) == (68, 67) {
            self.on_dhcp(&udp[8..]);
        }
    }

    fn on_dhcp(&mut self, msg: &[u8]) {
        // BOOTP: op(1) htype(1) hlen(1) hops(1) xid(4) secs(2) flags(2)
        //        ciaddr(4) yiaddr(4) siaddr(4) giaddr(4) chaddr(16) ...
        //        sname(64) file(128) magic(4) options
        if msg.len() < 240 || msg[0] != 1 {
            return;
        }
        let xid: [u8; 4] = msg[4..8].try_into().unwrap();
        let chaddr: [u8; 6] = msg[28..34].try_into().unwrap();
        if msg[236..240] != [0x63, 0x82, 0x53, 0x63] {
            return; // not a DHCP magic cookie
        }
        // Option 53 carries the message type: 1 DISCOVER, 3 REQUEST.
        let mut kind = 0u8;
        let mut opts = &msg[240..];
        while opts.len() >= 2 {
            let (code, len) = (opts[0], opts[1] as usize);
            if code == 255 {
                break;
            }
            if code == 0 {
                opts = &opts[1..];
                continue;
            }
            if opts.len() < 2 + len {
                break;
            }
            if code == 53 && len == 1 {
                kind = opts[2];
            }
            opts = &opts[2 + len..];
        }
        let reply_kind = match kind {
            1 => 2, // DISCOVER -> OFFER
            3 => 5, // REQUEST  -> ACK
            _ => return,
        };
        self.guest_mac = Some(chaddr);
        self.emit_dhcp(reply_kind, xid, chaddr);
    }

    fn emit_dhcp(&mut self, kind: u8, xid: [u8; 4], chaddr: [u8; 6]) {
        let mut m = vec![0u8; 240];
        m[0] = 2; // BOOTREPLY
        m[1] = 1; // ethernet
        m[2] = 6; // hlen
        m[4..8].copy_from_slice(&xid);
        m[16..20].copy_from_slice(&self.cfg.guest_ip); // yiaddr
        m[20..24].copy_from_slice(&self.cfg.host_ip); // siaddr
        m[28..34].copy_from_slice(&chaddr);
        m[236..240].copy_from_slice(&[0x63, 0x82, 0x53, 0x63]);
        let opt = |code: u8, data: &[u8], m: &mut Vec<u8>| {
            m.push(code);
            m.push(data.len() as u8);
            m.extend_from_slice(data);
        };
        opt(53, &[kind], &mut m);
        opt(54, &self.cfg.host_ip, &mut m); // server identifier
        opt(51, &3600u32.to_be_bytes(), &mut m); // lease time
        opt(1, &self.cfg.netmask, &mut m);
        opt(3, &self.cfg.host_ip, &mut m); // router
                                           // Offer ourselves as the DNS server so the guest's resolv.conf is
                                           // populated and name lookups fail fast rather than hanging on a
                                           // nonexistent server. Nothing here answers DNS: with a proxy the guest
                                           // never needs to resolve anything, because it hands us the hostname.
        opt(6, &self.cfg.host_ip, &mut m);
        m.push(255);

        // UDP 67 -> 68, broadcast: the guest has no address yet.
        let mut udp = Vec::with_capacity(8 + m.len());
        udp.extend_from_slice(&67u16.to_be_bytes());
        udp.extend_from_slice(&68u16.to_be_bytes());
        udp.extend_from_slice(&((8 + m.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&[0, 0]); // checksum optional in IPv4
        udp.extend_from_slice(&m);
        let frame = self.build_ip(BROADCAST, IP_PROTO_UDP, [255, 255, 255, 255], &udp);
        self.out.push(frame);
    }

    // ---- TCP --------------------------------------------------------------

    fn on_tcp(&mut self, src: [u8; 4], seg: &[u8]) {
        if seg.len() < 20 {
            return;
        }
        let sport = be16(&seg[0..2]);
        let dport = be16(&seg[2..4]);
        let seq = be32(&seg[4..8]);
        let ack = be32(&seg[8..12]);
        let data_off = (seg[12] >> 4) as usize * 4;
        let flags = seg[13];
        let window = be16(&seg[14..16]);
        if data_off < 20 || seg.len() < data_off {
            return;
        }
        let payload = &seg[data_off..];

        // Anything not aimed at the proxy port gets a reset, so the guest fails
        // fast instead of retrying a black hole.
        if dport != self.cfg.proxy_port {
            if flags & TCP_SYN != 0 {
                self.emit_rst(src, sport, dport, seq, payload.len());
            }
            return;
        }

        let existing = self.conns.iter().position(|c| c.guest_port == sport);
        match existing {
            None => {
                if flags & TCP_SYN != 0 && flags & TCP_ACK == 0 {
                    self.accept(sport, seq, window);
                } else if flags & TCP_RST == 0 {
                    self.emit_rst(src, sport, dport, seq, payload.len());
                }
            }
            Some(i) => self.on_segment(i, seq, ack, flags, window, payload),
        }
    }

    fn accept(&mut self, guest_port: u16, seq: u32, window: u16) {
        let id = self.next_id;
        self.next_id += 1;
        // A per-connection ISS. Deliberately deterministic: this stack talks to
        // exactly one peer inside the same process, so unpredictability buys
        // nothing, and reproducible sequence numbers make traces readable.
        let iss = 0x1000_0000u32.wrapping_add(id as u32 * 0x1_0000);
        self.conns.push(Conn {
            id,
            guest_port,
            state: State::SynReceived,
            rcv_nxt: seq.wrapping_add(1),
            snd_una: iss,
            snd_nxt: iss.wrapping_add(1),
            snd_wnd: window,
            unacked: Vec::new(),
            tx: VecDeque::new(),
            fin_pending: false,
            stalled: 0,
        });
        let i = self.conns.len() - 1;
        self.emit_flags(i, TCP_SYN | TCP_ACK, iss);
    }

    fn on_segment(
        &mut self,
        i: usize,
        seq: u32,
        ack: u32,
        flags: u16_flags,
        window: u16,
        payload: &[u8],
    ) {
        let flags = flags;
        if flags & TCP_RST != 0 {
            let id = self.conns[i].id;
            self.conns.remove(i);
            self.events.push(Event::Closed(id));
            return;
        }
        self.conns[i].snd_wnd = window;

        if flags & TCP_ACK != 0 {
            let c = &mut self.conns[i];
            let acked = ack.wrapping_sub(c.snd_una) as usize;
            if acked > 0 && acked <= c.unacked.len() + 1 {
                // The SYN and FIN each consume a sequence number without
                // occupying a byte of `unacked`.
                let data_acked = acked.min(c.unacked.len());
                c.unacked.drain(..data_acked);
                c.snd_una = ack;
                c.stalled = 0;
            }
            if c.state == State::SynReceived {
                c.state = State::Established;
                let id = c.id;
                self.events.push(Event::Opened(id));
            }
            if c.state == State::LastAck && c.unacked.is_empty() {
                self.conns.remove(i);
                return;
            }
        }

        if !payload.is_empty() {
            let c = &mut self.conns[i];
            if seq == c.rcv_nxt {
                c.rcv_nxt = c.rcv_nxt.wrapping_add(payload.len() as u32);
                let id = c.id;
                self.events.push(Event::Data(id, payload.to_vec()));
            }
            // Out of order or a duplicate: re-ACK what we do have and drop it.
            // Cannot normally happen on a lossless local wire, and reassembly
            // would be dead weight.
            self.emit_ack(i);
        }

        if flags & TCP_FIN != 0 {
            let c = &mut self.conns[i];
            if c.state == State::Established || c.state == State::SynReceived {
                c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
                c.state = State::CloseWait;
                let id = c.id;
                self.emit_ack(i);
                self.events.push(Event::Closed(id));
            }
        }
    }

    /// Send whatever is queued, respecting the guest's window, then FIN if the
    /// proxy asked to close and everything has gone out.
    fn pump_tx(&mut self) {
        let mut done = Vec::new();
        for i in 0..self.conns.len() {
            if self.conns[i].state == State::LastAck {
                continue;
            }
            loop {
                let c = &self.conns[i];
                if c.tx.is_empty() {
                    break;
                }
                // In-flight bytes must not exceed what the guest will accept.
                let in_flight = c.snd_nxt.wrapping_sub(c.snd_una) as usize;
                let room = (c.snd_wnd as usize).saturating_sub(in_flight);
                if room == 0 {
                    break;
                }
                let n = c.tx.len().min(MSS).min(room);
                let chunk: Vec<u8> = self.conns[i].tx.drain(..n).collect();
                let seq = self.conns[i].snd_nxt;
                self.conns[i].unacked.extend_from_slice(&chunk);
                self.conns[i].snd_nxt = seq.wrapping_add(n as u32);
                self.emit_segment(i, TCP_PSH | TCP_ACK, seq, &chunk);
            }
            let c = &self.conns[i];
            if c.fin_pending && c.tx.is_empty() && c.state != State::LastAck {
                let seq = c.snd_nxt;
                self.conns[i].snd_nxt = seq.wrapping_add(1);
                self.conns[i].state = State::LastAck;
                self.emit_flags(i, TCP_FIN | TCP_ACK, seq);
            }
            // Retransmit from the oldest unacked byte if the peer has stopped
            // acknowledging: the device drops frames when a mailbox is full, so
            // "lossless" is not quite true.
            let c = &mut self.conns[i];
            if !c.unacked.is_empty() {
                c.stalled += 1;
                if c.stalled >= RETRANSMIT_STALL {
                    c.stalled = 0;
                    let seq = c.snd_una;
                    let chunk: Vec<u8> = c.unacked.iter().take(MSS).copied().collect();
                    self.emit_segment(i, TCP_PSH | TCP_ACK, seq, &chunk);
                }
            }
            if self.conns[i].state == State::LastAck && self.conns[i].unacked.is_empty() {
                // Nothing further to deliver; the guest's ACK removes it, but a
                // guest that never ACKs must not leak the entry forever.
                if self.conns[i].stalled >= RETRANSMIT_STALL {
                    done.push(i);
                }
            }
        }
        for i in done.into_iter().rev() {
            self.conns.remove(i);
        }
    }

    fn emit_ack(&mut self, i: usize) {
        let seq = self.conns[i].snd_nxt;
        self.emit_flags(i, TCP_ACK, seq);
    }

    fn emit_flags(&mut self, i: usize, flags: u8, seq: u32) {
        self.emit_segment(i, flags, seq, &[]);
    }

    fn emit_segment(&mut self, i: usize, flags: u8, seq: u32, payload: &[u8]) {
        let c = &self.conns[i];
        let mut seg = Vec::with_capacity(20 + payload.len());
        seg.extend_from_slice(&self.cfg.proxy_port.to_be_bytes());
        seg.extend_from_slice(&c.guest_port.to_be_bytes());
        seg.extend_from_slice(&seq.to_be_bytes());
        seg.extend_from_slice(&c.rcv_nxt.to_be_bytes());
        let has_options = flags & TCP_SYN != 0;
        let data_off: u8 = if has_options { 6 } else { 5 };
        seg.push(data_off << 4);
        seg.push(flags);
        seg.extend_from_slice(&RCV_WINDOW.to_be_bytes());
        seg.extend_from_slice(&[0, 0]); // checksum, filled below
        seg.extend_from_slice(&[0, 0]); // urgent pointer
        if has_options {
            // MSS option, so the guest does not assume it may send full-MTU
            // segments that our 1500-byte frame limit would reject.
            seg.extend_from_slice(&[2, 4]);
            seg.extend_from_slice(&(MSS as u16).to_be_bytes());
        }
        seg.extend_from_slice(payload);
        let sum = tcp_checksum(&self.cfg.host_ip, &self.cfg.guest_ip, &seg);
        seg[16..18].copy_from_slice(&sum.to_be_bytes());
        let dst = self.guest_mac.unwrap_or(BROADCAST);
        let frame = self.build_ip(dst, IP_PROTO_TCP, self.cfg.guest_ip, &seg);
        self.out.push(frame);
    }

    /// Reset an unexpected segment, so the guest fails immediately.
    fn emit_rst(&mut self, src: [u8; 4], sport: u16, dport: u16, seq: u32, payload_len: usize) {
        let mut seg = vec![0u8; 20];
        seg[0..2].copy_from_slice(&dport.to_be_bytes());
        seg[2..4].copy_from_slice(&sport.to_be_bytes());
        // Acknowledge everything the guest sent so it accepts the reset.
        let ack = seq.wrapping_add(payload_len as u32).wrapping_add(1);
        seg[8..12].copy_from_slice(&ack.to_be_bytes());
        seg[12] = 5 << 4;
        seg[13] = TCP_RST | TCP_ACK;
        let sum = tcp_checksum(&self.cfg.host_ip, &src, &seg);
        seg[16..18].copy_from_slice(&sum.to_be_bytes());
        let dst = self.guest_mac.unwrap_or(BROADCAST);
        let frame = self.build_ip(dst, IP_PROTO_TCP, src, &seg);
        self.out.push(frame);
    }

    // ---- framing ----------------------------------------------------------

    fn emit_ip(&mut self, proto: u8, dst_ip: [u8; 4], payload: &[u8]) {
        let dst = self.guest_mac.unwrap_or(BROADCAST);
        let frame = self.build_ip(dst, proto, dst_ip, payload);
        self.out.push(frame);
    }

    fn build_ip(&self, dst_mac: [u8; 6], proto: u8, dst_ip: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let total = 20 + payload.len();
        let mut f = Vec::with_capacity(ETH_HDR + total);
        f.extend_from_slice(&dst_mac);
        f.extend_from_slice(&self.cfg.host_mac);
        f.extend_from_slice(&ETHERTYPE_IP.to_be_bytes());
        let ip_start = f.len();
        f.push(0x45); // IPv4, 20-byte header
        f.push(0); // DSCP/ECN
        f.extend_from_slice(&(total as u16).to_be_bytes());
        f.extend_from_slice(&[0, 0]); // identification
        f.extend_from_slice(&[0x40, 0]); // don't fragment
        f.push(64); // TTL
        f.push(proto);
        f.extend_from_slice(&[0, 0]); // checksum, filled below
        f.extend_from_slice(&self.cfg.host_ip);
        f.extend_from_slice(&dst_ip);
        let sum = checksum(&f[ip_start..ip_start + 20]);
        f[ip_start + 10..ip_start + 12].copy_from_slice(&sum.to_be_bytes());
        f.extend_from_slice(payload);
        f
    }
}

/// Alias documenting that TCP flags travel in the low byte.
#[allow(non_camel_case_types)]
type u16_flags = u8;

// ---- helpers --------------------------------------------------------------

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

pub fn fmt_ip(ip: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

/// One's-complement sum, as IPv4/ICMP/TCP all use.
fn checksum(data: &[u8]) -> u16 {
    !fold(sum16(data))
}

fn sum16(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for pair in data.chunks(2) {
        sum += if pair.len() == 2 {
            u16::from_be_bytes([pair[0], pair[1]]) as u32
        } else {
            (pair[0] as u32) << 8
        };
    }
    sum
}

fn fold(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    sum as u16
}

/// TCP checksum over the segment plus the IPv4 pseudo-header.
fn tcp_checksum(src: &[u8; 4], dst: &[u8; 4], seg: &[u8]) -> u16 {
    let mut sum = sum16(src) + sum16(dst) + IP_PROTO_TCP as u32 + seg.len() as u32;
    sum += sum16(seg);
    !fold(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUEST_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

    fn stack() -> NetStack {
        NetStack::new(NetConfig::default())
    }

    /// Ethernet + IPv4 frame from the guest.
    fn ip_frame(proto: u8, src: [u8; 4], dst: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&NetConfig::default().host_mac);
        f.extend_from_slice(&GUEST_MAC);
        f.extend_from_slice(&ETHERTYPE_IP.to_be_bytes());
        let start = f.len();
        f.push(0x45);
        f.push(0);
        f.extend_from_slice(&((20 + payload.len()) as u16).to_be_bytes());
        f.extend_from_slice(&[0, 0, 0x40, 0]);
        f.push(64);
        f.push(proto);
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(&src);
        f.extend_from_slice(&dst);
        let sum = checksum(&f[start..start + 20]);
        f[start + 10..start + 12].copy_from_slice(&sum.to_be_bytes());
        f.extend_from_slice(payload);
        f
    }

    fn tcp_seg(sport: u16, dport: u16, seq: u32, ack: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut s = Vec::new();
        s.extend_from_slice(&sport.to_be_bytes());
        s.extend_from_slice(&dport.to_be_bytes());
        s.extend_from_slice(&seq.to_be_bytes());
        s.extend_from_slice(&ack.to_be_bytes());
        s.push(5 << 4);
        s.push(flags);
        s.extend_from_slice(&65535u16.to_be_bytes());
        s.extend_from_slice(&[0, 0, 0, 0]);
        s.extend_from_slice(payload);
        s
    }

    /// Parse a stack-emitted frame as TCP: (flags, seq, ack, payload).
    fn parse_tcp(frame: &[u8]) -> (u8, u32, u32, Vec<u8>) {
        assert_eq!(be16(&frame[12..14]), ETHERTYPE_IP);
        let ip = &frame[ETH_HDR..];
        let ihl = (ip[0] & 0x0f) as usize * 4;
        assert_eq!(ip[9], IP_PROTO_TCP);
        // The IP header checksum must verify: a guest kernel drops the frame
        // silently otherwise, which is invisible from the console.
        assert_eq!(checksum(&ip[..ihl]), 0, "IP header checksum");
        let total = be16(&ip[2..4]) as usize;
        let seg = &ip[ihl..total];
        let src: [u8; 4] = ip[12..16].try_into().unwrap();
        let dst: [u8; 4] = ip[16..20].try_into().unwrap();
        assert_eq!(tcp_checksum(&src, &dst, seg), 0, "TCP checksum");
        let off = (seg[12] >> 4) as usize * 4;
        (
            seg[13],
            be32(&seg[4..8]),
            be32(&seg[8..12]),
            seg[off..].to_vec(),
        )
    }

    /// Complete a handshake on `port`; returns (conn id, our ISS+1).
    fn handshake(s: &mut NetStack, port: u16, guest_seq: u32) -> (ConnId, u32) {
        s.input(&ip_frame(
            IP_PROTO_TCP,
            NetConfig::default().guest_ip,
            NetConfig::default().host_ip,
            &tcp_seg(port, 8080, guest_seq, 0, TCP_SYN, &[]),
        ));
        let out = s.take_output();
        assert_eq!(out.len(), 1, "SYN must be answered");
        let (flags, seq, ack, _) = parse_tcp(&out[0]);
        assert_eq!(flags, TCP_SYN | TCP_ACK);
        assert_eq!(ack, guest_seq.wrapping_add(1), "SYN-ACK must ack the SYN");
        // Guest completes the handshake.
        s.input(&ip_frame(
            IP_PROTO_TCP,
            NetConfig::default().guest_ip,
            NetConfig::default().host_ip,
            &tcp_seg(port, 8080, guest_seq + 1, seq.wrapping_add(1), TCP_ACK, &[]),
        ));
        let events = s.take_events();
        assert_eq!(events.len(), 1);
        let id = match events[0] {
            Event::Opened(id) => id,
            ref e => panic!("expected Opened, got {e:?}"),
        };
        (id, seq.wrapping_add(1))
    }

    #[test]
    fn answers_arp_for_its_own_address_only() {
        let mut s = stack();
        let cfg = NetConfig::default();
        let mut arp = Vec::new();
        arp.extend_from_slice(&BROADCAST);
        arp.extend_from_slice(&GUEST_MAC);
        arp.extend_from_slice(&ETHERTYPE_ARP.to_be_bytes());
        arp.extend_from_slice(&1u16.to_be_bytes());
        arp.extend_from_slice(&ETHERTYPE_IP.to_be_bytes());
        arp.extend_from_slice(&[6, 4]);
        arp.extend_from_slice(&1u16.to_be_bytes()); // request
        arp.extend_from_slice(&GUEST_MAC);
        arp.extend_from_slice(&cfg.guest_ip);
        arp.extend_from_slice(&[0; 6]);
        arp.extend_from_slice(&cfg.host_ip);
        s.input(&arp);
        let out = s.take_output();
        assert_eq!(out.len(), 1);
        let reply = &out[0];
        assert_eq!(&reply[0..6], &GUEST_MAC, "reply is unicast to the asker");
        assert_eq!(&reply[6..12], &cfg.host_mac);
        assert_eq!(be16(&reply[20..22]), 2, "oper = reply");
        assert_eq!(&reply[22..28], &cfg.host_mac, "our MAC answers");
        assert_eq!(&reply[28..32], &cfg.host_ip);

        // An ARP for some other address is not ours to answer.
        let mut other = arp.clone();
        other[38..42].copy_from_slice(&[10, 0, 2, 99]);
        s.input(&other);
        assert!(s.take_output().is_empty());
    }

    #[test]
    fn dhcp_discover_and_request_get_offer_and_ack() {
        let mut s = stack();
        let cfg = NetConfig::default();
        let build = |kind: u8| {
            let mut m = vec![0u8; 240];
            m[0] = 1; // BOOTREQUEST
            m[1] = 1;
            m[2] = 6;
            m[4..8].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
            m[28..34].copy_from_slice(&GUEST_MAC);
            m[236..240].copy_from_slice(&[0x63, 0x82, 0x53, 0x63]);
            m.extend_from_slice(&[53, 1, kind, 255]);
            let mut udp = Vec::new();
            udp.extend_from_slice(&68u16.to_be_bytes());
            udp.extend_from_slice(&67u16.to_be_bytes());
            udp.extend_from_slice(&((8 + m.len()) as u16).to_be_bytes());
            udp.extend_from_slice(&[0, 0]);
            udp.extend_from_slice(&m);
            ip_frame(IP_PROTO_UDP, [0, 0, 0, 0], [255, 255, 255, 255], &udp)
        };

        for (request_kind, want_reply) in [(1u8, 2u8), (3, 5)] {
            s.input(&build(request_kind));
            let out = s.take_output();
            assert_eq!(out.len(), 1, "DHCP {request_kind} must be answered");
            let ip = &out[0][ETH_HDR..];
            assert_eq!(ip[9], IP_PROTO_UDP);
            assert_eq!(checksum(&ip[..20]), 0, "IP header checksum");
            let dhcp = &ip[28..];
            assert_eq!(dhcp[0], 2, "BOOTREPLY");
            assert_eq!(&dhcp[4..8], &[0xde, 0xad, 0xbe, 0xef], "xid echoed");
            assert_eq!(
                &dhcp[16..20],
                &cfg.guest_ip,
                "yiaddr is the offered address"
            );
            // Option 53 must carry the matching reply type, and the gateway and
            // netmask must be present or the guest cannot route to us.
            let opts = &dhcp[240..];
            let find = |code: u8| -> Option<Vec<u8>> {
                let mut o = opts;
                while o.len() >= 2 && o[0] != 255 {
                    let len = o[1] as usize;
                    if o[0] == code {
                        return Some(o[2..2 + len].to_vec());
                    }
                    o = &o[2 + len..];
                }
                None
            };
            assert_eq!(find(53), Some(vec![want_reply]));
            assert_eq!(find(1), Some(cfg.netmask.to_vec()), "subnet mask");
            assert_eq!(find(3), Some(cfg.host_ip.to_vec()), "router");
            assert_eq!(find(54), Some(cfg.host_ip.to_vec()), "server id");
        }
    }

    #[test]
    fn replies_to_ping() {
        let mut s = stack();
        let cfg = NetConfig::default();
        let mut icmp = vec![8, 0, 0, 0, 0x12, 0x34, 0, 1];
        icmp.extend_from_slice(b"payload-bytes");
        let sum = checksum(&icmp);
        icmp[2..4].copy_from_slice(&sum.to_be_bytes());
        s.input(&ip_frame(IP_PROTO_ICMP, cfg.guest_ip, cfg.host_ip, &icmp));
        let out = s.take_output();
        assert_eq!(out.len(), 1);
        let ip = &out[0][ETH_HDR..];
        assert_eq!(checksum(&ip[..20]), 0, "IP header checksum");
        let reply = &ip[20..];
        assert_eq!(reply[0], 0, "echo reply");
        assert_eq!(checksum(reply), 0, "ICMP checksum");
        assert_eq!(&reply[4..8], &icmp[4..8], "id/seq echoed");
        assert_eq!(&reply[8..], b"payload-bytes");
    }

    #[test]
    fn handshake_then_data_both_ways() {
        let mut s = stack();
        let cfg = NetConfig::default();
        let (id, our_seq) = handshake(&mut s, 40000, 1000);

        // Guest sends a request.
        s.input(&ip_frame(
            IP_PROTO_TCP,
            cfg.guest_ip,
            cfg.host_ip,
            &tcp_seg(
                40000,
                8080,
                1001,
                our_seq,
                TCP_ACK | TCP_PSH,
                b"hello proxy",
            ),
        ));
        assert_eq!(
            s.take_events(),
            vec![Event::Data(id, b"hello proxy".to_vec())]
        );
        let out = s.take_output();
        let (flags, _, ack, _) = parse_tcp(&out[0]);
        assert_eq!(flags, TCP_ACK);
        assert_eq!(ack, 1001 + 11, "must acknowledge exactly what arrived");

        // We reply.
        s.send(id, b"hello guest");
        let out = s.take_output();
        assert_eq!(out.len(), 1);
        let (flags, seq, _, payload) = parse_tcp(&out[0]);
        assert_eq!(flags, TCP_PSH | TCP_ACK);
        assert_eq!(seq, our_seq);
        assert_eq!(payload, b"hello guest");
    }

    #[test]
    fn a_large_response_is_split_into_mss_segments() {
        let mut s = stack();
        let (id, _) = handshake(&mut s, 40001, 5000);
        let body = vec![0x41u8; MSS * 2 + 100];
        s.send(id, &body);
        let out = s.take_output();
        assert_eq!(out.len(), 3, "two full segments plus a remainder");
        let mut seen = Vec::new();
        for (i, frame) in out.iter().enumerate() {
            let (_, _, _, payload) = parse_tcp(frame);
            if i < 2 {
                assert_eq!(payload.len(), MSS, "segment {i} must be a full MSS");
            }
            seen.extend(payload);
        }
        assert_eq!(seen, body, "stream reassembles to what was sent");
    }

    #[test]
    fn the_guest_window_bounds_what_we_send() {
        let mut s = stack();
        let cfg = NetConfig::default();
        // SYN advertising a small window.
        let mut syn = tcp_seg(40002, 8080, 900, 0, TCP_SYN, &[]);
        syn[14..16].copy_from_slice(&100u16.to_be_bytes());
        s.input(&ip_frame(IP_PROTO_TCP, cfg.guest_ip, cfg.host_ip, &syn));
        let (_, iss, _, _) = parse_tcp(&s.take_output()[0]);
        let mut ack = tcp_seg(40002, 8080, 901, iss.wrapping_add(1), TCP_ACK, &[]);
        ack[14..16].copy_from_slice(&100u16.to_be_bytes());
        s.input(&ip_frame(IP_PROTO_TCP, cfg.guest_ip, cfg.host_ip, &ack));
        let id = match s.take_events()[0] {
            Event::Opened(id) => id,
            ref e => panic!("{e:?}"),
        };

        s.send(id, &vec![0x42u8; 500]);
        let out = s.take_output();
        let total: usize = out.iter().map(|f| parse_tcp(f).3.len()).sum();
        assert_eq!(total, 100, "must not exceed the advertised window");

        // Acknowledging the first 100 bytes opens the window for the next 100.
        let mut more = tcp_seg(40002, 8080, 901, iss.wrapping_add(1 + 100), TCP_ACK, &[]);
        more[14..16].copy_from_slice(&100u16.to_be_bytes());
        s.input(&ip_frame(IP_PROTO_TCP, cfg.guest_ip, cfg.host_ip, &more));
        let total: usize = s.take_output().iter().map(|f| parse_tcp(f).3.len()).sum();
        assert_eq!(total, 100, "window reopens as data is acknowledged");
    }

    #[test]
    fn guest_fin_closes_and_our_close_sends_fin() {
        let mut s = stack();
        let cfg = NetConfig::default();
        let (id, our_seq) = handshake(&mut s, 40003, 7000);
        s.input(&ip_frame(
            IP_PROTO_TCP,
            cfg.guest_ip,
            cfg.host_ip,
            &tcp_seg(40003, 8080, 7001, our_seq, TCP_FIN | TCP_ACK, &[]),
        ));
        assert_eq!(s.take_events(), vec![Event::Closed(id)]);
        let (flags, _, ack, _) = parse_tcp(&s.take_output()[0]);
        assert_eq!(flags, TCP_ACK);
        assert_eq!(ack, 7002, "the FIN consumes a sequence number");

        // A half-close still lets us finish sending, then FIN.
        s.send(id, b"trailing body");
        s.close(id);
        let out = s.take_output();
        assert_eq!(out.len(), 2, "data then FIN");
        assert_eq!(parse_tcp(&out[0]).3, b"trailing body");
        assert_eq!(parse_tcp(&out[1]).0, TCP_FIN | TCP_ACK);
    }

    #[test]
    fn a_connection_to_any_other_port_is_reset() {
        let mut s = stack();
        let cfg = NetConfig::default();
        // The guest must fail fast rather than retry into a black hole: this
        // stack forwards nothing, so only the proxy port can ever answer.
        s.input(&ip_frame(
            IP_PROTO_TCP,
            cfg.guest_ip,
            cfg.host_ip,
            &tcp_seg(40004, 443, 1, 0, TCP_SYN, &[]),
        ));
        let out = s.take_output();
        assert_eq!(out.len(), 1);
        let (flags, _, ack, _) = parse_tcp(&out[0]);
        assert_eq!(flags, TCP_RST | TCP_ACK);
        assert_eq!(ack, 2);
        assert!(s.take_events().is_empty());
    }

    #[test]
    fn unacked_data_is_retransmitted_after_a_stall() {
        let mut s = stack();
        let (id, _) = handshake(&mut s, 40005, 3000);
        s.send(id, b"first send");
        let first = s.take_output();
        assert_eq!(first.len(), 1);
        // The guest never acknowledges — a frame the device dropped looks
        // exactly like this — so the data must go out again rather than stall.
        let mut retransmits = 0;
        for _ in 0..RETRANSMIT_STALL + 1 {
            for f in s.take_output() {
                if parse_tcp(&f).3 == b"first send" {
                    retransmits += 1;
                }
            }
        }
        assert!(retransmits >= 1, "expected a retransmission");
    }

    #[test]
    fn out_of_order_data_is_re_acked_not_accepted() {
        let mut s = stack();
        let cfg = NetConfig::default();
        let (id, our_seq) = handshake(&mut s, 40006, 100);
        // A segment from the future must not be delivered as if it were in
        // sequence; that would silently corrupt the byte stream.
        s.input(&ip_frame(
            IP_PROTO_TCP,
            cfg.guest_ip,
            cfg.host_ip,
            &tcp_seg(
                40006,
                8080,
                9999,
                our_seq,
                TCP_ACK | TCP_PSH,
                b"from the future",
            ),
        ));
        assert!(s.take_events().is_empty(), "gap must not be delivered");
        let (flags, _, ack, _) = parse_tcp(&s.take_output()[0]);
        assert_eq!(flags, TCP_ACK);
        assert_eq!(ack, 101, "re-ack what we actually have");
        let _ = id;
    }
}
