# R060 Binaryen post-link protocol

Date: 2026-08-08  
Status: frozen finite screen; production unchanged

## Question

Can an architecture-independent post-link optimization of the accepted main
runtime Wasm remove at least 10% of a complete failing scorecard row without
changing the guest selector, JIT policy, generated modules, or benchmark?

This is broader than the rejected decoder and sequence mechanisms: it can
optimize the complete precompiled interpreter/runtime/helper call graph and
requires no additional runtime compilation. It is also independently
falsifiable because control and candidate are immutable Wasm files consumed by
the existing fresh-process configuration A/B harness.

## Frozen inputs

- Input runtime:
  `target/wasm32-unknown-unknown/release/rv64_wasm.wasm`
- Required input SHA-256:
  `4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`
- Optimizer:
  `/nix/store/8b0awqgjigp3x6y06jq0ndcmmf6jlr00-binaryen-125/bin/wasm-opt`
- The only candidates are the standard whole-program levels `-O1`, `-O2`,
  `-O3`, and `-O4`. No individual pass, pass ordering, function annotation,
  guest-derived frequency, or workload-specific option may be selected.

`-O3` was run exploratorily before this finite protocol was written. It reduced
the file from 4,272,517 to 2,134,850 bytes and passed validation plus one
user-mode JIT workload. Its already observed three-pair product result is
retained rather than reclassified as preregistered: Boot paired speedup 0.940x
and Compile STEADY 0.976x. It is rejected. The remaining standard levels are
frozen here before their product timings.

## Gates

1. Every output must validate, retain the same import/export ABI, complete the
   user-mode JIT benchmark with exact exit/output, and be deterministic when
   emitted twice.
2. Each remaining standard level receives one alternating fresh-process
   Boot/Compile gross screen against exact R054. A level is eliminated if it
   is slower by 5% on either row. This single sample cannot promote a level.
3. At most one surviving level advances: it must have the largest geometric
   mean speedup across Boot and Compile without losing either row. Ties within
   2% prefer the lower optimization level; no new option is introduced.
4. Advancement requires a new five-pair Boot/Compile A/B with stable host
   probes, exact guest fingerprints/counters, paired median speedup at least
   1.10x on one complete failing row, 95% paired lower bound above 1.05x, and
   no more than 3% regression on the other row.
5. Promotion then requires the full RV64 differential/system/modern-Linux
   correctness matrix, an untouched three-way 13-row scorecard, and the fresh
   browser `/shared/bench.py` guard. Final success still requires Boot and
   Compile parity with copy/v86; a partial gain is only a new baseline.

If no standard level passes, Binaryen post-link optimization is closed in its
tested whole-program form. Do not search individual pass combinations against
the scorecard.
