//! End-to-end virtio-net: boot Linux, bring the NIC up, and ping a host that
//! this test implements.
//!
//! The device is layer 2 only, so proving it works means proving frames cross
//! it intact in both directions and that the guest's own TCP/IP stack accepts
//! what comes back. A ~60-line ARP + ICMP responder here is the smallest peer
//! that can demonstrate that, and it needs no relay, no privileges and no
//! network — the "host" is a function in this file.
//!
//! What a passing run establishes, beyond "frames move":
//!   * the guest uses the MAC we put in config space (so `VIRTIO_NET_F_MAC` and
//!     byte-wise config reads work — otherwise Linux invents a random address);
//!   * the 12-byte `virtio_net_hdr_v1` offset is right in both directions (a
//!     10-byte header would shift every frame and no reply would ever parse);
//!   * RX delivery reaches a guest that was not notifying us (`poll_net_rx`).
//!
//! Guest is the TinyEMU image set (`web/get-images.sh`), whose kernel has
//! virtio_net built in. Skips (passes) when the images are absent.

use rv64_system::{virtio, BootImages, Machine};
use std::path::PathBuf;

const GUEST_IP: [u8; 4] = [10, 0, 2, 15];
const HOST_IP: [u8; 4] = [10, 0, 2, 2];
const HOST_MAC: [u8; 6] = [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02];

fn images() -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/images");
    let read = |f: &str| std::fs::read(dir.join(f)).ok();
    Some((
        read("bbl64.bin")?,
        read("kernel-riscv64.bin")?,
        read("root-riscv64.bin")?,
    ))
}

// ---- the "network" on the other side of the NIC ---------------------------

/// One's-complement sum used by both IP and ICMP.
fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    for pair in data.chunks(2) {
        let word = if pair.len() == 2 {
            u16::from_be_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], 0])
        };
        sum += word as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Answer one guest frame, or `None` if it is not for us / not understood.
fn respond(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < 14 {
        return None;
    }
    match u16::from_be_bytes([frame[12], frame[13]]) {
        0x0806 => arp_reply(frame),
        0x0800 => icmp_reply(frame),
        _ => None, // IPv6 solicitations and the like: silently ignored
    }
}

fn arp_reply(frame: &[u8]) -> Option<Vec<u8>> {
    let arp = frame.get(14..42)?;
    let oper = u16::from_be_bytes([arp[6], arp[7]]);
    let target_ip = &arp[24..28];
    if oper != 1 || target_ip != HOST_IP {
        return None; // not a request, or not asking about us
    }
    let (sender_mac, sender_ip) = (&arp[8..14], &arp[14..18]);
    let mut out = Vec::with_capacity(42);
    out.extend_from_slice(sender_mac); // eth dst
    out.extend_from_slice(&HOST_MAC); // eth src
    out.extend_from_slice(&0x0806u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // htype: ethernet
    out.extend_from_slice(&0x0800u16.to_be_bytes()); // ptype: ipv4
    out.extend_from_slice(&[6, 4]); // hlen, plen
    out.extend_from_slice(&2u16.to_be_bytes()); // oper: reply
    out.extend_from_slice(&HOST_MAC);
    out.extend_from_slice(&HOST_IP);
    out.extend_from_slice(sender_mac);
    out.extend_from_slice(sender_ip);
    Some(out)
}

fn icmp_reply(frame: &[u8]) -> Option<Vec<u8>> {
    let ip_start = 14;
    let ihl = ((frame.get(ip_start)? & 0x0f) as usize) * 4;
    let total_len = u16::from_be_bytes([frame[ip_start + 2], frame[ip_start + 3]]) as usize;
    if frame[ip_start + 9] != 1 || frame[ip_start + 16..ip_start + 20] != HOST_IP {
        return None; // not ICMP, or not addressed to us
    }
    let icmp_start = ip_start + ihl;
    let icmp_end = (ip_start + total_len).min(frame.len());
    if frame.get(icmp_start)? != &8 {
        return None; // not an echo request
    }

    let mut out = Vec::with_capacity(icmp_end);
    out.extend_from_slice(&frame[6..12]); // eth dst = their src
    out.extend_from_slice(&HOST_MAC);
    out.extend_from_slice(&0x0800u16.to_be_bytes());
    // IP header: same as the request with the addresses swapped.
    let mut ip = frame[ip_start..icmp_start].to_vec();
    ip[10..12].copy_from_slice(&[0, 0]); // zero before recomputing
    ip[12..16].copy_from_slice(&HOST_IP);
    ip[16..20].copy_from_slice(&GUEST_IP);
    let ip_csum = checksum(&ip);
    ip[10..12].copy_from_slice(&ip_csum.to_be_bytes());
    out.extend_from_slice(&ip);
    // ICMP: echo request (8) becomes echo reply (0), id/seq/payload echoed back.
    let mut icmp = frame[icmp_start..icmp_end].to_vec();
    icmp[0] = 0;
    icmp[2..4].copy_from_slice(&[0, 0]);
    let icmp_csum = checksum(&icmp);
    icmp[2..4].copy_from_slice(&icmp_csum.to_be_bytes());
    out.extend_from_slice(&icmp);
    Some(out)
}

// ---- harness --------------------------------------------------------------

struct Guest {
    m: Machine,
    out: String,
    /// Every frame the guest transmitted, for after-the-fact assertions.
    sent: Vec<Vec<u8>>,
    replied: usize,
}

impl Guest {
    /// Run until `needle` appears, moving frames both ways every slice.
    fn wait_for(&mut self, needle: &str, slices: usize) -> bool {
        for _ in 0..slices {
            self.m.run_slice(5_000_000);
            let chunk = self.m.console_output();
            if !chunk.is_empty() {
                self.out.push_str(&String::from_utf8_lossy(&chunk));
            }
            for frame in self.m.net_take_output() {
                if let Some(reply) = respond(&frame) {
                    let _ = self.m.net_input(&reply);
                    self.replied += 1;
                }
                self.sent.push(frame);
            }
            if self.out.contains(needle) {
                return true;
            }
            if self.m.power_off {
                break;
            }
        }
        false
    }

    /// Run `cmd` and wait for `marker` in its output. The guest echoes what we
    /// type, so `marker` must not appear verbatim in `cmd` — quote a character
    /// (`OK_'x'`) to keep the typed and printed forms different.
    fn run(&mut self, cmd: &str, marker: &str, slices: usize) -> bool {
        assert!(
            !cmd.contains(marker),
            "marker {marker:?} appears in the command, so it would match the echo"
        );
        self.m.console_input(cmd.as_bytes());
        self.m.console_input(b"\n");
        self.wait_for(marker, slices)
    }
}

fn tail(out: &str) -> String {
    let lines: Vec<&str> = out.lines().collect();
    lines[lines.len().saturating_sub(25)..].join("\n")
}

#[test]
fn guest_pings_a_host_over_virtio_net() {
    let Some((bios, kernel, disk)) = images() else {
        eprintln!("SKIP net_boot (run web/get-images.sh)");
        return;
    };

    let m = Machine::new(
        128,
        BootImages {
            bios: &bios,
            kernel: Some(&kernel),
            cmdline: "console=hvc0 root=/dev/vda rw",
            disk: Some(disk),
            fs: vec![],
            net: Some(virtio::DEFAULT_MAC),
        },
    );
    let mut g = Guest {
        m,
        out: String::new(),
        sent: Vec::new(),
        replied: 0,
    };

    assert!(
        g.wait_for("~ #", 4000),
        "guest never reached a shell:\n{}",
        tail(&g.out)
    );

    // The NIC must have probed. Its MAC comes from our config space.
    assert!(
        g.run("ifconfig -a | grep -A1 eth0", "HWaddr", 2000),
        "no eth0 interface:\n{}",
        tail(&g.out)
    );
    let want_mac = virtio::DEFAULT_MAC
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");
    assert!(
        g.out.to_uppercase().contains(&want_mac),
        "guest MAC is not the one we advertised ({want_mac}):\n{}",
        tail(&g.out)
    );

    assert!(
        g.run(
            &format!(
                "ifconfig eth0 {} netmask 255.255.255.0 up && echo UP_'O'K",
                fmt_ip(&GUEST_IP)
            ),
            "UP_OK",
            2000,
        ),
        "bringing eth0 up failed:\n{}",
        tail(&g.out)
    );

    // The ping itself: ARP resolve, then echo request/reply. Both directions
    // and the guest's own stack all have to be right for this to come back.
    assert!(
        g.run(
            &format!("ping -c 2 -W 5 {}", fmt_ip(&HOST_IP)),
            "2 packets received",
            4000,
        ),
        "ping got no replies (guest sent {} frames, we answered {}):\n{}",
        g.sent.len(),
        g.replied,
        tail(&g.out)
    );

    // Corroborate at the frame level, so a passing ping cannot be a console
    // artefact: the guest really put an ARP request for our IP on the wire,
    // sourced from the MAC we gave it.
    let arp = g
        .sent
        .iter()
        .find(|f| f.len() >= 42 && f[12..14] == [0x08, 0x06] && f[38..42] == HOST_IP)
        .unwrap_or_else(|| {
            panic!(
                "no ARP request for {} among {} frames",
                fmt_ip(&HOST_IP),
                g.sent.len()
            )
        });
    assert_eq!(&arp[6..12], &virtio::DEFAULT_MAC, "ARP sender MAC");
    assert_eq!(&arp[0..6], &[0xff; 6], "ARP requests are broadcast");
    // And that we saw echo requests, not just ARP.
    let echoes = g
        .sent
        .iter()
        .filter(|f| f.len() > 34 && f[12..14] == [0x08, 0x00] && f[23] == 1 && f[34] == 8)
        .count();
    assert!(
        echoes >= 2,
        "expected 2 echo requests on the wire, saw {echoes}"
    );
}

fn fmt_ip(ip: &[u8; 4]) -> String {
    ip.iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(".")
}
