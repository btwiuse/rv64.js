# R083 full-system interpreter specialization protocol

Date: 2026-08-09  
Status: rejected at native performance gate; cleanly removed

## Question

Can the already machine-wide distinction between user-only and full-system
execution be resolved once at the outer interpreter entry, allowing the hot
full-system fetch/load/store path to avoid repeatedly inspecting
`Option<SysCsrs>` without changing the decoded ISA, JIT policy, or guest?

R081 supplies the prospective opportunity bound. On exact R080, `Cpu::step`
owns 50.28% of complete Boot CPU time and the phase is 93.41% runtime Wasm.
A local 6% reduction in that function has an Amdahl projection just above the
standing 3% whole-Boot cumulative floor. V8 already gives the function a
TurboFan body, so this tests native work removed from optimized code rather
than engine warmup or tier forcing.

## Immutable control

- main Wasm: 4,272,559 bytes,
  `e5415db83b27b32a1f525af2aa19e93539332a274068e389a1e28ebba41d8095`;
- loader: `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- production WANIX archive: `414a174542161f9d52d6814d1deaf9fbdd56e4fa152d11fa80d7167e76a45aa5`;
- modern Linux 6.12.7 / Alpine 3.24.1 scorecard inputs and CPUs 8-15;
- ordinary page policy, thresholds, async registration, generated modules,
  browser settings, and scorecard boundaries unchanged.

## Frozen implementation

1. Introduce one private const specialization for execution mode. `SYSTEM=true`
   is valid only while `Cpu::sys` is present; `SYSTEM=false` is valid only
   while it is absent. Safe public entry points prove that invariant once
   before entering their instruction loop.
2. Specialize the existing instruction step, direct RV64C step, translation
   context/tag, fetch translation, scalar load/store, fused-row publication,
   and FP-state checks. Compile-time false branches return the existing
   user-only behavior; compile-time true branches access the existing system
   state without another `Option` test.
3. Preserve the public `Cpu::step` behavior by selecting the matching private
   specialization. `run`, `run_until`, `run_until_observed`, and `run_traced`
   select once per call and execute the corresponding private loop; the scored
   full-system path must not redispatch on mode per guest instruction.
4. Add no state field, cache, threshold, counter, host import/export, JS
   setting, JIT policy, opcode subset, or guest-visible behavior. Do not alter
   interrupt cadence, trap/retirement/PC rules, WFI yield, memory capability
   validation, page crossing, MMIO, Sv39/MPRV, or dirty-code invalidation.
5. The selector is only the architectural machine construction invariant. No
   PC, symbol, binary, workload, benchmark phase, output, compiler artifact,
   browser, or observed frequency may influence it.

This is not a decoder-inlining, fetch-cache, callback-monomorphization, or
static Tier-0 retry. R023/R055/R056/R082 remain closed. The expected artifact
cost is a second mode-specialized interpreter body; code size and cold compile
time are measured costs, not reasons to waive a gate.

## Correctness and shape gate

Before performance timing, require:

- formatting and workspace build success;
- complete rv64-core, rv64-system, and rv64-dbt unit suites;
- direct public-step differentials in both user-only and full-system state;
- exhaustive/direct RV64C, integer/M/A/FP, scalar-memory, Sv39/MPRV,
  cross-page/MMIO, exception/trap, WFI, generated re-entry, and dirty-code
  tests already in the strict matrix;
- Wasm smoke, public API, Worker API, direct Linux, and OpenSBI Linux;
- candidate import/export/state-layout identity and exact scorecard inputs;
- static confirmation that the full-system specialization exists as a
  distinct Wasm function and no runtime configuration selects it.

Any failure stops the experiment. Correctness legs are not replaced or
weakened to obtain timing.

## Frozen native performance gate

Run five alternating fresh-process exact-R080/candidate pairs. Each pair runs
Matched Boot, Compile, and Python under the existing scorecard-v2 config-A/B
harness on CPUs 8-15. Retain all 30 legs in issued order. Require exact input,
output, guest-work, JIT-policy, generated-execution, affinity, and host-probe
proof with aggregate host spread no greater than 1.10x.

Advance only if all conditions hold:

- paired Boot control/candidate median is at least 1.03x and its deterministic
  paired-bootstrap 95% lower bound is at least 1.00;
- Compile and Python control/candidate medians are each at least 0.97x and
  their paired lower bounds are each at least 0.95;
- candidate Wasm grows by no more than 64 KiB, main-module cold construction
  regresses by no more than 5%, and no correctness or measurement problem is
  present.

Cold construction is the median `WebAssembly.compile` duration from seven
alternating fresh Node processes per artifact using
`tests/vs-v86/fresh-wasm-compile.mjs`; file I/O and process startup are outside
the interval. This sample plan is fixed before its first observation.

There is no threshold, mode, helper-boundary, inlining, or sample-count sweep
after observation. A narrow miss is retained as a miss.

## Browser, v86, and promotion gates

If the native gate passes, run five alternating fresh Chrome candidate/control
pairs. Python, SHA-256, and shared 9P may not regress more than 3% by paired
median or more than 10% at the confidence upper bound. Then run the frozen
candidate/v86 guard; the three existing RV64/v86 browser medians and exact
generated-code proofs must remain valid.

Only then run the untouched 117-trial three-way scorecard once. Promote only
when it is authoritative with no problem, Boot improves at least 3% versus
R080, no row regresses more than 5%, all 13 rows still beat legacy, browser
guards pass, and the production artifacts/defaults are explicitly recorded.
The fixed objective remains 13/13 versus copy/v86; a retained cumulative gain
may advance that objective without completing it in one experiment.

## Preregistered harness-path correction

The first native command issued all 30 planned worker processes with
`ARTIFACTS=target/bench`, but that directory does not contain
`scorecard-v2-modern-riscv64.cpio`. Every worker exited on the same `ENOENT`
before VM construction and before any guest or phase timer; there are zero
successful trials and zero timing pairs. The complete invalid report is
`target/bench/r083-full-system-step/native-ab/config-ab-2026-08-09T16-11-04-000Z.json`
(`25ed16e39999...`). It remains immutable and is not performance evidence.

Before any observation of candidate performance, correct only `ARTIFACTS` to
the already prepared directory
`/home/darren/src/arm64.js/target/bench`. Its modern riscv64 initramfs is
`cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808`,
the exact input hash recorded by R080/R082. Keep both Wasm paths, loader,
rows, five-pair alternating order, CPUs, thresholds, and all gates unchanged,
and issue the one valid run. This does not replace a failed or unfavorable
sample because the invalid setup produced no sample at all.

## Result

The implementation completed exactly the frozen scope. The candidate is
4,249,602 bytes,
`ac16c4ffb885ef441b95bc20b817c17c9eb74cbc1af988fb166b3eff43c61684`,
22,957 bytes smaller than R080. Imports, defined functions, exports, element
segments, and their counts are unchanged. Static Wasm inspection shows
distinct const-specialized true/false bodies; the Virt full-system step is
10,359 bytes and the user-only body 9,315 bytes. There is no runtime selector
or new state/configuration field.

All strict correctness stages pass before timing: 32 core, 53 DBT, 76 system,
QEMU differentials, 134 ISA tests, 109 Spike lockstep cases, 193 architecture
signatures, the complete Wasm/JIT/system/Sv39/A/FP/WFI/T2 matrix, standalone
Wasmtime, direct and OpenSBI Linux, stable page policy, FP context switching,
AMO differential, and modern virt smoke. Public and Worker API lifecycle tests
also pass. Seven alternating fresh compilation pairs pass the cold gate at
5.737/5.994 ms control/candidate medians, a 4.49% candidate regression under
the frozen 5% cap. Raw evidence is
`target/bench/r083-full-system-step/cold-compile.json` (`dbbe23a635e2...`).

The corrected native A/B is measurement-valid with no problems and host-probe
spread 1.021x. All 30 legs completed once. The immutable report is
`target/bench/r083-full-system-step/native-ab-valid/config-ab-2026-08-09T16-17-26-914Z.json`
(`e63699c3a1c2...`). Exact R080 control versus R083 candidate results are:

- Boot: 2,316.232/2,609.354 ms, side-median speedup 0.888x; paired median
  0.888x with interval `[0.869,0.898]`;
- Compile: 1,043.095/1,096.812 ms, side-median speedup 0.951x; paired median
  0.963x with interval `[0.905,1.074]`;
- Python: 3,073.553/3,120.635 ms, side-median speedup 0.985x; paired median
  0.985x with interval `[0.952,1.031]`.

Boot fails the required 1.03x target and 1.00 lower bound decisively. Compile's
side median also falls below its 0.97 guard. R083 therefore stops without a
browser run, v86 guard, scorecard, helper-boundary variant, or retry. Candidate
source before removal is preserved at
`target/bench/r083-full-system-step/source-before-removal.tgz`
(`5cb398084de6...`), and the candidate Wasm remains under
`target/bench/wasm-candidates/`.

Removal is byte-exact. The rebuilt release is R080's 4,272,559-byte
`e5415db83b27...` Wasm, the loader remains `2cbb264f4dac...`, and the production
archive remains `414a17454216...`. The restored build passes all 32 core units,
Wasm smoke, and public/Worker API checks. The accepted result stays R080 at
11/13 versus copy/v86.
