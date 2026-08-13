# R120 Interleaved Fused-TLB One-Percent Result

Date: 2026-08-10
Status: rejected at the frozen verified-1% native gate; live product unchanged

## Outcome

R120 fairly remeasured the exact R100 interleaved fused-TLB candidate because
its old five-pair Compile point (`1.017x`) had been stopped partly by the former
`1.03x` economic floor. Under the current prospective rule, a construction-
debited, confidence-verified `1.01x` Compile gain would have advanced even if
it remained below 3% and regardless of code size.

The old favorable point did not reproduce. The new fixed 15-pair Compile
median is `0.992069x`, with paired-bootstrap 95% interval
`[0.952178, 1.015084]`; normalized fixed-work throughput agrees at
`0.992040x`. The exact candidate therefore fails all three target conditions:
its median is below `1.01x`, its lower confidence bound is below parity, and
its normalized median is below `1.01x`.

Python also misses its protected `0.99x` median at `0.982841x`, although its
wide interval `[0.954431, 1.033982]` does not establish regression. Boot is
safe but not material: `1.003286x [1.001075, 1.013737]`, with normalized work
`1.003219x`. A sub-1% improvement on a protected row cannot rescue the failed
Compile target.

## Authenticity and frozen method

- Control Wasm: `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`,
  4,279,380 bytes.
- Candidate Wasm:
  `c36da489ebe3e2f15d960a1ad393b808e9ff285dc099d4988c745e0e81065b32`,
  4,278,772 bytes.
- Archived source built twice in isolated target directories; both outputs are
  byte-identical to the immutable R100 candidate.
- Both artifacts validate in the host WebAssembly engine.
- Candidate size is 608 bytes smaller. This was recorded but supplied no
  performance credit and no acceptance criterion.
- The protocol, exact artifacts, 15-pair construction rule, 15-pair-per-row
  sample count, and evaluator were frozen before new performance timing.
- The old five R100 pairs were motivation only and were not pooled, extended,
  or reinterpreted.

The first isolated build attempt omitted the vendored dependencies from its
temporary source tree and stopped before compilation. It produced neither a
candidate artifact nor performance data. Once the immutable dependency input
was included, both actual builds reproduced the candidate exactly. Isolated
build elapsed times were excluded by protocol.

## Construction accounting

Fifteen alternating fresh-process `RV64Debug.create` pairs were valid:

- control median: `20.543605 ms`;
- candidate median: `20.427555 ms`;
- paired candidate-minus-control median: `-0.223734 ms`;
- paired 95% interval: `[-0.586400, 0.231087] ms`; and
- conservative R107 debit: `0.231087 ms` charged to every candidate sample.

The accounting intentionally gives no credit for the favorable median and
uses the positive upper confidence endpoint as the debit.

## Native result

All 90 fresh-process legs completed. The report is valid, host probe spread is
`1.065187x`, affinity is CPUs 8--15, every identity and modern guest check
matches Linux 6.12.7 / Alpine 3.24.1, Compile MD5 and Python checksum match,
fixed work is present, and generated Wasm executes on every leg.

| Row | Debited paired median | 95% interval | Normalized work | Gate role |
| --- | ---: | ---: | ---: | --- |
| Boot | `1.003286x` | `[1.001075, 1.013737]` | `1.003219x` | protected, pass |
| Compile | `0.992069x` | `[0.952178, 1.015084]` | `0.992040x` | target, fail |
| Python | `0.982841x` | `[0.954431, 1.033982]` | `0.982796x` | protected median fail; no established regression |

The frozen evaluator reports integrity pass; Compile median, lower-bound, and
normalized-work failures; and a Python protected-median failure. Its decision
is `reject-at-native-gate-and-restore-baseline`.

## Decision and historical implication

Reject the exact interleaved representation without a rerun or a SIMD/scalar,
alignment, width, selector, access-family, threshold, or workload variant.
Stop before source reapplication, browser, WANIX, and the 117-leg scorecard.
The current product was never changed during timing and remains byte-exact
`d9f686a9...`.

R120 confirms both halves of the new policy:

1. R100 did deserve an independent modern recheck because a plausible 1.7%
   gain should not have been discarded merely for missing 3%.
2. It did not deserve retroactive acceptance. A five-pair small point was not
   reproducible as a net product gain under a powered, construction-debited
   measurement.

The correct acceptance rule is therefore not “accept every observed point
above 1%.” It is “accept every correctness-safe net gain of at least 1% whose
predeclared paired evidence establishes parity or better and whose protected
rows remain intact.” Small verified gains accumulate; noisy small estimates do
not.

## Evidence

- Frozen inputs and commands:
  `target/bench/r120-interleaved-one-percent/FROZEN_INPUTS.txt`.
- Freeze manifest:
  `target/bench/r120-interleaved-one-percent/SHA256SUMS`.
- Construction report:
  `target/bench/r120-interleaved-one-percent/construction.json`, SHA-256
  `021205d4c807c405457d674da202f5f83e23ac98cb3948a9fe2359a83a341eb8`.
- Native report:
  `target/bench/r120-interleaved-one-percent/native/config-ab-2026-08-10T14-22-13-064Z.json`,
  SHA-256
  `9177a521d937e129370491e15c1a8fa1082451f6a06f4f1c2d69410a74087265`.
- Frozen gate:
  `target/bench/r120-interleaved-one-percent/native-gate.json`, SHA-256
  `67aed58c940d92c84504a78a6d727f4c9d7ac93e967caf4a25866e4be177ec1e`.
- Restored/live identities: CPU source `aec4b31434a6...`, Wasm source
  `1da35e70bc9c...`, loader `2cbb264f4dac...`, release Wasm `d9f686a9ce4f...`.
