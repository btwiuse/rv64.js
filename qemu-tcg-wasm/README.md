# QEMU TCG wasm64 experiment

This directory builds Kohei Tokunaga's proposed wasm64 TCG backend for QEMU.
The backend was posted upstream as `[PATCH v4 00/33] wasm: Add Wasm TCG
backend based on wasm64` on 2026-01-26. As checked on 2026-08-06, it is not
merged into QEMU master.

The checked-out source is the patch author's `wasm64-tcg-b` branch at commit
`8f1406ba3307a10c58be24a8ff00ab6a5d3b6169`, corresponding to that v4 series.
It emits TCI for cold translation blocks, then emits and instantiates Wasm for
hot blocks.

## Build

All host tools and source archives for cross dependencies come from the pinned
Nix flake. The dependency versions match the upstream patch's build container:
Emscripten 4.0.23, GLib 2.84.0, zlib 1.3.1, libffi 3.5.2, and Pixman 0.44.2.

```sh
nix develop path:.
scripts/build-qemu.sh
scripts/verify.sh
scripts/smoke-run.sh
```

The default guest target is `x86_64-softmmu`. To build the RISC-V system
emulator instead:

```sh
QEMU_TARGET=riscv64-softmmu scripts/build-qemu.sh
QEMU_TARGET=riscv64 scripts/verify.sh
```

Outputs are copied to `dist/<guest-architecture>/`. Intermediate cross-built
libraries remain in `.deps/wasm64`, so repeat builds do not rebuild them.

See [BUILDING_AND_RUNNING.md](BUILDING_AND_RUNNING.md) for the complete build
record, commands for running QEMU under Node.js, guest-asset packaging, and
current limitations.

## Source provenance

- QEMU mailing-list series:
  <https://lists.gnu.org/archive/html/qemu-arm/2026-01/msg00987.html>
- Author's source branch:
  <https://gitlab.com/ktock/qemu/-/tree/wasm64-tcg-b>
- Browser/Node sample:
  <https://github.com/ktock/qemu-wasm-sample/tree/tcgdev64>

Upstream QEMU master can itself be compiled with Emscripten, but currently uses
the TCG interpreter (TCI). QEMU master commit
`3e3ccab106f879b1512f8e0d51a827dd4de30e22`, inspected on 2026-08-06, has no
wasm64 backend under `tcg/`. The `tcg/wasm64.c` implementation in this checkout
is the separate dynamic Wasm code-generating backend being evaluated here.
