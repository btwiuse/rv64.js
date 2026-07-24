#!/usr/bin/env bash
# Assemble a minimal Debian riscv64 rootfs (with python3) into an ext4 image
# that boots under our JIT-capable system machine (bbl64 + kernel-riscv64).
#
# No root and no binfmt needed: fakeroot debootstrap --foreign downloads and
# first-stage-unpacks on the host; we then dpkg-deb -x the --include packages
# (which --foreign leaves in the apt cache for the second stage we can't run),
# and mke2fs -d assembles the directory into an image. Runs entirely in the nix
# dev shell.  Usage:  nix develop -c tests/vs-v86/mk-debian-rootfs.sh <outdir>
set -euo pipefail
OUT="${1:-$PWD}"
ROOT="$OUT/deb-rv"
SUITE="${SUITE:-trixie}"                       # Debian 13: riscv64 is official
MIRROR="${MIRROR:-http://deb.debian.org/debian}"

rm -rf "$ROOT"
fakeroot debootstrap --foreign --arch=riscv64 --variant=minbase \
    --include=python3,libc-bin --no-check-gpg "$SUITE" "$ROOT" "$MIRROR"

# Unpack the downloaded-but-unconfigured packages (python3 + deps). A bare
# interpreter needs no maintainer-script configuration.
for d in "$ROOT"/var/cache/apt/archives/*.deb; do fakeroot dpkg-deb -x "$d" "$ROOT/"; done

# Minimal init: mount pseudo-filesystems then exec a shell (no systemd).
cat > "$ROOT/binit.sh" <<'EOF'
#!/bin/sh
mount -t proc proc /proc 2>/dev/null
mount -t devtmpfs dev /dev 2>/dev/null
mount -t sysfs sys /sys 2>/dev/null
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
echo BENCH_READY
exec /bin/sh
EOF
chmod 755 "$ROOT/binit.sh"

# arch-python payload (v86's arch-python.js runs the same fib(30)).
cat > "$ROOT/fib.py" <<'EOF'
import sys
def fib(n):
    return n if n < 2 else fib(n-2)+fib(n-1)
print("FIB_START"); sys.stdout.flush()
print("fib(30)=", fib(30)); sys.stdout.flush()
print("FIB_DONE"); sys.stdout.flush()
EOF

SZ=$(( $(du -sm "$ROOT" | cut -f1) + 80 ))
fakeroot mke2fs -q -t ext4 -d "$ROOT" -F -L rootfs "$OUT/deb-rootfs.ext4" "${SZ}M"
echo "assembled $OUT/deb-rootfs.ext4 (${SZ}M)"
# Boot: cmdline "console=hvc0 root=/dev/vda rw init=/binit.sh", ramMB>=512.
# python fib(30):  vm.consoleInput("/usr/bin/python3 /fib.py\n")  (see deb-python.mjs)
