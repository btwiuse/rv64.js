# WANIX rv64.js backend

This integration mirrors WANIX's `v86/` driver layout without modifying the
WANIX source tree. `make bundle` produces
`rv64.tgz`, containing `rv64-vm.wasm` (the WANIX adapter) and
`rv64_wasm.wasm` (the emulator). Bind the archive at `#vm/rv64` and start it
with `<wanix-vm type="rv64">`.

The adapter boots `boot/Image` directly, mounts the current WANIX namespace as
the guest's `host9p` root, routes the interactive terminal through `ttyS0`, and
uses the independent `hvc0` virtio console for WANIX host export.

From the rv64.js checkout, build both runtime archives with:

```sh
make -C integrations/wanix bundle linux
```

`build-linux-bundle.sh` checks out the pinned WANIX revision in a temporary
directory and cross-compiles its `wexec` and `hostexport` guest utilities for
RISC-V. The small build patch only teaches WANIX's platform-stat helper about
Go's `linux/riscv64` `Stat_t`; it does not require a WANIX checkout to be
modified or committed.

The adapter enables rv64.js's browser `fetch` network backend. It uses the
hosted Cloudflare Worker at
`wss://rv64-http-relay.darren-e4d.workers.dev/relay` as the default CORS
fallback; set `RV64_RELAY_URL` in the WANIX VM environment to override it.
The guest is configured as `10.0.2.15`, uses `10.0.2.2:8080` as its HTTP(S)
proxy, and installs the per-VM proxy CA from the `rv64-proxy` 9P export so
HTTPS clients such as `apk` can use the fetch backend.

Serve the two files from `dist/` and bind them from otherwise stock WANIX:

```html
<wanix-bind dst="." type="archive" src="/assets/wanix-linux-rv64.tgz"></wanix-bind>
<wanix-bind dst="#vm/rv64" type="archive" src="/assets/rv64.tgz"></wanix-bind>
<wanix-vm type="rv64" export="hvc0" mem="1G" term start></wanix-vm>
```

## Why rv64.js 9P needed an adapter

The existing rv64.js 9P implementation and v86's WANIX integration use the
same guest-visible protocol: 9P2000.L over virtio. They originally differed at
the host boundary. rv64.js contained a synchronous Rust 9P server backed by a
tar-loaded `MemFs`; v86 exposes raw request bytes through an asynchronous
`handle9p(request, callback)` hook. WANIX needs the latter because WANIX itself
owns the live namespace and may answer requests asynchronously.

rv64.js now supports both modes. Its original in-process server remains useful
for standalone and native use, while the external backend holds the guest
virtqueue entry pending, forwards the raw request to WANIX, and completes that
entry when WANIX returns the tagged reply.

## Side-by-side WANIX demo

`v86-rv64-side-by-side.html` is the tested two-VM demo. Copy it into WANIX's
`examples/` directory after building the normal v86 assets and the rv64.js
bundles. Both guests bind the same WANIX namespace at `/shared`; the page also
installs `wtop`, `wrepeat`, and a small Python benchmark in that namespace.

The demo exercises a few WANIX runtime capabilities that are not present at
the pinned upstream revision. Apply the included patches from the root of a
WANIX checkout before rebuilding WANIX:

```sh
git apply /path/to/rv64.js/integrations/wanix/wanix-host-js.patch
git apply /path/to/rv64.js/integrations/wanix/wanix-x86-python.patch
```

`wanix-host-js.patch` makes JavaScript tasks use their task namespace, passes
their environment into the worker, exposes their captured standard output and
error as `/task/<pid>/fd/{1,2}`, and terminates the worker when the task exits.
`wanix-x86-python.patch` adds Python to the stock x86 guest image so the same
benchmark can run in both guests. The RISC-V guest builder already installs
Python.
