# rv64.js

A RISC-V (RV64) emulator for the browser — [TinyEMU](https://bellard.org/tinyemu/)'s
scope with [copy/v86](https://github.com/copy/v86)'s architecture: Rust CPU core
compiled to WebAssembly, plain-JS device/browser layer.

**Status: boots Linux.** rv64gc interpreter (I/M/A/F/D/C + Zicsr + full
privileged arch with sv39/sv48 MMU) boots TinyEMU's stock Linux 4.15 +
buildroot image to an interactive busybox shell — natively (~50 Minsn/s)
and in the browser. User-mode emulation (qemu-user style) runs static
riscv64 musl binaries. The JIT is live in both run loops: hot blocks compile to wasm modules dispatched via call_indirect (~3x on hot code, with safe invalidation).
Devices are virtio-mmio: console, block, **9p** for sharing a host directory (or
an in-memory tree, in the browser) into the guest, and **net** — either over a
WebSocket relay, or through an in-browser HTTP proxy that needs no external
infrastructure at all (egress is the page's own `fetch`).
See [DESIGN.md](DESIGN.md) for architecture,
[ROADMAP.md](ROADMAP.md) for project-wide future work, and
[PERFORMANCE_PROGRESS.md](PERFORMANCE_PROGRESS.md) for current JIT performance
status, decisions, and experiments.

```sh
# boot Linux in the browser
web/get-images.sh
cargo build -p rv64-wasm --target wasm32-unknown-unknown --release
python3 -m http.server -d . 8000    # open http://localhost:8000/web/system.html

# boot Linux natively
cargo build --release -p rv64-system
target/release/rv64-boot web/images/bbl64.bin web/images/kernel-riscv64.bin web/images/root-riscv64.bin

# ...sharing a host directory over virtio-9p; in the guest:
#   mount -t 9p -o trans=virtio,version=9p2000.L host /mnt
target/release/rv64-boot web/images/*.bin --9p ~/src

# ...with networking through the in-process HTTP proxy; in the guest:
#   ifconfig eth0 10.0.2.15 netmask 255.255.255.0 up
#   export http_proxy=http://10.0.2.2:8080 && wget -O- http://example.com/
# HTTPS uses CONNECT through an ephemeral local CA. `--proxy` exposes its public
# certificate as /ca.der on the fixed 9p tag `rv64-proxy`; mount it read-only:
#   mount -t 9p -o trans=virtio,version=9p2000.L,ro rv64-proxy /run/rv64-proxy
# The Debian test image installs it into the system trust bundle during boot;
# other guests can convert/install it according to their own trust policy.
#   curl -x "$http_proxy" https://example.com/
target/release/rv64-boot web/images/*.bin --proxy

# optional browser CORS fallback (loopback-only by default)
node web/http-relay.mjs --port 8090
# In the page, before or after bootLinux({ proxy: true }):
#   vm.connectHttpRelay("ws://127.0.0.1:8090")

# boot the modern OpenSBI/Linux machine with the reusable Debian test disk
nix develop -c tests/vs-v86/mk-debian-rootfs.sh target/bench
nix develop -c tests/virt-proxy/run.sh

# run a static riscv64 Linux binary (qemu-user style)
cargo run --release -p rv64-run -- <static-elf> [args...]
```

The browser proxy tries `fetch()` first, retaining its zero-infrastructure
path. If a GET or HEAD fails before a response is exposed, an attached HTTP
relay retries it and remembers that origin for later requests. Non-idempotent
requests are never retried automatically; opt an origin in beforehand with
`vm.routeHttpViaRelay("https://example.com")`. The relay accepts only local
page origins by default; use `--allow-origin` explicitly for a remotely served
page, and put it behind a `wss://` reverse proxy when the page itself uses
HTTPS. See [web/HTTP_RELAY.md](web/HTTP_RELAY.md) for the wire protocol and
deployment details.

## Build & test

```sh
# reproducible dev environment (rust + cross targets, node, qemu, spike,
# riscv cross-gcc, wabt/binaryen, dtc — everything validation needs)
nix develop

# the full automated suite: cargo tests, guest builds, qemu differential,
# official riscv-tests (134/134), wasm build + Node smoke (user-mode, JIT,
# Linux boot). Stages skip gracefully when a tool is missing.
tests/run-all.sh

# individual pieces
cargo test --workspace                  # unit + integration tests
tests/run-isa-tests.sh                  # official ISA suite only
cargo build -p rv64-wasm --target wasm32-unknown-unknown --release
python3 -m http.server -d . 8000        # then open /web/system.html

# native TinyEMU oracle (differential testing)
make -C reference/tinyemu CONFIG_FS_NET= CONFIG_SDL= CONFIG_X86EMU= CONFIG_SLIRP=
```

Validation status lives in [tests/VALIDATION.md](tests/VALIDATION.md).

## Layout

- `crates/rv64-core` — portable CPU core (`no_std`, generic over a `Bus` trait)
- `crates/rv64-wasm` — `extern "C"` wasm export surface (no wasm-bindgen)
- `web/` — JS loader + demo page
- `reference/tinyemu/` — vendored TinyEMU (MIT, Fabrice Bellard): spec map & test oracle

## License

MIT. `reference/tinyemu/` retains its own MIT license and copyright
(Fabrice Bellard); see `reference/README.md`.
