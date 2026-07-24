#!/usr/bin/env bash
# Turn the Debian i386 rootfs (mk-debian-rootfs.sh ARCH=i386) into a single v86
# initramfs that runs python / tcc-compile / nbench, chosen by a `bench=` kernel
# cmdline token. Bakes in the prebuilt i386 tcc + nbench (mk-bench-bins.sh) and
# the shared compile source w.c (same file the rv64 side compiles).
#
#   nix develop -c tests/vs-v86/mk-v86-bench.sh <artifacts-dir>
# Produces $OUT/deb-i386-bench.cpio.gz (boot with vmlinuz-i386 from mk-v86-debian.sh).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-$PWD}"; ROOT="$OUT/deb-i386"
[ -d "$ROOT" ] || { echo "need $ROOT — run: ARCH=i386 mk-debian-rootfs.sh $OUT"; exit 1; }
for b in tcc.i386 nbench.i386; do [ -f "$OUT/$b" ] || { echo "need $OUT/$b — run mk-bench-bins.sh $OUT"; exit 1; }; done
[ -f "$OUT/w.c" ] || python3 "$HERE/../compile-bench/gen_c.py" "${NFUNCS:-2000}" > "$OUT/w.c"

cp "$OUT/tcc.i386"    "$ROOT/tcc"    && chmod 755 "$ROOT/tcc"
cp "$OUT/nbench.i386" "$ROOT/nbench" && chmod 755 "$ROOT/nbench"
cp "$OUT/w.c"         "$ROOT/w.c"

cat > "$ROOT/init" <<'INIT'
#!/bin/sh
mount -t devtmpfs dev /dev
exec >/dev/ttyS0 2>&1 </dev/ttyS0
mount -t proc proc /proc 2>/dev/null
B=$(sed 's/ /\n/g' /proc/cmdline | grep '^bench=' | cut -d= -f2)
echo BENCH_READY
case "$B" in
  python) /usr/bin/python3 /fib.py ;;
  tcc)    echo RUN_START; /tcc -c /w.c -o /w.o && md5sum /w.o; echo RUN_DONE ;;
  nbench) cd /; echo RUN_START; ./nbench; echo RUN_DONE ;;
  *)      echo "no bench selected (need bench=python|tcc|nbench)" ;;
esac
echo BENCH_END
while true; do sleep 3600; done
INIT
chmod 755 "$ROOT/init"

( cd "$ROOT" && find . -print0 | fakeroot cpio --null -o -H newc 2>/dev/null | gzip -1 ) > "$OUT/deb-i386-bench.cpio.gz"
echo "v86 bench initramfs: $OUT/deb-i386-bench.cpio.gz ($(du -h "$OUT/deb-i386-bench.cpio.gz" | cut -f1))"
