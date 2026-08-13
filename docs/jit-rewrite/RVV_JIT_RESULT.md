# RV64GCV JIT Lowering Result

Date: 2026-08-13
Disposition: semantic JIT lowering complete; correctness and no-material-regression gates pass; direct host-SIMD work remains

## Conclusion

The JIT now accepts the complete mandatory RVV 1.0 surface implemented by the
RV64GCV interpreter for the selected VLEN=128/ELEN=64 machine. Vector
instructions are represented as ordered effects in the DBT IR and lower
through typed user- or system-machine Wasm imports. Generated code commits the
precise pre-instruction architectural state, executes the vector effect, and
then either reloads canonical state or returns for precise interpreter fault
delivery.

This is deliberately a semantic helper lowering, not direct Wasm SIMD. The
surrounding scalar region remains generated Wasm, so RV64GCV binaries no longer
force whole blocks through the interpreter, but each vector instruction still
executes through the shared architectural vector implementation. Direct SIMD
lowering by instruction family is the next performance phase.

The implementation substantially improves the frozen RV64GCV checkpoint and
does not show a material regression on the unchanged scalar scorecard. The
broader historical objective of matching copy/v86 on every JIT scorecard row
is **not achieved**: the authoritative RV64GCV scorecard wins eight of thirteen
rows and loses Boot, Compile, String Sort, FP Emulation, and Assignment.

## Architecture-general implementation

- `rv64-dbt` has an explicit `Effect::Vector` with instruction position,
  fallthrough, and a complete precise side-exit state snapshot.
- `JitLayout` carries a typed `VectorCapability::User` or
  `VectorCapability::System`; a module cannot mix the two ABIs.
- The lifter accepts vector configuration, OP-V, and all supported vector
  memory encodings only when the layout advertises the vector capability.
- The Wasm emitter conditionally imports `env.user_vector` or
  `env.system_vector`. Scalar-only modules have no vector import.
- Successful vector effects invalidate cached integer, floating-point, and
  `fcsr` state before later IR reads. Faults return at the original PC without
  retiring the instruction, preserving partial vector-memory `vstart` state.
- System vector stores use a separate completion status when a compiled code
  page was dirtied, so invalidation remains precise.
- User and system runtimes route the typed calls to the same architectural
  RVV implementation used by the interpreter.

The work also fixed a generic precise-effect bug found by the system
differential: a scalar value computed immediately before an opaque helper
could be absent from the final region outputs and therefore remain stale in
canonical state. Side-exit publication now writes every current integer, FP,
and `fcsr` output required by the effect, independent of final-region
liveness. This rule applies to every precise effect and is not vector- or
workload-specific.

There is no dispatch by guest PC, symbol, program identity, benchmark, input
size, or loop signature. Production selection uses only decoded architectural
instruction fields, memory capability, and machine type.

## Correctness result

The strict release command

```sh
nix develop --command env REQUIRE_ALL=1 ARTIFACTS=target/bench tests/run-all.sh
```

completed with `ALL STAGES PASSED`. Its relevant coverage includes:

- 195 release-mode Rust unit and integration tests;
- all 8,310 QEMU RVV interpreter executions;
- all 8,310 hot-JIT RVV executions with full interpreter/JIT output equality;
- a hot vector load fault with identical PC, retired count, vector state, and
  `vstart=15`;
- full-system vector configuration, scalar/vector state transfer, vector
  load/store, code-dirty handling, and post-vector scalar consumption;
- 134/134 `riscv-tests`, 109/109 Spike lockstep tests, and 193 matching
  architecture-test signatures; and
- the existing integer, memory, M/A, FP, TLB, T2, Linux direct/OpenSBI, Wasm,
  and Virt smoke gates.

QEMU 11.0.3 compares the complete VCSR alias state. QEMU 8.2.7 has two known
reserved-bit WARL defects when directly writing `vxsat` or `vxrm`; only those
two oracle cases compare the architecturally defined target field on 8.2.
A separate core regression proves that the emulator discards reserved bits and
preserves the untouched alias fields.

## Authoritative RV64GCV JIT scorecard

The report is authoritative and measurement-valid: 13 rows, two sides, three
fresh processes per side, 78/78 eligible trials, an empty problem list,
generated-execution proof, maximum host-probe spread `1.0187`, and maximum
scored-sample spread `1.1384`. Lower duration is better; the ratio is v86 time
divided by rewrite time.

| Benchmark | Rewrite RV64GCV JIT | copy/v86 JIT | Ratio |
| --- | ---: | ---: | ---: |
| ALU | 1,782.6 ms | 3,243.7 ms | WIN `1.8196x` |
| Mixed | 1,527.4 ms | 2,184.2 ms | WIN `1.4300x` |
| Matched Boot | 2,171.0 ms | 1,569.5 ms | LOSS `0.7229x` |
| Python fib(30) | 2,028.0 ms | 3,508.9 ms | WIN `1.7302x` |
| Compile (`tcc -c`) | 883.9 ms | 718.4 ms | LOSS `0.8128x` |
| Numeric Sort | 246.0 ms | 330.7 ms | WIN `1.3444x` |
| String Sort | 5,075.7 ms | 646.4 ms | LOSS `0.1273x` |
| Bitfield | 129.8 ms | 206.1 ms | WIN `1.5874x` |
| FP Emulation | 6,757.9 ms | 959.9 ms | LOSS `0.1420x` |
| Fourier | 558.3 ms | 731.9 ms | WIN `1.3109x` |
| Assignment | 1,997.9 ms | 634.7 ms | LOSS `0.3177x` |
| IDEA | 319.5 ms | 724.8 ms | WIN `2.2685x` |
| Huffman | 590.0 ms | 656.3 ms | WIN `1.1124x` |

Evidence:

- JSON:
  `target/bench/rv64gcv-jit-authoritative-v1/scorecard-v2-2026-08-13T03-52-58-367Z.json`
  (`e2f5cb22a686a4e5689285b3d8c4d34e296fcbcd468a9ae5a5f437e232cef2dd`);
- rendered report:
  `target/bench/rv64gcv-jit-authoritative-v1/scorecard-v2-2026-08-13T03-52-58-367Z.md`
  (`b433f24c9e4ef04ccbcc7729dd16be18cffb15dcd0a6c79f9283b6915e976791`);
- measured release Wasm:
  `93c8e3e4e128a1364b63461a0130e207e0a7f1d67233986dd3bd45714318ee57`;
- pinned RVV-capable Linux image:
  `6029e2d5f0c24da911052be961cb7b3c1150206cff76666c8c8eebd8270a78d9`.

## Frozen checkpoint comparison

Checkpoint `9850777` was archived before vector lowering as Wasm
`39092beab711e2875692983c65a0304d50efa3bc75a0ae29bd5955e575e5e34d`.
One fresh-process, all-row diagnostic run compared it with the candidate on
the identical RV64GCV population. Artifact override makes the checkpoint run
ineligible as an authoritative scorecard, but the unchanged row timings give
the intended before/after attribution.

| Row | Checkpoint | Candidate | Candidate speedup |
| --- | ---: | ---: | ---: |
| ALU | 1,797.8 ms | 1,785.5 ms | `1.0069x` |
| Mixed | 1,533.6 ms | 1,532.3 ms | `1.0008x` |
| Boot | 2,180.9 ms | 2,137.0 ms | `1.0206x` |
| Python | 1,998.4 ms | 2,035.4 ms | `0.9818x` |
| Compile | 944.0 ms | 931.0 ms | `1.0139x` |
| Numeric Sort | 256.7 ms | 242.0 ms | `1.0607x` |
| String Sort | 21,460.8 ms | 5,417.3 ms | `3.9616x` |
| Bitfield | 129.7 ms | 129.8 ms | `0.9986x` |
| FP Emulation | 12,475.7 ms | 6,933.2 ms | `1.7994x` |
| Fourier | 572.9 ms | 573.7 ms | `0.9986x` |
| Assignment | 3,105.9 ms | 2,063.2 ms | `1.5054x` |
| IDEA | 327.3 ms | 322.0 ms | `1.0166x` |
| Huffman | 596.5 ms | 588.1 ms | `1.0143x` |

The largest apparent slowdown is Python at 1.85% in a single diagnostic
sample. Bitfield and Fourier differ by 0.14%. The unchanged modern scalar
scorecard provides an independent regression check: its current valid full
run is faster than the previous R087 authoritative rewrite median on twelve
rows and 0.19% slower on ALU. No material scalar regression is established.

Modern regression evidence:
`target/bench/rv64gcv-jit-modern-exploratory-v1/scorecard-v2-2026-08-13T04-10-57-850Z.json`
(`700c89738a28e3358c7ee98f6b3eeb3a9de3f3a3dc6f84619265ed2256e84a12`).
It is measurement-valid and wins all 13 rows against the frozen legacy JIT and
11 of 13 against v86, the same loss set as R087 (Boot and Compile).

## Next boundary

The semantic milestone is complete. Closing the remaining vector-heavy gap
requires direct generated-Wasm lowering, beginning with broad RVV families
that map naturally to Wasm SIMD128 while retaining the helper for uncommon,
faulting, or lane-serial semantics. Any such phase must keep the same complete
instruction differential and frozen scorecards; scorecard observations must
not become workload recognizers.
