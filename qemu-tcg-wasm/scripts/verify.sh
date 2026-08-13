#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/common.sh"

TARGET_NAME="${QEMU_TARGET:-x86_64}"
TARGET_NAME="${TARGET_NAME%-softmmu}"
BUILD_DIR="${QEMU_BUILD_DIR:-$ROOT_DIR/build-$TARGET_NAME}"
DIST_DIR="${QEMU_DIST_DIR:-$ROOT_DIR/dist/$TARGET_NAME}"
WASM="$DIST_DIR/qemu-system-$TARGET_NAME.wasm"
JAVASCRIPT="$DIST_DIR/qemu-system-$TARGET_NAME.js"
CONFIG_HOST="$BUILD_DIR/config-host.h"
BUILD_NINJA="$BUILD_DIR/build.ninja"

if [[ ! -f "$WASM" || ! -f "$JAVASCRIPT" || ! -f "$CONFIG_HOST" || ! -f "$BUILD_NINJA" ]]; then
    echo "error: expected build and dist artifacts; run scripts/build-qemu.sh first" >&2
    exit 1
fi

echo "==> QEMU build configuration"
grep -q '^#define HOST_WASM64 1$' "$CONFIG_HOST" || {
    echo "error: QEMU was not configured for a wasm64 host" >&2
    exit 1
}
grep -q '^#define CONFIG_TCG 1$' "$CONFIG_HOST" || {
    echo "error: TCG is not enabled" >&2
    exit 1
}
if grep -q '^#define CONFIG_TCG_INTERPRETER 1$' "$CONFIG_HOST"; then
    echo "error: this is a TCI build, not the native wasm64 TCG backend" >&2
    exit 1
fi
grep -q 'tcg_wasm64\.c\.o' "$BUILD_NINJA" || {
    echo "error: tcg/wasm64.c is absent from the build graph" >&2
    exit 1
}
echo "HOST_WASM64=1, CONFIG_TCG=1, TCI disabled, tcg/wasm64.c compiled"

echo "==> Artifact types"
file "$JAVASCRIPT" "$WASM"

echo "==> WebAssembly sections"
wasm-objdump -h "$WASM"

echo "==> Dynamic WebAssembly instantiation support"
if grep -q 'WebAssembly\.Module' "$JAVASCRIPT"; then
    echo "found WebAssembly.Module instantiation support"
else
    echo "error: WebAssembly.Module support was not found" >&2
    exit 1
fi
