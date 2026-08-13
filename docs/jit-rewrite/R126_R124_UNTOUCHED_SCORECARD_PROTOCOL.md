# R126 Exact-R124 Untouched Scorecard Protocol

Date: 2026-08-10  
Status: frozen before the first scorecard trial

## Decision context

D127 accepts R125 as WANIX qualification under the owner's explicit
one-percent material-regression policy. R125's immutable analyzer output stays
`gatePassed=false`; none of its samples or thresholds is changed and it is not
rerun. This protocol advances the unchanged R124 fixed-register-bank candidate
to the still-untouched authoritative modern three-way scorecard.

This is an incremental promotion decision. One candidate need not close both
remaining copy/v86 gaps. It must make verified progress without losing an
existing parity row, regressing unchanged `/shared/bench.py`, or moving any
protected normalized scorecard row by more than 1%.

## Exact candidate and provenance

| Item | SHA-256 | Bytes |
| --- | --- | ---: |
| candidate Wasm | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| isolated candidate A | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| isolated candidate B | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| new isolated reproduction | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| deterministic source archive | `6497fe464b64113525620f0f1ae4ac767a2137fd09b7d5cb843ba893f5627829` | 168,543 |
| public loader | `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385` | 87,961 |

The new reproduction was built in an isolated temporary copy after overlaying
only the archive's DBT API/emitter, unchanged Wasm runtime, and loader files.
It compares byte-for-byte with all three frozen R124 candidate artifacts. Live
source and release Wasm remained exact control while this proof ran.

## Immutable comparison inputs

| Item | SHA-256 |
| --- | --- |
| accepted R087 authoritative report | `1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7` |
| sealed R125 report | `9e051d1fd4b23c7b440134778e54a500c5fe6eb0cdc058e0d8bf3db359491868` |
| Linux 6.12.7 RV64 Image | `57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2` |
| Alpine 3.24.1 riscv64 scorecard initramfs | `cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808` |
| Alpine 3.24.1 i686 scorecard initramfs | `f626064d8ca2a2031f00b3e6389ba2a65866535df0a143ad0b02ab92a7f70be5` |
| legacy modern-Virt loader | `54df79c8b35cf50bcee34c4af02d7eb02b09e0439b717ee75bb830e733595b12` |
| legacy modern-Virt Wasm | `274aaab5799386956a8c509434961c4a426066f8fc9f520e994c210affd61709` |
| copy/v86 `src/main.js` | `85c609ad27a3ca4e5327280f4264b9ccd35d833c0e3354f5ded08d15e450ef63` |
| copy/v86 Wasm | `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1` |

The copy/v86 source identity remains commit
`2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`. Both RV64 sides execute Linux
6.12.7 / Alpine 3.24.1 riscv64; copy/v86 executes the matched i686 build. No
TinyEMU kernel, BBL firmware, or historical unrelated root is an input.

## Immutable tools

| Tool | SHA-256 |
| --- | --- |
| scorecard runner | `d329d7746987375772941a9535b59345bb2a79ffa304aaf8b2da6b1df5364685` |
| scorecard worker | `346c240378c8763053b7cddd5a093476eb0661ccddb5b8ef47b5f1698c77a175` |
| scorecard library | `377f32f4a5fbd467f4b262ecb8472febf8e3960a12ff40496c91c59009c29186` |
| public cadence helper | `ed7b65cab81fa963804dfd9da6108417e27e7ed3f53b640cc06cdfcf86f3eaaa` |
| benchmark math | `a8bdea3fb105ed63dc69a062077ff86a1bfdd82567df16f4c42925e0685c02b8` |
| scorecard selftest | `8b7acf55effcf5731c002744d95b4aedd9579e00e436f2c9ad6cf6841871b6b5` |
| R126 adjudicator | `836509255e4d1cfd4ad3602ff230d266ec0b885e65f0800da104fe8e7a1ab9c7` |
| R126 adjudicator selftest | `88c052ea574ff4160046fceda43c4c4394879c6cece8e18e9a762ac71cd4369b` |

Both selftests pass before sampling. The R126 selftest proves rejection for a
new v86 loss, a legacy loss, no 1% target gain, a protected normalized result
below `0.99x`, regressed WANIX Python, changed guest input, and changed
candidate identity.

## One formal command

Use Node v26.5.0, CPUs 8--15, production policy, corrected public one-slice
cadence, all three sides, all thirteen rows, three fresh processes per
side/row, and the separate v86 generated-execution preflight:

```sh
ARTIFACTS="$PWD/target/bench" \
LEGACY_ROOT="$PWD/target/scorecard-v2-legacy" \
V86DIR="/home/darren/src/arm64.js/target/bench/v86" \
SCORECARD_V2_REWRITE_WASM="$PWD/target/bench/r124-rvc-bank-hybrid/product-preflight/artifacts/candidate-d017a10f.wasm" \
REPS=3 AUTHORITATIVE=1 \
SCORECARD_V2_OUTPUT="$PWD/target/bench/r126-r124-scorecard/formal" \
taskset -c 8-15 node tests/vs-v86/scorecard-v2.mjs
```

The authoritative runner sets a nonzero exit status whenever full 13/13 v86
parity is not yet reached, even if the 117-trial measurement is valid. Preserve
that exit status, locate the sole emitted JSON, and invoke the frozen R126
adjudicator exactly once. Do not replace, trim, pool, or selectively rerun a
trial. An internal scorecard problem rejects the run.

The formal output directory did not exist at freeze.

## Frozen incremental promotion rule

The adjudicator requires:

1. an authoritative, measurement-valid 117-trial report with no internal
   problem, exact three repetitions, CPUs 8--15, production policy, public
   cadence, active generated execution, exact artifacts/inputs, and the modern
   guest contract;
2. exact candidate `d017a10f...`, exact isolated legacy comparator, and exact
   pinned copy/v86 in every applicable trial;
3. all 13 rows remain at or above the scorecard's `0.95x` legacy parity floor;
4. no new copy/v86 parity loss beyond accepted R087's Boot and Compile;
5. for every row, the candidate's rewrite/v86 ratio divided by accepted R087's
   rewrite/v86 ratio is at least `0.99x`; this common-v86 normalization protects
   against cross-run host drift while enforcing the material boundary;
6. at least one of normalized Boot or Compile improves by at least `1.01x`;
7. sealed R125 remains valid, exact-candidate, at least `0.99x` on every WANIX
   row, and at least parity on unchanged `/shared/bench.py`.

A pass promotes the exact source and Wasm as the new incremental product even
if Boot and Compile remain short of copy/v86 parity. A failure preserves every
artifact and leaves exact control `d9f686a9...` live. No candidate edit, score
weight, normalization, row, threshold, or repeat count changes after freeze.
