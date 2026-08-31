#!/usr/bin/env bash
# Build the release Alpine riscv64 ext4 image without root or guest execution.
set -euo pipefail

OUT="${1:-$PWD}"
VERSION="${ALPINE_VERSION:-3.24.1}"
EXPECTED_SHA256="${ALPINE_SHA256:-7201513262d851f39105102cf95519410100259bd7996fca13bade517838d7b7}"
BRANCH="v${VERSION%.*}"
MIRROR="${ALPINE_MIRROR:-https://dl-cdn.alpinelinux.org/alpine}"
ROOT="$OUT/alpine-riscv64"
IMG="$OUT/alpine-riscv64.ext4"
TARBALL="$OUT/alpine-minirootfs-$VERSION-riscv64.tar.gz"
URL="$MIRROR/$BRANCH/releases/riscv64/$(basename "$TARBALL")"

mkdir -p "$OUT"
if [ ! -f "$TARBALL" ]; then
    wget -O "$TARBALL" "$URL"
fi
printf '%s  %s\n' "$EXPECTED_SHA256" "$(basename "$TARBALL")" \
    | (cd "$OUT" && sha256sum -c -)

rm -rf "$ROOT"
mkdir -p "$ROOT"
fakeroot tar -xzf "$TARBALL" -C "$ROOT"
cat > "$ROOT/etc/apk/repositories" <<EOF
$MIRROR/$BRANCH/main
$MIRROR/$BRANCH/community
EOF

# Use the host apk only as a cross-architecture package extractor. Guest
# maintainer scripts are deliberately skipped; the boot script performs the
# small amount of runtime setup we need.
apk --root "$ROOT" --arch riscv64 --no-scripts --no-cache \
    --repositories-file "$ROOT/etc/apk/repositories" \
    add alpine-base ca-certificates openssl

mkdir -p "$ROOT/etc/profile.d" "$ROOT/run/rv64-proxy" "$ROOT/usr/local/sbin"
cat > "$ROOT/etc/profile.d/rv64-proxy.sh" <<'EOF'
if grep -qw 'rv64.network=fetch' /proc/cmdline 2>/dev/null; then
    export http_proxy=http://10.0.2.2:8080
    export https_proxy=http://10.0.2.2:8080
    export HTTP_PROXY="$http_proxy"
    export HTTPS_PROXY="$https_proxy"
else
    unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY
fi
EOF

cat > "$ROOT/rv64-init" <<'EOF'
#!/bin/sh
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev 2>/dev/null || true
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
. /etc/profile.d/rv64-proxy.sh

ip link set eth0 up
ip addr add 10.0.2.15/24 dev eth0
ip route add default via 10.0.2.2

# The emulator exposes the exact ephemeral proxy CA through this private 9p
# share before guest networking starts. apk can therefore use normal HTTPS
# repositories without disabling certificate verification.
if grep -qw 'rv64.network=fetch' /proc/cmdline && \
   mount -t 9p -o trans=virtio,version=9p2000.L,ro rv64-proxy /run/rv64-proxy; then
    openssl x509 -inform DER -in /run/rv64-proxy/ca.der \
        -out /usr/local/share/ca-certificates/rv64-proxy.crt
    update-ca-certificates >/dev/null
    echo PROXY_CA_READY
fi

hostname rv64
stty rows 24 cols 80 </dev/ttyS0 2>/dev/null || true
echo ALPINE_READY
echo 'Networking is configured. Try: apk update && apk add nano'
exec setsid -c /bin/sh -l </dev/ttyS0 >&0 2>&1
EOF
chmod 0755 "$ROOT/rv64-init"

size_mb=$(( $(du -sm "$ROOT" | cut -f1) + 64 ))
# apk's bbsuid helper is intentionally execute-only. mke2fs still needs to read
# every source file while constructing the image; retaining execute permission
# is sufficient for the guest.
find "$ROOT" -type f ! -readable -exec chmod u+r {} +
fakeroot mke2fs -q -t ext4 -d "$ROOT" -F -L rv64-alpine "$IMG" "${size_mb}M"

# mke2fs -d cannot preserve fakeroot's synthetic device nodes. Install the
# console node directly in ext4 so PID 1 has working standard streams before
# devtmpfs is mounted.
debugfs -w -R 'rm /dev/console' "$IMG" >/dev/null 2>&1 || true
debugfs -w -R 'mknod /dev/console c 5 1' "$IMG" >/dev/null

echo "assembled $IMG (${size_mb} MiB, Alpine $VERSION riscv64)"
