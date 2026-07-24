#!/usr/bin/env bash
# Turn a Debian i386 rootfs (from mk-debian-rootfs.sh ARCH=i386) into
# v86-bootable artifacts: a stock i386 kernel + an initramfs that runs a
# benchmark. This is the v86 side of the apples-to-apples comparison (SAME
# Debian userland as the riscv64 image).
#
# Why an initramfs and not a disk: v86 has ONLY IDE/ATA disks (no virtio-blk),
# and its bundled buildroot kernel lacks an ATA driver — so a rootfs disk isn't
# reachable. Booting the rootfs AS an initramfs needs no block device at all.
# We use a stock Debian kernel (it boots fine in v86 — SSE2/PAE baseline);
# trixie dropped the standalone i386 kernel, so we pull bookworm's.
#
#   ARCH=i386 nix develop -c tests/vs-v86/mk-debian-rootfs.sh <out>
#   BENCH='/usr/bin/python3 /fib.py' nix develop -c tests/vs-v86/mk-v86-debian.sh <out>
set -euo pipefail
OUT="${1:-$PWD}"
ROOT="$OUT/deb-i386"
KURL="${KURL:-http://deb.debian.org/debian/pool/main/l/linux-signed-i386/linux-image-6.1.0-47-686-pae_6.1.170-3_i386.deb}"
BENCH="${BENCH:-/usr/bin/python3 /fib.py}"
[ -d "$ROOT" ] || { echo "build the rootfs first: ARCH=i386 mk-debian-rootfs.sh $OUT"; exit 1; }

# stock i386 kernel (extract vmlinuz from the .deb)
if [ ! -f "$OUT/vmlinuz-i386" ]; then
    curl -sSL --connect-timeout 20 -o "$OUT/dki386.deb" "$KURL"
    rm -rf "$OUT/ki386"; mkdir -p "$OUT/ki386"
    fakeroot dpkg-deb -x "$OUT/dki386.deb" "$OUT/ki386"
    cp "$(ls "$OUT"/ki386/boot/vmlinuz-* | head -1)" "$OUT/vmlinuz-i386"
fi

# init: mount devtmpfs, wire stdio to the serial console (v86 ttyS0), run the
# benchmark, then stay alive (an initramfs init that exits panics the kernel).
cat > "$ROOT/init" <<EOF
#!/bin/sh
mount -t devtmpfs dev /dev
exec >/dev/ttyS0 2>&1 </dev/ttyS0
mount -t proc proc /proc 2>/dev/null
echo BENCH_READY
$BENCH
echo BENCH_END
while true; do sleep 3600; done
EOF
chmod 755 "$ROOT/init"

# the rootfs as a cpio.gz initramfs (fakeroot so device nodes/ownership survive)
( cd "$ROOT" && find . -print0 | fakeroot cpio --null -o -H newc 2>/dev/null | gzip -1 ) > "$OUT/deb-i386.cpio.gz"
echo "v86 artifacts: $OUT/vmlinuz-i386 + $OUT/deb-i386.cpio.gz  (bench: $BENCH)"
# Boot: bzimage=vmlinuz-i386, initrd=deb-i386.cpio.gz, cmdline="rdinit=/init
# console=ttyS0", memory_size>=1G (see compare-python.mjs).
