#!/usr/bin/env bash

# Append a deterministic holdout overlay to each frozen scorecard initramfs.
# The original archives remain byte-for-byte untouched. This script builds the
# sealed inputs but deliberately does not execute either emulator.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARTIFACTS="${1:?usage: prepare-interpreter-holdouts.sh ARTIFACTS [OUTPUT_DIR]}"
ARTIFACTS="$(cd "$ARTIFACTS" && pwd)"
OUTPUT="${2:-$ARTIFACTS/interpreter-holdouts-v1}"
mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"

RV64_BASE="$ARTIFACTS/scorecard-v2-modern-riscv64.cpio"
X86_BASE="$ARTIFACTS/scorecard-v2-modern-x86.cpio"
for required in \
    "$RV64_BASE" \
    "$X86_BASE" \
    "$HERE/interpreter-holdout-data.mjs" \
    "$HERE/interpreter-holdouts/holdout_gzip" \
    "$HERE/interpreter-holdouts/holdout_sort" \
    "$HERE/interpreter-holdouts/holdout_sha256" \
    "$HERE/interpreter-holdouts/holdout_aes"; do
    test -f "$required" || { echo "missing holdout input: $required" >&2; exit 2; }
done

scratch="$(mktemp -d "${TMPDIR:-/tmp}/rv64-interpreter-holdouts.XXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT
overlay="$scratch/overlay"
mkdir -p "$overlay/opt/holdout"

node "$HERE/interpreter-holdout-data.mjs" "$overlay/opt/holdout" \
    > "$OUTPUT/contract.json"
for workload in holdout_gzip holdout_sort holdout_sha256 holdout_aes; do
    install -m 0755 "$HERE/interpreter-holdouts/$workload" \
        "$overlay/opt/holdout/$workload"
done
find "$overlay" -exec touch -h -d '@0' {} +
(cd "$overlay" && find . -print0 | LC_ALL=C sort -z \
    | fakeroot cpio --null --create --format=newc --reproducible 2>/dev/null) \
    > "$scratch/holdout-overlay.cpio"
cp "$scratch/holdout-overlay.cpio" "$OUTPUT/holdout-overlay.cpio"

for arch in riscv64 x86; do
    if [ "$arch" = riscv64 ]; then
        base="$RV64_BASE"
    else
        base="$X86_BASE"
    fi
    output="$OUTPUT/interpreter-holdout-$arch.cpio"
    cp "$base" "$output"
    dd if="$scratch/holdout-overlay.cpio" of="$output" \
        oflag=append conv=notrunc status=none
done

cp "$overlay/opt/holdout/expected.env" "$OUTPUT/expected.env"
sha256sum \
    "$RV64_BASE" \
    "$X86_BASE" \
    "$OUTPUT/holdout-overlay.cpio" \
    "$OUTPUT/interpreter-holdout-riscv64.cpio" \
    "$OUTPUT/interpreter-holdout-x86.cpio" \
    "$HERE/interpreter-holdout-data.mjs" \
    "$HERE/interpreter-holdouts/holdout_gzip" \
    "$HERE/interpreter-holdouts/holdout_sort" \
    "$HERE/interpreter-holdouts/holdout_sha256" \
    "$HERE/interpreter-holdouts/holdout_aes" \
    > "$OUTPUT/SHA256SUMS"

echo "built sealed holdouts under $OUTPUT"
cat "$OUTPUT/SHA256SUMS"
