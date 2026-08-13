# R107 Amortized Main-Runtime Cold-Cost Policy

Date: 2026-08-10  
Status: accepted prospectively; calibrated; no candidate or parity credit

## Problem

R104 correctly lowered the mature rewrite's economic floor to a verified 1%,
but R106 exposed a mismatched preliminary gate. Its candidate was rejected for
a 5.116% increase in an approximately 12 ms construction measurement: an
absolute paired-median increase of only 0.631 ms. The fixed limit was exceeded
by 0.116 percentage points, before any Boot, Compile, or Python causal timing.

That result remains rejected under its frozen protocol. It does show why a
percentage of a tiny setup event is not a sound proxy for net product cost.
One percent of the current authoritative rewrite Compile and Boot rows is
approximately 9.545 ms and 21.765 ms respectively. Future candidates therefore
account for excluded construction in milliseconds against the end-to-end row
they are trying to improve.

## Timing boundary and non-duplication

The corrected scorecard loads and creates the main RV64 runtime before its
execution-only timer. Compilation, instantiation, and publication of generated
JIT modules happen while guest execution is being pumped and are already
inside Boot or workload time. Only candidate-dependent work before the first
guest pump is outside the scored clock.

The cold measurement must use the real product construction path,
`await RV64Debug.create(wasmBytes)`, in a fresh process. Process startup, file
I/O, hashing, and loader import occur before the timer. The timer includes
main-module compilation, instantiation, runtime-created helper modules, and
all synchronous construction performed by that API. If a candidate adds a
default product auxiliary module or other required pre-pump initialization,
its experiment-specific wrapper must include that work too.

A synthetic `WebAssembly.compile` or dummy-import instance is useful for
attribution but cannot substitute for this product measurement. Code, CODE
section, import, export, and memory sizes remain recorded diagnostics and ABI
proof inputs; none is a standalone performance rejection threshold.

## Frozen accounting rule

For the current host, collect 15 alternating fresh-process control/candidate
pairs on CPUs 8--15. A future environment may choose a different fixed count
only from control/control calibration completed before candidate timing. Pair
order, process isolation, construction recipe, artifact identities, Node/V8,
affinity, and output path are frozen before the first candidate sample. No leg
is replaced and there is no post-result extension.

For pair `i`, let:

```text
d_i = candidate_create_ms_i - control_create_ms_i
```

Compute the deterministic paired-bootstrap 95% interval of the median `d_i`.
The conservative one-time construction debit is:

```text
D = max(0, upper_95(median(d_i)))
```

For every native target and protected row, replace its ordinary paired speedup
with:

```text
adjusted_speedup_i = control_elapsed_ms_i /
                     (candidate_elapsed_ms_i + D)
```

Fixed-work normalized throughput uses the same adjusted candidate elapsed
time. The R104 rules then apply to these adjusted samples: target paired median
at least `1.01x`, target lower 95% bound at least `1.00x`, normalized work in
agreement, protected median at least `0.99x`, and no protected interval that
establishes regression.

The full debit is charged independently to every row. This deliberately models
a fresh emulator created solely for that workload and is more conservative
than spreading construction across the whole multi-row session. A statistically
supported construction improvement sets `D` to zero and is reported as cold
credit, but that credit cannot rescue runtime or claim execution-parity gain.
Candidate failures to construct, lifecycle/ABI changes, or persistent
background work remain correctness/product failures rather than amortizable
latency.

## Browser and product confirmation

Node establishes early native causality, not portable browser performance.
Every candidate that survives it must retain both clocks in a fresh Chromium
Worker:

- execution-only, from the first guest pump to the exact ready/work marker;
- construction-to-marker, from immediately before product VM construction to
  the same marker, with assets already fetched.

The execution-only clock attributes runtime gain. The construction-to-marker
clock is the browser-engine/product confirmation and must satisfy the target or
protected R104 rule declared for that candidate. The qualified fixed-work
WANIX guard and untouched 117-trial three-way scorecard remain mandatory. A
different Wasm JIT's behavior is therefore observed directly rather than
inferred from Node or from byte size.

## Calibration result

The reusable implementation is:

- `tests/vs-v86/amortized-cold-cost.mjs`;
- `tests/vs-v86/amortized-cold-cost-selftest.mjs`;
- `tests/vs-v86/main-runtime-construction.mjs`.

The exact-baseline/exact-baseline calibration is
`target/bench/r107-amortized-cold-policy/control-control-construction.json`,
SHA-256 `6fb8bc9f2f2c9efdb734793675315f2c8c878b9e54945c2101d57a2b03d2627e`.
It uses Node 26.5.0 / V8 14.6.202.34-node.24, CPUs 8--15, loader
`2cbb264f4dac...`, and baseline Wasm `d9f686a9ce4f...` (4,279,380 bytes).
Fifteen valid real-construction pairs measured control/candidate medians
20.629/20.518 ms. The same-artifact paired median delta is -0.323 ms with
interval `[-2.273,0.155]`, producing a conservative false debit of only
0.155 ms. That is about 0.016% of Compile and 0.007% of Boot, sufficient to
resolve costs relevant to the verified 1% floor on this host.

As a historical diagnostic only, applying the formula to R106's already
archived synthetic samples would produce a 1.386 ms debit. This does not alter
R106's decision, authorize a rerun, or provide missing runtime evidence.
