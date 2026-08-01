#!/usr/bin/env bash
# Prepare the modern OpenSBI/Linux browser-demo assets. Outputs are ignored by
# git because the Debian disk is large and generated reproducibly.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/web/images/modern"
bench="$root/target/bench"
mkdir -p "$out" "$bench"

kernel="$(nix build --no-link --print-out-paths "$root#virt-kernel" \
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

if [ ! -f "$bench/deb-riscv64.ext4" ]; then
    "$root/tests/vs-v86/mk-debian-rootfs.sh" "$bench"
fi
ln -sfn "../../../target/bench/deb-riscv64.ext4" "$out/debian.ext4"

echo "modern images ready in $out"
ls -lh "$out/opensbi.bin" "$out/Image" "$out/debian.ext4"
