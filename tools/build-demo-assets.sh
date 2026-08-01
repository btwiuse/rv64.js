#!/usr/bin/env bash
# Assemble the versioned binary set consumed by web/site.mjs. Run inside
# `nix develop` after preparing both image families.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/target/demo-images-v1}"
mkdir -p "$out"

"$root/web/get-images.sh"
"$root/web/prepare-modern-images.sh"
cargo build --manifest-path "$root/Cargo.toml" \
    -p rv64-wasm --target wasm32-unknown-unknown --release

install -m 0644 "$root/target/wasm32-unknown-unknown/release/rv64_wasm.wasm" "$out/rv64_wasm.wasm"
install -m 0644 "$root/web/images/bbl64.bin" "$out/fast-bbl64.bin"
install -m 0644 "$root/web/images/kernel-riscv64.bin" "$out/fast-kernel-riscv64.bin"
install -m 0644 "$root/web/images/root-riscv64.bin" "$out/fast-root-riscv64.bin"
install -m 0644 "$root/web/images/modern/opensbi.bin" "$out/modern-opensbi.bin"
install -m 0644 "$root/web/images/modern/Image" "$out/modern-Image"
disk="$(readlink -f "$root/web/images/modern/debian.ext4")"
install -m 0644 "$disk" "$out/modern-debian.ext4"

(cd "$out" && sha256sum \
    rv64_wasm.wasm \
    fast-bbl64.bin fast-kernel-riscv64.bin fast-root-riscv64.bin \
    modern-opensbi.bin modern-Image modern-debian.ext4 > SHA256SUMS)

echo "demo release assets ready in $out"
du -h "$out"/*
