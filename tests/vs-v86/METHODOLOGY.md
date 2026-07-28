# Performance-session methodology

This is the required workflow for JIT tuning. It separates fast rejection,
candidate-to-control evidence, cross-emulator scoring, and correctness. A
number is not a result unless it comes from the appropriate stage.
Every decided experiment, including invalid and negative results, is recorded
in [`PERFORMANCE_PROGRESS.md`](../../PERFORMANCE_PROGRESS.md).

Observed cycle times on the current machine are approximately:

| Gate | Typical duration |
|---|---:|
| Build Wasm | 2-5 seconds |
| Parallel Compile rejection screen | 7-15 seconds |
| Three-pair serial Compile A/B | 30-45 seconds |
| Two-repetition five-row wall scorecard | 2-3 minutes |
| One standalone Numeric kernel | 30-40 seconds |
| One standalone Assignment kernel | 80-100 seconds |
| Three-pair Numeric/Assignment medium gate | 12-15 minutes |
| One-pair exploratory 13-row scorecard | about 5 minutes |
| Authoritative 13-row scorecard | 15-20 minutes |

This means Compile/Python and aggregate counters form the inner loop.
Numeric/Assignment are a medium gate for promising changes, not something to
run after every edit.

The rv64 nbench worker has an explicit six-minute completion guard
(`NBENCH_TIMEOUT_MS=360000` by default). Completion means all eight scored
kernel values exist; the unscored Neural Net/LU tests and trademark footer are
not required. Crossing the guard fails the entire trial, and partial scored
tables are never accepted. Current-host measurements after fixing completion
detection are about 33 seconds for Numeric, 87 seconds for Assignment, and
122 seconds for the whole eight-row rv64 table.

## Non-negotiable rules

- System emulation only. v86 has no comparable user mode.
- Keep an immutable control Wasm. Never compare numbers remembered from a
  different build, artifact set, machine state, or scorecard.
- Candidate effects below 10% are ties.
- Invalid runs are discarded, including good-looking rows.
- A saved v86 number is not promotion evidence. It is permitted only in the
  legacy flip screen after a valid authoritative scorecard.
- Profiling changes execution overhead. `PROFILE=1` output is diagnostic, not
  timing evidence.
- No parallel benchmark is timing evidence. Parallel screens reject grossly
  bad ideas only.
- Every promoted candidate must pass the complete 13-row authoritative
  scorecard and `tests/run-all.sh`.

The orchestrators take an exclusive lock in `ARTIFACTS`, so two local benchmark
runs cannot silently contend with each other.

## 0. Establish the session control

Build HEAD and save the exact Wasm before editing JIT code:

```sh
export ARTIFACTS="$PWD/target/bench"
export NIX_CONFIG="access-tokens = github.com=$(gh auth token)"

nix develop -c cargo build --release -p rv64-wasm \
  --target wasm32-unknown-unknown
nix develop -c node tests/vs-v86/snapshot-wasm.mjs head-control
```

The snapshot manifest records the Wasm hash, git revision, dirty status, and a
hash of tracked and untracked source state. Use its `.wasm` path as
`BASE_WASM` throughout the session.

Do not replace the control after a disappointing result. A new control starts
a new experiment series.

## 1. Rejection screen — seconds to a few minutes

Use a parallel screen only to catch breakage or a very large regression:

```sh
ARTIFACTS=$ARTIFACTS node tests/vs-v86/screen.mjs compile 4
ARTIFACTS=$ARTIFACTS node tests/vs-v86/screen.mjs numeric 3
```

The output is explicitly labeled `REJECTION SCREEN ONLY`. Do not quote its
timings as an improvement.

## 2. Serial candidate A/B — normal development gate

Build the candidate, then compare it directly with the immutable control:

```sh
BASE_WASM=$ARTIFACTS/wasm-candidates/<head-control>.wasm \
CANDIDATE_WASM=target/wasm32-unknown-unknown/release/rv64_wasm.wasm \
ROWS=compile,numeric,assignment REPS=3 ARTIFACTS=$ARTIFACTS \
  nix develop -c node tests/vs-v86/ab.mjs
```

The runner:

- starts every sample in a fresh Node process;
- alternates A/B order on each repetition;
- verifies the exact Wasm hash used by every child;
- checks workload checksums/object hashes;
- verifies hashes of the exact BIOS, kernel, disk, and benchmark bytes loaded;
- retains raw values and host probes;
- reports medians, median absolute deviation, dispatches, and
  instructions/dispatch;
- classifies less than 10% as a tie.

Select the motivated row plus likely regressions. For trace work, the minimum
set is `numeric,fourier,assignment`; for compiler work it is
`compile,python,numeric,assignment`.

Runtime configuration comparisons do not need rebuilds:

```sh
BASE_WASM=$CONTROL CANDIDATE_WASM=$CONTROL \
BASE_CONFIG='{"TRACELVL":3}' \
CANDIDATE_CONFIG='{"TRACELVL":0}' \
ROWS=numeric,fourier REPS=3 ARTIFACTS=$ARTIFACTS \
  nix develop -c node tests/vs-v86/ab.mjs
```

A candidate advances only with a valid result, at least one motivated
improvement of 10% or more, and no selected-row regression of 10% or more.

## 3. Diagnostic profile — explain before redesigning

Run profiling separately from timing:

```sh
ARTIFACTS=$ARTIFACTS WASM=$CANDIDATE_WASM PROFILE=1 \
  nix develop -c node tests/vs-v86/rv64-scorecard-worker.mjs compile \
  > "$ARTIFACTS/compile-profile.jsonl"
```

The result includes:

- JIT-retired instructions and dispatch count;
- instructions per dispatch;
- compiled block and superblock lifecycle counters;
- inline-cache extensions and TLB fills;
- a dispatch-site coverage fingerprint;
- the top dispatch PCs;
- the top interpreter-fallback instruction families.

Use these counters to state a falsifiable hypothesis. For example: “this
change removes repeated TLB fallback at PCs X/Y and should reduce compile time
by at least 10%.” Do not proceed from “dispatch count looks high” alone; the
inline-cache experiment proved that a 36% dispatch reduction can be wall-time
neutral.

Per-PC profiling is intended for short rows such as Compile and Python.
Dispatch-heavy nbench kernels remain multi-minute runs even with sampling; use
their always-on aggregate JIT counters for normal A/B. A deliberately slow
nbench profile requires `ALLOW_SLOW_PROFILE=1` and is diagnostic only.

## 4. Authoritative cross-emulator promotion

Only a candidate that clears the serial A/B gate runs this:

```sh
AUTHORITATIVE=1 ARTIFACTS=$ARTIFACTS NBENCH=1 SB=1 \
REPS=3 NBREPS=3 \
  nix develop -c node tests/vs-v86/scorecard.mjs
```

Authoritative mode refuses to validate unless:

- v86 is present;
- exactly 13 rows are scored;
- every wall row has three complete fresh-process trials per emulator;
- the whole nbench table has three complete fresh-process trials per emulator;
- paired side order alternates;
- checksums and object hashes are consistent;
- host drift stays within 1.25×;
- nbench does not report instability in a majority of repetitions.

The JSON retains every raw trial, JIT counters, host probe, runtime knob,
artifact hash, v86 revision, and Wasm hash. Never reconstruct a result from
the Markdown median alone.

If a decisive row is within 10% of either the old candidate or the 0.95
pass/fail boundary, repeat the authoritative run. Call contradictory results a
tie, not a win.

## 5. Correctness and promotion decision

```sh
ARTIFACTS=$ARTIFACTS nix develop -c bash tests/run-all.sh
```

Stages 4, 5, and 6 must run, not skip. Promotion requires all stages passing.

Land or retain a change only when:

1. its direct A/B effect is reproducible and at least 10%;
2. the authoritative scorecard is valid and introduces no new loss;
3. motivated code shapes have regression tests where practical;
4. the full correctness suite passes;
5. the report names raw evidence files and records tied/regressed rows.

Otherwise revert the experiment and record the negative result before trying a
different mechanism.
