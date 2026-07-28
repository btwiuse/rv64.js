# Performance progress and experiment ledger

This is the canonical status tracker for rv64.js JIT performance work. It
records what is active, what worked, what failed, why decisions were made, and
which evidence supports them.

Testing procedure belongs in
[`tests/vs-v86/METHODOLOGY.md`](tests/vs-v86/METHODOLOGY.md). Architecture
belongs in [`DESIGN.md`](DESIGN.md). Future project-wide work belongs in
[`ROADMAP.md`](ROADMAP.md).

The former `ISSUES.md` was an append-only investigation narrative. All review
items in it were resolved, while later performance sections contained both
useful results and conclusions invalidated by subsequent measurements. Its
full text remains recoverable from Git at commit `1ec9130`; this document
retains the decisions that should guide future work.

## Current status

- **Best known engineering baseline:** the E003 narrow-window default in the
  current repository state (checkpoint parent `a161200`).
- **Frozen performance control:** Wasm SHA-256
  `21b638123cee4072cb84397c98f9cf340dad6e05e3907845bff4dc66642f50a1`.
- **Control artifact:** `target/bench/wasm-candidates/`
  `head-control-21b638123cee.wasm`.
- **Promoted default build:** Wasm SHA-256
  `b5a857b087bd49631866c9bcd77e3d9d98df1dfb93eedf3ce16782c3e02d4433`.
  It differs from the scored `c8a196b5…` artifact only by making the tested
  `TRACEWIN=1` runtime state the default, comment/source-location metadata,
  and use of the project-pinned Rust 1.97.1 toolchain; the final build passed
  all eight correctness stages.
- **Comparison v86 revision:** `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`.
- **Reproducible scorecard ceiling:** 11/13 with the narrow-window candidate.
  Reports `scorecard-2026-07-28T05-38-26.json` and
  `scorecard-2026-07-28T06-29-49.json` are valid authoritative host-toolchain
  results; `scorecard-2026-07-28T07-03-26.json` is the valid canonical
  Nix-toolchain result on the final artifact.
- **Current likely losses:** Compile and Assignment. Python has
  historically been bimodal and must be classified only by a replicated
  candidate-relative run.

An exploratory scorecard is a plumbing check, not status evidence. Only a
valid `AUTHORITATIVE=1`, `REPS>=3`, `NBREPS>=3` scorecard may change the
scorecard standing.

## Decision rules

1. Keep the frozen control for the entire experiment series.
2. State a falsifiable mechanism and expected affected rows before editing.
3. Parallel runs reject broken or grossly slow ideas; they never establish a
   win.
4. Use alternating, serial, fresh-process control/candidate trials for claims.
5. Treat effects below 10% as ties.
6. Reject incomplete, unstable, checksum-mismatched, input-mismatched, or
   host-drifted runs regardless of their apparent result.
7. Do not run the full scorecard until a candidate clears its focused A/B gate.
8. Do not retain a candidate that introduces a selected-row regression of 10%
   or more.
9. Record negative results here before starting the next mechanism.
10. Require the authoritative scorecard and all eight `tests/run-all.sh`
    stages before promotion.

## Resolved correctness and measurement work

These were review issues, not open work. Reopen one only with a new failing
test or concrete counterexample.

| Area | Resolution | Evidence/reasoning |
|---|---|---|
| Fuel and interrupt contract | Fixed in `a5f1620`: dynamic fuel cell and retirement-bounded chaining | `user_run(1)` overshoot fell from 16,777,224 instructions to a bounded single-block overshoot; Wasm smoke enforces the contract |
| Symmetric benchmark timing | Fixed in `a5f1620`, `d1cb131`, and the current isolated worker | Both emulators use streamed markers and equivalent fresh-process timing; paired order now alternates |
| System FP architectural state | Fixed in `a5dd523`, `f540317` | FP eligibility/Dirty semantics plus two-process FP context-switch coverage |
| Exact bailout retirement | Fixed in `a5dd523` | Interpreter/JIT retirement differential is bit-identical |
| Emitted-JIT differential tests | Added in `caec17b` and subsequently expanded | Randomized full-state JIT/interpreter comparisons run in stage 7 |
| Scorecard completeness | Initially fixed in `412fd82`, `ae1e5c1`; replaced by the current authoritative protocol | Required rows, exact repetitions, checksums, input/Wasm hashes, raw trials, instability checks, and nonzero invalid exit |
| Cache lifecycle | Fixed in `a2c88e8`, `8607ed8` | Boot clears JIT state; disabling JIT stops dispatch; table growth is bounded |
| Cross-ISA workload equivalence | Fixed in `fdf25f1`, `662531b` | Common integer widths, controlled FP flags, and checksum enforcement |
| Hot-counter aliasing | Fixed in `412fd82` | Direct-mapped slots now carry full-PC tags |
| `sys_insn_count` export | Restored in `a5f1620` | Used by retirement validation |
| Reserved encoding handling | Hardened in `cebcd91` | Shared predicates and tests prevent scanners/emitters from accepting reserved encodings |
| Silently skipped validation | Fixed in `4464a6f`, `318b55d`, `15be658` | 134 ISA tests, 109 lockstep tests, 193 architecture signatures, and AMO differential execute rather than skip |
| Benchmark-run contention | Fixed by the current artifact-directory lock | Concurrent local orchestrators fail instead of contaminating each other |
| nbench completion | Fixed by required-row completion rather than the unavailable Neural Net/footer | Whole rv64 table now completes in about 122 seconds instead of timing out after producing all scored rows |

## Performance work that succeeded

These mechanisms are part of the current baseline. Re-evaluate them only if a
new test shows a regression or a replacement mechanism is being compared.

| Mechanism | Outcome | Why it worked |
|---|---|---|
| Correct superblock landing | Finished page functions stopped being discarded because installation no longer re-probes an unrelated current address space | Mapping verification moved to the first dispatch in the requesting address space |
| Multi-page regions | Numeric improved dramatically over page-clamped 2–10 instruction fragments | Hot loops crossing page boundaries could execute inside one region |
| Productive region rebuilds | IDEA coverage became much less bimodal | Newly hot PCs can enter a rebuilt region; only unproductive rebuilds consume the allowance |
| FSGNJ.D, F extension, and AMO compilation | Large Fourier/Huffman/atomic coverage gains | Common instruction families stopped terminating compiled regions |
| Interpreter-equivalent FP eligibility | Removed tens of millions of unnecessary Fourier bailouts | Zero-operand and rounding behavior now follows the interpreter |
| Per-body FP gates | Integer entries no longer bail merely because another body in the function contains FP | Eligibility is checked at the first relevant instruction/body |
| Trace compilation (`e950060`) | Compile improved materially at `SB=0`; direct calls, returns, and branch side exits stay in one generated function | Executed paths became longer without requiring host dispatch |
| Whole-region discovery (`1ebecdc`) | Recovered bodies omitted by a leftover per-page bisect configuration | Leaders are discovered across the complete region |
| Retirement/fuel write thinning | Removed repeated host-memory bookkeeping | The generated/host ABI performs fewer redundant stores |
| Monomorphic indirect-target inline caches (`c155cbd`) | Compile dispatches fell about 32%; Fourier, String, and typical Python improved | Stable `jalr` targets can continue a trace behind one guard |
| Batching default off (`3855c72`) | Preserved Numeric instead of trading it for an unreliable Python/Assignment draw | Global batching helped some rows but cost Numeric roughly 25% |

## Closed or rejected approaches

Do not retry these unchanged. The final column states what new fact would be
needed to justify reopening the design.

| Approach | Result and reason for rejection | Reopen only if |
|---|---|---|
| Per-block `return_call_indirect` chaining | Fast hops, but importing the shared table made later `table.set` registration quadratic; Compile slowed 2.4–3× even when chain code did not execute | The runtime provides a non-quadratic table/instance registration path |
| Shared per-module chain helper | Smaller sites, same shared-table import and same quadratic behavior | The table-import cost is eliminated |
| Host `chain_next` dispatch | Reduced Compile dispatches from about 25M to 3.7M but slowed Compile, Assignment, and Python | A measured host transition becomes cheaper than the existing Wasm loop |
| Direct-tail-call batch modules | Only about 12% of exits stayed in batch; most executions paid guards without avoiding dispatch | A formation policy demonstrates much higher retained-exit coverage before code generation |
| Page-co-located batches | Lost at tested caps; instance switching was not the dominant cost | New profiling identifies instance switching as dominant |
| Aggressive superblock build spacing | 4M/1M spacing lost to rebuild churn; 16M was best among tested values | Build cost or invalidation behavior changes materially |
| Rotated-nest loop regions | Net-negative across tested kernels | A specific missed code shape has a correctness test and a materially different formation algorithm |
| Trace unrolling for Assignment | Displaced useful region coverage and lost overall | Code-cache pressure is removed or unrolling is proven selective |
| `KEEPMIN=24/48` | Within noise or worse; `0` was row-optimal | Region-claiming policy changes |
| Definedness tracking | Neutral; it did not address the indirect-edge limit | It enables a new cross-block optimization rather than serving as a standalone dispatch tweak |
| Next-executing-tail formation | Neutral; observed successors still crossed indirect edges that batches could not span | It composes with a mechanism that follows indirect targets without a host exit |
| More page functions/build-count increases | Extra V8 compilation cost exceeded dispatch savings | Per-function compilation becomes substantially cheaper |
| Node 22/V8 12 for the old module shape | Tiering many small modules caused severe regressions | A new engine/version is measured with the current module shape |
| Relaxed SIMD experimental mode | Enabling the flag taxed total execution more than it helped | Relaxed SIMD becomes stable/default or is isolated to generated code |
| Removing TLB-miss bailouts alone | Eliminating the observed misses did not move Compile wall time | New counters show TLB handling has become a dominant fraction |
| Global batch/IC composition | Assignment gained 9–21%, but Numeric lost about 25%; best Assignment draw still missed the old pass bar | A general, workload-independent gate predicts benefit without row-specific tuning |

## Invalidated and superseded conclusions

- The reported HUFFMAN “superblock miscompile” was a host-load artifact. The
  same known-good build varied by roughly 3× while a concurrent toolchain build
  drove load above core count.
- Fourier was not permanently limited by exact software FMA. Quiet-host runs
  later matched or beat v86 after coverage improvements.
- Single-sample claims for Numeric, Assignment, HUFFMAN, IDEA, FP Emulation,
  and Python are not evidence. Identical code has produced double-digit to
  multi-fold swings.
- The early estimate of a 1.2µs indirect tail-call hop was obsolete on the
  later Node/V8 version. Chaining still failed because of registration
  behavior, not hop latency.
- A historical 11/13 result does not establish an 11/13 baseline. Adjacent
  replicated runs of the same candidate returned 10/13.
- Reducing dispatch count is not sufficient evidence of a wall-time
  improvement. Inline caches reduced Compile dispatches substantially while
  Compile wall time remained effectively tied.

## Plan of attack

### P0 — Completed: trace-level configuration experiment

**Hypothesis:** trace compilation caused the persistent Numeric regression;
`TRACELVL=0`, `1`, or `2` may recover Numeric without surrendering Fourier or
Assignment.

**Protocol:** serial three-pair A/B for
`numeric,fourier,assignment`, comparing each level directly with level 3 on
the frozen control Wasm.

**Decision:** retain a level only with a valid Numeric improvement of at least
10% and no selected-row regression of at least 10%. Configuration results do
not justify workload-specific heuristics.

**Level 0 result:** rejected. Against level 3, Numeric improved 31.2%
(`518.26`→`679.77`) while Fourier regressed 35.0%
(`24867`→`16175`); Assignment changed −5.2%, a tie. MAD was at most 1.4% and
host-probe spread was 1.22×, so the row trade is real rather than measurement
noise. Continue with levels 1 and 2 to locate which trace capability creates
the conflict.

**Level 1 result:** rejected. Numeric (−2.5%) and Assignment (−0.4%) tied,
while Fourier regressed 10.9%. MAD was at most 1.7% and host spread was 1.23×.
Branch traces lose Numeric's level-0 benefit but do not provide all of level
3's Fourier benefit.

**Level 2 result:** tie. Numeric changed −1.3%, Fourier −8.6%, and Assignment
−1.8%. MAD was at most 3.4% and host spread was 1.22×. No lower global trace
level beats level 3. Numeric's gain exists only when branch traces are disabled
entirely; level 3's return/inline-cache following recovers Fourier relative to
levels 0–2.

### P1 — Completed: explain and recover Numeric

Current aggregate evidence shows roughly 1.7B dispatches at only about 18.7
retired instructions/dispatch. Compare trace levels using exact aggregate
counters and coverage fingerprints. Determine whether the loss comes from
shorter executed traces, coverage displacement, rebuild churn, or an
instruction-family fallback.

The output of this step must be a falsifiable code-change hypothesis, not
“dispatch count is high.”

**E002 hypothesis recorded before editing:** level 1 is sufficient to lose the
level-0 Numeric improvement, but the cumulative `TRACELVL` knob also changes
the translation window and disables the higher-level call/return/inline-cache
features. Add an independent conditional-branch continuation gate and compare
`TRACELVL=3,BRTRACE=0` with normal level 3. This keeps the wide loop-detector
window and all indirect-edge machinery. The candidate advances only if
Numeric improves by at least 10% without a 10% Fourier or Assignment
regression; otherwise it is a diagnostic result that narrows the next change.

**E002 result:** rejected after one complete serial pair. Numeric regressed
42.1% (`501.99`→`290.81`), Fourier regressed 28.5%
(`25180`→`18015`), and Assignment regressed 9.7%
(`20.15`→`18.19`). Host-probe spread was 1.21× and all integrity checks
passed. Dispatches fell on every row, but instructions per dispatch also fell
sharply on Numeric (`18.68`→`8.21`). Higher-level call/return/IC following
without conditional-branch continuation produces less useful traces, so the
temporary branch gate was removed rather than replicated.

**E003 hypothesis recorded before editing:** the level-0 comparison also
switches from the 64-page translation window to one page. Add a diagnostic
window override and complete the two missing factorial comparisons:
level-0 narrow versus level-0 wide, then level-3 wide versus level-3 narrow.
Use one serial pair of Numeric and Fourier for localization; only a
configuration that improves a row by at least 10% without regressing the other
earns three-pair replication.

**E003 screen result:** the translation window is causal. At level 0, forcing
the wide window regressed Numeric 57.1% (`678.67`→`290.94`) while Fourier tied
at +1.3%. At level 3, forcing the narrow window improved Numeric 62.6%
(`502.42`→`816.70`) while Fourier slowed from `24515` to `22124` (0.902×,
just across the harness's regression boundary). Both reports were valid with
host spread at most 1.23×. Because the Numeric effect is large and Fourier is
close to the decision boundary, replicate the narrow level-3 candidate for
three pairs and add Assignment before accepting or rejecting it.

**E003 focused result:** the frozen-control three-pair replication passed.
Numeric improved 60.1% (`504.67`→`807.93`); Fourier changed −7.3% and
Assignment −3.4%, both ties. MAD was at most 3.0%, host spread was 1.21×, and
all integrity checks passed. Before the full scorecard, run a one-pair
Compile/Python guard because cross-page compiler call graphs were the original
reason for widening the window. A regression of at least 10% still rejects
the candidate.

**E003 promotion status:** the Compile/Python guard passed (Compile +13.7%,
Python −7.6% tie). The first authoritative 3×/3× scorecard was valid at 11/13:
Numeric became a win at `800.93` versus v86 `738.79`; Compile and Assignment
were the only losses. Report `scorecard-2026-07-28T05-38-26.json`, candidate
Wasm `c8a196b5…`, no nbench instability, host spread 1.23×. Repeat the exact
authoritative scorecard before promotion because HUFFMAN was a borderline
match and historical isolated 11/13 runs did not reproduce.

The first repeat produced the same 11/13 shape (Numeric `807.73`, HUFFMAN
match), but report `scorecard-2026-07-28T05-55-29.json` is **invalid**:
host-probe spread was 1.26× against the fixed 1.25× limit. Its apparent
reproduction does not count. Do not loosen or round the limit; rerun the exact
candidate on a quiet host.

The next unrestricted attempt, `scorecard-2026-07-28T06-11-53.json`, again
produced 11/13 but was also invalid at 1.26× host spread. Probe outliers
clustered around Compile despite an otherwise idle 24-thread heterogeneous
host. A 100-probe preflight measured 1.233× unrestricted spread versus 1.015×
when pinned to physical-core threads `0,2,4,6`. Record actual Linux CPU
affinity in scorecard provenance and repeat under that affinity; this controls
scheduler migration without changing the 1.25× validity threshold.

The affinity-controlled repeat, `scorecard-2026-07-28T06-29-49.json`, is
valid at 1.24× host spread and records CPU affinity `0,2,4,6`. It reproduced
11/13: Numeric `818.32` versus v86 `747.05`, HUFFMAN match, with Compile and
Assignment the only losses. The tested narrow window is now the default. The
final `b5a857b0…` build passed all eight `tests/run-all.sh` stages inside the
Nix environment: Cargo, guest builds, three QEMU differentials, 134 ISA tests,
109 lockstep tests, 193 architecture signatures, Wasm/differential checks,
and modern virt boot.

The final canonical run used the project-pinned Rust 1.97.1/Node 20
environment, exact Wasm `b5a857b0…`, and recorded affinity `0,2,4,6`.
`scorecard-2026-07-28T07-03-26.json` is valid at 1.24× host spread and
reproduced 11/13: Numeric `844.96` versus v86 `730.07`; Compile and Assignment
were again the only losses. One of three rv64 nbench repetitions reported
internal instability, below the authoritative invalidation threshold. This is
the promotion report.

### P2 — Next: prototype cross-block guest-state caching

If configuration cannot recover the open rows, the next distinct mechanism is
emitted-code quality:

1. Select a small set of hot guest GPRs from a Compile profile.
2. Keep them in Wasm locals across direct and inline-cache-followed trace
   edges inside one generated function.
3. Spill at side exits, calls that require architectural memory state, traps,
   invalidation guards, and externally observable boundaries.
4. Pin aliasing, x0, fault, retirement, and interrupt behavior with
   differential tests.
5. Gate first on Compile plus `python,numeric,assignment`; generalize only
   after a valid improvement of at least 10%.

This is deliberately narrower than implementing a complete allocator. It
tests whether cross-block spills are actually causal before committing to a
multi-session backend rewrite.

Start a fresh experiment series before editing: commit E003, build its default
Wasm, and snapshot it as the immutable E004 control. The first E004 gate is a
focused serial A/B on `compile,python,numeric,assignment,fourier`; Compile must
improve by at least 10% with no selected-row regression. Only then run another
authoritative scorecard.

### P3 — Region live ranges/register allocation

Proceed only if P2 demonstrates material benefit. Add live-in/live-out
analysis, local allocation, and precise side-exit reconstruction across a
whole trace/region. Compile and Assignment are the motivating rows.

### P4 — Python coverage stability

Work on Python only if replicated candidate-relative trials still show a loss.
Use dispatch coverage fingerprints to distinguish address-layout collisions
from function-landing timing. Do not optimize a single fast or slow boot.

## Experiment ledger

Statuses are `LANDED`, `IMPLEMENTED`, `REJECTED`, `INVALID`, `DIAGNOSTIC`,
`TIE`, or `OPEN`. Historical entries summarize the old narrative; new work
must add one row immediately after its decision.

| ID | Date | Status | Candidate/mechanism | Protocol/evidence | Outcome and decision |
|---|---|---|---|---|---|
| H001 | 2026-07-24/25 | LANDED | Correctness and benchmark review fixes | Commits listed in “Resolved correctness and measurement work”; full differential suite | Closed every original correctness/measurement issue before accepting performance claims |
| H002 | 2026-07-25 | LANDED | Superblock landing, multi-page regions, productive rebuilds, ISA/FP coverage | Corrected scorecards plus differential tests | Moved the scorecard from 8/13 toward 10/13; mechanisms remain in baseline |
| H003 | 2026-07-25 | INVALID | Sparse-region HUFFMAN/Fourier investigation | Host load reached 29 on 24 cores; known-good build changed about 3× | All late-session miscompile/FMA conclusions discarded |
| H004 | 2026-07-26 | LANDED | Trace compilation and whole-region discovery (`e950060`, `1ebecdc`) | Quiet-host scorecards and row profiles | Compile improved at `SB=0`; coverage regression from per-page discovery fixed |
| H005 | 2026-07-26/27 | REJECTED | Three chaining designs | Interleaved multi-round Compile/Assignment/Python comparisons | Shared-table variants were quadratic; host dispatch reduced count but increased wall time |
| H006 | 2026-07-26/27 | REJECTED | Direct batch modules, page co-location, build-spacing sweeps | Interleaved focused comparisons | Avoided too few exits or added rebuild/guard cost; no reproducible wall-time win |
| H007 | 2026-07-26/27 | REJECTED | Rotated nests, `KEEPMIN`, definedness, next-tail formation | Focused multi-round comparisons | Neutral or negative; did not cross indirect control-flow boundaries |
| H008 | 2026-07-27 | LANDED | Monomorphic indirect-target inline caches (`c155cbd`) | Compile dispatch 25.5M→17.4M; Fourier/String/Python improvements | Retained as a genuine structural improvement; Compile wall time still unresolved |
| H009 | 2026-07-27 | REJECTED | Batch/IC composition (`1d1b107`) | Assignment +9–21%; Numeric about −25% | Default disabled in `3855c72`; row trade is not a general improvement |
| H010 | 2026-07-27 | TIE | Independent candidate comparison, Wasm `21b638…` | Three replicated 13-row runs without recorded problems | 10/13 each; frozen as current control |
| H011 | 2026-07-27 | TIE | Independent candidate, Wasm `96b410…` | Same-Wasm replicated runs: 10/13, 10/13, 11/13 | The 11/13 observation did not repeat; exact transplanted commit was not recorded, so the hash is the only trustworthy identity |
| H012 | 2026-07-27 | TIE | Independent candidate, Wasm `842164…` | Replicated 13-row runs | Remained 10/13; exact transplanted commit was not recorded |
| N001 | 2026-07-27/28 | IMPLEMENTED | Isolated benchmark methodology and profiler instrumentation | Complete 13-row plumbing runs, harness self-test, all eight correctness stages | Present in the working tree; instrumentation is not a performance candidate |
| N002 | 2026-07-27 | TIE | Frozen HEAD vs instrumented build on Compile | Three-pair serial fresh-process A/B, report `ab-2026-07-27T23-00-09.json` | 0.999×; profiler’s disabled path is timing-neutral |
| N003 | 2026-07-28 | DIAGNOSTIC | Frozen HEAD vs instrumented build on Numeric | One-pair integration A/B, report `ab-2026-07-28T00-12-48.json` | 0.999× with matching inputs and no nbench instability; integration proof, not promotion evidence |
| E001a | 2026-07-28 | REJECTED | `TRACELVL=0` versus `3` | Three-pair `numeric,fourier,assignment`; report `ab-2026-07-28T04-02-37.json`; host spread 1.22× | Numeric +31.2%, Fourier −35.0%, Assignment −5.2% tie; global level 0 is a row trade, not an enhancement |
| E001b | 2026-07-28 | REJECTED | `TRACELVL=1` versus `3` | Three-pair `numeric,fourier,assignment`; report `ab-2026-07-28T04-23-01.json`; host spread 1.23× | Numeric −2.5% tie, Assignment −0.4% tie, Fourier −10.9% regression; branch traces do not improve the trade |
| E001c | 2026-07-28 | TIE | `TRACELVL=2` versus `3` | Three-pair `numeric,fourier,assignment`; report `ab-2026-07-28T04-43-26.json`; host spread 1.22× | Numeric −1.3%, Fourier −8.6%, Assignment −1.8%; no global improvement, and the trace-level sweep is closed |
| E002 | 2026-07-28 | REJECTED | Independent conditional-branch trace gate (`TRACELVL=3,BRTRACE=0`) | One-pair `numeric,fourier,assignment`; report `ab-2026-07-28T04-54-44.json`; candidate Wasm `553a820b…`; host spread 1.21× | Numeric −42.1%, Fourier −28.5%, Assignment −9.7%; fewer dispatches but much less work per dispatch, so the temporary gate was removed |
| E003 | 2026-07-28 | LANDED | Narrow translation window with full traces (`TRACEWIN=1`, now default) | Factorial screens `ab-2026-07-28T04-58-37.json`/`ab-2026-07-28T05-01-52.json`; focused `ab-2026-07-28T05-20-58.json`; guard `ab-2026-07-28T05-22-07.json`; valid host-toolchain scorecards `scorecard-2026-07-28T05-38-26.json`/`06-29-49`; two intervening 1.26×-drift reports invalid; valid canonical Nix scorecard `scorecard-2026-07-28T07-03-26.json`; all eight correctness stages passed on final `b5a857b0…` build | Numeric +60.1% against frozen control and a reproducible 11/13 scorecard across host and pinned toolchains; Compile/Assignment remain the only losses |
| E004 | — | OPEN | Cross-block guest-state caching prototype | First snapshot E003 as the immutable control; profile Compile; focused three-pair `compile,python,numeric,assignment,fourier` gate | Test a small hot-GPR local cache before considering whole-region register allocation |

### Required record for new experiments

Add the ledger row, then record these details beneath it when more context is
needed:

```text
ID:
Date:
Status:
Hypothesis:
Control Wasm/config:
Candidate Wasm/config:
Motivating rows:
Guard rows:
Protocol and repetitions:
Raw report paths:
Host-probe spread:
Correctness fingerprints:
Median/MAD and aggregate-counter changes:
Decision:
Reason:
Reconsider only if:
```

Never delete a negative or invalid entry. Mark it superseded and link the new
evidence instead.
