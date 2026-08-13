# R101: structured fuel-check opportunity census

Status: complete; R102 was justified, evaluated, and rejected at native timing

## Question

Production structured regions currently compare cumulative retirement with the
invocation fuel after every emitted basic-block member.  R018 observed about
39.9 million structured-member entries during Compile STEADY, at only about
7.5 scheduled guest instructions per entry.  The comparison therefore has a
large dynamic footprint even though the scheduler permits retirement to stop
at 128-instruction block granularity.

R101 asks how many of those comparisons could be coalesced without increasing
the existing maximum retirement overshoot.

## Frozen baseline

- accepted product: R085
- clean executable-equivalent Wasm:
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`
- size: 4,279,380 bytes
- modern guest only: Linux 6.12.7, Alpine 3.24.1, no TinyEMU kernel and no BBL
- public scheduler cadence and the frozen scorecard workload/output contracts

R101 instrumentation is opt-in under `PROFILE=1`.  Product modules emitted
with profiling disabled must remain free of counter instructions.  The source
and production artifact are restored to the frozen baseline after the census
unless a separately pre-registered candidate is justified.

## Conservative opportunity policy

Let one safety segment be the guest instructions executed after one fuel
comparison and through the next comparison or generated-function return.  The
current implementation bounds a segment by one lifted member, whose hard
maximum is 128 guest instructions.

The diagnostic computes a conservative independent set of members whose
post-member comparison could be omitted:

1. A member with a dynamic, non-enumerable successor is never selected.
2. A self-looping member is never selected.
3. Two members joined by an internal CFG edge are never both selected.
4. For every selected member and each internal successor, the sum of their
   static retirement counts must be at most 128.
5. Accelerated variable-trip members are not selected.
6. A statically terminating member may be selected; returning to the runtime
   is itself a scheduling boundary, and both generated cross-module transfer
   paths independently test cumulative fuel before entering another function.

These rules leave at most one unchecked member followed by one checked member
on an internal path.  Consequently every possible entry point, branch arm,
cycle, and side exit retains the existing <=128-instruction bound.  R101 only
counts executions of selected members; it does not remove their comparisons.

## Measurements

Run a fresh exact-baseline Boot, Compile PRIME, and Compile STEADY profile with
the frozen guest/workload/cadence.  Record for every phase:

- exact structured member entries;
- exact entries selected by the conservative policy;
- selected-entry fraction;
- scheduled versus actually retired generated instructions;
- existing instruction mix, region calls, dispatches, and chain hops;
- artifact, loader, guest, output, and environment integrity.

The added selected-entry counter perturbs wall time, so profile elapsed times
are not performance evidence.  Only execution counts and integrity are used.

## Decision rule

There is no arbitrary byte limit or minimum percentage used to reject the
mechanism.  The census either demonstrates a meaningful dynamically active
cost center or it does not.  If it does, a separate R102 protocol will freeze
the exact lowering, correctness invariants, artifacts, paired sample count,
and performance gates before candidate timing.  Acceptance will depend on
measured correctness and statistically supported whole-row improvement, with
Boot and `/shared/bench.py` non-regression; code size remains diagnostic.

## Result

The first setup invocation used the obsolete `PROFILE=1` spelling. The v2
worker correctly ignored it, so that ordinary unprofiled report is excluded
from this census. The single corrected invocation used
`SCORECARD_V2_PROFILE=1`; the scorecard marks profile timing proof-only by
design, and its only two problems say exactly that the Boot and Compile
profiles entered a timed harness. Guest identity, production policy, public
cadence, runtime/loader hashes, generated execution, phase completion, and
workload outputs all passed their worker contracts.

The diagnostic Wasm is `8a75ec969f4ea230586b3d557f8d01371aaef9878421fb08e48e3be847a6ae71`
(4,294,159 bytes). The report is
`target/bench/r101-structured-fuel-census/profile/scorecard-v2-2026-08-10T04-53-36-748Z.json`,
SHA-256 `d965da38bc755f8a7678b03302f37c7943ba5c211f939050ed4c777fa119f2b4`.

| Phase | Member entries | Coalescible entries | Share | Scheduled insns/member | Four-op checks represented |
|---|---:|---:|---:|---:|---:|
| Boot FIRST | 11,565,216 | 1,066,218 | 9.219% | 6.380 | 4,264,872 |
| Compile FIRST | 36,844,567 | 13,664,198 | 37.086% | 7.789 | 54,656,792 |
| Compile PRIME | 39,774,118 | 14,851,855 | 37.341% | 7.703 | 59,407,420 |
| Compile STEADY | 39,972,369 | 14,873,571 | 37.210% | 7.705 | 59,494,284 |

Compile STEADY retired 309,117,066 generated instructions against 307,993,062
scheduled instructions. The small excess is expected from the independently
fuel-bounded bulk-copy helper; variable-trip members were excluded from the
coalescing set.

## Decision

This is a dynamically broad, architecture-general Compile mechanism. It does
not select benchmark PCs, opcodes, address ranges, privilege levels, or host
engine tiers. R102 is authorized to remove exactly the four-instruction
comparison at the precomputed selected members and no others. Boot is a guard
row because its exposure is only 9.2%; Compile is the motivated row. R102 must
freeze its complete gate before timing, prove the <=128-instruction segment
invariant structurally and dynamically, and stop on the first failed gate.

R102 subsequently removed exactly the selected comparisons and passed its
complete correctness and cold-construction gates, but its valid seven-pair
native Compile result was 0.997x paired speedup with `[0.978,1.083]` interval
and 0.997x normalized MIPS. The dynamic count was real, but the operators were
not a measurable standalone cost after the host Wasm engine optimized the hot
generated functions. R102 was rejected and exact baseline restored; see
`R102_BOUNDED_STRUCTURED_FUEL_COALESCING_PROTOCOL.md`.
