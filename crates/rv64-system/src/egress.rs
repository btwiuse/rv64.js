//! Native egress for the HTTP proxy: performs the guest's requests with real
//! sockets.
//!
//! HTTP uses a raw TCP stream. HTTPS uses rustls with the host's CA bundle and
//! validates both the upstream certificate chain and requested hostname. This
//! mirrors the browser build's `fetch()` egress closely enough for native,
//! full-Debian end-to-end tests of the guest-facing MITM proxy.
//!
//! Requests run on their own threads and complete through a channel, matching
//! [`Egress`]'s submit-then-poll contract rather than stalling the emulator for
//! the duration of a request.
//!
//! This backend reads a whole response before emitting it as `Head`/`Body`/`End`
//! rather than streaming incrementally. Incremental de-chunking would be the
//! only added value, and streaming matters where responses are large or
//! long-lived — which is the browser, where `fetch` streams natively.

use crate::httpproxy::{Completion, Egress, ReqId, Request, Response};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::pem::PemObject;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// How long a single upstream request may take before we give up on it.
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct NativeEgress {
    tx: Sender<Completion>,
    rx: Receiver<Completion>,
    tls: Result<Arc<ClientConfig>, String>,
}

impl Default for NativeEgress {
    fn default() -> NativeEgress {
        NativeEgress::new()
    }
}

impl NativeEgress {
    pub fn new() -> NativeEgress {
        let (tx, rx) = channel();
        NativeEgress {
            tx,
            rx,
            tls: system_tls_config(),
        }
    }
}

impl Egress for NativeEgress {
    fn submit(&mut self, id: ReqId, req: Request) {
        let tx = self.tx.clone();
        let tls = self.tls.clone();
        std::thread::spawn(move || match perform(&req, &tls) {
            Ok(response) => {
                let _ = tx.send(Completion::Head {
                    id,
                    status: response.status,
                    headers: response.headers,
                });
                if !response.body.is_empty() {
                    let _ = tx.send(Completion::Body {
                        id,
                        bytes: response.body,
                    });
                }
                let _ = tx.send(Completion::End { id });
            }
            Err(error) => {
                let _ = tx.send(Completion::Failed { id, error });
            }
        });
    }

    fn poll(&mut self) -> Vec<Completion> {
        let mut out = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            out.push(done);
        }
        out
    }
}

#[derive(Debug, PartialEq)]
struct Target {
    tls: bool,
    host: String,
    authority: String,
    port: u16,
    path: String,
}

// Boxing the TLS stream adds an allocation on every proxied TLS connection;
// the enum is short-lived and intentionally keeps both streams inline.
#[allow(clippy::large_enum_variant)]
enum Upstream {
    Plain(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),
}

impl Read for Upstream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Upstream::Plain(stream) => stream.read(buf),
            Upstream::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Upstream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Upstream::Plain(stream) => stream.write(buf),
            Upstream::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Upstream::Plain(stream) => stream.flush(),
            Upstream::Tls(stream) => stream.flush(),
        }
    }
}

/// Perform one request over a fresh connection.
fn perform(
    req: &Request,
    tls_config: &Result<Arc<ClientConfig>, String>,
) -> Result<Response, String> {
    let target = split_url(&req.url)?;
    let sock = TcpStream::connect((target.host.as_str(), target.port))
        .map_err(|e| format!("connect {}:{}: {e}", target.host, target.port))?;
    sock.set_read_timeout(Some(TIMEOUT)).ok();
    sock.set_write_timeout(Some(TIMEOUT)).ok();

    let mut stream = if target.tls {
        let config = tls_config
            .as_ref()
            .map_err(|error| format!("TLS trust store: {error}"))?;
        let server_name = ServerName::try_from(target.host.clone())
            .map_err(|_| format!("invalid TLS server name {}", target.host))?;
        let client = ClientConnection::new(Arc::clone(config), server_name)
            .map_err(|e| format!("TLS client for {}: {e}", target.host))?;
        Upstream::Tls(StreamOwned::new(client, sock))
    } else {
        Upstream::Plain(sock)
    };

    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\n",
        req.method, target.path, target.authority
    );
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
    stream
        .write_all(head.as_bytes())
        .map_err(|e| format!("send request: {e}"))?;
    stream
        .write_all(&req.body)
        .map_err(|e| format!("send body: {e}"))?;
    stream.flush().map_err(|e| format!("flush request: {e}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("read response: {e}"))?;
    parse_response(&raw)
}

/// Split an absolute HTTP(S) URL without pulling a general URL parser into the
/// emulator. Proxy parsing has already rejected fragments and malformed request
/// targets; this layer still validates the authority before opening a socket.
fn split_url(url: &str) -> Result<Target, String> {
    let (tls, rest, default_port) = if let Some(rest) = url.strip_prefix("http://") {
        (false, rest, 80)
    } else if let Some(rest) = url.strip_prefix("https://") {
        (true, rest, 443)
    } else {
        return Err(format!(
            "native egress requires http:// or https://, got {url}"
        ));
    };

    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'?' => (&rest[..i], format!("/{}", &rest[i..])),
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err(format!("invalid authority in {url}"));
    }

    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let close = bracketed
            .find(']')
            .ok_or_else(|| format!("bad IPv6 authority in {url}"))?;
        let host = &bracketed[..close];
        let suffix = &bracketed[close + 1..];
        let port = match suffix.strip_prefix(':') {
            Some(text) => text.parse().map_err(|_| format!("bad port in {url}"))?,
            None if suffix.is_empty() => default_port,
            None => return Err(format!("bad IPv6 authority in {url}")),
        };
        (host.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, text)) if !host.contains(':') => (
                host.to_string(),
                text.parse().map_err(|_| format!("bad port in {url}"))?,
            ),
            _ => (authority.to_string(), default_port),
        }
    };
    if host.is_empty() {
        return Err(format!("no host in {url}"));
    }
    Ok(Target {
        tls,
        host,
        authority: authority.to_string(),
        port,
        path,
    })
}

fn client_config(roots: RootCertStore) -> Result<Arc<ClientConfig>, String> {
    if roots.is_empty() {
        return Err("no usable root certificates".into());
    }
    let provider = Arc::new(oxitls_rustcrypto_provider::provider());
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS versions: {e}"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    // Empty ALPN makes HTTPS servers select HTTP/1.1. This egress deliberately
    // speaks only HTTP/1.1 and must never advertise h2.
    config.alpn_protocols.clear();
    Ok(Arc::new(config))
}

fn system_tls_config() -> Result<Arc<ClientConfig>, String> {
    let explicit = std::env::var_os("SSL_CERT_FILE")
        .or_else(|| std::env::var_os("NIX_SSL_CERT_FILE"))
        .map(std::path::PathBuf::from);
    let path = match explicit {
        Some(path) => path,
        None => [
            "/etc/ssl/certs/ca-certificates.crt",
            "/etc/pki/tls/certs/ca-bundle.crt",
            "/etc/ssl/ca-bundle.pem",
        ]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file())
        .ok_or("could not find a system CA bundle")?,
    };
    let pem = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let certs = CertificateDer::pem_slice_iter(&pem).filter_map(Result::ok);
    let mut roots = RootCertStore::empty();
    let (valid, _) = roots.add_parsable_certificates(certs);
    if valid == 0 {
        return Err(format!(
            "{} contained no usable certificates",
            path.display()
        ));
    }
    client_config(roots)
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
            Target {
                tls: false,
                host: "example.test".into(),
                authority: "example.test".into(),
                port: 80,
                path: "/a/b?c=1".into(),
            }
        );
        assert_eq!(
            split_url("https://127.0.0.1:8443?probe=1").unwrap(),
            Target {
                tls: true,
                host: "127.0.0.1".into(),
                authority: "127.0.0.1:8443".into(),
                port: 8443,
                path: "/?probe=1".into(),
            }
        );
        assert!(split_url("ftp://example.test/").is_err());
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
            assert!(
                request.starts_with("GET /probe HTTP/1.1\r\n"),
                "got: {request}"
            );
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
        let mut got = Vec::new();
        for _ in 0..2000 {
            got.extend(egress.poll());
            if got.iter().any(|c| matches!(c, Completion::End { .. })) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            matches!(
                got.first(),
                Some(Completion::Head {
                    id: 7,
                    status: 200,
                    ..
                })
            ),
            "expected a 200 head first, got {got:?}"
        );
        assert!(
            got.iter()
                .any(|c| matches!(c, Completion::Body { bytes, .. } if bytes == b"hi")),
            "expected the body, got {got:?}"
        );
        assert!(
            matches!(got.last(), Some(Completion::End { id: 7 })),
            "got {got:?}"
        );
    }

    #[test]
    fn performs_https_with_ca_and_hostname_verification() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut authority = crate::tlsproxy::TlsAuthority::new().unwrap();
        let ca = authority.ca_der().to_vec();
        let server = authority.server("localhost").unwrap();
        std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut stream = StreamOwned::new(server, sock);
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let n = stream.read(&mut chunk).unwrap();
                assert_ne!(n, 0, "client closed before its request head");
                request.extend_from_slice(&chunk[..n]);
            }
            assert!(
                request.starts_with(b"GET /secure HTTP/1.1\r\n"),
                "got: {}",
                String::from_utf8_lossy(&request)
            );
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\ntls-ok")
                .unwrap();
            stream.conn.send_close_notify();
            stream.flush().unwrap();
        });

        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from(ca)).unwrap();
        let response = perform(
            &Request {
                method: "GET".into(),
                url: format!("https://localhost:{port}/secure"),
                headers: vec![],
                body: vec![],
            },
            &Ok(client_config(roots).unwrap()),
        )
        .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"tls-ok");
    }

    #[test]
    fn rejects_an_https_certificate_for_another_hostname() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut authority = crate::tlsproxy::TlsAuthority::new().unwrap();
        let ca = authority.ca_der().to_vec();
        let server = authority.server("localhost").unwrap();
        std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut stream = StreamOwned::new(server, sock);
            let _ = stream.read(&mut [0u8; 1]);
        });

        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from(ca)).unwrap();
        let error = perform(
            &Request {
                method: "GET".into(),
                // The issuing CA is trusted, but the leaf contains localhost,
                // not this IP address. Chain trust alone must not be enough.
                url: format!("https://127.0.0.1:{port}/"),
                headers: vec![],
                body: vec![],
            },
            &Ok(client_config(roots).unwrap()),
        )
        .expect_err("hostname mismatch must fail");
        assert!(
            error.to_ascii_lowercase().contains("certificate"),
            "unexpected error: {error}"
        );
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
            if let Some(c) = egress.poll().into_iter().next() {
                assert!(
                    matches!(c, Completion::Failed { .. }),
                    "expected a connect failure, got {c:?}"
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("no result");
    }
}
