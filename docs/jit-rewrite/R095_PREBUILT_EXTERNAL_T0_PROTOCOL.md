# R095 Prebuilt Orchestrating External Tier-0 Protocol

Date: 2026-08-09  
Status: rejected at the frozen same-main causal gate; exact candidate evidence
archived and all product/runtime integration removed. Earlier pre-execution
amendments removed the unsupported hard main-CODE cap and corrected a
construction boundary that mixed unequal lazy state.

## Pre-measurement amendment: do not use code size as a rejection proxy

The original draft rejected main-Wasm CODE growth above 6,144 bytes. User
review challenged the evidentiary basis before any R095 performance timing.
The challenge is correct. The number was a conservative implementation budget,
not a measured performance boundary:

- R078 compared artifacts differing by roughly 57 KiB plus hot scheduler
  branches and linked machinery; it did not isolate bytes as the cause;
- R082's 9,204-byte bridge had neutral point estimates and failed only a noisy
  confidence floor, so it does not establish that 9,204 bytes is harmful; and
- R083 was 22,957 bytes smaller and substantially slower, directly showing
  that main-Wasm size is not a throughput proxy.

R095 will record and attribute main and auxiliary section sizes, but no result
will be rejected solely because of a byte count. Structural review still
forbids a linked emitter/runtime compiler and duplicated main-runtime
orchestration. The existing cold-construction, same-artifact causal, native,
browser, WANIX, and full-scorecard gates measure the actual effects that the
cap attempted to proxy. This amendment precedes all R095 timing and therefore
does not respond to candidate performance data.

## Gate-A construction boundary correction before execution timing

The first construction harness incorrectly stopped its exact-R085 leg before
`SYS_JIT` was lazily created, while `prepareExternalT0()` necessarily created
that state on the R095 leg. It reported 34.603 ms versus 41.161 ms, but 6.249
ms of the candidate median sat in layout preparation; actual auxiliary
instantiate and initialize medians were only 0.031 and 0.026 ms. R085 pays the
deferred state creation on its first execution. The endpoints were therefore
not equivalent and using their ratio as the module-construction gate would
reward or punish moving the same work across the timer boundary.

Preserve that report as an invalid readiness diagnostic. Correct Gate A to the
established R093 construction boundary: equivalent main compile/instantiate,
plus auxiliary compile, allocation, instantiate, initialize, and table
publication on the candidate. Seven alternating fresh-process pairs measure
19.208 ms versus 19.866 ms: candidate/control 1.034x by side medians and
1.0499x by paired median, within the frozen 1.05 limit. The auxiliary compile
overlaps main compilation; its allocation/instantiate/init/publication adds a
0.552 ms median.

This correction must not hide public cold latency. The exact-R085/product
five-pair gate below additionally records time from immediately before
`RV64Debug.create` through the Linux-ready marker, including main and auxiliary
compile/instantiate, machine assembly, layout preparation, and boot. Its
candidate/control elapsed median must be at most 1.03x. This amendment follows
only construction attribution; no R095 guest-execution timing existed when it
was frozen, and the candidate implementation is unchanged.

## Question and distinct mechanism

Can one guest-independent scalar Tier-0 executor remove enough of exact R085's
interpreter-dominated Boot cost when both code generation and the execution
orchestration loop are absent from the main emulator Wasm?

This is not a retry of R070--R077 or R082. R082 kept its emitter in a separate
compiler Wasm but added 9,204 bytes of main-runtime bridge and duplicated the
fast/slow/page-policy execution loop in the main artifact. Its frozen dormant
artifact confidence gate failed before active performance was measured. R095
materially changes both boundaries:

- the executor is generated deterministically at build time and shipped as one
  immutable position-independent Wasm module; no browser compiler module,
  runtime `wasm-encoder`, or runtime module emission exists;
- the auxiliary executor owns the scalar fast/slow orchestration and sampled
  policy chunk loop, loading one runtime layout record once per invocation;
- the main runtime exposes only the minimum typed architectural callbacks and
  one scheduler handoff; it does not duplicate the external loop; and
- the product always prepares and enables the tier. Dormant performance is not
  used to hide or excuse enabled product cost.

R076's independent Chrome execution boundary measured the related static core
at 1.175x, while R082's correctness boots proved 54.27M/61.63M externally
retired instructions with zero errors. R088 still attributes 51.972% of exact
R085 Boot to interpreter decode/execute and another 8.837% to scalar memory.
Those are admission evidence only, not R095 performance credit.

## Frozen architecture

1. The auxiliary module contains no guest PC, guest bytes, symbol, workload,
   phase, browser, or compiler-output selector. It dynamically fetches and
   executes the complete ordinary RV64I/M integer, control, scalar-memory, and
   integer-RVC families everywhere.
2. Unsupported, privileged, F/D, A, FENCE, faulting, cross-page, and MMIO
   instructions use the unchanged authoritative interpreter. Two consecutive
   zero-fast-progress exits may grant the existing fixed 64-instruction slow
   batch; the rule observes only progress and architecture support.
3. One immutable auxiliary module imports the emulator memory and typed
   callbacks. It receives a pointer to a validated layout record, loads the
   addresses/capabilities once per invocation, and never embeds per-instance
   addresses. Its bytes must be identical across instances and runs.
4. Fetch/load/store rows retain their complete mapping, permission, context,
   width, page-crossing, RAM-range, and dirty-code contracts. A miss refills or
   returns to the authoritative path; it never becomes a guest exception by a
   host Wasm trap.
5. Interrupt countdown, WFI host yield, device synchronization, direct SBI,
   precise generated-entry handoff, first control-target observation, page
   policy accounting, retirement, reset, and invalidation remain exact.
6. The ordinary generated page/region JIT is unchanged and remains enabled.
   The external tier handles residual execution; it does not change heat,
   threshold, region geometry, queueing, or publication policy.

Record and attribute the R095-capable main runtime's executable CODE-section
growth over exact R085. It may not depend on `wasm-encoder` or the external
emitter crate, contain a duplicate copy of the auxiliary orchestration loop, or
ship dormant experimental branches. The distribution must contain one
auxiliary executor and no runtime compiler artifact. Failure of those semantic
structural conditions stops R095 before performance timing; byte count alone
does not.

## Gate A: deterministic shape, construction, and correctness

Before performance measurement:

- regenerate the auxiliary module twice and require byte identity, validation,
  fixed imports/exports, no guest bytes, and no defined/imported memory other
  than the exact shared memory import;
- prove the main Wasm public ABI remains compatible, report and attribute its
  section deltas, prove no compiler/emitter links into it, and show release construction including
  auxiliary compile/instantiate is no more than 5% slower in seven alternating
  fresh-process pairs;
- exhaust all legal/reserved RV64C halfwords and directed/random RV64IM state at
  budgets 1, 32, 1,024, and 4,096;
- exercise full-system fetch/load/store hit/refill, Sv39/MPRV, permissions,
  unaligned/page-crossing/MMIO/fault paths, A/LR-SC, FP fallback, CSR/system
  fallback, dirty code, exact generated handoff, control observation,
  interrupts, WFI, reset, and lifecycle;
- boot modern Linux 6.12.7 through direct SBI and OpenSBI; and
- pass the complete strict repository matrix with zero external-tier errors and
  exact retirement accounting.

## Gate B: causal and native product timing

First run three alternating same-main/same-auxiliary pairs, varying only the
enable cell after identical preparation. Require Boot control/candidate paired
median at least 1.03x, exact lower bound at least 1.00, and Compile/Python no
more than 3% slower by paired median. This proves the external execution itself
rather than artifact layout.

Then run five alternating fresh exact-R085/product pairs on CPUs 8--15 with the
modern Linux 6.12.7 / Alpine 3.24.1 artifacts and public one-slice cadence.
Advance only when:

- Boot R085/product paired median is at least 1.05x and its exact paired-
  bootstrap 95% lower bound is at least 1.00;
- cold create-through-Linux-ready product/R085 elapsed median is at most 1.03x;
- Compile and Python product/R085 elapsed medians are each at most 1.03x and
  neither confidence lower bound shows a material regression;
- host spread is at most 1.10x; and
- exact inputs, outputs, guest instruction counts, ordinary generated
  execution, external retirement/accounting, module identity, and lifecycle
  proofs all pass.

No failed leg is replaced and no threshold, slow-batch, instruction-family,
layout, or callback variant follows a failed frozen gate.

## Gate C: browser integration and promotion

Run seven fresh Chrome Boot pairs first, requiring at least 1.03x paired median
and a lower bound of 1.00. Then run the exact qualified R094 32 MiB shared-9P
guard plus unchanged Python and SHA-256 phases on immutable control/product
pages. Python and SHA-256 product/R085 elapsed medians must be at most 1.03x
with upper bounds at most 1.10. Shared 9P must use the R094 fixed bytes,
duration, transfer, accounting, within-browser-spread, and confidence rules,
with the candidate non-regression direction substituted for the null bounds.

Only after every earlier gate passes, run the untouched 117-trial
legacy/rewrite/copy-v86 scorecard once. Promote only if it is authoritative,
keeps 13/13 against legacy, improves the parity count or materially reduces an
open-row gap without any row more than 5% slower than R085, and preserves the
browser guards. The full thread goal remains Boot and Compile parity with
copy/v86; a partial R095 gain does not redefine completion.

On any failure, archive exact source, main/auxiliary artifacts, reports, and
reason; remove the candidate completely and restore exact R085 main Wasm
`efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`,
loader `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`,
and production archive
`0b953be67610e130f79a852f86542c8400ad3a235001ec450fbdffc29ed3a61a`.

## Removal and restoration audit

The rejected auxiliary is absent from Cargo, the loader API, release and Pages
workflows, demo assets, WANIX archives/adapters, public/Worker tests, ordinary
system differentials, and the scorecard worker. A repository-wide source audit
finds no active `externalT0`, `rv64_t0`, or R095 product reference outside this
historical record and the archived evidence.

The restored source build is 4,279,380 bytes at `d9f686a9...`. Its complete
Wasm type/import/function/table/memory/global/export/element and executable
CODE sections match frozen R085; all 3,702 defined function bodies are
byte-identical. Its only binary differences are one source-location byte in a
passive data record and two LLVM-generated diagnostic-name suffixes. The
packaged runtime therefore installs the immutable tested 4,279,378-byte R085
artifact `efd7830307...`, while retaining the source-built artifact as evidence
that executable code was restored rather than silently accepting a new
implementation.

Release workspace tests, focused public/Worker and 23 Wasm/JIT differential
gates, and fresh direct/OpenSBI Linux 6.12.7 boots pass on the restored source
build. Public/Worker and both modern boot paths also pass after installing the
exact frozen artifact. The rebuilt WANIX archive contains only
`rv64-jit-vm.wasm`, `rv64_wasm.wasm`, and `rv64.js`; its current R094-qualified
adapter archive is `6d28e87a...`, with exact R085 Wasm `efd7830307...` and
loader `2cbb264f4...` inside. It is intentionally not claimed to equal the old
pre-R094 archive byte-for-byte.
