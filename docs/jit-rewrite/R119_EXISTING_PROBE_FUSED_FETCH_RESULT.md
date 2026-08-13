# R119 Existing-Probe Fused Execute-TLB Result

Date: 2026-08-10
Decision: reject at native gate and restore the exact baseline

## What was tested

R119 reused the scalar interpreter's existing execute-TLB tag proof but
changed its Fetch payload from `pa - va` to a stable direct-RAM pointer offset.
An ordinary hit therefore performed a halfword load without repeating the
physical-bus range, offset, and bounds path. It added no second probe, cache,
CPU field, runtime switch, guest-PC selector, or benchmark-specific rule.

This was selected from an exact current-control Boot profile. The physical-bus
fetch band owned 1.8554% of all sampled cycles, enough plausible exposure for
one candidate under the verified-one-percent rule. Its elapsed profiler timing
was excluded.

## Correctness and shape

Candidate CPU source was
`7121814faf1379b0904e95e43cad1c576f2eadb00a5abe9bc7355748e04d0eba`.
Two independent release builds were byte-identical at
`41b94faa9020b3c900919505a754c8ffe362d79fe5b68e68dce796e17f85def5`,
4,282,567 bytes. That is 3,187 bytes above the 4,279,380-byte control; bytes
were recorded but were not a pass/fail criterion.

All candidate unit, API, raw-Wasm, differential, ISA, Spike, architecture-
signature, direct-Linux, and OpenSBI-Linux gates passed. The strict Nix run
ended with `ALL STAGES PASSED`. Native inspection confirmed the predeclared
one-tag-comparison direct-load hit and absence of physical-bus range/bounds
work on that hit.

## Frozen measurements

Construction report:
`target/bench/r119-existing-probe-opportunity/construction.json`
(`52d9756f8e5ff499b20c2ac1e56182fd07641bbb7e9ec3264da35a743ef77203`).
It is valid with no problems. Fifteen pairs measure control/candidate medians
20.594398/20.841796 ms. R107 charges the positive upper 95% paired construction
delta, 1.258935 ms, to every candidate runtime sample.

Native report:
`target/bench/r119-existing-probe-opportunity/native/config-ab-2026-08-10T13-44-31-058Z.json`
(`bb9a925cb1e270396a694d3757e2dd3783a7593b6aab6009b5151ff18a566328`).
It is a complete valid 90-leg report with no problems, exact artifact/guest/
output/work/cadence guards, CPU affinity 8--15, and host spread 1.067733x.

| Row | Debited paired median | 95% interval | Normalized work | Frozen check |
| --- | ---: | ---: | ---: | --- |
| Boot target | `1.012411x` | `[0.997859,1.015302]` | `1.012351x` | fails lower bound |
| Compile protected | `0.984634x` | `[0.947863,1.016480]` | `0.984642x` | fails 0.99 median |
| Python protected | `1.001649x` | `[0.981848,1.029909]` | `1.001612x` | passes |

The frozen evaluator report is
`target/bench/r119-existing-probe-opportunity/native-gate.json`
(`cd6bb229ec866697140d8a72f4cb0bf0651828ad39deb05934490e40c85185c8`).
Every integrity check passes. `bootMedian`, `bootNormalizedMips`, and both
Python checks pass; `bootLower95` and `compileMedian` fail. Compile's interval
still includes parity, so the report does not establish that Compile is always
slower; its predeclared protected median nevertheless cannot be waived after
observation.

## Decision and interpretation

Reject without a rerun or extension and stop before Chromium, WANIX, and the
117-leg scorecard. The result is a useful near miss: it shows the mechanism can
move Boot by roughly 1%, but the current implementation does not prove a net
product win while protecting Compile.

The decision does not resurrect the old 10%/20% rule. A verified 1.01x result
would be accepted even with the 3,187-byte growth. R119 fails because its Boot
confidence interval crosses parity and its Compile median falls below 0.99,
not because its point gain or code size is considered too small.

Exact candidate artifacts, pre-timing source/harness archives, diagnostics,
all 90 legs, and manifests are preserved under
`target/bench/r119-existing-probe-opportunity/` and
`target/bench/r119-existing-probe-shape/`. The final opportunity directory is
authenticated by its 2,545-entry manifest
`36b8265db16758f4fae9836c3b0698ca979aaf6f1fe42db8cdeb4f9cc0cb017a`.
Live CPU source and release Wasm are restored byte-exact to
`aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`
and
`d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`.
