# Previous JIT versus clean-room rewrite

Date: 2026-08-06  
Upstream commit: `96aa93896e7bb6fa561d1f977c9bf23cd909a100`

## Result

The rewrite does not yet beat the previous JIT on workloads where both
implementations actually generate code. It is about 2.00x slower for a single
long-running ALU invocation, 1.44x slower for mixed user code, and 1.07x slower
for the fixed legacy-Linux MD5 work. The first gap is specifically a Wasm
baseline-tier/re-entry effect: it disappears when V8 starts with TurboFan or
when the host returns and re-enters every one million guest instructions.

The rewrite is about 1.97x faster for fixed work under both current Linux boot
paths. That is an important product improvement, but it is not an old-JIT
versus new-JIT code-quality result: the previous build records zero generated
instructions on the modern `VirtMachine`, while the rewrite records 92.75% to
98.33% coverage.

All ratios below are paired `old_time / new_time`; greater than one means the
rewrite is faster. The interval is a deterministic bootstrap 95% interval for
the paired median.

| Workload | Comparison | Old median | New median | Paired ratio (95% interval) | Generated coverage old -> new |
| --- | --- | ---: | ---: | ---: | ---: |
| user ALU, one 50B-instruction host slice | JIT vs JIT | 1,974.8 ms | 3,955.8 ms | 0.499x (0.472-0.506) | 99.99% -> 99.99% |
| user mixed integer/memory | JIT vs JIT | 2,172.7 ms | 3,126.1 ms | 0.696x (0.673-0.712) | 98.08% -> 99.91% |
| legacy Linux, 4 MiB zero-stream MD5 | JIT vs JIT | 1,047.5 ms | 1,118.6 ms | 0.936x (0.924-0.952) | 91.68% -> 91.59% |
| modern Linux, direct SBI, fixed shell work | product only | 11,788.5 ms | 6,058.8 ms | 1.965x (1.925-1.986) | 0.00% -> 92.75% |
| modern Linux, OpenSBI, fixed shell work | product only | 11,780.7 ms | 5,993.7 ms | 1.977x (1.949-2.004) | 0.00% -> 98.33% |

Cold-to-ready behavior is separate from fixed post-boot work:

| Boot path | Old median | New median | Paired ratio (95% interval) | Interpretation |
| --- | ---: | ---: | ---: | --- |
| legacy BBL/Linux | 1,506.1 ms | 1,578.2 ms | 0.949x (0.947-0.963) | rewrite is about 5.3% slower |
| modern direct SBI | 1,230.4 ms | 1,227.8 ms | 1.005x (0.991-1.092) | indistinguishable at this resolution |
| modern OpenSBI | 1,398.7 ms | 2,379.1 ms | 0.587x (0.569-0.626) | rewrite is about 1.70x slower to prompt |

The OpenSBI cold regression is consistent with eager compilation over a much
larger early footprint. A representative rewrite run registered 5,394 modules,
emitted 11.69 MiB, and spent 162.9 ms translating, versus 1,889 modules,
3.93 MiB, and 60.7 ms on direct boot. The reported asynchronous compile
durations overlap and must not be summed as serial wall time.

## Methodology and validity

The previous implementation was built as a black box from an isolated
`git archive` of the same upstream commit. Its compiler source was not opened
or imported into the rewrite. The active tree used `rv64-dbt`; the isolated
tree used the previous crate and loader exactly as upstream supplied them.

The accepted headline report uses:

- Node 26.5.0 and V8 14.6.202.34-node.24 on Linux x86-64;
- nine paired samples per workload;
- a fresh runtime process for every side of every pair;
- alternating old/new order;
- untouched production V8 tiering flags;
- native CPU probes before and after every leg;
- a predeclared 1.25x probe-spread/drift rejection threshold;
- exact loader, main-Wasm, and guest-input SHA-256 identities;
- raw samples, medians, p95, and bootstrap median intervals;
- separate, untimed generated-module capture.

The accepted run's CPU-probe spread was 1.197x, with no validity issue. An
earlier seven-pair all-in-one run had a valid 1.190x headline spread but a
1.265x spread during tiny frozen-frontend probes, narrowly failing the global
threshold. It is retained as rejected evidence; its deterministic module
hashes and sizes are useful, but its frontend timings are not headline data.

The median does not describe all ALU tail behavior. One of nine ordinary
rewrite samples took 13,010.1 ms while the other eight took 3,919.7-4,176.5 ms;
all previous-JIT samples took 1,969.1-2,012.3 ms. Its bracketing parent probes
were normal. A second nine-pair run constrained the whole process tree to CPUs
8-15 and again produced one rewrite outlier (11,155.5 ms), with a 0.443x paired
median; its previous-JIT range was only 1,963.7-1,967.0 ms. Affinity therefore
does not remove the tail. The paired median conclusion is repeatable, but the
rewrite also has an unexplained cold-tier latency risk that should be treated
as a performance bug rather than discarded as host noise.

The exact compared main artifacts were:

| Variant | Main Wasm bytes | Main Wasm SHA-256 | Loader SHA-256 |
| --- | ---: | --- | --- |
| previous | 3,612,552 | `a1e9ee2a0eadc7e63923582395c31c3a797569c16f1aad33b791d406a2e9a2b1` | `54df79c8b35cf50bcee34c4af02d7eb02b09e0439b717ee75bb830e733595b12` |
| rewrite | 3,861,625 | `542ec23b1f958720375d2bf70ffc6b2d8fb52859d1468653feb285e75cef8693` | `0e856cf1ef62e68251276c886cc7628a6edf4da2a09218420cf5fa159b415618` |

Correctness gates passed in every accepted sample. The ALU sides each retired
`8,000,000,113` instructions with the same output hash. The mixed sides each
retired `1,638,856,629` instructions with the same output hash. Every legacy
run produced MD5 `b5cfa9d6c8febd618f91ac2843d50a1c`; modern runs reached the same
`OLD_NEW_200010000` guest marker, settled pending builds, and direct SBI
reported zero unsupported extensions.

## Why the ALU result changes with the Wasm tier

The ALU workload installs two modules and enters the hot module once for a very
long internal Wasm loop. V8 compiles both variants with Liftoff, later finishes
TurboFan compilation, but cannot replace the currently executing invocation.
The optimized function is useful only after the host re-enters it.

| Diagnostic mode | Old ALU | New ALU | Paired ratio | Meaning |
| --- | ---: | ---: | ---: | --- |
| normal V8, one long call | 1,974.8 ms | 3,955.8 ms | 0.499x | production cold-tier behavior |
| Liftoff only | 1,981.0 ms | 3,926.6 ms | 0.510x | rewrite baseline code is about 1.96x slower |
| TurboFan from first call | 1,884.3 ms | 1,869.1 ms | 1.009x | optimized code is effectively tied |
| normal V8, 1M guest-instruction re-entry | 1,809.6 ms | 1,791.9 ms | 1.010x | 8,001 calls let optimized code take over |

Generated-Wasm inspection explains the Liftoff sensitivity. In the hot ALU
module, the previous function declares 15 locals and contains 45 `local.get` /
`local.set` operations. The rewrite declares 35 locals and contains 86 local
operations because `emit_single_latch_loop` assigns every SSA value a local,
including constants and single-use values. TurboFan eliminates much of that
traffic; Liftoff does not.

This does not make the one-call result artificial. It is the actual result for
an embedding that enters a long generated loop once. It does mean the result
must not be described as the rewrite's optimized steady-state throughput. A
portable runtime needs either baseline-friendly emission or a bounded
re-entry policy; it cannot assume that a host optimizing compiler can replace
an active Wasm call.

## Why mixed code remains slower

The mixed workload returns through compiled edges frequently enough for V8
tiering, but the rewrite still loses:

- previous: 59,804,397 generated dispatches, 23 modules, 126,638 emitted bytes;
- rewrite: 105,986,488 generated dispatches, 22 modules, 15,803 emitted bytes;
- coverage improves from 98.08% to 99.91%, so interpreter fallback is not the
  explanation;
- Liftoff-only paired ratio is 0.503x and TurboFan-from-start is 0.605x.

The rewrite emits much smaller modules but crosses the runtime dispatcher about
1.77x as often. The immediate targets are therefore both stack/local quality
and more profitable same-module edge fusion. More instruction lowerings or a
higher coverage percentage alone will not close this gap.

## Cross-engine diagnostic

Three-pair local diagnostics used the same artifacts and fresh-process paired
protocol. They are supporting evidence rather than the nine-pair headline.

| Runtime | ALU paired ratio | Mixed paired ratio |
| --- | ---: | ---: |
| Node 26.5 / V8 14.6 headline | 0.499x | 0.696x |
| Deno 2.1.9 / V8 13.0 | 0.503x | 0.652x |
| Bun 1.2.11 / JavaScriptCore | 1.042x | 0.526x |

An additional affinity-constrained Node run reports 0.443x for ALU and 0.676x
for mixed code. The mixed result agrees with the ordinary run; the ALU result
reinforces both its baseline-tier deficit and its unstable tail.

JavaScriptCore does not reproduce the long-call ALU penalty, but it penalizes
the mixed shape more strongly. That confirms the need to report exact engine
identity and to select portable defaults from multiple Wasm engines.

## Prioritized performance work

The comparison changes optimization order. The highest-value experiments are:

1. Replace one-local-per-SSA-value loop emission with liveness-based local
   reuse and stack scheduling. Inline constants and single-use values, and use
   `local.tee` only where it reduces real traffic.
2. Add a selective internal-loop re-entry budget for long single-entry loops.
   Measure break-even across V8 and JavaScriptCore instead of globally capping
   all regions.
3. Reduce mixed-workload dispatcher entries through direct same-module edge
   continuation and profile-guided region formation. Treat dispatch count as a
   first-class regression metric.
4. Delay or raise compilation thresholds during OpenSBI boot, and separate
   firmware-era code from durable Linux hot code so early translation does not
   dominate time-to-prompt.
5. Keep three regimes in every optimization report: production cold tier,
   bounded re-entry/time-to-tier, and explicitly stabilized top tier. Diagnostic
   V8 flags must never replace the production headline.

The rewrite outline and correctness delivery are complete. These items are the
next optimization phase, not missing implementation gates from Phases 0-6.

## Reports

Generated reports are deliberately under `target/` and are not source
artifacts:

- `target/jit-old-new-headline.json`: accepted nine-pair headline;
- `target/jit-old-new-comparison.json`: rejected global-probe run plus exact
  captured corpora;
- `target/jit-old-new-liftoff.json`: Liftoff-only diagnostic;
- `target/jit-old-new-turbofan.json`: TurboFan-from-start diagnostic;
- `target/jit-old-new-alu-reentry.json`: one-million-instruction re-entry;
- `target/jit-old-new-pinned-user.json`: CPU-affinity sensitivity run;
- `target/jit-old-new-deno-pilot.json` and
  `target/jit-old-new-bun-pilot.json`: cross-engine diagnostics.

See [REPRODUCING.md](REPRODUCING.md) for commands.
