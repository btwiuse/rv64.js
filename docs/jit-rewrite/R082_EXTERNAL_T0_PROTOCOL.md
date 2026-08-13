# R082 External Tier-0 Protocol

Date: 2026-08-09  
Status: protocol frozen before implementation

## Question and causal basis

Can the already proven guest-independent scalar execution core improve cold
and residual RV64 execution when its emitter/compiler is physically absent
from the main emulator Wasm?

This does not repeat R070-R077 under a new threshold. R076 measured the active
mechanism at 1.175x in Chrome's scorecard-equivalent execution boundary, while
R078 later proved the default-off artifact containing its linked emitter and
runtime was itself 1.178x slower than exact R054. R080 removed that residue and
restored a trustworthy baseline. R081 now measures `Cpu::step` at 50.28% of
R080 Boot and runtime Wasm at 93.41%, providing sufficient whole-row leverage
for one new representation under the standing 5% substantial-tier rule.

## Frozen baseline and non-goals

The control is exact R080 main Wasm
`e5415db83b27b32a1f525af2aa19e93539332a274068e389a1e28ebba41d8095`
and loader
`2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`,
with the Linux 6.12.7 / Alpine 3.24.1 modern scorecard artifacts and CPUs 8-15.
R080's untouched authoritative medians are Boot 2,338.368 ms, Compile
1,058.842 ms, and Python 3,085.7 ms.

R082 may not select behavior from a guest PC, symbol, virtual/physical page,
binary identity, workload, benchmark phase, checksum, browser, compiler
output, instruction-frequency table, or firmware/kernel identity. It may not
change the accepted page heat/region policy, tune a threshold, revive the
sampled-entry short/backoff cache, or hide any compiler/module work that occurs
inside the frozen guest-execution boundary.

## Representation and ABI

1. The decoder emitter is a separate `rv64-t0-compiler.wasm` artifact. The
   `rv64-wasm` crate does not depend on it, and no emitter, `wasm-encoder`
   path, or decoder-generation body may link into `rv64_wasm.wasm` through
   R082.
2. The loader compiles the compiler artifact before instantiating or starting
   the guest. After machine construction, both timing arms ask it to emit the
   same guest-independent scalar module from runtime capability addresses,
   instantiate that module against the emulator's memory and typed helpers,
   and place its `(i32 state) -> ()` entry in the existing function table.
3. Both timing arms therefore compile, instantiate, and publish byte-identical
   auxiliary modules at the same lifecycle point. The sole active A/B
   difference is one runtime enable cell set after successful registration.
4. The main runtime exposes only stable capability addresses, installation,
   enable, exact slow-path helpers, reset, and counters. It does not construct
   module bytes. The auxiliary module owns scalar fetch/decode/execute,
   permission-tagged fetch/load/store probes, direct RV64C expansion-table
   lookup, the existing interrupt countdown, and exact generated-entry
   handoff.
5. When enabled, the auxiliary core is the general interpreter residual for
   both ordinary and page-policy-sampled execution. It is offered every
   architecture-neutral residual budget; it does not classify entries or
   choose PCs. Unsupported, privileged, FP, atomic, faulting, cross-page,
   and MMIO instructions execute in the unchanged interpreter. Two adjacent
   zero-progress slow exits retain the fixed 64-instruction interpreter batch
   used to avoid pathological Wasm-to-Wasm ping-pong; this rule observes only
   progress and opcode support, never identity or workload.
6. Policy observation, precise generated re-entry, device synchronization,
   interrupt delivery, WFI yield, retirement accounting, dirty-page
   invalidation, reset, and direct/OpenSBI behavior remain architecturally
   identical. The first mapped non-sequential target is reported under the
   same contract as `run_until_observed`.

## Gate A: dormant-artifact non-inferiority

Before active timing, build the candidate-capable main runtime and leave the
new path unprepared and disabled. Run all focused Rust/Wasm/API lifecycle
tests, then five alternating fresh-process pairs against exact R080 on Boot,
Compile, and Python. Record exact source/main-Wasm/import/export/code-section
identities and V8 host spread.

R082 stops and all main-runtime changes are removed if the candidate-capable
disabled artifact is more than 3% slower on any median, if the paired
bootstrap lower bound for R080/candidate is below 0.97, or if correctness,
identity, generated-execution, affinity, or spread proof fails. A smaller main
Wasm, dead flag, or active-path speedup cannot rescue this gate.

## Gate B: semantic and lifecycle proof

On one identical candidate-capable main Wasm, prepare the same auxiliary
module in both arms and vary only enable. Require:

- exhaustive legal/reserved RV64C coverage and scalar RV64IM execution;
- randomized state/memory differentials at budgets 1, 32, 1,024, and 4,096;
- full-system fetch/load/store hit, refill, permission, Sv39/MPRV, cross-page,
  MMIO, exception, CSR, FP, A/LR-SC, and dirty-code differentials;
- exact generated-entry handoff, control-target observation, WFI, interrupt,
  reset/reboot, table/module lifecycle, direct Linux, and OpenSBI Linux gates;
- the complete strict repository matrix before any promotion timing;
- byte-identical auxiliary modules and exactly one registration in each arm;
  enabled retirement must be nonzero and errors must be zero.

## Gate C: focused performance

Run five alternating fresh R080-compatible control/candidate pairs with the
same candidate-capable main Wasm and prepared auxiliary module in both arms.
Boot is the target. Advance only when:

- paired Boot control/candidate median is at least 1.05x and its deterministic
  paired-bootstrap 95% lower bound is at least 1.00;
- Compile and Python candidate/control medians are each at least 0.97x and
  their lower bounds are at least 0.95;
- no output, guest-work, generated-code, module, lifecycle, affinity, host
  spread, or static-core proof fails.

No failed leg is replaced, no order is selected after observation, and no
configuration sweep follows a failure.

## Gate D: browser and copy/v86 guards

Use fresh Chrome processes, fixed CPUs, alternating order, and the existing
phase-synchronized harness. First compare candidate/control with the same
main/compiler/auxiliary artifacts. Python, SHA-256, and shared 9P may not
regress more than 3% by paired median or more than 10% at the confidence upper
bound. Then run the frozen candidate/v86 product guard; all three RV64/v86
elapsed medians must remain below 0.93 with upper bounds below 1.00, exact
outputs, and nonzero ordinary generated execution.

Compiler compilation, module emission, instantiation, and publication get
separate counters and cold timings. Work performed before the scorecard's
guest-execution boundary remains visible in the report and in a first-use
product diagnostic; it is not added to or subtracted from measured guest time.

## Gate E: promotion

Run the untouched 117-trial three-way scorecard exactly once. Promote only if
it is authoritative with no problem, Boot improves at least 5% versus R080,
no row regresses more than 5% versus R080, all 13 rows still beat legacy, the
existing browser guard passes, and the resulting loader/archive defaults are
explicitly documented. Copy/v86 parity need not arrive in one candidate, but
the fixed final objective remains 13/13.

Any failure is recorded with immutable artifacts and removes the candidate
from production. Historical R070-R077 results are not relabeled or pooled with
R082 timing.

## Result

R082 stops at Gate A. The candidate-capable main Wasm is 4,281,763 bytes,
`2cad849c7bebb50bd1ba8e35cbd904dcf3de69a50ad730c1952dd4b8f563656b`;
it is only 9,204 bytes larger than R080. Its physically separate 217,556-byte
compiler is
`83858f082f8f2b15ac96370378595c1e22e22e3491de4babb39ca6e3eef4af7e`.
The compiler emits one valid 11.3-KiB full-system scalar module and provides
all 65,536 RV64C expansion rows without linking either body into the main
runtime.

Focused semantic evidence was clean before timing. All 32 core, 53 DBT, 76
system, one Wasm, and four compiler units pass. Same-main/same-auxiliary
q1/q32/q1024 tests preserve exact PC, retirement, registers, memory, and page
policy and prove generated-entry handoff with zero errors. Modern Linux 6.12.7
direct and OpenSBI boots both reach Alpine and run a shell command; the
external tier retires 54.27M/61.63M instructions while the ordinary page JIT
retires 354.81M/354.30M, with zero external errors. These are correctness and
mechanism proofs, not promotion timings.

The frozen disabled-artifact run completed all 30 legs once. Its immutable
report is
`target/bench/r082-disabled-artifact/config-ab-2026-08-09T15-45-52-437Z.json`
(`35241786cad7...`), measurement-valid with no problems and host-probe spread
1.062x. Exact R080 control versus dormant-capable candidate results are:

- Boot: 2,310.650/2,309.627 ms, median ratio 1.000x; paired median 1.005x
  with interval `[0.967,1.092]`;
- Compile: 1,077.890/1,075.695 ms, median ratio 1.002x; paired median 1.029x
  with interval `[0.988,1.064]`;
- Python: 3,136.670/3,068.436 ms, median ratio 1.022x; paired median 1.014x
  with interval `[0.986,1.036]`.

All point estimates satisfy the 3% dormant limit, but Boot's frozen paired
lower bound is 0.967, below the prospectively required 0.970. R082 therefore
fails Gate A by 0.003 and stops without active performance pairs, browser/v86
guards, scorecard escalation, retry, or a relaxed interval. Candidate source
before removal is preserved at
`target/jit-policy-traces/r082-pre-removal-source/external-t0-source.tgz`
(`80d9aec0d505...`); the exact main and compiler artifacts remain in
`target/bench/wasm-candidates/`.

Cleanup is complete and independently checkable. Rebuilding the main runtime
after removing the workspace crate, JS loader API, CPU/system hooks, Wasm
exports, scheduler branch, state cells, and tests produces the exact promoted
R080 artifact: 4,272,559 bytes,
`e5415db83b27b32a1f525af2aa19e93539332a274068e389a1e28ebba41d8095`.
The loader is likewise byte-exact R080,
`2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`,
and the untouched production archive remains `414a17454216...`. Focused Wasm
smoke, public API, Worker API, full-system memory differential, and cold/warm
WFI-yield checks pass on the restored build. No R082 runtime identifier
remains in source, workspace manifests, lockfile, or active tests.
