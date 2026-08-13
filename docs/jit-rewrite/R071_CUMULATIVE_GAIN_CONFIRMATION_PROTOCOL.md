# R071 Cumulative-Gain Confirmation Protocol

Date: 2026-08-09  
Status: rejected at independent native Gate A

## Why this is a new experiment

R070 passed its semantic and local-opportunity gates, then produced a valid
three-pair Boot improvement of 1.047x `[1.001,1.061]`. It nevertheless failed
its prospectively frozen 1.10x Phase B advancement gate. That decision remains
true and the R070 report remains immutable.

After seeing that result, the project owner changed the *standing retention
economics*: general, reproducible gains around 5% are useful when accumulated,
even though the 10% rule is still appropriate as an inexpensive screen for a
large speculative mechanism. Applying a looser rule retroactively to R070's
existing samples would be outcome-dependent. R071 therefore freezes the new
project-wide cumulative-gain track and requires entirely fresh promotion data.
The old three pairs are motivation only.

## Frozen candidate and control

Both legs use the same main rewrite Wasm SHA-256
`8314d06d89bf1957548f4c6297d12aed1e4e9563fd29a31390e4393b12e97e62`.
Both construct exactly one identical guest-independent static-T0 auxiliary
module before the measured guest phase. The control leaves its invocation
disabled; the candidate enables it. No source, instruction family, 64-insn
slow-stretch rule, page policy, JIT threshold, benchmark, or guest artifact may
change between legs or after measurement starts.

This candidate qualifies for the cumulative track because it contains no
guest PC, symbol, binary hash, workload, browser, or scorecard selector; covers
the complete ordinary RV64I/M/integer-RVC/scalar-memory surface; constructs one
7,741-byte module in about 0.10 ms; and has already passed exhaustive RVC,
randomized scalar, full-system translation/memory/A/FP/WFI/T2, and modern
direct/OpenSBI Linux correctness gates. It addresses Boot, one of the two
remaining v86 losses. The separately discovered WFI-yield fix is not toggled
and remains in both legs.

## Gate A: independent native confirmation

Run five fresh alternating process-isolated pairs for Boot, Compile, and
Python on CPUs 8-15. Every process boots the exact Linux 6.12.7 / Alpine
3.24.1 riscv64 artifact set. Continue to construct the auxiliary module in
both legs so the switch isolates execution rather than lifecycle cost.

Advance only if:

- Boot's paired median speedup is at least 1.03x and its paired-bootstrap 95%
  lower bound is at least 1.00x;
- Compile and Python paired medians are each at least 0.97x;
- exact input, output, retirement, and generated-execution fingerprints match;
- the candidate records fast static-T0 retirement and zero static-T0 errors;
  and
- host-probe and within-side sample spread remain within the unchanged 1.25x
  limits.

The three R070 pairs may not be pooled with these five. There is no retry for
an unfavorable valid sample and no threshold, family, slow-stretch, affinity,
or row change after seeing it.

## Gate B: browser no-regression

If Gate A passes, run five alternating fresh-browser candidate/control pairs
using the same archived site and browser version. Compare every phase emitted
by `python /shared/bench.py`. No phase may regress by more than 3% by paired
median; output fingerprints must match; no run may be replaced; and browser
thermal/host validity checks remain unchanged. Then rerun the existing
rewrite-versus-v86 browser guard and require its established 1.10 maximum
slowdown on every phase.

## Gate C: product promotion

Only after Gates A and B pass may the candidate become default-on and enter an
untouched authoritative three-way scorecard. Rebuild, archive, and hash the
default-on artifact; rerun the complete correctness suite; then run all 13
rows with three alternating fresh processes each for rewrite, legacy, and
copy/v86. The modern guest, production page policy, benchmark inputs, v86
revision, and harness must remain unchanged.

Promotion requires a valid report, 13/13 versus legacy, no new loss versus
v86, no reduction from the accepted 11/13 v86 score, and no row more than 5%
slower than the accepted R054 control. Boot must retain at least the 3% direct
A/B gain proven in Gate A. Promotion does not complete the thread objective:
Boot and Compile must ultimately reach copy/v86 parity.

If any gate fails, retain the R070 emitter and semantic oracle as research
assets, retain the independent WFI-yield correctness fix, and remove the
static-system scheduler/lifecycle candidate. Record the negative result; do
not tune it into a passing benchmark.

## Frozen result

The independent report is
`target/bench/r071-static-t0-independent-confirmation/config-ab-2026-08-09T05-43-19-283Z.json`.
It contains all five fresh alternating pairs for all three rows, uses the
frozen same-main-Wasm control/candidate configurations, is measurement-valid,
has a 1.062x host-probe spread, stays within every sample-spread limit, matches
all guest inputs and output fingerprints, records nonzero candidate fast
retirement, and records zero static-T0 errors.

The paired results are:

- Boot 1.024x `[1.000,1.050]` (raw-side medians 2665.50/2575.62 ms, or
  1.035x);
- Compile 0.998x `[0.962,1.020]` (1111.07/1099.88 ms); and
- Python 1.032x `[0.983,1.043]` (3098.85/3011.04 ms).

The preregistered verifier `tests/vs-v86/r071-static-t0-gate.mjs` returns
`R071_GATE_A_FAIL` because Boot's paired median is below 1.03x. The favorable
raw-side-median ratio does not replace the prospectively selected paired
statistic, and the three older R070 pairs are not pooled into this sample.
Gates B and C are therefore not run. This does not discredit the standing
cumulative-gain policy; it says this exact residual-only activation did not
independently establish the minimum retained product effect.
