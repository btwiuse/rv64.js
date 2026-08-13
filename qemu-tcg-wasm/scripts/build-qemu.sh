#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/common.sh"

QEMU_TARGET="${QEMU_TARGET:-x86_64-softmmu}"
TARGET_NAME="${QEMU_TARGET%-softmmu}"
BUILD_DIR="${QEMU_BUILD_DIR:-$ROOT_DIR/build-$TARGET_NAME}"
DIST_DIR="${QEMU_DIST_DIR:-$ROOT_DIR/dist/$TARGET_NAME}"

if [[ ! -x "$QEMU_DIR/configure" ]]; then
    echo "error: QEMU source is missing from $QEMU_DIR" >&2
    exit 1
fi

"$SCRIPT_DIR/build-deps.sh"
mkdir -p "$BUILD_DIR" "$DIST_DIR"

if [[ ! -f "$BUILD_DIR/build.ninja" ]]; then
    echo "==> Configuring QEMU target $QEMU_TARGET with the wasm64 TCG backend"
    (
        cd "$BUILD_DIR"
        emconfigure "$QEMU_DIR/configure" \
            --cpu=wasm64 \
            --static \
            --disable-docs \
            --disable-tools \
            --disable-werror \
            --target-list="$QEMU_TARGET"
    )
fi

echo "==> Compiling QEMU target $QEMU_TARGET"
emmake make -C "$BUILD_DIR" -j"$JOBS"

for artifact in \
    "qemu-system-$TARGET_NAME.js" \
    "qemu-system-$TARGET_NAME.wasm" \
    "qemu-system-$TARGET_NAME.worker.js"; do
    if [[ -f "$BUILD_DIR/$artifact" ]]; then
        cp -f "$BUILD_DIR/$artifact" "$DIST_DIR/$artifact"
    fi
done

echo "==> Build artifacts"
find "$DIST_DIR" -maxdepth 1 -type f -printf '%f\t%s bytes\n' | sort
