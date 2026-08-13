# R081 R080 Residual Engine Profile Protocol

Date: 2026-08-09  
Status: diagnostic protocol frozen before collection

## Question

What host execution components now own the two remaining copy/v86 parity
losses on the clean R080 baseline? R055's last accepted-baseline profile
predates the R066-R080 experiment sequence, and R067 profiles a rejected
integrated interpreter. Neither may select the next R080 implementation
without a fresh check.

R080's valid authoritative raw gaps are:

- Boot: 2,338.368 ms rewrite versus 1,562.155 ms v86, 1.497x slower;
- Compile: 1,058.842 ms versus 718.276 ms, 1.474x slower.

The objective is attribution, not a timing claim. Inspector profiling changes
V8 execution and makes every trial measurement-ineligible.

## Frozen inputs and collection

Use ordinary production policy and exact source-built R080 Wasm
`e5415db83b27b32a1f525af2aa19e93539332a274068e389a1e28ebba41d8095`
with loader
`2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.
Use the same Linux 6.12.7 / Alpine 3.24.1 modern scorecard artifacts as the
authoritative R080 report and CPUs 8-15.

Collect one fresh process for each rewrite/v86 Boot and Compile leg. Capture
V8 CPU profiles inside every exact measured phase at a fixed 250-microsecond
sampling interval. Compile supplies FIRST, PRIME, and STEADY; Boot supplies its
single FIRST boundary. Keep all profiles and the proof-only scorecard report.
Do not compare their wall times as performance results.

Frozen tool identities are:

- scorecard driver
  `fda5fe33ae03d5c8d36c3da0287af1ca9e56310bc1240f9f77a60d9d9bc4ae14`;
- worker
  `cb9413c923db9d28e7fed4c1044a306d8bc09c366a6179a6e8cef42ebbaea3d9`;
- scorecard library
  `8681b09f81f3c71e30945d5770486517d993c712b2394c017b4980d481e31c61`;
- profile analyzer
  `25c7f59a9e416d608670895125e6fe849e60d78177cb795af0dae08d2c69b7d5`.

The fixed command is:

```sh
taskset -c 8-15 env \
  ARTIFACTS=/home/darren/src/arm64.js/target/bench \
  SIDES=rewrite,v86 ROWS=boot,compile REPS=1 \
  SCORECARD_V2_OUTPUT="$PWD/target/bench/r081-r080-engine-profile/report" \
  SCORECARD_V2_ENGINE_PROFILE_DIR="$PWD/target/bench/r081-r080-engine-profile/phase" \
  SCORECARD_V2_ENGINE_PROFILE_INTERVAL=250 \
  node tests/vs-v86/scorecard-v2.mjs
```

## Analysis and admission boundary

For each phase, report total sampled time, runtime Wasm, generated Wasm,
JavaScript, V8/native, GC, idle, the top runtime self frames, and the complete
`run_system_jit` subtree. Combine the profile only with exact same-trial guest
retirement, generated/interpreted retirement, dispatch, module, byte, and
translation counters. Compare rewrite and v86 component *shape*; do not claim
the profiled duration is a speed ratio.

A next implementation must target one coherent architecture-general component
with at least 3% measured whole-row leverage, or at least 5% when it adds a new
execution tier or substantial lifecycle complexity. It must differ causally
from the closed fetch caches, scalar-driver layouts, module-geometry/threshold
sweeps, entry ranking, stack/proof caches, scheduler thinning, and static-T0
forms. Profile PCs, module URLs, kernel symbols, compiler addresses, and
workload identity may diagnose concentration but may not select behavior.

Before implementation, write a separate protocol with exact semantics,
correctness gates, five alternating R080/candidate pairs, the standing
cumulative confidence and non-target limits, browser candidate/control plus
v86 guards, and full-scorecard promotion conditions. If no distinct component
has sufficient leverage, record the plateau instead of reopening a closed
family.

## Result

Collection completed once under the frozen command. The scorecard driver
returned nonzero as expected because inspector-enabled trials are proof-only;
all four requested legs and all FIRST/PRIME/STEADY phase profiles completed.
The immutable report is
`target/bench/r081-r080-engine-profile/report/scorecard-v2-2026-08-09T15-09-27-544Z.json`
(`10c30763bb65...`). The parsed analysis is
`target/bench/r081-r080-engine-profile/profile-analysis.json`
(`ed942beb6e6d...`). These wall times are not scorecard measurements.

R080 Boot sampled 2,321.638 ms: runtime Wasm owns 2,168.720 ms (93.41%)
and generated Wasm only 113.054 ms (4.87%). `Cpu::step` alone owns
1,167.209 ms (50.28% of the whole phase), followed by `Cpu::run_until`
279.932 ms, scalar loads 122.805 ms, the scheduler itself 96.076 ms, scalar
stores 72.431 ms, and `Cpu::run` 69.162 ms. The complete
`run_system_jit` subtree owns 2,254.649 ms (97.11%): policy/interpreter work
1,782.626 ms, final-outcome work 111.827 ms, translation/issue 111.580 ms,
generated execution 109.990 ms, scheduler self 96.076 ms, and hashing
22.040 ms. Exact counters report 180,746,109 guest instructions,
69,461,253 generated (38.43%), 111,284,856 interpreted, 519,997 dispatches,
507,988 interpreter calls, ten modules, 102.9 ms guest translation, and
543 ms host module compilation.

copy/v86 Boot has a materially different shape: of 1,635.485 sampled ms,
runtime Wasm owns 1,148.351 ms (70.21%) and generated Wasm 347.591 ms
(21.25%). The remaining parity deficit is therefore execution coverage and
runtime-interpreter throughput, not host module compilation alone.

R080 Compile STEADY sampled 1,640.621 ms: runtime Wasm is 860.485 ms
(52.45%) and generated Wasm 761.890 ms (46.44%). Its scheduler subtree is
1,563.806 ms (95.32%): generated execution 716.330 ms, policy/interpreter
290.821 ms, final-outcome work 235.835 ms, scheduler self 177.936 ms,
translation 113.059 ms, and hashing 26.153 ms. Exact counters report
325,772,269 guest instructions, 301,013,599 generated (92.40%), 24,758,670
interpreted, 506,088 dispatches, 313,255 interpreter calls, twelve modules,
106.97 ms translation, and 529 ms host compilation. copy/v86 Compile STEADY
spends 572.379 of 812.717 sampled ms (70.43%) in generated Wasm and
201.286 ms (24.77%) in runtime Wasm.

One additional native-shape diagnostic resolves an important false lead.
R080's dominant full-system `Cpu::step` is Wasm function 1433 (10,886-byte
Wasm body). V8 emits both a 26,880-byte Liftoff body and a 17,280-byte
TurboFan body, so the interpreter is not stranded in baseline code. The raw
`--print-wasm-code` log is preserved at
`target/bench/r081-r080-engine-profile/native-shape/worker.log`
(`fdd27e720285...`); the extracted symbol listing is `fae0bf867fe4...`.

The profile admits two causally distinct directions. A small cumulative
candidate may specialize the already-monomorphic full-system interpreter so
its hot fetch/load/store path does not repeatedly test `Option<SysCsrs>`;
because `Cpu::step` owns half of Boot, a roughly 6% local improvement can
clear the 3% whole-row floor. The higher-leverage direction is a clean
auxiliary scalar tier whose emitter/compiler is not linked into the main
runtime. R076 measured that execution mechanism at 1.175x in Chrome, while
R078 independently proved the old dormant integration made Boot 1.178x
slower. This is a measured engine/runtime cause change and permits a new
external/no-residue representation; it does not permit restoration of the
old linked emitter, thresholds, entry classifier, or lifecycle.
