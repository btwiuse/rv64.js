# Async Page Compile Policy

Status: policy shape selected; matched-workload constants refined by D050  
Selected: 2026-08-06  
Target: modern RV64 `VirtMachine`, direct Linux and OpenSBI Linux

## Outcome

The production system-mode policy no longer synchronously creates one Wasm
module for each hot PC. It interprets while accumulating approximate retired
instruction heat for a safe `(virtual page, physical page)` mapping, emits a
bounded multi-entry region only after that mapping is hot, and submits the
measured bounded number of `WebAssembly.compile` requests. Publication happens between
emulator slices. The stable `RV64` API enables this policy; `RV64Debug` retains
an explicit selector so differential tests can isolate the older dispatcher.

The current matched-workload defaults are:

- hotness threshold: 131,072 attributed retired guest instructions;
- sampling quantum: 1,024 interpreted guest instructions;
- queue bound: 64 mapping candidates;
- in-flight Wasm builds: two;
- stale candidate age: 2,097,152 globally sampled guest instructions;
- generated geometry: at most two pages and 512 leaders;
- multi-page admission: no more than 100 sampled non-sequential/control entries
  per thousand observations on every participating page;
- cross-module execution: frame-free tail transfer through one table-owning
  trampoline when the engine passes a Wasm tail-call feature probe;
- mapping identity: virtual page plus physical page, not SATP alone and never
  an unchecked physical-page-only alias.

This is a compile-policy result, not a claim that one threshold is optimal for
every future guest. All knobs and counters remain exported for controlled
experiments.

## Why the former behavior was slow

The rewrite compiler's generated steady state was already fast, but the
runtime reached it by synchronously translating and instantiating thousands of
small modules. Representative modern Linux runs produced roughly 2,000 module
builds and 12–18 seconds of cumulative frontend work. OpenSBI exposed a larger
early footprint and could produce more than 5,000 modules. The guest therefore
contended with its own second-stage Wasm compilation and made poor progress
before useful hot code landed.

The relevant cost is two-stage:

1. RV64 decode, region discovery, SSA lowering, optimization, and Wasm emission;
2. the browser's Wasm validation, baseline compilation, instantiation, and
   eventual engine tier-up.

Optimizing only the first stage or measuring only a warmed generated function
cannot select a production policy.

## Independent reference: copy/v86

Current copy/v86 commit `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`
uses several compatible ideas without being copied structurally:

- interpreted retirement is added to physical-page heat;
- the compile threshold is 200,000 instructions;
- interpreted entry points are collected per page;
- one 16-bit page or up to three 32-bit pages form a module;
- one global `compiling` state prevents a second concurrent build;
- guest analysis/emission is synchronous, while JavaScript calls asynchronous
  `WebAssembly.instantiate` and publishes the function on promise completion;
- writes invalidate page code and a 900-slot function table bounds residency.

Exact source references are recorded in [RESEARCH.md](RESEARCH.md). The RV64
policy differs where the architectures/runtime require it: Linux reuses
physical pages across many Sv39 address spaces, so heat is keyed by checked
VA→PA mapping; candidates are prioritized by current heat instead of strict
arrival order; incremental fragments cover late-discovered entries; and all
synchronous per-PC compilation is disabled while this policy is active.

## Trace and simulator evidence

Tracing is opt-in and does not add a branch to the ordinary interpreter path.
`Cpu::run_traced` reports retired PC, physical mapping, next PC, SATP, mode, and
instruction length. The modern machine also has a lower-overhead sampled
driver. Trace schema 2 records page heat, VA/PA/SATP contexts, entry PCs,
transfers, backedges, and sparse heat events.

Captured modern workloads:

| Trace | Retired instructions | Physical pages | Mapping contexts |
|---|---:|---:|---:|
| direct Linux boot | 42.14 M | 544 | 4,187 |
| OpenSBI + Linux boot | 48.60 M | 571 | 4,287 |
| long ALU | 551.36 M | 307 | 600 |
| long memory/ALU mix | 159.34 M | 262 | 512 |

Heat is concentrated: the ten hottest direct-boot pages account for 46.8% of
retirement and the hottest compute page accounts for about 98–99%. At a 200K
idealized threshold, mapping-key coverage is about 52.8% for direct boot,
56.5% for OpenSBI boot, and 98.4–98.9% for long compute.

The simulator showed that physical-page-only tracking has attractive reuse but
is unsafe when the same RAM page is mapped at different virtual addresses.
SATP-keyed tracking is safe but throws away useful sharing across processes.
The `(VA page, PA page)` identity recovers roughly nine percentage points of
boot coverage over SATP-specific bookkeeping near 200K while dispatch still
revalidates the live mapping before generated entry.

Simulation selected the policy shape, not the final threshold. It models ideal
coverage and bounded compiler latency, but real A/B runs exposed the cost of
compiling low-reuse boot pages. The initial compact-calibration 1M threshold
came from the end-to-end measurements below; matched CPython/SHA later
superseded that constant with the adaptive defaults listed above.

## Runtime algorithm

1. Interpret up to the configured quantum from the current PC. If this exact
   mapping and PC was already attempted or installed, use the unsampled exact
   interpreter because observation can no longer change policy; retain the
   same stop-at-compiled predicate.
2. Attribute retired work to that starting `(VA page, PA page)` mapping and
   remember the observed entry PC.
3. When mapping heat reaches the threshold, enqueue it unless it is pending,
   rejected, already queued, or under queue-pressure suppression.
4. While below the in-flight limit, discard stale candidates and choose the
   candidate with greatest accumulated heat, breaking ties by recency.
5. Discover bounded leaders from immutable physical code bytes, excluding
   entries already attempted or installed, emit one region module, and call
   the async host publisher.
6. Continue interpreting. The JavaScript promise compiles and instantiates the
   module, adds its function to the shared table, and calls `sys_sb_ready` only
   between Wasm invocations.
7. Publication rechecks the ticket/generation. Installed entry PCs become
   dispatchable. A later hot PC on the same mapping can form an incremental
   fragment rather than being stranded by the first page build.
8. A generated external successor validates its complete PC and current
   mapping generation, then may tail-call the single shared table trampoline.
   Generated modules never import the function table themselves.
9. A write to a marked physical code page removes its heat, entries, queued
   work, installed dispatch state, and generated dependencies. A late promise
   for invalidated code is harmless because its ticket is stale.

The queue uses hysteresis when full: a mapping offers once, then remains
suppressed until occupancy falls below half. This prevents a retry every sample
while preserving eventual progress. Default measurements reached a queue high
water mark of only two or three, with no drops or failures.

## Scheduler contract and the WFI bug

`WebAssembly.compile` completion runs on the JavaScript task/microtask system;
it cannot publish while the primary emulator Wasm call is still active. The
host therefore uses bounded guest slices and yields every slice while
`sys_pending_builds()` is nonzero.

The first threshold sweep appeared to show severe sampling overhead. The real
cause was a scheduler bug: the sampled interpreter could reach WFI after
retiring some instructions, but returned only the retirement count. The JIT
driver then spent the rest of the caller's two-million-instruction budget
across idle wakeups. Short shell commands acquired millions of unrelated guest
instructions and delayed output. The sampled machine now returns both
`(retired, yielded_on_wfi)`, and the driver returns to JavaScript on WFI even
when useful work preceded it. With compilation disabled, q1024 then matched the
exact bypass within run noise:

- ALU1: 144.5 ms policy control versus 148.8 ms exact-bypass median;
- ALU5: 559.8 ms versus 574.3 ms;
- mix20: 153.4 ms versus 149.2 ms.

This distinction matters to browser responsiveness as well as benchmark
validity.

## Threshold selection

The original long workloads were insufficient: any threshold from roughly
200K through 4M eventually reached 98% generated coverage. Deterministic short
and medium guest modes were added without changing the original checksums:

- `alu1`: one million register-only iterations, about 10M guest instructions;
- `alu5`: five million iterations, about 46M guest instructions;
- `mix20`: 20 passes over 256 KiB, about 9.5M guest instructions;
- original `alu` and `mix` remain the long steady-state controls.

Each threshold sample uses a fresh Node process and Linux VM. Workload outputs
are checksum-verified and run twice. The threshold sweep measures q1024 with
the exact-bypass report as a separate process-isolated control.

| Threshold | Boot median | Modules | ALU1 first | ALU5 first | mix20 first |
|---:|---:|---:|---:|---:|---:|
| 262,144 | 930.5 ms | 112 | 59.5 ms | 66.4 ms | 100.2 ms |
| 524,288 | 915.6 ms | 55 | 65.9 ms | 70.4 ms | 77.8 ms |
| **1,048,576** | **867.1 ms** | **26** | **58.4 ms** | **67.7 ms** | **89.8 ms** |
| 2,097,152 | 831.2 ms | 12 | 82.2 ms | 90.1 ms | 122.1 ms |

The exact interpreter boot median was 832.5 ms. At 1M, median first/repeat
speedups versus its separate exact-bypass medians were:

| Workload | First | Repeat | First/repeat generated coverage |
|---|---:|---:|---:|
| ALU1 | 2.55x | 3.55x | 74.1% / 91.0% |
| ALU5 | 8.49x | 11.58x | 94.3% / 97.9% |
| mix20 | 1.66x | 2.70x | 47.1% / 80.7% |

The 2M point minimizes boot compilation but sacrifices first-use work. The
262K point reaches compute just as quickly but compiles more than four times as
many modules and regresses boot. One million was the conservative knee for this
compact calibration corpus. Later matched CPython/SHA browser work selected
131,072 plus adaptive region geometry; D050 and [STATUS.md](STATUS.md)
supersede this historical constant.

## Actual browser A/B

The Chromium runner creates a fresh process and temporary profile for every
variant/sample, alternates process order, disables HTTP caching, and records
asset fetch and primary-emulator setup outside Linux/workload spans. DevTools
only polls the result outside timed regions. Both variants execute the same
Chrome 150 binary, kernel, disk, guest payload, shell transfer, and checksums.

Three-sample medians:

| Phase | Exact interpreter | Async page policy | Interpreter time / policy time |
|---|---:|---:|---:|
| Linux boot | 843.2 ms | 891.9 ms | 0.95x |
| ALU1 first / repeat | 134.2 / 138.9 ms | 57.0 / 40.2 ms | 2.35x / 3.46x |
| ALU5 first / repeat | 562.2 / 549.3 ms | 65.8 / 39.8 ms | 8.54x / 13.80x |
| mix20 first / repeat | 146.1 / 148.9 ms | 113.5 / 62.4 ms | 1.29x / 2.39x |

The policy emitted about 26–27 async-region modules. No synchronous module kind
was accepted by the harness. These are small-sample policy gates with raw data,
not universal confidence claims.

One OpenSBI correctness/performance gate ran all five payloads at the selected
settings. It reached Linux in 1,024.2 ms, emitted 32 async modules, executed
73.8% of ALU1 and 99.5% of long ALU in generated code on first use, and matched
every checksum.

## Why no dedicated compiler Worker is required

The async publisher uses `WebAssembly.compile`, allowing the engine to schedule
compilation without blocking the current JavaScript call. The implementation
does not assume that every engine dedicates a background thread; the two-build
bound protects guest progress even when compilation contends for the same CPU.
For UI isolation, rv64.js already supports running the whole emulator in a Web
Worker, which also moves translation and publication off the main thread.

A separate compiler Worker was tested in R057 rather than assumed beneficial.
The frozen accepted-R054 corpus transferred exact Boot and Compile modules to
two Workers and successfully cloned completed `WebAssembly.Module`s back.
Seven paired fresh-process measurements found no foreground-Wasm benefit:
Boot call/wall ratios were 0.998x/1.002x, while ordinary Boot module readiness
regressed to 0.489x control. The much larger Compile stream became ready
1.073x faster but still left foreground execution tied. Worker-local builds
were fast; buffer transfer, port scheduling, and module cloning consumed the
saving for normal modules. The raw report is
`target/bench/r057-compiler-worker-corpus.json`.

Therefore the separate service is rejected as a throughput default. A
`SharedArrayBuffer` still cannot publish a callable function into the existing
table; same-realm instantiation, cancellation, and generation validation would
remain necessary. An embedder may choose whole-emulator or compiler Workers
for UI isolation, but that is a responsiveness policy, not a supported speed
claim.

## Benchmark artifacts and commands

Primary reports under `target/jit-policy-traces/`:

- `calibration-medium-node.json`;
- `page-policy-threshold-sweep-q1024.json`;
- `page-policy-browser-chrome.json`;
- `page-policy-opensbi-default.json`;
- trace files `direct-boot.json`, `opensbi-boot.json`, `direct-alu.json`, and
  `direct-mix.json`;
- offline `simulation.json` and `simulation-calibrated.json`.
- matched five-pair Chrome and Edge protocols, logs, and `analysis.json` under
  `wanix-parity-known-fallback-pinned-8-15-{chrome,edge}150-20260807/`.

Reproduction commands are centralized in [REPRODUCING.md](REPRODUCING.md).

## Remaining policy work

- Repeat the real-emulator A/B on current Firefox/SpiderMonkey. The frozen
  region corpus already covers Firefox, but the new live page policy does not.
- Add a long-lived interactive/browser workload and report p95 pause and code
  memory, not only aggregate time.
- Make module capture a separate run when exact hashes are needed; retaining
  bytes during a timing run perturbs allocation.
- Consider additional adaptive admission using measured emitted bytes and
  observed reuse, but retain the shipped 131,072/control-density policy as the
  control. Avoid browser-brand branches.
- Revisit multi-function module partitioning and V8 tier re-entry only with
  cross-engine evidence; Wasm engines still control the second JIT.
