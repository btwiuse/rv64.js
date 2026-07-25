//! Native egress for the HTTP proxy: performs the guest's requests with real
//! sockets.
//!
//! **Plaintext HTTP only.** TLS would mean a TLS stack plus a trust store, and
//! natively that buys little — a native guest can be given a relay or a tap
//! device for real connectivity. The value here is development parity: the same
//! netstack and proxy that the browser build uses, driven by `cargo test` and
//! `rv64-boot`, without a browser in the loop. The browser gets HTTPS for free,
//! because `fetch()` does the TLS.
//!
//! Requests run on their own threads and complete through a channel, matching
//! [`Egress`]'s submit-then-poll contract rather than stalling the emulator for
//! the duration of a request.

use crate::httpproxy::{Egress, ReqId, Request, Response};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

/// How long a single upstream request may take before we give up on it.
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct NativeEgress {
    tx: Sender<(ReqId, Result<Response, String>)>,
    rx: Receiver<(ReqId, Result<Response, String>)>,
}

impl Default for NativeEgress {
    fn default() -> NativeEgress {
        NativeEgress::new()
    }
}

impl NativeEgress {
    pub fn new() -> NativeEgress {
        let (tx, rx) = channel();
        NativeEgress { tx, rx }
    }
}

impl Egress for NativeEgress {
    fn submit(&mut self, id: ReqId, req: Request) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((id, perform(&req)));
        });
    }

    fn poll(&mut self) -> Vec<(ReqId, Result<Response, String>)> {
        let mut out = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            out.push(done);
        }
        out
    }
}

/// Perform one request over a fresh connection.
fn perform(req: &Request) -> Result<Response, String> {
    let (host, port, path) = split_url(&req.url)?;
    let mut sock = TcpStream::connect((host.as_str(), port))
        .map_err(|e| format!("connect {host}:{port}: {e}"))?;
    sock.set_read_timeout(Some(TIMEOUT)).ok();
    sock.set_write_timeout(Some(TIMEOUT)).ok();

    let mut head = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", req.method, path, host);
    for (name, value) in &req.headers {
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
            continue; // set from the connection and the body below
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    // Close-delimited: the response ends at EOF unless it says otherwise, which
    // saves keeping a connection pool for a development backend.
    head.push_str(&format!("Content-Length: {}\r\n", req.body.len()));
    head.push_str("Connection: close\r\n\r\n");
    sock.write_all(head.as_bytes())
        .map_err(|e| format!("send request: {e}"))?;
    sock.write_all(&req.body)
        .map_err(|e| format!("send body: {e}"))?;

    let mut raw = Vec::new();
    sock.read_to_end(&mut raw)
        .map_err(|e| format!("read response: {e}"))?;
    parse_response(&raw)
}

/// `http://host[:port]/path` -> (host, port, path).
fn split_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("native egress speaks http:// only, got {url}"))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse().map_err(|_| format!("bad port in {url}"))?,
        ),
        None => (authority.to_string(), 80u16),
    };
    if host.is_empty() {
        return Err(format!("no host in {url}"));
    }
    Ok((host, port, path.to_string()))
}

fn parse_response(raw: &[u8]) -> Result<Response, String> {
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("truncated response: no end of headers")?
        + 4;
    let head = String::from_utf8_lossy(&raw[..head_end - 4]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("bad status line: {status_line}"))?;

    let mut headers = Vec::new();
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        match name.to_ascii_lowercase().as_str() {
            "content-length" => content_length = value.parse().ok(),
            "transfer-encoding" if value.to_ascii_lowercase().contains("chunked") => chunked = true,
            _ => {}
        }
        headers.push((name.to_string(), value.to_string()));
    }

    let rest = &raw[head_end..];
    let body = if chunked {
        dechunk(rest)?
    } else {
        // With `Connection: close` the body runs to EOF; honour an explicit
        // length when one is given, since a server may send both.
        match content_length {
            Some(n) => rest[..n.min(rest.len())].to_vec(),
            None => rest.to_vec(),
        }
    };
    Ok(Response {
        status,
        headers,
        body,
    })
}

fn dechunk(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("truncated chunk header")?;
        let size_text = String::from_utf8_lossy(&data[..line_end]);
        // Chunk extensions follow a ';' and are not ours to interpret.
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| format!("bad chunk size {size_text:?}"))?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if data.len() < size {
            return Err("truncated chunk body".into());
        }
        out.extend_from_slice(&data[..size]);
        data = data.get(size + 2..).unwrap_or(&[]); // skip the trailing CRLF
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::net::TcpListener;

    #[test]
    fn splits_urls() {
        assert_eq!(
            split_url("http://example.test/a/b?c=1").unwrap(),
            ("example.test".into(), 80, "/a/b?c=1".into())
        );
        assert_eq!(
            split_url("http://127.0.0.1:8080").unwrap(),
            ("127.0.0.1".into(), 8080, "/".into())
        );
        // https is the browser's job; failing loudly beats a mystery timeout.
        assert!(split_url("https://example.test/").is_err());
    }

    #[test]
    fn parses_a_content_length_response() {
        let raw = b"HTTP/1.1 201 Created\r\nContent-Length: 5\r\nX-A: b\r\n\r\nhello";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.status, 201);
        assert_eq!(r.body, b"hello");
        assert!(r.headers.contains(&("X-A".into(), "b".into())));
    }

    #[test]
    fn parses_a_chunked_response() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                    5\r\nhello\r\n7\r\n, world\r\n0\r\n\r\n";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.body, b"hello, world");
    }

    #[test]
    fn reads_a_close_delimited_response() {
        // No length and no chunking: the body is everything up to EOF.
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nbody to eof";
        assert_eq!(parse_response(raw).unwrap().body, b"body to eof");
    }

    #[test]
    fn performs_a_real_request_against_a_local_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            // Read the request head so the client's write completes.
            let mut reader = std::io::BufReader::new(sock.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            assert!(request.starts_with("GET /probe HTTP/1.1\r\n"), "got: {request}");
            assert!(
                request.to_lowercase().contains("x-from-guest: yes"),
                "guest headers must be forwarded: {request}"
            );
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
        });

        let mut egress = NativeEgress::new();
        egress.submit(
            7,
            Request {
                method: "GET".into(),
                url: format!("http://127.0.0.1:{port}/probe"),
                headers: vec![("X-From-Guest".into(), "yes".into())],
                body: Vec::new(),
            },
        );
        // submit is asynchronous, so poll until the thread reports back.
        for _ in 0..2000 {
            let done = egress.poll();
            if let Some((id, result)) = done.into_iter().next() {
                assert_eq!(id, 7);
                let response = result.expect("request should succeed");
                assert_eq!(response.status, 200);
                assert_eq!(response.body, b"hi");
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("request never completed");
    }

    #[test]
    fn an_unreachable_host_is_an_error_not_a_hang() {
        let mut egress = NativeEgress::new();
        egress.submit(
            1,
            Request {
                method: "GET".into(),
                // Port 1 on loopback: refused immediately.
                url: "http://127.0.0.1:1/".into(),
                headers: vec![],
                body: vec![],
            },
        );
        for _ in 0..5000 {
            if let Some((_, result)) = egress.poll().into_iter().next() {
                assert!(result.is_err(), "expected a connect error");
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("no result");
    }
}
