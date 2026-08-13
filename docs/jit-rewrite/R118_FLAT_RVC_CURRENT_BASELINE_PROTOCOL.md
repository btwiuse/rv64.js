# R118 current-baseline flat RV64C dispatch

Date: 2026-08-10  
Status: completed; rejected at frozen native Gate B and restored

Final result: `R118_FLAT_RVC_CURRENT_BASELINE_RESULT.md`.

## Question and corrected admission

Does replacing the interpreter's nested RV64C quadrant/funct3 match with one
complete 24-family selector produce a verified end-to-end Boot gain of at
least 1% on the exact current product without regressing Compile or Python?

R059 tested this exact architecture-wide control shape in a deterministic
balanced Wasm model. All seven steady pairs favored the flat form and measured
`1.592x` local throughput `[1.592,1.617]`, but one earlier warm call caught a
tier-publication transition and exceeded its then-frozen 1.25 spread limit.
The current scorecard methodology separates FIRST, PRIME/tier-up, and STEADY;
it does not reject a stable steady result because publication occurred during
warmup. D072's no-product clause is therefore superseded only for the single
R118 reconstruction below.

R059 supplies admission evidence only. None of its timings, artifacts, opcode
weights, or thresholds earns product credit. R118 must pass fresh current-
baseline correctness, construction, native, browser, WANIX, and scorecard
gates under R104/R107.

## Immutable control and candidate

The control is:

- CPU source `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`;
- Wasm runtime source `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- release Wasm `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`,
  4,279,380 bytes;
- loader `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- Linux 6.12.7 / Alpine 3.24.1 inputs, R087 one-slice cadence, and the
  corrected scorecard contract; and
- official 13/13 legacy and 11/13 copy/v86 score.

Target native row: Boot. Protected native rows: Compile and Python.

The sole candidate changes only `Cpu::step_compressed`:

1. Compute `family = ((c & 3) << 3) | ((c >> 13) & 7)` once.
2. Replace the outer three-way quadrant match plus its three inner eight-way
   funct3 matches with one exhaustive match for families 0 through 23.
3. Preserve each complete family body, operand extraction, legality check,
   exception order, memory operation, PC update, stop, and retirement exactly.
4. Preserve the zero-encoding check and the impossible 32-bit-prefix fallback.

### Pre-timing structural clarification

The first deterministic candidate build retained one extra cold `panic_fmt`
call because LLVM did not propagate the existing caller guarantee through the
combined integer selector. Before any performance sample, the implementation
made that pre-existing invariant explicit (`c & 3 != 3`) immediately before
computing `family`. This preserves the same impossible-prefix fallback and
does not alter any valid or illegal RVC family. The replacement builds are
byte-identical, retain exactly the control's direct/indirect call topology,
and leave every non-`Cpu::step` function body byte-identical. The superseded
precondition-less build is diagnostic only and cannot be timed or promoted.

The candidate may not reorder families by frequency, split or merge handlers,
outline or inline a helper, change fetch/decode/memory behavior, inspect a PC,
binary, symbol, workload, benchmark, output, browser, or engine, or compose any
other experiment. There is no selector, threshold, mode, or partial opcode
variant. Source and Wasm size are diagnostics only.

## Gate A: exact shape and correctness

Before any candidate performance sample:

1. Build the release Wasm twice from the completed candidate and require
   deterministic executable code and a validating module.
2. Decode every named `Cpu::step` monomorphization in control and candidate.
   Each candidate specialization must remove exactly three `br_table`
   operators, replace the current initial quadrant-plus-three-family tables
   with one complete combined-family table, and add no direct or indirect call.
   All non-`Cpu::step` defined function bodies must remain byte-identical.
3. Run the exhaustive 65,536-encoding direct-RVC-versus-expanded-reference
   test, including illegal encodings, plus all rv64-core/system/DBT units.
4. Run the complete scalar, RVC, M, A, FP, memory, Sv39/MPRV, WFI, T2,
   public/Worker, and raw-Wasm differential matrix.
5. Fresh direct and OpenSBI Linux 6.12.7 boots must reach readiness, execute a
   shell command, and prove nonzero generated execution.

A correctness repair may restore only one of the frozen family bodies and must
add a directed regression. It may not alter the selector or performance scope.
Failure stops R118 and restores exact control.

## Gate B: construction-debited native timing

Freeze candidate/control artifacts, source archive, harness, input hashes,
pair order, statistics, and gates before the first candidate timing.

1. Run 15 alternating CPU-pinned fresh-process pairs of the real
   `await RV64Debug.create(wasmBytes)` path. R107's debit `D` is the
   nonnegative upper endpoint of the paired-bootstrap 95% interval for median
   candidate-minus-control milliseconds.
2. Run 15 alternating fresh-process pairs for Boot, Compile, and Python on
   CPUs 8--15. Use exact fixed work, modern inputs, production policy, R087
   cadence, and complete output/work/generated-execution/host proofs. Do not
   replace a leg or extend the sample.
3. Add `D` once to each candidate runtime before analysis. Generated-module
   construction already occurs inside row timing and is not charged twice.

R118 advances only if:

- adjusted Boot paired-median speedup is at least `1.01x`, its 95% lower
  endpoint is at least `1.00x`, and adjusted normalized MIPS agrees at
  `1.01x` or better;
- adjusted Compile and Python paired medians are each at least `0.99x`, with no
  interval establishing regression;
- every artifact, input, output, cadence, policy, generated-execution,
  affinity, host-spread, and within-side stability proof passes.

An unresolved point estimate is inconclusive, not a win or proof of neutrality.
Do not reuse R059 timings, add pairs, select a raw statistic, or change the
candidate after observing Gate B.

## Gate C: browser, WANIX, and authority

Only a Gate-B survivor proceeds, without changing source or artifact:

1. Run 15 alternating fresh Chromium Worker pairs with execution-only and
   construction-to-ready Boot clocks. Require the same `0.99x` protected
   median and no-established-regression rule.
2. Run the R094-qualified seven-browser-by-three fixed-work WANIX guard,
   including unchanged `python /shared/bench.py`. Every phase must retain at
   least `0.99x` paired median with no confidence evidence of regression.
3. Run the untouched corrected-cadence 117-trial legacy/rewrite/v86 scorecard.

Promotion requires preservation of 13/13 legacy and at least 11/13 v86 rows,
the verified Boot gain, and every protected condition. A complete pass makes
the unique flat implementation the new product baseline. At the first failure,
archive exact evidence, restore control, and do not try a family layout,
frequency order, compiler spelling, or benchmark-specific successor.

## Gate A result and Gate B freeze

Gate A passed before any performance observation:

- candidate CPU source is `a0ac3777e5e7...`;
- two independent release builds are byte-identical candidate
  `0501207f314f...`, 4,280,734 bytes (a diagnostic +1,354 bytes);
- shape report `3d2f10034f8e...` proves all six `Cpu::step`
  specializations replace initial table lengths `4,9,9,9` with one complete
  length-25 table, remove exactly three `br_table` operators, preserve all
  direct/indirect call counts, and leave every non-step function body
  byte-identical;
- the exhaustive 65,536-encoding RVC equivalence test passes; and
- the strict Nix release suite passes 134/134 ISA tests, 109/109 Spike
  locksteps, 193/193 architecture signatures, the complete differential/API/
  raw-Wasm matrix, direct and OpenSBI Linux 6.12.7 with generated execution,
  and modern virt-smoke.

The 15-pair construction, 90-leg native A/B, and mechanical decision commands
and hashes are frozen in `target/bench/r118-flat-rvc/FROZEN_INPUTS.txt`.
`r118-native-gate-selftest.mjs` proves both the 1% target boundary and the
protected-row rejection path. No R118 construction or runtime performance
sample had been collected at this freeze.
