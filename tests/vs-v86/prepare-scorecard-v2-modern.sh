#!/usr/bin/env bash
# Build paired benchmark initramfs archives from the current Alpine guest
# release. This deliberately does not read or copy any historical TinyEMU,
# BBL, Debian, or Buildroot root image.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARTIFACTS="${1:?usage: prepare-scorecard-v2-modern.sh ARTIFACTS}"
VERSION="${ALPINE_VERSION:-3.24.1}"
BRANCH="v${VERSION%.*}"
MIRROR="${ALPINE_MIRROR:-https://dl-cdn.alpinelinux.org/alpine}"
INIT="$HERE/scorecard-v2-alpine-init"
FIB="$HERE/scorecard-v2-fib.py"

ARTIFACTS="$(cd "$ARTIFACTS" && pwd)"

sha_for() {
    case "$1" in
        riscv64) echo 7201513262d851f39105102cf95519410100259bd7996fca13bade517838d7b7 ;;
        x86) echo 634355e2245c9d56186d1b86fb6e034453eb303aea15b573ca250b343376fffd ;;
        *) return 2 ;;
    esac
}

suffix_for() {
    case "$1" in
        riscv64) echo rv64 ;;
        x86) echo i386 ;;
        *) return 2 ;;
    esac
}

for required in \
    "$INIT" \
    "$FIB" \
    "$ARTIFACTS/w.c" \
    "$ARTIFACTS/nbench-workload-contract.json" \
    "$ARTIFACTS/nbench-fixed-data32.patch" \
    "$ARTIFACTS/nbench-fixed-work.patch" \
    "$ARTIFACTS/nbench-rv64-fastmem.c" \
    "$ARTIFACTS/fastmem-selftest.rv64"; do
    test -f "$required" || { echo "missing required input: $required" >&2; exit 2; }
done

scratch="$(mktemp -d "${TMPDIR:-/tmp}/rv64-scorecard-v2.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

for arch in riscv64 x86; do
    suffix="$(suffix_for "$arch")"
    archive="$ARTIFACTS/alpine-minirootfs-$VERSION-$arch.tar.gz"
    output="$ARTIFACTS/scorecard-v2-modern-$arch.cpio"
    package_list="$ARTIFACTS/scorecard-v2-modern-$arch.packages"
    root="$scratch/$arch"
    name="$(basename "$archive")"

    if [ ! -f "$archive" ]; then
        curl --fail --location --show-error --output "$archive" \
            "$MIRROR/$BRANCH/releases/$arch/$name"
    fi
    printf '%s  %s\n' "$(sha_for "$arch")" "$name" \
        | (cd "$ARTIFACTS" && sha256sum -c -)

    mkdir -p "$root"
    fakeroot tar -xzf "$archive" -C "$root"
    cat > "$root/etc/apk/repositories" <<EOF
$MIRROR/$BRANCH/main
$MIRROR/$BRANCH/community
EOF
    fakeroot apk --root "$root" --arch "$arch" --no-scripts --no-cache \
        --repositories-file "$root/etc/apk/repositories" \
        add alpine-base ca-certificates openssl python3
    apk --root "$root" info | LC_ALL=C sort > "$package_list"

    mkdir -p "$root/opt/scorecard"
    install -m 0755 "$INIT" "$root/init"
    install -m 0644 "$FIB" "$root/opt/scorecard/fib.py"
    install -m 0644 "$ARTIFACTS/w.c" "$root/opt/scorecard/w.c"
    install -m 0755 "$ARTIFACTS/tcc.$suffix" "$root/opt/scorecard/tcc"
    install -m 0755 "$ARTIFACTS/nbench.$suffix" "$root/opt/scorecard/nbench"
    install -m 0755 "$ARTIFACTS/nbench-fixed.$suffix" "$root/opt/scorecard/nbench-fixed"
    install -m 0755 "$ARTIFACTS/nbench-native.$suffix" "$root/opt/scorecard/nbench-native"
    install -m 0644 "$ARTIFACTS/nbench-workload-contract.json" \
        "$root/opt/scorecard/nbench-workload-contract.json"
    install -m 0644 "$ARTIFACTS/nbench-fixed-data32.patch" \
        "$root/opt/scorecard/nbench-fixed-data32.patch"
    install -m 0644 "$ARTIFACTS/nbench-fixed-work.patch" \
        "$root/opt/scorecard/nbench-fixed-work.patch"
    # Embed the RV64-specific libc replacement source in both archives. It is
    # not linked on i686, but its shared hash makes that implementation choice
    # explicit in every cross-ISA result rather than hiding it in an ELF hash.
    install -m 0644 "$ARTIFACTS/nbench-rv64-fastmem.c" \
        "$root/opt/scorecard/nbench-rv64-fastmem.c"
    install -m 0755 "$ARTIFACTS/xbench/alu.$suffix" "$root/opt/scorecard/alu"
    mixed="rvbench_fs.$suffix"
    install -m 0755 "$ARTIFACTS/xbench/$mixed" "$root/opt/scorecard/mixed"
    if [ "$arch" = riscv64 ]; then
        install -m 0755 "$ARTIFACTS/fastmem-selftest.rv64" \
            "$root/opt/scorecard/fastmem-selftest"
    fi

    (cd "$root" && find . -print0 | LC_ALL=C sort -z \
        | fakeroot cpio --null -o -H newc 2>/dev/null) > "$output"
    sha256sum "$output" > "$output.sha256"
    echo "built $output"
done

diff -u \
    "$ARTIFACTS/scorecard-v2-modern-riscv64.packages" \
    "$ARTIFACTS/scorecard-v2-modern-x86.packages"

sha256sum \
    "$INIT" "$FIB" \
    "$ARTIFACTS/scorecard-v2-modern-riscv64.cpio" \
    "$ARTIFACTS/scorecard-v2-modern-x86.cpio"
echo "verified identical Alpine package names across architectures"
