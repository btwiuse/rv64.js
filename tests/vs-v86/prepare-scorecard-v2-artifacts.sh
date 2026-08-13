#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARTIFACTS="${1:?usage: prepare-scorecard-v2-artifacts.sh ARTIFACTS}"
SOURCE="$ARTIFACTS/deb-i386-bench.cpio.gz"
OUTPUT="$ARTIFACTS/scorecard-v2-i386.cpio.gz"

test -f "$SOURCE" || { echo "missing $SOURCE" >&2; exit 1; }

scratch="$(mktemp -d)"
cleanup() { rm -rf -- "$scratch"; }
trap cleanup EXIT

mkdir -p "$scratch/root"
(
    cd "$scratch/root"
    gzip -dc "$SOURCE" | fakeroot cpio -id --quiet --no-absolute-filenames
)
install -m 0755 "$HERE/scorecard-v2-v86-init" "$scratch/root/init"
(
    cd "$scratch/root"
    find . -print0 | fakeroot cpio --null -o -H newc --quiet | gzip -1 > "$OUTPUT"
)

sha256sum "$SOURCE" "$HERE/scorecard-v2-v86-init" "$OUTPUT"
