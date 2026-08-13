# R087 Public Scheduler Cadence Protocol

Date: 2026-08-09  
Status: corrected public cadence promoted as the authoritative harness baseline; product parity remains open

## Question

Does the authoritative scorecard still bias asynchronous RV64 JIT publication
by executing four 2,000,000-instruction RV64 slices per JavaScript event-loop
turn, and should its scored default match the one-slice cadence used by both
the public RV64 scheduler and copy/v86?

This is a benchmark correctness repair, not a JIT optimization. Product Rust,
generated modules, loader behavior, guest bytes, thresholds, policy, workload,
and v86 remain unchanged. A result from the corrected harness establishes a
new comparison baseline and is never added to R085 as a code-generation gain.

## Frozen evidence and identities

The production runtime remains exact promoted R085:

- runtime Wasm
  `efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`;
- loader
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- authoritative R085 scorecard
  `d733df2124a7388876f6566db71b6f67bf7cdc4dccc2ece5e1df2903c27d7479`.

Before this edit, the scorecard worker is
`cb9413c923db9d28e7fed4c1044a306d8bc09c366a6179a6e8cef42ebbaea3d9`.
Its `waitUntil` yields only on iterations 0, 4, 8, and so on unless the
diagnostic environment switch is present. The current public loader is the
exact hash above and its `Emulator` scheduler executes one 2,000,000-
instruction slice before `hostYield`. The pinned v86 runner calls `run()` once
and v86 schedules subsequent CPU work asynchronously.

R063's immutable same-Wasm diagnostic report is
`target/bench/r063-yield-every-pump-screen-valid/config-ab-2026-08-09T02-01-24-637Z.json`,
SHA-256 `6ba49a1b0e63cc98fd641aaefbb3bfa17fd73aa6507b52b81f3b8a3c6d1e2304`.
It measured paired Boot 1.008x, Compile 1.060x, and Python 1.246x with exact
inputs/outputs and 1.019x host spread. Those samples motivate this correction
but are not pooled with R087.

## Harness change

Make one RV64 pump per event-loop turn the ordinary scored default. Preserve
the old four-pump cadence behind the explicit diagnostic-only environment
variable `SCORECARD_V2_HISTORICAL_BATCHED_PUMPS=1`. Preserve
`SCORECARD_V2_YIELD_EVERY_PUMP=1` as a legacy explicit diagnostic so old
protocols remain reproducible, even though its behavior now equals the
default. Reject contradictory cadence variables.

Every worker result must record its cadence. The normal default is
measurement-eligible; either explicit diagnostic is not. The change applies
uniformly to Boot, workload preparation, FIRST/PRIME/STEADY, settling, rewrite,
legacy, and the no-op v86 wait predicate. No row, phase, side, PC, opcode,
guest, artifact, output, or timing value may select cadence.

Add a pure cadence helper and self-test proving:

1. default and legacy-explicit modes yield on every pump;
2. historical mode yields exactly on iterations 0, 4, 8, ...;
3. default is eligible while both explicit modes are diagnostic;
4. conflicting or invalid variables fail closed; and
5. the worker imports the helper rather than duplicating cadence logic.

## Fresh causal check

After syntax and harness self-tests pass, run five alternating fresh R085
control/candidate pairs on CPUs 8--15 for Boot, Compile, and Python:

- control `SCORECARD_V2_HISTORICAL_BATCHED_PUMPS=1`;
- candidate ordinary corrected default.

Use exact Linux 6.12.7 / Alpine 3.24.1 scorecard inputs and the same R085 Wasm
in all 30 legs. Require a valid report, exact fingerprints and inputs, complete
generated-execution and production-policy proof, at most 1.25x host/sample
spread, and no candidate row more than 3% slower by paired median. This check
tests the size and safety of the correction; symmetry with the public product
is the reason for promotion, so a neutral valid result does not restore the
known-unfair cadence.

The prior R063 samples may not be pooled, no leg may be replaced, and no pump
count, slice size, row, statistic, or guard may change after collection.

## Corrected authoritative baseline

If the causal check is valid and its guard passes, run the complete untouched
117-trial legacy/rewrite/v86 scorecard with the corrected default. Require all
existing measurement, modern-guest, workload, JIT-proof, output, artifact,
host-spread, and sample-spread gates. Record the cadence in the top-level
configuration and every trial.

The thread goal remains 13/13 versus legacy and 13/13 versus v86, with
`python /shared/bench.py` non-regression. Because product bytes do not change,
R085's strict correctness and browser/WANIX evidence remains applicable; verify
byte identities after the harness edit. The corrected scorecard determines the
new Boot/Compile gaps and therefore which product optimization is next.

## Result and decision

The pure cadence helper, worker integration, scorecard validation, and
self-tests are now frozen as:

- `scorecard-v2-cadence.mjs` `ed7b65cab81f...`;
- `scorecard-v2-worker.mjs` `abe22741809f...`;
- `scorecard-v2.mjs` `d329d7746987...`; and
- `scorecard-v2-selftest.mjs` `8b7acf55effc...`.

The fresh 30-leg same-Wasm causal report is
`target/bench/r087-public-cadence-ab/config-ab-2026-08-09T19-04-00-092Z.json`,
SHA-256 `4699b97692b193558b90e54b3e0090e31e4c5f2b066861d28e341d20ad59b302`.
Its frozen gate is `target/bench/r087-public-cadence-ab/gate.json`, SHA-256
`c3bd1f9c6ddcfa11c56b0557acdbd487beb87e8769056f5dba87db78d1b565c5`,
and passes with no problems. Corrected/default versus historical cadence
measured paired speedups:

- Boot 1.012x, interval `[0.971,1.037]`;
- Compile 1.064x, interval `[1.050,1.101]`; and
- Python 1.296x, interval `[1.198,1.373]`.

All 30 legs used exact R085 bytes, production policy, exact modern guest
inputs and outputs, and nonzero generated-execution proof. The change is
therefore retained for comparison fairness even though Boot's causal interval
includes parity. No product source or Wasm changed.

The complete corrected-cadence authoritative report is
`target/bench/r087-authoritative-three-way/scorecard-v2-2026-08-09T19-59-20-960Z.json`,
SHA-256 `1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7`.
Its independent gate is
`target/bench/r087-authoritative-three-way/gate.json`, SHA-256
`d106c8ecf63c56c3aa109621356fa37045b0a5aa8ed0f4c882230c9d80c98d16`,
and passes: all 117 trials are eligible, the report is authoritative and
measurement-valid, every result records `public-one-slice-per-turn`, modern
Linux/Alpine identities and output proofs match, and host/sample spread stays
within the frozen limits.

R087 remains 13/13 versus legacy and 11/13 versus copy/v86. The only losses
are Boot, 2,176.495 versus 1,563.646 ms, and Compile, 954.483 versus 728.859
ms. They require 28.16% and 23.64% reductions in rewrite elapsed time,
respectively. Python improves from R085's historical-cadence 3,034.673 ms to
2,346.773 ms and is 1.473x faster than v86's 3,456.560 ms. Relative to the old
R085 scorecard, the corrected harness reports rewrite speedups of 1.032x Boot,
1.065x Compile, and 1.293x Python; these are baseline corrections and are not
added to R085's product gain.

Promote the corrected cadence as the only ordinary scored default. Keep both
explicit cadence variables diagnostic-only. Product remains exact R085 Wasm
`efd7830307ef...`; the performance objective remains open at 11/13, with Boot
and Compile as the only unfinished rows.
