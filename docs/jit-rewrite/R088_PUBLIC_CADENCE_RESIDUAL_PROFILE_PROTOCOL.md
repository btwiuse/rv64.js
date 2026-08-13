# R088 Exact-R085 Public-Cadence Residual Profile Protocol

Date: 2026-08-09  
Status: diagnostic complete; exact re-entry product prototype admitted

## Question

Under R087's corrected one-slice-per-event-loop-turn scorecard cadence, which
architecture-general host operation owns the remaining exact-R085 Boot or
Compile gap, and does the exact generated-entry re-entry boundary still clear
the standing 3% cumulative-gain opportunity rule?

This is attribution only. Inspector-perturbed wall times are never performance
evidence. No product source, Wasm, loader, guest, policy, threshold, generated
module, or workload changes in R088.

## Frozen identities and collection

Use exact promoted R085 product bytes and R087's corrected harness:

- runtime Wasm
  `efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`;
- loader
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- corrected authoritative report
  `1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7`;
- scorecard driver `d329d7746987375772941a9535b59345bb2a79ffa304aaf8b2da6b1df5364685`;
- worker `abe22741809ffe158cde9b2947183d1e8a77f7dc246dc917a36852bc56352791`;
- cadence helper `ed7b65cab81fa963804dfd9da6108417e27e7ed3f53b640cc06cdfcf86f3eaaa`;
- scorecard library `8681b09f81f3c71e30945d5770486517d993c712b2394c017b4980d481e31c61`;
  and
- frozen closure classifier
  `efce26fcf275550efaa26c2d95d22e4e71d2243b189db2c9d2245b92e6181be5`;
  and
- R088 identity wrapper
  `645e57e1ebbd647db74165c4a8dadf58c27dff914897a4a22771ef1061887d26`.

Pin the process to CPUs 8--15. Collect one fresh rewrite process for modern
Boot and Compile with a fixed 250-microsecond V8 sampling interval. Capture
Boot FIRST and Compile FIRST, PRIME, and STEADY. The default cadence must be
recorded as `public-one-slice-per-turn` in every proof result; neither cadence
diagnostic variable may be set.

```sh
taskset -c 8-15 env \
  ARTIFACTS=/home/darren/src/arm64.js/target/bench \
  SIDES=rewrite ROWS=boot,compile REPS=1 \
  SCORECARD_V2_OUTPUT="$PWD/target/bench/r088-r085-public-cadence-profile/report" \
  SCORECARD_V2_ENGINE_PROFILE_DIR="$PWD/target/bench/r088-r085-public-cadence-profile/phase" \
  SCORECARD_V2_ENGINE_PROFILE_INTERVAL=250 \
  node tests/vs-v86/scorecard-v2.mjs
```

Run `tests/vs-v86/analyze-r088-residual.mjs` over the four profiles in phase
order and write `closure-analysis.json` once. It wraps the immutable R086
classifier and records that classifier's hash in the output.

## Frozen decision rule

The complete descending operation-family prefixes must explain at least 95%
of Boot and Compile STEADY. Reconfirm exact R085 product identity, corrected
cadence, modern guest identity, exact output, production policy, and nonzero
generated execution. Do not compare or quote profiled wall time.

Keep all mechanisms closed through R087 unless this whole-profile collection
shows a new contradiction. In particular, do not reopen hash choice, map
capacity, fetch/TLB layout, module geometry, tier thresholds, worker count,
generated-memory proof shape, workload-specific selection, or blind callback
thinning.

R056's immutable exact-call-shape result is the only admissible prior local
measurement for re-entry monomorphization: direct inline lookup was 1.494x
the indirect callback with exact hit/miss behavior. That mechanism earns one
new product prototype only if R088 attributes at least 9.0% of complete Boot
self time exclusively to `interpreter loop with exact generated re-entry`.
At 9.0%, the frozen local result projects just over a 3% whole-Boot speedup.
The implementation must preserve every per-instruction predicate call and
may only make its concrete dispatch-table lookup statically visible through
the Rust/Wasm call graph. Compile and Python remain guard rows.

If the 9.0% gate fails, do not implement it. Select another distinct family
only when exclusive whole-row attribution and a measured local mechanism
together project at least 3%. If no family qualifies, record the plateau and
move to a broader execution representation rather than tuning a closed axis.

## Result and decision

The collection completed once using exact R085 and the corrected public
cadence. The proof-only scorecard report is
`target/bench/r088-r085-public-cadence-profile/report/scorecard-v2-2026-08-09T20-07-30-143Z.json`,
SHA-256 `03e6b354e6917f9f2e71363313497358ee3f06b4b27425b3f21b18496147eff4`.
Its only reported problems are the two expected declarations that inspector
profiles entered Boot and Compile measurements; those durations remain
excluded. Every result proves exact product/guest/input identity, production
policy, one-slice cadence, and generated execution.

Raw profile SHA-256 values are:

- Boot FIRST `762f59cd05df26f79afc38f732e20ca6c1050117ae12bd227b1d4ef391c1ed56`;
- Compile FIRST `eba71f62efa791e79c780c849ee2c9dfeeb621be5806ab9cf72e78b70b26ff0c`;
- Compile PRIME `b48eea7210687229ed6098c57f4570445c8ca39cd9d1439923985a5f546fbe07`;
  and
- Compile STEADY `17f01833b0d4f4ab053d061e1a5e8dcc11a0763363c5abb03689e688e3d08f49`.

The deterministic closure report is
`target/bench/r088-r085-public-cadence-profile/closure-analysis.json`, SHA-256
`d0568e0da0bfafca9b9c8ea74f52441908b278afaf4bb7ac2c957153b83ef291`.
Its descending families explain 95.868% of Boot and 95.802% of Compile
STEADY. Boot assigns 51.972% to interpreter decode/execute, 12.719% to exact
generated re-entry, 8.837% to scalar memory helpers, 5.917% to translation,
4.767% to generated execution, and 4.259% to scheduler self. Compile STEADY
assigns 40.684% to generated execution, 16.898% to scheduler self, 15.123% to
decode/execute, 5.548% to translation, and only 1.763% to exact re-entry.

`tests/vs-v86/r088-profile-gate.mjs` (`5af3409b2733...`) independently
verifies the proof contract and frozen rule. Its result is
`target/bench/r088-r085-public-cadence-profile/gate.json`, SHA-256
`a96d6eb2e46a33ae93cd424f086e5cfd4894e93fc1a96e008242fd4575ca7d8d`,
with no problems. Applying R056's immutable 1.494x local result to the 12.719%
exclusive Boot fraction projects 1.0439x whole-Boot speedup. This clears the
9%/1.03x admission gate and authorizes exactly one R089 product prototype.
