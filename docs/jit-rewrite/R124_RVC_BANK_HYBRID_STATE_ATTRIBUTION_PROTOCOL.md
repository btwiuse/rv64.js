# R124 RVC-Bank Hybrid Structured-State Attribution Protocol

Date: 2026-08-10  
Status: owner-directed proxy-gate amendment frozen after A1 and before any A2
dynamic evidence; no product change

## Question

Can production structured regions reduce optimized-native frame pressure by
retaining only RV64C's architecture-defined compact integer-register bank
across members, while materializing the remaining integer registers at member
boundaries, with enough net operation and ordinary-V8 evidence to support a
verified 1% Compile gain?

R110 measures explicit optimized guest-body stack traffic with an 8.87%
whole-Compile ceiling. R116 proves that the zero-extra-work single-member
subset removes only 2.04% of declared locals. The next useful trade is broader:
pay bounded canonical-memory traffic for state that otherwise remains live
through the complete generated function.

## Frozen architecture rule

For `RegisterStructured` functions only, the resident integer set is exactly:

```text
x1, x2, x8, x9, x10, x11, x12, x13, x14, x15
```

RV64C assigns the three-bit compact register encoding to `x8`--`x15` and has
dedicated compressed control/stack forms involving `x1` and `x2`. The rule is
therefore fixed by the advertised `rv64gc` ISA, not by a guest PC, binary,
symbol, workload, observed frequency, compiler output, or host engine. It is
applied unchanged to code with or without compressed instructions.

Every referenced resident register keeps the current eager entry load,
function-local value, internal forwarding, precise-exit behavior, and common
exit commit. For every other integer register:

1. each member's first architectural `ReadX` loads the canonical CPU cell;
2. SSA forwarding within that member is unchanged;
3. each final member output stores the canonical cell before an internal edge;
4. a precise side exit stores only the current member's exact dirty snapshot;
   previously completed members are already canonical; and
5. no function-entry local or common-exit store is allocated for that state.

Floating-point registers, `fcsr`, PC, retirement, fuel, selectors, memory
proofs, helpers, CFG structure, region/function/module geometry, tail chaining,
thresholds, and publication remain exact current behavior. The implementation
must preserve bulk-copy and every precise fault path rather than disabling an
existing lowering.

This is not R039 temporary-local reuse, R103's full-GPR cross-module ABI,
R111 function partitioning, R116's selected single-member cold set, or R117
module globals. The deleted legacy E015 per-register profile supplies no
performance credit and may not alter the fixed bank.

## Gate A1: immutable static corpus

Before product implementation or candidate timing, analyze the immutable
R111 56-region / 6,258-member compiler-produced corpus. Two independent runs
must be byte-identical and reproduce every manifest/module identity. For each
region report:

- current integer/FP/`fcsr` state unions;
- resident and materialized integer masks;
- current decoded structured-function locals by type;
- projected locals after removing materialized integer state locals;
- current entry/exit integer operations per invocation;
- proposed resident entry/exit operations; and
- proposed materialized reads/outputs per member.

A1 originally required all modules to validate, no referenced state to be
unclassified, and eager-byte-weighted total declared locals to fall by at
least 10%. It passed at 13.2938%. This is opportunity evidence, not a runtime
projection or performance credit.

## Gate A2: measurement-ineligible dynamic operation census

Only after A1 passes, add proof-only counters behind the existing disabled
structured-profile capability. On every actually executed path count:

- current GPR entry loads and exit/precise-exit stores;
- the subset belonging to the frozen resident bank;
- proposed materialized member reads and normal outputs; and
- proposed materialized precise-exit snapshot stores.

Run exactly one modern Linux 6.12.7 / Alpine 3.24.1 Compile scorecard worker
with production page policy and public cadence. FIRST, PRIME, and STEADY must
produce MD5 `24eedf7e06beffd4d3ba1945585588db`, prove generated execution and
settled compilation, and close existing member/boundary totals. Counter code
perturbs tiering, so every elapsed value is excluded.

For each phase compute:

```text
projected hybrid GPR operations =
  resident entry loads + resident exit stores +
  materialized member reads + materialized normal outputs +
  materialized precise-exit stores
```

The original A2 admission targets were at most 80% for PRIME/STEADY and 85%
for FIRST. Record those targets and whether they are met, but do not use a miss
as a standalone rejection. The owner clarified before any A2 execution that a
verified net product gain as small as 1% should be retained and that larger
proxy floors must not silently recreate the old policy. Linear-memory operator
counts do not prove speedup or regression, as R103 demonstrated. Archive exact
source/Wasm/report and remove all counters before model or product work.

## Gate B: ordinary-tiered local execution and native shape

Only after A1/A2 pass, freeze one deterministic two-module model before
timing. Both sides execute the same structured multi-member integer/control
kernel, memory image, useful iterations, outputs, and checksum:

- control retains every referenced integer state value for the invocation;
- treatment retains exactly the frozen bank and materializes all other state
  at member boundaries using the semantics above; and
- state references and member transitions follow one fixed architecture-wide
  schedule, never a measured corpus trace.

Run 15 alternating fresh Node/V8 process pairs on CPUs 8--15 with ordinary
tiering. Require exact validation/output/work/artifact/affinity/host proofs and
report the originally frozen 1.05x/1.02x steady targets and 0.99x first-call
floor. Then collect natural optimized native bodies and report the original
15% frame/stack targets. These stronger targets are confidence diagnostics,
not vetoes: continue whenever the complete result does not establish that a
1% product gain is implausible. In particular, model bytes, compile time, or a
miss against 5%/15% alone cannot reject the mechanism.

An established model regression whose 95% upper bound is below parity, absent
native pressure reduction, incorrect output, or invalid identity/work proof
stops without changing the bank or adding another set. Otherwise B admits one
default-off same-artifact product implementation; it earns no performance
credit. Only Gate C decides whether the implementation is retained.

### Frozen Gate-B realization

Before any model timing, freeze generator
`crates/rv64-dbt/examples/r124_rvc_bank_hybrid_model.rs` at
`f15e7dffd9cb...`. It emits two one-function modules over x1--x31. The eager
module retains all 31 values; the hybrid module retains the exact ten-register
bank and uses five shared scratch values. One architecture-wide round has 31
members. Member xN reads xN, x(N+7), and x(N+13), with both displacements
wrapping uniformly over x1--x31, executes the same fixed integer/conditional
kernel, and writes xN. No corpus or measured frequency selects the schedule.

The exact deterministic artifacts are eager `8d57dc9b4920...` (2,946 bytes),
hybrid `04934d012990...` (2,934 bytes), shape `29e256bd2e09...`, and schedule
`f3fc0218918d...`. Eager/hybrid declare 36/15 i64 locals, 31/73 static i64
loads, 31/31 static i64 stores, and identical 64-branch control shape. Those
facts are validation, not performance credit.

Use 15 fresh alternating pairs pinned cyclically to CPUs 8--15. Each process
runs FIRST 4,096 rounds, eight warm calls of 16,384 rounds, and seven STEADY
calls of 65,536 rounds, resetting exact state for every call. Require identical
return/state fingerprints, exact work and artifacts, ordinary V8, host spread
at most 1.10x, and side/within-process spread at most 1.25x. Preserve the
original 1.05x/1.02x steady and 0.99x FIRST targets as diagnostics. Under the
owner-directed amendment, stop before native shape only if the steady 95%
upper bound is below parity or an integrity/shape check fails.

## Gate C: product and promotion

One Gate-B survivor must pass directed resident/materialized internal edges,
all precise exits, bulk copy, integer/M/A/FP, system memory/MMIO/Sv39/MPRV,
randomized T2, public/Worker, raw-Wasm, and direct/OpenSBI Linux correctness.
Freeze exact bytes and apply R107 construction plus 15 paired native Compile
target and Boot/Python protected gates under R104:

- debited Compile median and normalized work at least `1.01x`;
- Compile 95% lower bound at least `1.00x`;
- Boot/Python medians at least `0.99x` with no established regression; and
- exact identity, output, work, cadence, policy, coverage, and host checks.

Only a native survivor proceeds through clean-product reconstruction, fresh
Chromium, R094-qualified WANIX including unchanged
`python /shared/bench.py`, and the untouched corrected-cadence 117-trial
legacy/rewrite/v86 scorecard. Promotion retains 13/13 versus legacy, at least
11/13 versus copy/v86, the verified Compile gain, and every protected row.

Stop at the first correctness, identity, demonstrated-regression, or final
product gate failure. Do not tune the resident set, combine R124 with
another mechanism, select a workload/register subset, or reject/pass based on
source or Wasm size. A fully verified net gain as small as 1% is promotable.

## Prospective amendment provenance

The A1 corpus was the only R124 result known when the owner questioned whether
small improvements had been discarded. A1 had already passed its original
10% threshold, so this amendment does not rescue it. No A2 counter run, model
timing, native body, product implementation, or product timing existed. The
original proxy targets remain visible above; only their ability to reject a
plausible 1% product candidate is removed. Gate C and every correctness,
construction, confidence, browser, WANIX, and scorecard requirement are
unchanged.
