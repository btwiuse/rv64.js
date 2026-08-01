//! HTTP proxy for the guest, running on the host side of virtio-net.
//!
//! The guest points `http_proxy` at us and sends ordinary proxy requests over
//! TCP ([`crate::netstack`] terminates those). We parse each request and hand it
//! to an [`Egress`] implementation, which decides how it actually leaves: a
//! `fetch()` in the browser, a socket natively.
//!
//! Why a proxy rather than a NAT: because the guest hands us a *hostname and a
//! complete request* instead of packets to an IP. That deletes DNS, connection
//! tracking, and UDP from the problem, and it makes the browser's only egress
//! primitive — `fetch`, which is request/response shaped — a direct fit. In a
//! browser this is the only design that reaches the network with no external
//! infrastructure at all.
//!
//! ## Scope
//!
//! - Absolute-URI requests (`GET http://host/path`), which is what a client
//!   sends to a proxy. Origin-form plus a `Host` header is also accepted, since
//!   being lenient here costs nothing.
//! - Request bodies with `Content-Length` or `Transfer-Encoding: chunked`.
//!   Chunked bodies are decoded before crossing the request-shaped [`Egress`]
//!   boundary; request trailers and stacked transfer codings are rejected
//!   explicitly rather than silently discarded.
//! - `CONNECT` is terminated locally with rustls and certificates minted by an
//!   ephemeral proxy CA. The CA certificate is available inside the guest at
//!   `http://rv64-proxy.invalid/ca.der`; it must be trusted by the guest client.
//! - One request per connection: every response says `Connection: close`. A
//!   proxy is allowed to do this, it keeps request framing trivial, and it makes
//!   the response body EOF-delimited — which is exactly what lets responses
//!   stream without chunked encoding.

use crate::netstack::{ConnId, Event, NetStack};
use crate::tlsproxy::TlsAuthority;
use std::collections::HashMap;
use std::io::{Cursor as IoCursor, Read, Write};

/// A request the proxy wants performed on the guest's behalf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// Absolute URL, with the scheme [`Egress`] should use.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Identifies an in-flight request.
pub type ReqId = u64;

// ---- FFI wire form --------------------------------------------------------
//
// A request has to cross into JavaScript (where `fetch` lives) and a response
// has to come back. Both are encoded as explicit length-prefixed fields rather
// than JSON: no escaping questions, no dependency, and a DataView reads it in a
// dozen lines. All lengths are little-endian u32.

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cursor<'a> {
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.b.get(self.p..self.p + 4)?.try_into().ok()?);
        self.p += 4;
        Some(v)
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn string(&mut self) -> Option<String> {
        Some(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
}

impl Request {
    /// `method | url | nheaders | (name, value)* | body`
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_bytes(&mut out, self.method.as_bytes());
        put_bytes(&mut out, self.url.as_bytes());
        out.extend_from_slice(&(self.headers.len() as u32).to_le_bytes());
        for (name, value) in &self.headers {
            put_bytes(&mut out, name.as_bytes());
            put_bytes(&mut out, value.as_bytes());
        }
        put_bytes(&mut out, &self.body);
        out
    }

    pub fn decode(b: &[u8]) -> Option<Request> {
        let mut c = Cursor { b, p: 0 };
        let method = c.string()?;
        let url = c.string()?;
        let n = c.u32()? as usize;
        let mut headers = Vec::with_capacity(n);
        for _ in 0..n {
            headers.push((c.string()?, c.string()?));
        }
        Some(Request {
            method,
            url,
            headers,
            body: c.bytes()?.to_vec(),
        })
    }
}

/// Encode a response head for the FFI: `status | nheaders | (name, value)*`.
/// Bodies cross separately as raw chunks, so they need no framing.
pub fn encode_head(status: u16, headers: &[(String, String)]) -> Vec<u8> {
    let mut out = (status as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&(headers.len() as u32).to_le_bytes());
    for (name, value) in headers {
        put_bytes(&mut out, name.as_bytes());
        put_bytes(&mut out, value.as_bytes());
    }
    out
}

pub fn decode_head(b: &[u8]) -> Option<(u16, Vec<(String, String)>)> {
    let mut c = Cursor { b, p: 0 };
    let status = c.u32()? as u16;
    let n = c.u32()? as usize;
    let mut headers = Vec::with_capacity(n);
    for _ in 0..n {
        headers.push((c.string()?, c.string()?));
    }
    Some((status, headers))
}

/// What egress reports back about an in-flight request.
///
/// Responses are **streamed**, not delivered whole: an SSE or chunked API
/// response must reach the guest as it arrives rather than after the upstream
/// finishes, and a large body must never be buffered in full on the way past.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    /// Status and headers. Exactly one per request, before any `Body`.
    Head {
        id: ReqId,
        status: u16,
        headers: Vec<(String, String)>,
    },
    /// A run of body bytes.
    Body { id: ReqId, bytes: Vec<u8> },
    /// The response is complete.
    End { id: ReqId },
    /// The request could not be performed, or failed part way through.
    Failed { id: ReqId, error: String },
}

impl Completion {
    pub fn id(&self) -> ReqId {
        match self {
            Completion::Head { id, .. }
            | Completion::Body { id, .. }
            | Completion::End { id }
            | Completion::Failed { id, .. } => *id,
        }
    }
}

/// How requests actually leave the host.
///
/// Deliberately submit-then-poll rather than blocking: `fetch()` is async and
/// the guest's TCP connection has to stay open across it, so completion cannot
/// be a return value. Native implementations can complete immediately.
pub trait Egress {
    fn submit(&mut self, id: ReqId, req: Request);
    /// Progress since the last call, in order per request.
    fn poll(&mut self) -> Vec<Completion>;
}

/// Largest request head and body we will buffer from the guest, so a hostile or
/// broken guest cannot grow host memory without bound.
const MAX_HEAD: usize = 64 * 1024;
const MAX_BODY: usize = 32 * 1024 * 1024;
const CA_URL_HTTP: &str = "http://rv64-proxy.invalid/ca.der";
const CA_URL_HTTPS: &str = "https://rv64-proxy.invalid/ca.der";
/// Stable virtio-9p mount tag used to expose the public proxy CA to a guest.
pub const CA_9P_TAG: &str = "rv64-proxy";
/// Path of the DER-encoded public proxy CA inside [`CA_9P_TAG`].
pub const CA_9P_DER_PATH: &str = "/ca.der";

/// Headers that describe *this* hop and must not be forwarded (RFC 7230 §6.1),
/// plus `proxy-connection`, which is the pre-standard spelling clients still
/// send to proxies.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "proxy-connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    // fetch() sets its own, and forwarding the guest's confuses content coding.
    "accept-encoding",
];

#[derive(Default)]
struct ConnBuf {
    /// Bytes received but not yet forming a complete request.
    buf: Vec<u8>,
    /// Set once the request has been submitted, so later bytes are ignored
    /// rather than parsed as a second pipelined request.
    submitted: bool,
    /// Set once the response head has gone out. After that the status is
    /// committed: a mid-stream failure can only truncate, not become a 502.
    head_sent: bool,
    /// Present after a successful CONNECT response. Network bytes flow into
    /// this server session and all response bytes flow back through it.
    tls: Option<rustls::ServerConnection>,
}

pub struct Proxy {
    conns: HashMap<ConnId, ConnBuf>,
    /// In-flight request id -> the connection waiting for it.
    inflight: HashMap<ReqId, ConnId>,
    next_req: ReqId,
    /// Rewrite `http://` to `https://` on egress.
    upgrade_scheme: bool,
    requests: u64,
    /// Generated lazily: ordinary HTTP proxy use should not pay for keys and
    /// certificates, while the CA download and CONNECT must share one identity.
    tls: Option<TlsAuthority>,
}

impl Default for Proxy {
    fn default() -> Proxy {
        Proxy::new()
    }
}

impl Proxy {
    pub fn new() -> Proxy {
        Proxy {
            conns: HashMap::new(),
            inflight: HashMap::new(),
            next_req: 1,
            // On by default because a page served over https cannot fetch
            // http:// at all — the browser blocks it as mixed content — and
            // essentially every host worth reaching is https anyway. This also
            // maps decrypted CONNECT requests back to https:// fetch egress.
            upgrade_scheme: true,
            requests: 0,
            tls: None,
        }
    }

    /// Leave the guest's scheme alone. Only useful when egress genuinely wants
    /// plaintext, e.g. a test server or a page served over http.
    pub fn keep_scheme(mut self) -> Proxy {
        self.upgrade_scheme = false;
        self
    }

    /// Requests submitted since start, for status reporting.
    pub fn request_count(&self) -> u64 {
        self.requests
    }

    /// Build a tiny 9p export containing this proxy's public root certificate.
    ///
    /// Calling this eagerly creates the same authority later used for CONNECT
    /// tunnels. Only the public DER certificate enters the guest; the signing
    /// key remains private inside [`TlsAuthority`].
    pub fn ca_9p_server(&mut self) -> Result<crate::p9::Server, String> {
        let der = self.ca_der()?.to_vec();
        let mut fs = crate::p9fs::MemFs::new();
        fs.add_file(CA_9P_DER_PATH, &der, 0o444);
        Ok(crate::p9::Server::new(CA_9P_TAG, Box::new(fs)))
    }

    /// The public DER certificate for the authority used by CONNECT tunnels.
    pub fn ca_der(&mut self) -> Result<&[u8], String> {
        self.ensure_tls()?;
        Ok(self
            .tls
            .as_ref()
            .expect("TLS authority initialized")
            .ca_der())
    }

    fn ensure_tls(&mut self) -> Result<(), String> {
        if self.tls.is_none() {
            self.tls = Some(TlsAuthority::new()?);
        }
        Ok(())
    }

    /// Move everything forward: consume guest events, submit ready requests,
    /// and write back whatever egress has completed.
    pub fn pump(&mut self, stack: &mut NetStack, egress: &mut dyn Egress) {
        for event in stack.take_events() {
            match event {
                Event::Opened(id) => {
                    self.conns.insert(id, ConnBuf::default());
                }
                Event::Data(id, bytes) => self.on_data(id, bytes, stack, egress),
                Event::Closed(id) => {
                    // Keep any in-flight entry: its response is simply dropped
                    // when it arrives, since the guest is gone.
                    self.conns.remove(&id);
                }
            }
        }
        for completion in egress.poll() {
            // The guest may have gone away mid-response; its entry is dropped
            // and anything still arriving for it is discarded.
            let Some(&conn) = self.inflight.get(&completion.id()) else {
                continue;
            };
            match completion {
                Completion::Head {
                    status, headers, ..
                } => self.write_head(conn, status, &headers, stack),
                Completion::Body { bytes, .. } => self.send_bytes(conn, &bytes, stack),
                Completion::End { id } => {
                    self.inflight.remove(&id);
                    if let Some(state) = self.conns.get_mut(&conn) {
                        if let Some(tls) = state.tls.as_mut() {
                            tls.send_close_notify();
                        }
                    }
                    self.drain_tls(conn, stack);
                    stack.close(conn);
                    self.conns.remove(&conn);
                }
                Completion::Failed { id, error } => {
                    self.inflight.remove(&id);
                    let head_sent = self.conns.get(&conn).is_some_and(|c| c.head_sent);
                    if head_sent {
                        // The status line is already committed upstream of us,
                        // so all we can do is cut the body short; the guest
                        // sees a truncated response, which is what a real
                        // connection failure looks like.
                        stack.close(conn);
                        self.conns.remove(&conn);
                    } else {
                        self.write_error(conn, 502, "Bad Gateway", &error, stack);
                    }
                }
            }
        }
    }

    fn on_data(
        &mut self,
        id: ConnId,
        bytes: Vec<u8>,
        stack: &mut NetStack,
        egress: &mut dyn Egress,
    ) {
        let Some(state) = self.conns.get(&id) else {
            return;
        };
        if state.tls.is_some() {
            self.on_tls_data(id, &bytes, stack, egress);
            return;
        }
        if state.submitted {
            return; // one request per connection; see module docs
        }

        let too_large = {
            let state = self.conns.get_mut(&id).expect("connection disappeared");
            state.buf.extend_from_slice(&bytes);
            state.buf.len() > MAX_HEAD + MAX_BODY
        };
        if too_large {
            self.write_error(id, 413, "Payload Too Large", "request too large", stack);
            return;
        }

        let connect = {
            let state = self.conns.get(&id).expect("connection disappeared");
            parse_connect(&state.buf)
        };
        match connect {
            Ok(Some((host, consumed))) => {
                self.open_tunnel(id, host, consumed, stack, egress);
                return;
            }
            Ok(None) => {}
            Err(ParseError {
                status,
                reason,
                detail,
            }) => {
                self.write_error(id, status, reason, detail, stack);
                return;
            }
        }
        self.submit_buffered_request(id, stack, egress);
    }

    fn submit_buffered_request(
        &mut self,
        id: ConnId,
        stack: &mut NetStack,
        egress: &mut dyn Egress,
    ) {
        let parsed = {
            let Some(state) = self.conns.get(&id) else {
                return;
            };
            if state.submitted {
                return;
            }
            // An origin-form request decrypted from CONNECT is HTTPS even
            // when native egress preserves ordinary http:// proxy requests.
            // Browser mode additionally upgrades plaintext requests because
            // a secure page cannot fetch mixed-content http:// URLs.
            parse_request(&state.buf, state.tls.is_some() || self.upgrade_scheme)
        };
        match parsed {
            Ok(None) => {} // still arriving
            Ok(Some(req)) => {
                if req.method.eq_ignore_ascii_case("GET")
                    && (req.url == CA_URL_HTTP || req.url == CA_URL_HTTPS)
                {
                    self.serve_ca(id, stack);
                    return;
                }
                if let Some(state) = self.conns.get_mut(&id) {
                    state.submitted = true;
                }
                let req_id = self.next_req;
                self.next_req += 1;
                self.requests += 1;
                self.inflight.insert(req_id, id);
                egress.submit(req_id, req);
            }
            Err(ParseError {
                status,
                reason,
                detail,
            }) => self.write_error(id, status, reason, detail, stack),
        }
    }

    fn open_tunnel(
        &mut self,
        id: ConnId,
        host: String,
        consumed: usize,
        stack: &mut NetStack,
        egress: &mut dyn Egress,
    ) {
        if let Err(error) = self.ensure_tls() {
            self.write_error(id, 500, "Internal Server Error", &error, stack);
            return;
        }
        let tls = match self
            .tls
            .as_mut()
            .expect("TLS authority initialized")
            .server(&host)
        {
            Ok(tls) => tls,
            Err(error) => {
                self.write_error(id, 500, "Internal Server Error", &error, stack);
                return;
            }
        };

        let trailing = {
            let Some(state) = self.conns.get_mut(&id) else {
                return;
            };
            let trailing = state.buf.split_off(consumed);
            state.buf.clear();
            state.tls = Some(tls);
            trailing
        };
        stack.send(id, b"HTTP/1.1 200 Connection Established\r\n\r\n");
        if !trailing.is_empty() {
            self.on_tls_data(id, &trailing, stack, egress);
        }
    }

    fn on_tls_data(
        &mut self,
        id: ConnId,
        bytes: &[u8],
        stack: &mut NetStack,
        egress: &mut dyn Egress,
    ) {
        let result = (|| -> Result<Vec<u8>, String> {
            let state = self
                .conns
                .get_mut(&id)
                .ok_or_else(|| "connection disappeared".to_string())?;
            let tls = state
                .tls
                .as_mut()
                .ok_or_else(|| "TLS tunnel disappeared".to_string())?;
            let mut input = IoCursor::new(bytes);
            tls.read_tls(&mut input)
                .map_err(|e| format!("TLS input: {e}"))?;
            tls.process_new_packets()
                .map_err(|e| format!("TLS handshake: {e}"))?;

            let mut plaintext = Vec::new();
            let mut chunk = [0u8; 16 * 1024];
            loop {
                match tls.reader().read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => plaintext.extend_from_slice(&chunk[..n]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(format!("TLS plaintext: {error}")),
                }
            }
            Ok(plaintext)
        })();

        // Handshake messages and alerts are produced by process_new_packets.
        self.drain_tls(id, stack);
        let plaintext = match result {
            Ok(plaintext) => plaintext,
            Err(_) => {
                stack.close(id);
                self.conns.remove(&id);
                return;
            }
        };
        if plaintext.is_empty() {
            return;
        }
        let too_large = {
            let Some(state) = self.conns.get_mut(&id) else {
                return;
            };
            state.buf.extend_from_slice(&plaintext);
            state.buf.len() > MAX_HEAD + MAX_BODY
        };
        if too_large {
            self.write_error(id, 413, "Payload Too Large", "request too large", stack);
            return;
        }
        self.submit_buffered_request(id, stack, egress);
    }

    fn serve_ca(&mut self, conn: ConnId, stack: &mut NetStack) {
        if let Err(error) = self.ensure_tls() {
            self.write_error(conn, 500, "Internal Server Error", &error, stack);
            return;
        }
        let der = self
            .tls
            .as_ref()
            .expect("TLS authority initialized")
            .ca_der()
            .to_vec();
        let head = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/pkix-cert\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n",
            der.len()
        );
        if let Some(state) = self.conns.get_mut(&conn) {
            state.submitted = true;
            state.head_sent = true;
        }
        self.send_bytes(conn, head.as_bytes(), stack);
        self.send_bytes(conn, &der, stack);
        stack.close(conn);
        self.conns.remove(&conn);
    }

    fn write_head(
        &mut self,
        conn: ConnId,
        status: u16,
        headers: &[(String, String)],
        stack: &mut NetStack,
    ) {
        let mut out = format!("HTTP/1.1 {} {}\r\n", status, reason_phrase(status)).into_bytes();
        for (name, value) in headers {
            let lower = name.to_ascii_lowercase();
            // `content-length` and `content-encoding` are deliberately dropped.
            // `fetch()` decompresses transparently, so the bytes we forward are
            // already decoded while the upstream length still describes the
            // *compressed* body — forwarding either would truncate the response
            // or make the guest try to gunzip plaintext. The body is instead
            // delimited by the close below, which is always correct.
            if HOP_BY_HOP.contains(&lower.as_str())
                || lower == "content-length"
                || lower == "content-encoding"
            {
                continue;
            }
            out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        out.extend_from_slice(b"Connection: close\r\n\r\n");
        self.send_bytes(conn, &out, stack);
        if let Some(state) = self.conns.get_mut(&conn) {
            state.head_sent = true;
        }
    }

    fn send_bytes(&mut self, conn: ConnId, bytes: &[u8], stack: &mut NetStack) {
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        if let Some(tls) = state.tls.as_mut() {
            if tls.writer().write_all(bytes).is_err() {
                return;
            }
            self.drain_tls(conn, stack);
        } else {
            stack.send(conn, bytes);
        }
    }

    fn drain_tls(&mut self, conn: ConnId, stack: &mut NetStack) {
        let Some(tls) = self
            .conns
            .get_mut(&conn)
            .and_then(|state| state.tls.as_mut())
        else {
            return;
        };
        while tls.wants_write() {
            let mut wire = Vec::new();
            match tls.write_tls(&mut wire) {
                Ok(0) | Err(_) => break,
                Ok(_) => stack.send(conn, &wire),
            }
        }
    }

    fn write_error(
        &mut self,
        conn: ConnId,
        status: u16,
        reason: &str,
        detail: &str,
        stack: &mut NetStack,
    ) {
        let body = format!("{status} {reason}: {detail}\n");
        let head = format!(
            "HTTP/1.1 {status} {reason}\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n",
            body.len()
        );
        self.send_bytes(conn, head.as_bytes(), stack);
        self.send_bytes(conn, body.as_bytes(), stack);
        if let Some(state) = self.conns.get_mut(&conn) {
            if let Some(tls) = state.tls.as_mut() {
                tls.send_close_notify();
            }
        }
        self.drain_tls(conn, stack);
        stack.close(conn);
        self.conns.remove(&conn);
    }
}

// ---- request parsing ------------------------------------------------------

#[derive(Debug)]
struct ParseError {
    status: u16,
    reason: &'static str,
    detail: &'static str,
}

/// Recognize and validate a complete CONNECT head.
///
/// A non-CONNECT request and an incomplete head both return `Ok(None)`: the
/// ordinary HTTP parser below distinguishes those cases. The returned offset
/// lets a client place its first TLS record in the same TCP segment.
fn parse_connect(buf: &[u8]) -> Result<Option<(String, usize)>, ParseError> {
    let Some(head_end) = find_head_end(buf) else {
        if buf.len() > MAX_HEAD {
            return Err(ParseError {
                status: 431,
                reason: "Request Header Fields Too Large",
                detail: "no end of headers",
            });
        }
        return Ok(None);
    };
    let first_end = buf[..head_end]
        .windows(2)
        .position(|bytes| bytes == b"\r\n")
        .unwrap_or(head_end);
    let request_line = String::from_utf8_lossy(&buf[..first_end]);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    if !method.eq_ignore_ascii_case("CONNECT") {
        return Ok(None);
    }
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if target.is_empty()
        || !version.starts_with("HTTP/")
        || parts.next().is_some()
        || connect_host(target).is_none()
    {
        return Err(ParseError {
            status: 400,
            reason: "Bad Request",
            detail: "malformed CONNECT authority",
        });
    }
    Ok(Some((
        connect_host(target)
            .expect("validated CONNECT host")
            .to_string(),
        head_end,
    )))
}

fn connect_host(authority: &str) -> Option<&str> {
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = &rest[..end];
        let port = rest[end + 1..].strip_prefix(':')?;
        return (!host.is_empty() && port.parse::<u16>().is_ok()).then_some(host);
    }
    let (host, port) = authority.rsplit_once(':')?;
    (!host.is_empty() && !host.contains(':') && port.parse::<u16>().is_ok()).then_some(host)
}

/// Parse a complete proxy request from `buf`.
///
/// `Ok(None)` means the request has not fully arrived yet.
fn parse_request(buf: &[u8], upgrade_scheme: bool) -> Result<Option<Request>, ParseError> {
    let Some(head_end) = find_head_end(buf) else {
        if buf.len() > MAX_HEAD {
            return Err(ParseError {
                status: 431,
                reason: "Request Header Fields Too Large",
                detail: "no end of headers",
            });
        }
        return Ok(None);
    };
    let head = String::from_utf8_lossy(&buf[..head_end - 4]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    if method.is_empty() || target.is_empty() {
        return Err(ParseError {
            status: 400,
            reason: "Bad Request",
            detail: "malformed request line",
        });
    }

    let mut headers = Vec::new();
    let mut host = String::new();
    let mut content_length = None;
    let mut transfer_codings = Vec::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "host" => host = value.to_string(),
            "content-length" => {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(ParseError {
                        status: 400,
                        reason: "Bad Request",
                        detail: "invalid Content-Length",
                    });
                }
                let length = value.parse::<usize>().map_err(|_| ParseError {
                    status: 400,
                    reason: "Bad Request",
                    detail: "invalid Content-Length",
                })?;
                if content_length.is_some_and(|previous| previous != length) {
                    return Err(ParseError {
                        status: 400,
                        reason: "Bad Request",
                        detail: "conflicting Content-Length headers",
                    });
                }
                content_length = Some(length);
            }
            "transfer-encoding" => {
                transfer_codings.extend(
                    value
                        .split(',')
                        .map(|coding| coding.trim().to_ascii_lowercase()),
                );
            }
            _ => {}
        }
        // Egress reconstructs framing from Request::body. Never forward a
        // stale guest length after decoding a chunked body.
        if !HOP_BY_HOP.contains(&lower.as_str()) && lower != "content-length" {
            headers.push((name.to_string(), value.to_string()));
        }
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        // Would require terminating TLS here; see module docs.
        return Err(ParseError {
            status: 501,
            reason: "Not Implemented",
            detail: "CONNECT (https) is not supported yet; use an http:// URL",
        });
    }
    if content_length.is_some() && !transfer_codings.is_empty() {
        return Err(ParseError {
            status: 400,
            reason: "Bad Request",
            detail: "both Content-Length and Transfer-Encoding are present",
        });
    }
    if !transfer_codings.is_empty() && transfer_codings.as_slice() != ["chunked"] {
        return Err(ParseError {
            status: 501,
            reason: "Not Implemented",
            detail: "only a single chunked transfer coding is supported",
        });
    }
    if content_length.is_some_and(|length| length > MAX_BODY) {
        return Err(ParseError {
            status: 413,
            reason: "Payload Too Large",
            detail: "request body too large",
        });
    }

    // Wait for the whole body before submitting: egress is request/response
    // shaped and cannot stream a partial body.
    let body = if transfer_codings.is_empty() {
        let content_length = content_length.unwrap_or(0);
        if buf.len() < head_end + content_length {
            return Ok(None);
        }
        buf[head_end..head_end + content_length].to_vec()
    } else {
        let Some(body) = decode_chunked_body(buf, head_end)? else {
            return Ok(None);
        };
        body
    };

    let url = match absolute_url(&target, &host) {
        Some(url) => url,
        None => {
            return Err(ParseError {
                status: 400,
                reason: "Bad Request",
                detail: "request target is not a proxyable URL",
            })
        }
    };
    let url = if upgrade_scheme {
        match url.strip_prefix("http://") {
            Some(rest) => format!("https://{rest}"),
            None => url,
        }
    } else {
        url
    };

    Ok(Some(Request {
        method,
        url,
        headers,
        body,
    }))
}

/// Decode RFC-style chunk framing while the request is still arriving.
///
/// The returned body is bounded independently of the wire buffer. Chunk
/// extensions are harmless framing metadata and are ignored. Trailers are not
/// representable by [`Request`], so a non-empty trailer section is rejected.
fn decode_chunked_body(buf: &[u8], mut pos: usize) -> Result<Option<Vec<u8>>, ParseError> {
    let mut body = Vec::new();
    loop {
        let Some(line_end) = find_crlf(buf, pos) else {
            return Ok(None);
        };
        let size_field = buf[pos..line_end]
            .split(|byte| *byte == b';')
            .next()
            .unwrap_or_default();
        let size_field = std::str::from_utf8(size_field)
            .ok()
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .ok_or(ParseError {
                status: 400,
                reason: "Bad Request",
                detail: "invalid chunk size",
            })?;
        let size = usize::from_str_radix(size_field, 16).map_err(|_| ParseError {
            status: 400,
            reason: "Bad Request",
            detail: "invalid chunk size",
        })?;
        pos = line_end + 2;

        if size == 0 {
            let Some(trailer_end) = find_crlf(buf, pos) else {
                return Ok(None);
            };
            if trailer_end == pos {
                return Ok(Some(body));
            }
            return Err(ParseError {
                status: 501,
                reason: "Not Implemented",
                detail: "chunked request trailers are not supported",
            });
        }

        if body
            .len()
            .checked_add(size)
            .is_none_or(|len| len > MAX_BODY)
        {
            return Err(ParseError {
                status: 413,
                reason: "Payload Too Large",
                detail: "request body too large",
            });
        }
        let Some(data_end) = pos.checked_add(size) else {
            return Err(ParseError {
                status: 413,
                reason: "Payload Too Large",
                detail: "request body too large",
            });
        };
        if buf.len() < data_end + 2 {
            return Ok(None);
        }
        if &buf[data_end..data_end + 2] != b"\r\n" {
            return Err(ParseError {
                status: 400,
                reason: "Bad Request",
                detail: "chunk data is not followed by CRLF",
            });
        }
        body.extend_from_slice(&buf[pos..data_end]);
        pos = data_end + 2;
    }
}

fn find_crlf(buf: &[u8], start: usize) -> Option<usize> {
    buf.get(start..)?
        .windows(2)
        .position(|bytes| bytes == b"\r\n")
        .map(|offset| start + offset)
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Turn a request target into an absolute URL. Proxies get absolute-URI form;
/// origin-form plus `Host` is accepted as a convenience.
fn absolute_url(target: &str, host: &str) -> Option<String> {
    if target.starts_with("http://") || target.starts_with("https://") {
        return Some(target.to_string());
    }
    if target.starts_with('/') && !host.is_empty() {
        return Some(format!("http://{host}{target}"));
    }
    None
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ if status < 200 => "Informational",
        _ if status < 300 => "Success",
        _ if status < 400 => "Redirection",
        _ if status < 500 => "Client Error",
        _ => "Server Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Result<Option<Request>, ParseError> {
        parse_request(raw.as_bytes(), false)
    }

    #[test]
    fn parses_an_absolute_uri_request() {
        let req = parse("GET http://api.example/v1/things?a=1 HTTP/1.1\r\nHost: api.example\r\nUser-Agent: Wget\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, "http://api.example/v1/things?a=1");
        // Host and User-Agent pass through; nothing hop-by-hop does.
        assert!(req.headers.contains(&("User-Agent".into(), "Wget".into())));
        assert!(req.body.is_empty());
    }

    #[test]
    fn strips_hop_by_hop_headers() {
        let req = parse(
            "GET http://h/ HTTP/1.1\r\nHost: h\r\nProxy-Connection: keep-alive\r\n\
             Connection: keep-alive\r\nAccept-Encoding: gzip\r\nAccept: */*\r\n\r\n",
        )
        .unwrap()
        .unwrap();
        let names: Vec<String> = req.headers.iter().map(|(n, _)| n.to_lowercase()).collect();
        for banned in ["proxy-connection", "connection", "accept-encoding"] {
            assert!(
                !names.contains(&banned.to_string()),
                "{banned} must be stripped"
            );
        }
        assert!(
            names.contains(&"accept".to_string()),
            "end-to-end headers stay"
        );
    }

    #[test]
    fn waits_for_the_whole_head_and_body() {
        // Head split mid-way: nothing to submit yet.
        assert!(parse("GET http://h/ HTTP/1.1\r\nHost: h\r\n")
            .unwrap()
            .is_none());
        // Head complete but body outstanding.
        let partial = "POST http://h/x HTTP/1.1\r\nHost: h\r\nContent-Length: 11\r\n\r\nhello";
        assert!(parse(partial).unwrap().is_none());
        let whole = "POST http://h/x HTTP/1.1\r\nHost: h\r\nContent-Length: 11\r\n\r\nhello world";
        let req = parse(whole).unwrap().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.body, b"hello world");
    }

    #[test]
    fn decodes_a_chunked_request_body_incrementally() {
        let raw = "POST http://h/upload HTTP/1.1\r\n\
                   Host: h\r\n\
                   Content-Type: text/plain\r\n\
                   Transfer-Encoding: chunked\r\n\r\n\
                   4;name=value\r\nWiki\r\n\
                   5\r\npedia\r\n\
                   0\r\n\r\n";
        let head_end = find_head_end(raw.as_bytes()).unwrap();
        for cut in head_end..raw.len() {
            assert!(
                parse(&raw[..cut]).unwrap().is_none(),
                "request completed early at byte {cut}"
            );
        }
        let req = parse(raw).unwrap().unwrap();
        assert_eq!(req.body, b"Wikipedia");
        assert!(req
            .headers
            .contains(&("Content-Type".into(), "text/plain".into())));
        assert!(!req.headers.iter().any(|(name, _)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                || name.eq_ignore_ascii_case("content-length")
        }));
    }

    #[test]
    fn rejects_ambiguous_or_malformed_chunk_framing() {
        for raw in [
            "POST http://h/ HTTP/1.1\r\nHost: h\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            "POST http://h/ HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: gzip, chunked\r\n\r\n0\r\n\r\n",
            "POST http://h/ HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\nnot-hex\r\n",
            "POST http://h/ HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx!\r\n",
        ] {
            assert!(parse(raw).is_err(), "must reject: {raw:?}");
        }
        let trailers = "POST http://h/ HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nDigest: value\r\n\r\n";
        let err = parse(trailers).expect_err("trailers are not representable");
        assert_eq!(err.status, 501);

        let declared_too_large = format!(
            "POST http://h/ HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
            MAX_BODY + 1
        );
        let err = parse(&declared_too_large)
            .expect_err("oversized declared chunk must fail before buffering it");
        assert_eq!(err.status, 413);
    }

    #[test]
    fn accepts_origin_form_with_a_host_header() {
        let req = parse("GET /path HTTP/1.1\r\nHost: example.test\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(req.url, "http://example.test/path");
    }

    #[test]
    fn rejects_what_it_cannot_yet_do() {
        let err =
            parse("GET nonsense HTTP/1.1\r\n\r\n").expect_err("unproxyable target must be refused");
        assert_eq!(err.status, 400);
    }

    #[test]
    fn parses_connect_authorities() {
        let request = b"CONNECT api.example:443 HTTP/1.1\r\nHost: api.example:443\r\n\r\n";
        let (host, used) = parse_connect(request).unwrap().unwrap();
        assert_eq!(host, "api.example");
        assert_eq!(used, request.len());

        let (host, _) = parse_connect(b"CONNECT 127.0.0.1:8443 HTTP/1.1\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(host, "127.0.0.1");
        let err = parse_connect(b"CONNECT missing-port HTTP/1.1\r\n\r\n")
            .expect_err("CONNECT requires an authority port");
        assert_eq!(err.status, 400);
    }

    #[test]
    fn upgrades_the_scheme_for_egress() {
        // A page served over https cannot fetch http://, so egress uses https.
        let req = parse_request(
            b"GET http://api.example/v1 HTTP/1.1\r\nHost: api.example\r\n\r\n",
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(req.url, "https://api.example/v1");
        // An https target is left alone.
        let req = parse_request(
            b"GET https://api.example/v1 HTTP/1.1\r\nHost: api.example\r\n\r\n",
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(req.url, "https://api.example/v1");
    }

    #[test]
    fn the_ffi_wire_form_round_trips() {
        let req = Request {
            method: "POST".into(),
            url: "https://api.example/v1/chat".into(),
            headers: vec![
                ("Authorization".into(), "Bearer xyz".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: b"{\"a\":1}".to_vec(),
        };
        assert_eq!(Request::decode(&req.encode()).unwrap(), req);

        let headers = vec![
            ("Location".to_string(), "/things/1".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        let encoded = encode_head(201, &headers);
        assert_eq!(decode_head(&encoded).unwrap(), (201, headers));
        assert!(
            decode_head(&encoded[..3]).is_none(),
            "truncation is an error"
        );
        // Truncation is an error, never a panic or a half-parsed request.
        let encoded = req.encode();
        for cut in [0, 1, 4, 10, encoded.len() - 1] {
            assert!(Request::decode(&encoded[..cut]).is_none(), "cut at {cut}");
        }
    }

    // ---- end to end over the netstack, with no emulator ----

    use crate::netstack::{NetConfig, NetStack};

    /// Egress that answers every request from a canned table, in the streamed
    /// form real egress uses: head, then body chunks, then end.
    #[derive(Default)]
    struct FakeEgress {
        seen: Vec<Request>,
        done: Vec<Completion>,
        /// When set, fail instead of answering.
        fail: Option<String>,
        /// Split the body across this many Body completions.
        chunks: usize,
    }

    impl Egress for FakeEgress {
        fn submit(&mut self, id: ReqId, req: Request) {
            self.seen.push(req);
            if let Some(error) = self.fail.clone() {
                self.done.push(Completion::Failed { id, error });
                return;
            }
            self.done.push(Completion::Head {
                id,
                status: 200,
                headers: vec![("Content-Type".into(), "text/plain".into())],
            });
            let body = b"canned body";
            let chunks = self.chunks.max(1);
            for part in body.chunks(body.len().div_ceil(chunks)) {
                self.done.push(Completion::Body {
                    id,
                    bytes: part.to_vec(),
                });
            }
            self.done.push(Completion::End { id });
        }
        fn poll(&mut self) -> Vec<Completion> {
            core::mem::take(&mut self.done)
        }
    }

    /// Drive a guest TCP connection by hand and return what the proxy wrote.
    fn round_trip(request: &str, egress: &mut FakeEgress) -> String {
        let cfg = NetConfig::default();
        let mut stack = NetStack::new(cfg);
        let mut proxy = Proxy::new().keep_scheme();

        // Handshake: SYN, then ACK with the ISS the stack chose.
        let syn = tcp(&cfg, 50000, 100, 0, 0x02, &[]);
        stack.input(&syn);
        let iss = seq_of(&stack.take_output()[0]);
        stack.input(&tcp(&cfg, 50000, 101, iss.wrapping_add(1), 0x10, &[]));
        proxy.pump(&mut stack, egress);
        let _ = stack.take_output();

        // The request, then let the proxy answer.
        stack.input(&tcp(
            &cfg,
            50000,
            101,
            iss.wrapping_add(1),
            0x18,
            request.as_bytes(),
        ));
        proxy.pump(&mut stack, egress);

        let mut written = Vec::new();
        for frame in stack.take_output() {
            written.extend(payload_of(&frame));
        }
        String::from_utf8_lossy(&written).into_owned()
    }

    fn tcp(cfg: &NetConfig, sport: u16, seq: u32, ack: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut seg = Vec::new();
        seg.extend_from_slice(&sport.to_be_bytes());
        seg.extend_from_slice(&cfg.proxy_port.to_be_bytes());
        seg.extend_from_slice(&seq.to_be_bytes());
        seg.extend_from_slice(&ack.to_be_bytes());
        seg.push(5 << 4);
        seg.push(flags);
        seg.extend_from_slice(&65535u16.to_be_bytes());
        seg.extend_from_slice(&[0, 0, 0, 0]);
        seg.extend_from_slice(payload);

        let mut f = Vec::new();
        f.extend_from_slice(&cfg.host_mac);
        f.extend_from_slice(&[0x52, 0x54, 0, 1, 2, 3]);
        f.extend_from_slice(&0x0800u16.to_be_bytes());
        let ip_start = f.len();
        f.push(0x45);
        f.push(0);
        f.extend_from_slice(&((20 + seg.len()) as u16).to_be_bytes());
        f.extend_from_slice(&[0, 0, 0x40, 0]);
        f.push(64);
        f.push(6);
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(&cfg.guest_ip);
        f.extend_from_slice(&cfg.host_ip);
        let _ = ip_start;
        f.extend_from_slice(&seg);
        f
    }

    fn seq_of(frame: &[u8]) -> u32 {
        let ip = &frame[14..];
        let ihl = (ip[0] & 0x0f) as usize * 4;
        u32::from_be_bytes(ip[ihl + 4..ihl + 8].try_into().unwrap())
    }

    fn payload_of(frame: &[u8]) -> Vec<u8> {
        let ip = &frame[14..];
        let ihl = (ip[0] & 0x0f) as usize * 4;
        let total = u16::from_be_bytes([ip[2], ip[3]]) as usize;
        let seg = &ip[ihl..total];
        let off = (seg[12] >> 4) as usize * 4;
        seg[off..].to_vec()
    }

    #[test]
    fn a_guest_request_reaches_egress_and_the_response_comes_back() {
        let mut egress = FakeEgress::default();
        let written = round_trip(
            "GET http://example.test/hello HTTP/1.1\r\nHost: example.test\r\n\r\n",
            &mut egress,
        );
        assert_eq!(egress.seen.len(), 1, "the request reached egress");
        assert_eq!(egress.seen[0].url, "http://example.test/hello");
        assert!(written.starts_with("HTTP/1.1 200 OK\r\n"), "got: {written}");
        assert!(written.contains("Connection: close\r\n"), "got: {written}");
        // No Content-Length: the body is delimited by the close, because fetch
        // decompresses transparently and the upstream length would be wrong.
        assert!(!written.contains("Content-Length"), "got: {written}");
        assert!(written.ends_with("canned body"), "got: {written}");
    }

    #[test]
    fn a_chunked_guest_request_reaches_egress_dechunked() {
        let mut egress = FakeEgress::default();
        let written = round_trip(
            "POST http://example.test/upload HTTP/1.1\r\n\
             Host: example.test\r\n\
             Content-Type: text/plain\r\n\
             Transfer-Encoding: chunked\r\n\r\n\
             6\r\nhello \r\n\
             5\r\nworld\r\n\
             0\r\n\r\n",
            &mut egress,
        );
        assert_eq!(egress.seen.len(), 1);
        assert_eq!(egress.seen[0].body, b"hello world");
        assert!(!egress.seen[0]
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("transfer-encoding")));
        assert!(written.starts_with("HTTP/1.1 200 OK\r\n"), "got: {written}");
    }

    #[test]
    fn an_egress_failure_becomes_a_502() {
        let mut egress = FakeEgress {
            fail: Some("network unreachable".into()),
            ..Default::default()
        };
        let written = round_trip(
            "GET http://example.test/ HTTP/1.1\r\nHost: example.test\r\n\r\n",
            &mut egress,
        );
        assert!(
            written.starts_with("HTTP/1.1 502 Bad Gateway\r\n"),
            "got: {written}"
        );
        // The guest gets to see why, rather than a bare reset.
        assert!(written.contains("network unreachable"), "got: {written}");
    }

    #[test]
    fn connect_opens_a_tls_tunnel_without_submitting_it() {
        let mut egress = FakeEgress::default();
        let written = round_trip(
            "CONNECT api.example:443 HTTP/1.1\r\nHost: api.example:443\r\n\r\n",
            &mut egress,
        );
        assert_eq!(written, "HTTP/1.1 200 Connection Established\r\n\r\n");
        assert!(egress.seen.is_empty(), "nothing should have been submitted");
    }
}
