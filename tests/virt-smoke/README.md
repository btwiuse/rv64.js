# virt-smoke — modern-system (Debian-class) boot harness

Boots the **virt** machine (`crates/rv64-system/src/virt.rs`, runner
`bin/rv64-vboot`) — OpenSBI `fw_dynamic` + a stock riscv64 Linux kernel — and
runs a tiny initramfs whose `init` (`init.c`) drives the full-system paths that
were once broken. On success it prints `SMOKE_OK` and powers off; a hang makes
the harness time out and fail.

```
nix develop -c tests/virt-smoke/run.sh
```

First run builds the kernel and cross-toolchain from the flake
(`.#virt-kernel`, `.#virt-opensbi`, `.#virt-cc`); both are cached afterwards, so
later runs take a few seconds.

## What it exercises, and why it's layered

We found three real full-system bugs bringing this up (all now fixed). They are
**not equally catchable by an integration test**, which is the crux of the test
design:

| Bug | Symptom | Guarded by | Why |
|-----|---------|-----------|-----|
| **Missing 8250 THRE (TX) interrupt** | Guest wedges at `default_idle_call`; bash's tty drain-`ioctl` blocks forever | **this smoke test** (deterministic) | The interrupt-driven-TX / drain path fires on *any* real console workload, so a small test still trips it reliably. |
| **LR/SC reservation not cleared on trap** | Lost atomic update → intermittent lost wakeup | **unit test** `cpu::tests::trap_invalidates_lr_reservation` | Needs an interrupt to land in the exact LR→SC window of a *contended* atomic — rare on one hart. A simple boot passes even with the bug reverted, so an integration test is worthless here; a direct unit test is deterministic. |
| **`rdtime` frozen within a slice** | Busy-wait loops (`__delay`) stall; short-interval timing reads 0 | **unit test** `cpu::tests::rdtime_derives_live_from_insn_count` | A correctness property of the `time` CSR, not a boot-visible hang. |

**Lesson (verified empirically):** simplifying an integration test does *not*
automatically preserve its bug-catching power. We validated each guard by
reverting its fix and confirming the guard fails:

- THRE reverted → this harness times out with no `SMOKE_OK`. ✓
- LR/SC reservation reverted → this harness still passes (bug is probabilistic),
  but the unit test fails. ✓ → that's why the reservation and rdtime invariants
  live in fast, deterministic `cargo test -p rv64-core` unit tests, and the
  integration test only claims to guard the THRE hang + "Debian userland runs".

So the regression coverage is **layered**: unit tests pin the subtle CPU-core
invariants; this smoke test pins the emergent full-system behavior.

## init.c

Runs as PID 1 in the initramfs and, in order:

1. `rdtime` delta across a `nanosleep` (sanity that the clock moves).
2. A large console-output burst + `tcsetattr(TCSADRAIN)` — forces the 8250
   driver into interrupt-driven TX and blocks on the drain if THRE is missing.
3. A `fork`+`exec` loop with the timer ticking underneath (multi-process /
   atomic churn).
4. `SMOKE_OK`, then `reboot(RB_POWER_OFF)`.

## Heavier tier — full Debian + kernel compile (manual)

The smoke test uses a ~10 KB initramfs. The full capability — booting a real
Debian riscv64 rootfs and compiling the Linux kernel in-guest — is a separate,
much heavier workload (multi-GB image, ~100× slower than native under the
interpreter, impractical for CI). It's documented in `ROADMAP.md`; the rootfs is
assembled with `debootstrap --foreign` + `dpkg-deb -x` + `mkfs.ext4 -d` and
booted with `rv64-vboot … --disk <ext4.img> -- "root=/dev/vda rw init=…"`.
