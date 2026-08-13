#!/usr/bin/env bash

# Build fixed-work BYTEmark against each target's unmodified static musl and
# overlay the binaries onto the frozen scorecard root filesystems. The RV64
# side is built for the project's RV64GCV baseline (VLEN is an execution-time
# property, fixed at 128 bits by the emulator). No emulator is run here.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARTIFACTS="${1:?usage: prepare-interpreter-stock-nbench.sh ARTIFACTS NBENCH_ARCHIVE [OUTPUT_DIR]}"
ARCHIVE="${2:?usage: prepare-interpreter-stock-nbench.sh ARTIFACTS NBENCH_ARCHIVE [OUTPUT_DIR]}"
ARTIFACTS="$(cd "$ARTIFACTS" && pwd)"
ARCHIVE="$(cd "$(dirname "$ARCHIVE")" && pwd)/$(basename "$ARCHIVE")"
OUTPUT="${3:-$ARTIFACTS/interpreter-stock-musl-rv64gcv-v1}"
mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"
ZIG="${ZIG:-$(command -v zig)}"
RV64_CPU="${RV64_CPU:-baseline_rv64+v+zifencei}"

RV64_BASE="$ARTIFACTS/scorecard-v2-modern-riscv64.cpio"
X86_BASE="$ARTIFACTS/scorecard-v2-modern-x86.cpio"
for required in \
    "$RV64_BASE" \
    "$X86_BASE" \
    "$ARCHIVE" \
    "$HERE/nbench-fixed-data32.patch" \
    "$HERE/nbench-fixed-work.patch" \
    "$HERE/nbench-extras/hwstub.c" \
    "$HERE/nbench-extras/sysinfo.c" \
    "$HERE/nbench-extras/sysinfoc.c" \
    "$HERE/nbench-extras/stock-musl-contract.c" \
    "$HERE/interpreter-stock-musl-contract.json"; do
    test -f "$required" || { echo "missing stock-musl input: $required" >&2; exit 2; }
done
printf '%s  %s\n' \
    723dd073f80e9969639eb577d2af4b540fc29716b6eafdac488d8f5aed9101ac \
    "$ARCHIVE" | sha256sum -c -

scratch="$(mktemp -d "${TMPDIR:-/tmp}/rv64-stock-musl.XXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT
tar -xzf "$ARCHIVE" -C "$scratch"
source_root="$scratch/nbench-byte-2.2.3"
(cd "$source_root" && patch -p1 < "$HERE/nbench-fixed-data32.patch")
(cd "$source_root" && patch -p1 < "$HERE/nbench-fixed-work.patch")
cp "$HERE/nbench-extras/sysinfo.c" "$source_root/sysinfo.c"
cp "$HERE/nbench-extras/sysinfoc.c" "$source_root/sysinfoc.c"

common=(
    -DLINUX
    -DSCORECARD_FIXED_DATA32
    -DSCORECARD_FIXED_WORK
    -O2
    -static
    -fno-builtin-memmove
    -fno-builtin-memcpy
)
sources=(nbench0.c nbench1.c sysspec.c misc.c emfloat.c)

: > "$source_root/pointer.h"
(cd "$source_root" && "$ZIG" cc -target x86-linux-musl "${common[@]}" \
    -o "$OUTPUT/nbench-fixed.i386" "${sources[@]}" "$HERE/nbench-extras/hwstub.c")
printf '%s\n' '#define LONG64' > "$source_root/pointer.h"
(cd "$source_root" && "$ZIG" cc -target riscv64-linux-musl -mcpu="$RV64_CPU" \
    "${common[@]}" \
    -o "$OUTPUT/nbench-fixed.rv64" "${sources[@]}" "$HERE/nbench-extras/hwstub.c")

for arch in riscv64 x86; do
    overlay="$scratch/overlay-$arch"
    mkdir -p "$overlay/opt/scorecard"
    if [ "$arch" = riscv64 ]; then
        base="$RV64_BASE"
        binary="$OUTPUT/nbench-fixed.rv64"
    else
        base="$X86_BASE"
        binary="$OUTPUT/nbench-fixed.i386"
    fi
    install -m 0755 "$binary" "$overlay/opt/scorecard/nbench-fixed"
    install -m 0644 "$HERE/nbench-extras/stock-musl-contract.c" \
        "$overlay/opt/scorecard/nbench-rv64-fastmem.c"
    install -m 0644 "$HERE/interpreter-stock-musl-contract.json" \
        "$overlay/opt/scorecard/nbench-stock-musl-contract.json"
    find "$overlay" -exec touch -h -d '@0' {} +
    (cd "$overlay" && find . -print0 | LC_ALL=C sort -z \
        | fakeroot cpio --null --create --format=newc --reproducible 2>/dev/null) \
        > "$OUTPUT/stock-musl-overlay-$arch.cpio"
    output="$OUTPUT/interpreter-stock-musl-$arch.cpio"
    cp "$base" "$output"
    dd if="$OUTPUT/stock-musl-overlay-$arch.cpio" of="$output" \
        oflag=append conv=notrunc status=none
done

cp "$HERE/interpreter-stock-musl-contract.json" "$OUTPUT/contract.json"
sha256sum \
    "$ARCHIVE" \
    "$RV64_BASE" \
    "$X86_BASE" \
    "$OUTPUT/nbench-fixed.rv64" \
    "$OUTPUT/nbench-fixed.i386" \
    "$OUTPUT/stock-musl-overlay-riscv64.cpio" \
    "$OUTPUT/stock-musl-overlay-x86.cpio" \
    "$OUTPUT/interpreter-stock-musl-riscv64.cpio" \
    "$OUTPUT/interpreter-stock-musl-x86.cpio" \
    "$HERE/nbench-fixed-data32.patch" \
    "$HERE/nbench-fixed-work.patch" \
    "$HERE/nbench-extras/hwstub.c" \
    "$HERE/nbench-extras/sysinfo.c" \
    "$HERE/nbench-extras/sysinfoc.c" \
    "$HERE/nbench-extras/stock-musl-contract.c" \
    "$HERE/interpreter-stock-musl-contract.json" \
    > "$OUTPUT/SHA256SUMS"

file "$OUTPUT/nbench-fixed.rv64" "$OUTPUT/nbench-fixed.i386"
echo "built sealed stock-musl population under $OUTPUT"
cat "$OUTPUT/SHA256SUMS"
