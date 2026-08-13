# RV64 WebAssembly JIT Rewrite Plan

Status: implementation complete; parity follow-up concluded incomplete  
Started: 2026-08-05  
Completed: 2026-08-05  
Clean-room baseline: `4b0896decdff7538f9c1d2b44dc19a1d3d14f7c2`

External research and source links: [RESEARCH.md](RESEARCH.md)

Terminal note (2026-08-10): the owner concluded the later copy/v86 parity
exercise without a valid final-candidate three-way scorecard. The clean-room
implementation remains complete, but the separate parity objective remains
unachieved at 13/13 versus legacy and 11/13 versus copy/v86. See
[FINAL_EXERCISE_REPORT.md](FINAL_EXERCISE_REPORT.md). No continuation is
scheduled.

## Objective

Replace the deleted `rv64-jit` crate with a new, independently designed,
WebAssembly-only dynamic binary translator. Preserve the emulator's RV64
correctness and public behavior while optimizing both time-to-useful-execution
and steady-state performance in browser WebAssembly engines.

The old compiler implementation is not a design input. It remains recoverable
from Git history solely for provenance. The rewrite may use the interpreter,
architectural state, decoder, memory/device contracts, tests, and external host
ABI as specifications.

## Success criteria

1. The active tree contains no source from the former `rv64-jit` crate.
2. Interpreter-only native and Wasm builds remain usable throughout the work.
3. Compiled execution is differential-test equivalent to the interpreter for
   registers, memory, PC, retirement accounting, traps, and invalidation.
4. Generated modules validate and execute in the supported browser engines and
   a standalone Wasm runtime used for diagnostics.
5. Benchmarks separately report translation, Wasm validation/compilation,
   instantiation/publication, first execution, tier-up, steady-state execution,
   invalidation, and memory/code-cache use.
6. The default policy is chosen from reproducible cross-engine data, not one
   engine's diagnostic flags or a pre-warmed harness.

## Architecture

### Execution tiers

- T0: the existing correctness-first interpreter. It gathers low-overhead PC,
  edge, indirect-target, page, and exit profiles.
- T1: quickly compiled bounded regions. T1 performs local state forwarding and
  cheap simplification, and prioritizes low compilation latency.
- T2: profile-guided region re-formation for genuinely hot code. T2 may grow
  traces/superblocks, inline hot direct edges, specialize indirect branches,
  and hoist repeated memory checks. It uses the same IR and backend as T1.

### Compiler pipeline

1. Decode a bounded control-flow region from immutable code snapshots.
2. Lift instructions into typed, ordered SSA with explicit architectural,
   memory, trap, helper, and control effects.
3. Preserve an exact guest PC on every potentially faulting operation and side
   exit.
4. Run bounded-cost passes: constant/copy propagation, dead-code elimination,
   register-state forwarding, local common-subexpression elimination, branch
   folding, redundant extension removal, repeated-TLB-check elimination, and
   RV64 pattern fusion.
5. Form structured Wasm control flow and stackify values.
6. Emit related regions in batches, publish stable entry slots, and connect
   cross-batch edges without returning to JavaScript.

### Runtime invariants

- Guest registers stay in Wasm locals within a region and are materialized only
  when required by an exit, helper, observable operation, or publication
  boundary.
- Wasm traps are never used as guest exceptions. Division, memory, conversion,
  and indirect-call conditions that differ from RV64 are guarded explicitly.
- Common RAM accesses use an inline checked fast path. MMIO, permission faults,
  page crossings, uncommon alignment, and translation misses use precise slow
  paths.
- Compiled code is keyed by architectural context and code-page generations.
  Compilation results are revalidated before publication and entry.
- Execution is bounded so the host can service devices, interrupts, compilation
  completion, and UI/event-loop work.
- Generated code never crosses JavaScript on a normal hot edge.

## Work phases and gates

### Phase 0: clean-room reset and observability

- Remove the old crate wholesale.
- Inventory non-compiler integration without consulting old compiler source or
  historical tuning documents.
- Restore interpreter-only workspace, native tests, Wasm build, and smoke test.
- Add structured timing/counter records and exact environment manifests.

Gate: all applicable interpreter tests pass from a tree with no JIT crate.

### Phase 1: contracts and differential harness

- Define the CPU-state, memory, exit, invalidation, compilation, publication,
  and execution-budget interfaces.
- Add per-instruction and bounded-timeslice interpreter snapshots.
- Add a generated-module recorder/replayer using content hashes.

Gate: deterministic interpreter snapshots and replay manifests are stable.

### Phase 2: minimal T1 compiler

- Create the new compiler crate and typed IR.
- Lift RV64 integer ALU, immediate, comparison, and direct-control operations.
- Emit valid batched Wasm regions with precise exits and interpreter fallback.
- Keep the dispatcher inside Wasm for compiled-to-compiled edges.

Gate: randomized integer programs match the interpreter in every visible state
field, and compile/instantiate/execute accounting is separated.

### Phase 3: memory and full-system safety

- Implement loads/stores, address checks, inline translation fast paths,
  cross-page handling, MMIO/fault exits, code-page generations, `FENCE.I`, and
  context-keyed invalidation.

Gate: user and system memory differentials, fault tests, and self-modifying-code
tests pass without relying on host traps.

### Phase 4: coverage

- Add M, A, C, Zicsr, F, and D behavior in measured priority order.
- Use exact interpreter exits for uncompiled instructions until their direct
  lowering is proven.
- Add reservation, atomic-ordering, FP-state, rounding, flags, NaN-boxing, and
  privileged-state tests before enabling their fast paths.

Gate: supported workloads have complete behavioral parity; fallback remains
precise for every legal but not-yet-lowered operation.

### Phase 5: region quality and T2

- Add hot edge profiles, trace/superblock formation, side exits, direct-edge
  linking, polymorphic indirect-target caches, page-locality-aware batching,
  register liveness, lazy state materialization, and memory-check hoisting.
- Tune bounded region and batch-size policies per cross-engine results while
  retaining one portable default.

Gate: improvements survive randomized paired trials and do not regress cold
latency or tail pauses beyond documented tolerances.

### Phase 6: hardening and delivery

- Run architectural, randomized, Linux boot, syscall, FP, atomic, SMC, and
  cross-engine suites.
- Publish raw benchmark samples, module hashes, environment versions, confidence
  intervals, known limitations, and remaining optimization candidates.

Gate: the full repository verification matrix passes and results are
reproducible from documented commands.

## Benchmark regimes

- First visit: download/build inputs through first useful guest event.
- Cold guest JIT: empty translation cache in a fresh runtime process/profile.
- Component: decode/lift, optimize, emit, validate/compile, instantiate, publish.
- Lifecycle: first, worst, p50, p95, p99, break-even, and time-to-plateau.
- Steady state: translations frozen and host tiering deliberately stabilized.
- Frozen replay: identical generated bytes and logical publication order across
  engines, with translation removed from the measurement.

Primary workload metrics are wall time and useful guest work. Guest instruction
rate is diagnostic because instruction count is not architecture-independent.

## Autonomy policy

Implementation choices, experiments, reversions, and bounded refactors proceed
without per-step approval. Work pauses only for missing external inputs, an
irreversible action outside this stated scope, or a product choice that changes
architectural correctness or public compatibility.

## Current checkpoint

All six phases and their gates are complete on upstream commit `96aa938`. The
replacement has scalar `rv64gc` coverage, typed flat/full-system memory and
reservation capabilities, precise exits, T1 traces/loops, and T2
register-resident batches/regions with bounded indirect specialization and
multi-page invalidation.

Phase 5 evaluated eager, lazy, direct structured, materialized, and tail-call
state/dispatch shapes against identical frozen modules. Real compiler-generated
ELF regions cover seven page/leader geometries. Cross-engine execution and
frontend measurements initially selected eager register residency and bounded
regions. Subsequent matched-Linux work added CFG Stackifier lowering and refined
the system default to two pages, 512 leaders, and adaptive control-entry gating.
Experimental alternatives remain selectable and tested.

Phase 6 added 12-seed randomized T2 LR/SC/AMO fault stress, modern Linux 6.12
direct-SBI and OpenSBI boot gates, a standalone Wasmtime exact-execution gate,
fresh-process/profile Node/Chrome/Edge/Firefox measurement, raw samples, source and
module hashes, and bootstrap median confidence intervals. The strict repository
matrix passes all 164 Rust tests, 134 ISA tests, 109 Spike locksteps, 193
architecture signatures, directed/randomized Wasm differentials, both modern
boot paths, and the OpenSBI/Linux Virt smoke workload. See
[STATUS.md](STATUS.md) and [REPRODUCING.md](REPRODUCING.md) for the delivered
state and commands.

The separately requested post-checkpoint black-box comparison is also complete
on the same upstream commit. Earlier legacy-JIT microbenchmarks remain useful
backend diagnostics, while the matched product comparison now shows the
rewrite faster than copy/v86 on pure Python and SHA in both Chrome and Edge.
See [COMPARISON.md](COMPARISON.md) for the historical legacy comparison and
[STATUS.md](STATUS.md) for the matched copy/v86 result.

The first post-delivery optimization outline is now complete as well. It added
opt-in instruction/mapping traces, an offline admission simulator, deterministic
short/medium Linux payloads, a bounded hottest-first async page queue, exact
JIT-off calibration, process-isolated threshold/quantum sweeps, and a fresh
Chromium process/profile A/B. The stable loader now selects the matched-workload
131,072/q1024 policy with two in-flight builds, page cap two, leader cap 512,
the 100-per-thousand multi-page control gate, feature-tested frame-free region
tail chaining, and final-policy sampling bypass. Five alternating fresh Chrome
and five alternating fresh Edge RV64/v86 pairs, pinned to CPUs 8–15 and bound
to immutable artifacts, pass the 10% `/shared/bench.py` non-inferiority gate in
all phases. See
[COMPILE_POLICY.md](COMPILE_POLICY.md), [STATUS.md](STATUS.md), and
[REPRODUCING.md](REPRODUCING.md) for design, evidence, limits, and commands.

## Active parity milestone (2026-08-08)

The original implementation and bounded post-delivery optimization outline is
complete. The active performance objective remains copy/v86 parity on the
modern Linux 6.12.7 / Alpine 3.24.1 scorecard. The current authoritative
three-way report is
`target/bench/r054-final-three-way-rerun/scorecard-v2-2026-08-08T23-01-30-777Z.json`:
clean rewrite Wasm `4160333352b18b...` scores 11/13 against copy/v86, meets or
beats the isolated legacy comparator on all 13 rows, and has no measurement
problem. The only v86 losses are Matched Boot (2,260.5/1,525.8 ms) and Compile
(1,060.9/718.5 ms). The current five-pair Chrome `/shared/bench.py` guard also
passes decisively on Python, SHA-256, and shared 9P.

R054's architecture-wide fused interpreter-memory capability is the latest
promoted change. It reduced Boot 13.35% and Compile 4.73% versus the R043
accepted baseline without a guarded regression. R021's direct RV64C path is
the earlier promoted interpreter change. R022-R053 form a completed
rejection/attribution program: privileged
thresholds, forced inlining, decoded T0 caches, entry ranking, synchronous tiny
tiering, generated translation caches, TLB packing, SSA local allocation,
re-entry thinning, cold helper outlining, and redundant-load forwarding either
failed a correctness/performance gate or lacked enough dynamic opportunity.
Every candidate and temporary R041/R043 diagnostic field was removed before
the final artifact and scorecard.

The next plan is therefore a new architecture phase, not an extension of that
experiment series:

1. Freeze `4160333352b18b...` and the R054 reports as controls. Preserve
   `d93345139c5a...` and R044 only as the prior accepted historical baseline.
2. Split the remaining problem by lifecycle: privileged interpreter-dominated
   Boot versus generated-code-dominated Compile.
3. Admit a design only if static/dynamic attribution shows enough removable
   work to plausibly improve its target row by at least 40%; do not tune a
   closed mechanism until it crosses that opportunity gate.
4. Require a general architecture rule with no PC, symbol, workload, checksum,
   browser identity, or compiler-output exception.
5. Gate any implementation with exact differentials, balanced immutable A/B,
   no 10% regression on the other failing row, and the current browser guard.
6. Run the full authoritative 13-row three-way scorecard only after the focused
   candidate clears its predeclared 10% advancement gate.

R045 has now completed the first new leverage analysis. Exact profiles confirm
Boot's approximately 95% runtime-Wasm attribution and Compile's 92% generated
retirement. A physical-page trace admitted a decoded baseline tier, but direct
opcode dispatch and same-page chaining only reached a same-artifact timing tie;
that implementation was removed and the accepted artifact restored exactly.

R046 completed that bounded task and rejected the design. Although compact
one-page modules passed the offline coverage/size gate, two exact reverse-order
runtime screens regressed Boot 45.7%: 51-52 live module compilations displaced
only 13-14M extra interpreted instructions before readiness and accumulated
more than 6.7 seconds of competing V8 compile latency. Every experimental path
is removed and the release again exactly matches `d93345139c5a...`. Heat,
module geometry, synchronous tiny compilation, and concurrency are closed.

R047 completed the precompiled multi-instruction opportunity analysis and
closed it before implementation. Exact non-overlapping replay let a 256-triple
Boot library remove 40.81% of Boot dispatches, but the identical frozen library
removed only 31.20% on Compile and 24.78% on Python. This fails the required
cross-workload architecture gate; selecting new patterns from those failures
would be benchmark fitting. All diagnostic runtime code is removed and release
Wasm is again byte-identical to `d93345139c5a...`.

R048 completed that generated-engine attribution and the one admitted geometry
implementation. Rewrite's repeated 3.22 MiB function reaches TurboFan one phase
late, and exact capture identifies it as one relocated 512-entry physical page.
Entry-boundary splitting reduced it to about 79.8 KiB. After preserving all
recognized loop/dense/bulk lowerings, the fixed large-overlap form removed
57-62% of STEADY emitted bytes but tied Compile at 0.998x and Boot at 0.981x in
a valid alternating same-Wasm A/B. The candidate is removed and release Wasm
again exactly matches `d93345139c5a...`. Generated frontend volume, page alias
reuse, and late tier completion are now closed without a new causal result.

R049 completed the first dynamic-memory candidate after normalization. QEMU
proved that 99.9986% of dynamic `x2` writes are affine-immediate and all
82.47M stack-root accesses lie on the current stack page or its successor.
That broad invariant admitted a carried two-page load/store translation proof,
but the valid alternating same-Wasm A/B regressed Compile STEADY 41.1%
(1,052.18 to 1,484.94 ms). No tuning was performed. Every candidate path was
removed, the diagnostic was retained, all 84 relevant units pass, and release
Wasm again exactly matches `d93345139c5a...`. Stack translation caching is now
closed under the current generated-memory representation.

R050 completed stable optimized-tier component attribution. In STEADY,
generated module subtrees account for 43.86% of all CPU samples, residual
interpreter subtrees 34.20%, scheduler/cache work 11.86%, and synchronous
translation/issue 5.46%. PRIME reproduces the ordering. Exact retirement counts
show interpreted instructions costing 9.42-10.00x generated instructions in
the samples; only 7.64% residual interpreter retirement consumes more than one
third of CPU. Translation is closed as a standalone target because even its
impossible-elimination ceiling misses the 10% row gate.

R051 completed the proposed exact fallback-site partition, but its dominant
attempted-not-installed entry reproduced R032's result. R033 had already
removed that population with a general hot-entry ranking, and R034 had already
rejected it after Compile regressed 3.5% and Boot 1.4%. The transient R051
diagnostic is removed and production Wasm is again exact `d93345139c5a...`.

The closure audit is complete. R052 used frozen generated bytes and native V8
output to test the remaining simple generated-memory hypothesis: independent
load/store rows with one compressed `i64` Sv39 proof each. Exact full-system
correctness passed after the gates caught allocation and RAM-alignment bugs,
but seven paired fresh-process samples measured only 0.561x uncached and 0.885x
cached throughput versus the split proof. It was rejected without a scorecard,
encoding sweep, or relaxed correctness. Every candidate path is removed; the
accepted executable CODE bytes are restored, with only non-executable
Rust/LLVM name metadata differing after recompilation.

R053 completed the next architecture-general opportunity screen without
changing production. Although fixed-width pairing can remove 42.79-46.30% of
dispatches across three exact traces, an exhaustive 3,844-function pair tier
ran at only 0.879x single-handler throughput in seven alternating fresh
processes. It fails the preregistered 1.25x gate; handler subsets and popularity
sweeps are forbidden because they repeat R047's failed selection family.

R054 admitted a new mechanism from direct whole-row attribution rather than
reopening those closures. Scalar interpreter loads/stores accounted for
16.647% of accepted Boot CPU. Reusing the exact live fused JIT-TLB pointer
capability measured 3.030x on frozen memory bytes, then 1.151x Boot in exact
final-artifact A/B. The complete valid scorecard reduced Boot 13.35% and
Compile 4.73%, and the five-pair browser guard passed. The all-width,
unaligned, context, translated-memory, fallback, and invalidation tests are
part of the permanent correctness matrix. R054 is promoted as
`4160333352b18b...`.

R078-R080 subsequently discovered and removed disabled-path residue from the
rejected static-T0 series. R080 is now the promoted source-built baseline:
Wasm `e5415db83b27...`, valid 13/13 legacy and 11/13 v86 scorecard, complete
strict correctness, and a passing five-pair browser guard. Boot and Compile
remain the only v86 losses at approximately 1.50x and 1.47x slower.

R081 completed fresh R080 attribution. Boot is 93.4% runtime Wasm,
`Cpu::step` owns 50.3% of the complete phase, and generated Wasm owns only
4.9%. Compile STEADY divides 52.5% runtime/46.4% generated even though
generated retirement is 92.4%. V8 emits optimized TurboFan code for the
dominant interpreter function, ruling out a missing engine tier-up.

R082 then isolated the scalar compiler/emitter in a separate Wasm and proved
the active mechanism semantically, but its dormant-capable main artifact
failed the prospectively first non-inferiority gate: Boot's paired lower bound
was 0.967 against the frozen 0.970 floor. It stopped before active timing and
was removed. The rebuilt runtime and loader are byte-identical R080, so no
experimental path remains in production.

R083 tested R081's smaller alternative exactly: const-specialize full-system
interpreter fetch/load/store state and select the machine mode once. The
candidate was 22,957 bytes smaller and passed the complete strict matrix, but
the valid native gate measured only 0.888x Boot `[0.869,0.898]`, 0.963x
Compile, and 0.985x Python. It was rejected without variants or escalation and
removed; release artifacts are again byte-exact R080.

R084 then corrected the residual attribution to include hashing reached
through policy helpers. Default Rust hashing plus table-probe self time owns
6.586% of R080 Boot and 4.402% of Compile STEADY. A deterministic same-Wasm
corpus measured the selected randomized integer hasher at 5.508x raw hashing
and 3.021x representative state-map throughput, admitting R085 above the 3%
cumulative floor.

R085 is now the promoted source and product baseline. Its randomized private
integer-key builder changes only rv64-wasm JIT bookkeeping maps/sets; policy,
generated code, thresholds, geometry, and selectors are unchanged. The full
strict suite, cold construction, 30 native legs, 14 direct-Chrome boots, 14
WANIX browser/guest legs with 126 phase measurements, and all 117 authoritative
scorecard trials pass. Frozen Wasm is `efd7830307ef...`; production archive is
`0b953be67610...`. Against R080, authoritative Boot improves 1.041x, Compile
1.042x, and Python 1.017x. The score remains 13/13 versus legacy and 11/13
versus v86. Remaining raw gaps are 1.408x Boot and 1.378x Compile, equivalent
to 29.0% and 27.4% further rewrite-time reductions.

R086-R089 then completed the next residual cycle. R087 corrected the ordinary
scorecard scheduler to the public one-slice cadence and established a valid
117-trial baseline: 13/13 legacy and 11/13 v86, with only Boot and Compile
open. R088's corrected profile admitted exact re-entry monomorphization at a
1.0439x projected Boot gain. R089 proved the intended Wasm shape and complete
correctness, but its five-pair product gate measured Boot 0.972x
`[0.958,0.996]`; it was rejected and removed without variants. Exact R085 is
restored.

The remaining execution plan is:

1. Preserve exact R085 as product/control and R087 as the sole authoritative
   cadence baseline. Do not pool historical-cadence timings with new work.
2. Close exact callback monomorphization and adjacent generic/inlining variants;
   its local/profile projection was contradicted by valid whole-product timing.
3. Treat R090/R091 as closing scheduler representation and engine-tier shape:
   production feedback bookkeeping is dormant, while exact loop outlining
   successfully reaches TurboFan yet ties both target rows. Do not try adjacent
   helper, inlining, or boundary variants. Return to operation-level residual
   attribution: Cold Boot's 52% interpreter decode/execute population and
   Compile's generated-code plus unavoidable boundary work are the open axes.
4. Admit only a selector-free mechanism with both an exclusive whole-row
   ceiling and an independent local/counter test projecting at least 3%.
   Prefer mechanisms that reduce an existing dominant operation rather than
   add dormant code to the main Wasm.
5. Freeze one implementation at a time, run directed/full differentials before
   timing, then five alternating exact-R085/candidate Boot/Compile/Python pairs
   under the public cadence. Stop and restore at the first failed gate.
6. Require the same browser execution-Boot and `/shared/bench.py` product guard
   before the untouched corrected-cadence 117-trial scorecard.
7. Accumulate independently retained 3-5% gains until Boot and Compile satisfy
   the fixed copy/v86 parity rule. Current required elapsed-time reductions are
   28.16% Boot and 23.64% Compile.

R092 now closes the fixed whole-member range-versioning implementation under
that plan. It cleared native Compile by 13.2% and passed direct Chrome Boot,
but failed the frozen WANIX shared-9P guard at 0.810x `[0.741,0.894]` despite
correct output. Exact R085 is restored. Do not tune its access floor, privilege
mode, root, or cloned fallback after observing those rows.

The next cycle must therefore be independent of R092's selector family:

1. keep R085/R087 frozen as product and authoritative measurement controls;
2. use operation-level attribution on exact R085 to identify an exclusive,
   dynamically executed cost with at least a 3% whole-row ceiling;
3. require an independent shape/local-counter test before any product edit;
4. reject at the first correctness, Boot, Compile, Python, Chrome, or WANIX
   gate, retaining all evidence; and
5. run the 117-trial three-way scorecard only after every earlier gate passes.

R093--R100 have now closed additional standalone families: integrated scalar
Tier-0 at the WANIX confidence gate, prebuilt external Tier-0 at native Boot,
per-hop accounting, tail-proof metadata reuse, interrupt-deadline
representation, dormant region-policy machinery, and interleaved SIMD
fused-TLB rows. R100's 1.017x Compile point gain is useful attribution but not a
retained product gain because it misses both frozen causal thresholds. Exact
`d9f686a9...` remains the executable-equivalent product source build and R087
remains the authoritative 13/13 legacy, 11/13 v86 baseline.

The next implementation cycle must start from a fresh exact-baseline residual
measurement and target dynamically active operations. It may accept a general
reproducible 3--5% gain; there is no byte-size rejection rule. It must not retry
R100 through SIMD spelling, alignment, mixed rows, or workload selection, and
must retain the same correctness, cold, Boot/Compile/Python, Chrome, qualified
WANIX `/shared/bench.py`, and untouched-scorecard escalation order.

R101/R102 now also close simple post-member structured fuel comparisons. A
large exact operator census activated the mechanism, but removing 59.5 million
operators tied Compile at 0.997x paired speedup and normalized MIPS. Exact
`d9f686a9...` is restored. Do not retry independent-set order, larger unchecked
segments, branch spelling, or workload-shaped membership after observing this
result.

The next residual cycle should distinguish optimized native cost from emitted
Wasm count before product implementation:

1. capture exact R085-equivalent Compile generated modules and their optimized
   V8 tier, preserving public cadence and the modern guest;
2. attribute native samples/instruction shape to unavoidable structured
   successor dispatch, state commit, cross-module transfer, memory proof, and
   guest arithmetic rather than counting Wasm operators alone;
3. admit only a new architecture-general mechanism with an ordinary
   tiered-engine local proof and a prospectively powered path to the verified
   1% cumulative floor;
4. preserve code size as attribution only and gate actual cold construction;
5. retain the existing correctness, seven-pair native, Chrome, qualified long
   WANIX `/shared/bench.py`, and untouched three-way scorecard sequence.

R103 now closes the obvious cross-module state-carry representation before a
product edit. Exact counters show 331.2 million chain-attributable GPR boundary
operations, but a two-instance full-GPR carried ABI ties at 0.9989x and fails
its stability and confidence gates. Do not tune lane count or choose popular
registers after this result. Exact `d9f686a9...` remains active.

The next cycle must target work that remains expensive in optimized native
code, not merely Wasm memory traffic or branch count. Prefer a direct native
shape/cycle comparison of the dominant generated-memory proof or the remaining
interpreter decode path. A candidate may advance at 1% under R104, but its
screen must be powered for that effect and preserve Compile, Boot, Python,
browser, and WANIX protected work rather than selecting one favorable row.

R106 closes the exact R105-scalar plus R064-pending composition without a
runtime causal result: it passed all semantic gates but missed the frozen cold
point limit at `1.0511625x` versus `1.0500000x`. Do not rerun that exact
composition or alter its statistic after observation. Before the next
independent candidate, prospectively choose and power a cold-cost
non-inferiority rule (or an explicit end-to-end amortization budget), while
retaining the verified 1% runtime rule, protected Python/WANIX work, and the
unchanged browser and scorecard escalation order.

R107 completes that prerequisite prospectively. Future candidates use 15 real
fresh-process `RV64Debug.create` pairs and turn the upper-confidence
candidate-minus-control delta into a one-time millisecond debit on every
runtime row. There is no standalone percentage or byte-size veto. The current
control/control calibration contributes only 0.155 ms of conservative debit.

The next independent residual cycle is therefore:

1. preserve exact `d9f686a9...` and R087's corrected public cadence;
2. identify a dynamically active optimized-tier Boot or Compile cost that has
   not already been closed;
3. freeze a proof-only local model and a selector-free architecture-general
   candidate before product edits or candidate timing;
4. apply R107 construction debit and R104's powered 1% target/protected rules;
5. retain separate execution and construction-to-marker browser clocks, then
   run qualified WANIX and the untouched scorecard only after earlier gates;
6. promote every verified net-positive candidate and accumulate gains until
   both Boot and Compile meet copy/v86 parity.

R109 completed that sequence through the native gate. Its dense stackifier
passed the frozen corpus/model/correctness gates and produced a coherent 1.5%
Boot point improvement, but Boot's adjusted lower bound was 0.9973 and Compile
fell to 0.98926. It is rejected and fully removed under the no-variant stop
rule. Do not revisit tree/bitset width, hybrid containers, SCC traversal,
instruction-sink composition, or sample count from this result.

The next independent residual cycle must therefore:

1. start from exact `d9f686a9...` and corrected public cadence;
2. exclude the now-closed dense CFG representation and standalone typed sink;
3. identify a different dynamically active optimized-tier cost with enough
   whole-row closure to power a verified 1% decision;
4. freeze one selector-free mechanism before candidate timing;
5. use R107's construction debit, 15 fixed native pairs, and R104 target and
   protected-row rules without optional extension;
6. escalate to browser, qualified WANIX, and untouched scorecard only after a
   clean native pass.

R110 completes the optimized-native attribution prerequisite without touching
the product. Its corrected one-run census finds an 8.87% whole-Compile ceiling
in explicit guest-body frame/spill traffic, with 91.76% of sampled JIT-path
cycles already in TurboFan. Source-local reuse, full-state cross-module carry,
and per-module table ownership remain closed; native evidence does not reopen
their failed implementations.

The next independent cycle is R111:

1. quantify, from architecture-general CFG/liveness and current emitter shape,
   how a bounded same-module partition changes live architectural-state union,
   function count, and static boundary frequency;
2. freeze one ordinary-tiered V8 model that performs identical work with the
   current single-function form and the proposed partition form;
3. require exact output/work, natural Liftoff-to-TurboFan activation, smaller
   optimized native frames/stack exposure, stable paired timing, and a
   projection of at least 1% whole Compile after all boundary cost;
4. stop without product code if the model misses any admission gate; do not
   tune partition size, state subset, or CFG choice after seeing timing;
5. if admitted, freeze one selector-free product protocol based only on static
   graph/liveness limits, then run full semantics and R107/R104 construction,
   native target, normalized-work, and protected-row gates;
6. escalate a native survivor through fresh Chromium, qualified WANIX, and the
   untouched three-way scorecard. Preserve exact `d9f686a9...` until a complete
   promotion passes.

R111 stopped at the first frozen gate. Its static analyzer found the intended
local-pressure reduction, but the exact partition rule cut 52.88% of Boot and
42.68% of Compile edges and left oversized SCCs in 61.00% of eligible bytes.
Those results fail the 12.5%/20% admission limits, so Gates B/C, product code,
and timing are intentionally absent. Do not search alternate caps, orders,
state limits, SCC splitting, or ABIs from this result.

The next independent cycle must return to attribution rather than another
partition variant:

1. preserve exact `d9f686a9...`, R087 cadence, R104's verified-1% rule, and
   R107's absolute construction debit;
2. map R110's optimized native samples to generated-Wasm source offsets or
   semantic operator families if the preserved JIT debug records support a
   deterministic mapping;
3. separate frame traffic caused by architectural state, memory proofs,
   structured dispatch, and ordinary guest arithmetic before naming a new
   mechanism;
4. freeze at most one architecture-general local proof from that attribution,
   with no R111 cap/graph/ABI variant and no closed-axis reopening; and
5. implement and time product code only after the local proof demonstrates a
   powered path to a verified 1% whole-row gain with protected rows intact.

R112 completed step 2 with a negative answer. The preserved dump contains
valid source records for JavaScript, but none for the sampled generated Wasm;
source coverage of both TurboFan guest work and native-stack work is exactly
zero. The result is deterministic and closes every R110 period total, so do
not rerun perf or select another engine/debug flag.

The next attribution step may inspect the existing native code only. Before
another product mechanism, freeze one read-only classification of explicit
stack traffic by frame slot, read/write form, and proximity to calls/control
transfers. Its purpose is to distinguish call-boundary spills from distributed
register pressure. It may not infer guest operators, recollect samples, or
reopen R111 partition parameters. If no architecture-general family owns
enough removable whole-Compile exposure for the verified 1% floor, close that
route without product code.

R113 completed that classification. Call-boundary stack traffic is zero; the
cost is distributed across general-body reloads and branch-adjacent selector
traffic. No frozen form passes every admission condition, so do not tune the
sample floor, pool reloads/spills/comparisons, or implement a form-specific
candidate.

The follow-up source audit closed the proposed block-parameter model before it
was built: carrying the same architectural values through within-function
parameters mostly repackages the already-tested R039 local-state and R103
carried-state mechanisms. It is not an independent removal justified by R113.

R104's historical audit instead identifies R014 as a credible casualty of the
old coarse percentage gate. Reconstruct exactly one independent part against
the current product: defer architectural-PC local materialization across
fully covered structured edges, materializing it only on a leaving path. Do
not include R013/R014 fuel or safepoint lowering, which R102 independently
reconfirmed as a tie. R114 freezes the implementation and requires:

1. a default-off same-artifact causal switch with directed proofs for internal,
   external, dynamic, safety, precise-side-exit, and chain paths;
2. deterministic current real-region operator accounting and the complete
   correctness/Linux gate before timing;
3. R107's 15-pair construction debit and 15 fixed native Boot/Compile/Python
   pairs under R104, targeting a verified 1% Compile gain; and
4. only after native admission, clean product, Chromium, qualified WANIX, and
   untouched authoritative scorecard confirmation.

R014's old timings authorize the test but provide no performance credit. Stop
and restore exact baseline at the first failed gate; do not tune a second PC
placement after observing timing.

R114 is complete and stopped at its first performance failure. Structural and
correctness proofs passed, but the frozen 15-pair native result measured
debit-adjusted Compile at `0.98579x` with 95% interval
`[0.95202,0.99629]`; normalized work agrees at `0.98577x`. Coverage was
matched, so this is not R014's old coverage confound. The candidate and all
causal plumbing are removed, and the release rebuild is exact `d9f686a9...`.

Resume the parity search from that baseline. A future candidate may target a
verified cumulative gain as small as 1%; do not require a 10% or 20% result,
and do not use code size as a veto. It must still be admitted independently,
freeze one architecture-general mechanism before timing, charge construction,
clear confidence and normalized-work rules, and preserve Boot, Python, WANIX,
`python /shared/bench.py`, and the authoritative scorecard. Do not reopen
R114's PC placement from its negative current-baseline result.

R115 also closes R095's unresolved packaging confound. Same-instance linkage
removes a verified 3.4% Boot tax relative to an auxiliary Wasm instance, but
the exact embedded executor still regresses its identical disabled control by
about 2%, with the Boot interval wholly below parity. Preserve same-instance
linkage as a future design rule; do not revive this scalar Tier-0 executor.

Resume from exact `d9f686a9...`. The next candidate must come from a different
live cost, target a verified cumulative gain as small as 1%, freeze one
architecture-general mechanism before timing, and retain the complete
construction, confidence, normalized-work, browser, WANIX, and scorecard
sequence. Neither code size nor failure to produce a 20% jump is a veto.

R116 closes the no-extra-work hot/cold state split at its static gate. Although
77.52% of corpus byte weight contains at least one single-member acyclic
register, the frozen rule removes only 2.04% of total locals. Do not implement
or tune that rule. A future pressure mechanism must remove cross-member live
ranges without R111's edge cuts or explicitly prove that added
materialization/global traffic pays for itself in ordinary V8.

R117 supplies that explicit ordinary-V8 cost proof for private module globals
and fails decisively. Its normalized-equivalent all-state model is 0.97033x in
steady execution and 0.82697x on first execution. Stop before native capture
or product work; do not search partial global widths or selected registers.

Resume from exact `d9f686a9...`. R110's spill ceiling remains a diagnosis, but
R111 partition boundaries, R116 selective materialization, and R117 global
storage now close the three direct state-placement responses. The next cycle
must identify an independently removable live cost or change the generated
control representation without reintroducing those mechanisms. Freeze one
architecture-general proof before timing, admit any verified end-to-end gain
of at least 1% under R104/R107, and retain every protected browser, WANIX,
`/shared/bench.py`, and untouched-scorecard gate.

R118 closes the historical flat-RVC dispatch lead after a fair current-product
retest. Its full correctness and shape gates pass, but its stable Boot subset
establishes a 1.8% debit-adjusted regression. Do not retry a family order,
selector spelling, opcode-frequency layout, or composition; the favorable
Python point does not compensate for a failed Boot target.

Resume from exact `d9f686a9...`. The next cycle must come from an independent
dynamically active cost, not a historical percentage casualty already closed
by R114/R118 and not the R111/R116/R117 state-placement family. Before product
work, use current Boot/Compile evidence to freeze one architecture-general
opportunity with a plausible whole-row 1% closure. Then retain deterministic
shape/correctness, R107 construction debit, 15-pair Boot/Compile/Python native
gates, Chromium, qualified WANIX `/shared/bench.py`, and the untouched
three-way scorecard. Promote a verified 1% net gain even if code growth is
larger than R118's 1,354-byte diagnostic delta.

The expanded ledger audit adds legacy E005b (one-pair +4.5% Compile after
disabling the old per-trace TLB cache) and E006b (one-pair +2.6% Compile at old
tier threshold 32) to the list of experiments that deserved stronger evidence
instead of an automatic sub-10% tie. Do not restore them directly: both belong
to deleted `rv64-jit`, lack confidence/protected-product evidence, and have no
exact active production-page-policy equivalent. Check current profiles and
counters for an equivalent dynamically active cache or tier cost before
freezing any successor. If none exists, continue to a different active cost;
do not run a knob sweep merely because the legacy point was favorable.

R119 followed that attribution requirement and is now closed. The existing-
probe fused fetch passed correctness and native shape, but its debited Boot
result `1.012411x [0.997859,1.015302]` is not confidence-verified and Compile
misses protection at `0.984634x [0.947863,1.016480]`. Do not extend samples,
reorder the helper, add a second cache, or compose R119 with another mechanism.
Its candidate is archived and exact `d9f686a9...` is restored.

Resume from the exact baseline. Continue accepting candidates that prove a net
target gain as small as 1%; code-size growth is not a veto. The next mechanism
must be independent of R119's fetch-capability payload and selected from a
current active cost with plausible whole-row exposure. Freeze it before
implementation/timing, then retain the construction debit, 15-pair target and
protected rows, confidence/normalized-work rules, Chromium, qualified WANIX
`/shared/bench.py`, and untouched three-way scorecard sequence.

R120 completed the first exact remeasurement found by the small-gain audit.
R100's old `1.017x` Compile point justified a fair test under R104, but fifteen
new pairs reverse the point to `0.992069x [0.952178,1.015084]`; normalized
work agrees, and Python also misses protection at `0.982841x`. Do not pool the
old five pairs, rerun, alter vector spelling/alignment, or derive a selected
variant after seeing the result. Preserve exact evidence and keep baseline
`d9f686a9...`.

Continue the ledger audit under the same distinction. A historical candidate
rejected solely by a coarse floor deserves an independent current-baseline
reconfirmation if its mechanism still exists and is architecture-general; it
does not receive retroactive performance credit. Promote any candidate that
proves a construction-debited target median and normalized gain >=1.01x with
lower confidence bound >=1.00x, protected rows intact, and all correctness,
Chromium, WANIX `/shared/bench.py`, and scorecard gates. Candidate bytes alone
are never a veto. If no further exact artifact satisfies those prerequisites,
resume current-profile attribution for the next independent active cost.

R121 completed that current-profile dispatch attribution. Do not implement a
negative absence cache, parallel dispatch PA table, richer direct row, or
map-generation refresh variant: the complete fallback path owns only 0.7481%
of preserved main-thread period, and the sole cause census shows almost no
mapping rejection. The frequent empty and stale lookups are genuine, but no
subset has a credible construction- and overhead-adjusted 1.01x whole-row
projection. Exact product `d9f686a9...` is restored.

Continue from a different current active cost. The remaining optimized
scheduler sample region is post-dispatch policy/interpreter/lifecycle work,
but it is heterogeneous and only 1.2834% of main-thread period in R110. Before
any edit, attribute it to one source operation and require a realistic >1%
net projection; do not combine unrelated leaves to cross the floor. If that
region cannot qualify, return to the larger exact Boot interpreter profile or
generated-body work with a mechanism materially distinct from R111--R120.
Every admitted candidate retains R107 construction debit, 15 paired native
trials, confidence and normalized-work gates, protected Python, Chromium,
WANIX `/shared/bench.py`, and the untouched three-way scorecard. Bytes remain
diagnostic only.

R122 completed the larger Boot interpreter-body attribution. Do not implement
a quadrant-1 immediate-motion variant, quadrant-2 selector reorder, branchless
x0 scratch slot, compact step outcome, retirement batching, or another scalar
memory/fetch cache. Exact native blocks and the sole modern counter census show
that each independent removable population is below a credible net 1% result
or belongs to R054/R058/R105/R118/R119 closure. Exact product
`d9f686a9...` is restored.

Continue from a different active cost. The two official gaps remain Boot
(rewrite/copy-v86 `1.392x` elapsed ratio) and Compile (`1.311x`). Before another
edit, require a current uninstrumented native or generated-Wasm attribution to
identify one architecture-general mechanism with at least 1.25% complete
exposure and a realistic construction-debited whole-row gain of at least 1%.
Do not combine unrelated leaves or use opcode, binary, guest-PC, workload, or
engine identity as a selector. If a candidate is admitted, freeze it before
implementation and apply R107, 15 paired Boot/Compile/Python trials, confidence
and normalized-work checks, Chromium, qualified WANIX `/shared/bench.py`, and
the untouched scorecard. Promote any fully verified net gain >=1%; bytes alone
never veto it.

R123 closes the obsolete production flag check inside R054 fused memory. The
complete authenticated guard blocks own only 0.104231% of Boot main-thread
period and cannot support a 1% whole-row result even under impossible complete
elimination. Do not implement the compile-time Bus variant, remove the
diagnostic switch, or compose this leaf with unrelated work. Exact product
`d9f686a9...` remains the baseline.

Next return to the material R110 optimized generated-body population: explicit
native stack traffic owns 22.4351% of guest-body cycles and has an 8.87%
whole-Compile exposure ceiling. Attribute the real 56-region corpus's entry,
exit, and in-body architectural-state operations and liveness before selecting
a change. A successor must stay within functions (R111 partitioning is closed),
add no full-GPR boundary ABI (R103), avoid the rejected module-global form
(R117), and demonstrate a realistic >=1% whole-Compile projection before any
product edit. Freeze one architecture-general mechanism, then apply R107,
fixed paired Boot/Compile/Python gates, correctness, Chromium, WANIX
`/shared/bench.py`, and the untouched scorecard. Code size remains diagnostic.
