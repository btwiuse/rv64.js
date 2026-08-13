# R127 Exact-R124 Scorecard Replacement Protocol

Date: 2026-08-10  
Status: frozen before the first R127 scorecard trial

## Why R127 replaces R126

R126 is preserved as an invalid attempt, not a performance result. Its explicit
`SCORECARD_V2_REWRITE_WASM` override correctly made every RV64 result
diagnostic-only, while its missing matched x86 kernel prevented every copy/v86
trial from producing a result. R126 scheduled all 117 trials but admitted zero
eligible trials and therefore contributes no medians, no performance evidence,
and no samples to R127.

R127 starts all 117 trials again from zero. It changes no candidate, workload,
repeat count, score, threshold, or promotion rule. The only repairs are to run
the exact candidate as the live release build without a diagnostic override,
restore the already accepted matched x86 kernel, and require all external
copy/v86 boot inputs during runner preflight.

## Exact candidate and provenance

| Item | SHA-256 | Bytes |
| --- | --- | ---: |
| live release Wasm | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| frozen R124 candidate Wasm | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| independent R126 reproduction | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| deterministic candidate source archive | `6497fe464b64113525620f0f1ae4ac767a2137fd09b7d5cb843ba893f5627829` | 168,543 |
| live DBT API source | `e4be5025e892f417fdde56dfc0c4c5ead632b474de9ea849558b9ed7ffbed795` | |
| live DBT Wasm emitter source | `f60b8ae438cf1fcec7dc22215e6e3da5caf1755a7d5c214ec09b5928eb54d96e` | |
| unchanged Wasm runtime source | `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339` | |
| public loader | `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385` | 87,961 |

The live source and release build are the exact R124 fixed-register-bank
candidate. No code change is permitted between this freeze and adjudication.
Exact pre-candidate source backups remain available if the gate rejects it.

## Immutable guest and comparator inputs

| Item | SHA-256 |
| --- | --- |
| Linux 6.12.7 RV64 Image | `57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2` |
| Alpine 3.24.1 riscv64 scorecard initramfs | `cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808` |
| Alpine 3.24.1 i686 scorecard initramfs | `f626064d8ca2a2031f00b3e6389ba2a65866535df0a143ad0b02ab92a7f70be5` |
| matched Linux i686 kernel | `8854efec5534d0badf98aa34f7e7cb37fe3626d4d32d3a6909ca7fad8047acb5` |
| legacy modern-Virt loader | `54df79c8b35cf50bcee34c4af02d7eb02b09e0439b717ee75bb830e733595b12` |
| legacy modern-Virt Wasm | `274aaab5799386956a8c509434961c4a426066f8fc9f520e994c210affd61709` |
| copy/v86 `src/main.js` | `85c609ad27a3ca4e5327280f4264b9ccd35d833c0e3354f5ded08d15e450ef63` |
| copy/v86 Wasm | `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1` |
| SeaBIOS | `73e3f359102e3a9982c35fce98eb7cd08f18303ac7f1ba6ebfbe6cdc1c244d98` |
| VGA BIOS | `a4bc0d80cc3ca028c73dafa8fee396b8d054ce87ebd8abfbd31b06b437607880` |

The copy/v86 source identity is commit
`2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`. Both RV64 sides execute Linux
6.12.7 and Alpine 3.24.1 riscv64; copy/v86 executes the matched i686 build. No
TinyEMU kernel, BBL firmware, or unrelated historical root is an input.

## Immutable evidence and tools

| Item | SHA-256 |
| --- | --- |
| accepted R087 authoritative report | `1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7` |
| sealed R125 WANIX report | `9e051d1fd4b23c7b440134778e54a500c5fe6eb0cdc058e0d8bf3db359491868` |
| invalid R126 report | `ac096fecb8686eb76288af6409adeaf6b86808efc58310a52847197b90ee0d1f` |
| one-percent policy | `cbceb4843ff1542b954763e6537673e834104907836c76eeebfa977f20bea896` |
| corrected scorecard runner | `a3f5922ea981b4193e16c4b32f0d9dc229bfdbb036072c84ce6e6ffde4d2f399` |
| corrected scorecard selftest | `17d69d8dbe250d5f3fa725f52548794dd97f375acee9e468a825840e8874eccc` |
| scorecard worker | `346c240378c8763053b7cddd5a093476eb0661ccddb5b8ef47b5f1698c77a175` |
| scorecard library | `377f32f4a5fbd467f4b262ecb8472febf8e3960a12ff40496c91c59009c29186` |
| public cadence helper | `ed7b65cab81fa963804dfd9da6108417e27e7ed3f53b640cc06cdfcf86f3eaaa` |
| benchmark math | `a8bdea3fb105ed63dc69a062077ff86a1bfdd82567df16f4c42925e0685c02b8` |
| frozen R126/R127 adjudicator | `836509255e4d1cfd4ad3602ff230d266ec0b885e65f0800da104fe8e7a1ab9c7` |
| adjudicator selftest | `88c052ea574ff4160046fceda43c4c4394879c6cece8e18e9a762ac71cd4369b` |
| proof-only v86 dispatch log | `2537cffdafd84d5f1c10eb5b30c862b38de452e6e0895ca5e3a1d95934347709` |

Both selftests pass at freeze. The runner now preflights the matched x86
kernel, v86 JavaScript/Wasm, and both BIOS files before scheduling a trial. A
proof-only v86 worker dispatch also exited successfully after the kernel was
restored; it is deliberately ineligible performance evidence and is not
pooled with the formal run.

## One formal command

Use Node v26.5.0, CPUs 8--15, production policy, corrected public one-slice
cadence, all three sides, all thirteen rows, and three fresh processes per
side/row:

```sh
ARTIFACTS="$PWD/target/bench" \
LEGACY_ROOT="$PWD/target/scorecard-v2-legacy" \
V86DIR="/home/darren/src/arm64.js/target/bench/v86" \
REPS=3 AUTHORITATIVE=1 \
SCORECARD_V2_OUTPUT="$PWD/target/bench/r127-r124-scorecard/formal" \
taskset -c 8-15 node tests/vs-v86/scorecard-v2.mjs
```

There is intentionally no `SCORECARD_V2_REWRITE_WASM` override: the exact
candidate is the live release build. The formal output directory did not
exist at freeze. The run starts all 117 samples anew, and no failed, partial,
or R126 sample may be replaced, trimmed, pooled, or reused. The authoritative
runner may exit nonzero solely because full 13/13 v86 parity is not yet met;
adjudication depends on the emitted report's validity, not that parity exit
status.

## Frozen incremental promotion rule

After the complete report exists, invoke the unchanged R126 adjudicator once.
It requires:

1. an authoritative, measurement-valid 117-trial report with no internal
   problem, exact three repetitions, CPUs 8--15, production policy, public
   cadence, active generated execution, exact artifacts/inputs, and the modern
   guest contract;
2. exact candidate `d017a10f...`, exact isolated legacy comparator, and exact
   pinned copy/v86 in every applicable trial;
3. all 13 rows at or above the scorecard's `0.95x` legacy parity floor;
4. no new copy/v86 parity loss beyond accepted R087's Boot and Compile;
5. every candidate rewrite/v86 ratio divided by R087's corresponding
   rewrite/v86 ratio at or above `0.99x`;
6. at least one of normalized Boot or Compile at or above `1.01x`; and
7. sealed R125 exact-candidate evidence at or above `0.99x` on every WANIX
   row and at least parity on unchanged `/shared/bench.py`.

A pass promotes the exact candidate as the incremental product even if Boot
and Compile remain short of copy/v86 parity. A failure restores exact control
source and rebuilds it. No post-freeze candidate, threshold, score, workload,
or evidence change is permitted.
