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
#   .#virt-kernel-fast  slim riscv64 kernel Image used by the Alpine machine
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

echo "[virt-smoke] resolving flake inputs (first run builds the kernel)…"
# --print-out-paths can emit several outputs; search all of them for the file.
image="$(nix build --no-link --print-out-paths .#virt-kernel-fast \
    | xargs -I{} find {} -maxdepth 2 -name Image 2>/dev/null | head -1)"
fw="$(nix build --no-link --print-out-paths .#virt-opensbi \
    | xargs -I{} find {} -name fw_dynamic.bin 2>/dev/null | grep -E 'generic' | head -1)"
# The init is freestanding, so the bare-metal cross-gcc already in the dev
# shell (RISCV_PREFIX, set by flake.nix) suffices — no libc, no extra build.
cc="${RISCV_PREFIX:-riscv64-none-elf-}gcc"

[ -n "$image" ] && [ -n "$fw" ] && command -v "$cc" >/dev/null || {
  echo "[virt-smoke] FAIL: could not resolve image=$image fw=$fw cc=$cc"; exit 2; }

echo "[virt-smoke] building rv64-vboot…"
cargo build --release --bin rv64-vboot >/dev/null 2>&1

echo "[virt-smoke] building guest init + initramfs…"
# Freestanding (no libc): a static, non-PIE riscv64 ELF Linux can load and run
# directly. Works with any riscv64 gcc (bare-metal or linux cross).
"$cc" -nostdlib -ffreestanding -static -no-pie \
    -march=rv64gc -mabi=lp64d -Os \
    -Wl,-Ttext-segment=0x10000 -o "$work/init" "$here/init.c"
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
grep -aE 'SMOKE_START|RDTIME_OK|RTC_OK|TTY_DRAIN_OK|FORKS_OK|SMOKE_OK|FAIL_' "$out" | sed 's/^/    /' || true

if grep -qa 'SMOKE_OK' "$out" && grep -qa 'RTC_OK' "$out" && ! grep -qa 'FAIL_' "$out"; then
  echo "[virt-smoke] PASS"
  exit 0
else
  echo "[virt-smoke] FAIL — guest did not reach SMOKE_OK (hang or error). Last output:"
  tail -15 "$out" | sed 's/^/    /'
  exit 1
fi
