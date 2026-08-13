# R120 Interleaved Fused-TLB One-Percent Reconfirmation

Date: 2026-08-10
Status: Gate A passed; exact candidate, harness, and decision rule frozen before
new performance timing

## Why this exact candidate is being reconsidered

R100 replaced each generated load/store fused-TLB row's split tag and offset
arrays with same-size 16-byte `{tag, linear_offset}` entries. Generated Wasm
then used one `v128.load`, checked lane zero, and consumed lane one only after
the unchanged proof succeeded. It changed no capacity, hash, permission,
context, miss, invalidation, memory selector, region policy, threshold,
scheduler, guest PC, opcode family, workload, or engine policy.

The exact R100 candidate passed its shape and complete focused correctness
gates. Five current-baseline native pairs measured a `1.017x` Compile point and
the same normalized-work direction, but the then-frozen rule required a
`1.03x` median. R104/D115 has since replaced that coarse economic floor with a
verified net `1.01x` rule. This is a prospective policy correction of exactly
the sort the historical audit is meant to catch.

R120 does not add pairs to R100, pool its observations, or reinterpret its old
confidence interval. The old five pairs are motivation only. R120 makes one
new fixed 15-pair decision using the immutable R100 bytes and today's already-
frozen R104/R107 rules. No implementation, SIMD spelling, alignment, selector,
access family, or threshold may change.

## Frozen identities

- control Wasm:
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`,
  4,279,380 bytes;
- candidate Wasm:
  `c36da489ebe3e2f15d960a1ad393b808e9ff285dc099d4988c745e0e81065b32`,
  4,278,772 bytes;
- candidate source archive:
  `7b239d440da6e1730a5022a87a62443199a3a3a28521d24451515145a05b5c38`;
- loader:
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- Linux 6.12.7 image:
  `57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2`;
- Alpine 3.24.1 initramfs:
  `cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808`;
- old complete R100 native report, excluded from the new statistic:
  `e4ea8ac0c20bd8fc451c66a063b869b79a8b2d03788debcfa4b8bb5ddcf0f368`;
  and
- old R100 gate, excluded from the new decision:
  `d47ffc74b5732f507081390dfab4edcc21de82c0f6d01546d725289da958a82d`.

The archived candidate source identities are:

- `crates/rv64-core/src/cpu.rs`
  `f93b6bd6665e43c5cca4aea180cd6e8e9213ecb18f74daf49a1f258ad971c4cc`;
- `crates/rv64-dbt/src/lib.rs`
  `5d1ff08a65804689fc46738bb7e28f4598dfe38645d1d1ada50aac868485c4d4`;
- `crates/rv64-dbt/src/wasm.rs`
  `b27060eb7c3ca62d952721ddeb4410561552c51d1b30a9637e88614b59b45932`;
  and
- `crates/rv64-wasm/src/lib.rs`
  `7a00146b33c3a85484ca5fbdb96ce8e5f9694f45079ced0dc207f20f7605f8bc`.

The DBT archive predates proof-only R111 diagnostics in the live source. Those
diagnostics compile out of the default release—the restored current build is
still byte-exact `d9f686a9...`. If R120 passes native timing, apply only R100's
runtime interleaving changes onto current source and retain the diagnostic API;
the resulting default release must reproduce exact `c36da489...`.

## Gate A: authenticity, determinism, and prior semantics

Before new timing:

1. Validate both immutable Wasm artifacts with the host engine.
2. Build the archived source twice in isolated fresh target directories and
   require both results to be byte-exact `c36da489...`.
3. Mechanically audit the source delta and confirm it contains only the frozen
   interleaved entry representation, exported layout, SIMD emitter path,
   product layout selection, and their directed tests. R111 diagnostic-only
   source differences are explicitly not product behavior.
4. Reconfirm the frozen R100 shape/correctness record: one vector row load,
   lane-zero proof before lane-one use, separate load/store banks, all focused
   system-memory/Sv39/MPRV/A/FP/bulk/multi-entry/WFI/lifecycle tests, and direct
   plus OpenSBI modern Linux passed on these exact bytes.
5. Freeze the new evaluator and pass its synthetic self-test.

No elapsed value from an isolated build, old R100 run, test, or profiler is
promotion evidence.

Gate A result (completed before performance timing):

- Node's host engine validates both immutable Wasm inputs.
- Two isolated archived-source builds produced byte-identical 4,278,772-byte
  modules, each exactly
  `c36da489ebe3e2f15d960a1ad393b808e9ff285dc099d4988c745e0e81065b32`.
- The candidate matches the immutable R100 artifact byte for byte.
- The archived source delta and prior exact-byte shape/correctness record match
  the scope above.
- The prospective R120 gate's synthetic self-test passes its target,
  sub-one-percent, and protected-regression cases.

The first isolated-build invocation stopped before compilation because its
temporary source tree omitted the repository's `vendor/` input. This was a
setup failure, produced no candidate timing or candidate decision data, and
was corrected by including the same vendored dependency tree. Both actual
isolated builds then reproduced the frozen candidate exactly; neither build's
elapsed time is used as evidence.

## Construction and native gate

Archive exact control/candidate bytes and harness identities before the first
new sample. Run exactly 15 alternating fresh-process main-runtime construction
pairs on CPUs 8--15. Apply R107's nonnegative upper-95% candidate-minus-control
construction debit to every candidate runtime sample.

Then run exactly 15 alternating fresh-process pairs for Boot, Compile, and
Python under the public one-slice cadence and production page policy. Retain
all 90 legs. No retry, replacement, extension, old-sample pooling, threshold
change, or host-outcome selection is permitted.

Compile is the sole target row. Admit only if:

- debited paired Compile speedup median is at least `1.01x` and its paired-
  bootstrap 95% lower bound is at least `1.00x`;
- Compile normalized fixed-work median is at least `1.01x`;
- Boot and Python debited paired medians are each at least `0.99x`, and neither
  interval establishes regression by having its upper bound below parity;
- all identity, guest, output, fixed-work, generated-execution, cadence,
  affinity, sample-count, and host-stability guards pass; and
- Gate A remains exact.

A verified 1--3% Compile gain passes. Candidate size is 608 bytes smaller, but
that fact supplies neither credit nor a shortcut around construction timing.
A favorable point whose lower bound crosses parity is inconclusive and fails
this fixed run.

## Product qualification if admitted

Only a native survivor advances. Reapply the exact R100 delta to current live
source, require two deterministic exact-candidate release builds, then rerun
the complete strict `REQUIRE_ALL=1` correctness matrix and direct/OpenSBI
modern Linux boots.

Next run the fixed Chromium execution-Boot gate and the qualified WANIX guard,
including unchanged `/shared/bench.py`, SHA-256, and long shared-9P rows. Then
run the untouched 117-leg modern legacy/rewrite/copy-v86 scorecard. Promotion
requires all R104 protected-row, artifact, output, generated-coverage, browser,
WANIX, and scorecard guards, 13/13 versus legacy, no loss from 11/13 versus
copy/v86, and a reproducible reduction of the open Compile gap.

At the first failed gate, preserve all evidence, leave current product source
and `d9f686a9...` active, and close the exact interleaved representation. Do
not try a scalar/SIMD mix, another vector spelling, alignment, entry width,
row sharing, access selector, opcode subset, guest mode, engine selector, or
workload-derived variant.
