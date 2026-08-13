#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
rv64_dir="${RV64_DIR:-$(cd "$here/../.." && pwd)}"
guest_arch="${WANIX_GUEST_ARCH:-riscv64}"

case "$guest_arch" in
    riscv64)
        docker_platform=linux/riscv64
        apk_arch=riscv64
        go_arch=riscv64
        kernel_attr=virt-kernel-fast
        kernel_name=Image
        default_out="$here/dist/wanix-linux-rv64.tgz"
        kernel="${WANIX_KERNEL:-${RV64_KERNEL:-}}"
        ;;
    x86)
        docker_platform=linux/386
        apk_arch=x86
        go_arch=386
        kernel_attr=v86-kernel
        kernel_name=bzImage
        default_out="$here/dist/wanix-linux-x86.tgz"
        kernel="${WANIX_KERNEL:-${V86_KERNEL:-}}"
        ;;
    *)
        echo "unsupported WANIX_GUEST_ARCH: $guest_arch (expected riscv64 or x86)" >&2
        exit 2
        ;;
esac

out="${1:-$default_out}"
docker_cmd="${DOCKER_CMD:-docker}"
tmp="$(mktemp -d "/tmp/wanix-$guest_arch-root.XXXXXX")"
container="wanix-$guest_arch-root-$$"
wanix_src="$tmp/wanix"
rootfs="$tmp/rootfs"
wanix_ref="${WANIX_REF:-6594fe3763eb8712e81914f78b79243bb403f5cc}"
trap '$docker_cmd rm -f "$container" >/dev/null 2>&1 || true; rm -rf "$tmp"' EXIT

if [ -z "$kernel" ]; then
    kernel="$(nix build --no-link --print-out-paths "path:$rv64_dir#$kernel_attr" \
        | xargs -I{} find {} -maxdepth 2 -name "$kernel_name" -print | head -1)"
fi
test -n "$kernel"

"$docker_cmd" pull --platform="$docker_platform" alpine:3.22
"$docker_cmd" create --platform="$docker_platform" --name "$container" alpine:3.22 true >/dev/null
mkdir -p "$rootfs"
"$docker_cmd" export "$container" | tar -C "$rootfs" -xf -

# apk itself runs in a native build-platform container while installing the
# requested architecture's packages into the exported guest root. Package
# scripts are disabled because their binaries cannot execute on the host.
"$docker_cmd" run --rm --platform=linux/amd64 -v "$rootfs:/target" alpine:3.22 \
    apk --root /target --arch "$apk_arch" --no-scripts add python3
"$docker_cmd" run --rm --platform=linux/amd64 -v "$rootfs:/target" alpine:3.22 \
    chown -R "$(id -u):$(id -g)" /target

git clone --quiet https://github.com/tractordev/wanix.git "$wanix_src"
git -C "$wanix_src" checkout --quiet "$wanix_ref"
if [ "$guest_arch" = riscv64 ]; then
    git -C "$wanix_src" apply "$here/wanix-riscv64.patch"
fi
git -C "$wanix_src" apply "$here/wanix-wexec-js.patch"
git -C "$wanix_src" apply "$here/wanix-wexec-poll.patch"
git -C "$wanix_src" apply "$here/wanix-wexec-signal.patch"
git -C "$wanix_src" apply "$here/wanix-wexec-live-read.patch"

mkdir -p "$rootfs/boot" "$rootfs/bin" "$rootfs/etc" "$(dirname "$out")"
cp "$kernel" "$rootfs/boot/$kernel_name"
cp "$here/guest/init" "$rootfs/bin/init"
cp "$wanix_src/extras/linux/bin/domctl" "$wanix_src/extras/linux/bin/post-dhcp" \
    "$wanix_src/extras/linux/bin/startnet" "$wanix_src/extras/linux/bin/workerctl" "$rootfs/bin/"
cp "$wanix_src/extras/linux/etc/"* "$rootfs/etc/"
GOWORK=off GOOS=linux GOARCH="$go_arch" go build -C "$wanix_src" -o "$rootfs/bin/wexec" ./extras/wexec
GOWORK=off GOOS=linux GOARCH="$go_arch" go build -C "$wanix_src" -o "$rootfs/bin/hostexport" ./extras/hostexport
tar -C "$rootfs" -czf "$out" .
echo "$guest_arch Linux namespace: $out"
