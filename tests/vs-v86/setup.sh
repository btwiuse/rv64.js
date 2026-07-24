#!/usr/bin/env bash
# One-command setup for the rv64.js-vs-v86 benchmark suite: builds the wasm, the
# benchmark kernels, and the nbench rootfs (and optionally the Debian rootfs)
# into a single artifacts dir, ready for the compare*.mjs / nbench.mjs /
# deb-python.mjs harnesses. Idempotent. Run in the nix dev shell:
#
#   nix develop -c tests/vs-v86/setup.sh [outdir]      # + DEBIAN=1 for python
#   SC=<outdir> nix develop -c node tests/vs-v86/compare-sys.mjs
#
# The v86 side (compare*.mjs, nbench i386, ...) additionally needs a built
# copy/v86 checkout at <outdir>/v86 — see README "Reproducing".
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
OUT="${1:-$REPO/target/bench}"; mkdir -p "$OUT"

echo "== 1/3 building rv64-wasm =="
( cd "$REPO" && cargo build --release -p rv64-wasm --target wasm32-unknown-unknown )

echo "== 2/3 benchmark kernels =="
"$HERE/build-kernels.sh" "$OUT"

echo "== 3/3 nbench rootfs =="
"$HERE/mk-nbench-rootfs.sh" "$OUT"

if [ "${DEBIAN:-0}" = 1 ]; then
    echo "== + Debian riscv64 rootfs (python, arch-python) =="
    "$HERE/mk-debian-rootfs.sh" "$OUT"
    echo "== + Debian i386 rootfs + v86 kernel/initramfs (apples-to-apples python) =="
    ARCH=i386 "$HERE/mk-debian-rootfs.sh" "$OUT"
    "$HERE/mk-v86-debian.sh" "$OUT"
fi

cat <<EOF

Setup complete in: $OUT
Run benchmarks (add a built copy/v86 at $OUT/v86 for the v86 side):
  SC=$OUT nix develop -c node tests/vs-v86/compare-sys.mjs   # system-mode alu/mixed
  SC=$OUT nix develop -c node tests/vs-v86/compare-boot.mjs  # boot time
  SC=$OUT nix develop -c node tests/vs-v86/nbench.mjs        # BYTEmark
$([ "${DEBIAN:-0}" = 1 ] && echo "  SC=$OUT nix develop -c node tests/vs-v86/deb-python.mjs   # python fib(30)")
EOF
