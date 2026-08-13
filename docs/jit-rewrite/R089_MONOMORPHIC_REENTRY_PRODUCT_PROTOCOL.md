# R089 Exact Generated-Entry Re-entry Monomorphization Protocol

Date: 2026-08-09  
Status: rejected at frozen native Boot gate; candidate removed and exact R085 restored

## Hypothesis

R088 attributes 12.7193% of corrected-cadence exact-R085 Boot self time to
`Cpu::run_until`'s exact generated-entry re-entry loop. R056's immutable
engine-shape experiment measures the same mutable dispatch-tag predicate at
1.494x when its concrete lookup is statically visible instead of reached
through Wasm `call_indirect`. Amdahl projection is 1.0439x whole Boot, clearing
the standing 3% cumulative opportunity rule. Compile and Python are guards;
this candidate is not expected to close parity alone.

The R088 proof report is `03e6b354e691...`, closure analysis is
`d0568e0da0bf...`, and independent gate is `a96d6eb2e46a...`. The gate passes
with no problems and records the exact R085 product, modern guest, production
policy, public one-slice cadence, generated execution, four complete profiles,
and 95% closure coverage.

## Frozen control and implementation

The immutable control is
`target/bench/wasm-candidates/r085-fast-jit-state-efd7830307ef.wasm`, SHA-256
`efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`.
The loader remains
`2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.
Pre-edit source identities are:

- `crates/rv64-wasm/src/lib.rs`
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- `crates/rv64-system/src/virt.rs`
  `dc5d04e4670c94775858b06c5dbbf815d18b6cd0e26cc1e3c6ef7dcd7b0c6bdb`;
- `crates/rv64-system/src/lib.rs`
  `06c7cacff32e23aff00309838897937054c9a87507dfd428d6b737731f3b579d`;
  and
- `crates/rv64-core/src/cpu.rs`
  `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`.

Change only callback typing through the existing call chain:

1. make `SystemJitMachine::run_interpreter_until` generic over the concrete
   `FnMut(u64) -> bool` predicate;
2. make `SystemJitMachine::run_policy_interpreter` generic over that same
   predicate while leaving the sampling callback and all policy inputs exact;
3. apply those signatures to both legacy and virt implementations; and
4. make `VirtMachine::run_slice_sampled_until` generic over the concrete
   generated-entry predicate.

Do not change `Cpu::run_until`, instruction stepping, callback frequency,
dispatch indexing/tag comparison, precise-stop selection, sampling, policy,
thresholds, module geometry, generated code, scheduler cadence, guest I/O, or
any state transition. Do not add a runtime switch. The rule is uniform across
every PC, opcode, address space, guest binary, workload, and host engine.

## Activation and correctness gates

Build one release candidate and archive it by hash. `llvm-objdump` must show
that exact R085 has two `call_indirect` operations inside the named
`Cpu::run_until` body while the candidate has zero; the candidate must retain
the same direct `Cpu::step` call and exact dispatch-tag loads/comparison.
Record the observed counts rather than relying on source shape alone.

Before timing, require:

1. formatting and all core, system, DBT, Wasm/API/Worker tests;
2. the complete `REQUIRE_ALL=1 tests/run-all.sh` correctness matrix, including
   QEMU/Spike/signatures, randomized ISA/A/FP/Sv39/memory differentials, WFI,
   direct Linux, OpenSBI, and modern virt smoke;
3. at least three fresh modern Boot processes with exact shell/output and
   nonzero generated execution; and
4. unchanged Wasm imports/exports and successful validation.

Any semantic, lifecycle, output, or generated-entry mismatch rejects the
candidate before performance.

## Frozen performance gates

The candidate is built once after the edit and reused byte-for-byte.

1. Seven alternating fresh Node/V8 cold compile/instantiate pairs must show no
   more than 5% candidate median compile regression.
2. Run five alternating fresh-process pairs on CPUs 8--15 for Boot, Compile,
   and Python using exact modern scorecard artifacts and R087's ordinary
   `public-one-slice-per-turn` cadence. Every leg must have exact inputs,
   outputs, artifact identity, production policy, generated proof, host spread
   at most 1.10x, and sample spread within the scorecard limit.
3. Boot must have paired median speedup at least 1.03x and paired 95% lower
   bound at least 0.98x. Compile and Python candidate elapsed medians must each
   be no more than 1.03x control. No failed leg is replaced or pooled.
4. If native passes, run the standing fresh-Chrome execution-Boot and WANIX
   `/shared/bench.py` guard with immutable control/candidate archives. Python
   remains a hard no-regression row.
5. Only after browser success, run the untouched corrected-cadence 117-trial
   legacy/rewrite/v86 scorecard. Promotion requires 13/13 legacy, no v86
   parity-count loss, and no guarded rewrite-row regression beyond 3%. A valid
   general 3--5% Boot gain is retained even if the score remains 11/13.

Stop at the first failed gate. Preserve source/artifact/report hashes and the
reason. Do not tune generic boundaries, inlining attributes, callback shape,
row selection, thresholds, or unrelated code after seeing product timings.

## Result and decision

The pre-edit source archive is
`target/bench/r089-monomorphic-reentry/source-before-edit.tgz`, SHA-256
`0dd66a653709e6c558a5e9fc81d2dd78751b6f39d08532e9e603006f5967536e`.
The only candidate source archive is `source-candidate.tgz`, SHA-256
`e91c30e6dcddc4966106314e4c3cbcb07e849e4afb0b1d3a3e529ca6df6c54b8`.
The frozen 4,280,926-byte candidate is
`target/bench/wasm-candidates/r089-monomorphic-reentry-706dd6d5f507.wasm`,
SHA-256 `706dd6d5f50719e7b2c7c963a7fa764ec536d44cc72af5618b14abe099dfa45e`.
It retains 13 imports and 170 exports.

The static shape report is
`target/bench/r089-monomorphic-reentry/shape.json`, SHA-256
`65532026e0a4da323ce46e07de8a548c1782cbdf20195015cbe6a10a16f9ac5a`.
Exact R085 has two `call_indirect` operations in `Cpu::run_until` and one in
the observed counterpart; R089 has zero in both. `run_until` retains four
direct calls and gains the two expected inline index/tag loads and compares.
The mechanism therefore activated exactly as intended.

The complete pinned Nix correctness matrix passes: 134 ISA tests, 109/109
Spike locksteps, 193 architecture-signature comparisons, all core/system/DBT
units, randomized JIT/A/FP/Sv39/memory/T2 differentials, WFI, wasmtime,
direct/OpenSBI Linux, and modern virt smoke. Three additional fresh modern
Boot processes prove exact candidate identity, public cadence, production
policy, and generated execution in report `dacdf7b80b1e...`.

The seven-pair cold report is
`target/bench/r089-monomorphic-reentry/cold-compile.json`, SHA-256
`678eb56f3fac7415173270244385fb0ee20d2f6b1a30842c484ae6cbbb5df40f`.
Median main-module compile is 4.929 ms control and 4.894 ms candidate, a
0.993x candidate/control ratio; the 1.05x cap passes.

The valid 30-leg product report is
`target/bench/r089-monomorphic-reentry/native-ab/config-ab-2026-08-09T20-24-58-152Z.json`,
SHA-256 `fb8691317455a00951ddfb866511d994897f3f5535bb946756563eca91f58057`.
It has no problems, 1.025x host spread, exact modern inputs/outputs, production
policy, corrected cadence, exact artifact identity, and generated proof in
every leg. Paired results are:

- Boot 0.972x, interval `[0.958,0.996]`, 2,214.028 to 2,278.743 ms;
- Compile 1.026x, interval `[1.017,1.047]`, 974.887 to 950.086 ms; and
- Python 1.000x, interval `[0.921,1.030]`, with side medians 2,351.170 to
  2,296.042 ms.

The independent native gate is `native-gate.json`, SHA-256
`e8687455c75073f3fad41e2f14b5426c605fa0604b14a5b2dcff332fd3ea793b`.
Integrity and both guard rows pass, but Boot fails both the frozen 1.03x median
and 0.98x lower-bound requirements. The candidate is rejected without browser
or scorecard escalation and without a typing/inlining variant. This is a
useful engine-shape result: removing a sampled self-time frame does not imply
removing equivalent optimized wall time, and the changed monomorphized code
layout can outweigh the local indirect-call saving during cold Boot.

Both source files and the rebuilt release artifact are restored byte-exact to
R085 (`1da35e70bc9c...`, `dc5d04e4670c...`, and Wasm `efd7830307ef...`).
The loader remains `2cbb264f4dac...`. Production and goal status therefore
remain R087's corrected 11/13 baseline, with Boot and Compile open.
