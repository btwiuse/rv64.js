# R073 Sampled-Only Static-T0 Protocol

Date: 2026-08-09  
Status: Rejected at strengthened browser Gate C; default off

## Hypothesis and causal boundary

R072 combined two independently switchable mechanisms in its treatment:
R070's residual static decoder and the new exact page-policy sampled decoder.
R071 had already shown that residual-only activation independently produces
only a 1.024x Boot paired median and does not clear the cumulative gate. R072's
five native Boot treatments retire a median 113.601M static-fast instructions,
of which 107.013M (94.20%) are explicitly sampled. R072 Boot nevertheless
improves 1.209x `[1.190,1.239]`.

The combination is unnecessarily broad. In R072's normal browser Python legs,
sampled execution bridges only about 18-26 instructions per observation before
re-entering generated code. One confirmation leg with delayed generated
coverage instead sent about 19M additional *residual* instructions through
R070 and took 3.826 seconds. This does not prove R070 caused the delay, but it
shows that residual activation expands the treatment beyond the mechanism
that supplies nearly all Boot leverage.

R073 therefore enables only `STATIC_SYSTEM_T0_SAMPLED`. Ordinary residuals
continue through the accepted interpreter. This is an architecture-wide
mechanism boundary, not a PC, privilege, opcode, page, workload, browser,
benchmark, or phase selector. The main Wasm remains exact R072 SHA-256
`cb7ea81685b3cb9605f6e01b619f3c15005f4ef77ca00c258c20c7a519bb6df6`.
Both timing legs construct the same auxiliary module. The exact native
configurations are:

- control: `STATIC_SYSTEM_T0=0`, `SAMPLED_STATIC_T0=0`;
- candidate: `STATIC_SYSTEM_T0=0`, `SAMPLED_STATIC_T0=1`.

No R070/R071/R072 timing sample may be pooled into R073.

## Gate A: semantic combination and lifecycle

Before product timing, remove only the harness/API prohibition on sampled
activation without residual activation. Do not change the decoder, scheduler,
sample contents, q1024 quantum, thresholds, instruction families, slow
stretch, interrupt cadence, or policy constants.

Prove the sampled-only combination on the supported modern-Virt machine with
the exact q1/q32/q1024 ordered fingerprint differential (whose deterministic
program mixes control, scalar memory, A, and FENCE exits), generated-entry
handoff, partial-progress WFI, and modern Linux direct/OpenSBI boots. Retain
R072's unchanged main-artifact system memory/Sv39/A/FP/WFI and randomized
atomic/T2 matrix as the lower-level semantic coverage; sampled execution is
intentionally unsupported on the legacy machine used by several of those
directed tests.
Both legs must register exactly one identical auxiliary module; candidate
sampled retirement/samples/interrupt polls must be nonzero, residual-only
execution must remain disabled, and internal errors must remain zero. Because
the main Wasm bytes are unchanged from R072, the already-passed eight-stage
artifact suite remains applicable, but the new flag combination needs directed
coverage before timing.

Any semantic mismatch rejects R073. A bug fix must add a directed regression
without weakening observations or changing the frozen activation boundary.

Gate A passed without changing the main Wasm. Sampled-only q1/q32/q1024
fingerprints, complete state, generated-entry handoff, and partial-progress WFI
match their controls; direct and OpenSBI Linux reach the shell and execute
52.285M and 58.481M sampled instructions with zero errors. The public and
Worker APIs, scorecard selftests, syntax checks, and diff checks pass. The
exact Gate B artifact remains SHA-256
`cb7ea81685b3cb9605f6e01b619f3c15005f4ef77ca00c258c20c7a519bb6df6`.

## Gate B: native same-Wasm product timing

First run two alternating Boot gross pairs and stop on a correctness mismatch,
candidate error, or slowdown greater than 5%. Otherwise run five entirely
fresh alternating pairs for Boot, Compile, and Python on CPUs 8-15 with the
exact Linux 6.12.7 / Alpine 3.24.1 artifacts.

Advance only if Boot's paired median speedup is at least 1.10x with paired
bootstrap 95% lower bound at least 1.00x; Compile and Python paired medians are
each at least 0.97x; inputs/outputs and policy proofs match; host and within-
side native sample spreads stay within 1.25x; candidate sampled retirement is
nonzero; control sampled retirement is zero; and both sides record zero static
errors. The candidate-specific verifier is
`tests/vs-v86/r073-sampled-only-gate.mjs`.

There is no retry or residual enable, quantum, threshold, family, affinity, or
row change after a complete valid unfavorable result.

## Gate C: strengthened browser nonregression

R072 established that five single subsecond shared-9P observations give an
unstable point gate: its independent interval was `[0.829,1.268]`. R073 does
not reuse or reinterpret that outcome. Before any R073 browser sample, freeze
an immutable hash-named site and a candidate-specific runner using seven
alternating fresh Chrome processes/profiles/guests per side. Each leg executes
three phase-synchronized repetitions of every unchanged `/shared/bench.py`
phase. The per-leg phase value is the median of its three repetitions; the
primary statistic is the paired median of the seven candidate/control ratios.

Every repetition must preserve exact guest/checksum identity, production page
policy, generated-coverage proof, artifact/browser/CPU-affinity stability, one
prepared module in both sides, zero control sampled execution, nonzero
candidate sampled execution, residual static disabled in both sides, and zero
errors. No raw within-side spread cap is added. Python, SHA-256, and shared 9P
must each have paired median elapsed ratio at most 1.03 and exact paired-
bootstrap upper bound at most 1.10. No leg or repetition may be replaced.

A pass then runs the same seven-by-three candidate/v86 browser guard with the
established 1.10 upper-confidence noninferiority limit. This stronger method
is prospective and applies to both browser comparisons; it does not relabel
R072.

The frozen page is
`v86-rv64-three-way-r073-2b52e552d00929fa.html`, SHA-256
`1c70b211272fd9a843bfe52aefe804322d7260a144df7195a34363ad9f259aee`.
It binds RV64 archive
`rv64-jit-r073-2b52e552d00929fa.tgz`, SHA-256
`2b52e552d00929fa4c525c5b1aabc7abbce74d7d3ffe571a0e28d7d3b1cf199e`;
independent unpacking verifies inner main Wasm `cb7ea81685b3...` and loader
`5b1fd2f34976...`. The frozen entry points are
`tests/run-wanix-r073-pairs.mjs`,
`tests/analyze-wanix-r073-pairs.mjs`, and
`tests/vs-v86/r073-browser-gate.mjs`.

All seven preregistered pairs and all three repetitions per leg completed
without replacement. The measurement-valid report is
`target/jit-policy-traces/wanix-r073-cb7ea816-chrome-20260809-config-ab/analysis.json`,
SHA-256 `9bbaf2cd89cc78cea3b5ac07e8478ddc8a0a9b9d52eeb7ad55f55c6d9958671d`.
Candidate/control paired medians and exact bootstrap intervals are Python
`0.994 [0.979,0.997]`, SHA-256 `0.999 [0.984,1.003]`, and shared 9P
`1.058 [0.932,1.114]`. Exact outputs, artifacts, configurations, coverage,
sampled/residual lifecycle, and zero-error proofs all validate.

The analyzer initially required the boot-time Alpine banner to remain in the
terminal tail. Three repetitions push that banner outside the intentionally
retained last 35 lines even though the harness must observe exact release
`3.22.5` before it can start timing. The schema-3 validator was corrected to
bind the successful harness check to the exact frozen modern-Alpine root hash
`274a1e476646...`; phase correctness counts and the visible riscv64/Python
identity remain independently required. No sample, timing rule, limit, or
artifact changed and the data was not rerun.

Shared 9P fails both prospective limits: its 1.058 median exceeds 1.03 and its
1.114 upper bound exceeds 1.10. R073 is therefore rejected and remains
default-off. Candidate/v86, default promotion, and Gate D were not run.

## Gate D: default and authoritative promotion

Only after Gates A-C pass may sampled-only become the default. Rebuild and
archive the default-on artifact, rerun the complete strict correctness suite,
then execute the untouched 13-row three-way scorecard with three alternating
fresh processes per rewrite/legacy/v86 side.

Promotion requires a valid report, 13/13 versus legacy, no new v86 loss, at
least the accepted 11/13 v86 wins, no row more than 5% slower than accepted
R054, and Boot retaining at least 1.10x improvement relative to the same-Wasm
control. Promotion advances but does not complete the thread goal unless Boot
and Compile both reach copy/v86 parity and `/shared/bench.py` remains
nonregressed.
