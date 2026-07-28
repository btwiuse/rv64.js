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

- **Best known engineering baseline:** `314e441` (E003 narrow-window default).
- **Frozen performance control:** Wasm SHA-256
  `b5a857b087bd49631866c9bcd77e3d9d98df1dfb93eedf3ce16782c3e02d4433`.
- **Control artifact:** `target/bench/wasm-candidates/`
  `e003-control-b5a857b087bd.wasm`, snapshotted from clean commit `314e441`
  with Nix Node 20.
- **Prior E003-series control:** Wasm `21b638123cee…`, artifact
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
- **Extended E004–E008 outcome:** no runtime change was promoted. Page-cache
  and tier-threshold sweeps were neutral or negative; multi-latch loop
  compilation produced a large but invalid Assignment result that remains
  quarantined pending a deterministic explanation of its internal variance.

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
| Per-trace page-cache threshold sweep | Threshold 2 regressed Compile 6.6%; disabling the cache improved it only 4.5%, below the fixed gate | A changed memory-access shape or new counters show the cache is materially dominant |
| Ordinary JIT tier threshold sweep | Thresholds 128, 32, and 16 changed Compile by −2.3%, +2.6%, and −5.5% respectively | Module generation or interpreter/JIT crossover costs change materially |
| Individually selected multi-latch scan loops | Large-stride-only was an invalid +4.1%; small-stride-only was a valid −3.0% | A new structural selector identifies a different independently beneficial loop |
| Broad multi-latch scan-loop compilation | Apparent Assignment gain was 69.7%, but all three candidate trials failed nbench's internal confidence test | The variance is explained and the unchanged canonical benchmark produces a valid result; never relax the confidence rule |

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

### P2 — Next session: make the Assignment opportunity testable

The proposed cross-block GPR-cache prototype was based on an incomplete audit:
ordinary traces already allocate locals for every touched GPR/FP register,
keep them live across conditional, direct-call/return, and inline-cache-followed
edges, load only read registers, and spill only writes visible at each exit.
Sparse superblocks likewise retain selected locals across their internal
control-flow graph. Reimplementing that mechanism would not be an experiment.

A frozen-control Compile profile
(`ab-2026-07-28T14-28-24.json`) retired 322.7M JIT instructions across 16.6M
dispatches (19.42 instructions/dispatch), while only 3,109 entries used
superblocks. Historical `c155cbd` evidence is more decisive: inline caches
cut Compile dispatches by roughly one third without improving its wall time.
Compile is therefore an emitted-code-quality problem, not currently a
dispatcher or superblock-entry problem.

E003 is committed at `314e441`; its clean Nix build is frozen as
`e003-control-b5a857b087bd.wasm`. E008 found a concrete Assignment mechanism,
but the broad form is not a candidate: its +69.7% median failed nbench's
internal confidence check in every candidate trial, while neither structural
half helped alone.

Use this sequence for the next session:

1. Add a diagnostic-only fixed-work, checksum-bearing reproducer for the two
   `DoAssignIteration` scan-loop shapes. It may validate semantics and expose
   phase changes, but it cannot promote a candidate or replace nbench.
2. Capture nbench's internal sample count, mean, standard deviation, and
   confidence decision, plus multi-latch compile/entry counts. Keep the
   canonical workload, Node flags, timing boundaries, and invalidation rules
   unchanged.
3. Diagnose whether the variance follows Wasm tiering with separate
   non-canonical engine-flag runs. Such runs explain behavior only; they are
   never score evidence.
4. Recreate the exact broad E008 mechanism only after the variance has a
   measured cause. Screen one serial fresh-process pair. Continue to three
   alternating pairs only if the report is valid and the effect is at least
   10%.
5. If that gate passes, run guard rows
   `compile,python,numeric,fourier`, then the authoritative 13-row scorecard
   and all eight correctness stages. Any internal-instability majority,
   checksum mismatch, host spread over 1.25×, or selected-row regression of
   at least 10% rejects the candidate.

This keeps the quick cycle at one focused pair until evidence justifies the
more expensive stages and prevents a diagnostic benchmark from becoming a
new scoring methodology.

### P3 — Compile emitted-code investigation

The page-cache and tier-threshold axes are closed. Before another Compile
implementation, add immutable-control counters that divide generated memory
operations among page-local cache hits, full TLB hits/fills, crossings, and
bailouts, and record generated function/instruction size. Select one bounded
lowering only if a measured category is dominant. A focused serial Compile
pair remains the first gate; `python,numeric,assignment,fourier` are guards
only after a valid improvement of at least 10%.

True region live ranges/register allocation remains a possible larger backend
project, but it is not the same as adding a hot-register local cache. Start it
only if those counters show that region entry/exit register traffic is
material; current evidence does not.

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
| E004 | 2026-07-28 | DIAGNOSTIC | Audit proposed cross-block guest-state caching; profile immutable E003 control against itself | Source audit plus deterministic Compile profiles in `ab-2026-07-28T14-28-24.json`; 322.7M JIT instructions, 16.6M dispatches, 3,109 superblock entries | Superseded before implementation: trace and sparse-region GPR/FP local caching already implement the proposed mechanism; historical `c155cbd` also proved Compile wall time insensitive to a one-third dispatch reduction |
| E005a | 2026-07-28 | REJECTED | Lower the per-trace TLB page-cache activation threshold from 3 memory operations to 2 behind a diagnostic knob | One serial profiled Compile pair, report `ab-2026-07-28T14-31-45.json`; affinity `0,2,4,6`, host spread 1.02×, identical 16.62M dispatches and 19.42 instructions/dispatch | Compile regressed 6.6% (`2414.15`→`2584.19` ms); the extra cache guards/locals cost more than the avoided probes, so do not replicate or retain threshold 2 |
| E005b | 2026-07-28 | TIE | Disable the per-trace TLB page cache (`MEMCACHEMIN=0`) | One serial profiled Compile pair, report `ab-2026-07-28T14-32-25.json`; affinity `0,2,4,6`, host spread 1.05×, identical aggregate execution counters | Compile improved only 4.5% (`2444.81`→`2339.52` ms), below the fixed 10% threshold; close the page-cache sweep and remove the diagnostic knob |
| E006a | 2026-07-28 | TIE | Raise the ordinary JIT tier-up threshold from 64 to 128 behind a diagnostic knob | One serial profiled Compile pair, report `ab-2026-07-28T14-34-24.json`; affinity `0,2,4,6`, host spread 1.01× | Compile regressed 2.3%; 828 fewer modules did not pay for moving 4.5M instructions from JIT to interpreter |
| E006b | 2026-07-28 | TIE | Lower the ordinary JIT tier-up threshold from 64 to 32 | One serial profiled Compile pair, report `ab-2026-07-28T14-35-02.json`; affinity `0,2,4,6`, host spread 1.02× | Compile improved 2.6%; 321 extra modules recovered 3.6M interpreted instructions but remained well below the 10% bar |
| E006c | 2026-07-28 | REJECTED | Lower the ordinary JIT tier-up threshold from 64 to the historical value 16 | One serial profiled Compile pair, report `ab-2026-07-28T14-35-38.json`; affinity `0,2,4,6`, host spread 1.02× | Compile regressed 5.5%; threshold 32 was the best non-default sample at only +2.6%, so close the tier axis and remove the diagnostic knob |
| E007 | 2026-07-28 | DIAGNOSTIC | Profile immutable-control Assignment and select a structural candidate from its measured execution shape | The first integration report `ab-2026-07-28T14-36-23.json` is invalid and excluded because slow profiling was not explicitly enabled; slow identical-artifact diagnostic report `ab-2026-07-28T14-40-57.json` used the required opt-in, affinity `0,2,4,6`, and host spread 1.02×; timing/counter differences are not a performance claim because nbench is self-timed | The stable top sites are `0x100934e` and `0x10092c6` in `DoAssignIteration`, both about 7 instructions/dispatch and together dominant; disassembly shows multi-latch scan loops with multiple conditional continues plus a final unconditional backedge |
| E008a | 2026-07-28 | INVALID | Compile all precise multi-latch scan loops as structured Wasm loops | Exact-shape unit and feature-enabled full-state/retirement differential pass; invalid one-pair reports `ab-2026-07-28T14-51-45.json` and `ab-2026-07-28T14-56-31.json`; alternating three-pair report `ab-2026-07-28T15-09-00.json` | Three-pair medians showed +69.7% (`14.60`→`24.77`), external MAD 0.1%/1.0%, host spread 1.03×, but candidate nbench confidence failed in 3/3 trials; no code or diagnostic knob was retained, and this form cannot be reconsidered until the internal variance is explained |
| E008b | 2026-07-28 | REJECTED | Restrict multi-latch compilation to large-stride scans | One-pair report `ab-2026-07-28T15-16-28.json`; numerically +4.1%, and invalid because both sides reported nbench instability | The large-stride `0x100934e` loop does not explain E008a's gain; do not replicate |
| E008c | 2026-07-28 | TIE | Restrict multi-latch compilation to small-stride scans | Valid one-pair Assignment report `ab-2026-07-28T15-20-54.json`; affinity `0,2,4,6`, host spread 1.01×, no internal nbench warning | Assignment regressed 3.0% (`14.48`→`14.05`), below the 10% threshold; together E008b/E008c show that neither loop shape independently explains E008a, so close the factorial and retain no code |

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
