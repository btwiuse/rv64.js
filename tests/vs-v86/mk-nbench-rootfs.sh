#!/usr/bin/env bash
# Build nbench (BYTEmark) for riscv64 and bake it into a copy of the buildroot
# rootfs, producing $OUT/root-nbench.bin for tests/vs-v86/nbench.mjs.
#
# Fully reproducible: downloads the nbench source, generates the stubs/patches
# it needs to build freestanding-newlib for riscv64 (no popen, no /proc, a
# pointer-size header, low MINIMUM_SECONDS), builds with the dev-shell newlib
# cross-gcc, and bakes it in with debugfs (no root/mount). Run in the nix shell:
#   nix develop -c tests/vs-v86/mk-nbench-rootfs.sh [outdir]
#
# Env: NBENCH_URL, MINSECONDS (default 2).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
OUT="${1:-$PWD}"; mkdir -p "$OUT"
MINSECONDS="${MINSECONDS:-2}"
URL="${NBENCH_URL:-https://www.math.utah.edu/~mayer/linux/nbench-byte-2.2.3.tar.gz}"
SRC="$OUT/nbench-byte-2.2.3"
ROOTFS_IN="$REPO/web/images/root-riscv64.bin"

command -v riscv64-none-elf-gcc >/dev/null || { echo "run inside: nix develop -c $0"; exit 1; }

# --- fetch + unpack ---
if [ ! -d "$SRC" ]; then
    curl -sSL --connect-timeout 20 -o "$OUT/nbench.tar.gz" "$URL"
    tar -xzf "$OUT/nbench.tar.gz" -C "$OUT"
fi
cd "$SRC"

# --- generated stubs (see git history for why each is needed) ---
echo '#define LONG64' > pointer.h                       # 64-bit target
cat > hwstub.c <<'EOF'
#include <stdio.h>
/* nbench's hardware() reads /proc via popen, absent in newlib — stub it. */
void hardware(const int write_to_file, FILE *global_ofile) { (void)write_to_file; (void)global_ofile; }
EOF
cat > sysinfo.c <<'EOF'
sprintf(buffer,"**System: riscv64 under rv64.js\n"); output_string(buffer);
EOF
cat > sysinfoc.c <<'EOF'
sprintf(buffer,"C compiler          : riscv64-none-elf-gcc (newlib)\n"); output_string(buffer);
sprintf(buffer,"libc                : newlib\n"); output_string(buffer);
EOF

# --- patches: shorter runs; disable NNET (needs a data file, and riscv64 Linux
#     has only openat which newlib doesn't wire, so nbench can't open files) ---
sed -i "s/#define MINIMUM_SECONDS 5/#define MINIMUM_SECONDS ${MINSECONDS}/" nmglobal.h
grep -q 'tests_to_do\[8\]=0' nbench0.c || \
  sed -i 's/\ttests_to_do\[i\]=1;/\ttests_to_do[i]=1;\n\ttests_to_do[8]=0; \/* NNET off: needs a data file *\//' nbench0.c

# --- build + strip ---
riscv64-none-elf-gcc -DLINUX -O2 -static -march=rv64gc -mabi=lp64d \
    -o "$OUT/nbench.rv64" nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c -lm
riscv64-none-elf-strip -s "$OUT/nbench.rv64"

# --- bake into a copy of the rootfs ---
cp "$ROOTFS_IN" "$OUT/root-nbench.bin"
debugfs -w -R "rm /nbench" "$OUT/root-nbench.bin" 2>/dev/null || true
debugfs -w -R "write $OUT/nbench.rv64 nbench" "$OUT/root-nbench.bin" >/dev/null 2>&1
debugfs -w -R "sif /nbench mode 0100755" "$OUT/root-nbench.bin" >/dev/null 2>&1
echo "built $OUT/nbench.rv64 ($(stat -c%s "$OUT/nbench.rv64") bytes) -> baked into $OUT/root-nbench.bin"
