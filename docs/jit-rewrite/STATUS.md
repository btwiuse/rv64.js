# JIT Rewrite Status

Last updated: 2026-08-13

## Current disposition

The RV64GCV direct JIT-lowering milestone is **achieved**. The complete
mandatory RVV 1.0 surface for the selected VLEN=128/ELEN=64 machine remains an
ordered DBT IR effect with typed user/system helper fallback, while broad
exact instruction families now lower directly to Wasm SIMD or generated Wasm
loops. Precise scalar/FP barriers, fault replay, `vstart`, privileged dirty
state, and compiled-code invalidation are preserved. The strict release matrix
passes, including all 8,724 interpreter and all 8,724 hot-JIT RVV differential
executions.

The authoritative RV64GCV JIT scorecard is measurement-valid with 78/78
eligible trials and an empty problem list. It wins eight of thirteen rows
against pinned copy/v86. Against helper-only JIT lowering it improves String
Sort by `3.4587x`, FP Emulation by `1.7970x`, and Assignment by `1.2915x`.
The separately rerun modern scalar scorecard remains 11/13, with only Boot and
Compile losing. The historical all-row RV64GCV/copy-v86 parity objective
remains **not achieved** because Boot, Compile, String Sort, FP Emulation, and
Assignment still lose.

The complete implementation boundary, no-workload-identifier audit,
correctness matrix, scorecards, hashes, and remaining performance gap are in
[RVV_JIT_RESULT.md](RVV_JIT_RESULT.md). The preceding pure-interpreter
milestone remains recorded in
[RVV_INTERPRETER_FINAL_REPORT.md](RVV_INTERPRETER_FINAL_REPORT.md).

## Historical scalar-baseline disposition (superseded)

The pure-interpreter parity goal is **not achieved** and is now blocked under
the fixed comparison constraints. The previously reported
12-win/1-match result is withdrawn because the rewrite was tuned after seeing
the scorecard losses with exact recognizers for the measured BYTEmark and musl
instruction sequences. Those paths preserved guest semantics, but their
post-hoc workload specificity makes the result ineligible as evidence of
general interpreter parity with copy/v86.

The benchmark-derived direct-interpreter recognizers, helpers, counters, and
tests have now been removed from production source. The specialization-free
baseline release build was
`a2f42e55070478dd162ded55e58a7d4be2b050d859da7a0d7b48a94352336095`;
formatting, 109 Rust library tests, the scorecard self-test, the explicit
JIT-disabled bypass proof, and the full-state and system-memory differential
matrix passed. Its fresh all-row JIT-off development scorecard was measurement
valid and recorded eleven wins plus two losses: String Sort at `0.2400x`
(`4.17x` behind) and Bitfield at `0.8178x` (`1.22x` behind). It used one
fresh-process repetition, so it is exploratory rather than authoritative; all
26 workers proved JIT inactivity.

Eight prospectively frozen, architecture-general candidates have since been
evaluated. I001, I003, and I004 were retained; I002 and I005-I008 were rejected
without post-result variants and fully removed. The live release Wasm is exact
I004,
`7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`.
Two frozen multi-row attribution diagnostics then admitted no further source
edit: I009 resolved only the monolithic integrated body, and I010's native
samples mapped its qualifying bands to already retained or rejected
fetch/cache/driver mechanisms.

The complete three-repetition I004 development scorecard is now authoritative
and measurement-valid. All 78 fresh-process workers prove JIT inactivity; its
problem list is empty, maximum host-probe spread is `1.0146`, and maximum
within-side sample spread is `1.0962`. I004 wins eleven rows, matches Mixed at
`1.0301x`, and now wins Bitfield at `1.1229x`, but String remains a decisive
loss at `0.3186x` (`3.14x` behind). The development gate therefore fails and
the goal remains open. The exact result is recorded in
[INTERPRETER_I004_AUTHORITATIVE_DEVELOPMENT_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_DEVELOPMENT_RESULT.md).
The source and mechanism closure audit froze development tuning closed at I004
before either sealed population was opened. The strict correctness matrix then
passed, and exact I004 completed the first and only authoritative
`stock-musl-v1` execution. All 78 trials are eligible and JIT-inactive with no
report problems. I004 again wins eleven rows and matches Mixed, but loses the
fair stock String row at `0.3076x` (`3.25x` behind). The goal remains open; the
result cannot authorize another edit. Its complete record is
[INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md).
The first and only sealed holdout execution is also complete. All 24 trials are
eligible, JIT-inactive, checksum-correct, and measurement-valid; I004 wins all
four unseen rows by `1.7171x` to `2.5409x`. This is strong general-transfer
evidence, but the holdout-local pass cannot override the failed development and
stock gates. Its complete record is
[INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md).
The final audit followed
[INTERPRETER_STRING_PLATEAU_AND_UNSEAL_PROTOCOL.md](INTERPRETER_STRING_PLATEAU_AND_UNSEAL_PROTOCOL.md).
The JIT-enabled scorecard rerun is paused.

A subsequent clean generation tested one genuinely architecture-defined
mechanism without consulting the opened scorecards: a `FENCE.I`-coherent
decoded interpreter. Its complete design, 19-program Embench transfer suite,
balanced standalone stream, model, schedule, and thresholds were frozen before
timing. The only allowed opportunity run passed every identity, state, cache,
affinity, stability, and compile-cost check, but improved its isolated work by
only `1.6620x [1.6443, 1.6808]`, far below the preregistered `3.75x`/`3.50x`
leverage gates. G001 was rejected before any production edit and has no cache
variant successor. The Embench guests remain unexecuted, exact I004 remains
live, and the goal is still not achieved. See
[INTERPRETER_G001_FENCEI_DECODE_PROTOCOL.md](INTERPRETER_G001_FENCEI_DECODE_PROTOCOL.md).

The final distinct non-cache representation audit then froze G002: retain all
`x1`--`x31` values in Wasm locals and resolve every dynamic register operand
through a complete 32-way selector. Its workload-free 1,024-record schedule is
exactly balanced across `rd`, `rs1`, and `rs2`, and the standalone model omits
fetch, decode, guest data memory, interrupts, and other common work to give the
mechanism a favorable upper bound. Untimed deterministic/shape and exhaustive
single-record/full-state preparation passed. The only allowed seven-pair
timing passed every integrity and stability check but measured
`0.0981x [0.0978, 0.0985]`: complete local residency was about `10.19x` slower because
three dynamic local selectors cost far more than two direct loads and one
guarded store. G002 was rejected before production or guest execution, exact
I004 remains live, and the goal remains not achieved. See
[INTERPRETER_G002_LOCAL_GPR_PROTOCOL.md](INTERPRETER_G002_LOCAL_GPR_PROTOCOL.md).
The resulting evidence boundary and the comparison constraints that would
have to change before another credible attempt are recorded in
[INTERPRETER_POST_G002_FEASIBILITY_BOUNDARY.md](INTERPRETER_POST_G002_FEASIBILITY_BOUNDARY.md).

A final workload-blind build audit then tested the previously unscreened
standard LLVM source optimization levels. G003 used only the frozen balanced
direct-interpreter model: O1/O2/O3 compiled to identical bytes, while Os/Oz
compiled to one identical smaller artifact. The complete preregistered
56-process screen retained all samples. O1/O2 were causally neutral at about
`1.00x`; the smaller artifact was slower (`Os 0.9708x`, `Oz 0.9671x`). No
level approached the `3.25x` leverage gate, so no Cargo production build or
guest run followed. Exact I004 and the unmet goal remain unchanged. See
[INTERPRETER_G003_LLVM_OPT_LEVEL_PROTOCOL.md](INTERPRETER_G003_LLVM_OPT_LEVEL_PROTOCOL.md).

The final feasibility audit also reconciled the earlier hand-emitted static
Tier-0 line rather than silently omitting it. R070's complete guest-independent
RV64I/M, integer-RVC, and scalar-memory executor achieved `1.624x`--`2.445x`
on isolated corpora, but only `1.047x` full-system Boot; R071 independently
measured `1.024x`. Later sampled variants produced larger native points but
failed browser or product promotion and were removed. R095 then regressed
Boot to `0.944x` from a separate Wasm instance. R115 proved that instance
switching cost a real `1.03413x`, yet the same embedded executor still measured
`0.97987x [0.95574, 0.99572]` enabled versus disabled. Those historical
full-system runs retained the ordinary JIT and therefore are supporting
mechanism evidence, not JIT-off scorecard results. They close the strongest
known architecture-general hand-emitted alternative without changing exact
I004. The reconciled ledger is in
[INTERPRETER_POST_G002_FEASIBILITY_BOUNDARY.md](INTERPRETER_POST_G002_FEASIBILITY_BOUNDARY.md).

The final requirement audit confirms 78/78 eligible, JIT-inactive trials in
each authoritative 13-row population and 24/24 in the sealed holdout. Removal,
record correction, pinned-comparator validation, fair-input execution, and
holdout execution are complete; parity itself is contradicted by String at
`0.3186x` development and `0.3076x` stock. No open candidate in the complete
mechanism ledger has credible leverage for the remaining `3.139x` whole-row
gap. The goal is therefore blocked, not completed. Reopening it requires a
new independently qualified portable primitive or an explicit change to the
JIT, guest-ISA/build, or fairness constraints listed in the feasibility
boundary.

The input audit also found a pre-existing asymmetry in the scored BYTEmark
binaries: only the RV64 binary links `tests/vs-v86/nbench-extras/fastmem.c`, a
hand-written replacement whose own comment names String Sort as its target;
the i386 binary uses musl. The current scorecard is therefore development
evidence, not sufficient final parity evidence. The final audit therefore also
used a frozen stock-libc cross-ISA population and the once-sealed holdouts.
Their preregistered rules and original input identities are recorded in
[INTERPRETER_ANTI_OVERFIT_PROTOCOL.md](INTERPRETER_ANTI_OVERFIT_PROTOCOL.md).
The pinned comparator's ordinary dispatch and complete x86 REP implementation
are audited separately in
[INTERPRETER_COMPARATOR_AUDIT.md](INTERPRETER_COMPARATOR_AUDIT.md). That audit
found no comparable benchmark recognizer: the remaining String gap is dominated
by cross-ISA instruction-count amplification, not a hidden copy/v86 workload
specialization.

See [INTERPRETER_EXERCISE_REPORT.md](INTERPRETER_EXERCISE_REPORT.md) for the
withdrawal record and exact identity of the disqualified tuned artifact.

[FINAL_EXERCISE_REPORT.md](FINAL_EXERCISE_REPORT.md) remains the terminal audit
of the earlier JIT-enabled exercise. R126 was measurement-invalid, R127 was
externally terminated, and R128 was stopped at the owner's request; none was
reused in the interpreter population. Historical sections below are retained
as the chronological record and must not be mistaken for current status.

## Milestone

Phases 0 through 6 of the clean-room RV64-to-WebAssembly rewrite are complete.
The final strict repository matrix passes on the current upstream tree. The
first post-delivery compile-policy milestone is also complete: the stable web
loader now selects a bounded async page policy instead of synchronous per-PC
module compilation. See [COMPILE_POLICY.md](COMPILE_POLICY.md).

The removed JIT has not been used as a design input. The requested post-delivery
head-to-head comparison is now complete; it executed the previous build only as
a black box after the Phase 6 checkpoint.

## Historical parity goal

The clean-room implementation milestone and its measurement-closure outline
are complete, but the full performance goal is not. The authoritative modern
Linux 6.12.7 / Alpine 3.24.1 three-way v2 report is
`target/bench/r054-final-three-way-rerun/scorecard-v2-2026-08-08T23-01-30-777Z.json`.
It is valid with all 13 rows, three fresh-process repetitions per engine,
alternating side order, v86 generated-dispatch proof, exact artifact/workload
hashes, CPUs 8-15, and 1.070x host-probe spread. Clean rewrite artifact
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`
wins ten rows against copy/v86, matches String Sort, and loses only Boot
(2,260.5 versus 1,525.8 ms, 1.48x slower) and Compile (1,060.9 versus 718.5
ms, 1.48x slower). It meets or beats the isolated modern-Virt legacy
comparator on all 13 rows.

The current-artifact browser guard is also complete. Five preregistered fresh
Chrome 150/V8 15.0 pairs report rewrite/v86 paired geometric-mean ratios of
0.841 for Python, 0.619 for SHA-256, and 0.608 for shared 9P, with exact
paired-bootstrap 95% intervals `[0.778,0.885]`, `[0.608,0.631]`, and
`[0.499,0.742]`. The report is
`target/jit-policy-traces/wanix-r054-416033-chrome-20260808/analysis.json`.
No failed leg was replaced. An earlier v86 external-9P timeout remains an
unexplained invalid run, not evidence against the rewrite.

R054 is the latest promoted optimization. It lets scalar interpreter memory
operations consume the exact permission/context-tagged fused JIT-TLB pointer
capability already published by the normal path. The frozen engine-shape
corpus measured 3.030x local throughput. Five-pair same-Wasm testing measured
Boot at 1.161x `[1.113,1.181]`; exact final-artifact A/B measured 1.151x
`[1.130,1.168]`. In the untouched full scorecard, Boot fell 13.35% and Compile
fell 4.73% from the R043 accepted baseline, with no other row regressing 10%.
R021's direct RV64C interpretation remains the earlier promoted interpreter
change. R022-R053 closed the intervening threshold, layout, policy, generated
memory, and dispatch mechanisms through preregistered opportunity or timing
gates.

R045 completed the first structural attribution pass. Exact engine profiles
confirm that Boot is interpreter/runtime dominated (94.65% sampled runtime
Wasm, only 3.88% generated Wasm), while Compile is split between runtime and
generated Wasm despite 92.32% generated guest retirement. A physical-page
trace showed enough concentrated Boot work to admit a decoded baseline-page
tier, but three progressively cheaper implementations moved from a 9.4%
regression to a timing tie and never cleared the fixed 10% advancement gate.
The packed executor cost essentially the same self time as `Cpu::step`, so the
mechanism was rejected and completely removed. The rebuilt production module
is byte-for-byte the unchanged `d93345139c5a...` artifact; 30 core, 76 system,
and one Wasm unit test pass after restoration.

R046 then tested the remaining plausible cold dynamic-code shape rather than
retuning R045. An exact JIT-off snapshot trace showed that compact one-page,
64-leader modules had enough theoretical coverage and isolated compile latency
to pass the opportunity gate. Runtime evidence overturned that model: two
order-reversed same-Wasm screens reproduced a 45.7% Boot regression
(2.582 s to 3.763 s). The candidate expanded cold compilation from 10 modules
and about 0.42 s summed latency to 51-52 modules and 6.77-6.94 s, while only
moving another 13-14M instructions into generated code by readiness. Compile
STEADY tied and correctness markers passed. All candidate and trace-emission
code is removed; 160 relevant Rust tests and formatting pass, and the release
module is again byte-identical to `d93345139c5a...`. Smaller runtime-generated
modules do not solve cold Boot when their V8 work competes with the guest and
lands too late.

R047 then closed the precompiled exact-triple alternative at its transfer gate.
The top 256 normalized Boot triples removed 40.81% of exact non-overlapping
Boot dispatches, but only 31.20% on the exact Compile workload and 24.78% on
Python. Both held-out workloads passed their output fingerprints. Training a
new library from those failures is outside the architecture-general rule, so
no execution candidate was built. Every R047 runtime field/export is removed;
30 core and one Wasm unit test pass, and release Wasm again exactly equals
`d93345139c5a...`. R045-R047 close the currently evidenced Boot baseline-tier
shapes and supplied the precondition for R048's generated-engine attribution.

R048 completed that attribution and rejected its admitted implementation.
Rewrite generated functions total 33.03 MB with a 3.22 MiB maximum, versus
v86's 10.50 MB and 223 KiB maximum. A relocated 512-entry physical page emits
the 3.22 MiB function once per Compile phase, and its 1.46-1.54 s TurboFan job
finishes in the next phase or after result. Entry-boundary splitting reduced it
to about 79.8 KiB while preserving exact output and the existing bulk-copy
lowering. Nevertheless, a final geometry-only form cut STEADY emitted bytes
57-62% and tied Compile at 0.998x and Boot at 0.981x. The candidate and every
runtime/harness switch are removed; 53 DBT, 30 core, and one Wasm units pass,
and the release module is byte-for-byte `d93345139c5a...`. Only diagnostic
tier/capture support and the generic same-Wasm configuration A/B harness remain.

R049 then tested the strongest remaining generated-memory hypothesis. The
retained QEMU diagnostic proves that 99.9986% of dynamic `x2` writes are
affine-immediate and that all 82.47M observed stack-root memory operations use
the current stack page or its successor; the recent-two-page model misses only
9 events. A correctness-gated carried two-page load/store translation therefore
had broad measured opportunity, but a valid alternating same-Wasm A/B regressed
Compile STEADY from 1,052.18 to 1,484.94 ms (41.1%). It was rejected without
tuning. The rejected artifact is archived, every candidate DBT/runtime/harness
path is removed, 53 DBT, 30 core, and one Wasm units pass, and production Wasm
again exactly equals `d93345139c5a...`.

R050 has now resolved the coarse optimized-tier runtime attribution. In
STEADY, generated module subtrees consume 43.86% of all CPU samples, residual
interpreter subtrees 34.20%, scheduler self/cache hashing 11.86%, and
translation/issue 5.46%; PRIME reproduces the hierarchy. Exact counters give
2.45 sampled ns per generated guest instruction and 23.08 per interpreted one,
a 9.42x ratio. Thus the remaining 7.64% T0 retirement—not synchronous
translation—is the largest broad opportunity. A fresh proof-only exact-artifact
fallback report is retained under `target/bench/r050-runtime-attribution/`;
no production behavior changed.

R051 then partitioned the fallback work by exact PC and lifecycle state. Its
dominant attempted-not-installed site (3.828M instructions in 12,465 stretches)
reproduced R032's already known 3.807M/12,395 population. R033 had already
removed that work generally, and R034 had already rejected the result because
Compile regressed 3.5% and Boot 1.4%. No candidate was rebuilt. The transient
diagnostic was removed, and release Wasm remains byte-identical to
`d93345139c5a...`. This is an explicit correction against repeating closed
entry-policy work.

The component-to-closure audit is now complete in
[CLOSURE_AUDIT.md](CLOSURE_AUDIT.md). It admitted one bounded R052 structural
screen: keep independent load/store Sv39 rows but compress each exact proof and
memory32 offset into one `i64`, removing one table load. Correctness gates caught
an extra-bank Wasm allocation overflow and a false page-alignment assumption;
after both fixes, bare/alias and Sv39+MPRV differentials matched exactly. Seven
alternating paired fresh-process frozen-byte measurements then showed packed
generated memory at 0.561x the split uncached path and 0.885x the split cached
path. The idea was rejected before a scorecard or tuning. All R052 production,
DBT, runtime, and harness paths are removed. The rebuilt CODE section is
byte-identical to accepted `d93345139c5a...`; recompilation differs only in
non-executable Rust/LLVM name metadata.

R053 has now rejected the architecture-complete pair-dispatch continuation of
R047 before any production edit. Independent exact traces admitted the screen:
fixed-width pairs could remove 42.79-46.30% of all dispatches across Boot,
Compile, and Python. The frozen corpus then emitted every one of the 3,844
ordered handlers, traversed every pair, and matched exact state. Seven
alternating fresh Node/V8 pairs measured only 0.879x single-handler throughput
(0.878-0.882 interval), a 12.1% regression; cold construction added just 0.847
ms. The result closes function-per-pair dispatch without a popularity or size
sweep. At the R053 checkpoint, production remained the accepted
`d93345139c5a...` artifact.

R054 then admitted and promoted a genuinely new interpreter-memory mechanism.
Boot profiles attributed 16.647% of the complete row to scalar `Cpu::ld` and
`Cpu::st`; a frozen exact-memory corpus proved that reusing the live fused
JIT-TLB capability accelerates that local shape 3.030x, above the 2.51x
opportunity threshold. Exact all-width, unaligned, privilege-tag, Sv39/MPRV,
page-crossing, MMIO, and generated-code invalidation gates pass. The valid
untouched three-way rerun improves Boot from 2,608.9 to 2,260.5 ms and Compile
from 1,113.6 to 1,060.9 ms without a guarded regression, establishing
`4160333352b18b...` as the accepted production artifact.

R055 completed that fresh post-R054 attribution. Boot is still 93.59% runtime
Wasm, with complete interpreter subtrees at 78.88% and `Cpu::step` self time at
49.89%. Compile's generated-module subtree is only modestly larger in absolute
sample time than v86's; its remaining gap is mainly runtime/interpreter and
scheduler work. A frozen architecture-wide direct instruction-fetch capability
then measured 1.989x local throughput and admitted one prototype. The complete
prototype passed focused correctness and both modern Linux paths, but the
measurement-valid five-pair same-Wasm screen rejected it: Boot was 0.962x
`[0.935,1.152]` and Compile 1.008x `[0.982,1.053]`. It was removed without a
policy or workload sweep. Production again matches accepted R054 SHA
`4160333352b18b...` byte-for-byte, and 32 core tests pass after restoration.

R056 next tested the only distinct residual in the 13.101% `run_until` self
category: preserve every exact post-instruction dispatch probe but monomorphize
its visible Wasm `call_indirect`. A deterministic externally mutable corpus
measured a tight 1.494x local speedup `[1.492,1.500]`. The measured category
requires 3.27x local speed to produce a 10% whole-Boot gain, so the optimistic
projection is only 1.045x. R056 was rejected at its frozen gate before any
production edit; the accepted artifact is unchanged.

R057 then tested whether the accepted runtime-Wasm compiler work could be
isolated in a dedicated two-Worker pool. A scorecard diagnostic defect was
fixed first: Boot now actually installs the requested module-capture hook and
marks capture runs proof-only. Exact R054 capture froze ten timed Boot modules
(3.97 MiB) and fifteen Compile STEADY modules (5.75 MiB). Seven alternating
fresh-process pairs returned every exact module and matching cloned
descriptors. Worker compilation did not accelerate foreground Boot
(call/wall 0.998x/1.002x), and the Boot stream became ready only 0.489x as
fast. The larger Compile stream became ready 1.073x faster but foreground
execution still tied. R057 was rejected without a production service or
module-size selector; accepted Wasm remains exact R054.

R058 then bounded the `Cpu::step` result ABI, a genuinely broad T0 mechanism.
The accepted non-inlined call writes a 24-byte `Result<Option<...>, ...>` into
linear memory. A deterministic compact-i32 corpus covered the same normal,
stop, exception, checksum, and complete-memory states and retained exactly one
non-inlined direct step call. Seven fresh pairs measured compact at only 0.477x
sret throughput `[0.469,0.650]`; the independent warm-stability gate also
failed. Synchronous tier-up and ten further non-inlining traces retained the
dominant reversal. No production result sidecar or decoder refactor was
admitted, and accepted Wasm remains exact R054.

R059 next tested a complete one-table replacement for the accepted RV64C
quadrant-plus-funct3 dispatch. The architecture-balanced mechanism was locally
strong—1.592x `[1.592,1.617]` across seven fresh pairs with exact state and
four-versus-one static jump tables—but one measured flat warm sequence spread
1.301x and failed the predeclared 1.25x stability gate. The leg is retained.
Even the deliberately favorable local ratio projects only 1.130x whole Boot
when assigned to every compressed `Cpu::step` cycle, so unchanged real handler
work leaves little product margin. No source prototype was admitted.

No untested production candidate is currently admitted. The next bounded task
must use the refreshed R055 attribution while respecting the closure audit:
R020/R055 now close one-page instruction-fetch caches, R056 closes exact
re-entry callback monomorphization as a standalone change, and R057 closes a
separate compiler Worker as throughput isolation. R023/R058 close both giant
decoder inlining and a compact scalar return across its existing call boundary;
R059 closes RV64C dispatch flattening after its independent stability failure
and narrow upper bound. Prior rounds close decoded handler tiers,
selected/exhaustive pair libraries, cold page modules, threshold retuning,
scheduler thinning, and current generated-memory proof repacking. A new
candidate must change a genuinely broader execution shape, show at least 10%
whole-row leverage before implementation, remain architecture-general, and
clear the same correctness, focused A/B, all-row, and browser guards. The goal
remains open until Boot and Compile reach copy/v86 parity.

The exact copy/v86 source/runtime audit is now complete. It pins comparator
commit `2f1346b0e7d8...` and executed Wasm `4a1b966e5433...`, confirms that v86
compiles asynchronously, and localizes the remaining Compile difference to
runtime/fallback work rather than generated subtrees. An independent opcode
census shows 99.002% Boot and 99.790% Compile coverage for an
architecture-defined common scalar subset, but no implementation is admitted
from frequency alone. The current bounded next gate is a compact
multi-instruction, precompiled T0 corpus that must prove at least 10% whole-row
leverage without reproducing the already rejected decoder-inline,
decoded-handler, or exact-sequence designs. Failure at that gate will be
recorded as a plateau, not followed by policy tuning.

R060-R062 have now tested and rejected three broader exact-comparator
hypotheses without changing production. Standard Binaryen `-O1` through `-O4`
never improved a failing row by 10%; matching v86's `+simd128` build regressed
Compile 6.3%; and carrying v86-style physical/direct instruction backing in
the scalar interpreter regressed Boot 12.9%. All legs retained exact outputs
and stable host probes. The carried-fetch implementation and every harness
switch are removed, 32 core tests pass, and release Wasm is again byte-exact
R054 `4160333352b18b...`.

R063-R064 then closed the async-publication hypothesis. Source audit proved
that the scorecard could delay rewrite module publication relative to the
event-driven v86/public-RV64 schedulers. Yielding every pump improved Compile
6.0% and Python 24.6% but tied Boot, missing its preregistered failing-row gate.
A narrower runtime return only while a build was pending reduced recorded
availability latency by 20x-300x, yet delivered only 4.9% Compile and 5.4%
Python wall gains while regressing Boot 2.4%. The product prototype is fully
removed; only the general diagnostic cadence and immutable reports remain.
Production is still exact R054 `4160333352b18b...`.

R065 now separates raw parity from emulator throughput without changing the
score. On accepted Compile, RV64 performs 30.8% more guest instructions and is
12.9% slower per instruction; raw parity therefore requires 47.7% more current
throughput and would make rewrite 30.8% faster per guest instruction than v86.
On a fresh balanced Boot diagnostic, guest counts are nearly equal
(180.816M/183.982M), while rewrite throughput is only 78.964 versus 120.827
MIPS. Boot remains the genuine emulator bottleneck. The scorecard still uses
raw wall time; these normalized figures prevent compiler/ISA work inflation
from being mislabeled as JIT overhead.

R066 then implemented the broad scalar-T0 loop implied by that diagnosis. It
integrated every ordinary RV64I/M integer, control, scalar-memory, and integer
RVC family while carrying PC and retirement in Wasm locals; uncommon families
kept the complete decoder. Exhaustive actual-RVC-prefix and broad 32-bit
differentials, Wasm smoke both off/on, full-system memory/Sv39/MPRV/atomic/FP/
WFI/T2 gates, and direct/OpenSBI Linux all passed. Three same-Wasm pairs
measured Boot at 1.074x `[1.060,1.129]`, Compile at 0.974x
`[0.959,0.996]`, and Python at 0.987x `[0.947,0.999]`. The genuine Boot gain
misses the frozen 1.10x advancement gate and slightly regresses both guards,
so the candidate was not tuned or escalated. Every hook is removed and release
Wasm is again byte-exact R054 `4160333352b18b...`.

R067 verified that R066 genuinely emitted the intended shape and localized its
residual. The candidate VirtBus drivers grew from 272/381 to 6,138/6,331 code
bytes, contain the scalar decoder directly, and call the complete `Cpu::step`
only from disabled/slow-family paths. In the treatment profile the integrated
drivers still own 62.19% of all sampled time; scalar load/store helpers own only
8.78%. The pinned v86 Boot profile spends 21.59% of time in generated Wasm,
versus 4.82% for R066, while R066 still retires 111.363M instructions through
the interpreter. This closes further scalar-loop rearrangement and admits only
an offline opportunity test for a cold privileged tier that batches many hot
pages into far fewer compilation jobs than R046. No product behavior changed.

R068 completed that offline gate. A corrected 183.000M-instruction direct-Boot
trace packages 82 stable privileged pages into 11 eight-page memory-state
modules, covers 99.047% of observed entry events, and projects 73.622M
incremental early-window instructions after formation and fresh compile delay.
All modules validate; total bytes are 5.084 MB, the largest module is 615.8 KB,
and the largest function only 15.8 KB. Four-page packaging needs 21 jobs,
sixteen-page packaging breaches the 1 MiB module bound, and register-structured
packaging creates an 834 KB giant function. The exact frozen primary clears all
opportunity gates and admits one default-off runtime implementation; production
remains unchanged R054 pending that A/B.

R069 implemented that exact candidate and rejected it at the live lifecycle
gate. The same-Wasm switch-off control booted in 2.30 seconds; a standalone
treatment took 18.04 seconds. In the frozen three-pair Boot/Compile/Python
screen, all nine control legs completed with 1.031x host-probe spread, but
eight of nine treatment legs failed the existing 30-second JIT-settle check.
The sole complete treatment Boot pair was 17.260 versus 2.279 seconds
(0.132x). It issued/landed 13 batches covering 104 pages, spent 4.286 seconds
in host compilation, and recorded only 257,844 verified batch-retired
instructions. The invalid promotion report is retained rather than repaired or
replaced. Every R069 hook is removed, and production is again byte-exact R054
`4160333352b18b...` (4,272,517 bytes).

## Repository and clean-room boundary

- Active upstream commit: `96aa93896e7bb6fa561d1f977c9bf23cd909a100`
  (`origin/main` when the delivery matrix was run).
- Original clean-room baseline:
  `4b0896decdff7538f9c1d2b44dc19a1d3d14f7c2`.
- `crates/rv64-jit` is deleted from the active tree. Its three source files were
  not opened while designing or implementing the replacement.
- The independently designed replacement is `crates/rv64-dbt`.
- A local safety stash named
  `codex-clean-room-rewrite-before-upstream-96aa938` preserves the pre-update
  rewrite state until final handoff. It is not part of the implementation.
- The worktree is intentionally uncommitted; no branch was pushed and no pull
  request was opened.

## Head-to-head checkpoint

The accepted comparison uses nine alternating paired samples, one fresh Node
process per side, exact artifact hashes, raw samples and bootstrap intervals,
and a predeclared CPU-stability rejection threshold. It passed with a 1.197x
probe spread and no correctness issue.

Where both implementations generate code, the rewrite is currently 2.00x
slower on a one-call ALU loop, 1.44x slower on mixed user code, and 1.07x
slower on fixed legacy-Linux MD5 work. The ALU gap is a baseline-Wasm tiering
effect: TurboFan-from-start and a one-million-instruction re-entry quantum both
make the two implementations effectively tied. The mixed gap remains under
both baseline and optimizing tiers; the rewrite makes about 1.77x as many
generated dispatcher entries.

Two independent nine-pair ALU runs each contain one rewrite-only 11-13 second
cold-tier outlier. CPU affinity does not remove it. Median conclusions are
stable, but tail latency is now an explicit optimization and regression target.

On current Linux direct-SBI and OpenSBI steady work, the rewrite is about 1.97x
faster end to end. Those rows are product-capability comparisons, not JIT
code-quality comparisons, because the previous implementation has zero
generated-code coverage on the modern `VirtMachine`. Direct cold boot is tied;
rewrite OpenSBI cold-to-prompt is about 1.70x slower because eager compilation
touches a substantially larger early footprint.

See [COMPARISON.md](COMPARISON.md) for the full methodology, confidence
intervals, tier diagnostics, generated-Wasm evidence, and prioritized next
work.

### Historical rv64.js scorecard checkpoint

The valid authoritative 13-row rv64.js scorecard dated 2026-08-07 scored the
rewrite at 4/13 under the scorecard's historical low-level `SB=1` policy. It
won ALU, BITFIELD, and IDEA, matched FP EMULATION, and lost the other nine
rows. The run used three alternating fresh-process pairs per side, exact
checksums and artifact hashes, CPUs 8-15, and had only 1.08x host-probe spread.
Its JSON/Markdown reports are
`/home/darren/src/arm64.js/target/bench/scorecard-2026-08-07T16-13-33.*`.

This result does not contradict the later matched WANIX browser checkpoint:
the old scorecard instantiates `RV64Debug`, enables the adaptive superblock
compatibility path on its legacy-system rows, uses separate low-level defaults
for matched Boot, and never selects the stable API's async page policy or
feature-tested tail-call trampoline. It is nevertheless decisive evidence
that the compatibility path is currently much weaker than the removed JIT on
Mixed, Numeric Sort, String Sort, Python, and Compile. BYTEmark also showed
extreme cross-process Numeric/IDEA modes. The next scorecard work is to expose
an explicit production-policy configuration without changing the historical
default, validate it on those focused rows, and only then run another complete
13-row trial. Full row values and caveats are recorded in
[FINDINGS.md](FINDINGS.md).

## Delivered compiler/runtime

- T0 remains the precise interpreter, profiler, and differential oracle.
- T1 uses typed SSA, ordered effects, bounded folding/DCE/stackification,
  guarded forward traces, direct continuations, and cumulative-fuel loops.
- Scalar `rv64gc` lowering covers integer I/M, scalar A, C expansion, scalar
  F/D, CSR/FP-control behavior, flat memory, and full-system translated memory.
- Guest faults and uncommon memory/device cases use explicit precise exits;
  generated Wasm traps are never guest exception semantics.
- Full-system fast paths use typed TLB and reservation capabilities, re-probe
  after refill, preserve LR/SC across a store-translation exit, and participate
  in code-page generation/invalidation.
- T2 supports observed batches, static and sparse multi-page regions,
  register-resident multi-entry execution, exact side exits, monomorphic and
  bounded two-way indirect targets, cross-page PICs, and dependency-aware SMC
  invalidation/retranslation.
- Asynchronous region compilation is generation-checked before publication.
  Hosts expose event-loop opportunities while `sys_pending_builds()` is
  nonzero, so compile completion cannot depend on guest workload duration.
- Every publication path records translation, copied bytes, Wasm compile,
  instantiate, publish, first-call, warm-up, steady execution, and deterministic
  module hashes for frozen replay.

## Modern system integration

The JIT is integrated with the current `VirtMachine`, not just the historical
TinyEMU-compatible machine. Both supported modern paths pass:

- direct Linux entry in S-mode with the emulator's host SBI implementation;
- OpenSBI `fw_dynamic` entry in M-mode followed by Linux in S-mode.

The delivery workload uses the repository's Alpine image and Linux 6.12,
reaches `ALPINE_READY`, runs a 20,000-iteration shell workload, executes landed
T2 regions, and checks that direct SBI reports no unsupported extension. The
separate modern Virt smoke image validates timer/RTC behavior, UART draining,
forks, and shutdown through guest markers.

## Selected portable policy

The stable system policy is the measured WANIX policy, not the earlier compact
synthetic-loop policy:

- async-only compilation keyed by verified `(VA page, PA page)` mappings;
- a 131,072-instruction threshold, 1,024-instruction sampling quantum, bounded
  64-candidate queue, and at most two Wasm compilations in flight;
- eager register-resident state, direct structured CFG lowering, and the
  Stackifier backend for reducible control flow;
- at most two captured pages and 512 observed leaders;
- control-entry sampling enabled, with a second page admitted only when each
  participating page has at most 100 non-sequential entries per thousand
  observations;
- frame-free structured-region tail chaining when the engine supports Wasm
  tail calls, through one table-owning trampoline shared by every generated
  module;
- no page rebuild, measured late extension, generated TLB fill/hash cache, or
  recursive `chain_next` chaining by default.

The control-entry ratio is the important adaptive discriminator. CPython's
computed-dispatch pages remain single-page despite the two-page cap, while the
low-control SHA loop receives adjacent reachable pages. This avoids choosing a
single fixed geometry that helps one workload and severely regresses the other.
The stable API exports the threshold, concurrency, control gate, page cap, and
leader cap so packaged-runtime measurements can prove which defaults ran.

Lazy state, materialized state, page rebuild, measured extension/demotion,
generated TLB caching, recursive chaining, and the tail-call trampoline remain
explicit A/B knobs. Tail chaining is selected by a Wasm feature test, never by
browser identity.

## Frozen and real-region measurements

- The synthetic backend corpus contains 11 exact modules spanning eager,
  lazy, direct, materialized, tail-call, and system-memory/TLB variants.
- Fresh-process Node and fresh-profile Chrome/Firefox runners use alternating
  paired order, preserve raw samples, and report min/median/p95/max plus a
  deterministic bootstrap 95% interval for the median.
- Current measured engines are Node 26.5.0 / V8 14.6, Chrome 150, Edge 150,
  Firefox 153.0.3, and Wasmtime 47.0.3.
- The real compiler-generated corpus parses two RV64 ELF inputs (`rvbench` and
  Alpine musl's dynamic loader), captures 56 matched regions over seven
  page/leader geometries, and emits five state backends for 280 deterministic
  modules. Reports include source and module SHA-256 hashes.
- Real-region frontend data shows lazy state roughly doubles bytes and compile
  work, while direct structured dispatch is usually near eager state in bytes
  and frontend latency. Eager register residency costs more frontend work than
  materialized state, increasingly so for very large regions; the execution
  corpus shows why that cost pays back on hot code.
- Wasmtime 47.0.3 independently instantiates and executes all 11 standalone
  modules with exact-state checks and AOT-compiles all 56 eager real regions.

## Final verification gate

The strict command in [REPRODUCING.md](REPRODUCING.md) completed with
`ALL STAGES PASSED`. It includes:

- 164 Rust tests across the workspace;
- guest builds and three QEMU user-mode output differentials;
- 134/134 official `riscv-tests`;
- 109/109 applicable Spike locksteps, 24,103 register writebacks, and one
  spec-legal skip;
- 193/193 `riscv-arch-test` signatures;
- 338 generated-module validity samples and 60 randomized full-state programs;
- directed I/M/A/F/D, memory, Sv39, MMIO, fault, SMC, and T2 differentials;
- 12/12 deterministic randomized multi-entry atomic programs, spanning W/D,
  interference, refill/no-refill, batch/region, lazy/eager, and precise faults;
- standalone Wasmtime execution;
- modern Linux direct and OpenSBI boots with generated execution;
- in-guest FP context switching and AMO checksums;
- the modern OpenSBI/Linux Virt smoke workload.

In addition to the strict matrix's run, the formerly timing-sensitive T2
publication gate passed five consecutive runs under the older Node 20 engine
that exposed the scheduling race.

## Known boundaries

- The complete compiled ISA contract is scalar `rv64gc`, not RVA23, RVV, or an
  unrestricted extension set. Other supported legal instructions retain exact
  interpreter fallback until separately lowered and tested.
- The portable runtime currently uses memory32 with checked/translated 64-bit
  guest addresses. A memory64 backend remains a future, separately measured
  feature.
- Current atomic proofs cover this emulator's single-hart execution model.
  A shared-memory, multi-hart mapping of the RISC-V memory model to Wasm atomics
  is separate work.
- Safari remains unavailable on this Linux host. Bun supplied a supporting
  JavaScriptCore diagnostic, while browser policy continues to use V8 and
  SpiderMonkey plus independent Wasmtime validation.
- Synthetic throughput is a backend-shape diagnostic, not an emulator headline
  result. Modern guest workload and lifecycle measurements remain the policy
  evidence.

## Next work

No item remains in the original Phase 0-6 implementation, correctness,
black-box comparison, async compile-policy, browser guard, or authoritative
three-way measurement outline. The active goal is still open because accepted
R054 loses only Boot and Compile to copy/v86, but both lose by about 1.48x.
The work is at a measured structural plateau, not at an unfinished checklist:
R066 was the last admitted broad Rust-interpreter-loop design and recovered
only about 7% on Boot while slightly regressing the two guards.

Do not continue with scalar decoder layout, PC/minstret materialization, fetch
capability caching, decoded handlers, threshold/cadence tuning, module-size
heuristics, worker-pool changes, or scorecard-selected opcode subsets. Those
mechanisms now have direct negative or insufficient-leverage evidence. Compile
also cannot reach raw parity merely by matching v86's per-instruction rate:
its RV64 binary performs 30.8% more architectural work.

The next implementation requires a genuinely different representation and an
independent leverage gate before source changes. Plausible scopes are a
privileged baseline generated tier that amortizes compilation without one
WebAssembly module per code page, or a deliberately hand-shaped Wasm execution
core that changes register/state and memory representation rather than asking
LLVM to rearrange the Rust decoder. Either must first demonstrate at least 10%
whole-Boot opportunity on architecture-general traces/corpora and explain how
it escapes the R023/R025-R027/R042/R045/R046/R055/R058/R062/R066 closures.
Until such a mechanism is bounded, more implementation would be benchmark
waffling. Production remains R054 and the accepted scorecard/browser results
remain the honest current status.

R067 supplies that next bounded question. Its exact archived profiles show
111.363M remaining interpreted Boot instructions and only 4.82% generated-Wasm
engine time, versus v86's 21.59%. The active gate is now an offline,
JIT-disabled batch-geometry test: select privileged pages by architecture and
heat only, emit unrelated pages into a small number of modules, and account for
post-threshold entry coverage, module bytes, and actual V8 compile latency. A
runtime implementation is allowed only if that evidence projects at least 10%
whole-Boot leverage and avoids both R046's 51-52 one-page compile storm and
R048's giant-function late-tiering shape.

R068 answered the offline opportunity question positively, but R069 falsified
its runtime premise: eight-page packaging still caused live compile/settle
work to dominate and almost none of the projected generated retirement became
available in time. Do not sweep batch size, threshold, leader count,
concurrency, or timeout after this result; R046 and R069 jointly close runtime
cold privileged Wasm compilation in the current module-per-tier architecture.
No untested production candidate is now admitted. A successor must change the
execution or compilation representation itself and pass a new independent
whole-row opportunity gate before implementation.

## WANIX three-way comparison page

`integrations/wanix/v86-rv64-three-way.html` now gives copy/v86, the pinned
pre-rewrite `v0.1.0` RV64 release, and the workspace rewrite independent start
controls and terminals over one shared WANIX namespace. `make comparison`
produces distinct, coexistable archives and verifies the two downloaded legacy
payloads. Browser integration found and fixed both halves of asynchronous WFI
wakeup: zero-progress execution returns to JavaScript, and an external 9P reply
immediately publishes its used-ring entry and IRQ. The cold/warm WFI regression,
virtio device tests, modern direct/OpenSBI boots, adapter test, archive
layout/hash checks, and clean Chrome startup all pass. In a fresh profile the
rewrite reaches a prompt without injected input and executes a terminal command.
The guest also avoids a redundant BusyBox applet installation and skips the
unused fetch-proxy setup in comparison mode.

The reference pane no longer uses WANIX's stock x86 archive. That archive was
Alpine 3.22.5 but carried Linux 6.16.3, WANIX's older init, and no Python,
whereas the RV64 archive carried Linux 6.12.7, the optimized init, and Python.
One builder now produces matched Alpine 3.22.5 `x86`/i686 and `riscv64`
archives with identical package-world entries, init bytes, kernel version, and
pinned WANIX helper sources. Only guest-ISA binaries, kernel container format,
and required virtio-pci versus virtio-mmio configuration differ. A fresh
headless Chrome smoke boots the i686 pane, verifies `i686`, Alpine 3.22.5 and
Python 3.12.13, and completes every `/shared/bench.py` phase including live 9P
I/O.

## True interpreter comparison baseline

The WANIX pane used `rv64.jit=off` while establishing a clean interpreter
baseline; it now runs the selected async page JIT. The adapter retains that
flag for explicit controls. JIT-off branches around `run_system_jit` entirely:
one public Virt budget is passed directly to `VirtMachine::run_slice`, followed
by the normal console, export, network, and power-state drain. It does not
initialize `JitState`, probe caches, update hot counters, check generated-code
invalidation, or split the budget into fallback slices.

The dedicated Wasm regression retires 100,000 instructions from a minimal
modern Virt kernel and asserts zero generated retirement, dispatches, cache
entries, dispatcher fallback slices, translation attempts, and registered
Wasm modules. The stable public API, Wasm smoke suite, and directed full-system
atomic and memory differentials also pass with the direct bypass in place.

A three-pair diagnostic used fresh Node 26.5/V8 14.6 processes and alternated
the exact packaged legacy and rewrite cores over a 100-million-instruction
modern-Virt loop. Rewrite/legacy throughput ratios were 0.995x, 1.015x, and
1.005x (1.005x median); both sides reported zero generated retirement,
fallback slices, dispatches, entries, translation attempts, and registrations.
This synthetic loop is not a Linux workload claim, but it confirms that the
runtime interpreter paths are now comparable rather than separated by the
rewrite dispatcher's overhead.

## Async page compile policy

Opt-in instruction tracing captured direct boot, OpenSBI boot, long ALU, and
long memory/ALU mix without changing the ordinary interpreter path. Offline
simulation selected `(VA page, PA page)` as the safe reusable heat identity.
The live policy has a 64-candidate bounded priority queue, at most two async
Wasm builds in flight, generation-checked publication, targeted dirty-page
invalidation, a 131,072-instruction threshold, and a 1,024-instruction
sampling quantum.

Matched-Linux browser sweeps supersede the earlier 1,048,576/one-build compact
loop selection. A 512-leader cap restores roughly 99% generated coverage. A
two-page cap helps SHA but hurts CPython when applied indiscriminately, so the
100-per-thousand control-entry gate admits the second page only for pages with
mostly sequential/direct entry traffic. Browser A/B rejected entry-count-only
gating, removal of cross-page calls, eager measured extension, demotion, page
rebuild, generated TLB fill/hash caching, and recursive chaining as defaults.
Later matched measurements accepted frame-free tail chaining through one host
trampoline. The WANIX archive and three-way page ship the selected adaptive
policy.

## Matched WANIX CPython performance checkpoint

Five alternating pairs per browser used one fresh process/profile and one
fresh guest per VM sample, pinned to host CPUs 8–15. The fixed protocol was
written before the first sample and held a global experiment lock. Each sample
ran one phase-synchronized benchmark, retained all raw times, used no JIT
override, checked `riscv64` versus `i686`, Alpine 3.22.5, Linux 6.12.7, Python
3.12.13, the Python checksum, and the SHA digest. The primary ratio is the
paired geometric mean of RV64/v86 elapsed times; intervals are exact paired-
bootstrap percentile 95%.

Chrome 150.0.7871.186 / V8 15.0.245.21:

| `/shared/bench.py` phase | RV64 median | v86 median | paired ratio | 95% interval |
| --- | ---: | ---: | ---: | ---: |
| pure Python CPU | 1.761 s | 1.978 s | 0.873x | 0.820–0.930x |
| SHA-256, 32 MiB | 3.903 s | 4.762 s | 0.832x | 0.803–0.867x |
| shared 9P I/O | 1.095 s | 1.085 s | 1.021x | 1.008–1.035x |

Microsoft Edge 150.0.4078.105 / V8 15.0.23.12:

| `/shared/bench.py` phase | RV64 median | v86 median | paired ratio | 95% interval |
| --- | ---: | ---: | ---: | ---: |
| pure Python CPU | 1.764 s | 1.981 s | 0.899x | 0.875–0.924x |
| SHA-256, 32 MiB | 3.954 s | 4.601 s | 0.857x | 0.835–0.879x |
| shared 9P I/O | 1.193 s | 1.142 s | 1.046x | 1.020–1.072x |

Every upper bound passes the predeclared 10% maximum-slowdown criterion. In
Chrome the rewrite is about 12.7% faster on pure Python and 16.8% faster on
SHA, while shared 9P is 2.1% slower. In Edge it is about 10.1% and 14.3% faster
on the compute phases and 4.6% slower on shared 9P. SHA includes architecture-
specific native-library effects; pure Python remains the clearest JIT/emulator
signal.

Every RV64 phase proves page cap 2, leader cap 512, control threshold 100 per
thousand, control-profile experiment off, tail-chain feature enabled, nonzero
tail-chain transfers, and greater than 90% generated coverage. The analyzer
also rejects protocol/order overlap, CPU-affinity drift, browser or artifact
changes, missing correctness markers, overrides, and invalid guest identity.

The immutable comparison page SHA-256 is
`34a0d0a70730549d68582432f0a207535db3fa81bb468d17585f1d30cb834588`.
It binds archive
`2598532832a2b9ad27ca99889a70cc8ab42796296990b91dd2b60e70c793be86`,
whose Wasm SHA-256 is
`7136691c8ba0ab5ee3f4c9b3e74cd9f09af73bf43bb19b71cad97cff66a241ac`.
Raw logs, the pre-registered protocol, and `analysis.json` are under
`target/jit-policy-traces/wanix-parity-known-fallback-pinned-8-15-{chrome,edge}150-20260807/`.

## 2026-08-09: cumulative-gain policy is valid, but residual-only static T0 is not promoted

The project now permits architecture-general cumulative improvements below
10%, but only under the prospectively frozen, more expensive five-pair path in
`tests/vs-v86/METHODOLOGY.md`. Historical sub-10% rejections are not relabeled.
R071 independently retested the unchanged default-off R070 static decoder so
the earlier favorable three-pair sample could not decide its own new rule.

The valid five-pair modern Linux report is
`target/bench/r071-static-t0-independent-confirmation/config-ab-2026-08-09T05-43-19-283Z.json`.
Paired Boot/Compile/Python results are 1.024x `[1.000,1.050]`, 0.998x
`[0.962,1.020]`, and 1.032x `[0.983,1.043]`; host spread is 1.062x and all
semantic/lifecycle proofs pass. The frozen verifier rejects Boot below its
1.03x paired-median gate. Browser and full-scorecard escalation are skipped.

Accepted production remains R054: 11/13 against v86 and 13/13 against legacy.
The next broad opportunity is not a threshold retune. It is to preserve the
page policy's exact architecture-general observations while letting the static
core execute the 105M+ Boot instructions currently forced through the Rust
sampled interpreter.

## 2026-08-09: sampled static T0 clears native performance but fails the browser I/O guard

R072 implemented that exact sampled-observation hypothesis. It preserves the
ordered q1/q32/q1024 policy stream, generated-entry handoff, interrupt timing,
slow/fault/MMIO ownership, and one guest-independent module. All eight strict
correctness stages passed: 134 ISA cases, 109 Spike locksteps, 193 signatures,
the complete Wasm/system matrix, direct/OpenSBI Linux, and virt smoke.

The frozen same-Wasm native report measures Boot 1.209x faster with interval
`[1.190,1.239]`, Compile 1.021x, and Python 1.049x. Candidate Boot executes
roughly 106M sampled instructions with zero internal errors. This is a real
architecture-level improvement and demonstrates that 20% gains remain
possible, but 20% is not the standing minimum; R071's general 3% cumulative
track remains in force.

R072 is nevertheless not promoted. The first browser run is retained invalid
because a newly added 1.25x raw spread cap contradicted accepted historical
shared-9P behavior. A separately frozen five-pair confirmation did not reuse
it. Python (1.029x candidate/control) and SHA-256 (0.990x) pass the 1.03
nonregression limit; shared 9P is 1.041x and fails. Its interval
`[0.829,1.268]` is too wide to establish a causal 4% regression, exposing a
benchmark-design weakness, but the predeclared point-median rule remains the
decision. Candidate/v86, default-on, and full-scorecard gates were not run.

Current accepted status is therefore unchanged: R054, 11/13 versus copy/v86,
13/13 versus legacy, with Boot and Compile still the two parity losses. R072
remains default-off research code and the thread goal is not complete.

## 2026-08-09: sampled-only R073 preserves compute but still fails the strengthened browser I/O guard

R073 separated R072's treatment into its causal components: only exact
page-policy samples use static T0, while ordinary residual execution remains
on the accepted interpreter. Directed q1/q32/q1024, generated-entry, WFI,
direct-Linux, OpenSBI, API, and lifecycle gates pass with unchanged main Wasm
`cb7ea81685b3...`. Five fresh native pairs measure Boot 1.157x faster
`[1.137,1.187]`, Compile 1.036x, and Python 1.027x, with host spread 1.018x,
exact outputs, and zero errors.

The prospective browser method used seven alternating fresh Chrome guests and
three synchronized repetitions per phase. Its valid immutable report is
`target/jit-policy-traces/wanix-r073-cb7ea816-chrome-20260809-config-ab/analysis.json`
(`9bbaf2cd89cc...`). Candidate/control elapsed ratios are Python 0.994x
`[0.979,0.997]`, SHA-256 0.999x `[0.984,1.003]`, and shared 9P 1.058x
`[0.932,1.114]`. Thus compute is unchanged or slightly faster, but shared 9P
misses both its 1.03 median and 1.10 upper-confidence guards.

R073 is rejected without rerun and stays default-off. The analyzer's initial
Alpine-banner assertion was repaired without changing measurements: three
repetitions legitimately pushed the boot banner beyond the retained terminal
tail, so schema 3 now requires the exact frozen modern-Alpine root hash in
addition to the harness's successful release check and the recorded guest and
phase identities. Accepted production remains R054 at 11/13 versus copy/v86
and 13/13 versus legacy; Boot and Compile parity remain open.

## 2026-08-09: R074 preserves the native Boot gain but fails the valid browser I/O gate

R074 added an architecture-general per-mapped-entry backoff to R073. A first
sample shorter than 64 instructions marks its opaque entry; later page-policy
samples use the accepted interpreter. Exact state/policy differentials,
dirty/reset/generated-entry/WFI lifecycle, the full eight-stage suite, and
modern direct/OpenSBI Linux all pass. The exact archived Wasm is
`28ceaf7bcf63b...`.

The five-pair native report passes: Boot 1.154x `[1.145,1.177]`, Compile
0.979x `[0.948,1.054]`, and Python 0.999x `[0.981,1.061]`. The prospective
seven-pair, three-repetition Chrome report is measurement-valid and immutable
at
`target/jit-policy-traces/wanix-r074-28ceaf7b-chrome150-20260809-config-ab/analysis.json`
(`d3cf3a102966...`). Python 1.010x and SHA-256 1.002x pass, but shared 9P is
1.068x `[0.959,1.122]` and fails both frozen limits.

Backoff reduced shared static samples by roughly 93% versus R073, but candidate
shared execution still performs about 8% more guest instructions and issues a
median 10 rather than 6 page modules; external host 9P time is nearly equal.
R074 is rejected, candidate/v86 and promotion gates are skipped, and accepted
production remains byte-exact R054: 11/13 versus copy/v86 and 13/13 versus
legacy. Boot and Compile parity remain open. The next action is causal
page-module lifecycle attribution, not an entry-key or threshold retune.

## 2026-08-09: R075 fixes the browser I/O interaction but provides no Chrome Boot gain

R075 tested the product lifecycle missing from R074: both sides prepare the
same guest-independent static module before `vm.start`, while candidate learns
its general short-entry state throughout firmware and Linux boot. Two immutable
pages differ only in the adapter-only `rv64.static-t0` value. Parser/default
tests, lifecycle smokes, public/Worker APIs, explicit sampled-backoff
direct/OpenSBI Linux, and the complete strict eight-stage suite pass. Candidate
already has 64.186M sampled retirement, 224 marks, and 88,945 bypasses at the
WANIX shell; control activity is exactly zero.

All fourteen prospective Chrome legs completed without replacement. The
measurement-valid schema-5 report is
`target/jit-policy-traces/wanix-r075-preboot-28ceaf7b-chrome150-20260809-config-ab/analysis.json`
(`53d7233fb9dd...`). Browser shell speedup is only 0.997x control/candidate
`[0.993,1.002]`, failing the frozen 1.10x/lower-bound-1.00 rule. The seven-side
shell medians are 31,190.1/31,300.8 ms with low 1.012x/1.014x spreads, so this
is a precise tie rather than unresolved noise.

Every `/shared/bench.py` guard passes: Python is 1.014x `[0.990,1.034]`,
SHA-256 0.996x `[0.990,0.998]`, and shared 9P 0.871x `[0.799,1.059]`
candidate/control. Preboot learning therefore removes R074's I/O concern, but
the 1.154x same-Wasm Node/native Boot benefit does not transfer to Chrome/V8.
R075 is rejected without candidate/v86, promotion, or scorecard escalation.
Accepted production remains exact R054 at 11/13 versus v86 and 13/13 versus
legacy; Boot and Compile parity remain open.

## 2026-08-09: R076 isolates a real Chrome execution gain and clears the product guard

Boot-scoped Chrome profiles showed that R075's launch-to-shell timer hid the
emulator boundary inside roughly 31 seconds of common WANIX startup. R076
therefore reproduced the authoritative scorecard boundary in fresh Chrome
Workers without changing the main Wasm, guest, pump cadence, or page policy.
Both sides prepared the same untimed auxiliary module; only sampled/backoff
execution differed.

All fourteen frozen legs completed without replacement. The valid result under
`target/jit-policy-traces/r076-chrome-modern-boot-28ceaf7b-20260809-config-ab/`
measures 1.175x execution speedup `[1.167,1.189]` and 1.174x normalized-MIPS
gain `[1.167,1.189]`, with nearly equal 180.35M/180.30M instructions and zero
errors. This passes the prospective 5% cumulative-gain gate and confirms the
mechanism itself is useful in Chrome/V8.

The fresh seven-pair, three-repetition candidate-v86 WANIX guard also passes.
RV64/v86 elapsed ratios are Python 0.891 `[0.876,0.924]`, SHA-256 0.632
`[0.614,0.638]`, and shared 9P 0.669 `[0.577,0.830]`, with complete artifact,
correctness, generated-JIT, and lifecycle proof. R076 therefore admitted one
production-default integration and the strict/full-scorecard R077 escalation;
it did not itself change the accepted R054 scorecard baseline.

## 2026-08-09: R077 production promotion fails; default is restored off

R077 centralized the sampled/backoff lifecycle in the loader, enabled it for
every non-bare-metal machine before execution, rebuilt the WANIX archive, and
passed stable API/Worker/reset tests, adapter tests, directed differentials,
the full eight-stage strict suite, direct/OpenSBI Linux, a fresh production
Chrome confirmation, and a no-override WANIX smoke. Its valid Chrome A/B
confirmation measured 1.163x execution speedup `[1.136,1.191]` and identical
normalized-MIPS gain, proving the default integration invoked the intended
mechanism.

The untouched authoritative three-way run then completed all 117 trials once.
It is permanently invalid because legacy HUFFMAN spread 1.425x across
12.641/13.233/18.017-second STEADY samples, above the frozen 1.25x cap; no leg
was replaced. More importantly, the frozen independent promotion evaluator
also rejects raw Boot on its own stable samples: R077 median 2,293.093 ms versus
R054 2,260.485 ms, or 0.986x instead of the required 1.05x speedup. Descriptive
counts remain 11/13 versus v86 and 13/13 versus legacy, and every rewrite row
stays within 5% of R054, but neither fact rescues the failed Boot gate.

The exact report, verifier result, hashes, and rejection are recorded in
`docs/jit-rewrite/R077_DEFAULT_SAMPLED_T0_PROMOTION_PROTOCOL.md`. The rejected
loader/archive are preserved by hash; stable full-system and scorecard paths
are restored default-off. The rebuilt archive is `378219063e1b...`; a fresh
no-override WANIX rollback smoke reaches the shell, completes correct Python
with 566.4M generated instructions, and proves module index -1 plus all-zero
sampled/static counters. Accepted performance baseline remains R054 at 11/13
versus copy/v86 and 13/13 versus legacy. The thread goal is still open, with
raw Boot and Compile as the two v86 parity losses.

## 2026-08-09: R078-R080 restore a trustworthy source-built baseline

R078 compared the post-R077 default-off artifact directly with exact accepted
R054 in five alternating pairs. R054 was 1.178x faster on Boot
`[1.116,1.190]`, 1.037x on Compile, and 1.031x on Python. This proved that the
rejected static-T0 experiment imposed a material disabled-path regression and
admitted source removal. R079 removed the obvious static emitter/runtime, but
failed restoration: it was only 0.845x R054 on Boot `[0.809,0.885]`.

An exact session-history audit identified the missed R070/R072 residue: a JIT
fetch-context refresh, unused static fetch/interrupt helpers, a changed M/Bare
fetch probe, a mutable fingerprint branch in every page-policy observation,
two differential-only user-memory exports, and a loader entropy hook. R080
removes these while retaining the independently correct WFI-yield behavior.
Its 4,272,559-byte main Wasm is `e5415db83b27...`, with exactly R054's import,
function, export, element, and data counts and only a 38-byte code difference
from the WFI fix.

The full strict correctness matrix passes. A fresh 30-leg frozen A/B against
exact R054 passes with Boot 1.033x `[0.982,1.042]`, Compile 1.013x
`[0.974,1.072]`, and Python 1.011x `[0.981,1.034]` R080 speedups. The report
is
`target/bench/r080-residual-cleanup/config-ab-2026-08-09T13-53-23-904Z.json`.

The untouched R080 authoritative scorecard is valid with no problems and all
117 trials:
`target/bench/r080-authoritative-three-way/scorecard-v2-2026-08-09T14-47-31-985Z.json`.
R080 is 13/13 versus modern legacy and 11/13 versus copy/v86. Its only losses
are Boot (2,338.4/1,562.2 ms, 1.50x slower) and Compile
(1,058.8/718.3 ms, 1.47x slower); Python is 1.10x faster than v86.

The final five-pair fresh-Chrome browser guard also passes. Rewrite/v86
elapsed ratios are Python 0.875 `[0.863,0.888]`, SHA-256 0.608
`[0.592,0.630]`, and shared 9P 0.655 `[0.551,0.747]`. The archive hash is
`414a17454216...`, and every exact artifact, output, guest, and generated-code
proof passes. R080 is therefore the promoted clean source-built baseline.
The performance objective remains active at 11/13; Boot and Compile are the
only unfinished rows.

## Standing optimization economics

Twenty percent is not a retention threshold. It remains useful only as an
early opportunity screen for expensive speculative mechanisms. The standing
R071 cumulative track accepts a general, reproducible improvement from 3%
upward when it uses five fresh pairs, a non-regressing confidence bound,
complete correctness, tighter non-target/browser guards, and the untouched
scorecard. Historical decisions are not relabeled. Multiple honest 3-5% gains
are expected at this maturity and should be accumulated; workload-, PC-,
symbol-, compiler-output-, or browser-specific selectors remain inadmissible.

## 2026-08-09: R081 localizes the clean baseline's two remaining deficits

Fresh proof-only V8 profiles on exact R080 reproduce the authoritative row
shape without using their inspector-distorted durations as performance
claims. Boot is 93.4% runtime Wasm and only 4.9% generated Wasm;
`Cpu::step` alone owns 50.3% of the complete phase. Exact counters show only
38.43% generated coverage. copy/v86 Boot instead spends 21.3% of samples in
generated Wasm. The Boot problem is primarily cold/residual execution, not
host module compilation.

Compile STEADY is 52.5% runtime and 46.4% generated Wasm despite 92.40%
generated retirement. The scheduler subtree divides into 43.7% generated
execution, 17.7% policy/interpreter, 14.4% final-outcome handling, 10.8%
scheduler self, 6.9% translation, and 1.6% hashing. copy/v86 spends 70.4% in
generated Wasm. This preserves the earlier conclusion that a small residual
instruction population and dispatch boundary consume disproportionate host
time.

V8 native-code capture proves the 10,886-byte full-system `Cpu::step` reaches
TurboFan; lack of tier-up is not the cause. The next bounded implementation
must either remove a general hot interpreter cost or put the previously useful
guest-independent scalar execution core outside the main Wasm. The latter is
newly admissible only because R078 measured the old linked/dormant machinery
as a 1.178x Boot regression. Restoring that source or retuning its thresholds
remains forbidden. Production stays R080 at 11/13; Boot and Compile remain
open.

## 2026-08-09: R082 external Tier-0 stops at its first performance gate

R082 proved that the guest-independent scalar emitter can live in a separate
217,556-byte Wasm and that a small 11.3-KiB auxiliary execution module can
share the main runtime's memory and table correctly. Focused units,
q1/q32/q1024 exact differentials, generated handoff, and modern direct/OpenSBI
Linux all passed with zero external-tier errors.

The prospectively first gate compared exact R080 with the dormant-capable
4,281,763-byte main runtime in five alternating fresh-process pairs. Point
estimates were neutral or favorable, but Boot's paired interval was
`[0.967,1.092]`, missing the frozen 0.970 non-inferiority floor by 0.003. The
valid 30-leg report is retained at
`target/bench/r082-disabled-artifact/config-ab-2026-08-09T15-45-52-437Z.json`.
Per protocol, no active timing, browser guard, v86 comparison, retry, or gate
relaxation followed.

All R082 runtime integration has been removed. A clean release rebuild is
byte-identical to R080 at 4,272,559 bytes and `e5415db83b27...`; `web/rv64.js`
is byte-identical at `2cbb264f4dac...`, and the production archive remains
`414a17454216...`. Focused Wasm/API/Worker/system-memory/WFI checks pass after
restoration. Goal status is unchanged: production remains a trustworthy 11/13
baseline, with only Boot and Compile short of copy/v86 parity.

## 2026-08-09: R083 full-system specialization is smaller but much slower

R083 resolved the user-only/full-system construction invariant once per
interpreter call and generated const-specialized step, RV64C, translation,
load/store, fused-row, and FP-state paths. The 4,249,602-byte candidate is
22,957 bytes smaller than R080, retains identical import/function/export
counts, and exposes no runtime switch. The complete strict matrix, direct and
OpenSBI Linux, modern virt smoke, and public/Worker APIs all pass. Seven fresh
cold-compile pairs also pass their cap at a 4.49% candidate regression.

The valid five-pair native gate rejects it decisively. Boot is
2,316.232/2,609.354 ms control/candidate, paired speedup 0.888x
`[0.869,0.898]`; Compile is 0.963x `[0.905,1.074]`, and Python 0.985x
`[0.952,1.031]`. Host spread is 1.021x, inputs and outputs match, and all 30
legs are retained at
`target/bench/r083-full-system-step/native-ab-valid/config-ab-2026-08-09T16-17-26-914Z.json`.
The earlier path-error report has zero successful trials and remains recorded
as invalid setup evidence.

No variant, browser run, or scorecard followed. Candidate source and Wasm are
archived, all code was removed, and release Wasm/loader/archive are again exact
R080 (`e5415db83b27...` / `2cbb264f4dac...` / `414a17454216...`). Production
and goal status remain 11/13, with Boot and Compile open. No new implementation
is admitted until a closure-aware residual analysis identifies a distinct
general category with at least 3% whole-row leverage.

## 2026-08-09: R085-R089 establish the corrected 11/13 baseline

R085 promoted the private randomized integer-key JIT tables after the complete
strict, native, Chrome, WANIX, and authoritative gates. Product remains Wasm
`efd7830307ef...`, loader `2cbb264f4dac...`, and archive `0b953be67610...`.

R087 then fixed a scorecard-only fairness defect: normal rewrite execution now
yields after every 2M-instruction slice, matching the public scheduler and the
event-driven v86 runner. The corrected authoritative 117-trial report
`1d822f1c1f37...` is valid and still scores 13/13 versus legacy and 11/13
versus v86. Only Boot (2,176.495 versus 1,563.646 ms) and Compile (954.483
versus 728.859 ms) remain. Python is 2,346.773 versus 3,456.560 ms and therefore
has substantial no-regression headroom.

R088 admitted exact re-entry monomorphization from a 12.719% Boot profile
fraction and a frozen 1.0439x projection. R089 implemented and proved that
mechanism exactly, passed the complete correctness matrix and cold-artifact
gate, then failed the first product gate: Boot regressed to 0.972x
`[0.958,0.996]`. Compile improved 1.026x and Python tied, but the candidate was
rejected without variants or escalation. Source and release Wasm are restored
byte-exact R085. The goal remains active at 11/13; exact callback
monomorphization is now closed.

R090 then tested the next apparent scheduler cost before implementation. Its
instrumented modern Boot and Compile phases recorded 575k--733k generated
outer dispatches per measured phase but exactly zero non-region returns or
indirect-feedback checks. Production page-policy entries are all region
functions, so the existing region tag already skips the two map probes. The
metadata proposal is closed at zero opportunity, all instrumentation is
removed, and release Wasm is again byte-exact R085. The remaining Compile
scheduler target is the executed region call/return loop, not feedback maps.

R091 isolated that exact loop without changing its semantics. The Virt
scheduler shrank 18.98%, and a production native-code capture proves the helper
naturally advanced from Liftoff to TurboFan. Exact ABI, cold compile, the full
strict matrix, and three fresh modern Boots all pass. The valid five-pair
product gate, however, measures Boot 0.975x `[0.960,1.020]`, Compile 1.009x
`[0.950,1.070]`, and Python 1.010x `[0.978,1.038]`. Neither target reaches the
prospectively frozen 1.03x cumulative threshold. R091 stopped before browser or
scorecard escalation; no boundary variant was tried. Candidate source, Wasm,
native tier trace, and reports remain archived, while production source and
release Wasm are again byte-exact R085.

Current status is therefore unchanged in score but narrower in diagnosis:
13/13 versus legacy and 11/13 versus copy/v86, with only Boot and Compile open.
Corrected authoritative medians remain 2,176.495/1,563.646 ms for Boot and
954.483/728.859 ms for Compile. Missing optimized-tier code in the scheduler,
like dormant feedback bookkeeping and exact re-entry monomorphization, is now
closed as a standalone explanation. The next candidate must remove a measured
architecture operation rather than rearrange the same scheduler work.

## 2026-08-09: R092 wins Compile but fails the browser integration guard

R092 independently rebuilt the fixed R016 whole-member memory-range mechanism
as a separate default-on product Wasm. Its candidate `5baeccb5c5fe...` passed
the complete strict matrix and all native gates. Five fresh pairs measured a
1.132x Compile speedup `[1.030,1.136]`, with Boot inside its 3% elapsed guard
and Python essentially flat. Direct Chrome Boot also passed at 0.974x
`[0.954,1.017]`.

The frozen WANIX guard rejected the candidate. Seven fresh alternating browser
pairs, each with three guest samples per phase, measured R085/R092 speedups of
0.993x shell, 1.025x Python, 0.989x SHA-256, and only 0.810x shared 9P
`[0.741,0.894]`. All 126 workload results and all identities were correct, but
shared 9P was slower in six of seven pairs and missed both preregistered
non-regression limits. Per protocol, no full scorecard or post-hoc selector
variant followed.

R092 is rejected, not promoted. Its unique artifact, source archive, and all
positive and negative reports remain under `target/bench/r092-member-range/`.
The product source hashes and every active Wasm copy are restored byte-exact
R085 (`efd7830307ef...`), and restored DBT/Wasm units pass 53/53 and 4/4.
The goal remains active at 11/13; accepting general reproducible 3-5% gains is
still policy, but a target-row gain cannot override a material integration
regression.

## 2026-08-09: R093 confirms a 5-9% Boot gain but fails WANIX confidence

R093 used the explicit cumulative-gain exception to reconstruct the complete,
architecture-defined R066 scalar Tier-0 loop once against exact R085/R087. The
same-Wasm causal screen passes at Boot 1.045x `[1.029,1.080]`, while Compile
and Python remain within 1%. The clean default-on product then improves three
native Boots 1.074x `[1.057,1.088]` and seven fresh-Chrome Boots 1.089x
`[1.016,1.122]`. All exhaustive scalar/RVC and full-system correctness gates
pass. These results validate that reproducible general gains in the 3--5%
class are useful and can compound; a 20% per-change minimum would have thrown
away real progress.

The ordered WANIX guard nevertheless fails. Shell, Python, and SHA-256 are
1.001x, 1.000x, and 1.002x R085; shared 9P has a favorable 1.040x median but a
very wide `[0.730,1.580]` exact interval, below the frozen 0.909 lower bound.
The stop rule does not allow a post-result retry or exemption, so no untouched
scorecard followed. Candidate and all evidence are archived under
`target/bench/r093-integrated-scalar-t0/`.

R093 is rejected and completely removed. Core/Wasm source, release and WANIX
Wasm, loader, and archive are byte-exact R085 (`aec4b31434a6...`,
`1da35e70bc9c...`, `efd7830307ef...`, `2cbb264f4dac...`, and
`0b953be67610...`), and restored units pass. Official status remains 13/13
versus legacy and 11/13 versus copy/v86. Boot and Compile remain the two open
rows at the R087 medians; R093 is closed as a product candidate despite its
real Boot gain because the complete promotion contract did not pass.

## 2026-08-09: R094 qualifies a stable future shared-9P guard

R094 prospectively replaced only the versioned WANIX guard's 4 MiB shared-9P
work with 32 MiB. Both comparison pages bind the same immutable R085 archive
and normalize byte-identically; the public page, emulator, policy, Python,
SHA-256, and official scorecard are unchanged. Seven alternating fresh-browser
pairs with three synchronized samples per leg completed without retries.

All 42 samples lasted 23.765--25.757 seconds, wrote exactly 32 MiB through P9,
read at least 32 MiB, used 4 KiB maximum transfers, and proved generated
execution plus exact retirement accounting. Maximum within-browser spread was
1.068x. The exact-R085 null comparison measured 1.0004x with exact paired
bootstrap interval `[0.9984,1.0165]`, passing the frozen two-sided bounds.
Report `target/bench/r094-long-shared9p/gate.json` is
`760567deb6f7...`.

This qualifies the exact long guard for future candidates and fixes a real
measurement weakness without rewriting history. R093 remains rejected, exact
R085 remains production, and the official goal remains 11/13 versus v86 with
Boot and Compile open. Reproducible general 3--5% gains remain acceptable;
20--30% describes the aggregate parity gap, not a per-change admission rule.

Before R095 performance timing, user review correctly challenged its proposed
6,144-byte main-CODE rejection threshold. The exact number was an unmeasured
implementation budget, not a causal performance boundary: R078 confounded
size with linked/hot machinery, R082's smaller bridge had neutral point
estimates, and R083 was smaller but slower. R095 now records and attributes
section deltas but lets its cold-construction and execution gates decide their
cost. The semantic requirements—prebuilt position-independent auxiliary,
no runtime compiler/emitter in the main artifact, and no duplicated main
orchestration—remain unchanged. No R095 performance result existed when this
protocol correction was made.

R095 subsequently completed its implementation and correctness audit. The
immutable auxiliary was deterministic at 274,473 bytes (`2be7aab6...`), the
main preserved the complete R085 ABI, and exhaustive shipped-module,
full-system, 134 ISA, 109 Spike-lockstep, 193 architecture-signature, direct
Linux, and OpenSBI Linux gates passed with zero external errors. A first
construction diagnostic was invalid because it charged eager `SYS_JIT`
creation only to the candidate; the corrected R093-compatible module boundary
passed seven pairs at 19.208 versus 19.866 ms (1.034x, paired 1.0499x).

The frozen same-main/same-auxiliary causal gate then rejected R095. Three
complete alternating pairs measured Boot 2,209.07 versus 2,344.03 ms, paired
speedup 0.944x with interval `[0.933,0.949]`; Compile regressed 2.57%, while
Python improved 2.9%. Host spread was 1.034x, inputs/outputs matched, the
disabled leg retired zero external instructions, and the enabled leg retired
about 107.23M Boot instructions externally with zero errors. The external
boundary therefore made the dominant residual Boot path slower despite its
semantic success. Stop before product/browser/scorecard timing, archive the
candidate, remove all R095 product integration, and restore exact R085. Do not
tune or retry its threshold, callback, batching, layout, or opcode choices.

The removal audit is complete. Cargo, loader, CI/release/Pages, demo assets,
WANIX, public/Worker tests, system differentials, and scorecard code contain no
R095 product path. The clean source build's complete executable CODE section
and all 3,702 function bodies are byte-identical R085; only one diagnostic
source-location byte and two LLVM name suffixes differ (`d9f686a9...`,
4,279,380 bytes). The active packaged Wasm is the immutable tested R085
`efd7830307...`; loader remains `2cbb264f4...`. Release workspace tests, 23
focused Wasm/JIT gates, public/Worker APIs, and fresh direct/OpenSBI Linux
6.12.7 boots pass. The current R094-adapter WANIX archive is `6d28e87a...` and
contains only its adapter, exact R085 Wasm, and loader—no auxiliary module.

R096 then tested whether removing the non-architectural counter update on each
cross-module tail transfer would reduce the remaining Compile gap. Exact shape
and on/off execution proofs passed, as did the focused correctness matrix,
workspace units, and modern direct/OpenSBI boots. The frozen seven-pair
same-artifact result was Boot 1.005x `[0.999,1.009]`, Compile 0.991x
`[0.959,1.028]`, and Python 0.994x `[0.973,1.031]`, with host spread 1.022x.
It therefore failed on Compile before any product/browser/scorecard stage.
All live R096 product and harness machinery is removed; immutable evidence is
under `target/bench/r096-tail-chain-accounting/`. Official status remains
13/13 versus legacy and 11/13 versus copy/v86, with Boot and Compile open.

R097 next tested reuse of duplicate dispatch generation/index loads before a
tail transfer. A deterministic same-Wasm corpus found a large Liftoff-only win
of 1.649x `[1.525,1.743]`, but ordinary tiered V8 tied at 0.998x
`[0.992,1.005]`. The frozen opportunity gate therefore stopped before product
implementation. This closes local metadata reuse and indicates TurboFan already
eliminates those redundant loads. R085-equivalent product source and the 11/13
official score were unchanged.

R098 then removed a different architecture-wide interpreter cost without
changing interrupt timing.  Exact emitted shape proved that all nine concrete
drivers replaced their ordinary countdown decrement/store with a modular
retired-count comparison and retained the rare arm store and call topology.
The complete strict correctness matrix passed.  Five immutable-artifact pairs
measured Boot 1.020x `[0.988,1.032]`, Compile 1.008x `[0.974,1.089]`, and
Python 0.948x `[0.930,1.053]`, with 1.014x host spread and exact outputs.

This is a valid native-gate rejection: Boot misses the standing 1.03x
cumulative rule and Python exceeds the 3% elapsed guard.  Candidate size was
not a gate (it happened to be 48 bytes smaller).  No browser, WANIX, or full
scorecard run followed.  The candidate is archived under
`target/bench/r098-interrupt-deadline/`, all live CPU/test code is removed, and
the release rebuild is byte-exact R085-equivalent `d9f686a9...`.  Official
status remains 13/13 versus legacy and 11/13 versus copy/v86; Boot and Compile
are still open.

R099 then audited production scheduler activity rather than inferring it from
source shape. Every region-exit sampling, extension, drain, demotion, batching,
and indirect-cache-extension counter is zero in exact modern Boot and Compile,
while the outer dispatcher still runs hundreds of thousands of times. This
closes those dormant policy bodies as a parity target without changing the
runtime or official score.

R100 independently retested interleaved `{tag, offset}` fused-TLB entries under
the current 3% cumulative policy. Paired shape, ABI, focused correctness,
direct/OpenSBI Linux, and seven-pair cold-construction gates pass. The immutable
candidate is `c36da489...`; cold construction is 1.003x control and the
candidate happens to be 608 bytes smaller, neither a promotion nor rejection
criterion.

The valid 30-leg native report (`e4ea8ac0...`, host spread 1.015x) measures
Boot 0.989x `[0.956,1.018]`, Compile 1.017x `[0.966,1.083]`, and Python
0.986x `[0.961,1.075]`. Compile normalized MIPS agrees at 1.017x, but the
candidate misses both its 1.03 median and 1.00 lower-bound gates. R100 therefore
stops before Chrome, WANIX, or the full scorecard, with no encoding variants.
All candidate source and the release build are restored exactly to
`d9f686a9...`; restored core/DBT units pass 32/32 and 53/53. Official status
remains 13/13 legacy and 11/13 v86, with Boot and Compile open.

R101 then counted an architecture-general structured-control opportunity.
Compile STEADY entered 39,972,369 structured members; a conservative bounded
plan could omit 14,873,571 post-member comparisons, representing 59,494,284
emitted Wasm operators, without exceeding the existing 127-instruction fuel
overshoot. Boot exposure was only 9.219%. This justified one frozen R102
implementation; the profile elapsed time was never used as performance proof.

R102 passed exhaustive bounded-fuel execution, the complete strict correctness
matrix, modern direct/OpenSBI boots, and fresh-process construction at 1.018x
candidate/control despite adding 14,623 module bytes. The valid 42-leg native
gate (host spread 1.071x) measured Boot 1.007x `[0.987,1.036]`, Compile 0.997x
`[0.978,1.083]`, and Python 1.016x `[0.999,1.033]`; Compile normalized MIPS was
also 0.997x. The 59.5 million removed operators did not convert to a measurable
whole-row gain. R102 is rejected before browser/WANIX/scorecard escalation,
all live candidate code is removed, and the release build is byte-exact
`d9f686a9...` with 53/53 restored DBT units passing. Official status remains
13/13 legacy and 11/13 v86; Boot and Compile remain open.

R103 next measured the state-materialization boundary behind Compile's 8.56
million cross-module tail transfers. Proof-only counters close exactly:
9,092,297 STEADY generated invocations equal 533,462 outer calls plus 8,558,835
chain hops. They execute 364,282,880 GPR boundary operations; even charging the
maximum 62 to every outer call leaves a rigorous 331,208,236 chain-attributable
operations. The diagnostic output and all identities pass, and no diagnostic
elapsed time is used.

The frozen architecture-defined x1--x31 + PC/retirement/fuel model then failed
before product work. Both two-instance variants reached Liftoff and TurboFan
and produced identical state, but memory/carried medians were 19.3977/19.4303
ns per hop, paired speedup 0.9989x `[0.9861,1.0533]`. Control-side spread was
also 1.1876x against the 1.10 limit, so the run fails stability, positive-effect,
and projected-3% requirements without a retry. R103 diagnostic code is removed
and release Wasm is exact `d9f686a9...`. The official 13/13 legacy, 11/13 v86
status is unchanged; Boot and Compile remain open.

R104 prospectively lowers the economic floor from 3% to a verified 1%, while
retaining confidence, normalized-work, protected-row, browser, WANIX, and full
scorecard gates. Code size is now explicitly diagnostic only. The historical
audit admitted one fresh R105 reconstruction of the exact R093 integrated
scalar Tier-0 against the current baseline, without pooling any old timing.

R105 passed exhaustive candidate-on correctness, direct/OpenSBI Linux 6.12.7,
and cold construction. Its immutable 42-leg same-Wasm report (`1ee0190a3521...`)
is valid with `1.0212x` host spread: Boot improves `1.0588x`
`[1.0360,1.0834]` and normalized MIPS agrees at `1.0589x`, but Compile is
`0.9803x` `[0.9551,1.0079]` and Python is `0.9792x`
`[0.9600,0.9939]`. The candidate therefore fails both protected medians, with
Python establishing a regression. R105 stops before product/Chrome/WANIX/full
scorecard work and is cleanly removed. Core/Wasm source and release identities
are restored to `aec4b31434a6...`, `1da35e70bc9c...`, and `d9f686a9...`;
official status remains 13/13 legacy and 11/13 v86, with Boot and Compile open.

R106 prospectively tested the architecture-wide composition suggested by R105
and R064 counters. Exact source/shape proofs and the complete candidate-on
correctness matrix passed, including direct/OpenSBI Linux 6.12.7. The immutable
candidate (`6571795ef19c...`) then measured `1.0511625x` cold construction
against the frozen `1.0500000x` maximum and stopped before native workload
timing. Evidence is archived under
`target/bench/r106-balanced-scalar-publication/`; both mechanisms are removed
and core/Wasm/release identities are restored exactly to `aec4b31434a6...`,
`1da35e70bc9c...`, and `d9f686a9ce4f...`. Restored units, raw Wasm,
public/Worker APIs, and direct/OpenSBI boots pass. Official status remains
13/13 versus legacy and 11/13 versus copy/v86, with Boot and Compile open.

R107 now replaces future knife-edge percentage construction limits with
conservative absolute-cost accounting. Fifteen fresh exact-baseline
`RV64Debug.create` pairs measure 20.629/20.518 ms; the paired median delta is
-0.323 ms with interval `[-2.273,0.155]`, so the calibrated false debit is only
0.155 ms. A future candidate's upper-confidence construction delta is added to
each candidate Boot/Compile/Python sample before the verified 1% decision.
Generated JIT-module construction remains inside workload timing, and browser
survivors must pass a direct construction-to-marker clock. This policy earns no
parity credit and does not reopen R106. Exact `d9f686a9...` remains active;
official status is still 13/13 versus legacy and 11/13 versus v86.

R108 then re-attributed the corrected-cadence DBT frontend. Structured CFG and
ordered-tree closure owns 2.316% of Boot and 1.139% of Compile STEADY, while
generic instruction-enum encoding alone is below the 1% whole-row floor. The
typed-sink model is therefore closed standalone, and one prospectively frozen
R109 dense-CFG experiment was admitted without composition.

R109 retained all 133 production Boot/Compile CFGs before implementing one
fixed dense bit-matrix stackifier. The seven-pair ordinary-V8 model passed
decisively: Boot first/steady were 5.264x/6.628x and Compile first/steady were
5.052x/5.874x. All 14,931 production/exhaustive/random structure serializations
matched exactly, the full correctness matrix passed, and 280 deterministic
real-RV64 generated modules were byte-identical. Candidate size was 48,578
bytes smaller but remained diagnostic only.

The valid 15-pair R107 construction measurement produced a 1.041935 ms debit.
The complete valid 90-leg native gate (host spread 1.074x) then measured
debit-adjusted Boot 1.01522x `[0.99726,1.02755]`, Compile 0.98926x
`[0.96237,1.01259]`, and Python 1.00453x `[0.98732,1.02888]`; Boot normalized
MIPS agreed at 1.01507x. Boot misses the required 1.00 lower bound and Compile
misses the protected 0.99 median. R109 is rejected without extra samples,
variant, browser, WANIX, or scorecard work. Candidate code is archived and
removed; release Wasm is byte-exact `d9f686a9...`, core/Wasm source and loader
are restored to `aec4b314...`, `1da35e70...`, and `2cbb264f...`. Official
status remains 13/13 legacy and 11/13 v86, with Boot and Compile open.

R110 then sampled the actual optimized native code produced for the unchanged
modern Compile worker. The single diagnostic run is authentic and correct but
its perf/JIT-logging durations are excluded. A validated direct JIT-dump reader
maps 4,506 cycle samples to the low-index JIT path after repairing two malformed
Node/V8 debug-record lengths; its final report separates the 128-byte shared
tail trampoline from guest bodies and supersedes two preserved classifier
drafts without recollecting samples.

TurboFan owns 91.76% of the mapped period. Explicit native-stack reads and
writes own 22.44% of guest-body cycles; sampled TurboFan bodies reserve a
period-weighted 324-byte frame, and bodies above 512 bytes spend 31.02% of
their cycles on explicit stack traffic. Combining the guest-stack fraction of
the JIT path with R088's independent generated-execution share gives an 8.87%
whole-Compile exposure ceiling, not a claimed gain. This clears the frozen
opportunity floor for one proof-only R111 multi-function-partition model. No
product code or official score changes: exact `d9f686a9...` remains active,
the score is 13/13 versus legacy and 11/13 versus copy/v86, and Boot/Compile
remain open.

R111 then applied one prospectively frozen SCC/32-member/24-state
same-module partition rule to all 133 production CFGs and the unchanged 56
compiler-produced real regions. The local-pressure premise was real: 91.41%
of eager bytes were eligible to split, the byte-weighted state ratio fell to
62.90%, and the estimated maximum-local footprint fell 16.06%. The rule failed
its boundary and atomicity guards decisively, however: it cut 52.88% of Boot
and 42.68% of Compile static edges, versus a 12.5% limit, while oversized SCCs
occupied 61.00% of eligible bytes, versus 20%.

R111 is therefore closed at Gate A without building or timing its ordinary-V8
model and without a product implementation. Duplicate corpus generations and
three analyzer invocations are byte-identical; evidence is under
`target/bench/r111-partition-model/`. The diagnostic shape API remains
behind the non-default `r111-diagnostics` feature and unreachable from the
product. No cap/order/state/ABI variant follows from the result. Product Wasm
rebuilds byte-exact `d9f686a9...`; official status remains
13/13 versus legacy and 11/13 versus copy/v86, with Boot and Compile open.

R112 next tested whether the preserved R110 jitdump could map the measured
native spill traffic back to Wasm source positions without recollecting a
profile. Its final deterministic parser validates all 250 debug records / 6,007
entries, including V8's zero alignment padding and the existing `-6`/`-5`
length repairs, and reproduces every R110 sample/tier/role/family total exactly.
However, all debug records belong to JavaScript or Node internals. None belongs
to a sampled generated Wasm load, so both guest-body and explicit-stack source
coverage are 0%.

R112 is closed without a follow-on module/operator capture, another perf run,
or an engine/debug-flag variant. Product source and Wasm are untouched; release
Wasm remains `d9f686a9...`, official status remains 13/13 versus legacy and
11/13 versus copy/v86, and Boot/Compile remain open.

R113 then classified every preserved TurboFan guest stack sample by native
form and control context. All 897 samples / 1.959 billion period close exactly.
Call neighborhoods own zero stack period; 53.05% is general-body pressure,
32.66% is branch-adjacent, and 14.29% is in the entry prefix. Register reloads
are the largest form, with a 4.196% optimistic whole-Compile exposure spread
over 70 loads, but have 464 samples against the frozen 500-sample admission
minimum. No other form reaches both the 2% exposure and evidence floors.

R113 therefore admits no form-specific model and makes no product change. This
is not rejection of a measured small gain: there is no candidate or timing,
and exposure is only an upper bound. Three reports are byte-identical; evidence
is under `target/bench/r113-native-stack-context/`. Product remains exact
`d9f686a9...`; official status remains 13/13 versus legacy and 11/13 versus
copy/v86, with Boot and Compile open.

R114 then audited the most credible independently reconstructable experiment
stopped by the old coarse percentage gate: R014's lazy architectural-PC
materialization, without its already-retested fuel/safepoint component. The
candidate passed full correctness, direct/OpenSBI modern Linux, and exact
real-region shape/ABI proofs. Its 56 structured corpus pairs converted exactly
8,542 safety branches and added 33,860 aggregate generated bytes; size was
diagnostic only.

Fifteen fresh construction pairs produced a conservative 1.432463 ms R107
debit. The valid 90-leg native report (host spread 1.067722x) then measured
debit-adjusted Boot `0.99474x` `[0.97790,1.00430]`, Compile `0.98579x`
`[0.95202,0.99629]`, and Python `1.00438x` `[0.99406,1.02019]`.
Compile normalized work agrees at `0.98577x`, and control/candidate generated
coverage medians are closely matched. This is an established Compile
regression, not a small favorable result rejected by percentage or bytes.

R114 stops before clean-product, Chromium, WANIX, and scorecard gates. All
candidate/proof plumbing is removed. Restored DBT, public API, raw-Wasm, T2,
and direct/OpenSBI Linux tests pass; CPU/runtime/loader/release identities are
exactly `aec4b314...`, `1da35e70...`, `2cbb264f...`, and `d9f686a9...`.
Official status remains 13/13 versus legacy and 11/13 versus copy/v86; Boot
and Compile remain open. The prospective acceptance floor remains a verified
net 1%, not 10% or 20%.

R115 then completed one final causal audit of R095 without changing the live
product. Relocating the exact external Tier-0 into the same main Wasm instance
recovers a verified `1.03413x` Boot versus the external-instance form
(`[1.01796,1.04451]`), proving that instance switching was a material old
penalty. The stricter identical-artifact enabled/disabled comparison rejects
the executor itself: Boot is `0.97987x` with interval
`[0.95574,0.99572]`. Compile is unresolved at `0.99715x`, and Python is
unresolved at `1.00955x`; a Compile spread violation also prevents positive
use of that report.

No current source or product artifact was modified. R115 stops before product,
browser, WANIX, or scorecard work; evidence is under
`target/bench/r115-same-instance-proof/`. Exact `d9f686a9...`, the 13/13 and
11/13 official scores, and the open Boot/Compile parity gaps remain unchanged.

R116 then tested the safest within-function response to R110's distributed
register pressure: materialize only state referenced by one acyclic member.
The deterministic static census passes breadth and safety, but removes only
8.28% of weighted architectural state and 2.04% of total locals. Those miss
the frozen 20%/5% opportunity floors, so no backend mode or timing was built.

The live product remains exact `d9f686a9...`; official status remains 13/13
versus legacy and 11/13 versus copy/v86. Boot and Compile remain open.

R117 then tested whether private module globals could serve as cheaper spill
storage without adding a CFG or function boundary. Its deterministic modules
have byte-identical normalized 963-operator bodies, exact indexed state access,
and exact output. All 15 alternating CPU-pinned pairs are valid and stable.
The global candidate is slower: steady speedup is `0.970330x`
`[0.967047,0.973196]`, and first-execution speedup is `0.826972x`.

R117 stops at its frozen model gate without native capture or product code.
The result closes whole-state private globals and any post-result partial-width
or register-selected variant. Product identities and official scores are
unchanged: release Wasm remains `d9f686a9...`, 13/13 versus legacy and 11/13
versus copy/v86, with Boot and Compile parity still open.

R118 then completed the broader historical small-gain audit by reconstructing
R059's complete flat RV64C selector on the exact current product. This was a
legitimate retest because R059 had mixed a tier-publication warmup event into
its old steady-state stability gate. R118 passed deterministic shape,
exhaustive 65,536-encoding RVC equivalence, the full 134/109/193 strict suite,
and direct/OpenSBI Linux 6.12.7.

The native result is negative. After a 1.225862 ms construction debit, Boot is
`0.982183x [0.968403,0.993110]`, with normalized work `0.982098x`.
Compile/Python points are `0.996783x`/`1.022471x`; one Compile control sample
breaches the frozen spread limit. Boot is independently stable and establishes
regression, so no rerun or browser/WANIX/scorecard escalation is permitted.

The candidate is removed and CPU/runtime/loader/release are restored exactly
to `aec4b314...`, `1da35e70...`, `2cbb264f...`, and `d9f686a9...`. Official
status remains 13/13 versus legacy and 11/13 versus copy/v86; Boot and Compile
parity remain open. The active policy accepts any verified net gain of at least
1% with protected rows intact and never rejects on code-size growth alone.

The subsequent full-ledger audit corrected one overbroad statement in the
rewrite-only audit. Two deleted legacy-backend experiments, E005b and E006b,
were stopped after favorable one-pair Compile points of 4.5% and 2.6%
respectively because the old rule demanded 10%. They deserved more evidence at
the time, but they do not qualify for retroactive acceptance: neither has a
confidence interval or protected product rows, and their exact cache/tier paths
do not exist in the production rewrite page policy. They remain leads that may
justify a new current-baseline experiment only if current attribution proves an
equivalent active cost. No product code or official score changes.

R119 then profiled a different active cache-path cost and implemented one
architecture-general candidate: after the existing execute-TLB tag proof, use
the same row's stable direct-RAM capability rather than repeat the physical-bus
range/bounds path. Full strict correctness, direct/OpenSBI Linux, deterministic
Wasm, and native one-probe shape gates pass.

The fixed native gate is a near miss but not admissible. With a 1.258935 ms
construction debit, Boot is `1.012411x [0.997859,1.015302]`, Compile is
`0.984634x [0.947863,1.016480]`, and Python is
`1.001649x [0.981848,1.029909]`. The valid 90-leg report has no problems and
host spread 1.067733x. Boot misses the required parity lower bound and Compile
misses the protected 0.99 median, so R119 stops before browser, WANIX, and the
scorecard.

Candidate evidence remains archived; live CPU source and release Wasm are
restored exactly to `aec4b314...` and `d9f686a9...`. Official status remains
13/13 versus legacy and 11/13 versus copy/v86, with Boot and Compile parity
open. The verified-one-percent policy remains active; neither the candidate's
1.24% point nor its 3,187-byte growth was used as an automatic rejection.

R120 then reopened the exact R100 interleaved fused-TLB artifact because its
old five-pair Compile point was `1.017x` and the former rule required 3%. The
candidate and decision were frozen before timing; two isolated builds reproduce
exact `c36da489...`, and no old sample was pooled.

The valid 90-leg reconfirmation does not reproduce the gain. After a 0.231087
ms construction debit, Compile is `0.992069x [0.952178,1.015084]` with
normalized work `0.992040x`; Python is `0.982841x
[0.954431,1.033982]`; Boot is `1.003286x [1.001075,1.013737]`. Compile fails
all target conditions and Python misses its protected median, so R120 stops at
the frozen native gate.

No product source was reapplied. Live CPU/runtime/loader/release identities
remain exact `aec4b314...`, `1da35e70...`, `2cbb264f...`, and `d9f686a9...`.
Official status remains 13/13 versus legacy and 11/13 versus copy/v86; Boot
and Compile parity remain open. The policy is unchanged: accept reproducible,
correctness-safe net gains >=1%, not every noisy point estimate above 1%.

R121 then attributed the next active Compile scheduler cost without timing a
product candidate. Its deterministic native census closes all preserved R110
period and assigns 0.7481% of main-thread cycles to the complete authoritative
cache/mapping/refill fallback. A sole instrumented modern Compile run closes
every cause counter: STEADY fallbacks are 61.21% empty-line absence, 37.45%
stale generation, 1.31% collision, and 0.03% unverified publication. Only one
of 213,808 compiled-block mapping proofs fails.

The population is real but the removable exposure is not credibly above the
verified net 1% floor. Negative absence lines and parallel stale-generation
metadata are therefore closed before product implementation or timing. The
diagnostic build and reports are archived, all hot-path counters are removed,
and live runtime/release identities are exact `1da35e70...` / `d9f686a9...`.
The harness now records already-exported MMU counters outside timed intervals.
Official status remains 13/13 versus legacy and 11/13 versus copy/v86, with
Boot and Compile open.

R122 then returned to the larger exact Boot interpreter profile and closed its
semantic body without timing a product candidate. The immutable native census
assigns 16.1968% of main-thread period to compressed semantics and 14.0594%
to RV32 semantics, but every basic block above the frozen 1.25% admission floor
is required dense dispatch, an R118-closed compressed selector, accepted R054
memory work, or a mixed decode whose removable subset has only a 0.4216%
optimistic main-thread ceiling.

One measurement-ineligible modern Linux 6.12.7 / Alpine 3.24.1 counter run
closes 108,693,790 interpreted plus 71,704,074 generated retirements against
180,397,864 total guest instructions. Only 1.0958% of 66,630,535 common GPR
writes discard x0, and neither available branchless representation reduces
general work. Opcode/family frequency did not select a candidate. Instrumented
source, Wasm, and reports are authenticated under
`target/bench/r122-interpreter-body/`; all counters are removed and live
CPU/Wasm/loader/release identities are exact `aec4b314...`, `1da35e70...`,
`2cbb264f...`, and `d9f686a9...`.

Official status remains 13/13 versus legacy and 11/13 versus copy/v86. Boot
and Compile remain the two parity gaps. The verified-one-percent policy remains
active; R122 is an exposure closure, not rejection of a small measured gain.

R122's native report was then reissued after its objdump parser was found to
accept byte-only continuation lines as instructions. The corrected analyzer
and two byte-identical final reports are authenticated at `35a7b4fc...` and
`40f70f2e...`. Decoded-instruction count changes from 4,311 to 4,228; every
symbol/basic-block boundary, sampled period, exposure band, hot block, and
R122 decision remains unchanged.

R123 attributed the remaining production enable check inside accepted R054
fused scalar memory. All five optimized `ld`/`st` bodies own 7.0933% of Boot
main-thread period, but the complete flag-guard blocks own only 0.104231%.
Even impossible elimination projects `1.001043x` whole Boot; the broad `ld1`
bound already overcredits an unrelated spill.

No compile-time Bus specialization was built or timed. Evidence is archived
under `target/bench/r123-fused-memory-static-guard/`, exact product
`d9f686a9...` remains live, and Boot/Compile parity remain open. Continue with
optimized generated-region state pressure, whose R110 native exposure is
materially larger, while retaining the verified net 1% product rule.

The historical ledger and current admission rules have now been audited for
small-gain loss. Old coarse floors did discard favorable observations, but
fresh current-product reconstructions do not identify an artifact to restore:
R114/R118/R120 are negative and R119 is unresolved with a protected-row miss.
The prospective result is nevertheless important. R104 now explicitly bars
5--20% static/model proxy thresholds from vetoing a practical candidate while
a verified 1% end-to-end gain remains plausible.

R124's architecture-fixed x1/x2/x8--x15 hybrid state design passed its original
A1 static gate with a 13.2938% eager-byte-weighted local reduction. Before any
A2 execution or timing, its larger A2/B proxy targets were converted to
reported diagnostics rather than final vetoes. Correctness, construction,
confidence, protected Boot/Python, Chromium, WANIX `/shared/bench.py`, and the
untouched three-way scorecard remain mandatory; no R124 performance credit or
product change exists yet.

R124 A2 subsequently passed with projected hybrid/current GPR-operation ratios
of `0.739376`, `0.737961`, and `0.737698` for FIRST, PRIME, and STEADY.  Its
sole frozen ordinary-V8 model measured `0.966035x` steady, but post-result
inspection found the model put 65,536 member rounds inside one invocation.  It
therefore charged hybrid boundary traffic approximately 88,791 times relative
to eager instead of reproducing product invocation boundaries.  The raw result
is archived unchanged and receives neither product credit nor veto.

No post-result synthetic rerun will be made.  The one fixed-bank product
implementation is now in progress.  It must next pass directed and broad
correctness, exact construction-debited native Compile measurement, protected
Boot/Python rows, Chromium, WANIX `/shared/bench.py`, and the untouched modern
three-way scorecard under the verified net 1% rule.

The exact R124 product candidate is now `d017a10f...`; two isolated builds are
byte-identical.  All 177 workspace tests, broad integer/M/A/FP/system/T2
differentials, public/Worker/raw-Wasm gates, and direct/OpenSBI modern Linux
boots pass.  The candidate is 2,406 Wasm bytes larger, recorded only as a fact.

R124 then passed its frozen native product gate.  After a `0.168840 ms`
construction debit, Boot is `1.018471x [1.000973,1.035481]`, Compile is
`1.083675x [1.037357,1.112250]` with normalized work `1.083602x`, and Python is
`1.200538x [1.180720,1.220097]`.  All target, protected-row, identity, output,
work, cadence, policy, coverage, affinity, and host checks pass.

The unchanged candidate now advances to natural Chromium, R094-qualified
WANIX `/shared/bench.py`, and the untouched modern three-way scorecard.  No
additional mechanism or register-bank variant may be composed into R124.

R124 has now passed the frozen natural-Chromium dual-clock guard without a
retry. Seven fresh alternating pairs give execution-only Boot
`1.018971x [1.001431,1.037301]` and construction-to-ready
`1.017013x [0.998068,1.029593]`. Both protected clocks satisfy their `0.99x`
median and no-established-regression rules; all artifact, guest, policy,
output, retirement, cadence, browser, affinity, and timing-boundary proofs
pass.

The exact `d017a10f...` candidate remains unchanged. Its next gate is frozen
before sampling: seven alternating fresh WANIX browser pairs, three repetitions
of the unchanged Python and SHA workloads plus the R094-qualified 32 MiB
shared-9P phase, with active-JIT, exact-work, policy, byte, confidence, and
one-percent protected-row proofs. The untouched 117-trial scorecard remains
pending after WANIX.

The original R124 WANIX run is now invalid, not a performance failure. Pair 5
exact control stalled after correct shell, Python, and SHA-256 work while one
external-9P `T_UNLINKAT` response remained outstanding. No leg was replaced,
no partial candidate/control ratio was computed, and the untouched scorecard
was not run. Exact live product remains control `d9f686a9...`; candidate
`d017a10f...` remains archived unchanged.

The common integration defect is corrected at the WANIX adapter boundary by
FIFO single-flight delivery into its stream-backed Go 9P endpoint. Generic
loader concurrency remains enabled. Directed transport tests, the existing
concurrent-loader test, two byte-identical ordinary adapter builds, six fresh
prototype transfers, and six fresh actual-adapter transfers all pass; every
actual transfer completes the exact 32 MiB work without a stall.

R125 is frozen from zero with fresh pages, archives, tools, and an empty result
directory. It uses the corrected adapter identically for control and candidate
and preserves the seven-pair/three-repetition WANIX decision rules. The next
action is its sole formal run. A valid pass advances unchanged `d017a10f...` to
the untouched 117-trial scorecard; a valid protected-row failure rejects it;
another invalid run stops for diagnosis.

R125's sole formal run is complete and valid. All fourteen fresh-browser legs,
42 Python samples, 42 SHA-256 samples, and 42 exact 32 MiB shared-9P samples
finish with zero stderr and every frozen integrity proof. The corrected WANIX
adapter is therefore fully qualified; the R124 transport stall does not recur.

The exact candidate improves Python `1.079692x [1.070039,1.111911]`, SHA-256
`1.021724x [1.018486,1.037770]`, and shared 9P `1.009359x
[1.003915,1.016363]`. Shell measures `0.996257x [0.989513,0.998082]`. Although
that is only a 0.374% median slowdown and remains above `0.99x`, its confidence
upper endpoint is below parity and fails the frozen no-established-regression
rule.

R124 is therefore rejected under D125/D126, exact `d017a10f...` remains
archived, live release product remains `d9f686a9...`, and the untouched
scorecard was not run. The result raises a prospective policy question rather
than an excuse for a post-result waiver: use `0.99x` itself as the material
protected-row boundary, or predeclare an aggregate workload weighting. Until
that is settled, official status remains 13/13 versus legacy and 11/13 versus
copy/v86, with Boot and Compile open.

The owner has now resolved that policy question in favor of the explicit
one-percent material boundary. R125's raw `gatePassed=false` report is
unchanged, but its 0.374% WANIX shell slowdown no longer vetoes escalation:
the protected median is above `0.99x`, unchanged Python improves 7.97%, and
every integration proof passes. R125 is accepted as WANIX qualification, not
as final product promotion.

Exact archived R124 candidate `d017a10f...` now advances unchanged to one
untouched authoritative 117-trial modern scorecard. Before its first sample,
freeze the current candidate, Linux 6.12.7 / Alpine 3.24.1 inputs, isolated
legacy comparator, pinned copy/v86, scorecard tools, accepted R087 comparator,
and an independent adjudicator. Live release remains `d9f686a9...` until that
scorecard passes.

R126 attempted that matrix but is wholly measurement-invalid. Candidate
selection used the worker's diagnostic-only Wasm override, making all 78 RV64
results ineligible. The matched x86 kernel was also absent and omitted from
top-level preflight, so all 39 copy/v86 workers exited before results. The
report has zero eligible trials; no printed median or parity value is used.

Preserve invalid report `ac096fec...` and collect no replacement leg from it.
The next step is a prospectively frozen complete replacement: apply exact
archived R124 source as the ordinary release, reproduce `d017a10f...`, restore
exact R087 x86 kernel `8854efec...`, add missing required-input preflight, use
no diagnostic override, and rerun all 117 trials from zero. Official accepted
status remains 13/13 legacy and 11/13 copy/v86 until that succeeds.

R127 applied those repairs and passed both top-level preflight and real v86
generated dispatch, but its interactive execution session was externally
aborted during the Python row. The parent vanished without a report, benchmark
error, or formal output directory. R127 is wholly invalid and supplies no
performance result; none of its launched processes will be reused.

The immediate action is R128: freeze the unchanged candidate, inputs, tools,
and incremental gate again, then rerun all 117 trials from zero with the parent
detached from the interactive tool session and raw stdout/stderr persisted.
Official accepted parity remains 13/13 legacy and 11/13 copy/v86 while that
replacement is pending.

The owner has paused the effort during R128. Its detached launch and repaired
admission worked, but it was intentionally terminated in Python after ALU,
Mixed, and Boot orchestration completed. No formal report or runner exit record
exists; R128 is incomplete and supplies no performance verdict. All R128
processes are stopped and its raw logs are preserved.

Exact candidate `d017a10f...` remains applied only in the uncommitted
experimental worktree/release; it is not scorecard-promoted or committed.
Official evidence at the pause remains 13/13 versus legacy and 11/13 versus
copy/v86, with Boot and Compile still open. R125 remains accepted only as
WANIX qualification under the clarified one-percent materiality policy.

## RV64GCV direct SIMD follow-up (2026-08-13)

The separate RV64GCV effort now has architecture-general direct JIT lowering
for broad integer lane operations, unit and guarded strided memory, whole-
register transfers, scalar moves, slides/gather, packed comparisons, and
flag-free FP bit/move permutations. Exact helper fallback remains for every
guard miss and for FP arithmetic, reductions, masks/restarts, fault candidates,
and other non-representable families. No PC, symbol, binary, input, or
benchmark recognizer exists.

The strict release suite passes, including 8,724 QEMU comparisons and 8,724
hot interpreter/JIT comparisons. The authoritative RV64GCV scorecard is valid
and remains 8/13, while String Sort improves `3.4587x`, FP Emulation `1.7970x`,
and Assignment `1.2915x` versus helper-only lowering. The unchanged scalar
population was rerun separately and remains exactly 11/13 against copy/v86,
with Boot and Compile as its only losses. See `RVV_JIT_RESULT.md` for hashes,
tables, and retained evidence. RV64GCV parity remains open.
