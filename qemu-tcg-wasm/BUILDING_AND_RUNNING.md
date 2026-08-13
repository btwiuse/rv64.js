# QEMU wasm64 TCG build and run notes

## Result

This experiment successfully builds and starts a QEMU system emulator whose
host is wasm64 and whose native TCG backend is `tcg/wasm64.c`. The checked-out
source is Kohei Tokunaga's `wasm64-tcg-b` branch at commit
`8f1406ba3307a10c58be24a8ff00ab6a5d3b6169`. That commit corresponds to the
33-patch v4 series posted to QEMU development on January 26, 2026.

This is an upstream-submitted implementation, not code merged into the official
QEMU master branch. That status was checked on August 6, 2026 against QEMU
master commit `3e3ccab106f879b1512f8e0d51a827dd4de30e22`, whose `tcg/` tree has no
wasm64 backend. Official QEMU can also be compiled as a wasm64 program, but
without this series it uses TCI, the TCG interpreter. The patch adds a real
wasm64 TCG code generator: cold translation blocks initially use TCI, while hot
blocks are compiled into WebAssembly modules and instantiated at runtime.

Source references:

- v4 QEMU patch-series cover letter:
  <https://lists.gnu.org/archive/html/qemu-arm/2026-01/msg00987.html>
- Author's exact QEMU branch:
  <https://gitlab.com/ktock/qemu/-/tree/wasm64-tcg-b>
- Official QEMU master TCG tree at the status-check commit:
  <https://gitlab.com/qemu-project/qemu/-/tree/3e3ccab106f879b1512f8e0d51a827dd4de30e22/tcg>
- Author's Node/browser example:
  <https://github.com/ktock/qemu-wasm-sample/tree/tcgdev64>

## What is in this directory

- `qemu/` is the patch author's QEMU source checkout.
- `flake.nix` and `flake.lock` pin all system tools and dependency sources.
- `scripts/build-deps.sh` cross-compiles QEMU's C-library dependencies.
- `scripts/build-qemu.sh` configures, builds, and collects QEMU artifacts.
- `scripts/verify.sh` proves that the wasm64 TCG backend, rather than TCI, was
  selected and inspects the output WebAssembly module.
- `scripts/run.sh` starts the generated emulator with Node.js.
- `scripts/smoke-run.sh` performs a bounded `--version` runtime test.
- `scripts/package-assets.sh` packages a kernel, disk image, and firmware into
  Emscripten's virtual filesystem.
- `.deps/wasm64/`, `.work/`, `build-*`, and `dist/` are generated and ignored.

The flake pins Emscripten 4.0.23 and Node.js 24. It also pins the versions used
by the patch author's build environment: GLib 2.84.0, zlib 1.3.1, libffi 3.5.2,
Pixman 0.44.2, and PCRE2 10.44. Those libraries are fetched through Nix and then
cross-compiled to static wasm64 archives by `build-deps.sh`. No host-installed
development libraries are used.

## Compile

Run these commands from this directory. `path:.` is intentional: this directory
is currently untracked inside a larger Git worktree, so it forces Nix to use
this directory itself as the flake source.

```sh
cd /home/darren/src/jit/qemu-tcg-wasm
nix develop path:.
scripts/build-qemu.sh
scripts/verify.sh
```

Or run the same build without entering a persistent shell:

```sh
cd /home/darren/src/jit/qemu-tcg-wasm
nix develop path:. -c scripts/build-qemu.sh
nix develop path:. -c scripts/verify.sh
```

The default target is `x86_64-softmmu`. Its deliverables are:

```text
dist/x86_64/qemu-system-x86_64.js
dist/x86_64/qemu-system-x86_64.wasm
```

The dependency stamps under `.deps/wasm64/` and Ninja's build state make later
builds incremental. To use a different supported system-emulation target, set
`QEMU_TARGET`. For example:

```sh
QEMU_TARGET=riscv64-softmmu scripts/build-qemu.sh
QEMU_TARGET=riscv64 scripts/verify.sh
```

The first successful x86-64 build reported QEMU 10.2.50, `host CPU: wasm64`,
`TCG backend: native (wasm64)`, and compiled `tcg_wasm64.c.o`. The generated
configuration contains `HOST_WASM64=1` and `CONFIG_TCG=1`, with
`CONFIG_TCG_INTERPRETER` disabled. The output sizes from that build were about
308 KiB for the JavaScript loader and 17 MiB for the WebAssembly module.

## Run a smoke test

The build is a WebAssembly module plus Emscripten JavaScript glue, not a native
ELF executable. Run it through the QEMU-provided Node.js adapter:

```sh
nix develop path:. -c scripts/smoke-run.sh
```

The verified result is:

```text
QEMU emulator version 10.2.50
Copyright (c) 2003-2026 Fabrice Bellard and the QEMU Project developers
```

The helper treats a timeout after that output as success. There is a known
Emscripten atexit problem in this QEMU build: commands such as `--version` and
the QEMU monitor's `quit` can finish their work without causing Node to exit.
Use Ctrl-C, SIGINT, or SIGTERM. To invoke QEMU directly and stop it yourself:

```sh
nix develop path:.
scripts/run.sh --version
# Press Ctrl-C after the version is printed.
```

Node.js 24 or newer is needed for this wasm64 output; the flake supplies it.

## Run a Linux guest

QEMU needs a guest kernel, root filesystem/disk image, and firmware files. They
are not downloaded by this experiment. Put compatible x86-64 assets into a
staging directory called `pack` and include QEMU's PC firmware:

```sh
cd /home/darren/src/jit/qemu-tcg-wasm
nix develop path:.

mkdir -p pack
cp /path/to/x86_64-kernel.img pack/kernel.img
cp /path/to/rootfs.bin pack/rootfs.bin
cp -a qemu/pc-bios/. pack/

scripts/package-assets.sh pack
scripts/run.sh \
  -nographic -m 512M \
  -L /pack \
  -drive if=virtio,format=raw,file=/pack/rootfs.bin \
  -kernel /pack/kernel.img \
  -append "earlyprintk=ttyS0 console=ttyS0 root=/dev/vda loglevel=7"
```

`package-assets.sh` creates `dist/x86_64/qemu-system-x86_64.data` and
`dist/x86_64/load.mjs`. It maps the staging directory to `/pack` in
Emscripten's virtual filesystem. `run.sh` automatically loads `load.mjs` when
it exists. A different loader can be selected with `QEMU_PRELOAD=/path/file`.

The author's `qemu-wasm-sample` `tcgdev64` branch is a useful source for a small
BusyBox root filesystem, Linux kernel build, and browser terminal example.

## Important runtime details

- This build uses full wasm64 (`-sMEMORY64=1`), not Emscripten's wasm32-compatible
  `MEMORY64=2` lowering. The runtime must implement WebAssembly memory64.
- QEMU's Emscripten link configuration requests 2 GiB of initial WebAssembly
  memory and pthreads. Browser deployment therefore also needs
  `SharedArrayBuffer` and cross-origin isolation (COOP/COEP headers).
- `scripts/run.sh` is the terminal/Node path. A browser UI is not included here;
  use the author's sample application as the integration reference.
- Generated assets and builds are ignored by the surrounding Git repository.
  To force a fresh QEMU configuration, remove only the applicable `build-*`
  directory. To rebuild cross libraries, remove `.deps/wasm64` and `.work`.
