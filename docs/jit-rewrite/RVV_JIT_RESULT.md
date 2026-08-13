# RV64GCV direct JIT lowering result

Date: 2026-08-13 America/Phoenix

Disposition: direct lowering promoted; correctness and scalar-regression gates
pass; cross-ISA parity remains open

## Conclusion

The RV64GCV JIT now lowers broad, architecture-defined RVV families directly
to generated WebAssembly SIMD or scalar Wasm loops. The complete typed helper
lowering remains the precise fallback for configurations or operations that
cannot be represented exactly. Selection uses decoded instruction fields and
runtime architectural state only; it does not inspect guest PCs, symbols,
binaries, benchmark names, inputs, or loop signatures.

The authoritative frozen RV64GCV scorecard remains **8/13**, but its three
vector-heavy losses improved substantially relative to the helper-only
checkpoint:

- String Sort: `5,075.7 -> 1,467.5 ms` (`3.4587x` faster);
- FP Emulation: `6,757.9 -> 3,760.6 ms` (`1.7970x` faster); and
- Assignment: `1,997.9 -> 1,547.0 ms` (`1.2915x` faster).

The unchanged scalar population remains **11/13** against copy/v86. Its only
losses are still Boot and Compile. The apparent 8/13 result is therefore not a
regression from 11/13: it is the result for a different, RV64GCV-compiled
binary population.

The broader parity objective is not achieved. RV64GCV still loses Boot,
Compile, String Sort, FP Emulation, and Assignment.

## Architecture-general lowering

The IR records a conservative `VectorDirect` description beside every ordered
vector effect. Generated code selects a direct path only after proving the
current architectural state is compatible; otherwise it invokes the same RVV
implementation used by the interpreter.

Promoted direct families include:

- integer add/subtract, min/max, bitwise, scalar/immediate shifts, multiply,
  and vector/scalar/immediate moves;
- unmasked unit-stride transfers and positive-stride transfers;
- whole-register loads, stores, and moves;
- integer scalar element insert/extract;
- immediate slides, slide-one permutations, and immediate gather;
- one-register-group integer comparisons with exact packed mask writes; and
- flag-free FP sign injection, bit broadcasts, scalar element moves, and
  slide-one permutations.

The lowering supports integer and fractional LMUL at VLEN=128. Partial
fractional groups use lane-width stores so bytes above the architectural group
remain unchanged.

Ordinary direct paths require a valid e8/e16/e32/e64 vtype, `vstart=0`,
`vl=VLMAX`, legal register alignment, and an unmasked instruction. Whole-
register transfers use their separate architectural rules. Direct memory
requires an exact in-range extent; system execution additionally proves one
fused-TLB direct-RAM page and the current translation context. Strided paths
also bound the stride before multiplication and prove that the last address
cannot wrap. Page crossings, MMIO, missing translations, code-dirty stores,
negative or wrapping strides, and faulting/restarted operations retain the
helper.

FP arithmetic, reductions, conversions, and fused operations also retain the
helper because ordinary Wasm SIMD cannot reproduce RVV rounding and `fflags`
for every input. This boundary is semantic, not benchmark-driven.

## State and fault correctness

Direct vector effects still use the precise effect boundary established by the
semantic lowering:

- pending integer, FP, `fcsr`, PC, and retirement state is published before
  the instruction;
- successful direct effects reconcile the exact cached scalar/FP outputs that
  publication may have changed;
- scalar- or FP-producing vector moves reload canonical cached state;
- system paths enforce and dirty `mstatus.VS` and, where required,
  `mstatus.FS`; and
- a guard miss or helper fault returns at the unretired instruction with exact
  `vstart` and partial-memory state.

The direct-path counter is opt-in for differential tests. Production
scorecards leave it disabled, so measurement does not add a hot counter update.

## Correctness result

The strict release command

```sh
nix develop --command env REQUIRE_ALL=1 ARTIFACTS=target/bench tests/run-all.sh
```

completed with `ALL STAGES PASSED`. Relevant coverage includes:

- the complete workspace Rust suite;
- 8,724 QEMU RVV interpreter comparisons;
- 8,724 hot interpreter/JIT equality comparisons across 1,454 encodings, six
  data profiles, and 128 repetitions, with 22,997 direct executions per
  profile;
- focused coverage for the RVV unsigned shift-immediate encoding at e64,
  including the corrected shared interpreter semantics for uimm values 16--31;
- exact user fault restart at `vstart=15`;
- hot full-system direct vector load/store, strided memory, state publication,
  and post-vector scalar consumption, including a dedicated fractional-LMUL
  system-memory case with 393,094 direct executions;
- 134/134 `riscv-tests`, 109/109 Spike lockstep tests, and 193 matching
  architecture-test signatures; and
- all integer, memory, M/A, FP, TLB, T2, Linux direct/OpenSBI, Wasm, and Virt
  smoke gates.

## Authoritative RV64GCV scorecard

The report is measurement-valid: 78/78 eligible timed trials, three fresh
processes per side, an empty problem list, generated-execution proof, maximum
host-probe spread `1.0123`, and maximum scored-sample spread `1.1526`. Lower
duration is better; the ratio is v86 time divided by rewrite time.

| Benchmark | Rewrite RV64GCV JIT | copy/v86 JIT | Ratio |
| --- | ---: | ---: | ---: |
| ALU | 1,779.5 ms | 3,237.1 ms | WIN `1.8192x` |
| Mixed | 1,528.9 ms | 2,276.6 ms | WIN `1.4890x` |
| Matched Boot | 2,176.8 ms | 1,564.3 ms | LOSS `0.7186x` |
| Python fib(30) | 2,027.0 ms | 3,347.9 ms | WIN `1.6517x` |
| Compile (`tcc -c`) | 931.5 ms | 727.9 ms | LOSS `0.7814x` |
| Numeric Sort | 222.9 ms | 344.6 ms | WIN `1.5461x` |
| String Sort | 1,467.5 ms | 641.3 ms | LOSS `0.4370x` |
| Bitfield | 129.6 ms | 198.4 ms | WIN `1.5310x` |
| FP Emulation | 3,760.6 ms | 965.3 ms | LOSS `0.2567x` |
| Fourier | 578.4 ms | 693.1 ms | WIN `1.1984x` |
| Assignment | 1,547.0 ms | 639.7 ms | LOSS `0.4135x` |
| IDEA | 324.5 ms | 739.6 ms | WIN `2.2792x` |
| Huffman | 597.2 ms | 651.5 ms | WIN `1.0909x` |

Evidence:

- helper-only baseline JSON:
  `target/bench/rv64gcv-jit-authoritative-v1/scorecard-v2-2026-08-13T03-52-58-367Z.json`
  (`e2f5cb22a686a4e5689285b3d8c4d34e296fcbcd468a9ae5a5f437e232cef2dd`);
- JSON:
  `target/bench/rv64gcv-jit-direct-simd-authoritative-v3/scorecard-v2-2026-08-13T07-38-57-253Z.json`
  (`5067d5e42ab88998da86b267f152dbf1e31781388a7828b25b90acb1f6da0584`);
- rendered report:
  `target/bench/rv64gcv-jit-direct-simd-authoritative-v3/scorecard-v2-2026-08-13T07-38-57-253Z.md`
  (`6d13ac92bf0dae53306355e0abda027bedb4c1502645d66dd8a839531dc0c7a1`);
- measured release Wasm:
  `87c2ed0d39642144941cabc1c1837e10e36c51e9299d1950a0e143215cd9e24b`;
- pinned RVV-capable Linux image:
  `6029e2d5f0c24da911052be961cb7b3c1150206cff76666c8c8eebd8270a78d9`.

## Scalar regression scorecard

The complete two-side modern scalar rerun is measurement-valid with 78/78
eligible trials, an empty problem list, maximum host-probe spread `1.0700`, and
maximum sample spread `1.1246`. It wins 11/13 against v86 and loses only Boot
and Compile.

Evidence:

- JSON:
  `target/bench/rvv-direct-simd-modern-regression-v3/scorecard-v2-2026-08-13T07-46-37-219Z.json`
  (`4864a90d280d965806897b62a27e0d4888c4818607d2d3d329aebb33c5d5a59a`);
- rendered report:
  `target/bench/rvv-direct-simd-modern-regression-v3/scorecard-v2-2026-08-13T07-46-37-219Z.md`
  (`5736ca85e26d38879e5b9d50e1e7ed189f2cbfc9740eccd5e8b1cc49792e1e29`).

## Focused promotion evidence

Integer comparison lowering received a five-pair alternating A/B because its
first unpaired Assignment result was ambiguous. Against the identical build
with comparison lowering disabled, the candidate was neutral on String
(`0.992x`, 95% CI `[0.975,1.010]`) and improved Assignment `1.096x` (95% CI
`[1.045,1.111]`). The report is
`target/bench/rv64gcv-jit-compare-paired-ab/config-ab-2026-08-13T06-23-35-787Z.json`.

An attempted integer-reduction lowering was removed before promotion because
it regressed Assignment. No benchmark-specific recognizer or special case was
retained.

## Remaining boundary

The direct phase is promoted, but parity is still open. The remaining vector
gap is concentrated in exact helper families such as masked/strided segment
operations, reductions, widening/narrowing integer operations, and FP
arithmetic whose flags and rounding cannot be represented naively by Wasm
SIMD. Future work must preserve the same fallback, full differential, frozen
population, and scalar regression gates.
