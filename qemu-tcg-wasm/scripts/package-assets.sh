#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/common.sh"

if [[ $# -eq 1 && ( "${1:-}" == "-h" || "${1:-}" == "--help" ) ]]; then
    echo "usage: $0 DIRECTORY" >&2
    echo "Packages DIRECTORY as /pack in Emscripten's virtual filesystem." >&2
    exit 0
fi
if [[ $# -ne 1 ]]; then
    echo "usage: $0 DIRECTORY" >&2
    echo "Packages DIRECTORY as /pack in Emscripten's virtual filesystem." >&2
    exit 2
fi
if [[ ! -d "$1" ]]; then
    echo "error: asset directory does not exist: $1" >&2
    exit 1
fi

PACK_DIR="$(cd -- "$1" && pwd)"
TARGET_NAME="${QEMU_TARGET:-x86_64}"
TARGET_NAME="${TARGET_NAME%-softmmu}"
DIST_DIR="${QEMU_DIST_DIR:-$ROOT_DIR/dist/$TARGET_NAME}"
DATA_FILE="$DIST_DIR/qemu-system-$TARGET_NAME.data"
LOADER_FILE="$DIST_DIR/load.mjs"
EMSCRIPTEN_ROOT="$(em-config EMSCRIPTEN_ROOT)"
FILE_PACKAGER="$EMSCRIPTEN_ROOT/tools/file_packager.py"

if [[ ! -f "$DIST_DIR/qemu-system-$TARGET_NAME.js" ]]; then
    echo "error: QEMU is not built under $DIST_DIR" >&2
    exit 1
fi
if [[ ! -f "$FILE_PACKAGER" ]]; then
    echo "error: Emscripten's file_packager.py was not found" >&2
    exit 1
fi

echo "==> Packaging $PACK_DIR as /pack"
python "$FILE_PACKAGER" "$DATA_FILE" \
    --preload "$PACK_DIR@/pack" \
    --js-output="$LOADER_FILE" \
    --export-es6 \
    --quiet

echo "==> Guest asset bundle"
ls -lh "$DATA_FILE" "$LOADER_FILE"
