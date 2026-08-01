//! Minimal WebSocket client (RFC 6455) for the virtio-net relay.
//!
//! The net device only moves Ethernet frames; something has to carry them off
//! the host. A relay is the transport we picked (see ROADMAP item 8): the proxy
//! at the far end holds the privileges and does the NAT, so the emulator needs
//! no `CAP_NET_ADMIN` and no userspace TCP/IP stack of its own. It is also the
//! only option in a browser, which means one protocol serves both targets.
//!
//! Wire format is v86's, and websockproxy's: **one binary WebSocket message per
//! Ethernet frame**, no envelope.
//!
//! Deliberately small — no external crates, matching the rest of this crate:
//! client-side only, `ws://` only (no TLS), text frames ignored. It does verify
//! the server's `Sec-WebSocket-Accept`, because the alternative to checking the
//! handshake is silently reading garbage from whatever answered the port.

use std::io::{Read, Write};
use std::net::TcpStream;

/// A connected relay. Frames in, frames out; no interpretation of contents.
#[derive(Debug)]
pub struct Relay {
    sock: TcpStream,
    /// Bytes read but not yet forming a complete WebSocket frame.
    pending: Vec<u8>,
    /// Payload accumulated across a fragmented message.
    partial: Vec<u8>,
    closed: bool,
}

impl Relay {
    /// Connect to `ws://host[:port][/path]` and complete the upgrade.
    pub fn connect(url: &str) -> Result<Relay, String> {
        let rest = url
            .strip_prefix("ws://")
            .ok_or_else(|| match url.starts_with("wss://") {
                // Being explicit beats a confusing handshake failure: TLS would
                // mean either a TLS stack or a crate dependency, and a relay is
                // usually reached over loopback or a private network anyway.
                true => format!("wss:// is not supported (no TLS); use ws://: {url}"),
                false => format!("relay URL must start with ws://: {url}"),
            })?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let addr = if authority.contains(':') {
            authority.to_string()
        } else {
            format!("{authority}:80")
        };

        let mut sock = TcpStream::connect(&addr).map_err(|e| format!("connect {addr}: {e}"))?;
        // Frame latency matters more than packing: a 60-byte ARP reply must not
        // wait on Nagle for a second frame that may never come.
        let _ = sock.set_nodelay(true);

        let key = base64(&random_bytes());
        let req = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {authority}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {key}\r\n\
             Sec-WebSocket-Version: 13\r\n\
             \r\n"
        );
        sock.write_all(req.as_bytes())
            .map_err(|e| format!("send upgrade: {e}"))?;

        // Read headers up to the blank line. Anything past it is already
        // WebSocket data and must be kept.
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while !ends_with_crlf_crlf(&buf) {
            match sock.read(&mut byte) {
                Ok(0) => return Err("relay closed during handshake".into()),
                Ok(_) => buf.push(byte[0]),
                Err(e) => return Err(format!("read upgrade response: {e}")),
            }
            if buf.len() > 8192 {
                return Err("handshake response too large".into());
            }
        }
        let head = String::from_utf8_lossy(&buf);
        let status = head.lines().next().unwrap_or_default();
        if !status.contains("101") {
            return Err(format!("relay refused upgrade: {status}"));
        }
        let expect = accept_key(&key);
        let got = head
            .lines()
            .find_map(|l| {
                let (name, value) = l.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("sec-websocket-accept")
                    .then(|| value.trim().to_string())
            })
            .unwrap_or_default();
        if got != expect {
            return Err(format!(
                "relay is not a WebSocket server (accept {got:?}, expected {expect:?})"
            ));
        }

        sock.set_nonblocking(true)
            .map_err(|e| format!("set nonblocking: {e}"))?;
        Ok(Relay {
            sock,
            pending: Vec::new(),
            partial: Vec::new(),
            closed: false,
        })
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Send one Ethernet frame as a masked binary message.
    pub fn send(&mut self, frame: &[u8]) {
        if self.closed {
            return;
        }
        let mut msg = vec![0x82u8]; // FIN | binary
        let mask = random_bytes();
        let len = frame.len();
        if len < 126 {
            msg.push(0x80 | len as u8);
        } else if len <= u16::MAX as usize {
            msg.push(0x80 | 126);
            msg.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            msg.push(0x80 | 127);
            msg.extend_from_slice(&(len as u64).to_be_bytes());
        }
        // Client-to-server payloads are always masked (RFC 6455 §5.3).
        msg.extend_from_slice(&mask[..4]);
        msg.extend(frame.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        if self.sock.write_all(&msg).is_err() {
            self.closed = true;
        }
    }

    /// Every complete Ethernet frame that has arrived since the last call.
    /// Never blocks.
    pub fn recv(&mut self) -> Vec<Vec<u8>> {
        let mut buf = [0u8; 16384];
        loop {
            match self.sock.read(&mut buf) {
                Ok(0) => {
                    self.closed = true;
                    break;
                }
                Ok(n) => self.pending.extend_from_slice(&buf[..n]),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.closed = true;
                    break;
                }
            }
        }
        self.drain_frames()
    }

    /// Parse as many whole WebSocket frames out of `pending` as possible.
    fn drain_frames(&mut self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let b = &self.pending;
            if b.len() < 2 {
                break;
            }
            let fin = b[0] & 0x80 != 0;
            let opcode = b[0] & 0x0f;
            let masked = b[1] & 0x80 != 0;
            let len7 = (b[1] & 0x7f) as usize;
            let (payload_len, mut off) = match len7 {
                126 if b.len() >= 4 => (u16::from_be_bytes([b[2], b[3]]) as usize, 4),
                127 if b.len() >= 10 => (
                    u64::from_be_bytes(b[2..10].try_into().unwrap()) as usize,
                    10,
                ),
                126 | 127 => break, // length not fully arrived
                n => (n, 2),
            };
            // A conforming server never masks, but honour the bit rather than
            // mis-parse the stream if one does.
            let mask = if masked {
                if b.len() < off + 4 {
                    break;
                }
                let m = [b[off], b[off + 1], b[off + 2], b[off + 3]];
                off += 4;
                Some(m)
            } else {
                None
            };
            if b.len() < off + payload_len {
                break;
            }
            let mut payload = b[off..off + payload_len].to_vec();
            if let Some(m) = mask {
                for (i, byte) in payload.iter_mut().enumerate() {
                    *byte ^= m[i % 4];
                }
            }
            self.pending.drain(..off + payload_len);

            match opcode {
                // Binary, or a continuation of one.
                0x2 | 0x0 => {
                    if fin && self.partial.is_empty() {
                        out.push(payload);
                    } else {
                        self.partial.extend_from_slice(&payload);
                        if fin {
                            out.push(core::mem::take(&mut self.partial));
                        }
                    }
                }
                0x8 => {
                    self.closed = true;
                    break;
                }
                // Ping: the relay expects a pong with the same payload, or it
                // will eventually drop us as dead.
                0x9 => self.send_control(0x8a, &payload),
                _ => {} // text, pong: not our protocol
            }
        }
        out
    }

    fn send_control(&mut self, opcode: u8, payload: &[u8]) {
        let mask = random_bytes();
        let mut msg = vec![opcode, 0x80 | payload.len().min(125) as u8];
        msg.extend_from_slice(&mask[..4]);
        msg.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        if self.sock.write_all(&msg).is_err() {
            self.closed = true;
        }
    }
}

fn ends_with_crlf_crlf(b: &[u8]) -> bool {
    b.len() >= 4 && &b[b.len() - 4..] == b"\r\n\r\n"
}

/// 16 bytes for a handshake key or a frame mask. Both only need to be
/// unpredictable to a cache or proxy, not cryptographically strong.
fn random_bytes() -> [u8; 16] {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut b).is_ok() {
            return b;
        }
    }
    // No /dev/urandom: any varying value will do.
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    for (i, byte) in b.iter_mut().enumerate() {
        *byte = (t >> (i % 8 * 8)) as u8 ^ (i as u8).wrapping_mul(31);
    }
    b
}

fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The `Sec-WebSocket-Accept` value a server must return for `key`:
/// base64(SHA-1(key + magic GUID)).
pub fn accept_key(key: &str) -> String {
    let mut input = key.as_bytes().to_vec();
    input.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64(&sha1(&input))
}

/// SHA-1, needed only for the handshake above.
fn sha1(msg: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut data = msg.to_vec();
    let bit_len = (msg.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());

    for block in data.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_and_base64_match_the_rfc_example() {
        // RFC 6455 §1.3: this key must produce this accept value.
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
