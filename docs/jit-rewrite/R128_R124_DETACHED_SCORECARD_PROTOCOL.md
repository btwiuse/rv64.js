# R128 Exact-R124 Detached Scorecard Protocol

Date: 2026-08-10  
Status: frozen before the first R128 process

## Replacement scope

R128 replaces externally terminated R127 from zero. R127 produced no formal
report and supplies no measurement, median, process, or elapsed-time value.
R128 retains the exact R127 candidate, source, loader, modern guests, isolated
legacy comparator, pinned copy/v86 build and BIOS, scorecard tools, accepted
R087 baseline, sealed R125 WANIX result, 117-process cadence, affinity,
timeouts, and frozen incremental promotion rule.

The only operational change is process ownership: a frozen launcher starts the
scorecard in a new session with standard input detached and raw stdout/stderr
persisted. This prevents an interactive agent/tool handoff from delivering a
hangup to the benchmark parent. It does not alter benchmark code or data.

## Exact candidate

| Item | SHA-256 |
| --- | --- |
| live release Wasm | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` |
| live DBT API source | `e4be5025e892f417fdde56dfc0c4c5ead632b474de9ea849558b9ed7ffbed795` |
| live DBT Wasm emitter source | `f60b8ae438cf1fcec7dc22215e6e3da5caf1755a7d5c214ec09b5928eb54d96e` |
| unchanged Wasm runtime source | `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339` |
| public loader | `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385` |
| deterministic candidate source archive | `6497fe464b64113525620f0f1ae4ac767a2137fd09b7d5cb843ba893f5627829` |

The release Wasm is byte-identical to the frozen R124 artifact and independent
R126 reproduction (4,281,786 bytes). No candidate edit is permitted before
adjudication.

## Exact tools and evidence

| Item | SHA-256 |
| --- | --- |
| R127 invalidation record | `6b2cc6b9ad6ad09c2d4462a50423eaff24f4e975a14cc04479621febbd2522e8` |
| one-percent policy | `cbceb4843ff1542b954763e6537673e834104907836c76eeebfa977f20bea896` |
| scorecard runner | `a3f5922ea981b4193e16c4b32f0d9dc229bfdbb036072c84ce6e6ffde4d2f399` |
| scorecard selftest | `17d69d8dbe250d5f3fa725f52548794dd97f375acee9e468a825840e8874eccc` |
| scorecard worker | `346c240378c8763053b7cddd5a093476eb0661ccddb5b8ef47b5f1698c77a175` |
| scorecard library | `377f32f4a5fbd467f4b262ecb8472febf8e3960a12ff40496c91c59009c29186` |
| cadence helper | `ed7b65cab81fa963804dfd9da6108417e27e7ed3f53b640cc06cdfcf86f3eaaa` |
| benchmark math | `a8bdea3fb105ed63dc69a062077ff86a1bfdd82567df16f4c42925e0685c02b8` |
| frozen adjudicator | `836509255e4d1cfd4ad3602ff230d266ec0b885e65f0800da104fe8e7a1ab9c7` |
| adjudicator selftest | `88c052ea574ff4160046fceda43c4c4394879c6cece8e18e9a762ac71cd4369b` |
| detached launcher | `c620e2a2f9f63fa6dbbf3974fc6da4bab7c71451d9ee13dda1b6f5a93891c6c5` |
| accepted R087 report | `1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7` |
| sealed R125 report | `9e051d1fd4b23c7b440134778e54a500c5fe6eb0cdc058e0d8bf3db359491868` |
| invalid R126 report | `ac096fecb8686eb76288af6409adeaf6b86808efc58310a52847197b90ee0d1f` |

The launcher verifies the frozen manifest and both selftests before executing
the scorecard. It rejects an existing formal directory or concurrent
scorecard, records its OS PID, uses Node v26.5.0 and CPUs 8--15, and records
raw logs plus the runner exit status.

## Exact guest and comparator inputs

| Item | SHA-256 |
| --- | --- |
| Linux 6.12.7 RV64 Image | `57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2` |
| Alpine 3.24.1 riscv64 initramfs | `cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808` |
| Alpine 3.24.1 i686 initramfs | `f626064d8ca2a2031f00b3e6389ba2a65866535df0a143ad0b02ab92a7f70be5` |
| matched Linux i686 kernel | `8854efec5534d0badf98aa34f7e7cb37fe3626d4d32d3a6909ca7fad8047acb5` |
| legacy loader | `54df79c8b35cf50bcee34c4af02d7eb02b09e0439b717ee75bb830e733595b12` |
| legacy Wasm | `274aaab5799386956a8c509434961c4a426066f8fc9f520e994c210affd61709` |
| copy/v86 main | `85c609ad27a3ca4e5327280f4264b9ccd35d833c0e3354f5ded08d15e450ef63` |
| copy/v86 Wasm | `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1` |
| SeaBIOS | `73e3f359102e3a9982c35fce98eb7cd08f18303ac7f1ba6ebfbe6cdc1c244d98` |
| VGA BIOS | `a4bc0d80cc3ca028c73dafa8fee396b8d054ce87ebd8abfbd31b06b437607880` |

Both RV64 sides execute Linux 6.12.7 / Alpine 3.24.1 riscv64 and copy/v86
executes the matched i686 build. No TinyEMU kernel, BBL, or unrelated root is
used.

## Sole launch

At freeze, no scorecard process, formal directory, runner log, PID record, or
exit record exists. Launch exactly once:

```sh
nohup setsid /bin/bash \
  target/bench/r128-r124-scorecard/launch-detached.sh \
  >/dev/null 2>&1 </dev/null &
```

The launcher invokes the same formal command frozen for R127, without a
`SCORECARD_V2_REWRITE_WASM` override. All 117 processes are new. Do not replace,
trim, pool, or selectively rerun any process. A benchmark invalidity stops for
diagnosis; the runner's expected nonzero full-parity exit does not by itself
invalidate a complete report.

## Frozen incremental promotion rule

After one complete measurement-valid report exists, invoke the unchanged
adjudicator exactly once. It requires all of the following:

1. exact authoritative 117-trial modern contract, three repetitions,
   production policy, public cadence, active generated execution, CPUs 8--15,
   correct outputs, exact artifacts, and no internal problem;
2. all 13 rewrite rows at or above `0.95x` legacy;
3. no new copy/v86 loss beyond R087's Boot and Compile;
4. every current rewrite/v86 ratio normalized to R087 at or above `0.99x`;
5. normalized Boot or Compile at or above `1.01x`; and
6. sealed R125 exact-candidate evidence at or above `0.99x` on all WANIX rows
   and at least parity on unchanged `/shared/bench.py`.

A pass promotes exact `d017a10f...` as the incremental product. A failure
restores exact control source and rebuilds it. No threshold, workload,
candidate, score, or evidence change is permitted after this freeze.
