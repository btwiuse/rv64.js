# R072 Independent Browser Confirmation Protocol

Date: 2026-08-09  
Status: complete; candidate rejected at independent confirmation

## Why the first browser sample is invalid

R072 passed its complete correctness gate and its five-pair native gate. Its
first five-pair browser candidate/control sample is permanently retained at
`target/jit-policy-traces/wanix-r072-cb7ea816-chrome-20260809-config-ab`.
The analyzer report has SHA-256
`91da7722e3289f586c89c5fd01c623c0e82c95c1d15b9a13b989e86ef5b08776`
and remains invalid. Although every preregistered paired-median timing rule
passed, the new analyzer also imposed a 1.25x within-side spread cap on every
browser phase. Shared 9P measured 1.385x in the control and 1.433x in the
candidate, so no performance decision may use that sample.

That cap was a harness specification error, not an unfavorable candidate
result. The established browser analyzer has never imposed a within-side
spread limit. More importantly, evidence collected before R072 proves such a
limit is incompatible with the accepted workload: accepted R054's five-pair
shared-9P RV64 samples have 1.612x spread, and accepted R043's have 1.284x
spread. Both were valid under the established fresh-process, immutable
artifact, exact-output, chronology, CPU-affinity, runtime-proof, paired
bootstrap method. Applying 1.25x would retroactively invalidate the accepted
baseline itself.

The invalid sample is neither repaired nor pooled. No leg is deleted or
replaced, and its favorable timing values do not count toward confirmation.

## Frozen independent confirmation

Run five entirely fresh alternating browser candidate/control pairs with the
same Chrome 150/V8 15.0 engine, CPUs 8-15, immutable page SHA-256
`28957e0d5ce381184addb291805ba26a6e64d421a51882c4ae56e0512a82cd3d`,
archive SHA-256
`159fc55c4337345a685252e384d64be39fc50c743b4478e2b864289ad8bb8690`,
and inner main Wasm SHA-256
`cb7ea81685b3cb9605f6e01b619f3c15005f4ef77ca00c258c20c7a519bb6df6`.
Both legs prepare the same module after shell readiness. The control sets
`staticSystemT0=false,sampledStaticT0=false`; the candidate sets both true.

The schema-2 runner mode records the invalid predecessor identity before its
first leg. The analyzer requires five chronological process/profile/guest
pairs, fixed alternating order, unchanged artifact snapshots/browser/CPU
affinity, exact Alpine/Python/checksum identity, production page-policy
settings, generated-code coverage, a registered auxiliary module in both
legs, zero static errors, zero control static retirement, and nonzero candidate
fast/sampled retirement, samples, and interrupt polls in every phase.

The established candidate/control decision statistic from R071 remains
unchanged: each of Python, SHA-256, and shared 9P must have a paired-median
candidate/control elapsed-time ratio no greater than 1.03. Raw within-side
spreads and exact paired-bootstrap intervals remain reported, but do not add a
new rejection rule that the established browser workload cannot satisfy.

There is no retry after a complete valid unfavorable result. This sample may
not be pooled with the invalid predecessor. A pass advances to the separately
frozen five-pair candidate/v86 browser guard with its established 1.10 upper
confidence limit; a failure rejects R072 without tuning.

## Frozen result

The five fresh pairs completed without replacement under Chrome
150.0.7871.186 / V8 15.0.245.21. The report is
`target/jit-policy-traces/wanix-r072-cb7ea816-chrome-20260809-config-ab-confirmation/analysis.json`
with SHA-256
`50e36b53eb1037fb76a81648ba6c20e7f2a491262a4066ab3472f9e25b58a4cb`.
Every artifact, chronology, guest identity/output, policy, coverage, module,
and static-execution proof passes. The paired-median candidate/control elapsed
ratios are Python 1.029x, SHA-256 0.990x, and shared 9P 1.041x. Shared 9P's
raw paired ratios are 0.894, 0.829, 1.060, 1.268, and 1.041, giving exact
median interval `[0.829,1.268]`.

Reject R072 because the valid shared-9P point median exceeds 1.03. The wide
interval is evidence that a future, project-wide browser I/O regression gate
needs a longer workload or evidence-based uncertainty rule; it does not alter
this prospectively frozen decision. Do not pool the invalid predecessor,
replace an observation, run candidate/v86, or promote the default.
