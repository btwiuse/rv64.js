# R100 Interleaved Fused-TLB Reconfirmation Protocol

Date: 2026-08-10  
Status: rejected at the frozen native causal gate; candidate archived and
product source restored byte-exact

## Question and independent basis

Can one interleaved `{tag, linear_offset}` entry per load/store fused-TLB slot
let generated RV64 memory use one 128-bit Wasm load instead of two scalar loads,
without changing translation, permission, fault, invalidation, or store-to-code
semantics?

This does not relabel R037. R037's single proof-only screen remains rejected
under its historical 10% rule. It nevertheless supplied mechanism evidence:
the exact architecture-wide representation improved Compile STEADY 6.4% in
one screen while Boot changed by -0.6%. It was removed without replication
solely because it missed 10%. The standing prospective cumulative policy now
admits general 3--5% gains, and R088/R018 independently show that generated
memory is broad: roughly 45% of Compile's scheduled structured instructions
are loads/stores. R099 first ruled out the newly audited scheduler-policy paths
as dynamically dormant.

The immutable control is the clean executable-R085-equivalent Wasm
`d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`
(4,279,380 bytes) and loader
`2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.
Pre-edit source hashes are:

- `crates/rv64-core/src/cpu.rs`: `aec4b31434a6...`;
- `crates/rv64-dbt/src/lib.rs`: `fed2e33326d5...`;
- `crates/rv64-dbt/src/wasm.rs`: `b5e9c11ec1bf...`; and
- `crates/rv64-wasm/src/lib.rs`: `1da35e70bc9c...`.

## Frozen architecture

1. Replace each split fused-TLB row with the same-size interleaved array of
   16-byte `{u64 tag, i64 linear_offset}` entries. Load and store rows remain
   separate.
2. Publication writes both fields of exactly one selected entry. Every full,
   page-scoped, privilege/context, changed-index-policy, and compiled-code-page
   invalidation retains the existing semantic boundary.
3. Generated code computes the identical hashed/direct index and expected
   canonical page-plus-context tag. It performs one `v128.load`, retains the
   vector in one function local, compares lane zero, and consumes lane one only
   after the same proof succeeds.
4. Misses retain the exact refill-or-precise-side-exit behavior. A successful
   refill re-probes and reloads the same entry; a failed refill cannot expose
   the offset lane.
5. Do not change TLB capacity, hash, context encoding, page size, permission
   rows, refill policy, memory-access selector, region formation, thresholds,
   scheduler, workload, guest PC, opcode set, or browser/engine policy.
6. Emit SIMD only for explicitly interleaved system-memory capabilities. Keep
   the scalar emitter valid for standalone/non-product layouts, but ship one
   default-on product candidate. Do not time a SIMD selector or spelling
   variant.
7. Artifact and generated-module byte sizes are diagnostics, never an
   automatic rejection reason.

## Shape and correctness gates

Before performance timing:

1. Unit shape tests prove exactly one `v128.load` and lane extractions replace
   the two scalar row loads for a representative hit, while miss/refill paths
   remain present and generated modules validate.
2. Core tests exercise publication, hits, collision/context misses, page and
   full invalidation, index hashing, store-row invalidation, and privilege
   retention against the interleaved rows.
3. Bare/direct/hashed/refill/side-exit system-memory, Sv39/MPRV, A/atomics, FP,
   bulk-copy, multi-entry, randomized atomic, WFI, lifecycle, and T2 tests pass.
4. Direct and OpenSBI modern Linux reach their exact readiness markers.
5. A fresh Compile opportunity run preserves the exact object MD5, production
   policy, public cadence, and generated execution.

Any semantic mismatch, invalid module, absent SIMD shape, changed output, or
material coverage collapse stops the experiment without timing variants.

## Performance gates

Archive the candidate source and Wasm once. First measure seven alternating
fresh-process control/candidate compile-and-instantiate pairs; candidate cold
elapsed may regress by at most 5% and the report must prove distinct executable
code.

Then run five alternating fresh Node/V8 pairs for Boot, Compile, and Python on
CPUs 8--15, modern artifacts, production policy, and public cadence. All 30
legs must preserve identity, input/output, generated execution, host spread at
most 1.10x, and ordinary sample limits. Advance when:

- Compile paired median speedup is at least 1.03x and its paired-bootstrap 95%
  lower bound is at least 1.00x;
- Boot and Python candidate/control elapsed medians are each at most 1.03x;
- normalized MIPS improves consistently with elapsed time; and
- no correctness or lifecycle proof fails.

If native passes, run the strict complete correctness matrix, three additional
modern Boots, Chrome execution-Boot, and the qualified long-work WANIX guard,
including `/shared/bench.py`. Only then run the untouched 117-trial three-way
scorecard. Promotion requires no loss from 13/13 legacy and 11/13 v86, no
Python regression, and a reproducible Compile improvement. No encoding,
alignment, row-sharing, scalar/SIMD mixture, access-family, engine, or workload
variant is permitted after timing begins.

## Result

The implementation satisfied the frozen architecture. A paired emitter test
proved that one `v128.load` plus lane-zero/lane-one extraction replaced exactly
the scalar tag and offset loads. Core ABI tests proved 16-byte interleaved
entries and separate same-size load/store banks. The complete focused system
memory, Sv39, context, A/atomics, FP, bulk-copy, multi-entry, randomized atomic,
WFI, lifecycle, direct Linux, and OpenSBI Linux gates passed. The immutable
candidate is `c36da489ebe3e2f15d960a1ad393b808e9ff285dc099d4988c745e0e81065b32`
(4,278,772 bytes); its source archive is `7b239d440da6...`.

The seven-pair construction gate (`09179d780325...`) passed. Control and
candidate cold compile-plus-instantiate medians were 5.726 and 5.743 ms, a
1.003x candidate/control ratio. Their CODE payload hashes are distinct
(`b90313ef0c2f...` versus `728c71dfba44...`). The candidate happened to be
608 total bytes and 833 CODE-payload bytes smaller, but neither delta entered
the decision.

The complete five-pair causal report is
`target/bench/r100-interleaved-tlb/native-ab/config-ab-2026-08-10T04-32-39-017Z.json`
(`e4ea8ac0c20b...`). It is measurement-valid with 1.015x host spread, all 30
fresh-process legs, exact modern inputs and outputs, production policy, public
cadence, and generated execution. Results are:

- Boot: 2,173.22 versus 2,209.44 ms, 0.989x paired
  `[0.956,1.018]`; the 1.017x elapsed ratio passes the 1.03 guard.
- Compile: 947.93 versus 936.97 ms, 1.017x paired
  `[0.966,1.083]`; normalized MIPS also improves 1.017x.
- Python: 2,352.09 versus 2,364.61 ms, 0.986x paired
  `[0.961,1.075]`; the 1.005x elapsed ratio passes the 1.03 guard.

Compile fails both preregistered promotion requirements: 1.017 is below the
1.03 median floor and 0.966 is below the 1.00 confidence floor. Gate
`target/bench/r100-interleaved-tlb/native-gate.json` (`d47ffc74b573...`)
therefore rejects R100 before browser, WANIX, or full-scorecard timing. No
encoding, alignment, scalar/SIMD mixture, or access-family variant was tried.

Candidate evidence remains under `target/bench/r100-interleaved-tlb/`. All
product changes were removed. The four pre-edit source hashes are restored
exactly (`aec4b31434a6...`, `fed2e33326d5...`, `b5e9c11ec1bf...`, and
`1da35e70bc9c...`), the release build is exact executable-R085-equivalent
`d9f686a9ce4f...` (4,279,380 bytes), and restored core/DBT units pass 32/32
and 53/53.
