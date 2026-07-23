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

- [ ] **2b. Fused JIT software-TLB (the system-mode unlock)** — the
  diagnostic shows system JIT is 1.00× on memory-heavy code because inline
  memory ops emit ~15 wasm instructions each. A dedicated JIT-TLB whose
  present entries encode RAM+writable+not-compiled makes a load tag-compare
  + offset-add + load (~4 ops).
  - **First attempt (2026-07-22) — reverted, regressed.** Built the fused
    JIT-TLB (Cpu.jtlb arrays, Bus::jit_ram_offset, emitter probe, wasm
    wiring). Correct (md5 bit-identical) but a **net loss**: sys-md5
    69→38, boot 69→38 Minsn/s. Root cause: the JTLB was *filled* inside the
    interpreter's `translate()` (the ~89%-hot path), and `flush_tlb()` wipes
    it on **every privilege-change trap** (each timer interrupt). So the
    expensive `bus.jit_ram_offset` fill ran on nearly every interpreter
    memory op → the interpreter itself slowed ~45%. The probe was cheap; the
    fill placement + flush frequency was fatal.
  - **Second attempt (2026-07-23) — privilege-tagged JTLB. Also reverted.**
    Built exactly the fix above: mode folded into the free low-12 tag bits,
    `jit_active` gate so the interpreter pays nothing when JIT is off, JTLB
    flushed only on satp/SFENCE/SUM-MXR (not per-trap). It *did* remove the
    interpreter-tax problem — but with the JIT actually engaged (slice 256)
    **boot regressed to 0.90×** (compile + dispatch overhead on one-shot
    boot code exceeds savings), and a benchmark reported an intermittent
    md5 mismatch (not reproduced in a cleaner 12-run test, so likely a
    harness artifact — but the perf regression alone made it non-viable).
  - **CORRECTION (2026-07-23): the above conclusion was WRONG.** It came
    from unrepresentative benchmarks (md5 memory-extreme, boot one-shot,
    shell a tree-walker) plus a terminal-echo harness bug that made the
    shell/compute A/Bs measure nothing. A correct measurement — a compute
    binary run inside booted Linux, echo disabled, waiting for its real
    checksum (tests/bench-sys.mjs) — shows the item-2 system-mode JIT is a
    large win: 2.8x (register-heavy) to 3.2x (memory+ALU mix), the
    memory-heavier workload benefiting MORE. md5 (~1.0x) is a memory-density
    outlier. Shipped config: slice=256, JIT_ON_THRESHOLD=64 (boot only ~5%
    slower; compute 2.8-3.2x). The 2b JTLB work is NOT required for a
    system-mode win: item 1's inline-TLB memory ops already deliver it. 2b
    remains a possible further optimization but is no longer the gate.
    Both user-mode (1.56x) and system-mode (2.8-3.2x) JIT are now proven,
    shipped wins.

- [ ] **3. FP ops inside JIT blocks** — FP instructions currently end
  blocks. Reuse the interpreter's sticky-NX/RNE eligibility guard (see
  cpu.rs `fp_fast64`): emit a runtime guard on fcsr (JitLayout needs an
  fcsr_addr) + inline wasm `f32/f64` ops with the same operand/result
  shape checks; bail to interpreter otherwise. Medium effort; big win for
  FP-heavy guest loops. Differentially fuzz blocks against softfp like the
  interpreter fast path.

- [ ] **4. (Optional) residual-based flag recovery** — extend the FP fast
  path to work before NX is set, via error-free transformations (TwoSum
  for add, Dekker splitting for mul — no FMA needed, so wasm-compatible).
  Low priority: sticky-NX already covers real workloads.

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

## Features (TinyEMU parity and beyond)

- [ ] **7. virtio-9p** — host filesystem sharing into the guest (TinyEMU's
  `fs.c`/9P2000.L is the reference; browser backend via fetch/IndexedDB).
  This is jslinux's killer feature and what makes browser Linux genuinely
  useful. Medium-large effort.

- [ ] **8. virtio-net** — guest networking. Browser needs a WebSocket
  relay (v86's approach); native can use a tun/tap or slirp port. Large
  effort, needs infrastructure.

- [ ] **9. Modern guest images** — current images are TinyEMU's 2018
  Linux 4.15 + buildroot. Build a current kernel + rootfs (toolchain is in
  the nix shell) to validate modern virtio drivers and get a nicer
  userland. Also replaces BBL with OpenSBI.

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
