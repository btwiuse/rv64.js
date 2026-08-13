# R086 Exact-R085 Residual Profile Protocol

Date: 2026-08-09  
Status: diagnostic protocol frozen before collection

## Question

After promoting R085's faster randomized JIT-state hash tables, what distinct
architecture-general host operation still owns at least 3% of either open
modern scorecard row? Default Rust hashing and its measured replacement are
now closed. This collection may select a subsequent experiment, but its
inspector-perturbed durations are never performance evidence.

The exact promoted identities are:

- runtime Wasm
  `efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`;
- loader
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- production archive
  `0b953be67610e130f79a852f86542c8400ad3a235001ec450fbdffc29ed3a61a`.

R085's valid authoritative medians are Boot 2,245.209 ms versus v86
1,594.247 ms and Compile 1,016.241 ms versus 737.385 ms. The score is 13/13
against legacy and 11/13 against v86. Python remains a hard non-regression
row.

## Frozen collection

Use CPUs 8-15 and the exact Linux 6.12.7 / Alpine 3.24.1 scorecard artifacts.
Collect one fresh rewrite process for Boot and Compile. Capture a V8 CPU
profile inside Boot FIRST and Compile FIRST, PRIME, and STEADY at a fixed
250-microsecond interval. Keep the proof-only scorecard report and every raw
profile. Do not quote or compare the resulting wall times.

The v86 runtime, loader, guest, and profile mechanism did not change. Reuse
R081's immutable v86 profiles as shape-only references rather than spending
another measurement:

- Boot `target/bench/r081-r080-engine-profile/phase/v86-boot-first.cpuprofile`;
- Compile FIRST/PRIME/STEADY in the same directory.

Current tool SHA-256 identities, unchanged from R081, are:

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
  SIDES=rewrite ROWS=boot,compile REPS=1 \
  SCORECARD_V2_OUTPUT="$PWD/target/bench/r086-r085-engine-profile/report" \
  SCORECARD_V2_ENGINE_PROFILE_DIR="$PWD/target/bench/r086-r085-engine-profile/phase" \
  SCORECARD_V2_ENGINE_PROFILE_INTERVAL=250 \
  node tests/vs-v86/scorecard-v2.mjs
```

Afterward, run the unchanged analyzer over the four new rewrite profiles and
the four archived v86 profiles, writing one immutable JSON report.

## Closure-aware classification

Reconstruct every sampled stack. For Boot and Compile STEADY, attribute leaf
self time to complete operation families and explain at least 95% of the
phase. FIRST and PRIME are lifecycle cross-checks. Report the nearest
non-library caller and the first caller below `run_system_jit`.

Explicitly separate the new `FastHasher`/`FastHashMap` frames from default
`RandomState`/hashbrown frames. Any residual hashing is closed unless a fresh
whole-profile attribution contradicts R085's measured product gain; no seed,
multiplier, capacity, key-shape, or map-subset variant is admissible.

Also subtract the previously closed families through R085: fetch and TLB
caches, helper partitioning, const system specialization, scalar-driver and
static/external tier layouts, scheduler thinning, module geometry and tier
thresholds, worker pools, entry ranking, structured local allocation, stack
proof carry, generated-memory proof packing/SIMD, multi-latch/batch selection,
and workload-specific paths.

An implementation may be admitted only when one coherent, selector-free
operation has a measured whole-row ceiling above 3% and a realistic local
projection of at least 3%. A new execution tier or substantial lifecycle must
project at least 5%. Guest PC, symbol, binary, compiler output, URL, workload,
checksum, and browser identity may diagnose concentration but may never select
behavior. If nothing qualifies, record the exact local plateau and move to a
larger execution representation rather than reopening a closed family.

## Result

The frozen collection completed with exact R085 runtime bytes and the four
requested rewrite profiles. The proof-only scorecard report is
`target/bench/r086-r085-engine-profile/report/scorecard-v2-2026-08-09T18-41-49-253Z.json`,
SHA-256 `708d9b66f35cc88e10120c787b23e2c454b2c7729df5493e032212a8aebafa4d`.
Its wall times are inspector-perturbed and remain excluded. The raw profile
SHA-256 values are:

- Boot FIRST `c6e368eb89cb9e2a306527b899a2a21c8e882133f4886a1d677122e2117ac4e9`;
- Compile FIRST `db458f5122d2e25d87cd53d0628de02fc77fa1c11428069a65e60904115f133b`;
- Compile PRIME `fab4486b07b21306b7868c02ae922d8bcf0f5fa453d463ac2c9afea7363d0f69`;
- Compile STEADY `93220dbd775623042f52368e5bcb8ec61f74afcee94c06d52c1866ada1c57ac0`.

`tests/vs-v86/analyze-r086-residual.mjs` reconstructs every sampled stack,
records the first child below `run_system_jit` and nearest project caller, and
regenerates deterministically. Its SHA-256 is
`efce26fcf275550efaa26c2d95d22e4e71d2243b189db2c9d2245b92e6181be5`.
The resulting closure report is
`target/bench/r086-r085-engine-profile/closure-analysis.json`, SHA-256
`c45991049a343509b3f9487661d71d55863813bc100920e57b326c4548c5ce66`.
The smallest descending family prefixes explain 95.155% of Boot and 95.131%
of Compile STEADY.

Boot sampled 2,279.784 ms. Exclusive whole-phase families are interpreter
decode/execute 53.16%, exact generated re-entry loop 11.66%, scalar memory
helpers 9.22%, translation/issue 5.17%, generated execution 4.31%, scheduler
self 4.28%, and the ordinary interpreter loop 3.44%. Compile STEADY sampled
1,542.036 ms: generated execution is 47.28%, interpreter decode/execute
18.85%, scheduler self 11.65%, translation/issue 6.69%, final-outcome driver
3.30%, and exact generated re-entry 2.96%.

R085's remaining fast-table mechanics are only 1.304% of Boot and 0.894% of
Compile STEADY; default/non-JIT tables are 0.120% and 0.020%. This confirms the
hash family is closed. No new local implementation clears the frozen rule
after subtracting the tested interpreter, scheduler, translation, generated
layout, and lifecycle families. The exact re-entry loop is material, but R056
already measured its call-shape mechanism; applying R056's 1.494x local result
to R086's exclusive samples projects 1.040x Boot and 1.010x Compile, not a new
R086 discovery. R086 therefore makes no product edit.

The source audit also reconfirmed a benchmark-methodology debt outside this
local-operation protocol: the scorecard still batches four RV64 slices per
event-loop turn while the public RV64 scheduler and v86 yield after each
slice. R063 measured a general 1.060x Compile effect but retained the mismatch
because the historical promotion floor was 1.10x. The project owner has now
explicitly reaffirmed the cumulative 3--5% policy. R087 prospectively tests
and corrects this comparison cadence before another product candidate is
judged; it does not credit a harness correction as JIT speedup.
