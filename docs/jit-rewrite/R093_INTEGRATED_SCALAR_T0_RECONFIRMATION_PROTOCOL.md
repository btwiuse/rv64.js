# R093 Integrated Scalar T0 Reconfirmation Protocol

Date: 2026-08-09  
Status: rejected at frozen WANIX guard; exact R085 restored

## Why this mechanism is eligible for one reconfirmation

R066 integrated the complete ordinary RV64I/M integer, control, scalar-memory,
and integer-RVC families into the interpreter driver. It carried PC and
retirement in Wasm locals and used the existing complete `Cpu::step` only for
F/D, A, FENCE, SYSTEM/CSR, compressed floating-point, and other uncommon
families. The selector was the ISA family itself; it contained no guest PC,
symbol, binary, compiler, workload, browser, or result-specific rule.

The valid frozen three-pair R066 report is
`target/bench/r066-integrated-scalar-t0-screen/config-ab-2026-08-09T02-43-43-436Z.json`.
Against exact R054 it measured:

- Boot paired speedup 1.074x, 95% interval `[1.060,1.129]`;
- Compile paired speedup 0.974x, interval `[0.959,0.996]`; and
- Python paired speedup 0.987x, interval `[0.947,0.999]`.

R066 was removed because its frozen promotion rule required Boot at least
1.10x. Its 2.6% Compile and 1.3% Python median costs were inside the later 3%
cumulative guards. R080 prospectively replaced the former 10% single-change
rule with a 3--5% cumulative policy, and the project owner has now explicitly
confirmed that a reproducible general 5% gain should be retained. R084 permits
one independent confirmation when a general mechanism gained at least 3% and
was rejected by the superseded 10% rule. R066 satisfies that exception.

This is not permission for a scalar-family, privilege-mode, decoder-layout,
or threshold sweep. R067 still closes variants of the mechanism. R093 tests
the exact complete R066 family once against the current accepted baseline.

## Frozen control and reconstruction

The immutable control is promoted R085:

- runtime Wasm
  `efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`;
- loader
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- `crates/rv64-core/src/cpu.rs`
  `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`;
- `crates/rv64-wasm/src/lib.rs`
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
  and
- corrected authoritative report
  `1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7`.

Reconstruct the R066 implementation from its original recorded patch sequence
in the project session log and compare the resulting shape with archived R066
Wasm `d2ac8852eaf1950c55042cd8109a6e2f43aae2154a019ea6ee8c637aa628fdce`.
Adapt only source context required by intervening accepted correctness/runtime
changes. Preserve the exact admitted families, state-commit points, interrupt
polling, exception behavior, stop predicate, and slow fallback. Do not add a
new opcode, mode, PC, page, workload, or frequency selector.

The first candidate remains default-off behind one same-Wasm diagnostic switch
so off/on legs prove causality with identical module bytes. If and only if that
screen passes, create one separately hashed default-on product candidate. No
implementation variant is allowed after any product timing.

## Correctness and shape gates

Before timing:

1. Re-run exhaustive actual compressed-prefix coverage and the broad directed
   and randomized 32-bit scalar corpus against `Cpu::step`, comparing PC,
   registers, memory, exceptions, stop reason, and retirement.
2. Exercise x0 writes, M-extension edge cases, fetch and data faults,
   page-straddling instructions, unaligned/cross-page scalar memory, control
   links, interrupt delivery, WFI, and slow-family fallback.
3. Pass core, system-memory, Sv39/MPRV, atomic, FP/FS, WFI, T2, Wasm/API/Worker,
   direct Linux, and OpenSBI Linux gates with the candidate enabled.
4. Prove the emitted Virt `Cpu::run` and `Cpu::run_until` bodies contain the
   integrated scalar decoder and retain complete-step calls only on slow
   paths. Imports/exports other than the temporary diagnostic setter must be
   unchanged.
5. Seven alternating fresh-process main-Wasm compile/instantiate pairs may
   regress by no more than 5%.

Any semantic mismatch, missing intended shape, or cold-construction failure
stops the experiment and restores exact R085.

## Frozen performance path

Run five alternating fresh-process same-Wasm pairs for Boot, Compile, and
Python on CPUs 8--15 with exact Linux 6.12.7 / Alpine 3.24.1 artifacts,
production page policy, and R087's public one-slice-per-turn cadence. All 30
legs must preserve exact identity, input, output, JIT, and host-spread proof.

Advance when:

- Boot paired median speedup is at least 1.03x and its paired 95% lower bound
  is at least 0.98x;
- Compile and Python candidate/control elapsed medians are each at most 1.03x;
  and
- no correctness, sample-spread, host-spread, or policy guard fails.

If the causal screen passes, archive source and Wasm once, make the exact
mechanism the default in one product candidate, remove proof-only selection
from ordinary execution, and repeat relevant correctness. Then run, in order:

1. at least three additional ordinary modern Boots;
2. the fresh-Chrome execution-Boot guard;
3. the immutable WANIX shell, `/shared/bench.py`, SHA-256, and shared-9P guard;
4. only after every prior gate passes, the untouched corrected-cadence
   117-trial legacy/rewrite/v86 scorecard.

Promotion requires 13/13 versus legacy, no loss from 11/13 versus v86, no
rewrite row beyond the 3% regression guard, exact browser Python
non-regression, and a reproducible Boot improvement over R085. Stop at the
first failed gate, retain all evidence, remove R093 completely, and restore
the exact R085 source/Wasm/loader identities. Do not repair a failure by
changing family membership, privilege mode, slice size, threshold, callback
shape, or workload selection.

## Result and decision

The exact R066 mechanism was reconstructed without a family, mode, threshold,
or workload selector. The default-off candidate is
`target/bench/wasm-candidates/r093-integrated-scalar-t0-118db9b85ba2.wasm`,
SHA-256 `118db9b85ba281fe6134cfbc71225a636eb15f1877c46a07a5137ef2ab5588ef`.
Its Virt `Cpu::run` and `Cpu::run_until` bodies are 6,138 and 6,331 bytes with
22 branch tables each, matching archived R066. Imports are unchanged and its
only extra export is the proof setter. Exhaustive scalar/RVC units, the
complete Wasm smoke, full-system memory/Sv39/MPRV/atomic/FP/WFI/T2 gates, and
direct/OpenSBI Linux all pass with the mechanism forced on. Seven fresh cold
construction pairs pass narrowly at 1.046x candidate/control elapsed under the
1.05x cap; report SHA-256 is `730f898c54cf...`.

The valid same-Wasm 30-leg report is
`target/bench/r093-integrated-scalar-t0/native-ab/config-ab-2026-08-09T22-55-45-382Z.json`,
SHA-256 `eca7f6874e0c...`. It records Boot 1.045x `[1.029,1.080]`, Compile
1.004x paired speedup with only a 0.6% candidate elapsed cost by raw medians,
and Python 0.993x paired with a 0.2% raw-median elapsed cost. All identities,
outputs, JIT proofs, and host guards pass. This clears the frozen cumulative
screen and confirms that a general 4--7% gain is real and worth evaluating.

The one clean default-on product candidate is Wasm `b40bb5f3f55b...`, with the
same 13-import/170-export ABI as R085. Its repeated correctness matrix passes.
Three ordinary native Boot pairs improve 1.074x `[1.057,1.088]`; the seven-pair
fresh-Chrome guard improves 1.089x `[1.016,1.122]`. Those reports have SHA-256
`6a3d1b7aaa8a...` and `41beaa76504e...` respectively.

The frozen WANIX gate stops promotion. Its valid 7-by-3 immutable-artifact
report is `target/bench/r093-integrated-scalar-t0/wanix-browser-gate.json`,
SHA-256 `e6c701f6422f...`. Paired medians are shell 1.001x, Python 1.000x,
SHA-256 1.002x, and shared 9P 1.040x. Every result and generated-execution
proof is correct, but shared 9P is highly variable and its exact paired
bootstrap interval is `[0.730,1.580]`, below the preregistered 0.909 lower
bound. The median is favorable, but the confidence guard is binding.

Per the frozen stop rule there is no retry, gate relaxation, full scorecard,
or family/mode variant. R093 is rejected. Candidate source, Wasm, Chrome,
WANIX, and timing evidence remain archived. Production is restored exactly:
core source `aec4b31434a6...`, Wasm source `1da35e70bc9c...`, release Wasm
`efd7830307ef...`, loader `2cbb264f4dac...`, and WANIX archive
`0b953be67610...`. Restored core/Wasm units pass 32/32 and 4/4. The score stays
13/13 versus legacy and 11/13 versus copy/v86; Boot and Compile remain open.
