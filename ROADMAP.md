# rv64.js roadmap

All six original phases are complete (see DESIGN.md): rv64gc + privileged
architecture, boots Linux natively and in the browser, JIT live in both run
loops, softfloat F/D with native fast path, riscv-tests 134/134, validated
against Spike/QEMU/TinyEMU. This file tracks what comes next, in rough
priority order. Check items off as they land.

## Performance (the JIT's next tiers)

- [x] **1. Inline-TLB memory ops in full-system JIT blocks** *(done; the
  system-mode win)* — system JIT blocks translate guest loads/stores inline
  (TLB probe → direct RAM on hit, bail to interpreter on miss/MMIO/page-
  cross/store-to-compiled-page), with per-page invalidation. Combined with
  item 2's cheap dispatch and slice=256/threshold=64, this delivers
  **2.8-3.2x on real in-guest compute** (tests/bench-sys.mjs). It looked
  neutral for a while only because the benchmarks were unrepresentative and
  the harness had an echo bug — see the item-2b CORRECTION and
  tests/BASELINE.md.

- [x] **2. Cheap dispatch** *(done 2026-07-22)* — direct-mapped dispatch
  array (replaces HashMap+SipHash per block) + `cpu.jit_flush_gen`
  (drops the per-dispatch pa-verify TLB walk; flush only on satp/SFENCE).
  +6% on user-int+fp; the `jit_set_enabled` diagnostic shows user JIT is
  1.56× over the wasm interpreter. Dispatch is no longer the bottleneck.

- [ ] **2b. Fused JIT software-TLB** *(deferred)* — a dedicated JIT-TLB
  encoding RAM+writable+not-compiled to shrink inline memory ops from ~15 to
  ~4 wasm ops. Attempted twice and reverted (interpreter-fill tax, then
  one-shot-boot dispatch overhead; full post-mortem in git history). Item 1's
  inline TLB already delivers the system win; superseded in priority by the
  register-locals work below.

### Closing the JIT gap to v86 (measured plan, 2026-07-23)

`tests/vs-v86/` benchmarks us head-to-head against copy/v86 on the same
machine. Result: **our interpreter is a peer to v86's, but our JIT is ~11×
(mixed) to ~18× (pure ALU) slower than v86's.** Deep-dive into both codebases
found the gap is 100% implementation maturity — and riscv64 is a *better* JIT
target than x86 (no condition flags → we skip v86's largest subsystem, its
lazy-EFLAGS machinery; fixed-width decode; clean IEEE F/D vs x87). v86 wins
via three techniques; below is our phased plan to adopt them, each measured
with `tests/vs-v86` (baseline vs v86: ALU 50.5s vs 2.9s; mixed 17.1s vs 1.5s).

- [ ] **3a. Phase 1 — registers in wasm locals** *(the #1 ALU win)* — today
  every guest-register access is a linear-memory load/store to the CPU state
  struct (`rv64-jit` `push_reg`/`store_post`); the xorshift loop does ~8
  memory ops/iteration where v86 does zero. Cache the GPRs a block touches in
  `i64` wasm locals (v86's `register_locals`): prologue loads read-regs →
  locals; `push_reg`→`local.get`, `store_post`→`local.set`; **every exit and
  every mid-block bail** flushes dirty locals → state (the interpreter reads
  state). Moderate effort, large ALU win.

- [ ] **3b. Phase 2 — FP ops inside JIT blocks** — FP currently *ends* the
  block, so FP-heavy code gets ~0% coverage (half the mixed workload). Emit
  wasm `f32/f64` ops under the interpreter's sticky-NX/RNE eligibility guard
  (`cpu.rs fp_fast64/32`; JitLayout needs `fcsr_addr` + `f_base`), cache FPRs
  in `f64` locals, bail otherwise. Easier for us than for v86 (no x87).
  Differentially fuzz blocks against softfp like the interpreter fast path.

- [ ] **3c. Phase 3 — compile loops/regions as one wasm function** *(the
  architectural leap, highest ceiling)* — today each basic block is its own
  wasm function and a hot loop re-`call_indirect`s through the Rust dispatcher
  every iteration, so V8 can't optimize across the back-edge and locals get
  flushed/reloaded each time. Structure a hot region/loop into a single wasm
  function with internal control flow (wasm `loop`/`br`), like v86's
  `control_flow.rs` (SCC → Loop/Block/Dispatcher). Combined with Phase 1 this
  keeps registers in locals across iterations → V8 JITs the loop near-native.

- [ ] **4. (Optional) residual-based flag recovery** — extend the FP fast
  path to work before NX is set, via error-free transformations (TwoSum
  for add, Dekker splitting for mul — no FMA needed, so wasm-compatible).
  Low priority: sticky-NX already covers real workloads.

The old **item 2b (fused JIT software-TLB)** stays deferred — item 1's inline
TLB already delivers the system-mode win; register-locals (3a) is the bigger
lever now. Its two reverted attempts are documented in git history.

## Validation

- [x] **5. Architecture-test compliance vs Spike** *(done 2026-07-22)* —
  implemented as direct signature comparison (`tests/run-arch-tests.sh`):
  the official riscv-arch-test 3.9.1 suites (I M A C F D Zifencei
  privilege) compile once against a shared HTIF/CLINT env and run on both
  rv64.js (`rv64-isa-test --signature`) and Spike (`+signature=`).
  **193/193 signatures bit-identical.** This is the substance of RISCOF
  (same tests, same golden model, same pass criterion); adopting the
  RISCOF report framework itself remains optional polish.

- [x] **6. Lockstep differential vs Spike** *(done 2026-07-22)* —
  `tests/lockstep.py` + `rv64-isa-test --trace`: per-instruction
  x-register writeback streams diffed against `spike --log-commits`
  (flake builds Spike with `--enable-commitlog`). 109/109 riscv-tests
  lockstep-identical (~24k writebacks; ma_data skipped as the documented
  spec-legal misaligned-access divergence).

- [x] **6b. Modern-system smoke / regression harness** *(done 2026-07-23)* —
  `tests/virt-smoke/run.sh` boots the virt machine with a ~10 KB initramfs
  whose init exercises the paths that were broken (8250 THRE TX interrupt,
  LR/SC-across-trap, live rdtime) and checks for `SMOKE_OK` vs a hang. Inputs
  come from the flake (`.#virt-kernel`, `.#virt-opensbi`; the freestanding init
  is built by the dev-shell cross-gcc) so it's reproducible. **Coverage is layered** because not all bugs are catchable by
  an integration test: the THRE hang is deterministic and caught by the smoke
  test, but the LR/SC reservation and rdtime bugs are probabilistic/latent —
  a simple boot passes even with them reverted — so they are pinned by
  deterministic unit tests (`cpu::tests::trap_invalidates_lr_reservation`,
  `rdtime_derives_live_from_insn_count`). Each guard was validated by
  reverting its fix and confirming it fails. See `tests/virt-smoke/README.md`.

## Features (TinyEMU parity and beyond)

- [ ] **7. virtio-9p** — host filesystem sharing into the guest (TinyEMU's
  `fs.c`/9P2000.L is the reference; browser backend via fetch/IndexedDB).
  This is jslinux's killer feature and what makes browser Linux genuinely
  useful. Medium-large effort.

- [ ] **8. virtio-net** — guest networking. Browser needs a WebSocket
  relay (v86's approach); native can use a tun/tap or slirp port. Large
  effort, needs infrastructure.

- [x] **9. Modern guest images** *(done 2026-07-23)* — a new **virt**-class
  machine (`crates/rv64-system/src/virt.rs`, runner `bin/rv64-vboot`) boots a
  stock **Debian riscv64 6.18 kernel** via **OpenSBI fw_dynamic** (not BBL),
  with a full PLIC, ns16550 UART, sifive-test, CLINT and virtio-mmio, plus a
  modern DTB (incl. `/chosen/rng-seed`). With a virtio-blk/ext4-builtin kernel
  it mounts a real Debian rootfs and runs Debian userland; a debootstrap-built
  rootfs with build-essential **compiles the Linux kernel in-guest** (gcc-14 +
  make; ~100× slower than native under the interpreter — a full build is a
  days-long soak, not for CI). Getting here fixed several real full-system
  bugs — the 8250 THRE transmit interrupt, LR/SC reservation-on-trap, and
  live rdtime; see the smoke harness below and `tests/virt-smoke/README.md`.

- [ ] **10. Snapshot save/restore** — serialize machine state (CPU + RAM +
  devices are all plain data) for v86-style instant-boot-from-snapshot in
  the browser. Small-medium effort.

## Housekeeping

- [ ] **11. Publish** — create the GitHub repo and push (no remote yet).
- [ ] **12. CI** — run tests/run-all.sh in GitHub Actions via the nix
  flake (`nix develop -c tests/run-all.sh`).
- [ ] Optionally rename the directory (`~/src/arm64.js` → `rv64.js`).

## Recommended sequencing

1 → 2 → 3 (the perf trilogy — they compound; inline-TLB is what makes the
other two matter in system mode), then 7 (the feature that changes what
the project *is*). 8–10 as interest dictates. 11/12 any time.
Validation items 5–6 are done and run as suite stages 5–6.
