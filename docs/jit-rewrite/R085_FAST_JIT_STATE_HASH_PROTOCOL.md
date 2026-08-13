# R085 Fast JIT-State Hash Protocol

Date: 2026-08-09  
Status: promoted cumulative baseline; copy/v86 parity remains open

## Hypothesis

R084 measures Rust default hashing at 5.062% of exact R080 Boot and 3.619% of
Compile STEADY, excluding another 1.524%/0.783% of hash-table probe self time.
Its same-Wasm opportunity corpus measures the proposed integer-key hasher at
5.508x and the representative table mix at 3.021x. Crediting only exclusive
hash work projects 1.043x Boot and 1.031x Compile STEADY. This is a valid
cumulative candidate under the standing 3% rule; it is not expected to close
either remaining copy/v86 gap by itself.

## Frozen implementation

Add a private `FastBuildHasher`/`FastHasher` to `rv64-wasm`. The state update is
the exact R084 corpus operation: XOR the prior state with the input word plus
`0x9e3779b97f4a7c15`, then apply the two-multiply mx3 avalanche using
`0xd6e8feb86659fd93`. Override every integer `Hasher::write_*` method used by
the runtime; the byte fallback consumes complete little/native-endian u64 words
and one length-tagged tail. `finish` returns the already-avalanched state.

The multiplier is odd, making the one-word transform a permutation over u64.
The production builder must additionally carry a different nonzero seed. A
thread-local seed sequence is initialized once from the existing
`host_random` cryptographic import and advanced through the same avalanche for
each builder. This preserves per-instance unpredictability without any host
call on lookup or insertion. Raw hash values are internal and never persisted
or exposed.

Define private `FastHashMap<K,V>` and `FastHashSet<K>` aliases and use them for:

- every map/set field in `JitState`, including nested attempted/installed sets;
- temporary sets created by rv64-wasm JIT invalidation, region construction,
  and template indexing;
- rv64-wasm JIT helper signatures that traffic in those sets.

Do not change `rv64-dbt`, `rv64-core`, `rv64-system`, dispatch geometry, map
capacity, iteration/sort policy, JIT thresholds, module selection, page-policy
semantics, or generated code. Do not select by guest PC, symbol, binary,
workload, browser, checksum, scorecard row, or compiler output. The candidate
must have no runtime switch: R080 is retained as an immutable Wasm comparator,
and rejection restores source from the pre-edit archive.

## Correctness gates

Before performance:

1. unit tests prove stable same-seed hashing, different seeded state, no
   full-width collisions across at least one million sequential and page-shaped
   u64 keys, tuple order sensitivity, and map/set semantic equivalence;
2. all existing core, DBT, system, QEMU, ISA, Spike, architecture-signature,
   Wasm, JIT differential, Sv39/MPRV, A/atomic, FP, WFI, T2, wasmtime,
   direct-Linux, OpenSBI, virt, public API, and Worker API gates pass under
   `REQUIRE_ALL=1 tests/run-all.sh`;
3. run at least three fresh modern Boot smoke processes to cover different
   randomized builders and require the same shell/output contract;
4. candidate Wasm validates and retains the exact R080 import/export contract.

Guest results must match exactly. Adaptive module counts, generated coverage,
and retirement may vary only within the already-randomized R080 policy and are
reported rather than silently normalized.

## Frozen performance order

Archive and hash the pre-edit source and exact R080 Wasm first. After
correctness, build one hash-named candidate and never rebuild it between gates.

1. **Cold artifact gate:** seven alternating fresh Node processes compile R080
   and candidate Wasm. Candidate median cold `WebAssembly.compile` time may not
   regress more than 5%; import/function/export/table structure and byte size
   are recorded.
2. **Native cumulative gate:** five alternating fresh R080/candidate pairs on
   CPUs 8-15, exact prepared modern artifacts, rows Boot, Compile, and Python.
   Require valid inputs/outputs, host spread at most 1.10x, and no failed leg.
   At least one open target row must have paired median speedup >=1.03x; its
   paired 95% interval must exclude a material regression. Every non-target
   elapsed median must be <=1.03x candidate/control. Do not rerun, drop, or
   replace a pair.
3. **Browser guard:** if native passes, create one immutable candidate/control
   archive/page and run the standing fresh Chrome candidate/control execution
   Boot plus `/shared/bench.py`, SHA-256, and shared-9P gates. Python is a hard
   no-regression row under the already-frozen R080 browser limits.
4. **Promotion:** only after browser success, run the untouched 117-trial
   modern legacy/rewrite/v86 scorecard. It must remain 13/13 versus legacy,
   improve or preserve the v86 parity count, and introduce no guarded row
   regression. A retained 3-5% gain that does not yet change 11/13 is still a
   valid cumulative promotion if every preceding gate passes.

Stop immediately at the first failed gate. Preserve the candidate Wasm,
source archive, raw report, checksums, and reason; remove all candidate source
and rebuild byte-exact R080. No seed, multiplier, map subset, capacity, or
operation-family sweep is permitted after observing product timing.

## Result

R085 passed every frozen gate without a rerun, dropped pair, multiplier/seed
variant, map-subset sweep, or post-timing policy change.

The pre-edit source archive is
`target/bench/r085-fast-jit-state-hash/source-before-edit.tgz`, SHA-256
`8e833d674728d5d6a5e45e4b6b48cb6ea0ec7614cf26f21473da1aa97198ab32`.
The one frozen candidate is the 4,279,378-byte
`target/bench/wasm-candidates/r085-fast-jit-state-efd7830307ef.wasm`, SHA-256
`efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`.
It retains R080's 13 imports and 170 exports. The release loader remains
`2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.

Correctness completed under `nix develop --command bash -lc
'REQUIRE_ALL=1 tests/run-all.sh'`: core, DBT, system, QEMU, ISA, Spike
lockstep, architectural signatures, Wasm/JIT differentials, memory/Sv39/A/FP,
WFI/T2, wasmtime, direct/OpenSBI Linux, virt, public, and Worker gates all
passed. Three additional randomized modern-Linux processes passed both boot
paths. The new hash tests cover stable/different seeds, tuple order, one
million sequential keys, one million page-shaped keys, and 65,536 map entries.

The cold-compile report
`target/bench/r085-fast-jit-state-hash/cold-compile.json` has SHA-256
`b3811280c4c92250cc9ab26c52c9150ec4ec148c53f5c8a088425c8d6f91e5b2`.
Across seven alternating fresh Node/V8 processes, median main-module compile
time fell from 5.672 ms to 5.553 ms, or 2.1%; the 5% regression cap passed.

The 30-leg native report
`target/bench/r085-fast-jit-state-hash/native-ab/config-ab-2026-08-09T16-56-42-508Z.json`
has SHA-256
`8d4218279ea297028ec74523763e90058b521cc62bca1c827386e697628865a7`.
It is measurement-valid with exact inputs/outputs, no problems, and 1.061x
host spread. Paired results are:

- Boot 1.017x, interval `[0.989,1.049]`;
- Compile STEADY 1.051x, interval `[1.044,1.241]`;
- Python STEADY 0.994x, interval `[0.992,1.066]`.

The execution-only modern Chrome gate used seven alternating fresh browser
process/profile/Worker/guest pairs. Protocol SHA-256 is
`6eeb7ed1f509dbfc5ec4f999365b2f0a6daa78770f73fadfedba527072d27c36`;
analysis SHA-256 is
`abe63207f827b316399b0f5e762e7038e3dfce4c9b05cc413580b91a777d2859`.
All seven pairs favored R085: paired Boot is 1.023x with exact-bootstrap
interval `[1.023,1.057]` on Linux 6.12.7 / Alpine 3.24.1, with generated
retirement and exact production policy proved in every leg.

The public WANIX integration guard used seven more alternating fresh browsers
and three synchronized repetitions per phase. Protocol SHA-256 is
`6845bc6b37225f02d18ac63dcb27c60f16d9c0c717d1b33ff2ce592d5c112be4`;
analysis SHA-256 is
`5ee4f43f457fec72925512d33061edc4c04a1e05afa38e55f0d1966c51b519aa`.
Paired medians and intervals are shell 1.000x `[0.996,1.012]`, Python 1.028x
`[1.012,1.044]`, SHA-256 1.008x `[0.997,1.013]`, and noisy shared 9P 1.068x
`[0.942,1.516]`. All 126 phase results, checksums, fetched artifact identities,
and generated-execution proofs passed. This public integration root is the
standing Alpine 3.22.5 `/shared/bench.py` guard; the authoritative emulator
scorecard below remains the required Alpine 3.24.1 guest.

The untouched authoritative report is
`target/bench/r085-authoritative-three-way/scorecard-v2-2026-08-09T18-32-14-460Z.json`,
SHA-256
`d733df2124a7388876f6566db71b6f67bf7cdc4dccc2ece5e1df2903c27d7479`.
It completed all 117 trials, is measurement-valid, has no problems, remains
13/13 versus legacy and 11/13 versus copy/v86, and preserves every guarded
row. Relative to exact R080, authoritative rewrite medians improve:

- Boot: 2,338.368 ms to 2,245.209 ms, 1.041x;
- Compile: 1,058.842 ms to 1,016.241 ms, 1.042x;
- Python: 3,085.727 ms to 3,034.673 ms, 1.017x.

The frozen promotion gate is
`target/bench/r085-fast-jit-state-hash/promotion-gate.json`, SHA-256
`e6e1c89f783c53d1cccb0527a40435585c03f4ee237dfb452fe9c23fd3b209a7`.
The two small negative point estimates are FP Emulation 0.995x and Assignment
0.986x, both inside the prospectively fixed 3% guard. Every other row is flat
or faster.

Promote R085. The production JIT archive is the immutable tested archive
`0b953be67610e130f79a852f86542c8400ad3a235001ec450fbdffc29ed3a61a`;
the production comparison page is
`7b6e2eb1510992306356857d3a8726f6e6ca17dd7612bcd61b313b814d6c25cf`.
A fresh post-promotion browser loaded those exact identities, reached the
WANIX shell, produced Python checksum `38460b78`, and proved 98.2% generated
coverage in the phase. The parity objective remains open: Boot is now 1.408x
the v86 elapsed time and Compile is 1.378x, requiring approximately 29.0% and
27.4% further rewrite-time reductions respectively.
