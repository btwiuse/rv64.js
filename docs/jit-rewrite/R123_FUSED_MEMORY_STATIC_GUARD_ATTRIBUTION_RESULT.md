# R123 Fused-Memory Static-Guard Attribution Result

Date: 2026-08-10  
Status: closed below verified-1% exposure; no product change

## Outcome

Do not replace the production runtime flag with a compile-time Bus capability.
The flag guard is genuine and production always enables it, but the complete
authenticated guard blocks own only `0.104231%` of uninstrumented Boot
main-thread period. Even impossible zero-cost elimination would produce only
`1.001043x` whole-Boot speedup, far below the frozen `1.25%` exposure screen
and `1.01x` verified product target.

This is not rejection of a measured small gain: no source candidate was built
or timed. It is a deterministic attribution stop. Code size was never a gate.

## Immutable native result

Two independent analyzer runs are byte-identical at SHA-256
`d08985d08b4a...`. They consume exact preserved R119 perf data
`24028310d77f...` and authenticate the five optimized scalar memory bodies.
All `14,564,953,263` period units close, of which `10,575,960,013` belong to
the main thread.

| Population | Period | Main-thread share |
|---|---:|---:|
| all five optimized `ld`/`st` bodies | 750,185,288 | 7.093307% |
| complete flag-guard blocks | 11,023,476 | 0.104231% |

The guard blocks are only 1.469434% of the sampled optimized memory bodies.
Their per-body period is:

| Body | Whole body | Guard block |
|---|---:|---:|
| `ld1` | 54,800,902 | 2,187,983 |
| `ld4` | 95,961,942 | 0 |
| `ld8` | 309,148,821 | 6,679,631 |
| `st4` | 28,748,709 | 0 |
| `st8` | 261,524,914 | 2,155,862 |

The authenticated removable sequence in every body is the byte flag load,
comparison with enabled, and conditional branch. The `ld1` half-open band also
contains an unrelated spill between the flag load and comparison. Charging the
entire band to the candidate deliberately overstates exposure; the true
removable share is no larger. Zero sampled period in `ld4`/`st4` is retained
rather than inferred from another width.

The once-per-`run_system_jit` setter cannot close the missing approximately
0.9 percentage points. It is one field store per host invocation, whereas the
measured guards execute in scalar guest memory helpers; combining it with
unrelated scheduler work would not describe one causal mechanism.

## Decision and restoration

No Bus associated constant, CPU-layout edit, export change, scorecard switch,
or release artifact was created. R054 fused interpreter memory remains active,
including its exact diagnostic disable capability. Evidence and a verified
manifest are under `target/bench/r123-fused-memory-static-guard/`.

The live product remains exact:

- core source `aec4b31434a6...`;
- Wasm runtime source `1da35e70bc9c...`;
- loader `2cbb264f4dac...`; and
- release Wasm `d9f686a9ce4f...` (4,279,380 bytes).

R123 stops before correctness, construction, native A/B, Chromium, WANIX,
`/shared/bench.py`, or scorecard timing because there is no candidate. Continue
from an independent active cost with enough complete exposure to support a
verified net 1% gain.
