# R067 Post-R066 Residual Audit

## Purpose

R066 improved Boot by a reproducible paired 1.074x but failed its frozen 1.10x
promotion gate. Before admitting another architecture, establish whether its
intended integrated shape was actually emitted and identify the residual cost
after that shape executes. This is an attribution audit, not a product
candidate and not a scored comparison.

## Frozen artifacts

- Accepted production R054:
  `target/wasm32-unknown-unknown/release/rv64_wasm.wasm`, expected SHA-256
  `4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
- Rejected default-off R066 replay artifact:
  `target/bench/r066-integrated-scalar-t0/rv64-integrated-scalar-t0.wasm`,
  expected SHA-256
  `d2ac8852eaf1950c55042cd8109a6e2f43aae2154a019ea6ee8c637aa628fdce`.
- Modern Linux 6.12.7 / Alpine 3.24.1 Boot input and production JIT policy are
  unchanged from the valid R066 screen.

## Static realization check

Use the candidate symbol table and disassembly to record, for the VirtBus
`Cpu::run` and `Cpu::run_until` monomorphizations:

1. function byte size and local count relative to R054;
2. whether scalar decode/execute is present directly in the driver;
3. all remaining direct calls to the complete `Cpu::step` and their placement;
4. branch-table count and absence/presence of a separately named scalar-T0
   function.

If scalar execution remains behind a per-instruction helper, R066 does not
bound integrated execution and its source mechanism must be reconsidered. If
it is directly present and `Cpu::step` is reached only from slow-family exits,
the product result is a valid bound on that design.

## Engine residual profile

Replay the same candidate bytes with `jit_set_integrated_scalar_t0(0)` and
`jit_set_integrated_scalar_t0(1)` in separate fresh, CPU-pinned Boot guests.
Capture phase-isolated V8 CPU profiles with the existing scorecard profiler.
The temporary replay selector must:

- apply only to the archived R066 artifact;
- make every result measurement-ineligible;
- change no guest, scheduler, JIT policy, workload, or generated module;
- be removed after the profiles are captured.

Profiles are attribution evidence only. Report total runtime/generated-Wasm
categories, the top runtime self frames, and the complete `run_system_jit`
subtree. Do not compare profiled wall times as scorecard results.

## Admission decision

Compare the realized R066 residual with accepted R054 and the pinned copy/v86
interpreter structure. Admit a next implementation only if one coherent,
architecture-general mechanism owns enough measured residual to project at
least 10% whole-Boot improvement and is not an alias of a closed experiment.
Candidate scopes must explain how they differ from decoder inlining/local PC
state (R023/R066), decoded handlers (R025-R027/R045), early page modules
(R046), fetch capabilities (R055/R062), and compact step returns (R058).

If no such mechanism is supported, record the plateau explicitly. Do not
combine unrelated optimistic percentages or tune scorecard-specific PCs,
opcodes, privilege modes, thresholds, cache sizes, or binaries.

## Result: the integrated shape was realized

The archived R066 artifact passed the static realization check. In accepted
R054, the VirtBus monomorphizations of `Cpu::run` and `Cpu::run_until` are 272
and 381 code bytes. In R066 they are 6,138 and 6,331 bytes, with approximately
28/30 locals and 22 `br_table` instructions each. There is no separately named
scalar-T0 helper. The scalar decoder and execution bodies are present directly
inside both drivers.

The complete `Cpu::step` remains at three static call sites: the disabled
control path and the two slow-family fallbacks. Ordinary RV64I/M integer,
control, scalar-memory, and integer-RVC execution therefore does not cross that
call boundary. R066 is a valid bound on the proposed integrated-loop shape;
its 1.074x Boot result was not limited by accidentally retaining the old
per-instruction decoder call.

Scalar loads and stores still call the width-specific `Cpu::ld`/`Cpu::st`
helpers. Their combined treatment self time is 194.468 ms, or 8.783% of the
complete sampled profile. Even eliminating that category entirely cannot
close the approximately 34% wall-time reduction still required for raw Boot
parity, and using it to tune R066 after the frozen result would violate the
protocol.

## Result: residual engine profiles

The temporary replay selector was confined to the archived R066 bytes and has
been removed. The saved evidence is:

- control: `target/bench/r067-post-r066-residual/control/rewrite-boot-first.cpuprofile`;
- integrated treatment: `target/bench/r067-post-r066-residual/treatment/rewrite-boot-first.cpuprofile`;
- pinned copy/v86: `target/bench/r067-post-r066-residual/v86/v86-boot-first.cpuprofile`;
- common analyses: `target/bench/r067-post-r066-residual/{analysis,combined-analysis}.json`.

These are phase-isolated attribution profiles, not scorecard timings. The
control sampled 2,390.689 ms: runtime Wasm owned 2,249.455 ms (94.09%) and
generated Wasm 99.394 ms (4.16%). `Cpu::step` alone owned 1,222.637 ms
(51.14%). The integrated treatment sampled 2,214.060 ms: runtime Wasm owned
2,065.771 ms (93.30%) and generated Wasm 106.682 ms (4.82%). Its inlined
`Cpu::run_until` plus `Cpu::run` self time was 1,376.808 ms (62.19%), while
the slow-family `Cpu::step` fell to 33.476 ms (1.51%). The policy-interpreter
subtrees still owned 1,794.422 ms (81.05%). This confirms that R066 moved cost
inside a larger driver but did not change the dominant interpreted execution
representation.

The exact pinned v86 profile sampled 1,698.277 ms. Runtime Wasm owned
1,187.300 ms (69.91%) and generated Wasm 366.702 ms (21.59%). Thus v86 spends
roughly 4.5 times the fraction of sampled Boot time in generated Wasm. Exact
rewrite counters for the R066 treatment retire 69.419M generated and 111.363M
interpreted instructions. At the previously measured approximate rates of
661 MIPS generated and 62 MIPS interpreted, moving 40M remaining interpreted
instructions to generated execution has an optimistic gross value near 585
ms. That is coherent whole-Boot leverage above the 10% admission floor.

## Decision

R067 closes further scalar-driver rearrangement. Decoder layout, PC/minstret
localization, fetch caching, `Cpu::step` call shape, and load/store-only work do
not provide a credible route to raw parity from the measured residual.

One architecture-general mechanism remains admitted for an opportunity test:
a cold privileged generated tier that batches multiple unrelated hot code
pages into a small number of modules. This differs from R046: R046 created
51-52 independent one-page modules, accumulated 6.77-6.94 seconds of async
compile latency, and displaced only 13-14M instructions by readiness. The new
gate must demonstrate comparable early coverage while amortizing those pages
into substantially fewer WebAssembly compilations. It is not permission to
retune the privileged threshold, page cap, leader cap, or concurrency.

Before any runtime candidate is admitted, an offline JIT-disabled Boot trace
must establish all of the following with frozen, architecture-defined
selection rules:

1. at least 10% projected whole-Boot leverage after threshold and entry
   coverage are accounted for;
2. materially fewer host `WebAssembly.compile` jobs than R046's 51-52;
3. valid emitted modules with bounded total bytes and no single giant function
   resembling the R048 late-tiering failure;
4. no PC, symbol, binary hash, workload name, or scorecard-row selector.

If the batch geometry fails any gate, record the structural plateau and do not
implement the runtime tier.
