#!/bin/sh
# Tiered three-way compilation benchmark driver: native (host tcc) vs
# emulated-interpreter vs emulated-JIT, compiling a real riscv64 C
# translation unit with tcc inside booted Linux.
#
#   tests/compile-bench/bench.sh <tier>   (tier: quick | medium | soak)
#
# Requires (all from `nix develop`): host tcc, riscv64 static tcc, node,
# mke2fs/debugfs, python3. The riscv64 tcc path is read from
# $RISCV_TCC or nix-built on demand.
set -eu
cd "$(dirname "$0")/../.."
TIER="${1:-quick}"
case "$TIER" in
  quick)  N=300 ; REPS=3 ;;
  medium) N=2000; REPS=3 ;;
  soak)   N=12000; REPS=2 ;;
  *) echo "tier must be quick|medium|soak"; exit 2 ;;
esac

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# 1. host tcc (native baseline) + riscv64 tcc (guest)
HOST_TCC="${HOST_TCC:-$(nix build --no-link --print-out-paths 'nixpkgs#tinycc' 2>/dev/null | grep -v -- -man | head -1)/bin/tcc}"
RISCV_TCC="${RISCV_TCC:-$(nix build --no-link --print-out-paths 'nixpkgs#pkgsCross.riscv64.pkgsStatic.tinycc' 2>/dev/null)/bin/tcc}"
[ -x "$HOST_TCC" ] || { echo "no host tcc"; exit 1; }
[ -x "$RISCV_TCC" ] || { echo "no riscv tcc"; exit 1; }

# 2. generate the translation unit
python3 tests/compile-bench/gen_c.py "$N" > "$WORK/w.c"
echo "tier=$TIER  ($(wc -l < "$WORK/w.c") lines, $(wc -c < "$WORK/w.c") bytes)"

# 3. native timing (host tcc), best of 3
best=99999
for i in 1 2 3; do
  t0=$(date +%s.%N); "$HOST_TCC" -c "$WORK/w.c" -o "$WORK/w.o" 2>/dev/null; t1=$(date +%s.%N)
  ms=$(echo "($t1 - $t0) * 1000" | bc | cut -d. -f1)
  [ "$ms" -lt "$best" ] && best=$ms
done
echo "native (host tcc): ${best} ms"

# 4. build guest image with tcc + source
IMG_SIZE=192M tests/compile-bench/mkimage.sh "$WORK/disk.img" "$RISCV_TCC" "$WORK/w.c:/w.c" >/dev/null
echo "guest image built; running emulated three-way ($REPS reps)..."

# 5. emulated interp vs jit (correctness = object md5 identical)
node tests/compile-bench/run.mjs "$WORK/disk.img" /w.c "$best" "$REPS"
