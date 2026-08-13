#!/usr/bin/env bash
set -euo pipefail

# Rebuild the historical rv64-jit comparator without putting any of its code
# back into the rewrite's production crates. The base is an immutable upstream
# commit; legacy-modern-virt.patch is deliberately limited to machine wiring.

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
source_commit="5b896f9"
adapter_patch="$script_dir/legacy-modern-virt.patch"
output_root="${LEGACY_MODERN_ROOT:-$repo_root/target/scorecard-v2-legacy}"
expected_patch_sha="3d2f5b786ab8483f87257c1d49cbc61edce20a05cbfc0d2b0706fa6e7a82a107"
expected_loader_sha="54df79c8b35cf50bcee34c4af02d7eb02b09e0439b717ee75bb830e733595b12"
expected_wasm_sha="274aaab5799386956a8c509434961c4a426066f8fc9f520e994c210affd61709"

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/rv64-legacy-modern.XXXXXXXX")"
cleanup() {
    find "$work_dir" -depth -delete
}
trap cleanup EXIT

git -C "$repo_root" cat-file -e "$source_commit^{commit}"

patch_sha="$(sha256sum "$adapter_patch" | awk '{print $1}')"
if [[ "$patch_sha" != "$expected_patch_sha" ]]; then
    echo "legacy adapter patch hash mismatch: $patch_sha" >&2
    exit 1
fi

git -C "$repo_root" archive "$source_commit" | tar -x -C "$work_dir"
patch --batch --fuzz=0 -d "$work_dir" -p1 < "$adapter_patch"

toolchain="$(rustc --version)"
if [[ "$toolchain" != rustc\ 1.97.1* ]]; then
    echo "legacy comparator requires rustc 1.97.1; found: $toolchain" >&2
    exit 1
fi

cargo build \
    --manifest-path "$work_dir/Cargo.toml" \
    --package rv64-wasm \
    --target wasm32-unknown-unknown \
    --release

mkdir -p \
    "$output_root/web" \
    "$output_root/target/wasm32-unknown-unknown/release"
install -m 0644 \
    "$work_dir/web/rv64.js" \
    "$output_root/web/rv64.js"
install -m 0644 \
    "$work_dir/target/wasm32-unknown-unknown/release/rv64_wasm.wasm" \
    "$output_root/target/wasm32-unknown-unknown/release/rv64_wasm.wasm"

loader_sha="$(sha256sum "$output_root/web/rv64.js" | awk '{print $1}')"
wasm_sha="$(sha256sum "$output_root/target/wasm32-unknown-unknown/release/rv64_wasm.wasm" | awk '{print $1}')"
if [[ "$loader_sha" != "$expected_loader_sha" ]]; then
    echo "legacy comparator loader hash mismatch: $loader_sha" >&2
    exit 1
fi
if [[ "$wasm_sha" != "$expected_wasm_sha" ]]; then
    echo "legacy comparator Wasm hash mismatch: $wasm_sha" >&2
    exit 1
fi

echo "legacy modern comparator ready: $output_root"
echo "  source: $source_commit"
echo "  patch:  $patch_sha"
echo "  loader: $loader_sha"
echo "  wasm:   $wasm_sha"
