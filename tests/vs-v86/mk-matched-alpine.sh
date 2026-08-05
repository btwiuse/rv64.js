#!/usr/bin/env bash
# Build paired Alpine 3.24 initramfs images for the rv64.js/v86 boot row.
# Both roots contain the same package set and byte-identical init script. The
# archives are intentionally uncompressed so guest decompression is excluded.
set -euo pipefail

OUT="${1:-$PWD}"
VERSION="${ALPINE_VERSION:-3.24.1}"
BRANCH="v${VERSION%.*}"
MIRROR="${ALPINE_MIRROR:-https://dl-cdn.alpinelinux.org/alpine}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

sha_for() {
    case "$1" in
        riscv64) echo 7201513262d851f39105102cf95519410100259bd7996fca13bade517838d7b7 ;;
        x86) echo 634355e2245c9d56186d1b86fb6e034453eb303aea15b573ca250b343376fffd ;;
        *) return 2 ;;
    esac
}

for arch in riscv64 x86; do
    root="$OUT/matched-alpine-$arch"
    archive="$OUT/alpine-minirootfs-$VERSION-$arch.tar.gz"
    name="$(basename "$archive")"
    if [ ! -f "$archive" ]; then
        curl --fail --location --show-error --output "$archive" \
            "$MIRROR/$BRANCH/releases/$arch/$name"
    fi
    printf '%s  %s\n' "$(sha_for "$arch")" "$name" \
        | (cd "$OUT" && sha256sum -c -)

    rm -rf "$root"
    mkdir -p "$root"
    fakeroot tar -xzf "$archive" -C "$root"
    cat > "$root/etc/apk/repositories" <<EOF
$MIRROR/$BRANCH/main
$MIRROR/$BRANCH/community
EOF
    fakeroot apk --root "$root" --arch "$arch" --no-scripts --no-cache \
        --repositories-file "$root/etc/apk/repositories" add alpine-base
    apk --root "$root" info | LC_ALL=C sort \
        > "$OUT/matched-alpine-$arch.packages"

    cat > "$root/init" <<'EOF'
#!/bin/sh
mount -t devtmpfs dev /dev 2>/dev/null || true
exec >/dev/ttyS0 2>&1 </dev/ttyS0
mount -t proc proc /proc
mount -t sysfs sys /sys
echo MATCHED_ROOT_READY
echo ALPINE_READY
exec /bin/sh
EOF
    chmod 0755 "$root/init"

    (cd "$root" && find . -print0 | LC_ALL=C sort -z \
        | fakeroot cpio --null -o -H newc 2>/dev/null) \
        > "$OUT/matched-alpine-$arch.cpio"
    echo "built $OUT/matched-alpine-$arch.cpio"
done

diff -u "$OUT/matched-alpine-riscv64.packages" \
    "$OUT/matched-alpine-x86.packages"
echo "verified identical Alpine package names across architectures"
