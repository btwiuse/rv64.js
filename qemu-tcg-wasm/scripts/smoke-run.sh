#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/common.sh"

SMOKE_TIMEOUT="${SMOKE_TIMEOUT:-20}"
OUTPUT_FILE="$(mktemp)"
trap 'rm -f "$OUTPUT_FILE"' EXIT

echo "==> Starting the Wasm QEMU under Node.js"
set +e
timeout --signal=TERM "${SMOKE_TIMEOUT}s" "$SCRIPT_DIR/run.sh" --version \
    >"$OUTPUT_FILE" 2>&1
status=$?
set -e
cat "$OUTPUT_FILE"

if ! grep -q '^QEMU emulator version ' "$OUTPUT_FILE"; then
    echo "error: QEMU did not print its version" >&2
    exit 1
fi

case "$status" in
    0)
        echo "==> Runtime smoke test passed"
        ;;
    124)
        echo "==> Runtime smoke test passed; Node was stopped after the expected Emscripten atexit hang"
        ;;
    *)
        echo "error: Node/QEMU exited with status $status" >&2
        exit "$status"
        ;;
esac
