# R092 Member-Range Reconfirmation Protocol

Date: 2026-08-09  
Status: rejected at the frozen WANIX regression guard; exact R085 restoration in progress

## Why this closed mechanism may be retested once

R016 tested whole-member memory-range versioning under the former fixed 10%
advancement rule. It used a fixed architecture-level selector: at least three
ordinary loads/stores share one `ReadX` address root, their constant-offset
span fits one system page, and the member is not already handled by dense-copy,
dense-store, or bulk-copy lowering. One outer bounds/permission proof guarded
a cloned direct-linear-memory arm; the unchanged member was the fallback.

The six frozen reports are:

- controls: `9c0e4db193c4...`, `3ebd4a085205...`, and `969b7404070d...`;
- candidates: `f3a83551199b...`, `ad83105c5ab42...`, and
  `cd24eea67a0f...`.

All used main Wasm `ce5227c5142f...`, modern Linux 6.12.7 / Alpine 3.24.1,
production page policy, CPUs 8--15, and exact Compile MD5
`24eedf7e06beffd4d3ba1945585588db`. Alternation was control/candidate,
candidate/control, control/candidate. Median FIRST was
3,664.155/3,711.308 ms, PRIME 1,238.291/1,190.280 ms, and STEADY
1,111.450/1,019.576 ms. Thus FIRST regressed 1.29%, PRIME improved 4.03%
in throughput, and STEADY improved 9.01% in throughput (8.27% lower elapsed
time). Median selected candidate members were 270 and generated coverage was
92.38%/92.39%.

This is admission evidence only. The controls are valid, but the three
candidate reports are correctly marked proof-only because R016 toggled the
feature diagnostically inside the same Wasm. They cannot be retroactively
promoted. R080 prospectively replaced the old 10% single-change rule with the
standing cumulative 3--5% rule, and R091 explicitly froze the current 1.03x
product threshold before this reconsideration. R092 therefore permits one
independent confirmation against the exact current baseline; it does not
reinterpret the old reports as authoritative.

## Frozen control and candidate

The immutable control is promoted R085 Wasm
`efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`,
loader `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`,
and corrected authoritative report `1d822f1c1f37...`. Pre-edit source hashes
are:

- `crates/rv64-dbt/src/lib.rs`: `fed2e33326d5...`;
- `crates/rv64-dbt/src/wasm.rs`: `b5e9c11ec1bf...`; and
- `crates/rv64-wasm/src/lib.rs`: `1da35e70bc9c...`.

Construct exactly one separately hashed, default-on product candidate. Restore
the frozen R016 mechanism with these immutable rules:

1. minimum three ordinary accesses sharing one architectural `ReadX` root;
2. choose the largest group, with deterministic SSA-root tie breaking;
3. include complete access width in the constant-offset span and require the
   span to fit the configured system page;
4. exclude conditional stores and the existing dense-copy, dense-store, and
   bulk-copy members;
5. prove same-page bounds once, then independently prove the existing load
   and/or store permission row once before entering the direct arm;
6. retain the exact ordinary emitted member as the fallback, including refill,
   side-exit, exception, retirement, and state behavior; and
7. expose only translation-count telemetry. Do not add a runtime or harness
   feature switch.

Do not tune the access floor, root register, page/span rule, access classes,
clone shape, proof order, workload, PC, opcode, symbol, or threshold after any
timing. R017/R019/R049 already close broader carried-region variants.

## Shape, opportunity, and correctness gates

Before product timing:

1. format and all `rv64-dbt`/`rv64-wasm` unit tests pass; generated modules
   validate and a directed shape test distinguishes candidate from control;
2. a deterministic full-system differential must execute at least five
   selected structured members and nonzero generated instructions, reach
   poweroff, and match interpreter registers, memory, and PC;
3. the complete T2 multi-entry, system-memory, Sv39/MPRV, atomic, FP, WFI, and
   randomized differential set passes;
4. one exact modern Compile run has nonzero selected members in every scored
   phase, exact MD5, production policy, public one-slice cadence, and nonzero
   generated execution; and
5. seven alternating fresh-process main-Wasm compile/instantiate pairs may
   regress by no more than 5%.

If selection is zero, semantics differ, or the intended direct arm is absent,
stop and restore R085. No selector variant is allowed.

## Native and promotion gates

Archive the candidate once by source and Wasm hash. Run five alternating fresh
Node/V8 pairs for Boot, Compile, and Python on CPUs 8--15 using exact modern
artifacts, production page policy, and R087 public cadence. All 30 legs must
retain exact identity/input/output/JIT proof, host spread at most 1.10x, and
ordinary sample limits.

The candidate advances if Compile's paired median speedup is at least 1.03x
and its paired 95% lower bound is at least 0.98x. Boot and Python must each
have candidate/control elapsed medians no greater than 1.03x. This is the
already-established cumulative rule, not a threshold selected from R016.

If native passes, run the complete pinned `REQUIRE_ALL=1 tests/run-all.sh`
matrix, at least three additional fresh modern Boots, the standing fresh
Chrome execution-Boot guard, and the WANIX `/shared/bench.py` guard. Only then
run the untouched corrected-cadence 117-trial legacy/rewrite/v86 scorecard.
Promotion requires 13/13 versus legacy, no loss from 11/13 versus v86, every
rewrite row within the 3% guard, exact browser Python non-regression, and a
reproducible Compile improvement over R085. Otherwise retain the evidence and
restore exact R085.

## Result and decision

The one permitted product candidate was frozen before timing as Wasm
`5baeccb5c5feaf2d3f7605fd42f741f9cbaa89e566a86c0bbea201a3c6389023`
(4,294,006 bytes), with source archive
`51112af62ab7951c704b097876f152193647609f8f8907e7693d6a8835297398`.
The separately packaged WANIX archive is
`e0488c48b3ea398060a42854715883eea4b34c96a13d397c5068f8a8e1c4029b`;
its loader and adapter are byte-identical to R085 and its inner Wasm has the
candidate identity above.

Shape, opportunity, correctness, and native gates passed:

- the directed full-system differential selected five members, landed one
  generated module, retired generated instructions, and matched interpreter
  registers, memory, PC, and poweroff;
- the exact modern Compile opportunity report
  `9469c377e2619571cfd3a5cc33cec1e901940e3f3f865fce15cedbcfddcd3e0e`
  selected 1,154/379/287 members in FIRST/PRIME/STEADY and preserved the exact
  output MD5;
- the cold-module report `088a4722184f...` measured 0.979x candidate/control
  elapsed and passed the 5% cap;
- the valid 30-leg native report `4badb7a4602d...`, interpreted by corrected
  gate `a54dcb1472c9...`, measured Boot elapsed 1.026x, Compile speedup 1.132x
  with interval `[1.030, 1.136]`, Python elapsed 1.009x, and host spread
  1.023x; and
- the complete strict correctness matrix and three additional ordinary,
  measurement-valid modern Boots passed.

The first native gate output `ba8eda...` is retained but invalid as a gate
decision: its script incorrectly required the intentionally proof-only child
legs of an artifact-override A/B to be independently measurement eligible.
The immutable 30-leg report itself was valid; the corrected gate changed only
that integrity interpretation and did not change a performance threshold.

The seven-pair direct Chrome Boot guard passed at 0.974x paired speedup with
95% interval `[0.954, 1.017]`; report `b7fd63d3194a...`. The subsequent frozen
seven-by-three WANIX guard was measurement-valid but failed:

| row | R085/R092 paired median | 95% interval |
|---|---:|---:|
| shell | 0.993x | `[0.992, 1.000]` |
| Python | 1.025x | `[1.010, 1.063]` |
| SHA-256 | 0.989x | `[0.981, 0.990]` |
| shared 9P | 0.810x | `[0.741, 0.894]` |

The WANIX protocol and analysis are `5255ea2f856f...` and
`73aef718769a...`. All 126 timed workload results were correct and every
artifact identity matched. Shared 9P missed both its fixed 0.971x median and
0.909x lower-bound guards, with R092 slower in six of seven pairs. This is a
promotion failure even though Compile improved materially and Python did not
regress. No untouched 117-trial scorecard is run after a frozen browser guard
failure.

Decision: reject R092, preserve all evidence and the unique candidate, and
restore byte-exact R085 source/Wasm/loader/archive. Do not tune the access
floor, access classes, mode, or clone shape using these observed workload
results; any future memory-versioning design requires a separately motivated
protocol rather than a post-hoc R092 variant.
