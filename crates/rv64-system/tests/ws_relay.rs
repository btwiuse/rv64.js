//! The WebSocket relay client, against a WebSocket server implemented here.
//!
//! Exercises what the emulator actually depends on: a real handshake, and
//! masked client frames / unmasked server frames surviving a round trip at both
//! the 7-bit and 16-bit length encodings (an Ethernet frame crosses that
//! boundary at 126 bytes, so both paths run in normal use).

use rv64_system::ws;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

/// A one-connection WebSocket server that echoes binary frames back.
///
/// Returns the URL to connect to; the server runs until the client closes.
fn spawn_echo_server(extra: ServerBehaviour) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let req = read_headers(&mut sock);
        match extra {
            ServerBehaviour::NotWebSocket => {
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                return;
            }
            ServerBehaviour::WrongAccept => {
                let _ = sock.write_all(
                    b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
                      Connection: Upgrade\r\nSec-WebSocket-Accept: wrong\r\n\r\n",
                );
                return;
            }
            ServerBehaviour::Echo => {}
        }
        // Compute the accept value the client will demand.
        let key = req
            .lines()
            .find_map(|l| {
                let (name, value) = l.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("sec-websocket-key")
                    .then(|| value.trim().to_string())
            })
            .expect("client sent no Sec-WebSocket-Key");
        let accept = ws::accept_key(&key);
        let resp = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        sock.write_all(resp.as_bytes()).expect("send 101");

        // Echo every binary frame back, unmasked (server-to-client rule).
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match sock.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
            while let Some((opcode, payload, consumed)) = parse_client_frame(&buf) {
                buf.drain(..consumed);
                match opcode {
                    0x2 => {
                        let _ = sock.write_all(&server_frame(0x82, &payload));
                    }
                    // A ping from us must come back as a pong.
                    0x9 => {}
                    0x8 => return,
                    _ => {}
                }
            }
        }
    });
    format!("ws://127.0.0.1:{port}/relay")
}

enum ServerBehaviour {
    Echo,
    NotWebSocket,
    WrongAccept,
}

fn read_headers(sock: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !(buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n") {
        match sock.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => buf.push(byte[0]),
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Parse one client frame: returns (opcode, unmasked payload, bytes consumed).
fn parse_client_frame(b: &[u8]) -> Option<(u8, Vec<u8>, usize)> {
    if b.len() < 2 {
        return None;
    }
    let opcode = b[0] & 0x0f;
    assert!(b[1] & 0x80 != 0, "client frames must be masked");
    let len7 = (b[1] & 0x7f) as usize;
    let (len, mut off) = match len7 {
        126 => (u16::from_be_bytes([*b.get(2)?, *b.get(3)?]) as usize, 4),
        127 => (
            u64::from_be_bytes(b.get(2..10)?.try_into().unwrap()) as usize,
            10,
        ),
        n => (n, 2),
    };
    let mask: [u8; 4] = b.get(off..off + 4)?.try_into().unwrap();
    off += 4;
    let masked = b.get(off..off + len)?;
    let payload = masked
        .iter()
        .enumerate()
        .map(|(i, byte)| byte ^ mask[i % 4])
        .collect();
    Some((opcode, payload, off + len))
}

fn server_frame(first: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![first];
    if payload.len() < 126 {
        out.push(payload.len() as u8);
    } else {
        out.push(126);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

/// Poll for frames until `want` of them arrive, or give up.
fn collect(relay: &mut ws::Relay, want: usize) -> Vec<Vec<u8>> {
    let mut got = Vec::new();
    for _ in 0..2000 {
        got.extend(relay.recv());
        if got.len() >= want {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    got
}

#[test]
fn frames_survive_a_round_trip_at_both_length_encodings() {
    let url = spawn_echo_server(ServerBehaviour::Echo);
    let mut relay = ws::Relay::connect(&url).expect("connect");

    // 7-bit length: a minimum-size Ethernet frame.
    let small: Vec<u8> = (0..60u8).collect();
    // 16-bit length: a full MTU frame, the path a 126-byte threshold makes
    // unavoidable in real traffic.
    let large: Vec<u8> = (0..1514).map(|i| (i % 251) as u8).collect();
    relay.send(&small);
    relay.send(&large);

    let got = collect(&mut relay, 2);
    assert_eq!(got.len(), 2, "expected both frames back, got {}", got.len());
    assert_eq!(got[0], small);
    assert_eq!(got[1], large);
    assert!(!relay.is_closed());
}

#[test]
fn many_frames_in_one_read_are_all_parsed() {
    // The relay coalesces: several frames commonly arrive in a single TCP
    // segment, and the parser has to split them without losing any.
    let url = spawn_echo_server(ServerBehaviour::Echo);
    let mut relay = ws::Relay::connect(&url).expect("connect");
    let frames: Vec<Vec<u8>> = (0..32u8).map(|i| vec![i; 64 + i as usize]).collect();
    for f in &frames {
        relay.send(f);
    }
    let got = collect(&mut relay, frames.len());
    assert_eq!(got, frames);
}

#[test]
fn a_plain_http_server_is_refused_not_misread() {
    let url = spawn_echo_server(ServerBehaviour::NotWebSocket);
    let err = ws::Relay::connect(&url).expect_err("must not accept a 200 OK");
    assert!(err.contains("refused upgrade"), "unhelpful error: {err}");
}

#[test]
fn a_bad_accept_header_is_refused() {
    // Checking the accept value is what stops us reading garbage from whatever
    // happened to answer the port.
    let url = spawn_echo_server(ServerBehaviour::WrongAccept);
    let err = ws::Relay::connect(&url).expect_err("must not accept a wrong hash");
    assert!(
        err.contains("not a WebSocket server"),
        "unhelpful error: {err}"
    );
}

#[test]
fn unsupported_urls_say_why() {
    let err = ws::Relay::connect("wss://example.invalid/").expect_err("no TLS");
    assert!(err.contains("wss:// is not supported"), "got: {err}");
    let err = ws::Relay::connect("http://example.invalid/").expect_err("wrong scheme");
    assert!(err.contains("must start with ws://"), "got: {err}");
}
