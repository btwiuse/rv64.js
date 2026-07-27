#!/usr/bin/env bash
# Assemble a minimal Debian rootfs into an ext4 image. Besides Python for the
# cross-emulator benchmark, the default image includes a real TLS client and
# the small set of networking tools needed by proxy integration tests.
#
# No root and no binfmt: fakeroot debootstrap --foreign downloads and
# first-stage-unpacks on the host; we dpkg-deb -x the --include packages (which
# --foreign leaves unconfigured for the second stage we can't run), then
# mke2fs -d assembles the directory into an image. In the nix dev shell:
#   ARCH=riscv64 nix develop -c tests/vs-v86/mk-debian-rootfs.sh <outdir>
#   ARCH=i386    nix develop -c tests/vs-v86/mk-debian-rootfs.sh <outdir>
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-$PWD}"; mkdir -p "$OUT"
ARCH="${ARCH:-riscv64}"
ROOT="$OUT/deb-$ARCH"
IMG="$OUT/deb-$ARCH.ext4"
SUITE="${SUITE:-trixie}"                       # Debian 13
MIRROR="${MIRROR:-http://deb.debian.org/debian}"
# nbench was removed from Debian (unmaintained) — build it separately
# (mk-nbench-rootfs.sh) if you want it. `INCLUDE=...` remains an escape hatch
# for specialised/minimal images.
if [ -z "${INCLUDE+x}" ]; then
    INCLUDE="python3,libc-bin,curl,ca-certificates,iproute2,busybox-static,openssl"
    validate_default_tools=1
else
    validate_default_tools=
fi

rm -rf "$ROOT"
# --foreign can exit non-zero (it stops before the second stage we can't run),
# so don't let set -e abort on it; verify by the downloaded .debs instead.
fakeroot debootstrap --foreign --arch="$ARCH" --variant=minbase \
    --include="$INCLUDE" --no-check-gpg "$SUITE" "$ROOT" "$MIRROR" || true
shopt -s nullglob
debs=("$ROOT"/var/cache/apt/archives/*.deb)
[ "${#debs[@]}" -gt 0 ] || { echo "debootstrap first stage failed (no .debs)"; exit 1; }

# Unpack the downloaded-but-unconfigured packages (python3 + deps). Bare
# programs need no maintainer-script configuration.
for d in "${debs[@]}"; do fakeroot dpkg-deb -x "$d" "$ROOT/"; done

# The foreign first stage cannot run ca-certificates' maintainer script, so
# assemble the conventional bundle explicitly. curl and Python's ssl module
# both discover this path without requiring an emulated second-stage chroot.
mapfile -d '' certs < <(find "$ROOT/usr/share/ca-certificates" -type f -name '*.crt' -print0 2>/dev/null | sort -z)
if [ "${#certs[@]}" -gt 0 ]; then
    mkdir -p "$ROOT/etc/ssl/certs"
    cat "${certs[@]}" > "$ROOT/etc/ssl/certs/ca-certificates.crt"
elif [ -n "$validate_default_tools" ]; then
    echo "ca-certificates unpacked no certificates"
    exit 1
fi

# busybox-static supplies a dependency-free DHCP client. Debian does not
# install an udhcpc command symlink, so expose the applet explicitly.
if [ -x "$ROOT/bin/busybox" ]; then
    mkdir -p "$ROOT/usr/sbin"
    ln -sf ../../bin/busybox "$ROOT/usr/sbin/udhcpc"
    install -Dm755 "$HERE/udhcpc.script" "$ROOT/usr/share/udhcpc/default.script"
fi

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
# `--proxy` attaches a second, read-only 9p share containing the exact public
# CA used for this run's CONNECT certificates. Install it before announcing
# readiness so every normal TLS client can use the system trust bundle.
mkdir -p /run/rv64-proxy
if mount -t 9p -o trans=virtio,version=9p2000.L,ro rv64-proxy /run/rv64-proxy 2>/dev/null; then
    mkdir -p /usr/local/share/ca-certificates
    if openssl x509 -inform DER -in /run/rv64-proxy/ca.der \
        -out /usr/local/share/ca-certificates/rv64-proxy.crt 2>/dev/null &&
       update-ca-certificates >/dev/null 2>&1; then
        echo PROXY_CA_READY
    else
        echo PROXY_CA_BAD
    fi
fi
echo BENCH_READY
exec /bin/sh
EOF
chmod 755 "$ROOT/binit.sh"

if [ -n "$validate_default_tools" ]; then
    for required in \
        usr/bin/python3 usr/bin/curl usr/bin/openssl usr/sbin/ip \
        usr/sbin/update-ca-certificates \
        bin/busybox usr/sbin/udhcpc usr/share/udhcpc/default.script \
        etc/ssl/certs/ca-certificates.crt
    do
        [ -e "$ROOT/$required" ] || { echo "missing required guest file: /$required"; exit 1; }
    done
fi

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
