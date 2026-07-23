#!/bin/sh
# Build a guest disk image for the compilation benchmark: a copy of the
# TinyEMU root ext2, grown and with the tcc compiler + C source workloads
# injected via debugfs (no mount needed — works on WSL2).
#
# Usage: mkimage.sh <out.img> <tcc-binary> <file:guestpath>...
set -eu
OUT="$1"; TCC="$2"; shift 2
SRC="${ROOTFS:-web/images/root-riscv64.bin}"
SIZE="${IMG_SIZE:-128M}"

cp "$SRC" "$OUT"
truncate -s "$SIZE" "$OUT"
e2fsck -fy "$OUT" >/dev/null 2>&1 || true
resize2fs "$OUT" >/dev/null 2>&1
# inject the compiler
debugfs -w -R "rm /tcc" "$OUT" >/dev/null 2>&1 || true
debugfs -w -R "write $TCC /tcc" "$OUT" >/dev/null 2>&1
debugfs -w -R "sif /tcc mode 0100755" "$OUT" >/dev/null 2>&1
# inject workload files (host:guest pairs)
for pair in "$@"; do
    host="${pair%%:*}"; guest="${pair#*:}"
    debugfs -w -R "rm $guest" "$OUT" >/dev/null 2>&1 || true
    debugfs -w -R "write $host $guest" "$OUT" >/dev/null 2>&1
done
echo "image $OUT ready ($(du -h "$OUT" | cut -f1))"
