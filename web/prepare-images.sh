#!/usr/bin/env bash
# Prepare the Alpine/Linux browser-demo assets. Outputs are ignored by
# git because the Alpine disk is generated reproducibly.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/web/images/alpine"
bench="$root/target/bench"
mkdir -p "$out" "$bench"

kernel="$(nix build --no-link --print-out-paths "$root#virt-kernel-fast" \
    | xargs -I{} find {} -maxdepth 2 -name Image 2>/dev/null | head -1)"
opensbi="$(nix build --no-link --print-out-paths "$root#virt-opensbi" \
    | xargs -I{} find {} -name fw_dynamic.bin 2>/dev/null | grep -E 'generic' | head -1)"
[ -n "$kernel" ] && [ -n "$opensbi" ] || {
    echo "could not resolve the kernel or OpenSBI firmware" >&2
    exit 2
}

rm -f "$out/Image" "$out/opensbi.bin"
install -m 0644 "$kernel" "$out/Image"
install -m 0644 "$opensbi" "$out/opensbi.bin"

if [ ! -f "$bench/alpine-riscv64.ext4" ]; then
    "$root/tests/virt-smoke/mk-alpine-rootfs.sh" "$bench"
fi
ln -sfn "../../../target/bench/alpine-riscv64.ext4" "$out/alpine.ext4"

echo "demo images ready in $out"
ls -lh "$out/opensbi.bin" "$out/Image" "$out/alpine.ext4"
