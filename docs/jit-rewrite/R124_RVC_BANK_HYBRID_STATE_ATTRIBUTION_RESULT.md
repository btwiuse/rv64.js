# R124 RVC-Bank Hybrid Structured-State Attribution Result

Date: 2026-08-10  
Status: A1/A2 opportunity established; frozen Gate-B model archived as
topologically invalid; exact product implementation admitted

## Result through Gate A

The immutable 56-region / 6,258-member corpus validated twice with identical
output.  Removing non-bank integer state locals reduced eager-byte-weighted
declared locals from `131.569795` to `114.079169`, a `13.293801%` reduction.

The sole measurement-ineligible modern Linux 6.12.7 / Alpine 3.24.1 dynamic
census completed with exact output, generated execution, settled compilation,
and public-cadence proofs.  Its projected hybrid/current GPR-operation ratios
were:

| Phase | Current operations | Projected operations | Ratio | Reduction |
|---|---:|---:|---:|---:|
| FIRST | 334,990,106 | 247,683,658 | 0.739376 | 26.0624% |
| PRIME | 362,585,006 | 267,573,522 | 0.737961 | 26.2039% |
| STEADY | 365,480,697 | 269,614,475 | 0.737698 | 26.2302% |

Those are operation counts, not performance credit.

## Frozen model observation

The one frozen ordinary-V8 model run was internally stable and correct for
the modules it executed.  Fifteen fresh alternating pairs measured a steady
paired speedup of `0.966035x` with 95% median interval
`[0.962105, 0.967411]`; FIRST measured `1.018706x`
`[1.008923, 1.024598]`.  Host spread was `1.020880x`, and all artifact,
affinity, work, schedule, output, and state checks passed.

After the result, inspection found that the frozen realization did not model
the product boundary topology required by Gate B.  It placed 65,536 complete
31-member rounds inside one generated-function invocation.  Consequently:

- eager state paid 31 loads plus 31 stores once per call; while
- hybrid state paid 10 resident loads plus 10 resident stores once, then 63
  materialized loads plus 21 materialized stores on every inner round.

At STEADY this is `20 + 84 * 65,536 = 5,505,044` hybrid state operations
against 62 eager operations, an approximately `88,791x` relative overcharge.
Production A2 instead projects hybrid at `0.737698x` current state operations.
The model therefore reverses the measured production relationship and cannot
establish a product regression.

The raw report remains immutable and still says
`stop-before-native-or-product-work`; that is the honest outcome of its
original checks.  This result document does not rewrite the report.

## Decision

Archive the frozen model as a failed model/harness experiment with no product
performance credit and no product veto.  Do not repair and rerun a synthetic
model after observing its result.  Gate A shows a large enough real-product
opportunity to admit the one already-frozen, architecture-general product
implementation under D121's proxy-gate guardrail.

The exact product is now the first authoritative runtime decision point.  It
must retain the fixed x1/x2/x8--x15 bank, preserve bulk copy and precise exits,
pass the full correctness matrix, debit construction, and meet the verified
net 1% rule plus all protected Boot/Python/browser/WANIX/scorecard checks.
Source or Wasm size has no pass/fail role.

## Evidence

- Static reports: `target/bench/r124-rvc-bank-hybrid/static-census-a.json` and
  `static-census-b.json`, both `8ed3a6a3dcf4...`.
- Dynamic report:
  `target/bench/r124-rvc-bank-hybrid/dynamic-census/dynamic-census.json`,
  `0f424205c3fb...`.
- Model report:
  `target/bench/r124-rvc-bank-hybrid/model-gate/gate.json`,
  `e32d62872f9f...`.
- Model modules: eager `8d57dc9b4920...`, hybrid `04934d012990...`.

## Exact-product native gate

The sole frozen exact-product run completed all 90 retained legs with no
integrity problem and host-probe spread `1.054948x`.  Fifteen real-construction
pairs produced a conservative `0.168840 ms` debit.  After that debit:

| Row | Paired median | 95% median interval | Normalized work |
|---|---:|---:|---:|
| Boot | `1.018471x` | `[1.000973, 1.035481]` | `1.018231x` |
| Compile | `1.083675x` | `[1.037357, 1.112250]` | `1.083602x` |
| Python | `1.200538x` | `[1.180720, 1.220097]` | `1.200500x` |

Every frozen integrity, target, confidence, normalization, and protected-row
check passes.  Candidate `d017a10f...` therefore advances unchanged to
Chromium, WANIX, and the untouched three-way scorecard.  This is the first
R124 product performance credit; A1, A2, and the synthetic model remain
diagnostic only.
