//! End-to-end HTTP proxy: boot Linux, point the guest's `http_proxy` at the
//! in-process proxy, and fetch through it.
//!
//! Nothing external is involved — no relay, no network, no privileges. The full
//! path under test is: guest TCP/IP → virtio-net → netstack (ARP + TCP
//! termination) → proxy (HTTP parse) → Egress → response → back up the same
//! path. That is exactly the path a browser build takes, with `fetch` in place
//! of the canned Egress here.
//!
//! Guest is the TinyEMU image set (`web/get-images.sh`). Skips when absent.

use rv64_system::httpproxy::{Completion, Egress, Proxy, ReqId, Request};
use rv64_system::netstack::{NetConfig, NetStack};
use rv64_system::{virtio, BootImages, Machine};
use std::path::PathBuf;

fn images() -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/images");
    let read = |f: &str| std::fs::read(dir.join(f)).ok();
    Some((
        read("bbl64.bin")?,
        read("kernel-riscv64.bin")?,
        read("root-riscv64.bin")?,
    ))
}

/// Answers from a fixed table, so the test depends on no network. Records every
/// request so the guest-visible result can be corroborated against what the
/// proxy actually produced.
#[derive(Default)]
struct CannedEgress {
    seen: Vec<Request>,
    done: Vec<Completion>,
    /// Body chunks released one per `poll`, to exercise a response that arrives
    /// over time rather than all at once.
    trickle: Vec<(ReqId, Vec<u8>)>,
}

fn body_for(url: &str, req: &Request) -> Option<Vec<u8>> {
    match url {
        "http://example.test/hello" => Some(b"PROXY-BODY-OK\n".to_vec()),
        // Big enough to span many TCP segments and many body chunks.
        "http://example.test/big" | "http://example.test/slow" => {
            Some((0..20000u32).map(|i| b'a' + (i % 26) as u8).collect())
        }
        "http://example.test/echo" => Some(
            format!(
                "METHOD={} BODY={}\n",
                req.method,
                String::from_utf8_lossy(&req.body)
            )
            .into_bytes(),
        ),
        "http://example.test/missing" => Some(b"nope\n".to_vec()),
        _ => None,
    }
}

impl Egress for CannedEgress {
    fn submit(&mut self, id: ReqId, req: Request) {
        let url = req.url.clone();
        let Some(body) = body_for(&url, &req) else {
            self.seen.push(req);
            self.done.push(Completion::Failed {
                id,
                error: "no route to host in canned egress".into(),
            });
            return;
        };
        let status = if url.ends_with("/missing") { 404 } else { 200 };
        self.done.push(Completion::Head {
            id,
            status,
            headers: vec![("Content-Type".into(), "text/plain".into())],
        });
        if url.ends_with("/slow") {
            // Release one chunk per poll: the guest must assemble a response
            // that arrives across many slices, which is what an SSE stream or a
            // slow download looks like.
            for chunk in body.chunks(1000) {
                self.trickle.push((id, chunk.to_vec()));
            }
            self.trickle.push((id, Vec::new())); // sentinel: end
        } else {
            self.done.push(Completion::Body { id, bytes: body });
            self.done.push(Completion::End { id });
        }
        self.seen.push(req);
    }

    fn poll(&mut self) -> Vec<Completion> {
        let mut out = core::mem::take(&mut self.done);
        if !self.trickle.is_empty() {
            let (id, bytes) = self.trickle.remove(0);
            out.push(if bytes.is_empty() {
                Completion::End { id }
            } else {
                Completion::Body { id, bytes }
            });
        }
        out
    }
}

/// Guest plus the host-side plumbing, pumped together each slice.
struct Harness {
    m: Machine,
    stack: NetStack,
    proxy: Proxy,
    egress: CannedEgress,
    out: String,
}

impl Harness {
    fn wait_for(&mut self, needle: &str, slices: usize) -> bool {
        for _ in 0..slices {
            self.m.run_slice(5_000_000);
            let chunk = self.m.console_output();
            if !chunk.is_empty() {
                self.out.push_str(&String::from_utf8_lossy(&chunk));
            }
            // The whole host-side path, once per slice.
            for frame in self.m.net_take_output() {
                self.stack.input(&frame);
            }
            self.proxy.pump(&mut self.stack, &mut self.egress);
            for frame in self.stack.take_output() {
                self.m.net_input(&frame);
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
    /// type, so `marker` must not appear verbatim in `cmd`.
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
    lines[lines.len().saturating_sub(30)..].join("\n")
}

#[test]
fn guest_fetches_through_the_in_process_proxy() {
    let Some((bios, kernel, disk)) = images() else {
        eprintln!("SKIP proxy_boot (run web/get-images.sh)");
        return;
    };

    let cfg = NetConfig::default();
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
    let mut h = Harness {
        m,
        stack: NetStack::new(cfg),
        // The canned egress speaks plaintext, so keep the guest's scheme rather
        // than upgrading to https as a browser build must.
        proxy: Proxy::new().keep_scheme(),
        egress: CannedEgress::default(),
        out: String::new(),
    };

    assert!(
        h.wait_for("~ #", 4000),
        "guest never reached a shell:\n{}",
        tail(&h.out)
    );

    // Configure by DHCP rather than by hand: the netstack serves the lease, and
    // this is the path a user (or a browser demo) actually takes.
    //
    // The lease has to be *applied* by a script udhcpc runs, and this guest
    // image ships none — busybox has the default path compiled in
    // (/usr/share/udhcpc/default.script) but the file does not exist, so
    // udhcpc reports a lease and leaves the interface unconfigured. Supply a
    // minimal one so the test exercises the whole path rather than just the
    // protocol exchange.
    assert!(
        h.run(
            r#"printf '#!/bin/sh\n[ "$1" = bound ] && ifconfig $interface $ip netmask $subnet\nexit 0\n' > /tmp/dh.sh; chmod +x /tmp/dh.sh; echo SCRIPT_'O'K"#,
            "SCRIPT_OK",
            2000
        ),
        "could not write the udhcpc script:\n{}",
        tail(&h.out)
    );
    // `-n -q` makes udhcpc exit once it has a lease instead of daemonising.
    assert!(
        h.run(
            "ifconfig eth0 up; udhcpc -i eth0 -n -q -s /tmp/dh.sh; echo DHCP_'O'K",
            "DHCP_OK",
            4000
        ),
        "udhcpc never finished:\n{}",
        tail(&h.out)
    );
    // The offer itself must have come from us, with the address we serve.
    assert!(
        h.out.contains(&format!("Lease of {} obtained", ip(&cfg.guest_ip))),
        "udhcpc did not take our lease:\n{}",
        tail(&h.out)
    );
    // The lease must actually have been applied to the interface, not merely
    // offered — udhcpc reports success even when its script does nothing.
    assert!(
        h.run(
            &format!(
                "ifconfig eth0 | grep -q {} && echo LEASE_'O'K",
                ip(&cfg.guest_ip)
            ),
            "LEASE_OK",
            2000
        ),
        "the DHCP lease was not applied to eth0:\n{}",
        tail(&h.out)
    );

    // The proxy is on-link, so ARP alone should reach it. Prove that before
    // blaming HTTP for anything that follows.
    assert!(
        h.run(&format!("ping -c 1 -W 5 {}", ip(&cfg.host_ip)), "1 packets received", 3000),
        "the proxy host is not reachable at all:\n{}",
        tail(&h.out)
    );

    let proxy_env = format!("export http_proxy={}", h.stack.proxy_url());
    assert!(
        h.run(&format!("{proxy_env}; echo SET_'O'K"), "SET_OK", 1000),
        "could not set http_proxy:\n{}",
        tail(&h.out)
    );

    // The actual thing: an unmodified client fetching a URL through the proxy.
    assert!(
        h.run("wget -q -O- http://example.test/hello", "PROXY-BODY-OK", 3000),
        "fetch through the proxy failed:\n{}",
        tail(&h.out)
    );
    assert_eq!(h.egress.seen.len(), 1, "exactly one request reached egress");
    let req = &h.egress.seen[0];
    assert_eq!(req.method, "GET");
    assert_eq!(req.url, "http://example.test/hello");
    // The client sent absolute-URI form because it was talking to a proxy, and
    // the hop-by-hop headers it adds for the proxy hop were stripped.
    let names: Vec<String> = req.headers.iter().map(|(n, _)| n.to_lowercase()).collect();
    assert!(!names.contains(&"proxy-connection".to_string()), "got {names:?}");
    assert!(!names.contains(&"connection".to_string()), "got {names:?}");

    // A response larger than one segment must reassemble byte-exact in the guest.
    assert!(
        h.run(
            "wget -q -O- http://example.test/big | wc -c",
            "20000",
            4000
        ),
        "a multi-segment response did not arrive intact:\n{}",
        tail(&h.out)
    );
    assert!(
        h.run(
            "wget -q -O- http://example.test/big | md5sum | cut -c1-8; echo D'O'NE",
            "DONE",
            4000
        ),
        "checksum command did not run:\n{}",
        tail(&h.out)
    );
    // Compare against the same bytes hashed on the host, so a corrupted stream
    // that happens to have the right length still fails.
    let expected: Vec<u8> = (0..20000u32).map(|i| b'a' + (i % 26) as u8).collect();
    let want = md5_hex(&expected);
    assert!(
        h.out.contains(&want[..8]),
        "guest md5 does not match host md5 ({}):\n{}",
        &want[..8],
        tail(&h.out)
    );

    // A response that arrives in pieces across many slices must reassemble
    // identically — the streaming path, as an SSE or slow download would use it.
    assert!(
        h.run(
            "wget -q -O- http://example.test/slow | md5sum | cut -c1-8; echo SL'O'W",
            "SLOW",
            8000
        ),
        "a trickled response never completed:\n{}",
        tail(&h.out)
    );
    assert!(
        h.out.matches(&want[..8]).count() >= 2,
        "the trickled response does not hash the same as the buffered one ({}):\n{}",
        &want[..8],
        tail(&h.out)
    );

    // An upstream error status is passed through, not turned into a proxy error.
    assert!(
        h.run(
            "wget -q -O- http://example.test/missing; echo RC$?",
            "RC1",
            3000
        ),
        "a 404 was not surfaced to the client:\n{}",
        tail(&h.out)
    );

    // A host the egress cannot reach becomes a 502 the client can read.
    assert!(
        h.run(
            "wget -q -O- http://nowhere.test/ 2>&1 | head -2; echo D'O'NE2",
            "DONE2",
            3000
        ),
        "an egress failure produced no response at all:\n{}",
        tail(&h.out)
    );
    assert!(
        h.out.contains("502"),
        "expected a 502 to reach the guest:\n{}",
        tail(&h.out)
    );
}

fn ip(o: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3])
}

/// MD5, only so the test can compare a guest-computed digest of a large
/// response body against the same bytes hashed here.
fn md5_hex(data: &[u8]) -> String {
    let s: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    let mut h: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = (0..16)
            .map(|i| u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap()))
            .collect();
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let tmp = d;
            d = c;
            c = b;
            let sum = a
                .wrapping_add(f)
                .wrapping_add(k[i])
                .wrapping_add(m[g]);
            b = b.wrapping_add(sum.rotate_left(s[i]));
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
    }
    h.iter()
        .flat_map(|w| w.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}
