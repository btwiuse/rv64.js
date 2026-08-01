#!/usr/bin/env bash
# Build the cross-ISA benchmark binaries the scorecard's compile + nbench rows
# need, so both emulators run the SAME source, same compiler commit:
#
#   tcc.i386 / tcc.rv64   tcc @ d9d02c5 (one tree, two targets) — compile bench
#   nbench.i386           BYTEmark for i386 — v86 nbench (rv64 uses mk-nbench)
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
NB="$OUT/nbench-byte-2.2.3"
if [ ! -d "$NB" ]; then curl -sSL --connect-timeout 20 -o "$OUT/nbench.tar.gz" "$NBENCH_URL"; tar -xzf "$OUT/nbench.tar.gz" -C "$OUT"; fi
cd "$NB"
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
i386cc -DLINUX -O2 -static -o "$OUT/nbench.i386" \
   nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c
echo "  $OUT/nbench.i386: $(file -b "$OUT/nbench.i386" | cut -d, -f1-2)"

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
rv64cc -DLINUX -O2 -static -fno-builtin-memmove -fno-builtin-memcpy -o "$OUT/nbench.rv64" \
   nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c fastmem.c
echo "  $OUT/nbench.rv64: $(file -b "$OUT/nbench.rv64" | cut -d, -f1-2)"
rv64cc -O2 -static -s -o "$OUT/assignment-repro.rv64" \
   "$HERE/nbench-extras/assignment-repro.c"
echo "  $OUT/assignment-repro.rv64: $(file -b "$OUT/assignment-repro.rv64" | cut -d, -f1-2)"
# bake into the buildroot rootfs image the nbench harness boots
cp "$HERE/../../web/images/root-riscv64.bin" "$OUT/root-nbench.bin"
debugfs -w -R "rm /C" "$OUT/root-nbench.bin" >/dev/null 2>&1 || true
debugfs -w -R "symlink C /tmp/C" "$OUT/root-nbench.bin" >/dev/null
debugfs -w -R "rm /nbench" "$OUT/root-nbench.bin" >/dev/null 2>&1 || true
debugfs -w -R "write $OUT/nbench.rv64 nbench" "$OUT/root-nbench.bin" >/dev/null 2>&1
debugfs -w -R "sif /nbench mode 0100755" "$OUT/root-nbench.bin" >/dev/null 2>&1
cp "$OUT/root-nbench.bin" "$OUT/root-assignment-repro.bin"
debugfs -w -R "rm /nbench" "$OUT/root-assignment-repro.bin" >/dev/null
debugfs -w -R "write $OUT/assignment-repro.rv64 assignment-repro" "$OUT/root-assignment-repro.bin" >/dev/null
debugfs -w -R "sif /assignment-repro mode 0100755" "$OUT/root-assignment-repro.bin" >/dev/null
echo "  baked $OUT/root-nbench.bin"

echo "done: tcc.i386 tcc.rv64 nbench.i386 nbench.rv64 assignment-repro.rv64 root-nbench.bin root-assignment-repro.bin in $OUT"
