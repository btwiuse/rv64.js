# R124 Chromium Dual-Clock Gate Result

Date: 2026-08-10  
Status: passed; exact candidate advances unchanged

The sole frozen seven-pair run passed every identity, guest, policy, output,
generated-execution, retirement, cadence, affinity, and decision check.

| Clock | Control median | Candidate median | Paired speedup | Exact bootstrap 95% |
|---|---:|---:|---:|---:|
| execution-only | 2254.0 ms | 2235.7 ms | 1.018971x | [1.001431, 1.037301] |
| construction-to-ready | 2508.7 ms | 2484.3 ms | 1.017013x | [0.998068, 1.029593] |

Execution-only Boot is a confidence-verified improvement.  The inclusive
clock, which begins immediately before `RV64.create`, has a favorable median
and its interval does not establish regression.  Thus both prospectively
frozen protected-clock rules pass.  The report is
`target/bench/r124-rvc-bank-hybrid/chromium-gate.json`, SHA-256
`c3d5b975ea09219ffb62948b03154d8167ad505c4af045b102b694b56f2e6243`.

No sample was replaced or rerun, and no candidate code changed after the
native gate.  Exact candidate `d017a10f...` advances to the frozen WANIX gate.
