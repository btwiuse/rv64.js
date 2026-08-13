# JIT Rewrite Decision Log

## D001: clean-room compiler replacement

Date: 2026-08-05  
Status: accepted

The former `rv64-jit` source and historical tuning narratives will not be read
or reused. The previous implementation may be executed only as a black-box
baseline if a future comparison is necessary. The new compiler is derived from
the ISA, interpreter behavior, runtime contracts, tests, and independent JIT
research.

## D002: preserve recoverability without retaining active source

Date: 2026-08-05  
Status: accepted

Delete the old crate from the active tree, but retain ordinary Git history at
baseline commit `4b0896decdff7538f9c1d2b44dc19a1d3d14f7c2`. Do not rewrite history merely
to make the deletion irreversible.

## D003: interpreter is T0 and correctness oracle

Date: 2026-08-05  
Status: accepted

The existing interpreter remains the execution fallback, profiling tier, and
differential oracle. JIT development must not require maintaining an unrelated
second architectural semantics implementation for uncommon instructions.

## D004: optimize regions and lifecycle, not isolated instructions

Date: 2026-08-05  
Status: accepted

The fundamental compilation unit is a bounded multi-block region emitted in a
module batch. The design must explicitly measure the second WebAssembly JIT's
compile/instantiate/tier-up lifecycle. One-module-per-basic-block and
JavaScript-per-edge designs are outside the intended architecture.

## D005: replacement crate has a new identity

Date: 2026-08-05  
Status: accepted

The replacement compiler is `rv64-dbt`, not a recreation of `rv64-jit`. This
makes the clean-room boundary visible in the filesystem and dependency graph.
During Phase 0 it supplies an inert compatibility adapter that emits no code,
allowing the existing runtime to exercise only the interpreter while new typed
contracts replace the adapter incrementally.

## D006: state reads precede architectural commit

Date: 2026-08-05  
Status: accepted

All entry architectural reads are materialized before any output register is
stored. This preserves source-before-destination semantics when an instruction
aliases its input and output, notably `JALR rd == rs1`, while still allowing
single-use pure expressions to remain stackified.

## D007: optional host capabilities are not mandatory ABI imports

Date: 2026-08-05  
Status: accepted

The main Wasm module imports only publication functions actually referenced by
the selected compiler/runtime policy. Tests allow known optional publishers but
require only active host services; this keeps release LTO and future publisher
replacement from being mistaken for ABI breakage.

## D008: guest exceptions never use Wasm traps

Date: 2026-08-05  
Status: accepted

Every operation whose Wasm failure behavior differs from RV64 is guarded before
the partial Wasm instruction executes. Flat-memory bounds failures commit the
pre-instruction PC, register state, and retirement count and return to T0.
Integer divide/remainder guards return the architecturally specified RISC-V
values directly. Eager `select` is not a valid trap guard; structured control
flow is required.

## D009: effectful loops carry complete dirty state

Date: 2026-08-05  
Status: accepted

A single-latch loop with memory effects keeps a carry local for every dirty
architectural register, including registers the static body writes without
reading. A side exit chooses a current-iteration SSA definition when one exists
and otherwise commits the carry value from the preceding completed iteration.
This preserves precise faults without per-iteration architectural stores.

## D010: full-system memory requires a new typed contract

Date: 2026-08-05  
Status: accepted

Flat user memory may compile when the layout supplies an explicit base/length
capability. Merely exposing the surviving system TLB offsets is not sufficient
authorization or specification for code generation. System loads/stores remain
precise interpreter exits until translation permissions, page crossing, MMIO,
TLB fill, code-page protection, and generation validation are represented in a
typed runtime contract and covered by differential tests.

## D011: forward traces use guards and architectural snapshots

Date: 2026-08-05  
Status: accepted

T1 follows forward fallthrough/direct-JAL edges when their bytes are available.
A forward conditional branch continues along fallthrough and records a guard
whose taken path commits the exact post-branch register, FP, CSR, PC, and
retirement state. Backward edges remain terminal unless they form the supported
single-latch loop. This removes common diamond/call dispatches without requiring
an unrestricted CFG compiler in the latency-sensitive tier.

## D012: exact FP is the baseline; native FP requires a proof

Date: 2026-08-05  
Status: accepted

Generated modules use one Wasm-to-Wasm `fp_exec` import for scalar F/D
arithmetic, sqrt, min/max, comparisons, classifications, conversions, and FMA.
The helper returns raw bits and updates the forwarded `fcsr` cell. Native Wasm
FP is permitted only when guards prove it cannot change any not-already-sticky
architectural flag or violate the active rounding mode. The first accepted fast
path requires RNE, NX already set, finite operands, and safe result classes;
all other cases use software FP.

## D013: lifecycle results use isolated processes and frozen bytes

Date: 2026-08-05  
Status: accepted

Cold samples run in fresh engine processes and retain their raw measurements.
Guest translation, byte copying, Wasm compile, instantiation, and publication
are timed separately. Exact generated modules are content-hashed and replayed
without guest translation. Best-of-N, an unreported warm cache, and one engine's
diagnostic tier flags are not acceptable headline methodology.

## D014: Wasm tier re-entry is policy, not compiler semantics

Date: 2026-08-05  
Status: accepted

The runtime exposes a user-loop quantum experiment because engines such as V8
do not on-stack-replace an active Wasm invocation. The compiler always emits
fuel-bounded loops, but the portable default does not add periodic re-entry:
current paired experiments added thousands of calls without a repeatable gain.
The policy remains independently tunable and must be reconsidered with
cross-engine data and larger regions.

## D015: advertise an ISA profile, not an ambiguous “RV64”

Date: 2026-08-05  
Status: accepted

Initial complete JIT coverage targets the `rv64gc` compiler contract. Newer
RVA23 binaries can require vector, bit-manipulation, conditional, and additional
FP extensions and will not be advertised as fully compiled until those features
have explicit semantics and tests. Interpreter fallback preserves correctness
for supported legal instructions during incremental coverage.

## D016: translation refill is followed by a row re-probe

Date: 2026-08-05  
Status: accepted

The full-system fast path consumes a typed row containing a virtual tag and a
linear-memory offset. A permitted `tlb_fill` helper is an attempt to establish
that row, not an alternate translation result ABI. Generated code always
re-probes the row after the helper returns. No sentinel value encodes RAM,
MMIO, permission failure, or a host pointer.

## D017: uncommon full-system memory stays a precise T0 exit

Date: 2026-08-05  
Status: accepted

An access crossing a guest page, resolving to MMIO/non-RAM, failing permission
checks, or storing to a compiled code page materializes the exact
pre-instruction state and returns to the interpreter. This keeps device,
fault-priority, and invalidation semantics centralized until measurements show
a specific slow path is worth duplicating in generated code.

## D018: opaque runtime state is represented by typed capabilities

Date: 2026-08-05  
Status: accepted

Reservation helpers explicitly identify user or system state, and system
memory explicitly identifies its translation-row layout and miss policy. A raw
integer address is not sufficient authorization to generate a helper call.
This prevents ABI-valid but semantically invalid pointer casts as more runtime
state is exposed to generated modules.

## D019: system FP state is an ordered effect

Date: 2026-08-05  
Status: accepted

Every compiled system FP instruction checks `mstatus.FS` before retirement and
before the operation's other observable effects. Instructions that write FP or
FP-control state mark FS Dirty at the interpreter-defined point; FP stores only
check availability. This effect remains explicit in IR so optimization cannot
move it across a memory fault or exact-helper call.

## D020: the portable multi-entry primitive is an in-module dispatcher

Date: 2026-08-05  
Status: accepted

Related T1 bodies in one module share a balanced PC dispatcher. Batch entry
exports may all reference that dispatcher, which selects from architectural PC
and continues covered edges internally. This works in core Wasm without relying
on tail-call instructions, avoids JavaScript on a hot edge, and keeps one
correctness path for batches and static/sparse regions. Direct calls and
register-resident CFG lowering may optimize this primitive but are not required
for correctness.
Wasm tail calls are now standardized and broadly implemented, so a same-module
direct-tail backend remains a valid measured optimization. It does not replace
this portable correctness primitive unless cross-engine lifecycle and execution
results justify doing so.

## D021: a profiled indirect edge is a precise dynamic-PC guard

Date: 2026-08-05  
Status: accepted

A stable observed JALR target may be fused into the same SSA region only after
an ordered comparison with the computed architectural target. Match continues
at the profiled target. Mismatch materializes the exact post-JALR registers,
retirement, and computed PC before returning to T0. No speculative state is
rolled back and a changing target remains correct on its first miss.

## D022: linked execution consumes one cumulative fuel budget

Date: 2026-08-05  
Status: accepted

Every loop and multi-entry transfer compares total retirement for the public
Wasm invocation against one fuel cell. A body entered after another body does
not receive a fresh loop allowance. Zero-retirement exits and a defensive hop
cap terminate malformed or no-fuel embeddings without changing the ordinary
block-granularity overshoot contract.

## D023: every publication form participates in lifecycle capture

Date: 2026-08-05  
Status: accepted

Single blocks, contiguous batches, and asynchronously compiled regions all
record byte copy, Wasm compile, instantiation, and publication time and expose
their exact generated bytes/hash metadata to frozen replay. Asynchronous wall
latency and fresh-process frozen frontend time are reported separately; one
must not be substituted for the other.

## D024: general multi-entry execution retains architectural state

Date: 2026-08-05  
Status: accepted

The portable multi-entry backend uses one shared defined Wasm function. It
loads the union of required architectural state at public entry, keeps GPRs,
FP registers, `fcsr`, PC, retirement, and fuel in locals across covered edges,
and materializes once on an external or precise side exit. Member exports refer
to that shared function. The earlier materialized-member backend remains only
as a differential and benchmark mode. Both measured browser engines strongly
favor register residency, and the invariant also removes redundant memory
traffic independently of any engine-specific Wasm extension.

## D025: invocation-local TLB proof caching is opt-in

Date: 2026-08-05  
Status: accepted

Generated full-system regions may retain one proven load and one proven store
page translation for the duration of an invocation. Refills still re-probe the
authoritative translation row, and context/mapping operations remain precise
interpreter exits. The runtime default is disabled: paired frozen samples show
the optimization helping Firefox and Node while materially hurting Chrome.
`jit_set_region_tlb_cache` exposes it for explicit engine/workload policy; a
portable build does not silently choose from the embedding engine's identity.

## D026: polymorphic indirect specialization is bounded to two targets

Date: 2026-08-05  
Status: accepted

Indirect feedback uses explicit generated guard-miss cells rather than treating
the final PC of a fused invocation as the original edge's successor. Runtime
profiling retains two constant-space candidates. A site may compile at most a
monomorphic specialization and one two-way upgrade; a balanced site can skip
the monomorphic intermediate. The two-way region dispatches observed targets
inside one register-resident Wasm function. Unobserved and megamorphic targets
remain exact side exits, preventing code-size growth and recompile storms.

## D027: cross-page PICs use targeted sparse snapshots

Date: 2026-08-05  
Status: accepted

A two-way indirect upgrade captures only the distinct source and observed
target pages, lifting each target as an independent entry. Ordinary T1 traces
retain the measured one-page snapshot default; a rare cross-page target does
not justify restoring a 256-KiB capture window for every compilation. Every
captured physical page is attached to the published entry for mapping checks
and invalidation. An unavailable target is skipped while the valid source still
compiles, so a failed speculative upgrade cannot blacklist executable code.

## D028: SSA temporary locals are pooled across member bodies

Date: 2026-08-05  
Status: accepted

Independent multi-entry bodies never execute concurrently, and each copies its
architectural outputs into shared state locals before control can reach another
body. Non-architectural SSA locals therefore use one i32 and one i64 pool sized
to the largest member. They are not allocated cumulatively by region size. This
reduces generated local pressure for both baseline and optimizing Wasm compilers
without adding runtime validity checks or changing precise side-exit state.

## D029: SC consumes its reservation only after checked generated memory

Date: 2026-08-05  
Status: accepted

The SC helper first probes ownership without mutation. Generated code then
executes the conditional store through the ordinary precise memory capability
and clears the reservation only after that path completes (successful store or
known failed condition). A translation, permission, MMIO, page-crossing, or
other store-address side exit occurs before the clear, allowing T0 to
re-execute the same SC with canonical reservation state. Conditional stores are
an SC-specific IR contract and validation rejects an unrelated predicate.

## D030: guest setup waits for observable markers, not guessed work

Date: 2026-08-05  
Status: accepted

Boot, shell setup, file creation, each transfer chunk, and decode complete on a
guest-emitted marker with a wall-clock bound. Commands quote part of each marker
so terminal echo cannot satisfy it. Correctness/benchmark execution starts only
after setup output is discarded, runs in a fresh VM per compared mode, and
retains its own bounded completion marker. Failures report phase, PC,
retirement, and console tail; fixed untimed instruction drains are not evidence
that setup completed.

## D031: modern Virt is the full-system delivery target

Date: 2026-08-05  
Status: accepted

The supported full-system gate is the current `VirtMachine` with the repository's
slim Linux 6.12/Alpine image. Both direct S-mode kernel entry with host SBI and
M-mode OpenSBI `fw_dynamic` entry must boot, reach the shell, and execute T2
code. The legacy TinyEMU/BBL/Linux 4.15 path may remain a compatibility test,
but it is not the source of current boot policy or the only JIT acceptance gate.

## D032: eager register residency is the portable state default

Date: 2026-08-05  
Status: accepted

Multi-entry modules eagerly load the bounded union of live architectural state
once and retain it through covered edges. Lazy valid-bit loads approximately
double real-region bytes and frontend compilation work and lose the synthetic
integer execution comparison in Node, Chrome, and Firefox. Materializing on
every member has a smaller frontend but repeatedly loses hot execution. Lazy
and materialized forms remain selectable for diagnostics and future hot/cold
partition experiments.

## D033: balanced dispatch is default; direct and tail dispatch are experiments

Date: 2026-08-05  
Status: accepted

The balanced comparison-tree in-function PC dispatcher is the portable
multi-entry default. Direct structured lowering carries a known dense successor
through a `br_table`; it changes real-region bytes and frontend time only
slightly, but the current fixed-work execution corpus does not justify replacing
the simpler shared path. Same-module `return_call` is
feature-detected and retained as an experiment; it is disabled by default
because it is neutral on Chrome/Node integer state and severely regresses the
Firefox integer corpus. Feature availability alone is not a performance policy.

## D034: regions default to three pages and 32 leaders

Date: 2026-08-05  
Status: accepted

The hard experimental bounds remain three pages and 512 leaders, while the
portable runtime default is three pages and 32 leaders. A fresh-process modern
Linux sweep found essentially unchanged generated coverage from 32 through 512
leaders, but 32 leaders was about 16% faster than 512 and emitted roughly one
sixth as many bytes. One-, two-, and three-page runs overlap at the current
sample resolution; three pages retains cross-page opportunity without the
large leader-union cost. Both caps remain runtime-settable for measurement.

## D035: policy evidence includes real compiler-generated regions

Date: 2026-08-05  
Status: accepted

Synthetic kernels isolate execution shape but cannot determine production
frontend cost. The frozen corpus therefore includes deterministic regions from
a compiled RV64 benchmark and Alpine musl's dynamic loader, selected at ELF
entry, `main`, and large function symbols over seven page/leader geometries.
Every eager/lazy/direct/materialized/tail module is hash-addressed, and reports
retain input ELF hashes. Default changes must reconcile this frontend evidence
with exact-state execution measurements and a live modern-Linux workload.

## D036: measurements retain raw samples and uncertainty

Date: 2026-08-05  
Status: accepted

Fresh process/profile trials alternate paired order. Reports preserve every raw
sample and publish min, conventional median, p95, max, and a deterministic
fixed-seed bootstrap 95% interval for the median. Browser timer quantization,
unsupported feature probes, and standalone CLI/AOT overhead are recorded rather
than normalized away. A report is diagnostic when its sample count or timer
resolution cannot support a stronger claim.

## D037: asynchronous publication is an explicit scheduling contract

Date: 2026-08-05  
Status: accepted

Large regions use asynchronous `WebAssembly.compile`. The core exposes
`sys_pending_builds()`, and an event-driven host must yield between bounded
`runSystem` calls while builds are pending. Correctness tests pause a still-live
guest for bounded compile settlement before asserting generated execution; they
do not assume a particular engine can compile a region within 100,000 guest
instructions. Publication still validates boot generation and code-page state.

## D038: a standalone engine executes, not merely validates, frozen code

Date: 2026-08-05  
Status: accepted

Wasmtime is the independent delivery engine. A generated driver preloads the
required `env` and `jit` modules, executes all 11 synthetic backend variants,
and checks exact visible state. AOT compilation of every eager real region is a
second frontend gate. Browser and Node results remain primary Web-embedding
evidence; standalone validation is not treated as a substitute for execution.

## D039: old/new claims require generated coverage on both sides

Date: 2026-08-06  
Status: accepted

The previous implementation may be built and executed as a black box only
after the clean-room delivery checkpoint. Headline pairs use the same upstream
commit and inputs, fresh processes, alternating order, exact hashes, raw
samples, paired bootstrap intervals, and a predeclared host-stability gate. A
row is called JIT versus JIT only when both sides record generated retirement.
Modern-machine results with zero previous coverage are explicitly product
comparisons, even when they are useful delivery evidence.

## D040: cold Wasm tier and re-entry are separate performance contracts

Date: 2026-08-06  
Status: accepted

A long generated Wasm invocation can finish before an embedding engine can use
newly compiled optimized code because the active call is not replaced. Reports
therefore separate untouched production cold-tier behavior, bounded re-entry
and time-to-tier, and an explicitly stabilized optimizing-tier diagnostic.
Optimizing-tier flags never replace the production headline. The portable
compiler must improve baseline stack/local shape; the runtime may additionally
apply selective re-entry to long single-entry loops when cross-engine paired
measurements show a net benefit. This refines D014 rather than enabling a
global loop quantum without measurement.

## D041: zero-progress WFI returns control to the host

Date: 2026-08-06  
Status: accepted

A full-system interpreter call that retires zero instructions after WFI ends
the current public run slice immediately. It must not consume the remaining
budget through synthetic one-instruction iterations: asynchronous devices such
as an external 9P backend can deliver the wakeup only after Wasm returns to the
JavaScript event loop. Cold and warm fallback paths share this rule. Caller
budgets still bound active guest work, and the dedicated WFI regression checks
that the number of internal slices does not scale with a large unused budget.

## D042: asynchronous device replies publish completion before returning

Date: 2026-08-06  
Status: accepted

Accepting an external virtio-9P reply completes its outstanding descriptor,
updates the used ring, and asserts the device interrupt in the same host call.
It is not deferred to a later interpreter poll. A deferred completion cannot
wake a hart already sleeping in WFI because the interrupt needed to cause the
next interpreter pass does not exist yet. Device-level tests assert the used
ring and interrupt state immediately after host reply delivery.

## D043: JIT-off is a direct interpreter-driver bypass

Date: 2026-08-06  
Status: accepted

`jit_set_enabled(0)` is a performance baseline contract, not only a compile
policy. User and full-system execution branch before allocating or consulting
JIT state. Full-system calls pass the caller's complete budget to the machine's
ordinary `run_slice` and drain host I/O once afterward. Clearing caches and
pending asynchronous publications remains a correctness backstop, but setting
the threshold out of reach is not accepted as an interpreter baseline because
it retains dispatcher, profiling, invalidation, and sub-slicing overhead.

## D044: system compile admission is keyed by verified VA-to-PA mapping

Date: 2026-08-06  
Status: accepted

The page-policy key is `(virtual page, physical page)`. Physical-only heat can
compile or dispatch bytes under the wrong virtual identity; SATP-specific heat
needlessly fragments shared kernel and process mappings. Dispatch, asynchronous
publication, and dirty-page invalidation retain their independent mapping and
generation checks. A sample may approximate heat, but it never authorizes an
unchecked mapping.

## D045: production system compilation is bounded and async-only

Date: 2026-08-06  
Status: accepted

While page policy is active, synchronous per-PC module creation is disabled.
A hottest/recent candidate queue is capped at 64, stale candidates expire by
retired-instruction age, queue-pressure retries use hysteresis, and one global
build is allowed in flight. Late hot entries on a compiled mapping form an
incremental fragment. `WebAssembly.compile` promise completion publishes only
between emulator calls and only after ticket/generation validation.

## D046: fixed 1M/q1024 is the portable admission control

Date: 2026-08-06  
Status: accepted

The stable loader enables page policy with a 1,048,576-instruction mapping
threshold and a 1,024-instruction interpreter sampling quantum. Trace
simulation selected the shape; fresh-process Node threshold/quantum sweeps and
fresh-process/profile Chrome actual-emulator A/B selected the constants. The
1M point is the boot/first-use knee, not the fastest isolated cell. Browser
identity does not change the default, and future adaptive policies must retain
this fixed control.

## D047: WFI is a host yield even after partial slice progress

Date: 2026-08-06  
Status: accepted

An interpreter sub-slice that retires instructions and then reaches WFI returns
both facts to the JIT driver. The driver accounts the retired work and ends the
public call so timers, devices, console output, and async compilation can run.
Zero retirement is not the only valid WFI-yield case. Workload harnesses must
not hide this contract by draining a fixed guest-instruction budget.

## D048: stable API selects production policy; debug API remains explicit

Date: 2026-08-06  
Status: accepted

`RV64.create` enables the selected async page policy before machine assembly,
including when invoked inside rv64.js's emulator Worker. `RV64Debug` leaves the
selector explicit for differential and policy tests. `setJitEnabled(false)`
continues to select the exact direct interpreter bypass and clears pending
generated state; re-enabling JIT preserves the stable loader's policy choice.

## D049: architecture comparisons use matched logical guest images

Date: 2026-08-06  
Status: accepted

The WANIX comparison must not use its stock copy/v86 root against the custom
RV64 root. One recipe builds both guests from the same Alpine release and
package world, Linux source/version, init, pinned WANIX helper revision, and
benchmark payload. Architecture and emulated-machine requirements remain
explicit rather than hidden: copy/v86 is i686 (not x86-64) and uses bzImage,
virtio-pci, and hvc0; rv64.js uses a 64-bit RISC-V Image, virtio-mmio, and
ttyS0. Results from unmatched roots are functional observations only and are
not accepted as emulator performance comparisons.

## D050: matched workloads select adaptive two-page geometry

Date: 2026-08-06  
Status: accepted; supersedes the constants in D045 and D046

The stable system policy uses a 131,072-instruction mapping threshold, a
1,024-instruction sampling quantum, two asynchronous compilations in flight,
512 leaders, and a hard selected region cap of two pages. It records
non-sequential/control entries during cold and warm interpreter samples. A
second reachable page is admitted only when every participating page has no
more than 100 control entries per thousand observations.

This is one adaptive policy, not per-benchmark configuration. CPython's dense
computed-dispatch pages remain single-page, while SHA's mostly direct loop may
cross into its adjacent page. Entry-count-only gating, removing cross-page
calls, eager measured extension, short-stay demotion, rebuild, generated TLB
fill/hash caching, and region/tail chaining did not satisfy both workloads and
remained experimental at this checkpoint. D054 later accepts a different
table-independent tail-chain implementation. The safe `(VA page, PA page)`
identity, bounded queue, generation validation, async-only compilation, and
targeted invalidation from D044/D045 remain unchanged.

## D051: WANIX parity is a paired fresh-process non-inferiority claim

Date: 2026-08-06  
Status: accepted

A matched WANIX parity claim requires at least three, and currently uses five,
alternating RV64/copy-v86 pairs. Every VM sample gets a fresh browser process,
fresh browser profile, and fresh guest. One benchmark repetition is run per
process with phase synchronization. All samples are retained; best-of-N and
cross-process warm-up are prohibited. The harness verifies guest ISA, Python
version, Python checksum, SHA digest, and the absence of a JIT override, while
the page binds and archive hashes prove the matched Alpine roots.

The primary statistic is the paired geometric mean of RV64/v86 elapsed-time
ratios. Its 95% interval is an exact paired percentile bootstrap with the pair
as the resampling unit; paired medians and every raw ratio are also reported.
The current practical parity gate is non-inferiority with at most 10% slowdown:
the upper interval bound must be no greater than 1.10 for every requested
`/shared/bench.py` phase. Faster-than-v86 results do not fail a two-sided
equivalence test because the product objective is performance parity or better.

## D052: structured CFG lowering localizes, rather than globalizes, dispatch

Date: 2026-08-06  
Status: accepted

Reducible region SCCs are rendered with nested Wasm `loop` and `block` scopes
and direct branches. Small irreducible SCCs may use bounded block duplication;
larger multi-entry SCCs receive a dispatcher only at the irreducible header.
Unknown indirect guest targets exit precisely. This follows the Stackifier
family described in Leaning Technologies'
[structured-control-flow article](https://medium.com/leaningtech/solving-the-structured-control-flow-problem-once-and-for-all-5123117b1ee2),
while keeping value stackification a separate optimization and keeping the
portable dispatcher backend as an explicit differential control.

## D053: parity must reproduce in the user's Chromium engine

Date: 2026-08-06  
Status: accepted

Chrome evidence alone is insufficient when the intended interactive browser is
Microsoft Edge. The D051 paired protocol is therefore repeated with an
isolated current Edge stable executable, preserving fresh profiles, alternating
order, matched guests, runtime assertions, and the same non-inferiority gate.
Edge 150 passes all phases; browser-specific policy selection remains forbidden.

## D054: cross-module tail chaining uses one table-owning trampoline

Date: 2026-08-06  
Status: accepted; supersedes the table-importing tail-chain experiment in D050

When Wasm tail calls are available, stable structured regions may transfer to a
published region without growing the Wasm call stack. Exactly one host-created
Wasm helper imports the shared function table and implements
`(state, table_index) -> ()` with `return_call_indirect`. Every generated module
imports that helper as an ordinary function and reaches it with `return_call`;
generated modules never import the table.

This layout preserves a frame-free Wasm-to-Wasm transfer while avoiding V8
publication work proportional to the number of table-importing generated
instances. Selection is by a concrete tail-call feature probe, not browser
identity. The runtime exposes both enabled state and transfer count; stable
measurements must prove both. A precise zero-retirement exit cannot chain, fuel
remains cumulative, and dispatch still validates the complete PC and live
mapping generation before transfer.

## D055: stop policy sampling after an exact entry reaches a final outcome

Date: 2026-08-06  
Status: accepted

The page-policy sampler is required only while an exact `(VA page, PA page,
PC)` can still change heat, candidate admission, or emitted membership. Once
that entry is recorded in `policy_attempted` or `policy_installed`, the observer
would return without changing policy state. Deliberate generated-code exits at
privileged CSR paths can revisit such entries hundreds of thousands of times,
so repeated control-entry profiling is measurable overhead.

Those known-final entries use the ordinary exact interpreter with the same
stop-at-compiled predicate. New mappings and PCs continue through the sampled
interpreter. This is a profiling bypass, not an instruction or dispatch bypass:
guest state, exception semantics, compiled-entry stopping, and compilation
decisions remain identical.

## D056: accepted browser results require immutable artifacts and a fixed host protocol

Date: 2026-08-06  
Status: accepted; strengthens D051

The pair count, alternating order, phase set, one-repetition rule, maximum
slowdown, browser executable, comparison URL, and CPU affinity are written to
`protocol.json` before the first result. One runner holds a host-wide lock for
the complete experiment, pins every child to the declared CPU set, and refuses
to extend the sample after observing results. The page and every bound archive
are hashed before and after each browser leg; all samples must name the same
browser/engine, artifact snapshot, roots, and runtime defaults.

The analyzer rejects missing, overlapping, reordered, or post-hoc pairs. An
unpinned noisy sequence and a sequence whose served archive changed between
legs are invalid evidence, even if their point estimates look favorable. The
accepted Chrome and Edge results use five pairs each, CPUs 8–15, a hash-named
immutable page/archive, and exact paired-bootstrap intervals.

## D057: close the current outline at 11/13 and require structural leverage for the next phase

Date: 2026-08-08  
Status: accepted

The clean-room implementation, correctness matrix, production compile policy,
current-artifact browser guard, and authoritative three-way measurement are
complete. The valid R044 scorecard establishes one current baseline: rewrite
Wasm `d93345139c5a...` wins ten copy/v86 rows, matches String Sort, and loses
only Matched Boot and Compile. It wins every row against the isolated legacy
comparator. The implementation outline is therefore complete even though the
performance objective remains active at 11/13.

The R022-R043 sequence closes local threshold, inlining, decode-cache,
entry-ranking, tiny-tier, translation-cache, TLB-layout, SSA-local,
re-entry-thinning, helper-partition, and redundant-load variants under the
current architecture. A new variant of one of these mechanisms is not admitted
merely because the overall goal remains open. It requires new dynamic evidence
that invalidates the earlier opportunity or wall-time result.

Any new performance phase must target Boot or Compile explicitly and establish
enough removable work to plausibly close a roughly 40-50% relative performance
gap before implementation. The rule must be architecture-general and may not
select a PC, symbol, workload, checksum, compiler binary, or browser identity.
Focused immutable A/B retains the fixed 10% advancement/regression gates, and
`/shared/bench.py` remains a mandatory regression guard. Only a candidate that
clears those gates earns another authoritative 13-row run.

## D058: a handler-dispatched decoded page is not the Boot baseline tier

Date: 2026-08-08  
Status: rejected and removed

R045 proved that Boot has ample physical-page concentration, but a packed,
generation-checked decoded page executor does not turn that opportunity into
wall-time leverage. Even after same-page chaining and direct opcode dispatch
reduced host entries, its inner executor consumed essentially the same sampled
self time as the reference decoder/executor and the final same-artifact result
was a timing tie. It did not clear the fixed 10% advancement gate.

Do not restore this mechanism or retune its page threshold. A future Boot tier
must change execution mechanics—for example compact generated Wasm with direct
operations and control flow—not merely cache decoded operands behind another
Wasm handler dispatch. It still must prove a 40% remaining-gap opportunity,
use a general architecture rule, pass exact differentials, and retain the
Compile and `/shared/bench.py` guards before promotion.

## D059: reject cold per-page runtime Wasm compilation despite offline coverage

Date: 2026-08-08  
Status: rejected and removed

R046's compact privileged tier passed its static/dynamic opportunity gate but
failed two order-reversed live screens by regressing modern Boot 45.7%. The
offline model correctly measured reachable work and module bytes, but isolated
post-boot compilation latency was not a model of promise availability,
background-engine contention, or useful generated execution before readiness.
Fifty-one to fifty-two compact modules displaced only 13-14M extra interpreted
instructions while raising summed cold compilation latency above 6.7 seconds.

Do not rescue this design by changing its heat, page cap, leader cap, or
in-flight count; those are adjacent to already-closed R005, R022, and R035
axes, and the observed regression is far beyond the advancement gate. The next
Boot mechanism must avoid per-page runtime Wasm compilation. A precompiled
main-module mechanism is admissible only if it executes multiple guest
instructions per host/Wasm dispatch, because R045 already rejected
one-instruction handler dispatch. Any such mechanism must first prove broad
architecture-level sequence coverage without selecting a guest PC, symbol,
binary, workload identity, or engine.

## D060: reject exact opcode-triple libraries that do not transfer

Date: 2026-08-08  
Status: closed at opportunity gate and removed

R047 corrected its overlapping frequency bound with a fresh, exact greedy
replay. A 256-handler library trained once on modern Boot removed 40.81% of
Boot dispatches, just clearing the predeclared 40% gate, but the identical
library removed only 31.20% on Compile and 24.78% on Python. Smaller libraries
failed even on Boot. Correctness fingerprints passed in all three captures.

Do not merge, retune, or retrain exact opcode triples against the two failing
scorecard rows. That would violate the required architecture-wide selector and
would replace held-out validation with benchmark fitting. This closes the
precompiled exact-pattern interpretation proposed by D059; it does not forbid
a future workload-independent multi-instruction representation supported by
new evidence. All R047 runtime instrumentation is removed and the accepted
`d93345139c5a...` artifact is restored byte-for-byte.

The next admitted phase targets Compile's nested-engine behavior. Before
changing generated code, measure baseline-versus-optimizing Wasm compilation,
tier-up completion, function size, and module geometry for rewrite and v86.
Only an architecture-general emission change with enough measured engine-tier
leverage may proceed to implementation.

## D061: reject entry-overlap splitting despite eliminating oversized tier work

Date: 2026-08-08  
Status: rejected and removed

R048 directly confirmed that rewrite emits much larger generated Wasm than
v86 and that a repeated 3.22 MiB function reaches TurboFan one phase late. The
same physical 512-entry page is rebuilt at relocated virtual addresses in all
three Compile phases. Ending independently lifted members at later callable
entries reduced that function to about 79.8 KiB without removing coverage.

That static result is not sufficient for promotion. After preserving existing
loop/dense/bulk lowerings, all-region splitting regressed Compile 4.1%. A fixed
large-overlap form changed only modules above 1 MiB whose split form was at
least four times smaller. It removed 57-62% of STEADY emitted bytes but was a
0.998x Compile timing tie and a 0.981x Boot tie in a valid alternating
same-Wasm A/B. Exact outputs and host stability passed.

Do not tune the 1 MiB threshold, overlap ratio, leader cap, or member-boundary
shape. The experiment already removes nearly all of the attributed pathological
function while retaining normal modules, yet supplies no wall-time gain. Large
background tier work is therefore diagnostic engine activity, not a sufficient
proxy for the scored critical path. The candidate state, lifter, runtime and
harness switches are removed and `d93345139c5a...` is restored exactly.

Further Compile work must bound dynamic execution cost rather than generated
frontend volume. It must also separate the unavoidable current 1.256x RV64/i386
guest-instruction ratio from emulator time per guest instruction before
claiming enough leverage for another implementation.

## D062: reject carried two-page stack translation despite near-perfect locality

Date: 2026-08-08  
Status: rejected and removed; diagnostic retained

R049 proved an unusually strong dynamic invariant in the exact Compile guest:
99.9986% of `x2` writes are affine-immediate, there are 13.09 stack-root memory
operations per update, and every observed effective address lies on the
current stack page or its successor. A two-recent-page model misses only 9 of
82,473,135 events. This clears the opportunity question; lack of stack locality
is not why the design failed.

The admitted implementation carried demand-filled load/store translation
offsets for those two pages through structured generated code and refreshed
them at `x2` updates. In a valid alternating same-Wasm A/B it changed Compile
STEADY from 1,052.18 to 1,484.94 ms, a 41.1% regression, while preserving the
exact object fingerprint. The added selection, validity, update, and local
pressure costs overwhelm the already optimized fused-TLB lowering.

Do not tune the number of retained pages, eligibility threshold, update
distance, or access selector. R012 already rejected a per-access persistent
page, R016/R017 rejected member/region versioning, R019 rejected stable
component carry, and R049 now rejects the near-ideal two-page carried form.
These results close stack-translation caching under the current generated
memory representation. Retain the QEMU semantic counters as evidence, but
remove the execution path and restore `d93345139c5a...` exactly.

The next bounded action is attribution, not implementation: map the 52.54%
runtime-Wasm share in optimized-tier Compile to exact functions and distinguish
necessary runtime orchestration from generated side exits/helpers. Only a
broad mechanism with measured whole-row leverage may proceed to code.

## D063: target residual interpreter boundaries before more generated-code micro-optimization

Date: 2026-08-08  
Status: accepted diagnostic direction; no production change

R050 decomposes the stable `run_system_jit` subtree rather than treating every
main-module sample alike. In STEADY, residual interpreter subtrees consume
34.20% of all samples while retiring only 7.64% of guest instructions. Their
sample-normalized cost is 9.42x a generated instruction, reproduced at 10.00x
in PRIME. Scheduler self/cache hashing is a distinct 11.86%; synchronous
translation/issue is only 5.46%.

Do not resume module-byte, compile-registration, or translation-frontend work:
even impossible elimination of the measured translation component misses the
10% whole-row advancement gate. Also do not optimize the single hottest opcode
or PC. The current opcode histogram says the top 30 starting forms cover only
49.42% of residual instructions and cannot distinguish why generated execution
stopped.

The next diagnostic partitioned fallback stretches by exact starting PC,
instruction form, generated-entry state, and page-policy outcome, while keeping
the policy-sampled and final-outcome paths separate. D064 records why that
result did not admit a candidate.

## D064: do not reopen attempted-not-installed entry ranking

Date: 2026-08-08  
Status: accepted closure correction; no production change

R051's exact fallback-site table found one dominant attempted-but-not-installed
entry at essentially the same scale and lifecycle state already recorded by
R032. The earlier general R033 implementation had already removed almost all
of that population without selecting an address; R034 then showed that the
additional generated coverage regressed Compile 3.5% and Boot 1.4%.

Treat R051 as independent corroboration of the R032 diagnosis and R034 wall-time
closure, not as a new optimization lead. Stop before implementing or retiming
the same mechanism, never use the observed PC as a selector, and remove the
transient diagnostic completely. Before another production edit, map the R050
cost components to the R001-R049 closure ledger and require a genuinely new
architecture-level mechanism with at least 10% measured whole-row leverage.

## D065: reject compressed one-load Sv39 proof rows

Date: 2026-08-08  
Status: rejected and removed

R052 preserved independent load and store rows and exact semantics, avoiding
R038's shared-row permission collision. Each `i64` contained a lossless
canonical Sv39 VPN/context proof and the exact wrapping memory32 translation
offset. Full-system bare, alias, Sv39, and MPRV differentials matched after the
gates caught both an invalid extra-bank allocation and an invalid assumption
that Rust-allocated guest RAM was page aligned.

The representation still failed its structural performance gate. Seven
alternating paired fresh-process measurements of frozen generated bytes gave a
0.561x packed/control median without the generated page cache and 0.885x with
it. The former is a 43.9% regression with a 95% median interval wholly below
0.59. V8's native output shows that reconstructing and checking canonical VPN
and permission context costs more than the second scalar table load it removes.
No guest scorecard, encoding sweep, or relaxed correctness rule is justified.

Do not retry scalar tag/offset compression, a packed canonical marker, or a
SIMD/interleaved spelling of this same proof. R037 already bounded the
interleaved SIMD form below the 10% gate; R038 rejected a shared scalar row;
R052 rejects separate scalar rows without collision confounding. Any future
generated-memory candidate must eliminate or amortize translation through a
different architectural mechanism, such as an exact sparse/mirrored mapping,
and prove at least 10% whole-row opportunity before production code changes.
The R052 candidate is removed; accepted executable CODE bytes are restored,
with only non-executable Rust/LLVM name metadata changing on recompilation.

## D066: reject exhaustive function-per-pair dispatch

Date: 2026-08-08  
Status: rejected before production implementation

R053 tested the only architecture-complete continuation of R047 that did not
train a handler library on a workload. A deterministic corpus emitted all
3,844 ordered pairs of the 62 normalized scalar operation kinds. The control
executed one of 62 single-operation functions per operation; the candidate
executed one specialized pair function per two operations. Both traversed
every pair, produced exact state, and were measured in seven alternating fresh
Node/V8 processes under the preregistered 1.25x admission gate.

The exhaustive pair tier reached only 0.879x control throughput, with a
0.878-0.882 bootstrap median interval. Its cold construction cost was just
0.847 ms greater, so the failure is execution shape: a 3,844-way polymorphic
indirect target costs more than the eliminated calls. No production code was
changed.

Do not rescue this result with a popular-pair subset, workload-weighted corpus,
pair-count sweep, or benchmark-derived handler list. Those are exactly the
non-transferable selection family R047 closed. Together R047 and R053 close
precompiled exact opcode-pair/triple handler libraries. A new privileged-T0
mechanism must avoid both per-instruction handler dispatch and a large
function-per-sequence indirect target space.

## D067: promote exact fused interpreter-memory capability hits

Date: 2026-08-08  
Status: accepted; new production baseline

Permit every scalar T0 load/store to consume the existing exact fused JIT-TLB
row when its virtual-page tag, permission context, and index match. Treat the
row's native pointer as a live capability only while the backing RAM cannot
move and the normal mapping/code-page invalidation rules have not cleared it.
On a miss, retain the standard translation, physical bus dispatch, exception,
and publication path without semantic changes. Generated-code-page stores do
not publish store rows, and marking a page as generated clears any prior store
capability.

This is admitted by measured whole-row leverage and frozen local bytes:
`Cpu::ld`/`Cpu::st` accounted for 16.647% of accepted Boot samples, and the
exact corpus achieved 3.030x local throughput against the required 2.51x.
Five-pair same-Wasm Boot A/B achieved 1.161x with its complete 95% interval
above 1.11. The exact final default-on artifact achieved 1.151x Boot, and the
untouched valid three-way matrix reduced Boot 13.35% and Compile 4.73% with no
guarded row regression. Full architectural differentials and the browser
non-regression guard pass.

Promote SHA-256
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.

Do not reinterpret this as permission to bypass an exact tag/context check, to
cache MMIO pointers, or to weaken page-crossing and generated-code
invalidation. Re-profile Boot and Compile before admitting a successor because
R054 materially changes the runtime/T0 cost composition. Copy/v86 parity is
still open on those two rows.

## D068: reject direct interpreter instruction-fetch capability caching

Date: 2026-08-08  
Status: rejected and cleanly removed

R055 followed a fresh post-R054 profile and used a complete architecture-wide
rule: one exact execute-context-tagged live RAM pointer for the most recent
instruction page, with authoritative fallback on every miss, page split,
permission failure, or non-RAM target. The frozen mixed compressed/32-bit
corpus passed decisively at 1.989x paired throughput `[1.984,1.993]`, so one
production prototype was permitted.

Correctness passed, but five alternating fresh-process same-Wasm Linux pairs
rejected the mechanism. Boot paired speedup was 0.962x `[0.935,1.152]` against
the preregistered 1.10x/lower-bound-1.00 gate; Compile tied at 1.008x
`[0.982,1.053]`. The report is measurement-valid, host spread is 1.023x, and
guest inputs plus Compile output fingerprints match. The result may not be
rescued by tuning the page count, refill policy, tag representation, PC set,
opcode set, or benchmark population.

Remove the complete runtime path and retain only reproducible negative
evidence. Production again builds byte-for-byte as accepted R054 SHA
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
Together with R020, this closes both physical-translation and direct-pointer
one-page instruction-fetch caches in the current scalar interpreter. A future
Boot proposal must change a broader execution shape rather than add another
fetch-side cache probe.

## D069: reject exact re-entry callback monomorphization as a standalone change

Date: 2026-08-08  
Status: rejected before production implementation

The accepted module contains a real `call_indirect` boundary for the exact
post-instruction dispatch-tag predicate. R056 preserved every probe and
compared that shape with an inline spelling in deterministic, externally
mutable Wasm modules. Seven alternating fresh-process pairs measured a precise
1.494x local speedup `[1.492,1.500]`, with exact miss/hit results and negligible
cold cost.

That result fails the preregistered 3.30x local requirement derived from the
complete 13.101% Boot category. Its optimistic whole-row projection is only
1.045x, below the fixed 1.10x advancement gate. Do not build a production
candidate, relax the gate, combine projections from unrelated mechanisms, or
retune the tag population. Retain only the frozen evidence. Production was
never modified and remains accepted R054 SHA `4160333352b18b...`.

## D070: reject a dedicated generated-Wasm compiler Worker pool

Date: 2026-08-08  
Status: rejected before production implementation

R057 tested a mechanism left explicitly open by the async compile design. The
control used the accepted two-in-flight `WebAssembly.compile` path. The
candidate transferred identical owned bytes to two compiler Workers, built one
module synchronously per Worker, and structured-cloned each completed module
back. The frozen corpus contains the exact ten-module timed Boot stream and
fifteen-module Compile STEADY stream from accepted R054; fixed foreground Wasm
work makes compiler interference observable without changing a guest policy.

Every module hash, descriptor, count, and foreground checksum matched in seven
alternating fresh-process pairs. Host-probe spread was 1.071x. Boot foreground
call/wall ratios were only 0.998x/1.002x against the required 1.10x, and
last-module readiness regressed to 0.489x control. Although Worker-local module
construction was faster, transfer, message delivery, and module clone more
than consumed that saving for the ordinary Boot population. Compile's larger
stream became ready 1.073x faster but left foreground call/wall effectively
tied at 1.005x/1.008x. Four warmup sequences also failed the predeclared
1.25x tier-stability check; no leg is replaced.

Do not implement the service, tune pool size, batch completions, change
foreground yield cadence, or introduce a module-size selector around the one
favorable large-module subcase. R048 already establishes that removing most
large-module frontend work is not a scored Compile wall-time win. A compiler
Worker remains permissible as an explicit UI-isolation embedder option, but it
is not a throughput optimization and cannot be the production default based
on this evidence. Production Wasm remains accepted R054 SHA
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.

## D071: reject a compact scalar interpreter step outcome

Date: 2026-08-08  
Status: rejected before production implementation

R058 tested the 24-byte Rust structure-return boundary visible between
accepted `Cpu::step` and `Cpu::run_until`. Its control writes and reloads the
normal `Ok(None)` tags in linear memory. Its candidate returns one i32 outcome
and writes exception sidecar state only on error. The frozen modules retain an
identical executed body and a separate non-inlined direct call; all normal,
stop, exception, checksum, and complete-memory results match.

Seven alternating fresh-process pairs measured 370.972 Mstep/s for sret and
177.587 Mstep/s for compact. Paired compact/sret throughput was 0.477x with
interval `[0.469,0.650]`, versus required 1.20x/1.15. Host spread was 1.015x
and construction negligible. One favorable compact leg caused the independent
warm-spread gate to fail, but was retained. Synchronous-tier, Liftoff,
forced-top-tier, and ten fresh non-inlining-traced diagnostics all preserved
the dominant reversal; none is substituted into the frozen report.

Do not implement outcome sidecars, change the public step contract, force the
10.9 KiB decoder into callers, retune warmup, or select the single favorable
engine leg. The native mechanism is engine-specific, but on the only target
that matters here—Wasm through V8—the scalar return is not a cheaper
non-inlined call boundary. Production remains byte-identical accepted R054
SHA `4160333352b18b...`.

## D072: reject flattened RV64C dispatch after the frozen stability failure

Date: 2026-08-08  
Status: rejected before production implementation

R059 flattened the complete three-quadrant/eight-funct3 compressed dispatch
without selecting an opcode population. Deterministic architecture-balanced
modules match exact state and full exported memory. Static shape proves four
control jump tables versus one and no helper call; fixed yields let both
functions reach TurboFan.

The mechanism is locally strong: seven fresh-process pairs measured 603.125
Mdispatch/s nested versus 960.376 flat, or 1.592x `[1.592,1.617]`, clearing the
1.45x/1.40 throughput gates. Host spread was 1.012x and cold cost negligible.
Nevertheless, the predeclared warm-stability gate failed when one flat leg
spread 1.301x across its four measured warm calls. Keep that observation and
the later stable steady samples; do not replace either.

The best-case local ratio projects only 1.130x whole Boot by assigning the
entire compressed share of `Cpu::step` to dispatch, although real handlers
retain fetch, operand, memory, ALU, PC, exception, and retirement costs. The
narrow true margin does not justify overriding an independent failed gate.
Do not implement, tune yields, weight the selector with Boot opcodes, or create
a replacement run. Production remains exact accepted R054 SHA
`4160333352b18b...`.

## D073: retain a prospective cumulative-gain track, but reject R070 residual-only activation

Date: 2026-08-09  
Status: standing policy accepted; candidate rejected at Gate A

The former 10% rule remains the inexpensive standard advancement track, but
is no longer a universal retention threshold. A general change that addresses
a failing product row may prospectively choose a cumulative track: five fresh
pairs, at least 3% paired median improvement with a non-regressing confidence
bound, tighter native/browser/full-scorecard guards, complete correctness, and
bounded maintenance/lifecycle cost. Results through R070 retain their original
decisions.

R071 applied that rule independently to the unchanged residual-only R070
static decoder. The same-Wasm five-pair report is valid and exact, but Boot is
1.024x `[1.000,1.050]` against the frozen 1.03x gate. Compile is 0.998x and
Python 1.032x. Reject without pooling the earlier sample, selecting raw-side
medians, or running later gates. Preserve the emitter/oracles and independent
WFI-yield fix as evidence; do not make this activation production default.

A successor may reuse the architecture-wide static decoder only if it emits
the page policy's exact observation contract from accelerated sampled
stretches. It may not skip sampling, hide instruction families, or select
benchmark PCs to inflate fast retirement.

## D074: reject R072 at the frozen browser guard despite its large native Boot gain

Date: 2026-08-09  
Status: candidate rejected; methodology issue retained separately

R072 satisfies the architectural successor requirement from D073. It executes
ordinary q1024 page-policy samples through one guest-independent static Wasm
decoder while preserving exact ordered observations, generated-entry stops,
interrupts, traps, slow-family fallback, and device behavior. The full strict
suite passes. Five fresh same-Wasm native pairs measure Boot 1.209x faster
`[1.190,1.239]`, Compile 1.021x, and Python 1.049x, with exact outputs and zero
errors. It clears the standing cumulative native gate by a wide margin.

Do not use the first browser sample for a decision. A newly introduced 1.25x
within-side spread limit invalidated it, and historical accepted R054 proves
that limit was not compatible with the established shared-9P workload. The
separately preregistered confirmation uses five wholly new pairs and preserves
the established validation method. It measures candidate/control elapsed
ratios 1.029x Python, 0.990x SHA-256, and 1.041x shared 9P. The last value fails
the prospectively frozen maximum 1.03x.

Reject R072 without pooling the favorable invalid sample, deleting the 1.268x
shared pair, running candidate/v86, changing the default, or entering the full
scorecard. Retain the decoder, correctness oracle, immutable artifact, and
reports as research evidence. The shared-9P interval `[0.829,1.268]` shows the
future gate needs redesign, but changing it after this outcome would be reward
hacking. Accepted production remains R054 and the parity goal remains open.

## D075: reject R073 at the strengthened browser guard and preserve the sampled-only causal result

Date: 2026-08-09  
Status: candidate rejected; default remains off

R073 enables only exact q1024 page-policy sampled execution in the static T0
decoder and leaves residual execution unchanged. It passes directed lifecycle
and Linux correctness, and five same-Wasm native pairs retain a large 1.157x
Boot gain `[1.137,1.187]` with guarded Compile/Python results. This establishes
that sampled execution, not R070 residual activation, is the useful mechanism.

The prospectively strengthened Chrome gate completed seven alternating pairs,
three synchronized repetitions per leg, without replacement. The valid report
`target/jit-policy-traces/wanix-r073-cb7ea816-chrome-20260809-config-ab/analysis.json`
has SHA-256 `9bbaf2cd89cc...`. Python and SHA-256 candidate/control ratios are
0.994x `[0.979,0.997]` and 0.999x `[0.984,1.003]`; shared 9P is 1.058x
`[0.932,1.114]`. Shared 9P exceeds both the frozen 1.03 median and 1.10 upper
limits.

Reject without rerunning, pooling R072, replacing any leg, making sampled-only
default, running candidate/v86, or executing the authoritative scorecard. The
post-run analyzer repair only replaces an impossible retained-tail assertion
with the exact frozen Alpine-root hash already captured in every summary; it
does not change data or performance criteria. A successor may gate sampled
execution on architecture-general density/lifecycle evidence, but may not use
guest PCs, binaries, devices, shell state, benchmark phases, or outputs.

## D076: reject R074 short-entry backoff at the valid browser gate

Date: 2026-08-09  
Status: candidate rejected; default remains off

R074 prospectively classified exact mapped entries after their first sampled
static stretch: fewer than 64 retired instructions marks an entry for later
accepted-interpreter samples. The rule reuses an architecture floor, treats
the PC only as an opaque cache key, and clears on dirty code/reset. Complete
semantic and Linux gates pass. Five native pairs retain a 1.154x Boot speedup
`[1.145,1.177]` while Compile 0.979x and Python 0.999x satisfy their guards.

The frozen schema-4 browser sample completes every one of seven alternating
pairs and three synchronized repetitions without replacement. Python and
SHA-256 candidate/control are 1.010x `[0.995,1.021]` and 1.002x
`[0.986,1.010]`; shared 9P is 1.068x `[0.959,1.122]`, above both the 1.03
median and 1.10 confidence-upper rules. Reject without rerun, threshold/key
sweep, candidate/v86, default promotion, or authoritative scorecard.

The negative result also rejects the causal hypothesis, not merely its point
estimate. Backoff reduces shared static sample calls from R073's 439,866 side
median to 29,656 and records 391,360 bypasses, yet candidate work remains 8%
higher and page modules rise from 6 control to 10 candidate while host 9P
service is equal. Do not pursue another per-entry sparse-stretch classifier.
Diagnose asynchronous page-module admission with a non-scoring fixed policy;
any successor must use general execution/lifecycle state and preserve the
accepted benchmark and output contract.

## D077: reject R075 preboot sampled static T0 because Chrome Boot is unchanged

Date: 2026-08-09  
Status: candidate rejected; default remains off

R075 gave R074 the exact lifecycle a production default would have: both legs
prepare one identical static decoder before `vm.start`, and candidate learns
short-entry backoff only through ordinary execution from firmware onward. The
two immutable pages differ by one adapter-only flag. Parser/default behavior,
both browser lifecycle smokes, full strict correctness, public/Worker APIs,
and sampled-backoff direct/OpenSBI Linux all pass.

The prospective schema-5 run completed seven alternating fresh Chrome pairs,
three synchronized repetitions per benchmark phase, and all fourteen legs
without replacement. Report
`target/jit-policy-traces/wanix-r075-preboot-28ceaf7b-chrome150-20260809-config-ab/analysis.json`
has SHA-256 `53d7233fb9dd...`. Shell control/candidate medians are
31,190.1/31,300.8 ms and paired speedup is 0.997x `[0.993,1.002]`, below the
frozen 1.10x and confidence-lower-1.00 requirements. Python 1.014x, SHA-256
0.996x, and shared 9P 0.871x all pass their nonregression guards.

Reject without candidate/v86, default promotion, scorecard escalation, or
reinterpreting the faster shared-I/O result as Boot progress. R075 proves that
preboot learning removes R074's I/O interaction, but it also precisely rejects
the hypothesis that the same 1.154x Node/native Boot gain transfers to Chrome.
Use fresh boot-scoped engine profiles only for causal attribution. A successor
must change the browser execution representation or remove a measured Chrome
runtime cost; another static threshold, backoff key, activation point, guest
selector, or lifecycle retest is inadmissible. Production remains exact R054.

## D078: accept R076 as mechanism evidence and admit one production escalation

Date: 2026-08-09  
Status: Chrome and candidate-v86 gates passed; not itself a production baseline

R075 measured complete WANIX launch-to-shell, not the emulator execution
boundary. R076 prospectively reproduced the authoritative scorecard timer in
fresh Chrome module Workers. Control and candidate use identical loader, main
Wasm, Linux 6.12.7 / Alpine 3.24.1 guest, one untimed auxiliary module, page
policy, pump cadence, and retired-work contract; only sampled/backoff execution
is disabled or enabled.

All fourteen legs completed once. The valid frozen analysis reports 1.175x
execution speedup `[1.167,1.189]` and 1.174x normalized-MIPS improvement
`[1.167,1.189]`, with about 180.35M instructions on each side and zero errors.
The separate fresh candidate-v86 WANIX guard passes Python, SHA-256, and shared
9P at 0.891, 0.632, and 0.669 RV64/v86 elapsed ratios; every bootstrap upper
bound is below 0.93.

Accept these as causal mechanism and product-guard evidence. They clear the
standing 5% cumulative-gain path and authorize exactly one production-default
integration, complete strict correctness run, and authoritative scorecard.
They do not supersede R054 before that final gate, do not revive R075's failed
launch-to-shell claim, and do not authorize a threshold/backoff/lifecycle
sweep.

## D079: reject R077 production promotion and restore default-off behavior

Date: 2026-08-09  
Status: rejected at frozen authoritative promotion gate; rollback verified

R077 integrated exactly the R076 sampled/backoff mechanism as a non-bare-metal
loader default. Stable/Worker/reset, adapter, directed differential, full
eight-stage, direct/OpenSBI, production Chrome, and final WANIX correctness
gates all pass. Its seven-pair production Chrome confirmation retains a 1.163x
execution/MIPS gain `[1.136,1.191]`, so the integration is real and correct.

The authoritative run completed all 117 prescribed trials without retry. It
is invalid because legacy HUFFMAN's three STEADY samples spread 1.425x, above
the unchanged 1.25x cap. Preserve that report and do not replace the single
leg. The frozen independent evaluator also supplies a separate decisive
rejection: raw rewrite Boot is 2,293.093 ms versus R054's 2,260.485 ms, a
0.986x baseline-to-current ratio instead of the required 1.05x speedup. Its
descriptive 11/13 v86 and 13/13 legacy match counts, plus all rows within 5%,
cannot override invalid measurement or failed Boot.

Reject R077 without changing the 5% threshold, pooling Chrome evidence,
replacing HUFFMAN, or rerunning for a favorable Boot. Preserve the exact
default-on loader/archive and reports by hash, remove automatic activation
from stable full-system and scorecard paths, and retain sampled/backoff only as
explicit research configuration. The rebuilt default-off archive and a fresh
no-override WANIX smoke prove module index -1, zero static activity, correct
Python, and ordinary generated execution. Accepted scorecard baseline remains
R054; Boot and Compile parity remain open.

## D080: accept R078's regression diagnosis and require source restoration

Date: 2026-08-09  
Status: diagnostic gate passed; source repair admitted

Five fresh alternating pairs prove exact R054 is 1.178x faster on Boot
`[1.116,1.190]`, 1.037x on Compile, and 1.031x on Python than the post-R077
default-off artifact. Every leg reports zero static activity and passes exact
identity, output, generated-execution, affinity, and spread checks.

Accept the causal conclusion that rejected dormant machinery materially
regressed production even while disabled. Do not install the archived R054
binary directly. Remove the rejected implementation from source, rebuild, and
require independent correctness and source-built performance gates.

## D081: reject R079 as an incomplete source restoration

Date: 2026-08-09  
Status: rejected at frozen restoration gate

R079 removes the obvious static-T0 emitter, scheduler, controls, counters, and
loader lifecycle, but its complete 30-leg report measures only 0.845x R054
Boot `[0.809,0.885]`, 0.952x Compile, and 0.968x Python. The valid failure is
not rescued by its smaller Wasm or similar symbol inventory.

Reject R079 without reinstalling rejected code or profiling it as a new
baseline. Audit exact session history for all R070/R072 residues and test one
fully predetermined cleanup. Preserve the independent WFI-yield fix.

## D082: promote R080 as the clean source-built baseline

Date: 2026-08-09  
Status: promoted; parity objective remains active

R080 removes the complete rejected fetch-context, helper, M/Bare probe,
fingerprint, differential-export, and loader-entropy residue. Full strict
correctness passes. The frozen 30-leg A/B passes the R054 restoration envelope
with Boot/Compile/Python speedups of 1.033x/1.013x/1.011x. The untouched
authoritative scorecard is valid, has no problem, and scores 13/13 versus
legacy and 11/13 versus v86. The five-pair Chrome guard passes Python, SHA-256,
and shared 9P at 0.875x, 0.608x, and 0.655x RV64/v86 elapsed time.

Promote Wasm `e5415db83b27...` and WANIX archive `414a17454216...` as the
current reproducible source baseline. This promotion restores trustworthy
implementation state; it does not claim parity. Boot remains 1.50x slower and
Compile 1.47x slower than v86.

## D083: use cumulative 3-5% promotion economics for mature optimization

Date: 2026-08-09  
Status: standing D073 policy reaffirmed

Do not require every future optimization to deliver 20%. Keep 10-20% only as
an inexpensive opportunity screen where implementation cost warrants it. A
general change may use the already prospective cumulative track: at least 3%
paired improvement on its target row, a non-regressing confidence lower bound,
five fresh pairs, complete correctness, tight non-target/browser guards, and
an untouched full scorecard before promotion. Three-to-five-percent general
gains are valuable because they compound; historical candidates retain their
original frozen decisions.

The lower gain floor does not loosen admissibility. No guest PC, symbol,
binary, workload, benchmark phase, checksum, browser, or compiler-output
selector may influence the optimization. Reject a small gain when its
maintenance, code-size, cold-latency, or variance cost outweighs the measured
benefit.

## D084: accept R081 attribution and reject tier-up forcing

Date: 2026-08-09  
Status: diagnostic complete; next architecture work admitted

R081's fixed inspector run attributes exact R080 Boot to 93.41% runtime Wasm,
with `Cpu::step` alone at 50.28%, and only 4.87% generated Wasm. Compile
STEADY is 52.45% runtime and 46.44% generated Wasm at 92.40% generated guest
retirement. The profiles reproduce the two remaining scorecard deficits and
show that neither is principally host compilation latency.

Native capture proves the dominant full-system interpreter function reaches
TurboFan. Reject forced V8 tier-up, engine-flag tuning, and a harness warmup as
solutions. Admit general full-system interpreter specialization under the 3%
cumulative track. Also admit one clean external/no-residue scalar-tier design
under the 5% substantial-tier rule: R076 measured its execution benefit, and
R078 independently measured the linked dormant runtime as the cost that erased
it. This causal change does not reopen the old linked emitter, sampled-entry
threshold, backoff, or activation experiments.

## D085: reject R082 at dormant-artifact Gate A

Date: 2026-08-09  
Status: rejected; remove candidate runtime and retain R080

R082 successfully moved the scalar emitter into a separate 217,556-byte
compiler Wasm and left only 9,204 bytes of capability/execution integration in
the main artifact. Focused units, exact q1/q32/q1024 policy differentials,
generated handoff, and modern direct/OpenSBI Linux all pass. Active mechanism
proof records tens of millions of external-tier instructions with zero errors.

The prospectively first performance gate nevertheless fails. Five alternating
exact-R080/dormant-capable pairs have neutral point estimates on Boot, Compile,
and Python, but Boot's paired interval is `[0.967,1.092]`, below the frozen
0.970 non-inferiority floor. The valid report has no problem and cannot be
replaced. Reject without active timing, threshold/lifecycle revision, browser
guard, or authoritative scorecard. Preserve both artifacts and the source
archive, remove every candidate runtime path, and rebuild exact R080.

Removal is verified complete: the rebuilt main Wasm is byte-identical to R080
at `e5415db83b27...` and 4,272,559 bytes, the loader is byte-identical at
`2cbb264f4dac...`, focused lifecycle/correctness checks pass, and the production
WANIX archive remains `414a17454216...`. The next admitted family is a small,
general full-system interpreter specialization under the standing 3%
cumulative-gain rules; R082's integration and threshold family stays closed.

## D086: reject R083 full-system const specialization

Date: 2026-08-09  
Status: rejected at native gate; cleanly removed

R083 implements exactly the R081-admitted small candidate: select full-system
versus user-only execution once and const-specialize the existing step,
compressed, translation, load/store, fused-row, and FP-state paths. It adds no
policy, cache, runtime switch, or guest selector. Its Wasm is 22,957 bytes
smaller than R080, and the full strict correctness matrix plus cold
construction gate passes.

The complete five-pair result is unfavorable and conclusive. Boot paired
speedup is 0.888x `[0.869,0.898]`, Compile 0.963x `[0.905,1.074]`, and Python
0.985x `[0.952,1.031]`, with exact outputs, no report problem, and host spread
1.021x. Reject without helper/inlining variants, browser escalation, or
scorecard. Preserve the artifact/source/report and restore exact R080.

The restored Wasm and loader are byte-identical at `e5415db83b27...` and
`2cbb264f4dac...`; production archive remains `414a17454216...`. Close this
specialization family. Admit no new implementation merely from a source-level
branch count; first require a closure-aware measured residual with at least 3%
whole-row leverage.

## D087: reject R093 at the frozen WANIX confidence gate

Date: 2026-08-09  
Status: rejected after Chrome; exact R085 restored

R093 proves that the standing cumulative policy is economically sound. The
general scalar Tier-0 loop improves same-Wasm Boot 1.045x `[1.029,1.080]`, the
default-on product improves native Boot 1.074x `[1.057,1.088]`, and fresh
Chrome improves 1.089x `[1.016,1.122]`. Compile and Python remain guarded, and
all instruction/full-system correctness tests pass. Therefore do not restore a
20% per-change requirement; reproducible 3--5% general gains remain eligible.

Promotion still requires every earlier frozen product gate. WANIX shared 9P
has a favorable 1.040x median but `[0.730,1.580]` confidence interval, missing
the 0.909 lower bound. Reject at that first failure without retry, threshold
change, family/mode variant, or full scorecard. Preserve all evidence and
restore exact R085 source, release Wasm, loader, and archive. This decision
closes the exact R066/R093 integrated-loop product under the current contract;
it does not erase the measured CPU-side opportunity or weaken the cumulative
gain policy.

## D088: qualify the R094 long shared-9P guard prospectively

Date: 2026-08-09  
Status: harness qualified; no product or score change

The old 4 MiB WANIX shared-9P phase produced sub-second samples and an interval
too wide to support a 3% cumulative-gain policy. Before testing another product
candidate, R094 froze a 32 MiB fixed-work replacement on two byte-normalized
pages that both bind exact R085. It retained three repetitions in each of seven
alternating fresh-browser pairs and added duration, P9 byte/transfer,
within-browser spread, generated-execution, and retirement-accounting proofs.

The null qualification passes without retries or threshold changes: 42/42
samples satisfy every proof, maximum local spread is 1.068x, paired median is
1.0004x, and the exact 95% interval is `[0.9984,1.0165]`. Adopt this exact
versioned guard for candidates admitted after R094. Do not alter the public
page, award performance credit, retroactively apply it to R093, or use its
success to reopen any rejected candidate. Exact R085 remains production and
the official result remains 11/13 versus v86.

## D089: remove R095's unsupported hard main-CODE cap before timing

Date: 2026-08-09  
Status: protocol corrected before candidate performance measurement

The original R095 draft imposed a 6,144-byte main-Wasm CODE-section ceiling.
That exact threshold had no isolated empirical foundation. R078 combined about
57 KiB of artifact growth with linked machinery and hot scheduler changes;
R082's 9,204-byte bridge had neutral performance point estimates and missed
only its uncertainty floor; and R083 proved that a 22,957-byte smaller artifact
can be materially slower. These results justify measuring construction and
execution cost, but not treating bytes as a linear performance proxy.

At user review, before any R095 timing, remove the hard cap. Continue to record
and attribute section deltas and enforce the architectural constraints: no
linked emitter/compiler, no runtime-generated executor, no duplicated main
orchestration loop, and no dormant experiment branches. The already frozen
cold-construction, same-artifact causal, native, Chrome, qualified WANIX, and
untouched-scorecard gates remain unchanged and decide whether the added code
is actually acceptable.

## D090: reject R095 at the same-main causal execution gate

Date: 2026-08-09  
Status: candidate rejected and archived for removal

R095 passed deterministic shape, artifact, construction, exhaustive ISA, full
system, lifecycle, and modern direct/OpenSBI Linux gates. Its immutable
274,473-byte auxiliary had no guest selector or runtime compiler, main CODE
grew only 8,484 bytes, and corrected module construction was 1.034x by side
medians (1.0499x paired), inside the frozen 1.05 gate.

The first actual execution measurement used the identical current main and
auxiliary on both legs and changed only the post-preparation enable cell. The
valid three-pair report (`2838073613dc...`, host spread 1.034x) measured Boot
0.944x `[0.933,0.949]`, Compile 0.975x paired, and Python 1.024x paired. All
outputs and lifecycle proofs passed; disabled retirement was zero and enabled
Boot retirement was about 107.23M fast plus 1.37M slow with zero errors.

This rejects the mechanism, not its correctness. Cross-instance typed calls,
539k sampled-policy records, and 3.51M interrupt polls replaced nearly all
Rust-interpreter retirement but still made Boot about 6% slower. Python's gain
cannot override the primary Boot gate. Do not run exact-R085 product, browser,
WANIX, or scorecard legs and do not tune callbacks, batching, layout, opcodes,
or thresholds. Archive exact evidence, remove the product path, and restore
R085.

Removal is verified rather than inferred: no active source/product reference
remains, the source-built executable CODE section and every defined function
body match R085 exactly, and the packaged artifact is exact `efd7830307...`.
The current R094-qualified WANIX adapter necessarily changes the tarball hash;
its rebuilt `6d28e87a...` archive contains no auxiliary module and embeds exact
R085 Wasm plus loader `2cbb264f4...`.

## D091: reject optional tail-chain accounting at the causal gate

Date: 2026-08-10  
Status: candidate rejected, archived, and cleanly removed

R096 tested a small architecture-wide cleanup without any byte or fixed
percentage gate. The same main Wasm emitted cross-module tail-transfer sites
with or without only the diagnostic `CHAIN_HOPS` load/add/store. A structural
test proved the remaining complete operator streams identical; directed
execution, the focused correctness matrix, workspace units, and modern
direct/OpenSBI Linux all passed.

Seven complete fresh pairs used one `05daf545b519...` artifact and varied only
the emission-time accounting cell. The valid report (`058b6183c223...`, host
spread 1.022x) measures Boot 1.005x `[0.999,1.009]`, Compile 0.991x
`[0.959,1.028]`, and Python 0.994x `[0.973,1.031]`. Compile therefore fails
both the positive-median and non-regressing-lower-bound gates. Reject without
a counter representation, sampling, aggregation, browser, or scorecard
variant.

Archive source/artifact/report/gate under
`target/bench/r096-tail-chain-accounting/` and remove every live switch, ABI,
stat, worker, test-mode, and emitter branch. This result is useful closure:
millions of diagnostic transfers are dynamically real, but their linear-memory
increment is not a material standalone cause of the remaining Compile gap.

## D092: close tail-chain metadata reuse at the engine opportunity gate

Date: 2026-08-10  
Status: opportunity rejected; no product change

R097 modeled the exact transfer predicate with identical locals and a
cross-instance Wasm memory barrier. Reusing the first generation/index loads
is a large Liftoff improvement: seven pairs measure 1.649x `[1.525,1.743]`.
The same immutable module under ordinary tiered V8 measures 0.998x
`[0.992,1.005]`. Every success/failure case and memory-barrier count matches.

The frozen gate required a positive ordinary-V8 point estimate because the hot
Compile modules naturally tier. It fails, so do not implement a runtime switch
or product candidate. The evidence strongly suggests TurboFan already performs
the local redundancy elimination. Forcing Liftoff or selecting by engine/tier
would optimize the harness rather than the portable emulator. Preserve the
proof-only emitter, harness, and `b05217f9088d...` report; production source and
artifact were unchanged.

## D093: reject the interrupt-deadline representation at the native gate

Date: 2026-08-10  
Status: candidate rejected, archived, and cleanly removed

R098 replaced the full-system interpreter's per-instruction interrupt
countdown decrement/store with a low-32-bit absolute retired-count deadline.
It preserved the exact 33-instruction poll stride and immediate trap, xRET,
and CSR rechecks.  The final candidate retained the original `Cpu` layout; an
untimed 64-bit pre-shape build that Rust reordered was excluded before causal
measurement.  All focused semantics, workspace, ISA, Spike, architecture
signature, Wasm/JIT, direct/OpenSBI Linux, and native virt-smoke gates pass.

The valid five-pair report (`6bda539f619e...`, host spread 1.014x) measures
Boot 1.020x `[0.988,1.032]`, Compile 1.008x `[0.974,1.089]`, and Python
0.948x `[0.930,1.053]`.  Gate `dd90ff8dd414...` rejects because Boot misses
both the standing 1.03x cumulative median and 1.00 lower-bound requirements,
and Python misses the 3% elapsed guard.  The candidate being 48 bytes smaller
does not enter the decision.

Stop before Chrome, WANIX, or the authoritative scorecard.  Do not sweep poll
interval, deadline width, modular comparison spelling, privilege mode, or
helper layout.  Archive exact source/artifacts/reports under
`target/bench/r098-interrupt-deadline/`, remove candidate CPU/tests, and
restore byte-exact R085-equivalent source build `d9f686a9...`.  This closes the
countdown-versus-deadline representation as a standalone optimization; it does
not change the official 11/13 parity status.

## D094: close dormant production region-policy machinery as a target

Date: 2026-08-10  
Status: diagnostic complete; no product change

R099 read existing counters outside the timed boundaries of exact
`d9f686a9...` Boot and Compile runs. All sampled-region exit, extension queue,
drain, demotion, batch, and indirect-cache-extension counts are zero in every
measured phase, despite 581,658 Boot and 533k--696k Compile outer dispatches.
The production page policy reaches the surrounding scheduler but does not use
those policy bodies.

Do not optimize region-exit sampling, feedback tables, batching, or extension
draining as a path to parity. Their branch shells may exist, but they lack the
dynamic operation volume required for a broad gain, and R091 already closed
whole-loop outlining. Preserve the observability fields in the scorecard; they
do not alter runtime behavior.

## D095: reject interleaved SIMD fused-TLB entries at the native gate

Date: 2026-08-10  
Status: candidate rejected, archived, and cleanly removed

R100 independently reconfirmed R037's architecture-wide interleaved
`{tag, offset}` representation under the current cumulative-gain policy. Exact
shape tests proved one `v128.load` replaces two scalar row loads, the complete
focused correctness matrix and modern direct/OpenSBI boots pass, and seven
fresh construction pairs tie at 1.003x candidate/control. The artifact was
608 bytes smaller; code size was diagnostic only.

The valid five-pair report (`e4ea8ac0c20b...`, host spread 1.015x) measures
Boot 0.989x `[0.956,1.018]`, Compile 1.017x `[0.966,1.083]`, and Python
0.986x `[0.961,1.075]`. Compile normalized MIPS agrees at 1.017x, so the small
point gain is genuine in direction, but it misses both the frozen 1.03 median
and 1.00 lower-confidence requirements. Boot and Python pass their 3% elapsed
guards.

Reject without SIMD spelling, alignment, row-sharing, mixed scalar/SIMD,
access-family, or workload variants. Stop before Chrome, WANIX, and the full
scorecard. Archive immutable candidate/source/reports under
`target/bench/r100-interleaved-tlb/` and restore all four source files plus the
release build exactly to `d9f686a9...`. This closes interleaved fused-TLB
loading as a standalone product optimization, not generated memory as a whole.

## D096: reject bounded structured fuel-check coalescing at the native gate

Date: 2026-08-10  
Status: candidate rejected, archived, and cleanly removed

R101 measured a legitimate broad opportunity: a conservative independent set
covered 37.210% of Compile STEADY structured-member entries and represented
59,494,284 removable Wasm operators. R102 omitted exactly those four-operator
post-member checks while retaining retirement, side exits, successor choice,
outer fuel boundaries, and the <=127-instruction overshoot. Exhaustive boundary
execution, complete correctness, and modern Linux gates passed.

The 14,623-byte-larger candidate passed seven-pair cold construction at 1.018x
candidate/control; size was diagnostic only. The valid 42-leg native report
then measured Boot 1.007x `[0.987,1.036]`, Compile 0.997x `[0.978,1.083]`, and
Python 1.016x `[0.999,1.033]`, with 1.071x host spread. Compile normalized MIPS
was independently 0.997x. It therefore fails the 1.03 median and 1.00 lower
bound target gates while both guard rows pass.

Reject without ordering, segment-length, successor, or opcode-spelling
variants. Stop before Chrome, WANIX, and scorecard timing. Archive immutable
evidence under `target/bench/r102-structured-fuel/`, remove all live candidate
code, and restore the release artifact byte-exact to `d9f686a9...`. Close
post-member fuel comparisons as a standalone optimized-engine target; keep
structured boundary/transfer work open only to independently measured new
mechanisms.

## D097: close fixed carried-GPR cross-module state before product work

Date: 2026-08-10  
Status: opportunity rejected; no product implementation

R103's proof-only census establishes that state materialization is dynamically
broad: 94.13% of Compile STEADY generated invocations are chained, and at least
331,208,236 GPR memory operations are rigorously chain-attributable. This
justifies a local engine test, not automatic product admission.

The frozen model carries all x1--x31 plus PC, cumulative retirement, and fuel
between two generated instances through one table-owning tail trampoline. Both
variants reach Liftoff and TurboFan and have byte-identical final state. Seven
alternating fresh-process pairs measure 0.9989x carried speedup with interval
`[0.9861,1.0533]`; median saving is negative and the conservative whole-Compile
projection is 1.0000x. Control spread also fails at 1.1876x versus 1.10.

Reject before adding any product wrapper, carried table, signature, or browser
path. Do not retry register counts, select ABI-popular registers, lengthen fuel,
force TurboFan, or use guest/workload identity. Archive the census/model and
remove diagnostic counters; exact `d9f686a9...` remains active. This closes a
fixed full-integer carried ABI, not every future approach to module boundaries.

## D098: use a verified one-percent cumulative-gain floor

Date: 2026-08-10  
Status: accepted prospectively

The mature rewrite may retain a general change whose target-row paired median
improves at least 1%, whose paired 95% lower bound excludes regression, and
whose fixed-work normalized throughput agrees. This replaces the former 3%
economic floor, not the correctness, generality, cold-lifecycle, browser,
WANIX, or untouched-scorecard gates. Protected native rows may not regress more
than 1% by paired median, and an interval establishing regression rejects the
candidate.

Five or seven pairs are not automatically adequate for a 1% signal. Use a
prospective control/control power calibration, longer fixed work, or one frozen
maximum-sample extension. An unresolved maximum sample is inconclusive, not an
accepted gain and not proof that the mechanism is neutral. Do not use optional
stopping, replace legs, pool old experiments, or select a favorable statistic.

Code and section sizes remain diagnostics. Actual construction, startup,
memory, and execution effects decide cost; no fixed byte count rejects a
candidate. Full policy and historical audit are in R104.

## D099: admit one fresh integrated scalar-T0 reconfirmation

Date: 2026-08-10  
Status: completed and superseded by D100

The historical audit identifies R071 as the clearest old threshold casualty,
but R078 later proved its surrounding linked static machinery was not a safe
product baseline. R093 is the clean mature successor: it passed exhaustive
correctness, native Boot, and Chrome Boot and stopped only at the obsolete
short-work shared-9P confidence gate. R094 independently qualified a stable
32 MiB replacement before any new candidate.

Admit exactly one R105 reconstruction from archived source against exact
`d9f686a9...`. This explicitly supersedes D088's no-reopen clause only for the
frozen R105 experiment. No R093 timing enters the decision, and no family,
mode, cadence, threshold, helper, or workload variant is allowed. R105 must
pass fresh correctness, cold, seven-pair native, product, Chrome, qualified
WANIX, and untouched-scorecard gates under D098 or be removed completely.

## D100: reject R105 on protected Compile and Python regressions

Date: 2026-08-10  
Status: candidate rejected, archived, and cleanly removed

R105 reconstructed the exact archived integrated scalar Tier-0 executable
against current `d9f686a9...`, with no family, privilege, threshold, cadence,
or workload variant. Exhaustive scalar/RVC, M/A/FP, system-memory, Sv39/MPRV,
WFI, T2, public/Worker, and fresh direct/OpenSBI Linux correctness passed.
Seven-pair cold construction also passed at `0.9893x` candidate/control despite
adding 39,234 module bytes; byte size was diagnostic only.

The valid 42-leg same-Wasm gate (`1ee0190a3521...`, host spread `1.0212x`)
measured Boot `1.0588x` `[1.0360,1.0834]`, Compile `0.9803x`
`[0.9551,1.0079]`, and Python `0.9792x` `[0.9600,0.9939]`. Boot normalized
MIPS independently agrees at `1.0589x`. Boot therefore clears the new 1%
target rule, but both protected medians breach `0.99x`, and Python's entire
interval establishes regression.

Reject at the preregistered native gate. Do not build a default-on product,
run Chrome/WANIX/scorecard, select a privilege or workload subset, or retry the
sample. Preserve R105 evidence, remove candidate/proof code, and restore exact
R085-equivalent source/release identities. This demonstrates that D098 accepts
small verified gains but does not call a single-row gain net positive when
other protected product work becomes measurably slower.

## D101: admit one indivisible scalar/publication pipeline

Date: 2026-08-10  
Status: completed and superseded by D102

R105's failure counters provide a causal integration clue: the scalar tier
improved Boot `1.0588x`, but median residual interpreter retirement rose
`1.0122x` in Compile and `1.0555x` in Python while generated retirement fell.
R064's already-frozen pending-publication boundary is the exact complementary
mechanism: it reduced residual interpreter retirement to `0.8944x` and
`0.6269x` in those rows and improved them `1.0494x` and `1.0544x`, at a
`0.9758x` Boot cost.

The deterministic R106 admission proof (`aaf86c4afff5...`) gives independent
median-product projections of `1.0333x` Boot, `1.0287x` Compile, and `1.0324x`
Python. These projections award no timing credit, but establish an
architecture-wide reason to test exactly one composition under D098.

Admit the exact R105 scalar driver plus exact R064 pending checks as one
indivisible same-Wasm candidate. Control has both proof switches off;
candidate has both on. No component legs, selector, threshold, quantum,
privilege split, sample extension, or post-result variant is allowed. R105 and
R064 remain rejected individually. The combination must pass the complete
R106 correctness, cold, seven-pair native, product, Chrome, qualified WANIX,
and untouched-scorecard sequence or both mechanisms are removed together.

## D102: reject R106 at its frozen cold-construction boundary

Date: 2026-08-10  
Status: candidate rejected, archived, and cleanly removed

R106 exactly composed the archived R105 scalar driver and R064 pending-return
boundary. Its admission proof, source subtraction, ABI/disassembly shape, full
candidate-on differential matrix, public/Worker APIs, and direct/OpenSBI Linux
6.12.7 boots all pass. The candidate added 38,127 module bytes and 36,807 CODE
bytes, but those sizes were diagnostics rather than rejection criteria.

The preregistered seven-pair construction gate measured `1.0511625x`
candidate/control against its `1.0500000x` maximum. Paired median was
`1.0515631x` with interval `[1.0102710,1.1289065]`. Although the numerical miss
is only 0.116 percentage points and the samples are noisy, changing the
statistic, limit, or sample after seeing it would be optional stopping. Reject
at the first failed gate, collect no native Boot/Compile/Python timing, remove
both mechanisms, and restore exact `d9f686a9...`.

This result is not evidence that a measured 1% runtime gain was discarded; the
candidate never reached runtime causality. Future cold-cost protocols must
predeclare a confidence or amortization rule with adequate power. They may not
retroactively reclassify or rerun R106.

## D103: debit excluded main-runtime construction in absolute milliseconds

Date: 2026-08-10  
Status: accepted prospectively; no candidate credit

Future candidates use R107 instead of rejecting on a percentage change in a
small standalone construction event. Fifteen alternating fresh-process pairs
measure the real `RV64Debug.create` product path. The upper 95% bound of the
paired median candidate-minus-control construction delta, clipped at zero, is
added once to every candidate runtime sample before applying R104's 1% target
and protected-row rules. Cold improvements are recorded but cannot rescue a
runtime regression.

The exact-baseline control/control calibration measures 20.629/20.518 ms and
produces only a 0.155 ms conservative false debit. Generated-module compile,
instantiate, and publish time remains inside workload timing and is never
charged twice. Surviving candidates must also retain execution-only and
construction-to-marker clocks in a fresh Chromium Worker, followed by the
qualified WANIX and untouched scorecard gates.

R106 remains rejected under D102: its code is gone and it has no native timing.
R107 is prospective evidence policy, not permission to reinterpret or rerun a
known artifact. Full rule and calibration are in
`R107_AMORTIZED_MAIN_RUNTIME_COLD_COST_POLICY.md`.

## D104: capture and model one dense CFG stackifier

Date: 2026-08-10  
Status: proof-only experiment admitted

R108 finds a live bounded compiler cost that can clear the new 1% floor.
Structured CFG and ordered-tree operations own 2.316% of Boot and 1.139% of
Compile STEADY. Region indices are already dense and capped at 512, so a dense
bit matrix is a representation correction rather than a policy or workload
specialization. A 1.80x local result projects just over 1% whole Boot.

Admit R109's exact production-graph capture and one tree-versus-dense model.
Product work is forbidden until complete structure equality and the frozen
local speed gates pass. The independently positive typed `InstructionSink`
model is closed standalone because its directly removable sampled ceiling is
only 0.558% Boot / 0.875% Compile; do not bundle it to manufacture threshold
clearance. Full frozen sequence is in `R109_DENSE_CFG_STACKIFIER_PROTOCOL.md`.

## D105: reject R109 at the verified-1% native gate

Date: 2026-08-10  
Status: completed; candidate removed; baseline restored

R109 passed its prospectively frozen production-corpus and model admission.
The dense representation was 5.264x/6.628x faster on Boot first/steady and
5.052x/5.874x on Compile, with exact structure bytes across 14,931 graphs.
Complete candidate correctness passed, including direct/OpenSBI Linux, and all
280 deterministic real-RV64 modules regenerated byte-identically.

Fifteen real construction pairs produced a conservative 1.041935 ms debit.
The valid 90-leg native report then measured adjusted Boot 1.01522x
`[0.99726,1.02755]`, Compile 0.98926x `[0.96237,1.01259]`, and Python 1.00453x
`[0.98732,1.02888]`, with 1.074x host spread. Boot normalized MIPS agrees at
1.01507x. Boot clears the 1% median but not the non-regression lower bound;
Compile misses its protected 0.99 median.

Reject at Gate D. This is not a rejection because the gain is only 1--2%, nor
because the candidate is 48,578 bytes different (it is smaller). It is the
mechanical result of the confidence and protected-workload rules that make a
small gain verifiably net positive. Do not collect extra pairs, try a hybrid
or alternate bit width, alter SCC order, or compose the R108 sink. Stop before
Chromium, WANIX, and scorecard. Archive evidence under
`target/bench/r109-dense-cfg/`, remove the product candidate, and restore exact
`aec4b314...`, `1da35e70...`, `d9f686a9...`, and `2cbb264f...` identities.

## D106: admit one same-module partition model from native spill evidence

Date: 2026-08-10  
Status: proof-only experiment admitted; no product change

R110's single exact-modern Compile collection is valid for attribution and
invalid for elapsed-time comparison by design. The complete corrected report
maps 4,506 hardware-cycle samples to the JIT path, separates the shared tail
trampoline, and finds TurboFan responsible for 91.76% of the mapped period.
Explicit native-stack reads/writes own 22.44% of guest-body cycles. Combined
with R088's independent generated-execution fraction, this is an 8.87%
whole-Compile exposure ceiling. Period-weighted native frame size is 323.84
bytes and positively tracks stack-cycle share.

This evidence admits R111's proof-only comparison of the current one-function
region against one bounded same-module multi-function partition. Partitioning
must be determined entirely from static architecture-general CFG/liveness
properties and preserve exact work. Its ordinary optimized-V8 result must show
both smaller native frames/stack exposure and enough net execution improvement
to project at least 1% whole Compile after added boundaries.

R110 does not admit product work, earn parity credit, or reopen source-local
reuse (R039), fixed full-GPR carrying (R103), per-module table ownership (D054),
or forced-engine-tier variants. If the frozen local model fails, stop without
trying a different partition width, state subset, boundary ABI, graph class,
or workload selector. Exact `d9f686a9...` remains the product baseline.

## D107: close the frozen same-module partition at its static gate

Date: 2026-08-10  
Status: completed; no model timing or product change

R111's deterministic analyzer reproduced all existing module identities and
applied the frozen SCC/32-member/24-state rule once to every production graph
and real region. It passed the local opportunity measures: 91.4146% eligible
bytes split, the weighted state ratio is 0.629014, and the weighted estimated
maximum-local reduction is 0.160587.

It fails three prospective Gate-A requirements. Boot cuts 52.8772% of static
edges and Compile cuts 42.6839%, each above 12.5%. Oversized atomic SCCs occupy
60.9966% of eligible bytes, above 20%. Therefore reject this exact rule before
constructing Gate B, collecting Gate C, editing the product, or gathering any
candidate elapsed time.

The failure is structural: the rule makes a much smaller frame estimate by
turning nearly half of ordinary control-flow edges into explicit state/function
boundaries, while leaving many large loop SCCs indivisible. Do not try another
member/state cap, SCC/order variant, state ABI, or selected graph/workload.
Archive the deterministic evidence under `target/bench/r111-partition-model/`
and retain the product baseline `d9f686a9...` unchanged.

## D108: close jitdump source attribution for generated Wasm

Date: 2026-08-10  
Status: diagnostic completed; no product change

R112 parses the preserved R110 dump rather than recollecting samples. Two
byte-identical final reports validate 250/250 debug records and 6,007 entries,
reproduce every R110 period partition, and show zero ambiguous relevant joins.
All debug records describe JavaScript/Node code; none associates with a sampled
generated-Wasm code load. TurboFan guest-body and explicit-stack non-sentinel
source coverage are therefore both exactly 0%, versus 90% gates.

Close the jitdump source-position route. Do not rerun with another perf/debug
flag, engine, workload, or selected load, and do not infer that JS line fields
are Wasm offsets. No operator-level R113 is admitted. A new diagnostic may use
the existing native stack-slot/control context, but it cannot modify the
product or reopen R111 merely because source mapping is absent. Exact
`d9f686a9...` remains active.

## D109: close native-form specialization; calls are not the spill source

Date: 2026-08-10  
Status: diagnostic completed; no model or product change

R113 closes all R110 sample and period totals, then partitions every TurboFan
guest stack access. Call neighborhoods own 0%; entry/control/general contexts
own 14.29%/32.66%/53.05%. Register reloads are broad across 70 loads and have a
4.196% optimistic whole-Compile exposure, but record 464 samples versus the
prospectively frozen 500-sample requirement. Immediate comparisons and spills
remain below the 2% exposure floor and also below the sample floor.

Admit no form-specific model. Do not lower the sample count, combine forms, or
select loads after the result. This does not discard a measured >1% gain: no
candidate was implemented or timed, and exposure assumes complete removal.
Close helper/call-boundary spill work because its measured opportunity is zero.
The distributed-pressure result may motivate a separately frozen
within-function structured-state SSA model, provided it is explicitly distinct
from R039 local reuse, R103 cross-function carrying, and R111 partitioning.
Exact `d9f686a9...` remains active.

## D110: reject R114 after fresh verified-one-percent reconstruction

Date: 2026-08-10  
Status: rejected at native gate; candidate removed

R014 was legitimately worth retesting because the former 10% threshold could
discard useful cumulative improvements. R114 therefore reconstructed exactly
one independent component—lazy architectural-PC materialization across covered
structured edges—on the current baseline. It did not reuse old timing, sparse
safepoints, coverage strata, or a benchmark/PC selector.

Structural, correctness, and modern-Linux gates pass. The frozen construction
report charges 1.432463 ms to each candidate sample. The valid 15-pair-per-row
native gate measures adjusted Boot `0.99474x` `[0.97790,1.00430]`, Compile
`0.98579x` `[0.95202,0.99629]`, and Python `1.00438x`
`[0.99406,1.02019]`; adjusted Compile normalized work is `0.98577x`.
Generated coverage is matched closely in every row.

Reject because Compile fails the 1.01 median, 1.00 lower-bound, and 1.01
normalized-work rules, with its upper interval establishing regression. This
decision is unrelated to the candidate's source or Wasm size. Do not add
samples, tune a second PC placement, or run product/browser/WANIX/scorecard
gates. Preserve the immutable evidence under
`target/bench/r114-lazy-internal-pc/`, remove all candidate plumbing, and
restore exact `aec4b314...`, `1da35e70...`, `2cbb264f...`, and
`d9f686a9...` identities.

Retain D098's prospective policy: a future architecture-general candidate with
a verified net gain above 1% and intact protected rows is acceptable even if
it is far below 10% or 20%. Historical point estimates alone are not.

## D111: retain the R095 boundary finding and reject its embedded executor

Date: 2026-08-10  
Status: historical audit completed; no product change

R115 tested one missing causal variable in R095: Wasm instance ownership. It
relocated the exact already-validated executor into the R095 main module and
mapped its helper imports to same-module direct calls. The valid seven-pair
boundary report measures Boot `1.03413x` `[1.01796,1.04451]`, Compile
`1.02349x` `[0.99847,1.03595]`, and Python `1.00661x`
`[0.95828,1.02686]`. Therefore record cross-instance execution as a proven
3.4% Boot tax and avoid that packaging in future Wasm-only designs.

Do not promote or reconstruct the executor. The prospectively frozen
enabled-versus-disabled screen on the identical embedded artifact measures
Boot `0.97987x` `[0.95574,0.99572]`, Compile `0.99715x`, and Python
`1.00955x`. One Compile control spread invalidates that row for positive
admission; independently, Boot's stable complete sample establishes a
protected-row regression and no open row clears the verified-1% rule.

Reject before current-product integration, construction debit, Chromium,
WANIX, and scorecard. Size is not a decision criterion. Preserve the linker,
artifact, raw reports, and gate under
`target/bench/r115-same-instance-proof/`; the live product was untouched and
remains exact `d9f686a9...`.

## D112: close zero-extra-work selective state residency at Gate A

Date: 2026-08-10  
Status: static opportunity failed; no product change

R116 applies one frozen selector-free rule to every deterministic real region:
only architectural state referenced in exactly one acyclic member becomes
materialized. This preserves the current per-invocation upper bound on state
loads/stores and creates no CFG or function boundary.

The census is exact and deterministic. Eligible regions represent 77.5156% of
eager bytes, with zero cyclic-cold or operation-bound violations. But weighted
resident state is 91.7212% of the current union versus an 80% ceiling, and
total local footprint falls only 2.0376% versus the 5% minimum.

Reject at Gate A. Build no model or candidate, collect no elapsed timing, and
do not widen coldness to multi-member/cyclic values or select registers after
the census. Evidence is retained under
`target/bench/r116-selective-state/`; exact `d9f686a9...` remains active.

## D113: reject module-global architectural state at Gate A

Date: 2026-08-10  
Status: proof-only model rejected; no product change

R117 compares one normalized-equivalent structured kernel with all 31
long-lived i64 state values in function locals or private mutable globals.
Deterministic shape evidence proves one function and 963 operators on each
side, with exact indexed correspondence for 160 state reads and 95 writes,
identical memory/control work, no candidate state locals, and exactly 31
candidate mutable i64 globals.

Every one of 30 fresh CPU-pinned legs passes output, work, artifact, affinity,
host, and spread proofs. The frozen 15-pair result measures global/local
steady speedup `0.970330x` `[0.967047,0.973196]` and first-execution speedup
`0.826972x` `[0.814919,0.842724]`. Thus private globals are an established
3% optimized regression and a much larger first-execution regression, not a
route around V8's local-state frame pressure.

Reject before native-code capture or product work. Do not test partial global
sets, select registers, or change the kernel after observing the result.
Artifact size is diagnostic only. Preserve the builder, normalized streams,
raw report, and hashes under `target/bench/r117-module-global/`; the live
product remains exact `d9f686a9...` and the official score remains 13/13
versus legacy and 11/13 versus copy/v86.

## D114: reject flat current-product RVC dispatch at native Boot

Date: 2026-08-10  
Status: rejected at native Gate B; candidate removed

R118 fairly reconstructs the R059 mechanism because R059's former rejection
incorrectly treated a warmup tier-publication event as unstable steady state.
No old timing earns credit. The sole current candidate combines the complete
quadrant/funct3 selector without an opcode, PC, workload, or engine selector.

Deterministic shape, exhaustive semantics, full release correctness, and both
modern Linux paths pass. Fifteen construction pairs impose a conservative
1.225862 ms debit. The stable Boot subset then measures adjusted
`0.982183x [0.968403,0.993110]` and normalized `0.982098x`; even its upper
confidence endpoint establishes regression. Compile is `0.996783x`, Python is
`1.022471x`, and one Compile control spread exceeds 1.25, independently
forbidding positive use of the complete report.

Reject before Chromium, WANIX, `/shared/bench.py`, and scorecard. Do not rerun,
replace the Compile sample, reorder families, alter selector spelling, or
compose another mechanism after seeing the split row result. The candidate's
1,354-byte growth is not a criterion. Preserve evidence under
`target/bench/r118-flat-rvc/` and restore exact CPU/runtime/loader/release
identities `aec4b314...`, `1da35e70...`, `2cbb264f...`, and `d9f686a9...`.

Retain the verified-one-percent policy. R118 confirms an old experiment could
deserve a corrected retest even when it ultimately fails; it does not justify
retroactive adoption of any historical artifact.

## D115: accept verified one-percent gains and preserve old small leads without retroactive credit

Date: 2026-08-10  
Status: standing policy clarified; no product change

The ledger-wide audit finds that legacy E005b and E006b were stopped after
favorable one-pair Compile points of `1.045x` and `1.026x` because the then
standing rule treated every result below 10% as a tie. The old cutoff was too
coarse. General end-to-end improvements at or above 1% are worth retaining and
may compound; source or Wasm byte growth alone is not a rejection criterion.

Do not relabel either E-series result as accepted. A single pair is not a
verified gain, neither experiment protected Boot/Python/browser/WANIX, and both
belong to the deleted legacy backend. The current rewrite page policy does not
execute E006b's ordinary tier-up path, and its invocation cache is not E005b's
per-trace cache. Historical timing supplies motivation only.

For all future candidates, `net >=1%` means the target paired median is at
least `1.01x`, its 95% lower bound is at least parity, normalized fixed work
agrees, protected medians remain at least `0.99x` without confidence evidence
of regression, actual construction is debited, and every correctness,
Chromium, qualified WANIX `/shared/bench.py`, and untouched-scorecard gate
passes. An underpowered favorable result is inconclusive and eligible only for
its prospectively frozen extension; it is neither accepted nor evidence that
the mechanism is worthless.

## D116: reject R119 at its frozen native gate, not for being small

Date: 2026-08-10  
Status: candidate rejected and exact baseline restored

Accept R119's correctness and causal evidence: reusing the existing execute-
TLB proof removes the intended physical-bus hit subtree, and the implementation
passes the complete strict suite. Do not accept it as a product optimization.
After its 1.258935 ms construction debit, Boot's `1.012411x` median has lower
95% bound `0.997859x`, while protected Compile has median `0.984634x`.
Python is protected at `1.001649x`; every report-integrity guard passes.

Apply the prospectively frozen decision without a rerun, sample extension, or
post-result helper/layout variant. Stop before Chromium, WANIX, and scorecard
qualification; preserve evidence under
`target/bench/r119-existing-probe-opportunity/`; remove the candidate; and
restore exact `aec4b314...` / `d9f686a9...`.

The 3,187-byte Wasm growth is diagnostic only. The positive Boot point is not
rejected because 1.24% is too small; it is rejected because its interval does
not verify parity and the same candidate fails a protected median. Retain the
standing rule that a confidence-verified net gain of 1% is promotable.

## D117: reject exact R100 after independent R120 one-percent reconfirmation

Date: 2026-08-10  
Status: candidate rejected at frozen native gate; live product unchanged

R100 was legitimately disadvantaged by the old percentage rule: its exact
interleaved fused-TLB candidate had a five-pair `1.017x` Compile point while
the gate demanded `1.03x`. Reconsider it once under R104 using exact immutable
bytes, a prospectively frozen 15-pair rule, and no pooling of old samples.

Authenticity and construction pass. Two isolated source builds reproduce
`c36da489...`; the candidate is 608 bytes smaller, which is diagnostic only.
Fifteen construction pairs impose a conservative 0.231087 ms debit. All 90
native legs and integrity guards pass, but adjusted Compile is only
`0.992069x [0.952178,1.015084]`, normalized work is `0.992040x`, and protected
Python is `0.982841x [0.954431,1.033982]`. Boot is safe at
`1.003286x [1.001075,1.013737]` but cannot rescue the failed Compile target.

Reject without reapplying source or running Chromium, WANIX, or scorecard.
Do not extend, rerun, pool, or try a SIMD/scalar, alignment, width, selector,
access-family, or workload-derived variant. Preserve evidence under
`target/bench/r120-interleaved-one-percent/`; exact live product remains
`d9f686a9...`.

Retain R104 unchanged. A gain above 1% is accepted only when it is a verified
net result with protected rows intact—not merely when a small point estimate
exceeds 1%.

## D118: close runtime dispatch-miss variants below the verified net floor

Date: 2026-08-10  
Status: diagnostic complete; no product candidate admitted

R121's exact preserved-native closure assigns 0.7481% of main-thread period to
the complete fallback cache lookup, mapping proof, and refill path. The sole
counter build shows that STEADY fallbacks are frequent—61.71% of outer
visits—but divide into 61.21% absent entries and 37.45% stale generations.
Only one of 213,808 compiled-block proofs actually drops a mapping.

Do not implement negative absence lines, a parallel PA capability, or a
combined richer dispatch row. Each removes only a subset of a below-1% complete
band while adding metadata traffic and publication/collision obligations; late
one-second samples have no authenticated phase boundary and cannot override
the whole collection. This is a prospective exposure rejection, not a
code-size veto and not a rejection of a measured positive product result.

Retain the permanent scorecard readout of existing MMU counters 89--95, remove
all diagnostic hot-path counters, archive evidence under
`target/bench/r121-runtime-dispatch/`, and restore exact `1da35e70...` source
and `d9f686a9...` release. Continue attribution on an independent active cost.

## D119: close R122 interpreter-body leaves without inventing an opcode-specific candidate

Date: 2026-08-10  
Status: diagnostic complete; no product candidate admitted

R122's immutable native analysis closes all optimized `Cpu::step` period and
assigns 30.2562% of main-thread period to compressed plus RV32 semantic bodies.
The population is large, but its five basic blocks above the prospectively
frozen 1.25% evidence floor are required dense opcode assembly/dispatch,
quadrant-2 family selection already closed by R118, accepted R054 scalar
memory work, or a mixed quadrant-1 decode whose removable subset has at most a
0.4216% proportional main-thread ceiling.

The sole modern Linux 6.12.7 / Alpine 3.24.1 count build closes exactly:
108,693,790 interpreted and 71,704,074 generated retirements equal the
180,397,864 guest total. Compressed/32-bit and sequential/non-sequential
subtotals independently close. Only 730,168 of 66,630,535 GPR-write-helper
calls discard x0. Neither clearing x0 after every instruction nor selecting a
33rd scratch slot removes that branch without adding more architecture-wide
work.

Admit no product candidate and do not use opcode or compressed-family
frequency to choose a decision-tree/order/spelling variant. This is not a
measured sub-1% rejection: the instrumented elapsed values are ineligible and
no candidate is timed. Remove all counters and temporary harness/export code,
preserve evidence under `target/bench/r122-interpreter-body/`, and restore exact
core/Wasm/loader/release identities `aec4b314...`, `1da35e70...`,
`2cbb264f...`, and `d9f686a9...`.

Retain D115/R104 unchanged. A real correctness-safe, construction-debited,
confidence-verified net gain of 1% remains promotable regardless of code-size
growth; R122 simply does not identify one.

## D120: close the fused-memory runtime guard below current exposure

Date: 2026-08-10  
Status: diagnostic complete; no product candidate admitted

R123 authenticates the production `interpreter_fused_memory` load, comparison,
and branch in all optimized scalar memory bodies. Their complete guard blocks
own 11,023,476 period units, or only 0.104231% of Boot main-thread period.
This already overcredits an unrelated instruction in the broad `ld1` block;
impossible complete removal projects just `1.001043x` whole Boot.

Do not replace the runtime diagnostic flag with a compile-time Bus capability.
Do not combine the once-per-host-call setter with unrelated scheduler work to
cross the admission floor. No product candidate was implemented or timed, so
this is neither a rejection of a measured small gain nor a byte-size decision.
Preserve R123 evidence, retain exact `d9f686a9...`, and continue from generated
region state pressure or another independent cost with credible verified-1%
exposure.

## D121: prevent proxy gates from reinstating a coarse promotion floor

Date: 2026-08-10  
Status: standing policy clarified before R124 dynamic evidence

Accept the owner's clarification that every general, correctness-safe,
construction-debited, confidence-verified net product gain of at least 1% is
worth retaining. Code or Wasm size is diagnostic, and failure to produce a
5%, 10%, or 20% improvement is not a rejection reason.

Extend R104 accordingly: static exposure, operation counts, models, and native
shape may stop a candidate when they prove a complete sub-1% ceiling,
correctness/architecture failure, measurement invalidity, or an established
regression. They may not veto a bounded implementation merely because a proxy
misses a larger percentage target while a 1% end-to-end gain remains
plausible. Actual Gate-C product measurements remain authoritative.

Apply this prospectively to R124 after A1 passed at 13.2938% and before any A2
counter execution, model timing, native capture, product implementation, or
product timing. Preserve the original A2/B targets as reported diagnostics;
do not tune the frozen RV64C bank. This is a policy correction independent of
an observed R124 runtime result.

## D122: archive R124's topologically invalid model and measure the exact product

Date: 2026-08-10  
Status: product implementation admitted; no model performance credit

Preserve the sole frozen Gate-B result exactly as observed: hybrid steady is
`0.966035x [0.962105,0.967411]`, while every internal integrity check passes.
Do not use it to reject the product mechanism.  Post-result inspection proves
that the model placed 65,536 architectural rounds inside one function call,
so eager state paid 62 boundary operations once while hybrid paid 84
materialized operations per round.  This approximately `88,791x` relative
state-traffic inversion contradicts the product boundary topology and A2's
measured `0.737698x` STEADY projection.

Do not repair or rerun a synthetic model after observing this failure.  Under
D121, treat it as invalid proxy evidence and proceed with the single frozen,
architecture-general x1/x2/x8--x15 product implementation.  The exact product
correctness and construction-debited runtime gates are authoritative.  Retain
only if the net Compile gain is confidence-verified at least 1%, protected
rows hold, and the downstream Chromium, WANIX, and untouched scorecard gates
pass.  Code and Wasm size remain diagnostics only.

## D123: retain R124 through the exact native gate

Date: 2026-08-10  
Status: native promotion passed; browser/WANIX/scorecard qualification pending

Retain exact candidate `d017a10f...`.  The prospectively frozen 15-pair gate
passed all 90 legs and every integrity check with a `0.168840 ms` construction
debit and `1.054948x` host spread.  Debited Compile is `1.083675x
[1.037357,1.112250]` with normalized fixed work `1.083602x`.  Protected Boot
is `1.018471x [1.000973,1.035481]`; protected Python is `1.200538x
[1.180720,1.220097]`.

This comfortably satisfies the verified-one-percent policy without relying on
candidate size, A1/A2 projections, or the invalid synthetic model.  Freeze the
exact source/Wasm and advance it without changing the bank or composing
another mechanism.  It becomes product only after natural Chromium,
R094-qualified WANIX including unchanged `/shared/bench.py`, and the untouched
117-trial modern three-way scorecard retain all required rows.

## D124: retain R124 through the natural-Chromium dual-clock gate

Date: 2026-08-10  
Status: Chromium promotion passed; WANIX/scorecard qualification pending

The sole frozen seven-pair Chromium run passes. Execution-only Boot is
`1.018971x [1.001431,1.037301]`; construction-to-ready is
`1.017013x [0.998068,1.029593]`. The first result verifies the gain in V8, and
the second rules out an established inclusive-construction regression under
the prospectively frozen rule.

Retain exact `d017a10f...` without modification and advance it to the frozen
R094-qualified WANIX product guard. Do not pool browser and native samples,
attribute additional target credit to the browser check, or alter the fixed
register bank. WANIX must independently protect shell, unchanged Python,
SHA-256, and 32 MiB shared 9P before the untouched scorecard may run.

## D125: invalidate the R124 WANIX run and single-flight only its WANIX endpoint

Date: 2026-08-10  
Status: runtime correction qualified; from-zero replacement gate frozen

The sole frozen R124 WANIX run is invalid and supplies no product credit. Pair
5 exact control reached the shell, completed correct Python and SHA-256, then
timed out in the first 32 MiB shared-9P sample. No completed leg is replaced or
pooled.

Diagnostic-only evidence closes the console hypothesis. The visible release
token arrived; exact write and read work crossed the emulator; guest
instructions continued to retire; and WANIX never replied to one decoded
`T_UNLINKAT` request for the temporary file. The failure is therefore in the
common external-9P integration, not evidence that candidate `d017a10f...`
regresses.

Retain generic loader multiplexing, but deliver requests FIFO single-flight
through the WANIX adapter's stream-backed Go 9P endpoint. This is a general
transport boundary, not an unlink/filename/PC special case. Queue semantics,
tag collision, unknown replies, and post-failure recovery pass directed tests;
the generic concurrent-loader test still passes; and two ordinary Go/Wasm
builds reproduce adapter `bba6baaf...`.

Six fresh loader-prototype runs and six fresh actual-adapter runs each complete
the exact 32 MiB work without a stall. Actual-adapter samples span only
1.020x, remain in R094's qualified envelope, and still expose loader maximum
pending 3, proving concurrency was not globally disabled. These elapsed values
qualify the correction only and give the candidate no performance credit.

Freeze R125 from zero with new pages, archives, tools, protocol, and samples.
Use common adapter `bba6baaf...`, exact control `d9f686a9...`, and unchanged
candidate `d017a10f...`; retain the seven-pair/three-repetition R124 decision
rules. A pass advances to the untouched scorecard, a valid protected-row
failure rejects R124, and another invalid run stops for diagnosis. Continue to
exclude all R124 formal and diagnostic timing from the replacement decision.

## D126: reject R124 at the frozen R125 WANIX protected-shell rule

Date: 2026-08-10  
Status: valid gate failure; candidate archived; scorecard not run

The sole from-zero R125 run is valid and complete. All fourteen fresh-browser
legs finish in their frozen alternating order with zero stderr, correct Linux,
Alpine, Python, artifacts, outputs, active-JIT coverage, exact 9P work, browser
identity, affinity, spread, and freshness proofs. The adapter correction is
therefore qualified in the full formal workload and the prior transport
invalidity does not recur.

Candidate `d017a10f...` improves Python `1.079692x
[1.070039,1.111911]`, SHA-256 `1.021724x [1.018486,1.037770]`, and shared 9P
`1.009359x [1.003915,1.016363]`. Shell is `0.996257x
[0.989513,0.998082]`. Its median stays within the separate one-percent floor,
but the upper endpoint establishes a 0.374% regression and fails the frozen
no-established-regression rule.

Honor the prospectively declared decision: reject R124, preserve its exact
artifact and report, retain live control `d9f686a9...`, and do not run the
untouched scorecard. The four-row unweighted geometric-mean point is
`1.026272x`, but no such weighting was frozen and it cannot rescue the result.

Before another candidate is sampled, clarify whether the owner's net-positive
policy means (a) a verified target gain of at least 1% with protected medians
no worse than `0.99x`, where smaller established regressions are reported but
not vetoes, or (b) a product aggregate with workload weights fixed in advance.
Do not answer that policy question by reinterpreting or rerunning R125 after
observing it.

## D127: supersede R125's zero-regression veto and advance exact R124

Date: 2026-08-10  
Status: owner policy clarified; WANIX qualification accepted; scorecard next

Preserve D126 as the faithful verdict of the prospectively frozen R125
analyzer. Do not edit its raw report, replace a sample, or rerun WANIX. The
owner now explicitly resolves the policy question raised by that verdict: a
protected slowdown smaller than 1% is materiality-tolerated even when measured
consistently. It is rejected only when the protected paired median falls below
`0.99x` or confidence evidence establishes a regression larger than 1%.

R125 shell is `0.996257x [0.989513,0.998082]`; its point slowdown is 0.374%,
not a scorecard row, and remains within the material boundary. Python is
`1.079692x`, SHA-256 `1.021724x`, and shared 9P `1.009359x`; all transport,
correctness, work, and active-JIT proofs pass. Accept R125 for integration
escalation without calling its original analyzer a pass.

Advance unchanged R124 candidate `d017a10f...` to one untouched authoritative
117-trial legacy/rewrite/copy-v86 scorecard. Promotion does not require one
candidate to close both remaining v86 gaps. Retain it when the report is valid,
13/13 legacy remains intact, v86 parity does not lose a row, at least one of
Boot or Compile improves by at least 1% relative to the accepted scorecard,
and no protected normalized row regresses by more than 1%. R125 already
protects unchanged `/shared/bench.py`. Freeze exact artifacts and the
scorecard adjudicator before the first trial.

## D128: invalidate R126 and repair scorecard input admission

Date: 2026-08-10  
Status: no performance credit; complete from-zero replacement required

R126 is invalid. Although the runner scheduled 117 processes, exact candidate
selection through `SCORECARD_V2_REWRITE_WASM` made every RV64 result
diagnostic-only, and a missing matched x86 kernel made every v86 worker exit
before a result. The report has zero eligible trials and zero v86 results.
Ignore all displayed medians and `goalMet`; preserve report `ac096fec...`.

Do not weaken worker eligibility or treat the override as production. Apply
the already frozen candidate source as the ordinary live build and require
exact `d017a10f...`, eliminating the override altogether. Restore exact R087
matched kernel `8854efec...`, not the unrelated `507a759c...` v86 image. Add
top-level existence checks for every kernel/BIOS file the workers require so a
future omission stops before sampling.

Freeze a new complete 117-trial replacement. It may reuse artifacts and rules,
but no R126 result or elapsed value. This is a prospective harness/input
correction after a run with no eligible performance population, not a retry of
an unfavorable measurement.

## D129: invalidate externally terminated R127 and detach its replacement

Date: 2026-08-10  
Status: no performance credit; complete from-zero replacement required

R127 passed the repaired input preflight and began the frozen matrix, but the
interactive execution session carrying its parent was externally aborted
during an agent continuation handoff. The parent and active worker disappeared
without a report or benchmark error while Python was in progress. No formal
JSON exists and the formal output directory was never created.

Treat every launched R127 process as ineligible and do not invoke the
adjudicator. This is infrastructure invalidation, not a candidate result.
Freeze R128 with the same exact candidate, tools, inputs, 117-process cadence,
and R126 adjudicator. Launch only the parent detached from the interactive
tool session, capture raw stdout/stderr and its OS PID, and rerun from zero.
Detachment changes process ownership only; it must not change affinity,
workloads, timeout, scoring, repeats, or the promotion rule.

## D130: pause R128 at owner request without a performance verdict

Date: 2026-08-10  
Status: paused; no active scorecard; no performance credit

R128 passed its frozen manifest, selftests, and copy/v86 generated-execution
preflight. Its detached matrix completed ALU, Mixed, and Boot orchestration and
had launched the first legacy Python leg when the owner requested a pause.

Terminate only R128's isolated process group and preserve its raw logs. No
formal report or runner exit record exists, so do not adjudicate, infer partial
medians, or reuse a process. Exact candidate `d017a10f...` remains applied in
the uncommitted worktree for the experiment, but it is not scorecard-promoted
or committed. Official parity remains 13/13 legacy and 11/13 copy/v86.

## D131: conclude the exercise without declaring parity

Date: 2026-08-10  
Status: terminal owner decision; objective unachieved; work preserved

The owner has chosen to conclude the JIT rewrite performance exercise. Do not
restart R128, schedule another scorecard, continue optimization, or reinterpret
invalid/partial evidence. No benchmark process remains.

Record the outcome faithfully: the clean-room rewrite and correctness matrix
are substantial deliverables; R087 remains valid at 13/13 legacy and 11/13
copy/v86; R124/R125 establish promising focused and WANIX gains for exact
candidate `d017a10f...`; but that candidate has no valid complete modern
three-way report. The requested copy/v86 parity goal is therefore not achieved
or proven.

Leave exact candidate source/release applied in the uncommitted worktree and
preserve exact control source/artifacts. This is preservation, not promotion.
Do not commit, merge, publish, restore, or delete either state without a new
explicit owner instruction. The terminal audit is
`docs/jit-rewrite/FINAL_EXERCISE_REPORT.md`.
