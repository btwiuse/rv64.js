#!/usr/bin/env bash
# Assemble a minimal Debian rootfs (python3 + the nbench package) into an ext4
# image, for apples-to-apples benchmarking of the SAME userland on both
# emulators: ARCH=riscv64 (default) boots under our JIT machine; ARCH=i386
# boots under v86. Prebuilt .debs only — no build-from-scratch.
#
# No root and no binfmt: fakeroot debootstrap --foreign downloads and
# first-stage-unpacks on the host; we dpkg-deb -x the --include packages (which
# --foreign leaves unconfigured for the second stage we can't run), then
# mke2fs -d assembles the directory into an image. In the nix dev shell:
#   ARCH=riscv64 nix develop -c tests/vs-v86/mk-debian-rootfs.sh <outdir>
#   ARCH=i386    nix develop -c tests/vs-v86/mk-debian-rootfs.sh <outdir>
set -euo pipefail
OUT="${1:-$PWD}"; mkdir -p "$OUT"
ARCH="${ARCH:-riscv64}"
ROOT="$OUT/deb-$ARCH"
IMG="$OUT/deb-$ARCH.ext4"
SUITE="${SUITE:-trixie}"                       # Debian 13
MIRROR="${MIRROR:-http://deb.debian.org/debian}"
# nbench was removed from Debian (unmaintained) — build it separately
# (mk-nbench-rootfs.sh) if you want it. python3 gives us arch-python.
INCLUDE="${INCLUDE:-python3,libc-bin}"

rm -rf "$ROOT"
fakeroot debootstrap --foreign --arch="$ARCH" --variant=minbase \
    --include="$INCLUDE" --no-check-gpg "$SUITE" "$ROOT" "$MIRROR"

# Unpack the downloaded-but-unconfigured packages (python3, nbench, deps). Bare
# programs need no maintainer-script configuration.
for d in "$ROOT"/var/cache/apt/archives/*.deb; do fakeroot dpkg-deb -x "$d" "$ROOT/"; done

# nbench (Debian package) reads NNET.DAT from the cwd; expose it at / and drop a
# tiny command file (glibc's open works, unlike the newlib build) for short runs.
NNET="$(find "$ROOT" -name NNET.DAT 2>/dev/null | head -1 || true)"
[ -n "$NNET" ] && cp "$NNET" "$ROOT/NNET.DAT"
printf 'MINSECONDS=2\n' > "$ROOT/nbench.cmd"

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

SZ=$(( $(du -sm "$ROOT" | cut -f1) + 100 ))
fakeroot mke2fs -q -t ext4 -d "$ROOT" -F -L rootfs "$IMG" "${SZ}M"
echo "assembled $IMG (${SZ}M, $ARCH)"
# riscv64: boot under our machine, cmdline "console=hvc0 root=/dev/vda rw \
#   init=/binit.sh", ramMB>=512 (see deb-python.mjs).
# i386: boot under v86 with this image as a disk (see deb-v86.mjs).
