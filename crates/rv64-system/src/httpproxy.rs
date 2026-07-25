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
//! - Request bodies with `Content-Length` (so `POST` to an API works). Chunked
//!   request bodies are rejected with 501 rather than silently mishandled.
//! - `CONNECT` is answered 501. Supporting it means terminating TLS ourselves —
//!   a TLS server and per-host certificate minting — which is deliberately a
//!   later step. Until then the guest addresses `http://` URLs and [`Egress`]
//!   decides the wire scheme.
//! - One request per connection: every response says `Connection: close`. A
//!   proxy is allowed to do this, and it keeps request framing trivial.

use crate::netstack::{ConnId, Event, NetStack};
use std::collections::HashMap;

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

/// How requests actually leave the host.
///
/// Deliberately submit-then-poll rather than blocking: `fetch()` is async and
/// the guest's TCP connection has to stay open across it, so completion cannot
/// be a return value. Native implementations can complete immediately.
pub trait Egress {
    fn submit(&mut self, id: ReqId, req: Request);
    /// Requests that have finished since the last call. `Err` becomes a 502.
    fn poll(&mut self) -> Vec<(ReqId, Result<Response, String>)>;
}

/// Largest request head and body we will buffer from the guest, so a hostile or
/// broken guest cannot grow host memory without bound.
const MAX_HEAD: usize = 64 * 1024;
const MAX_BODY: usize = 32 * 1024 * 1024;

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
}

pub struct Proxy {
    conns: HashMap<ConnId, ConnBuf>,
    /// In-flight request id -> the connection waiting for it.
    inflight: HashMap<ReqId, ConnId>,
    next_req: ReqId,
    /// Rewrite `http://` to `https://` on egress.
    upgrade_scheme: bool,
    requests: u64,
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
            // essentially every host worth reaching is https anyway. The guest
            // still writes http:// URLs, since without CONNECT support that is
            // all it can address.
            upgrade_scheme: true,
            requests: 0,
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
        for (req_id, result) in egress.poll() {
            let Some(conn) = self.inflight.remove(&req_id) else {
                continue;
            };
            match result {
                Ok(response) => self.write_response(conn, &response, stack),
                Err(err) => self.write_error(conn, 502, "Bad Gateway", &err, stack),
            }
        }
    }

    fn on_data(&mut self, id: ConnId, bytes: Vec<u8>, stack: &mut NetStack, egress: &mut dyn Egress) {
        let Some(state) = self.conns.get_mut(&id) else {
            return;
        };
        if state.submitted {
            return; // one request per connection; see module docs
        }
        state.buf.extend_from_slice(&bytes);
        if state.buf.len() > MAX_HEAD + MAX_BODY {
            self.write_error(id, 413, "Payload Too Large", "request too large", stack);
            return;
        }
        match parse_request(&state.buf, self.upgrade_scheme) {
            Ok(None) => {} // still arriving
            Ok(Some(req)) => {
                state.submitted = true;
                let req_id = self.next_req;
                self.next_req += 1;
                self.requests += 1;
                self.inflight.insert(req_id, id);
                egress.submit(req_id, req);
            }
            Err(ParseError { status, reason, detail }) => {
                self.write_error(id, status, reason, detail, stack)
            }
        }
    }

    fn write_response(&mut self, conn: ConnId, response: &Response, stack: &mut NetStack) {
        let mut out = format!(
            "HTTP/1.1 {} {}\r\n",
            response.status,
            reason_phrase(response.status)
        )
        .into_bytes();
        for (name, value) in &response.headers {
            if HOP_BY_HOP.contains(&name.to_ascii_lowercase().as_str())
                || name.eq_ignore_ascii_case("content-length")
            {
                continue; // length is authoritative below
            }
            out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        out.extend_from_slice(format!("Content-Length: {}\r\n", response.body.len()).as_bytes());
        out.extend_from_slice(b"Connection: close\r\n\r\n");
        out.extend_from_slice(&response.body);
        stack.send(conn, &out);
        stack.close(conn);
        self.conns.remove(&conn);
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
        stack.send(conn, head.as_bytes());
        stack.send(conn, body.as_bytes());
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
    let mut content_length = 0usize;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "host" => host = value.to_string(),
            "content-length" => content_length = value.parse().unwrap_or(0),
            "transfer-encoding" if value.to_ascii_lowercase().contains("chunked") => chunked = true,
            _ => {}
        }
        if !HOP_BY_HOP.contains(&lower.as_str()) {
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
    if chunked {
        return Err(ParseError {
            status: 501,
            reason: "Not Implemented",
            detail: "chunked request bodies are not supported yet",
        });
    }
    if content_length > MAX_BODY {
        return Err(ParseError {
            status: 413,
            reason: "Payload Too Large",
            detail: "request body too large",
        });
    }

    // Wait for the whole body before submitting: egress is request/response
    // shaped and cannot stream a partial body.
    if buf.len() < head_end + content_length {
        return Ok(None);
    }
    let body = buf[head_end..head_end + content_length].to_vec();

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
            assert!(!names.contains(&banned.to_string()), "{banned} must be stripped");
        }
        assert!(names.contains(&"accept".to_string()), "end-to-end headers stay");
    }

    #[test]
    fn waits_for_the_whole_head_and_body() {
        // Head split mid-way: nothing to submit yet.
        assert!(parse("GET http://h/ HTTP/1.1\r\nHost: h\r\n").unwrap().is_none());
        // Head complete but body outstanding.
        let partial = "POST http://h/x HTTP/1.1\r\nHost: h\r\nContent-Length: 11\r\n\r\nhello";
        assert!(parse(partial).unwrap().is_none());
        let whole = "POST http://h/x HTTP/1.1\r\nHost: h\r\nContent-Length: 11\r\n\r\nhello world";
        let req = parse(whole).unwrap().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.body, b"hello world");
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
        // CONNECT needs TLS termination; better an explicit 501 than a hang.
        let err = parse("CONNECT api.example:443 HTTP/1.1\r\nHost: api.example:443\r\n\r\n")
            .err()
            .expect("CONNECT must be refused");
        assert_eq!(err.status, 501);
        let err = parse("POST http://h/ HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n")
            .err()
            .expect("chunked must be refused");
        assert_eq!(err.status, 501);
        let err = parse("GET nonsense HTTP/1.1\r\n\r\n")
            .err()
            .expect("unproxyable target must be refused");
        assert_eq!(err.status, 400);
    }

    #[test]
    fn upgrades_the_scheme_for_egress() {
        // A page served over https cannot fetch http://, so egress uses https
        // even though the guest can only address http:// without CONNECT.
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

    // ---- end to end over the netstack, with no emulator ----

    use crate::netstack::{NetConfig, NetStack};

    /// Egress that answers every request from a canned table.
    #[derive(Default)]
    struct FakeEgress {
        seen: Vec<Request>,
        done: Vec<(ReqId, Result<Response, String>)>,
        /// When set, fail instead of answering.
        fail: Option<String>,
    }

    impl Egress for FakeEgress {
        fn submit(&mut self, id: ReqId, req: Request) {
            self.seen.push(req);
            let result = match &self.fail {
                Some(err) => Err(err.clone()),
                None => Ok(Response {
                    status: 200,
                    headers: vec![("Content-Type".into(), "text/plain".into())],
                    body: b"canned body".to_vec(),
                }),
            };
            self.done.push((id, result));
        }
        fn poll(&mut self) -> Vec<(ReqId, Result<Response, String>)> {
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
        stack.input(&tcp(&cfg, 50000, 101, iss.wrapping_add(1), 0x18, request.as_bytes()));
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
        assert!(written.contains("Content-Length: 11\r\n"), "got: {written}");
        assert!(written.contains("Connection: close\r\n"));
        assert!(written.ends_with("canned body"), "got: {written}");
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
        assert!(written.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "got: {written}");
        // The guest gets to see why, rather than a bare reset.
        assert!(written.contains("network unreachable"), "got: {written}");
    }

    #[test]
    fn a_refused_request_is_answered_not_dropped() {
        let mut egress = FakeEgress::default();
        let written = round_trip(
            "CONNECT api.example:443 HTTP/1.1\r\nHost: api.example:443\r\n\r\n",
            &mut egress,
        );
        assert!(written.starts_with("HTTP/1.1 501 Not Implemented\r\n"), "got: {written}");
        assert!(egress.seen.is_empty(), "nothing should have been submitted");
    }
}
