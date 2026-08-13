#!/usr/bin/env bash
# Build the cross-ISA benchmark binaries the scorecard's compile + nbench rows
# need, so both emulators run the SAME source, same compiler commit:
#
#   tcc.i386 / tcc.rv64   tcc @ d9d02c5 (one tree, two targets) — compile bench
#   nbench-fixed.*        fixed-work, matched 32-bit BYTEmark — scored
#   nbench.*              self-timed matched-data BYTEmark — diagnostic
#   nbench-native.*       historical ABI-native-long BYTEmark — diagnostic
#
# All static (musl) via `zig cc`, so no 32-bit-glibc host toolchain and no
# qemu-in-rootfs is needed — the binaries run in any i386/riscv64 Linux guest.
# The one wrinkle: tcc's build runs a generated helper (c2str) on the BUILD
# host, so for the riscv64 target we pre-generate its output with the host cc.
#
#   nix develop -c tests/vs-v86/mk-bench-bins.sh <artifacts-dir>
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-$PWD}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
TCC_URL="${TCC_URL:-https://repo.or.cz/tinycc.git}"
TCC_COMMIT="${TCC_COMMIT:-d9d02c5}"                 # 0.9.28rc mob@d9d02c5
NBENCH_URL="${NBENCH_URL:-https://www.math.utah.edu/~mayer/linux/nbench-byte-2.2.3.tar.gz}"
NBENCH_SHA256="${NBENCH_SHA256:-723dd073f80e9969639eb577d2af4b540fc29716b6eafdac488d8f5aed9101ac}"

# zig is our portable cross-compiler; pull it from the flake-pinned nixpkgs.
ZIG="$(nix shell --inputs-from "$HERE/../.." nixpkgs#zig -c sh -c 'command -v zig')"
i386cc() { "$ZIG" cc -target x86-linux-musl "$@"; }
rv64cc() { "$ZIG" cc -target riscv64-linux-musl "$@"; }

# --- tinycc (one checkout, built twice) ---
SRC="$OUT/tinycc"
[ -d "$SRC/.git" ] || git clone "$TCC_URL" "$SRC"
git -C "$SRC" fetch --all -q 2>/dev/null || true
git -C "$SRC" checkout -q "$TCC_COMMIT"

build_tcc() { # <cc-fn> <cpu> <out>
  local ccfn="$1" cpu="$2" out="$3"
  ( cd "$SRC" && make distclean >/dev/null 2>&1 || true )
  cat > "$OUT/.zcc-$cpu" <<EOF
#!/bin/sh
exec "$ZIG" cc -target $([ "$cpu" = i386 ] && echo x86 || echo riscv64)-linux-musl "\$@"
EOF
  chmod +x "$OUT/.zcc-$cpu"
  ( cd "$SRC" && CC="$OUT/.zcc-$cpu" ./configure --cpu="$cpu" --cc="$OUT/.zcc-$cpu" \
      --extra-cflags="-O2 -static" --extra-ldflags="-static" >/dev/null )
  # riscv64: c2str helper would be cross-built and can't run on the host — make
  # its output with the host cc first so `make` doesn't try to run a foreign one.
  if [ "$cpu" = riscv64 ]; then
    ( cd "$SRC" && cc -DC2STR conftest.c -o c2str.exe && ./c2str.exe include/tccdefs.h tccdefs_.h )
    touch "$SRC/tccdefs_.h"
  fi
  ( cd "$SRC" && make tcc >/dev/null )
  cp "$SRC/tcc" "$out"
  echo "  $out: $(file -b "$out" | cut -d, -f1-2)"
}
echo "== tcc =="
build_tcc i386cc    i386    "$OUT/tcc.i386"
build_tcc rv64cc    riscv64 "$OUT/tcc.rv64"

# --- nbench i386 (same patches as the rv64 newlib build in mk-nbench-rootfs.sh) ---
echo "== nbench.i386 =="
if [ ! -f "$OUT/nbench.tar.gz" ]; then
  curl -sSL --connect-timeout 20 -o "$OUT/nbench.tar.gz" "$NBENCH_URL"
fi
printf '%s  %s\n' "$NBENCH_SHA256" "$OUT/nbench.tar.gz" | sha256sum -c -
NB_SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/rv64-nbench-build.XXXXXX")"
trap 'rm -rf "$NB_SCRATCH"' EXIT
tar -xzf "$OUT/nbench.tar.gz" -C "$NB_SCRATCH"
NB="$NB_SCRATCH/nbench-byte-2.2.3"
cd "$NB"
patch -p1 < "$HERE/nbench-fixed-data32.patch"
patch -p1 < "$HERE/nbench-fixed-work.patch"
for parameter in DONUMSORT DOSTRINGSORT DOBITFIELD DOEMF DOFOUR DOASSIGN DOIDEA DOHUFF; do
  grep -q "\"$parameter\"" nbench0.h || {
    echo "BYTEmark command parameter is missing: $parameter" >&2
    exit 2
  }
done
: > pointer.h                                       # 32-bit target: long is 32-bit
cat > hwstub.c <<'X'
#include <stdio.h>
void hardware(const int w, FILE *f){(void)w;(void)f;}
X
cat > sysinfo.c  <<'X'
sprintf(buffer,"**System: i386 under v86\n"); output_string(buffer);
X
cat > sysinfoc.c <<'X'
sprintf(buffer,"C compiler          : zig cc (i386 musl)\n"); output_string(buffer);
sprintf(buffer,"libc                : musl\n"); output_string(buffer);
X
sed -i "s/#define MINIMUM_SECONDS 5/#define MINIMUM_SECONDS ${MINSECONDS:-2}/" nmglobal.h
grep -q 'tests_to_do\[8\]=0' nbench0.c || \
  sed -i 's/\ttests_to_do\[i\]=1;/\ttests_to_do[i]=1;\n\ttests_to_do[8]=0;/' nbench0.c
i386cc -DLINUX -O2 -static -o "$OUT/nbench-native.i386" \
   nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c
echo "  $OUT/nbench-native.i386: $(file -b "$OUT/nbench-native.i386" | cut -d, -f1-2)"
i386cc -DLINUX -DSCORECARD_FIXED_DATA32 -O2 -static -o "$OUT/nbench.i386" \
   nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c
echo "  $OUT/nbench.i386: $(file -b "$OUT/nbench.i386" | cut -d, -f1-2)"
i386cc -DLINUX -DSCORECARD_FIXED_DATA32 -DSCORECARD_FIXED_WORK -O2 -static \
   -o "$OUT/nbench-fixed.i386" \
   nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c
echo "  $OUT/nbench-fixed.i386: $(file -b "$OUT/nbench-fixed.i386" | cut -d, -f1-2)"

# --- nbench riscv64 (musl, SAME libc family as the i386 side — the old newlib
# build's generic byte-loop memmove made STRING SORT ~5x slower than the code
# deserves; fastmem.c is the ISA-fair counterpart of musl's i386 asm strings) ---
echo "== nbench.rv64 (musl + fastmem) =="
cd "$NB"
echo '#define LONG64' > pointer.h
cat > sysinfo.c  <<'X'
sprintf(buffer,"**System: riscv64 under rv64.js\n"); output_string(buffer);
X
cat > sysinfoc.c <<'X'
sprintf(buffer,"C compiler          : zig cc (riscv64 musl)\n"); output_string(buffer);
sprintf(buffer,"libc                : musl\n"); output_string(buffer);
X
cp "$HERE/nbench-extras/fastmem.c" fastmem.c
rv64cc -DLINUX -O2 -static -fno-builtin-memmove -fno-builtin-memcpy -o "$OUT/nbench-native.rv64" \
   nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c fastmem.c
echo "  $OUT/nbench-native.rv64: $(file -b "$OUT/nbench-native.rv64" | cut -d, -f1-2)"
rv64cc -DLINUX -DSCORECARD_FIXED_DATA32 -O2 -static -fno-builtin-memmove -fno-builtin-memcpy -o "$OUT/nbench.rv64" \
   nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c fastmem.c
echo "  $OUT/nbench.rv64: $(file -b "$OUT/nbench.rv64" | cut -d, -f1-2)"
rv64cc -DLINUX -DSCORECARD_FIXED_DATA32 -DSCORECARD_FIXED_WORK -O2 -static \
   -fno-builtin-memmove -fno-builtin-memcpy -o "$OUT/nbench-fixed.rv64" \
   nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c fastmem.c
echo "  $OUT/nbench-fixed.rv64: $(file -b "$OUT/nbench-fixed.rv64" | cut -d, -f1-2)"
install -m 0644 "$HERE/nbench-workload-contract-v3.json" "$OUT/nbench-workload-contract.json"
install -m 0644 "$HERE/nbench-fixed-data32.patch" "$OUT/nbench-fixed-data32.patch"
install -m 0644 "$HERE/nbench-fixed-work.patch" "$OUT/nbench-fixed-work.patch"
install -m 0644 "$HERE/nbench-extras/fastmem.c" "$OUT/nbench-rv64-fastmem.c"
rv64cc -O2 -static -s -o "$OUT/assignment-repro.rv64" \
   "$HERE/nbench-extras/assignment-repro.c"
echo "  $OUT/assignment-repro.rv64: $(file -b "$OUT/assignment-repro.rv64" | cut -d, -f1-2)"
rv64cc -O2 -static -s -fno-builtin-memmove -fno-builtin-memcpy \
   -o "$OUT/fastmem-selftest.rv64" \
   "$HERE/nbench-extras/fastmem-selftest.c" "$HERE/nbench-extras/fastmem.c"
echo "  $OUT/fastmem-selftest.rv64: $(file -b "$OUT/fastmem-selftest.rv64" | cut -d, -f1-2)"
echo "done: tcc.*, scored nbench-fixed.*, diagnostics, and fastmem-selftest.rv64 in $OUT"
