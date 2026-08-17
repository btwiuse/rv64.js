# RV64GCV JIT 12-of-13 result

Date: 2026-08-14 America/Phoenix

Disposition: owner-set 12/13 objective achieved; String Sort remains the sole
copy/v86 loss

> **Historical milestone.** Architecture-general work continued after this
> report. Exact Wasm `6dec2dc687f...` now completes the frozen scorecard at
> 13/13 and passes the WANIX Shell gate. See
> [RV64GCV_JIT_13_OF_13_RESULT.md](RV64GCV_JIT_13_OF_13_RESULT.md). The numbers
> below are intentionally retained as the preceding 12/13 checkpoint.

## Conclusion

The frozen `scorecard-v2-rv64gcv-v1` population now records ten wins, two
matches, and one loss for the rewrite JIT against pinned copy/v86. This is
**12/13 non-losses**, the requested target. The report is authoritative and
measurement-valid, uses three fresh processes per side, and has an empty
problem list.

The scorecard runner records `goalMet=false` because its older built-in product
goal requires 13/13. That field is not a failure of the explicitly selected
12/13 milestone. String Sort is still a real, large loss and is not being
reclassified.

No benchmark, binary, guest PC, symbol, input, opcode-sequence, or loop-pattern
recognizer is present. The retained implementation is selected only from
decoded architectural operations, architectural state, memory capabilities,
and deterministic region shape.

## Authoritative RV64GCV scorecard

Lower duration is better. The ratio is copy/v86 duration divided by rewrite
duration.

| Benchmark | Rewrite RV64GCV JIT | copy/v86 JIT | Result |
| --- | ---: | ---: | ---: |
| ALU | 1,783.9 ms | 3,245.1 ms | WIN `1.82x` |
| Mixed | 1,539.4 ms | 2,290.2 ms | WIN `1.49x` |
| Matched Boot | 1,550.1 ms | 1,581.2 ms | MATCH `1.02x` |
| Python fib(30) | 1,933.5 ms | 3,410.5 ms | WIN `1.76x` |
| Compile (`tcc -c`) | 737.5 ms | 737.9 ms | MATCH `1.00x` |
| Numeric Sort | 212.7 ms | 329.4 ms | WIN `1.55x` |
| String Sort | 1,243.7 ms | 650.3 ms | LOSS `1.91x` behind |
| Bitfield | 128.5 ms | 201.9 ms | WIN `1.57x` |
| FP Emulation | 891.5 ms | 954.8 ms | WIN `1.07x` |
| Fourier | 548.5 ms | 724.1 ms | WIN `1.32x` |
| Assignment | 609.9 ms | 642.3 ms | WIN `1.05x` |
| IDEA | 324.4 ms | 736.9 ms | WIN `2.27x` |
| Huffman | 584.0 ms | 648.4 ms | WIN `1.11x` |

Evidence:

- JSON:
  `target/bench/rv64gcv-jit-outlined-authoritative-v1/scorecard-v2-2026-08-15T04-32-10-316Z.json`
  (`308bea55ce444a7c782489c5ee83ecc2a8a39664726813a77db3d0e954537963`);
- rendered report:
  `target/bench/rv64gcv-jit-outlined-authoritative-v1/scorecard-v2-2026-08-15T04-32-10-316Z.md`
  (`5a99fcfccdd49d1079056b2f01ea79dca2d4d52e4205c9ab6ef7fe840f9377ff`);
- input population: `scorecard-v2-rv64gcv-v1`;
- base source commit:
  `013715bcd47dfbe06ee522d22fed4f8692f8129e`; and
- measured release Wasm:
  `261ea50fc40787aa8101f560723175769ada59d780cb6a5a80aeb3db958cc46b`.

## Retained architecture-general mechanisms

RVV instructions remain ordered typed IR effects. Exact families lower to
Wasm SIMD or generated scalar Wasm; every unsupported configuration retains
the same precise interpreter helper. The direct guards cover vtype, VL,
vstart, masking, register grouping, memory extent, translation permissions,
dirty architectural state, and fault replay rather than guest identity.

The final Compile improvement comes from an architecture-general structured
member-range lowering. A region qualifies only when at least three ordinary
loads or unconditional stores share one architectural integer-register root
plus constant offsets and the complete width-inclusive extent fits in one
configured system page. Dense copy/store and bulk shapes are excluded.
Selection and tie-breaking are deterministic.

The system entry path only probes already-established permission/context TLB
rows. It does not refill translations or mutate accessed/dirty state. A guard
hit lets covered members reuse one proven linear-memory base. A miss publishes
precise state and invokes a private ordinary fallback function, preserving the
normal refill, MMIO, page-fault, code-dirty, retirement, and scheduler behavior.
Unit coverage proves that two accesses do not select, three common-root loads
do select, conditional stores do not count, and the generated dispatcher plus
private fallback validates.

## Exact paired attribution and product gates

The final candidate was compared with frozen control Wasm
`8fbe394bd169b2b652a0f06f9cc619797534ec1ce6485b6c1e2a14b4ac2d37af`.
The candidate used by every gate is the live release Wasm above.

- Compile, five alternating pairs: paired speedup `1.09253x`, exact 95%
  interval `[1.08109, 1.21498]`; control median 825.17 ms, candidate median
  742.76 ms. The output fingerprint is identical on every leg.
- Cold Boot, five alternating pairs: `1.00149x`
  `[0.99280, 1.01699]`, neutral.
- Python, five alternating pairs: `1.09819x`
  `[1.06438, 1.11250]`.

Paired evidence:

- Compile JSON:
  `target/bench/gcv-compile-outlined-member-range/compile-ab/config-ab-2026-08-15T03-38-58-312Z.json`
  (`75623c6528f247a832c4dec7859f957ddc7e06e97d4c19529176669a150daa7c`);
- Boot/Python JSON:
  `target/bench/gcv-compile-outlined-member-range/native-ab/config-ab-2026-08-15T03-41-51-652Z.json`
  (`7cbe31237390eb24730ed2ebe5e8f1bd197836a3ddd542c57418b669056d5f7e`).

The mandatory WANIX `/shared/bench.py` guard completed seven alternating
fresh-Chrome pairs with three repetitions per browser. Every artifact,
freshness, identity, active-JIT, output, and 9P work proof passed:

| Row | Paired speedup | Exact paired-bootstrap 95% interval |
| --- | ---: | ---: |
| Shell | `0.99476x` | `[0.99305, 0.99772]` |
| Python | `1.01709x` | `[1.00067, 1.03767]` |
| SHA-256 | `1.00979x` | `[1.00474, 1.01202]` |
| shared 9P | `1.00234x` | `[0.98711, 1.00959]` |

The raw analyzer correctly preserves its older zero-regression verdict because
Shell's confidence interval is below parity. Under standing owner decision
D127, a protected slowdown smaller than 1% is reported but tolerated as
immaterial; rejection occurs below a `0.99x` paired median or when confidence
establishes a slowdown larger than 1%. Shell's 0.524% point slowdown remains
above that floor, so WANIX does not veto the candidate. No raw sample,
threshold, or analyzer result was changed.

WANIX report:
`target/bench/gcv-compile-outlined-member-range/wanix-browser/analysis.json`
(`e02defe9e07bde54628a4f32e3f25379b74027882d9adea3878fa0e86eea7127`).

## Scalar non-regression scorecard

The unchanged scalar population is authoritative, measurement-valid, and has
an empty problem list. It records **13/13 non-losses** against copy/v86: ten
wins and matches for Boot, Compile, and Assignment. It also beats the legacy
JIT on every row.

| Benchmark | Rewrite | copy/v86 | Result |
| --- | ---: | ---: | ---: |
| ALU | 1,783.5 ms | 3,254.2 ms | WIN `1.82x` |
| Mixed | 1,555.5 ms | 2,262.9 ms | WIN `1.45x` |
| Matched Boot | 1,519.8 ms | 1,588.2 ms | MATCH `1.05x` |
| Python fib(30) | 1,982.2 ms | 3,479.0 ms | WIN `1.76x` |
| Compile (`tcc -c`) | 734.8 ms | 738.1 ms | MATCH `1.00x` |
| Numeric Sort | 261.5 ms | 309.3 ms | WIN `1.18x` |
| String Sort | 208.4 ms | 231.3 ms | WIN `1.11x` |
| Bitfield | 170.0 ms | 212.1 ms | WIN `1.25x` |
| FP Emulation | 200.5 ms | 892.0 ms | WIN `4.45x` |
| Fourier | 494.8 ms | 768.6 ms | WIN `1.55x` |
| Assignment | 508.9 ms | 523.6 ms | MATCH `1.03x` |
| IDEA | 285.9 ms | 539.4 ms | WIN `1.89x` |
| Huffman | 272.8 ms | 1,653.5 ms | WIN `6.06x` |

Evidence:

- JSON:
  `target/bench/scalar-jit-outlined-authoritative-v1/scorecard-v2-2026-08-15T05-26-04-571Z.json`
  (`ee7b0c32d91dbd1d3725904597478d9893c9603b4a882338e5f03c13dd5d5b23`);
- rendered report:
  `target/bench/scalar-jit-outlined-authoritative-v1/scorecard-v2-2026-08-15T05-26-04-571Z.md`
  (`41a5deab04da50b17db279e7237099561ede7dc34be31c8edd40085a9ed07526`).

## Correctness and release gate

The exact measured source and release Wasm pass formatting, `git diff
--check`, focused member-range differentials, and the strict command:

```sh
nix develop --command env \
  REQUIRE_ALL=1 \
  ARTIFACTS="$PWD/target/bench" \
  tests/run-all.sh
```

It completed with `ALL STAGES PASSED`. Coverage includes:

- all workspace unit and integration tests;
- guest builds and exact QEMU differentials;
- 8,814 RVV interpreter executions and 8,814 hot interpreter/JIT executions;
- 134/134 official ISA tests;
- 109/109 Spike lockstep comparisons and 24,103 writebacks;
- 193/193 architecture-test signatures;
- integer, M/A, FP, memory, faults, Sv39, WFI, page-policy, T2, atomic-random,
  bulk-copy, and system-vector differentials;
- standalone Wasmtime, direct and OpenSBI modern Linux, public page-policy
  Linux, FP context switching, and AMO equality; and
- full virt-smoke with all guest markers.

The source patch is archived at
`target/bench/gcv-compile-outlined-member-range/outlined-v1-source.patch`
with SHA-256
`45ae9ee0dc3f7e46652bdbabc290a35fea7862885a804dc4aa6fd209b99c3e1f`.

## Remaining boundary

String Sort remains `1.91x` slower than copy/v86 for the frozen RV64GCV
population. Closing that final row is a separate 13/13 objective. It must not
be pursued with a benchmark or instruction-sequence recognizer; any successor
needs an independently justified ISA- or architecture-general mechanism and
the same frozen scorecard, scalar, WANIX, cold-Boot, and strict-correctness
gates.
