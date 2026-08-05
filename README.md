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
and an Alpine 3.24/Linux 6.12 system with working `apk` networking.

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
- virtio networking through fetch, WISP, wsproxy, or a browser-local LAN

Architecture details live in [DESIGN.md](DESIGN.md). See
[ROADMAP.md](ROADMAP.md) for future work and
[PERFORMANCE_PROGRESS.md](PERFORMANCE_PROGRESS.md) for measured JIT results.
The planned stable JavaScript API and direct-Linux boot contract are recorded
in [API_DESIGN.md](API_DESIGN.md).
Claims here describe the current test coverage, not complete RISC-V or Linux
compatibility.

## Quick start

### Browser

The hosted demo is available at
<https://ibuildthecloud.github.io/rv64.js/>. To run it locally:

```sh
nix develop -c web/prepare-modern-images.sh
cargo build -p rv64-wasm --target wasm32-unknown-unknown --release
python3 -m http.server -d . 8000
```

Open <http://localhost:8000/web/>.

The page boots Alpine 3.24 on the current Linux 6.12 kernel. Prepare its images
once with:

```sh
nix develop -c web/prepare-modern-images.sh
```

The same boot paths are available as runnable Node examples. These commands
forward the host terminal to the guest:

```sh
node examples/boot-linux.mjs fast
node examples/boot-linux.mjs modern
```

For a non-interactive smoke test, stop after a known boot marker:

```sh
RV64_UNTIL='~ #' node examples/boot-linux.mjs fast
RV64_UNTIL=ALPINE_READY node examples/boot-linux.mjs modern
```

The browser and Node example uses the typed public API from `web/rv64.js`:

```js
import { RV64 } from "./rv64.js";

const vm = await RV64.create({
  wasm: { url: "./rv64_wasm.wasm" },
  memoryMB: 512,
  boot: {
    mode: "linux-direct",
    kernel: { url: "./Image" },
    disk: { url: "./alpine.ext4" },
    cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
  },
  events: { console: (bytes) => terminal.write(bytes) },
});
await vm.start();
vm.console.send("uname -a\n");
vm.console.send("apk update\n");
```

The library owns the execution loop. Its lifecycle is `start()`, `stop()`,
`reset()`, and `destroy()`; `on()` provides typed events and returns an
unsubscribe function. See [web/rv64.d.ts](web/rv64.d.ts) and
[API_DESIGN.md](API_DESIGN.md). `linux-direct` is part of the declared boot
model and boots Linux in supervisor mode through the emulator-provided SBI,
without loading or executing a firmware image. Use `mode: "linux-direct"` and
omit `firmware`; the kernel, disk/initrd, command line, and lifecycle are
otherwise unchanged.

### JavaScript and TypeScript API

`web/rv64.js` is a standard ES module. Its adjacent `web/rv64.d.ts` declaration
provides the same API to TypeScript without a separate `@types` package:

```ts
import { RV64, type RV64Options } from "./web/rv64.js";

const options: RV64Options = {
  wasm: { url: "./rv64_wasm.wasm" },
  memoryMB: 512,
  boot: {
    mode: "linux-direct",
    kernel: { url: "./Image" },
    disk: { url: "./alpine.ext4" },
    cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
  },
  network: { mode: "fetch", relayURL: "wss://relay.example/relay" },
  events: {
    downloadProgress: ({ image, loaded, total }) => {
      console.log(image, loaded, total);
    },
    console: bytes => terminal.write(bytes),
    error: error => console.error(error),
  },
};

const vm = await RV64.create(options); // created and assembled, but stopped
const unsubscribe = vm.on("stop", ({ reason }) => console.log(reason));
await vm.start();
vm.console.send("uname -a\n");
await vm.stop();
await vm.reset();
unsubscribe();
await vm.destroy();
```

Embedders that already own a 9P2000.L server can expose it as a live
virtio-9P mount instead of loading a tar-backed filesystem:

```js
boot: {
  mode: "linux-direct",
  kernel,
  cmdline: "console=ttyS0 root=host9p rootfstype=9p rootflags=trans=virtio,version=9p2000.L",
  p9: { tag: "host9p", handle: request => namespace9p(request) },
}
```

The handler may return a `Uint8Array` or a promise. External handlers require
local execution mode; they are intended for hosts such as WANIX that already
run rv64.js inside their own Worker.

The self-contained [WANIX integration](integrations/wanix/README.md) builds an
rv64 VM-driver archive and matching RISC-V Linux namespace without patching a
WANIX checkout.

Execution is local (on the calling thread) by default. Browser applications
that need to keep their UI responsive can opt into a dedicated module Worker
without changing the VM lifecycle or event API:

```js
const vm = await RV64.create({
  ...options,
  execution: { mode: "worker" },
});
```

Worker mode runs Wasm, devices, image downloads, and networking off the main
thread. `running` and `instructions` are cached snapshots in this mode (updated
every 500 ms by default); set `statisticsIntervalMs` to change that interval.
URL and byte image sources work in both modes. A `Response` source is local-mode
only because response bodies cannot be transferred reliably between browsers.
Browser execution slices yield through `MessageChannel`, avoiding the nested
timer clamping that applies to repeated `setTimeout(..., 0)` calls.

Images may be `{ url }`, `Response`, `Uint8Array`, `ArrayBuffer`, or another
array-buffer view. `RV64.create()` resolves all images and emits download
progress before the `ready` event. `start()` runs cooperative execution slices;
calling it repeatedly is harmless. Always call `destroy()` when the VM is no
longer needed so sockets, channels, and listeners are closed.

The supported boot configurations are:

- `linux-direct`: boot a Linux kernel in supervisor mode using rv64.js's SBI.
- `firmware`: boot through an explicitly supplied firmware image.
- `bare-metal`: load an image at an explicit address without Linux devices.

The stable facade intentionally does not expose raw Wasm exports or JIT control
methods. Architecture and emulator tests use the separately named
`RV64Debug` API; applications should not depend on it.

Linux machines use the `fetch` HTTP/HTTPS backend by default. The Alpine image
configures `http_proxy`/`https_proxy` only when that backend is selected,
attaches the NIC at `10.0.2.15`, and trusts the per-VM proxy CA automatically.
`apk update` works directly in Node
and native builds. Browsers can fetch CORS-enabled destinations directly;
Alpine's package CDN currently requires the optional request relay, supplied as
`relayURL` or to the demo as `?relay=wss://your-relay.example`.
Networking can also be selected explicitly:

```js
network: { mode: "fetch" }                            // Linux default
network: { mode: "fetch", relayURL: "wss://…" }      // CORS fallback
network: { mode: "wisp", url: "wisps://…" }          // TCP/UDP relay
network: { mode: "wsproxy", url: "wss://…" }         // layer-2 relay
network: { mode: "inbrowser", channel: "my-lan" }    // browser-local LAN
network: { mode: "external" }                         // advanced frame API
network: { mode: "none" }
```

`fetch` translates guest HTTP requests to the host's `fetch()` API. `wisp`
provides outbound TCP and UDP through a WISP v1 relay. `wsproxy` carries raw
Ethernet frames to a websockproxy-compatible relay. `inbrowser` joins VMs in
the same browser to a local Ethernet segment using `BroadcastChannel` and has
no Internet access.

`vm.network.proxyURL` reports the guest proxy address in `fetch` mode. In
`external` mode, outbound frames arrive through `networkTransmit` and inbound
frames are passed to `vm.network.receive(frame)`.

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

### Browser HTTP relay internals

```sh
node web/http-relay.mjs --port 8090
```

The browser proxy implementation tries `fetch()` first, retaining its zero-infrastructure
path. If a GET or HEAD fails before a response is exposed, an attached HTTP
relay retries it and remembers that origin for later requests. Non-idempotent
requests are never retried automatically. This transport is exercised by the
repository tests and is exposed as `network: { mode: "fetch", relayURL }`. See
[web/HTTP_RELAY.md](web/HTTP_RELAY.md) for the wire protocol and deployment
details.

### Modern Alpine/Linux machine

```sh
nix develop -c tests/virt-smoke/mk-alpine-rootfs.sh target/bench
node tests/alpine-boot.mjs
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
