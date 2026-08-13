# R125 WANIX Product Gate Result

Date: 2026-08-10  
Status: valid old-gate failure; accepted for scorecard escalation by explicit
owner policy supersession

## Execution integrity

The sole frozen R125 run completed all fourteen fresh-browser legs in the
preregistered seven-pair alternating order. Each leg created a fresh Chrome
process/profile, WANIX Worker, RV64 emulator, and Linux guest pinned to host
CPUs 8--15. Every leg reached Linux 6.12.7 / Alpine 3.22.5, proved active JIT
execution, and completed three synchronized repetitions each of unchanged
Python, SHA-256, and the exact 32 MiB shared-9P workload. All fourteen stderr
files are empty. No leg was replaced or repeated.

The common corrected adapter is `bba6baaf...`; control core is
`d9f686a9...`; candidate core is unchanged `d017a10f...`. The immutable
protocol is `54e8087e...`, and the analyzer report is `9e051d1f...`. Every
archive, page, browser, guest, output, generated-coverage, 9P byte/transfer,
within-browser-spread, affinity, and freshness proof passes. The measurement
is valid.

## Frozen result

| Row | Paired-median speedup | Exact paired-bootstrap 95% interval | Verdict |
| --- | ---: | ---: | --- |
| shell | `0.996257x` | `[0.989513, 0.998082]` | fail: interval establishes regression |
| Python | `1.079692x` | `[1.070039, 1.111911]` | pass |
| SHA-256 | `1.021724x` | `[1.018486, 1.037770]` | pass |
| shared 9P | `1.009359x` | `[1.003915, 1.016363]` | pass |

Shell is the only failing rule. Its candidate median is 31,316.958 ms versus
31,183.869 ms control, a 0.374% median slowdown. That remains above the
separate frozen `0.99x` median floor, but the frozen rule also required the
confidence upper endpoint to reach parity. Its upper endpoint is only
`0.998082x`, so the analyzer correctly emits
`R125_WANIX_BROWSER_GATE_FAIL`.

The unweighted geometric mean of the four row point estimates is
`1.026272x`, but no aggregate weighting was preregistered. It is useful only
to expose the policy tension; it cannot override the frozen row-wise verdict.

## Consequence

R124 does not advance to the untouched 117-trial scorecard under D125. Exact
candidate `d017a10f...` remains archived and receives no product credit; exact
live release product remains control `d9f686a9...`.

This valid boundary result is also evidence for a prospective policy
clarification. A rule intended to tolerate protected-row changes smaller than
1% should reject when a protected paired median falls below `0.99x`, rather
than reject every confidence-established slowdown of any magnitude. A true
weighted "net" policy would instead require workload weights frozen before
sampling. Neither change may be applied retroactively to this result.

## Owner adjudication after the sealed result

The owner clarified that the project intends a one-percent material-regression
boundary, not a ban on every detectable slowdown. The immutable analyzer
report remains `gatePassed=false`; no sample, interval, threshold, or raw file
was changed and WANIX was not rerun.

Under the clarified product rule, shell `0.996257x` passes the `0.99x`
protected floor, while Python, SHA-256, and shared 9P are positive and every
correctness/integration proof passes. R125 is therefore accepted as WANIX
qualification for escalation. This is not final product promotion: exact
candidate `d017a10f...` must next pass the untouched 117-trial modern
three-way scorecard.
