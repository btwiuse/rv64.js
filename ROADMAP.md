# rv64.js roadmap

All six original phases are complete (see DESIGN.md): rv64gc + privileged
architecture, boots Linux natively and in the browser, JIT live in both run
loops, softfloat F/D with native fast path, riscv-tests 134/134, validated
against Spike/QEMU/TinyEMU. This file tracks what comes next, in rough
priority order. Check items off as they land.

## Performance (the JIT's next tiers)

- [ ] **1. Inline-TLB memory ops in full-system JIT blocks** — the biggest
  remaining perf lever. System blocks are ALU/branch-only today; every guest
  load/store ends a block and drops to the MMU interpreter, which fragments
  kernel/userland code badly. Plan: emit an inline TLB probe in the block
  (tag-match against a flattened TLB array in linear memory → direct RAM
  access on hit; bail to interpreter with pc set on miss). Invalidation
  already works (satp pa-verify + compiled-page store tracking). Large
  effort, transforms system-mode throughput.

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
  - **The fix (identified, not yet built): privilege-tagged JTLB decoupled
    from the priv-change flush.** Encode the effective privilege in the
    entry tag (fold mode into the low 12 bits of `va & ~0xfff`, which are
    free). Then `flush_tlb()` stops touching the JTLB; the JTLB is flushed
    only at true mapping/permission-model changes (satp write, SFENCE.VMA,
    SUM/MXR writes) — all rare, none per-interrupt. The JTLB stays warm
    across timer interrupts, so fills are rare and the interpreter tax
    amortizes away, while probes gain one cheap tag compare. Validate with
    the full gate (md5 + arch-tests 193 + lockstep 109) before keeping.

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
