# Networking and proxy handoff (updated 2026-07-27)

Status snapshot for whoever picks up the networking/proxy work next. Read this
before ROADMAP.md items 7/8/8b (which have the full narrative) — this is the
condensed "what's true right now" version.

The original HTTP proxy work described here landed on `main` as:
```
d88b429 proxy: guest configures itself by DHCP; record why MITM is deferred
b61f018 proxy: responses stream instead of buffering; restore the browser path
```
The RTC, modern-Debian, CONNECT/MITM, native HTTPS, 9p CA-delivery, and relay
continuation is the completed follow-on described below; its functional commit
is `e2d4730 feat(proxy): add end-to-end HTTPS MITM support`.

## Implementation continuation (2026-07-26)

The prerequisites that were missing in the original snapshot are now
implemented and tested:

- Both machine models expose a goldfish RTC at `0x00101000`, PLIC source 11.
  Native runners and the raw wasm host ABI seed it from Unix wall time and
  refresh it between CPU slices. `tests/virt-smoke` now requires
  `CLOCK_REALTIME` to be a modern epoch.
- `rv64-vboot --proxy` drives the same in-process DHCP/netstack/HTTP proxy as
  `rv64-boot --proxy`.
- `tests/vs-v86/mk-debian-rootfs.sh target/bench` builds the persistent
  `target/bench/deb-riscv64.ext4` guest. It contains Python, curl/OpenSSL, CA
  roots, iproute2, busybox/udhcpc, and a working lease hook.
- The flake's virt kernel has virtio block/net/9p, ext4, and packet sockets
  built in. Packet sockets are required by `udhcpc`; 9p makes the proxy CA
  available before networking; and the root image intentionally has no matching
  kernel module tree.
- The virtio-net ring advertises 256 entries. A 16-entry ring lets modern
  `virtio_net` send once and then leaves its TX queue stopped because it cannot
  reserve enough descriptors for a maximally fragmented skb.
- `tests/virt-proxy/run.sh` boots that Debian disk and passes the complete RTC
  -> boot-time CA mount/install -> DHCP -> loopback HTTP/chunked regressions ->
  real `http://example.com` -> MITM `https://example.com` path. It is deliberately
  a manual integration test because guest creation downloads packages, it
  requires external networking, and the first cross-kernel realization is
  expensive.

## Context: how we got here

The user asked how a VM running in a browser gets outbound network connectivity
("can it `curl`?"). Investigation found:
- A page's only egress primitive is `fetch()`, gated by CORS on the response
  side and by same-origin/mixed-content on the request side.
- A WebSocket relay (v86/websockproxy style) gives full fidelity but needs
  external infrastructure that doesn't exist yet — no relay is shipped.
- The chosen design: an **in-browser HTTP proxy**. The guest sets `http_proxy`;
  we terminate its TCP, parse the HTTP request, and perform it via `fetch()`.
  This needs *no* external infrastructure, which is the entire point.

## What's done and verified

**virtio-net device** (`crates/rv64-system/src/virtio.rs`, `Backend::Net`) —
layer-2 only, RX queue 0 / TX queue 1, `VIRTIO_NET_F_MAC`. Two transports:
- `ws.rs` — RFC 6455 client, no dependencies, for a WebSocket relay.
- `netstack.rs` + `httpproxy.rs` — the in-browser proxy path (below).

**`netstack.rs`** — minimal host-side stack behind the NIC. ARP, DHCP
(server), ICMP echo, and TCP terminated to exactly one address:port (the
proxy). Deliberately **not** a NAT/router: no DNS, no per-destination
tracking, no UDP beyond DHCP. Pure logic (frames in, frames out, events out),
so it's unit-testable with no CPU emulator involved.

**`httpproxy.rs`** — parses absolute-URI HTTP requests off the netstack's TCP
streams, hands them to an `Egress` trait (`submit`/`poll`, async-friendly).
**Responses stream**: `Egress` reports `Head` → `Body`* → `End`/`Failed`
rather than one buffered `Response`, so an SSE/long response reaches the guest
as it arrives. `content-length` and `content-encoding` are deliberately
stripped from the forwarded head (fetch decompresses transparently — forwarding
either would corrupt or truncate); `Connection: close` delimits the body
instead.

**Two `Egress` impls:**
- `egress.rs` — native, real sockets for HTTP and a rustls client for HTTPS.
  It loads the host CA bundle and validates the upstream chain and hostname;
  ALPN deliberately stays on HTTP/1.1.
- `web/rv64.js` `performHttp` — browser, real `fetch()`, so it gets HTTPS for
  free (the browser does the TLS). Streams via `response.body.getReader()`.

**Wiring:**
- `rv64-boot --proxy` and `rv64-vboot --proxy` (native).
- `bootLinux({ proxy: true, proxyUpgradeHttps })` in `web/rv64.js`, plus
  `sys_proxy_enable`/`sys_http_head`/`sys_http_body`/`sys_http_end`/
  `sys_http_fail`/`sys_proxy_url` in the wasm ABI (`rv64-wasm/src/lib.rs`).

**Tests (all passing):**
- `crates/rv64-system/tests/proxy_boot.rs` — boots the TinyEMU guest, guest
  configures **by real DHCP** (`udhcpc`, our server answers), sets
  `http_proxy`, `wget`s through it. Covers: a multi-segment response
  (md5-verified), a **trickled** response (body released one chunk per host
  poll, proving the streaming path reassembles correctly across many CPU
  slices), a 404 passed through, and an egress failure surfacing as a 502.
- `tests/wasm-smoke.mjs` — same path through the **real browser `fetch()`**
  against a loopback origin, under the JIT.
- Unit tests for `netstack.rs` (TCP handshake/window/retransmit/FIN, ARP, DHCP,
  ICMP — mutation-tested: a corrupted TCP checksum makes the test fail, proving
  it isn't vacuous) and `httpproxy.rs` (parsing, hop-by-hop stripping, codec).
- `ws_relay.rs`, `net_boot.rs` cover the WebSocket-relay transport separately.

103 cargo tests pass workspace-wide; wasm-smoke, virt-smoke, and the 134/134
riscv-tests ISA suite are all green.

The final 2026-07-27 validation reran the 103 release-mode workspace tests,
the release Wasm build and raw-import audit, the Node HTTP-relay and Wasm smoke
tests, and the Debian virt-proxy E2E. The latter reached real `example.com`
over both HTTP and HTTPS and printed `PROXY_CA_READY`, `CA_9P_OK`,
`CURL_HTTP_EXAMPLE_OK`, and `CURL_TLS_MITM_OK`; curl verified a leaf for
`example.com` issued by `rv64.js ephemeral proxy CA`.

**Trap worth knowing:** the TinyEMU guest image has **no udhcpc lease
script** — busybox has `/usr/share/udhcpc/default.script` compiled in as a
path, but the file doesn't exist in this rootfs. `udhcpc` reports a lease and
successfully completes the DHCP exchange but leaves the interface
unconfigured. This looks exactly like a broken DHCP server and isn't one —
`proxy_boot.rs` writes a minimal script before invoking `udhcpc -s`.

## CONNECT/TLS continuation

**`CONNECT` + TLS termination is implemented and tested.** The chosen
dependency/ABI route is:

- rustls 0.23 with `oxitls-rustcrypto-provider` 0.2, so `ring` is not in the
  active `rv64-system` graph;
- rcgen with a small local Ed25519 signing-key adapter, avoiding
  `oxitls-rcgen`'s unconditional Tokio/mio dependency path;
- a custom getrandom 0.2 backend wired to the existing raw `host_random`
  import;
- vendored `rustls-pki-types` 1.15.1 with only its wasm `UnixTime::now()` path
  changed from `web-time`/wasm-bindgen to raw `env.host_unix_ms`.

`tlsproxy.rs` owns one ephemeral CA and leaf key, caches per-host rustls server
configuration, and `httpproxy.rs` routes decrypted requests through the
existing `Egress` interface. The CA remains available as DER from
`http://rv64-proxy.invalid/ca.der`, and every proxy-enabled machine now also
exposes its public certificate as `/ca.der` on the fixed virtio-9p tag
`rv64-proxy`. The private signing key never enters the guest. Generic guests
still choose their own trust policy; the purpose-built Debian integration image
mounts that tag and installs the CA into its system bundle before `BENCH_READY`.

The optimized `rv64_wasm.wasm` import audit contains exactly the expected raw
`env` functions (`host_random`, `host_now_ms`, `host_unix_ms`, `host_write`,
`host_http_request`, both JIT registration calls, and `host_net_send`). It has
no `__wbindgen`, externref-table, or `web_time` imports. Native egress now uses
the same rustls provider as a validating TLS client for its independent
upstream connection. `tests/virt-proxy` reaches both forms of real example.com;
for HTTPS, guest curl reports `subject: CN=example.com`,
`issuer: CN=rv64.js ephemeral proxy CA`, and `SSL certificate verify ok.`

Chunked request bodies are also complete. `httpproxy.rs` incrementally waits
for a complete chunk stream, enforces the decoded-body limit, strips framing,
and sends the reconstructed body through `Egress`. Ambiguous length/framing,
unsupported stacked transfer codings, malformed chunks, and unrepresentable
request trailers receive explicit errors. `tests/virt-proxy` proves this with
a real chunked curl POST from Debian.

Per-request relay fallback is complete. `connectHttpRelay()` attaches a
request-level WebSocket relay without replacing the in-process proxy or its
zero-infrastructure fetch path. GET/HEAD failures before a response head are
retried through the relay and the origin is cached for direct relay routing.
Unsafe methods are never automatically retried because a CORS-hidden response
does not prove the origin missed the request; opt those origins in beforehand
with `routeHttpViaRelay()`. `web/http-relay.mjs` is the loopback-default Node
relay, with explicit Origin allowlisting for remote pages. Tests cover both
the real WebSocket server and fetch-failure fallback through a booted Wasm
guest.

## Recommended next steps, in order

1. Keep the final release-Wasm import audit when upgrading rustls,
   rustls-pki-types, rcgen, or the crypto provider; compile success alone does
   not prove compatibility with the raw host ABI.
2. The planned networking/proxy work is complete. The next roadmap feature is
   snapshot save/restore (item 10).

## Key files

| File | What |
|---|---|
| `crates/rv64-system/src/netstack.rs` | ARP/DHCP/ICMP/TCP termination, no CPU needed to test |
| `crates/rv64-system/src/httpproxy.rs` | HTTP/CONNECT parse + TLS record routing + streaming completions |
| `crates/rv64-system/src/tlsproxy.rs` | Ephemeral CA, per-host leaf certificates, rustls server sessions |
| `crates/rv64-system/src/egress.rs` | Native `Egress`: HTTP sockets + validating rustls HTTPS client |
| `crates/rv64-system/src/ws.rs` | WebSocket relay client (separate transport, not the proxy) |
| `crates/rv64-system/src/virtio.rs` | `Backend::Net` and multiple tagged `Backend::Fs` 9p devices |
| `crates/rv64-wasm/src/lib.rs` | wasm ABI: `sys_proxy_*`, `sys_http_*`, `sys_net_*` |
| `web/rv64.js` | Browser `Egress`: fetch-first routing, HTTP relay client, codecs |
| `web/http-relay.mjs` | Loopback-default request relay for CORS-blocked origins |
| `crates/rv64-system/tests/proxy_boot.rs` | End-to-end proof, DHCP + streaming + errors |
| `tests/http-relay.mjs` | Real WebSocket request-relay protocol and streaming test |
| `tests/wasm-smoke.mjs` | Same, through real browser `fetch()` |
| `tests/virt-proxy/run.sh` | Debian RTC + DHCP + HTTP/HTTPS curl through `rv64-vboot --proxy` |
| `tests/vs-v86/mk-debian-rootfs.sh` | Reproducible TLS-capable Debian guest builder |
| `vendor/rustls-pki-types-1.15.1/RV64-PATCH.md` | Vendored clock-patch rationale and upgrade audit |
| `ROADMAP.md` items 7, 8, 8b | Full narrative history, dates, measurements |

## Measured facts worth keeping (don't re-derive)

- CORS is open (`Access-Control-Allow-Origin: *`) on `api.github.com`,
  `raw.githubusercontent.com`, `api.openai.com`, `registry.npmjs.org` (incl.
  `.tgz` artifacts), `files.pythonhosted.org`, `static.crates.io`. Closed on
  `deb.debian.org`, `codeload.github.com`, ordinary web pages. So `pip
  install`/`npm install`-style traffic works with zero infrastructure; general
  browsing does not.
- virtio-net header is **12 bytes** (`virtio_net_hdr_v1`), not 10 — we
  negotiate `VIRTIO_F_VERSION_1`, which changes the header size Linux expects.
  Confirmed by mutation test (shrinking it to 10 breaks RX silently).
- The guest kernel currently used (`web/images/*`) is TinyEMU's 2018 buildroot
  image — no TLS, no wall clock, DHCP client present (busybox `udhcpc`) but no
  lease script.
