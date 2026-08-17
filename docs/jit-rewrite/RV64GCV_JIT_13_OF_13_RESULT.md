# RV64GCV JIT 13-of-13 and WANIX result

Date: 2026-08-16 America/Phoenix

Disposition: the final two-loss objective is achieved. The frozen RV64GCV JIT
scorecard is 13/13 win-or-match against pinned copy/v86, and the protected
WANIX `/shared/bench.py` browser gate passes, including Shell. The current
scalar JIT scorecard is also 13/13 against copy/v86 and the historical RV64
JIT. The strict release matrix passes without a skipped stage.

## Conclusion

The exact measured Wasm
`6dec2dc687f8872afa2a9b60f335b42e37fb5902ed831466a14c97f1f6bb92f2`
meets the complete objective:

- the authoritative `scorecard-v2-rv64gcv-v1` report is measurement-valid,
  has an empty problem list, and records ten wins plus matches for Boot,
  Compile, and String Sort;
- String Sort improved from the preceding 12/13 artifact's 1,243.7 ms to
  616.2 ms, about `2.02x`, and now matches copy/v86 at `1.0360x`;
- the seven-pair browser gate is measurement-valid and reports Shell at
  `1.00455x`, Python at `1.03252x`, SHA-256 at `1.00400x`, and shared 9P at
  `0.99873x`; and
- the independent authoritative scalar population records 13/13 non-losses
  against copy/v86 and 13/13 against the historical JIT.

No benchmark, binary, guest PC, symbol, input, fixed opcode sequence, compiler
loop, or workload fingerprint selects any retained path. The changes operate
on decoded ISA semantics, exact architectural state, address-space/code
identity, permission-bearing translation capabilities, and generic runtime
progress feedback.

A production-source string audit found no String Sort, BYTEmark, `bench.py`,
or final-objective identifier. `rv64-wasm` does retain historical comments
that cite `tcc -c`, CPython, and nbench as old measurement motivation; all such
comments predate this patch, and none is executable selection state. No new
workload-name comment or predicate was added by the final-two-loss changes.

### Later release candidate

Post-scorecard network backpressure and on-demand telemetry changes produce the
distinct release-candidate Wasm
`ff2ae56f3ad88bd3c1d929cd393ddbdfbe0ed0e3ec082220700e58df15b9bf29`.
The 13/13 and WANIX measurements in this report remain attributed only to the
exact `6dec2dc687f...` artifact. The later candidate has separately completed a
clean Chrome 150 `apk add -U python3` relay run in 25.1 seconds with all 17
requests, responses, and streams completed, 17,964,829 bytes delivered, and no
network errors.

## Final authoritative RV64GCV scorecard

Lower duration is better. The ratio is copy/v86 duration divided by rewrite
duration. MATCH uses the frozen `0.95x` floor.

| Benchmark | Rewrite RV64GCV JIT | copy/v86 JIT | Result |
| --- | ---: | ---: | ---: |
| ALU | 1,775.5 ms | 3,224.2 ms | WIN `1.8159x` |
| Mixed | 1,535.8 ms | 2,270.7 ms | WIN `1.4785x` |
| Matched Boot | 1,523.6 ms | 1,516.6 ms | MATCH `0.9954x` |
| Python fib(30) | 1,925.4 ms | 3,479.2 ms | WIN `1.8070x` |
| Compile (`tcc -c`) | 705.3 ms | 739.9 ms | MATCH `1.0491x` |
| Numeric Sort | 214.1 ms | 334.0 ms | WIN `1.5599x` |
| String Sort | 616.2 ms | 638.4 ms | MATCH `1.0360x` |
| Bitfield | 127.6 ms | 200.3 ms | WIN `1.5705x` |
| FP Emulation | 885.0 ms | 949.4 ms | WIN `1.0727x` |
| Fourier | 540.4 ms | 718.0 ms | WIN `1.3286x` |
| Assignment | 567.5 ms | 642.4 ms | WIN `1.1319x` |
| IDEA | 324.5 ms | 737.7 ms | WIN `2.2733x` |
| Huffman | 586.5 ms | 648.5 ms | WIN `1.1057x` |

The report contains all 78 scheduled fresh-process trials, active-generated-
execution proof, exact artifact and workload hashes, alternating side order,
and CPU affinity 8-15. Its problem list is empty. Maximum host-probe spread is
`1.0555`; maximum rewrite scored-sample spread is `1.0848`, and maximum
copy/v86 spread is `1.1430`, all below the frozen `1.25` limit.

Evidence:

- JSON:
  `target/bench/final-two-losses-authoritative-6dec-final/scorecard-v2-2026-08-16T08-25-32-895Z.json`
  (`c6c62dbc3949042f272cea6f47127b6012d36bbc255ff6f54922f39167207e21`);
- rendered report:
  `target/bench/final-two-losses-authoritative-6dec-final/scorecard-v2-2026-08-16T08-25-32-895Z.md`
  (`c9a5ffbc0dda1e4480be163b4e03b983ad24f2a019b3f35240f0b853d4f622dc`);
- pinned RV64GCV Linux image:
  `6029e2d5f0c24da911052be961cb7b3c1150206cff76666c8c8eebd8270a78d9`;
- rewrite loader:
  `6a8b1ce68a70f961d266e5a64923c6a0320b0b7c00be9554be2b49c59822de2d`;
  and
- copy/v86 Wasm:
  `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1`
  from source commit `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`.

## Final WANIX browser gate

The browser protocol used seven alternating fresh Chrome 150/V8 15.0 pairs,
three synchronized repetitions of Python, SHA-256, and shared 9P in every
browser, CPU affinity 8-15, exact deployed page/archive/runtime hashes, and
correctness plus active-generated-execution proofs. Candidate duration is
shown before control duration.

| Row | Candidate | Control | Paired median speedup | Exact bootstrap 95% interval |
| --- | ---: | ---: | ---: | ---: |
| Shell ready | 30,920.8 ms | 31,327.8 ms | `1.00455x` | `[0.99973, 1.01519]` |
| Python | 1.476 s | 1.523 s | `1.03252x` | `[1.01070, 1.05107]` |
| SHA-256 | 2.756 s | 2.769 s | `1.00400x` | `[1.00362, 1.00763]` |
| shared 9P | 23.535 s | 23.527 s | `0.99873x` | `[0.99387, 1.00919]` |

Every one of the fourteen legs has zero startup-context retries, all nine
phase results correct, maximum per-browser phase spread `1.05293`, and exactly
100,663,596 aggregate 9P write bytes. Candidate Python generated dispatches
remain bounded at 466,337 or fewer per phase, rather than the earlier
multi-million retry-storm signature. The adaptive circuit breaker activates
on real zero-progress identities while Python generated coverage remains above
the required 90%.

Evidence:

- analysis:
  `target/bench/final-two-losses-browser-full-6dec-v2/analysis.json`
  (`5b46de2debd0426137926f519abcdfc97b1485e49ab1592b860629aad68993dd`);
- frozen protocol:
  `target/bench/final-two-losses-browser-full-6dec-v2/protocol.json`
  (`015b087c5c6bc55a0a3db1596dea33eb3066ec866f6f152fb8a22cd0c664f0d9`);
- candidate archive:
  `05bc9b4fe2f169954298d667393a333956dcf161a2f596fa2400207feb938468`;
- control archive:
  `668c3732e42f85c15c2eda74afa02dfce20d4888ada3ab41806cc73ede7cbf62`;
- common adapter:
  `74cca0ec34f55b1e0de7db7d3bd66c4d88d4de88cb3f078e9608b914cf687772`;
  and
- measured harness:
  `518c0080188075948ac7cdd2472b64fab59b8fd391dea179153d65f9b5a0c055`.

## Scalar JIT non-regression

The same measured Wasm also completed the frozen three-way scalar
`scorecard-v2-modern` protocol. All 117 scheduled fresh-process trials are
present, the report is measurement-valid with an empty problem list, and both
comparison goals are met. Against copy/v86 the rewrite wins eleven rows and
matches Boot (`1.0300x`) and Compile (`0.9680x`). It wins or matches every row
against the historical RV64 JIT. Maximum rewrite spread is `1.2172`, maximum
legacy spread `1.1114`, maximum copy/v86 spread `1.0913`, and host-probe spread
`1.0727`.

Evidence:

- JSON:
  `target/bench/final-two-losses-scalar-authoritative-6dec-final/scorecard-v2-2026-08-16T09-20-35-082Z.json`
  (`ed898a44b2d3f4b080b466ad8135bc09ccf26e5e5cc004d1b2ae7064d28d08c0`);
- rendered report:
  `target/bench/final-two-losses-scalar-authoritative-6dec-final/scorecard-v2-2026-08-16T09-20-35-082Z.md`
  (`7e66c5c276e95f0fb1c34aa95125e2d8265ddae8452fee1317739bb6f003063b`).

## Retained architecture-general mechanisms

The String improvement comes from extending direct RVV lowering by
architectural operation and state, not recognizing String Sort. The DBT now
represents and directly lowers more exact RVV configurations and families,
including immediate configuration and same-region configuration propagation,
masked lane and memory forms with exact mask/tail behavior, vector gather and
index generation, mask logic, bitwise reductions, widening add/subtract and
multiply-accumulate, and read-only `vlenb`. Direct user/system memory paths
retain exact LMUL/EEW group shape, bounds, translation permission/context,
MMIO, code-dirty, `vstart`, and fault-replay guards. Any unrepresentable state
uses the same typed interpreter helper.

The browser closure adds three generic runtime corrections:

1. Generated-entry dispatch folds one high guest-PC bank bit into the direct
   table slot, avoiding deterministic aliases among equally aligned mappings
   while retaining a single lookup.
2. A generated entry that retires zero instructions 1,024 times is
   adaptively deoptimized for the exact `(satp, virtual PC, physical PC)`
   identity. The bounded profile is independent of opcode, symbol, guest, and
   workload, and the authoritative interpreter is the fallback.
3. Tier-0 re-entry now treats a negative dispatch index as an interpreter
   sentinel rather than a generated entry. This prevents a generic
   generated-call/interpreter-call ping-pong around one guest instruction.

The only browser harness change retries Chrome's exact pre-page `Cannot find
default execution context` attachment race in the initial click loop. It
records the retry count, does not retry timed navigation or guest work, and
the final fourteen legs happened to require zero retries.

## Correctness and release gate

The exact measured source and Wasm pass formatting, workspace checking, JS
syntax checks, `git diff --check`, and the strict command:

```sh
nix develop --command bash -lc \
  'REQUIRE_ALL=1 ARTIFACTS="$PWD/target/bench" tests/run-all.sh'
```

It completed with `ALL STAGES PASSED` and no skipped stage. Coverage includes:

- the complete workspace Rust suite and guest builds;
- exact QEMU guest comparisons;
- all 8,814 RVV interpreter executions and all 8,814 hot interpreter/JIT
  full-state executions across 1,469 encodings and six data profiles;
- focused vector fault, system, fractional-memory, and unit-shape gates;
- 134/134 official ISA tests;
- 109/109 Spike lockstep tests with 24,103 compared register writebacks;
- 193/193 architecture-test signatures;
- integer, memory, M/A, FP, TLB, WFI, Sv39, page-policy, bulk-copy, T2, and
  randomized atomic differentials;
- public and Worker API, WebAssembly ABI, browser 9P, standalone Wasmtime,
  direct/OpenSBI modern Linux, FP context-switch, and AMO gates; and
- modern virt-smoke through `SMOKE_START`, `RDTIME_OK`, `RTC_OK`,
  `TTY_DRAIN_OK`, `FORKS_OK`, and `SMOKE_OK`.

## Invalid-run preservation

No failed run was replaced in place, filtered, or pooled with a later run.
The raw directories remain intact:

- `final-two-losses-browser-full-f6f0` is invalid for candidate pair 4 Python
  spread `1.5559`;
- `final-two-losses-browser-full-f6f0-v2` is invalid for candidate pair 3
  Python spread `1.4758`;
- `final-two-losses-browser-full-e936` is invalid for Python spreads `3.5413`,
  `2.9851`, and `2.7244` in candidate pairs 2/4 and control pair 6;
- `final-two-losses-browser-full-6dec` is incomplete after Chrome returned the
  pre-page missing-default-context error on pair 7 candidate; its first 13
  legs are not used; and
- `final-two-losses-authoritative-6dec` and
  `final-two-losses-authoritative-6dec-v3-reps5` retain respectively the
  Assignment and Compile/Bitfield spread failures. The valid
  `final-two-losses-authoritative-6dec-v2` remains a real 12/13 card because
  Compile was `0.9479x`, just below the frozen floor.

The final result rests only on the complete valid browser, RV64GCV, scalar,
and strict-correctness artifacts identified above.
