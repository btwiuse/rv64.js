#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-$ROOT/target/bench}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

rv_kernel="$(nix build --no-link --print-out-paths "path:$ROOT#virt-kernel-fast")"
x86_kernel="$(nix build --no-link --print-out-paths "path:$ROOT#v86-kernel")"
opensbi="$(nix build --no-link --print-out-paths "path:$ROOT#virt-opensbi" \
    | xargs -I{} find {} -path '*generic*fw_dynamic.bin' | head -1)"

install -m 0644 "$rv_kernel/Image" "$OUT/matched-linux-rv64-Image"
install -m 0644 "$x86_kernel/bzImage" "$OUT/matched-linux-x86-bzImage"
install -m 0644 "$opensbi" "$OUT/matched-opensbi.bin"
"$ROOT/tests/vs-v86/mk-matched-alpine.sh" "$OUT"

ls -lh "$OUT"/matched-linux-* "$OUT"/matched-opensbi.bin \
    "$OUT"/matched-alpine-*.cpio
