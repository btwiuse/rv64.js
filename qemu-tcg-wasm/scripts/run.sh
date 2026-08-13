#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/common.sh"

TARGET_NAME="${QEMU_TARGET:-x86_64}"
TARGET_NAME="${TARGET_NAME%-softmmu}"
DIST_DIR="${QEMU_DIST_DIR:-$ROOT_DIR/dist/$TARGET_NAME}"
RUNNER="$QEMU_DIR/scripts/run-emscripten.mjs"
PRELOAD="${QEMU_PRELOAD:-}"

if [[ ! -d "$DIST_DIR" ]]; then
    echo "error: QEMU output directory does not exist: $DIST_DIR" >&2
    exit 1
fi
DIST_DIR="$(cd -- "$DIST_DIR" && pwd)"
JAVASCRIPT="$DIST_DIR/qemu-system-$TARGET_NAME.js"
if [[ ! -f "$JAVASCRIPT" ]]; then
    echo "error: $JAVASCRIPT does not exist; run scripts/build-qemu.sh first" >&2
    exit 1
fi
if [[ ! -f "$RUNNER" ]]; then
    echo "error: QEMU's Node.js runner is missing: $RUNNER" >&2
    exit 1
fi
if [[ -z "$PRELOAD" && -f "$DIST_DIR/load.mjs" ]]; then
    PRELOAD="$DIST_DIR/load.mjs"
fi
if [[ -n "$PRELOAD" && ! -f "$PRELOAD" ]]; then
    echo "error: QEMU_PRELOAD does not name a file: $PRELOAD" >&2
    exit 1
fi
if [[ -n "$PRELOAD" ]]; then
    PRELOAD="$(cd -- "$(dirname -- "$PRELOAD")" && pwd)/$(basename -- "$PRELOAD")"
fi

runner_args=(node "$RUNNER")
if [[ -n "$PRELOAD" ]]; then
    runner_args+=(--preload "$PRELOAD")
fi
runner_args+=("$JAVASCRIPT" -- "$@")

# file_packager.py emits a data-file basename. Run beside its loader (or beside
# QEMU when there is no loader) so Node finds the corresponding data bundle.
RUN_DIR="$DIST_DIR"
if [[ -n "$PRELOAD" ]]; then
    RUN_DIR="$(dirname -- "$PRELOAD")"
fi
cd -- "$RUN_DIR"
exec "${runner_args[@]}"
