#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
rv64_dir="${RV64_DIR:-$(cd "$here/../.." && pwd)}"
out="${1:-$here/dist/wanix-linux-rv64.tgz}"
docker_cmd="${DOCKER_CMD:-docker}"
tmp="$(mktemp -d /tmp/wanix-rv64-root.XXXXXX)"
container="wanix-rv64-root-$$"
wanix_src="$tmp/wanix"
rootfs="$tmp/rootfs"
wanix_ref="${WANIX_REF:-6594fe3763eb8712e81914f78b79243bb403f5cc}"
trap '$docker_cmd rm -f "$container" >/dev/null 2>&1 || true; rm -rf "$tmp"' EXIT

kernel="${RV64_KERNEL:-}"
if [ -z "$kernel" ]; then
    kernel="$(nix build --no-link --print-out-paths "path:$rv64_dir#virt-kernel-fast" \
        | xargs -I{} find {} -maxdepth 2 -name Image -print | head -1)"
fi
test -n "$kernel"

"$docker_cmd" pull --platform=linux/riscv64 alpine:3.22
"$docker_cmd" create --platform=linux/riscv64 --name "$container" alpine:3.22 true >/dev/null
mkdir -p "$rootfs"
"$docker_cmd" export "$container" | tar -C "$rootfs" -xf -

# apk itself runs in a native build-platform container while installing the
# requested architecture's packages into the exported RISC-V root. Package
# scripts are disabled because their binaries cannot execute on the host.
"$docker_cmd" run --rm --platform=linux/amd64 -v "$rootfs:/target" alpine:3.22 \
    apk --root /target --arch riscv64 --no-scripts add python3
"$docker_cmd" run --rm --platform=linux/amd64 -v "$rootfs:/target" alpine:3.22 \
    chown -R "$(id -u):$(id -g)" /target

git clone --quiet https://github.com/tractordev/wanix.git "$wanix_src"
git -C "$wanix_src" checkout --quiet "$wanix_ref"
git -C "$wanix_src" apply "$here/wanix-riscv64.patch"
git -C "$wanix_src" apply "$here/wanix-wexec-js.patch"
git -C "$wanix_src" apply "$here/wanix-wexec-poll.patch"
git -C "$wanix_src" apply "$here/wanix-wexec-signal.patch"
git -C "$wanix_src" apply "$here/wanix-wexec-live-read.patch"

mkdir -p "$rootfs/boot" "$rootfs/bin" "$rootfs/etc" "$(dirname "$out")"
cp "$kernel" "$rootfs/boot/Image"
cp "$here/guest/init" "$rootfs/bin/init"
cp "$wanix_src/extras/linux/bin/domctl" "$wanix_src/extras/linux/bin/post-dhcp" \
    "$wanix_src/extras/linux/bin/startnet" "$wanix_src/extras/linux/bin/workerctl" "$rootfs/bin/"
cp "$wanix_src/extras/linux/etc/"* "$rootfs/etc/"
GOWORK=off GOOS=linux GOARCH=riscv64 go build -C "$wanix_src" -o "$rootfs/bin/wexec" ./extras/wexec
GOWORK=off GOOS=linux GOARCH=riscv64 go build -C "$wanix_src" -o "$rootfs/bin/hostexport" ./extras/hostexport
tar -C "$rootfs" -czf "$out" .
echo "rv64 Linux namespace: $out"
