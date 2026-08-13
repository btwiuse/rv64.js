# R124 Exact-Product One-Percent Gate Freeze

Date: 2026-08-10  
Status: frozen before construction or native product timing

## Scope already fixed

The candidate is the sole architecture-general R124 implementation described
by the attribution protocol: `RegisterStructured` retains exactly
x1/x2/x8--x15 and synchronizes every other referenced integer register at
actual generated member boundaries.  There is no runtime selector, guest PC,
workload, binary, opcode-frequency, or host-engine specialization.  FP state,
CFG shape, thresholds, chaining, fuel, and publication policy are unchanged.
The existing whole-loop bulk-copy lowering and precise side exits remain
enabled and were adapted to the same state contract.

No exact-product construction or native scorecard timing existed when this
file and the evaluator were frozen.  The earlier synthetic result is excluded
for the topology error recorded in the R124 result document.  Correctness-test
wall clocks and isolated build durations are not performance evidence.

## Immutable product artifacts

| Artifact | SHA-256 | Bytes |
|---|---|---:|
| control Wasm | `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d` | 4,279,380 |
| candidate Wasm | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| isolated candidate A | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| isolated candidate B | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |

The candidate is 2,406 bytes larger.  This is recorded only as a construction
fact and has no promotion or rejection role.

Frozen source identities:

- DBT API/layout: `e4be5025e892f417fdde56dfc0c4c5ead632b474de9ea849558b9ed7ffbed795`;
- DBT emitter: `f60b8ae438cf1fcec7dc22215e6e3da5caf1755a7d5c214ec09b5928eb54d96e`;
- Wasm runtime: `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- loader: `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- deterministic source archive:
  `6497fe464b64113525620f0f1ae4ac767a2137fd09b7d5cb843ba893f5627829`.

## Correctness admission

Before this freeze:

- all 177 workspace tests passed, including the new fixed-bank structural
  assertion;
- 60 randomized integer programs and the M, A, and FP differential suites
  matched full interpreter state;
- system A, FP, bulk-copy, MMIO/cross-page/code-page exits, fused memory, Sv39,
  and MPRV differentials passed;
- all 12 randomized atomic/T2 cases and all 11 multi-entry/T2 cases passed;
- public API, Worker API, raw-Wasm ABI, retirement accounting, and 338 emitted
  module validations passed; and
- direct and OpenSBI Linux 6.12.7 / Alpine 3.24.1 boots reached the shell with
  generated execution.  Legacy BBL/TinyEMU tests were deliberately excluded.

No legacy kernel, BBL firmware, or TinyEMU root is an input to this gate.

## Frozen harness and inputs

- prospective evaluator: `tests/vs-v86/r124-native-gate.mjs`,
  `a5dc17b19e9d...`;
- evaluator selftest: `tests/vs-v86/r124-native-gate-selftest.mjs`,
  `9b650942cf96...`, passed before freeze;
- construction harness `d835b2f83a14...`;
- native A/B harness `74d624e8b401...`;
- worker `346c240378c8...`;
- scorecard library `377f32f4a5fb...`;
- benchmark math `a8bdea3fb105...`;
- cold-cost accounting `ddc401ff9f43...`;
- statistics `7b9454f5523f...`;
- Linux 6.12.7 image `57d077974820...`; and
- Alpine 3.24.1 riscv64 initramfs `cbb75afb016d...`.

Use Node `v26.5.0`, V8 `14.6.202.34-node.24`, CPUs 8--15, production page
policy, public one-slice-per-turn cadence, and fresh processes.  Run 15
alternating pairs for Boot, Compile, and Python.  Every leg is retained.

## Commands frozen before timing

```sh
mkdir -p target/bench/r124-rvc-bank-hybrid/product-gate/native

taskset -c 8-15 node tests/vs-v86/main-runtime-construction.mjs \
  target/bench/r124-rvc-bank-hybrid/product-preflight/artifacts/control-d9f686a9.wasm \
  target/bench/r124-rvc-bank-hybrid/product-preflight/artifacts/candidate-d017a10f.wasm \
  target/bench/r124-rvc-bank-hybrid/product-gate/construction.json 15

ARTIFACTS=/home/darren/src/jit/target/bench \
ROWS=boot,compile,python REPS=15 \
SCORECARD_V2_OUTPUT=/home/darren/src/jit/target/bench/r124-rvc-bank-hybrid/product-gate/native \
CONTROL_CONFIG='{"SCORECARD_V2_REWRITE_WASM":"/home/darren/src/jit/target/bench/r124-rvc-bank-hybrid/product-preflight/artifacts/control-d9f686a9.wasm"}' \
CANDIDATE_CONFIG='{"SCORECARD_V2_REWRITE_WASM":"/home/darren/src/jit/target/bench/r124-rvc-bank-hybrid/product-preflight/artifacts/candidate-d017a10f.wasm"}' \
taskset -c 8-15 node tests/vs-v86/scorecard-v2-config-ab.mjs

node tests/vs-v86/r124-native-gate.mjs \
  target/bench/r124-rvc-bank-hybrid/product-gate/native/config-ab-<timestamp>.json \
  target/bench/r124-rvc-bank-hybrid/product-gate/construction.json \
  target/bench/r124-rvc-bank-hybrid/product-gate/native-gate.json
```

Execute this sequence once.  Do not replace, extend, pool, trim, or selectively
rerun samples after seeing the result.

## Prospective decision

Compile is the sole target.  After debiting the full observed candidate-minus-
control construction cost, require:

- paired Compile median at least `1.01x`;
- paired-bootstrap 95% lower bound at least `1.00x`; and
- normalized fixed-work Compile throughput at least `1.01x`.

Boot and Python are protected: each debited paired median must be at least
`0.99x`, and neither interval may establish a regression.  Every artifact,
input, output, fixed-work, cadence, policy, affinity, generated-execution,
settling, and host-stability check must pass.

A confidence-verified 1% gain passes.  Candidate size is never a veto.  A
survivor advances unchanged to natural Chromium, qualified WANIX including
`python /shared/bench.py`, and the untouched 117-trial three-way scorecard.  A
failure is archived and exact `d9f686a9...` is restored without tuning the
bank or trying another state subset.
