# R126 Exact-R124 Scorecard Invalid Result

Date: 2026-08-10  
Status: invalid; no scorecard or product credit

The sole R126 command launched all 117 scheduled fresh processes, but its
report is measurement-invalid with 135 problems. It contains 78 RV64 result
objects, zero eligible results, and zero copy/v86 result objects. Therefore no
displayed median, parity count, or `goalMet` value is performance evidence.

The immutable report is
`target/bench/r126-r124-scorecard/formal/scorecard-v2-2026-08-10T20-23-56-474Z.json`,
SHA-256 `ac096fecb8686eb76288af6409adeaf6b86808efc58310a52847197b90ee0d1f`.
Its generated Markdown is `bcebe6c0...`. No R126 trial will be reused or
pooled.

## Cause 1: the candidate override is diagnostic by design

R126 selected immutable candidate `d017a10f...` with
`SCORECARD_V2_REWRITE_WASM`. The worker deliberately marks any such override
`measurementEligible=false` and records `runtime.diagnostic.rewriteWasmOverride`.
This is correct behavior for focused A/B diagnostics, but it means an
authoritative product scorecard must execute the candidate as the ordinary
live source-built release, without an override. Both rewrite and legacy
workers inherited the environment variable, so all 78 RV64 result objects
were proof-only.

## Cause 2: an unverified required v86 input was absent

Every copy/v86 worker, including the generated-dispatch preflight, failed
before measurement because
`target/bench/matched-linux-x86-bzImage` was absent. The scorecard top-level
preflight checked both initramfs files and the v86 runtime but not this kernel,
so it continued through the full matrix instead of failing before the first
trial.

The exact R087 kernel still exists at
`/home/darren/src/arm64.js/target/bench/matched-linux-x86-bzImage`, SHA-256
`8854efec5534d0badf98aa34f7e7cb37fe3626d4d32d3a6909ca7fad8047acb5`.
That matches every accepted R087 v86 trial. It is not the unrelated
`buildroot-bzimage68.bin` (`507a759c...`).

## Replacement requirements

Preserve R126 unchanged. Before a from-zero full replacement:

1. overlay the exact archived R124 source through `apply_patch`, build the
   ordinary live release, and require exact Wasm `d017a10f...`;
2. run with no `SCORECARD_V2_REWRITE_WASM` or other diagnostic override;
3. restore the exact `8854efec...` matched i686 kernel to the scorecard
   artifact directory;
4. make the top-level preflight require the RV64 kernel, matched x86 kernel,
   v86 BIOS, and VGA BIOS before acquiring any performance result;
5. rerun all 117 trials from zero under a newly hashed protocol and tools.

This replacement corrects input admission only. It does not alter a workload,
timer, side, row, cadence, candidate, comparator, statistic, or threshold.
