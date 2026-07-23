#!/usr/bin/env bash
# rv64.js virt-machine smoke / regression test.
#
# Boots the modern "virt" machine (OpenSBI fw_dynamic + a stock riscv64 Linux
# kernel) with a tiny initramfs whose init (init.c) exercises the full-system
# paths that were once broken: the 8250 THRE transmit interrupt, LR/SC
# reservation handling across traps, and a live-advancing rdtime. On success
# the guest prints SMOKE_OK and powers off; a hang (missing THRE interrupt,
# etc.) makes this script time out and FAIL.
#
# All inputs come from the flake so the test is reproducible:
#   .#virt-kernel   stock riscv64 kernel Image (virtio/serial/initramfs)
#   .#virt-opensbi  OpenSBI fw_dynamic firmware
#   .#virt-cc       riscv64 Linux C cross-compiler (builds init.c)
# The first run builds the kernel + toolchain (cached thereafter).
#
# Usage:  nix develop -c tests/virt-smoke/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$root"

echo "[virt-smoke] resolving flake inputs (first run builds kernel+toolchain)…"
# --print-out-paths can emit several outputs (e.g. gcc's out/man/info); search
# all of them for the file we need.
image="$(nix build --no-link --print-out-paths .#virt-kernel \
    | xargs -I{} find {} -maxdepth 2 -name Image 2>/dev/null | head -1)"
fw="$(nix build --no-link --print-out-paths .#virt-opensbi \
    | xargs -I{} find {} -name fw_dynamic.bin 2>/dev/null | grep -E 'generic' | head -1)"
cc="$(nix build --no-link --print-out-paths .#virt-cc \
    | xargs -I{} find {}/bin -name '*-linux-*-gcc' 2>/dev/null | head -1)"

[ -n "$image" ] && [ -n "$fw" ] && [ -n "$cc" ] || {
  echo "[virt-smoke] FAIL: could not resolve image=$image fw=$fw cc=$cc"; exit 2; }

echo "[virt-smoke] building rv64-vboot…"
cargo build --release --bin rv64-vboot >/dev/null 2>&1

echo "[virt-smoke] building guest init + initramfs…"
"$cc" -static -Os -o "$work/init" "$here/init.c"
mkdir -p "$work/irfs"
cp "$work/init" "$work/irfs/init"
( cd "$work/irfs" && find . | cpio -o -H newc 2>/dev/null | gzip ) > "$work/initramfs.cpio.gz"

echo "[virt-smoke] booting virt machine…"
out="$work/out.log"
VBOOT_MAX_INSNS=9000000000000000 timeout 120 \
  "$root/target/release/rv64-vboot" "$fw" "$image" \
  --initrd "$work/initramfs.cpio.gz" --ram 1 \
  -- "console=ttyS0 earlycon=uart8250,mmio,0x10000000 rdinit=/init" \
  < /dev/null > "$out" 2>&1 || true

echo "[virt-smoke] guest markers:"
grep -aE 'SMOKE_START|RDTIME_OK|TTY_DRAIN_OK|FORKS_OK|SMOKE_OK|FAIL_' "$out" | sed 's/^/    /' || true

if grep -qa 'SMOKE_OK' "$out"; then
  echo "[virt-smoke] PASS"
  exit 0
else
  echo "[virt-smoke] FAIL — guest did not reach SMOKE_OK (hang or error). Last output:"
  tail -15 "$out" | sed 's/^/    /'
  exit 1
fi
