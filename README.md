# rv64.js

> [!NOTE]
> This codebase was written 100% with AI using Claude Code and Codex, directed
> by [@ibuildthecloud](https://github.com/ibuildthecloud), who is not a
> virtualization or JIT expert.

A RISC-V RV64 full-system emulator written in Rust, with a WebAssembly target
and a JavaScript browser frontend. Its machine model is based on
[TinyEMU](https://bellard.org/tinyemu/), and its browser architecture is
inspired by [copy/v86](https://github.com/copy/v86).

**Project status: pre-release, with working Linux boots.** The native Rust
interpreter and the browser's WebAssembly interpreter/JIT boot Linux to an
interactive shell. The browser demo offers a small Linux 4.15/BusyBox system
and a larger OpenSBI/Linux 6.12/Debian system.

The separate Linux user-mode runner is **experimental**. It can load static
riscv64 ELF executables and supports enough of the Linux syscall ABI for the
repository's small, static musl test programs. It is not a general-purpose
`qemu-riscv64` replacement: there is currently no guest filesystem, process
creation, networking, threading, or complete signal handling, and several
implemented syscalls are compatibility stubs.

Implemented and tested paths include:

- RV64 I/M/A/F/D/C, Zicsr, and the privileged architecture
- Sv39 and Sv48 virtual memory
- WebAssembly JIT in both user-mode and full-system browser run loops
- virtio console and block devices
- virtio-9p host-directory and in-memory file sharing
- virtio networking through WebSocket or an in-browser HTTP proxy

Architecture details live in [DESIGN.md](DESIGN.md). See
[ROADMAP.md](ROADMAP.md) for future work and
[PERFORMANCE_PROGRESS.md](PERFORMANCE_PROGRESS.md) for measured JIT results.
Claims here describe the current test coverage, not complete RISC-V or Linux
compatibility.

## Quick start

### Browser

The hosted demo is available at
<https://ibuildthecloud.github.io/rv64.js/>. To run it locally:

```sh
web/get-images.sh
cargo build -p rv64-wasm --target wasm32-unknown-unknown --release
python3 -m http.server -d . 8000
```

Open <http://localhost:8000/web/>.

The page offers a fast BusyBox machine and a modern OpenSBI/Linux 6.12 Debian
machine. Prepare the modern images once with:

```sh
nix develop -c web/prepare-modern-images.sh
```

The same boot paths are available as runnable Node examples. These commands
forward the host terminal to the guest:

```sh
node examples/boot-linux.mjs fast
node --max-old-space-size=2048 examples/boot-linux.mjs modern
```

For a non-interactive smoke test, stop after a known boot marker:

```sh
RV64_UNTIL='~ #' node examples/boot-linux.mjs fast
RV64_UNTIL=BENCH_READY node --max-old-space-size=2048 examples/boot-linux.mjs modern
```

### Native full-system emulator

```sh
cargo build --release -p rv64-system
target/release/rv64-boot web/images/bbl64.bin web/images/kernel-riscv64.bin web/images/root-riscv64.bin
```

Share a host directory over virtio-9p:

```sh
target/release/rv64-boot web/images/*.bin --9p ~/src
```

Then mount it in the guest:

```sh
mount -t 9p -o trans=virtio,version=9p2000.L host /mnt
```

Enable networking through the in-process HTTP proxy:

```sh
target/release/rv64-boot web/images/*.bin --proxy
```

Configure the guest:

```sh
ifconfig eth0 10.0.2.15 netmask 255.255.255.0 up
export http_proxy=http://10.0.2.2:8080
wget -O- http://example.com/
```

HTTPS uses CONNECT through an ephemeral local CA. The `--proxy` option exposes
its public certificate as `/ca.der` on the read-only 9p tag `rv64-proxy`.

### Browser HTTP relay

```sh
node web/http-relay.mjs --port 8090
```

Connect it before or after `bootLinux({ proxy: true })`:

```js
vm.connectHttpRelay("ws://127.0.0.1:8090");
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

### Modern OpenSBI/Linux machine

```sh
nix develop -c tests/vs-v86/mk-debian-rootfs.sh target/bench
nix develop -c tests/virt-proxy/run.sh
```

### User-mode emulator

This runner is experimental and intended for the included static test guests,
instruction testing, and JIT development. It does not currently provide a
general Linux userspace environment.

```sh
cargo run --release -p rv64-run -- <static-elf> [args...]
```

## Build & test

```sh
# reproducible dev environment (rust + cross targets, node, qemu, spike,
# riscv cross-gcc, wabt/binaryen, dtc — everything validation needs)
nix develop

# the full automated suite: cargo tests, guest builds, qemu differential,
# official riscv-tests, architecture-signature comparison against Spike,
# wasm build + Node smoke (limited user mode, JIT, Linux boot). Unavailable
# external stages are reported as skips.
tests/run-all.sh

# release gate: treat any unavailable tool-dependent stage as a failure
REQUIRE_ALL=1 tests/run-all.sh

# individual pieces
cargo test --workspace                  # unit + integration tests
tests/run-isa-tests.sh                  # official ISA suite only
cargo build -p rv64-wasm --target wasm32-unknown-unknown --release
python3 -m http.server -d . 8000        # then open /web/

# native TinyEMU oracle (differential testing)
make -C reference/tinyemu CONFIG_FS_NET= CONFIG_SDL= CONFIG_X86EMU= CONFIG_SLIRP=
```

Validation status lives in [tests/VALIDATION.md](tests/VALIDATION.md).
The source-release checklist and known gate limitations live in
[RELEASING.md](RELEASING.md).

## Layout

- `crates/rv64-core` — portable CPU core (`no_std`, generic over a `Bus` trait)
- `crates/rv64-wasm` — `extern "C"` wasm export surface (no wasm-bindgen)
- `web/` — JS loader + demo page
- `reference/tinyemu/` — vendored TinyEMU (MIT, Fabrice Bellard): spec map & test oracle

## License

MIT; see [LICENSE](LICENSE). `reference/tinyemu/` retains its own MIT license and copyright
(Fabrice Bellard); see `reference/README.md`.
