#!/usr/bin/env bash
# Assemble the versioned binary set consumed by web/site.mjs. Run inside
# `nix develop` to build the Alpine browser demo.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/target/demo-images-v4}"
mkdir -p "$out"

"$root/web/prepare-images.sh"
cargo build --manifest-path "$root/Cargo.toml" \
    -p rv64-wasm --target wasm32-unknown-unknown --release

install -m 0644 "$root/target/wasm32-unknown-unknown/release/rv64_wasm.wasm" "$out/rv64_wasm.wasm"
install -m 0644 "$root/web/images/alpine/Image" "$out/modern-Image"
disk="$(readlink -f "$root/web/images/alpine/alpine.ext4")"
install -m 0644 "$disk" "$out/modern-alpine.ext4"

(cd "$out" && sha256sum \
    rv64_wasm.wasm modern-Image modern-alpine.ext4 > SHA256SUMS)

echo "demo release assets ready in $out"
du -h "$out"/*
