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

- [ ] **2. In-wasm block chaining** — dispatch currently costs a HashMap
  lookup + `call_indirect` per block (~1 dispatch per loop iteration on
  small loops). Options: patch direct block→block calls for constant-target
  ends (JAL/branches), or a wasm-side pc→funcindex dispatch table to keep
  the loop inside wasm. Medium effort, multiplies hot-loop throughput.

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

- [ ] **5. RISCOF formal compliance** — `pip install riscof`, write the
  rv64.js plugin (DUT runner = rv64-isa-test-style ELF loader + signature
  dump), reference model = Spike (in the nix shell). Gives a formal
  riscv-arch-test compliance report vs "passes riscv-tests". Mostly
  plumbing.

- [ ] **6. Lockstep differential vs Spike** — run a guest instruction-by-
  instruction on rv64.js and Spike, diffing full architectural state each
  step. Catches divergences functional tests can't. Needs a state-dump
  hook on our side and Spike's commit log (`spike -l`). Medium effort.

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
other two matter in system mode), then 5 while validation context is
fresh, then 7 (the feature that changes what the project *is*). 8–10 as
interest dictates. 11/12 any time.
