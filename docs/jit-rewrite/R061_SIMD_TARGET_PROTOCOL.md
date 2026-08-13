# R061 SIMD target protocol

Date: 2026-08-08  
Status: frozen product screen; production unchanged

## Mechanism

Build the unchanged accepted source with the ordinary WebAssembly SIMD target
feature enabled:

```text
-C target-feature=+simd128
```

The exact copy/v86 comparator enables this same feature in its release build.
The candidate keeps the current export-table/growable-table linker flags and
changes no source, policy, module emitter, guest selector, or workload. No
function-specific vectorization flag or source rewrite is permitted.

## Frozen artifact evidence

- Control SHA-256: `4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`
- Candidate SHA-256: `5cbdfaa3de7f9d6b99b32f5a15c1e915209725ba8c473fbe26332d91237bab47`
- Candidate path:
  `target/bench/r061-simd-build/wasm32-unknown-unknown/release/rv64_wasm.wasm`
- Static disassembly contains 18,565 actual SIMD operations.
- The complete 13-import/170-export ABI matches. JIT-off and JIT-on user
  execution both retire exactly 230,000,332 instructions and match output SHA
  `ea1a7f7c24897c66ea06a0e37f623b298de82db87bbb489112c48c311ffbd1cc`.

## Admission gate

Run three alternating fresh-process pairs for modern Boot and Compile STEADY
on CPUs 8-15 using the existing immutable-artifact A/B harness. Host spread
must be at most 1.25x and all guest fingerprints/counters must match.

The candidate advances only if paired speedup is at least 1.10x on one failing
row, its paired 95% lower bound exceeds 1.05x, and the other row does not
regress more than 3%. Otherwise reject it without source-level SIMD tuning.

Any advancing artifact must still pass the full correctness matrix, untouched
three-way scorecard, and fresh-browser `/shared/bench.py` guard before the
build feature can be promoted.
