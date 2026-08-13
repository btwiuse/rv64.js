# R103: carried-GPR cross-module boundary opportunity

Date: 2026-08-10  
Status: opportunity rejected; diagnostic removed and exact baseline restored

## Question

Compile STEADY executes about 8.5 million successful generated cross-module
tail transfers. Every current transfer first commits the source module's dirty
architectural locals to shared linear memory; the target module then reloads
its required register union plus PC, retirement, and fuel. R096 showed that the
adjacent diagnostic hop increment is not material, and R097 showed that
TurboFan removes duplicate dispatch metadata loads. Neither experiment tested
the architectural state materialization itself.

R103 asks whether a fixed, architecture-defined carried integer-state ABI has
an independently measured >=3% whole-Compile opportunity. This is diagnostic
only. It does not alter production state, generated semantics, scheduling, or
the official scorecard.

## Frozen baseline and scope

- clean executable-equivalent R085 Wasm:
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`
  (4,279,380 bytes);
- loader `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- R087 public one-slice cadence and production page policy;
- modern Linux 6.12.7 / Alpine 3.24.1 only, with no TinyEMU kernel or BBL;
- CPUs 8--15 and the existing host identity/probe rules; and
- R087 Compile median 954.483 ms as the fixed whole-row projection denominator.

The proposed state family is all architectural integer registers x1--x31 plus
PC, cumulative retirement, and fuel. It is not a register-popularity subset,
guest-PC selector, workload-derived ABI, or host-engine switch. FP state is not
included in this opportunity: an eventual product may retain current FP
materialization across integer-only transfers and must fall back safely at any
incompatible boundary.

## Exact dynamic census

Under the existing opt-in scorecard profile mode, add proof-only counters to
each register-resident generated invocation for:

1. generated function invocations;
2. emitted GPR entry loads and GPR exit stores;
3. emitted FP entry loads and FP exit stores; and
4. emitted `fcsr` entry loads and exit stores.

The counter increments are absent when profiling is disabled. Their wall time
is invalid by construction. Run one fresh exact-baseline Compile profile and
require:

- exact artifact, loader, guest, policy, cadence, input, output, and generated
  execution identity;
- `boundary invocations == region calls + successful chain hops` in every
  phase, proving the census covers the real transfer topology;
- nonzero GPR loads/stores and successful chain hops; and
- a rigorous chain-attributable GPR-operation lower bound computed as
  `total GPR operations - 62 * outer region calls`. An outer call can account
  for at most 31 GPR loads plus 31 GPR stores, so the remainder cannot be
  blamed on required runtime entry/exit materialization.

Archive the diagnostic Wasm, source, report, and exact derived table, then
remove the counters and restore exact baseline before any product work.

## Census result

The diagnostic artifact is
`2294651b34529b44271a180455b0f8a77ad51bb9a20ecaf90e445c5edf1139e7`
(4,280,233 bytes). Its proof-only report is
`target/bench/r103-carried-gpr-opportunity/profile/scorecard-v2-2026-08-10T05-40-21-967Z.json`,
SHA-256 `bc147a6194695016a421d089ac6b0d4760ebaa28ce3af6d2195ebcca97ecca19`.
The only scorecard problem is the required declaration that a proof profile
entered Compile timing; every identity, policy, workload, output, and generated
execution contract passed.

| Phase | Outer calls | Chain hops | Boundary invocations | Chain share | GPR ops/invocation | Rigorous chain GPR ops/hop |
|---|---:|---:|---:|---:|---:|---:|
| FIRST | 786,853 | 7,554,037 | 8,340,890 | 90.566% | 40.337 | 38.081 |
| PRIME | 559,386 | 8,529,000 | 9,088,386 | 93.845% | 40.115 | 38.679 |
| STEADY | 533,462 | 8,558,835 | 9,092,297 | 94.133% | 40.065 | 38.698 |

Every phase satisfies `invocations == outer calls + chain hops` exactly. In
STEADY, 185,783,621 GPR loads plus 178,499,259 GPR stores execute at generated
boundaries. Even assigning the maximum possible 62 GPR operations to every
outer call leaves 331,208,236 operations attributable to cross-module chains.
FP loads/stores are only 405,253 each and `fcsr` is zero, supporting the frozen
integer-only scope without using a workload-selected register subset.

The derived census is `target/bench/r103-carried-gpr-opportunity/census.json`,
SHA-256 `2c7256dec4e92ec0a5ea102e747bc79ee0f7fe0c554fc5b60fce587c5e795c9b`;
its independent integrity validator passes. For the local proof, the exact
conservative state-operation scale is `(38.697817635 + 5) / 67 =
0.6522062334`: 38.698 rigorously chain-attributable GPR operations plus five
fixed PC/retirement/fuel operations, relative to the frozen 34-load/33-store
model. No elapsed time from the diagnostic report is used.

## Ordinary-tiered-engine boundary proof

The earlier one-process `abi-microbench.mjs` run is exploratory and cannot
admit a product. Build a new checksum-bearing two-instance Wasm model that
matches the actual topology:

- one table-owning trampoline uses `return_call_indirect`;
- two independently instantiated generated modules alternate through it;
- the control reloads and stores x1--x31 plus PC/retirement/fuel through shared
  linear memory on every hop;
- the treatment loads once at the public wrapper, carries the same fixed state
  as typed parameters through every tail transfer, and commits once at the
  final return;
- both execute identical integer transforms, hop count, final memory image,
  and checksum; and
- ordinary tiered V8 is used, with fixed warmup sufficient to prove Liftoff and
  TurboFan compilation for both variants. No forced tier, engine detection,
  or scorecard timing is allowed.

Run seven alternating fresh-process pairs. Require identical output, complete
pairs, sample spread <=1.10 per side, paired median per-hop improvement, a
paired-bootstrap 95% lower bound above 1.00, and a conservative whole-Compile
projection of at least 1.03 using the lower-bound per-hop saving, the exact
STEADY chain count, and the fixed 954.483 ms denominator. Scale the projected
saving by at most the exact census's chain-attributable GPR-operation fraction;
never assume all 62 GPR memory operations are present when the census says
otherwise.

## Decision rule

Only if both diagnostics pass may R104 freeze a carried-GPR product design.
R104 would still need a conventional `(state) -> ()` public wrapper, complete
precise-exit commits, a separate carried-entry table/signature, safe fallback
for FP or unsupported-tail-call paths, exhaustive integer/FP/A/memory/Sv39/
dirty-code/lifecycle differentials, modern direct/OpenSBI boots, cold
construction, and the existing Boot/Compile/Python, Chrome, qualified long
WANIX `/shared/bench.py`, and untouched three-way scorecard gates.

Failure closes carried GPR state as a standalone boundary mechanism. Do not
respond by choosing popular registers, benchmark PCs, larger fuel grants,
forcing TurboFan, or omitting architectural state. Module or CODE size is
recorded only; direct cold construction remains the size-related gate.

## Boundary-proof result and decision

The frozen two-instance model validated and matched one exact final-state
fingerprint (`0x598b80a8561dbe8d`) for both variants. Trace collection proves
three Liftoff and three TurboFan events for the materialized topology, and four
Liftoff plus three TurboFan events for the carried topology. Thus a missing
optimizing tier cannot explain the result.

Seven alternating fresh-process pairs measured 19.3977 ns/hop for the current
memory boundary and 19.4303 ns/hop for the carried ABI at the median. Paired
speedup is 0.9989x with bootstrap interval `[0.9861,1.0533]`; paired median
saving is -0.0208 ns/hop. The conservative whole-Compile projection is exactly
1.0000x because the lower-bound saving is non-positive. One unusually fast
memory leg also makes control-side spread 1.1876x, above the frozen 1.10 limit,
while every within-process spread passes.

The local run is therefore not admissible performance evidence: it fails the
side-stability, positive-median, positive-lower-bound, and >=1.03 projection
requirements independently. A failed opportunity gate cannot be repaired by
adding pairs after observing it. Report
`target/bench/r103-carried-gpr-opportunity/boundary-proof.json` has SHA-256
`1102fa5b05e84f098598ecd771b279510e0607be372ce80fb62772c454fc5e99`.

Reject R103 before any product ABI, wrapper, table, or runtime implementation.
The result is consistent with TurboFan trading linear-memory traffic for the
cost of carrying 34 typed values through the cross-instance tail-call ABI; the
large dynamic operation count alone is not a speedup. Do not choose a popular
register subset, retry another lane count, force a tier, or increase chain
length. The diagnostic counters are removed, restored DBT units pass 53/53,
and the active release artifact is byte-exact
`d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`.
