#!/usr/bin/env bash
# One-command setup for the rv64.js-vs-v86 benchmark suite: builds the wasm, the
# benchmark kernels, and the nbench rootfs (and optionally the Debian rootfs)
# into a single artifacts dir, ready for the compare*.mjs / nbench.mjs /
# deb-python.mjs harnesses. Idempotent. Run in the nix dev shell:
#
#   nix develop -c tests/vs-v86/setup.sh [outdir]      # + DEBIAN=1 for python
#   ARTIFACTS=<outdir> nix develop -c node tests/vs-v86/compare-sys.mjs
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

echo "== 3/3 nbench rootfs (rv64 BYTEmark) =="
"$HERE/mk-nbench-rootfs.sh" "$OUT"

echo "== + cross-ISA benchmark binaries (compile + v86 nbench) =="
"$HERE/mk-bench-bins.sh" "$OUT"                       # tcc.i386/tcc.rv64/nbench.i386
python3 "$HERE/../compile-bench/gen_c.py" "${NFUNCS:-2000}" > "$OUT/w.c"
echo "== + rv64 compile image (tcc + w.c) =="
IMG_SIZE=192M "$HERE/../compile-bench/mkimage.sh" "$OUT/cc-bench.img" "$OUT/tcc.rv64" "$OUT/w.c:/w.c" >/dev/null
echo "  $OUT/cc-bench.img"

if [ "${DEBIAN:-0}" = 1 ]; then
    echo "== + Debian riscv64 rootfs (python, arch-python) =="
    "$HERE/mk-debian-rootfs.sh" "$OUT"
    echo "== + Debian i386 rootfs + v86 kernel/initramfs (apples-to-apples python) =="
    ARCH=i386 "$HERE/mk-debian-rootfs.sh" "$OUT"
    "$HERE/mk-v86-debian.sh" "$OUT"
    echo "== + v86 bench initramfs (python + compile + nbench, one image) =="
    "$HERE/mk-v86-bench.sh" "$OUT"
fi

cat <<EOF

Setup complete in: $OUT
Run benchmarks (add a built copy/v86 at $OUT/v86 for the v86 side):
  ARTIFACTS=$OUT nix develop -c node tests/vs-v86/compare-sys.mjs   # system-mode alu/mixed
  ARTIFACTS=$OUT nix develop -c node tests/vs-v86/compare-boot.mjs  # boot time
  ARTIFACTS=$OUT nix develop -c node tests/vs-v86/nbench.mjs        # BYTEmark
$([ "${DEBIAN:-0}" = 1 ] && echo "  ARTIFACTS=$OUT nix develop -c node tests/vs-v86/deb-python.mjs   # python fib(30)")
EOF
