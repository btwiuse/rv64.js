# R125 WANIX Product Gate Freeze

Date: 2026-08-10  
Status: frozen before the first formal sample

## Scope

This is a from-zero replacement for the invalid R124 WANIX run. It tests the
unchanged R124 fixed-register-bank candidate against its exact control after a
common, independently qualified WANIX transport correction. It cannot reuse,
replace, or pool any R124 formal or diagnostic sample.

The correction and its twelve fresh-browser qualification runs are documented
in `R125_WANIX_SINGLEFLIGHT_QUALIFICATION.md`, SHA-256
`5a541cf6874db6d7917578d536154e765311f076ab4238587f411d80a971be62`.

## Immutable artifacts

| Item | Bytes | SHA-256 |
| --- | ---: | --- |
| control page | 12,042 | `0fa38205e9c6306564f9b1f0d438816f24caef5bd113d8a8a86a0fddc4ba08f0` |
| candidate page | 12,044 | `2da494b4c379e1deebca322cb7526a11a7fa33873836a43811456767ef0dc8c3` |
| control archive | 1,820,749 | `5fe22762302f6414c4b9c3aa6f85010996cf7bce8eb7ff4da3eb37099059a75e` |
| candidate archive | 1,820,896 | `88274081a25ee03a0d3e926bc2a0d9a90c53e00cce665c5a98a51b95c58720a4` |

Both archives contain the exact common loader `2cbb264f...` and exact common
single-flight WANIX adapter `bba6baaf...`. Their only differing member is:

- control core `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`;
- candidate core `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59`.

Independent archive builds exactly reproduce `5fe22762...` and `88274081...`.
The pages normalize byte for byte after replacing their one archive URL. The
deployed localhost copies were hash-verified before freeze.

## Immutable tools

| Tool | SHA-256 |
| --- | --- |
| runner `run-r125-wanix-browser-pairs.mjs` | `d8e48565a7f17db1587a7df34cc40bfb4098dd770c073b798239644cd465fe5d` |
| analyzer `analyze-r125-wanix-browser-pairs.mjs` | `3d5ff23aa585e3423e62055575fcf50d4637fa3637cd11ee531bbe2b949b258b` |
| analyzer selftest | `e6cff6b359c65de1e796665b150fecb2a580988653a5a0184e827e0c7b0cf58f` |
| source/artifact selftest | `c2dbbb23af8ffd468f653a980f850fbf5b336b9d3fe282d78ece3ee6fbd5a37e` |
| unchanged browser harness | `525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545` |

All syntax, analyzer decision, source/artifact, queue, generic concurrency, and
adapter Go tests pass before sampling.

## Immutable execution protocol

- exactly seven alternating pairs, starting control/candidate;
- exactly three synchronized repetitions in every fresh browser leg;
- one fresh Chrome process, profile, WANIX Worker, RV64 instance, and Linux
  guest per leg;
- Chrome 150.0.7871.186 / V8 15.0.245.21;
- CPUs 8--15 for the complete runner and every child;
- Linux 6.12.7, Alpine 3.22.5, Python 3.12.13, riscv64;
- unchanged phase order `python`, `sha256`, `shared9p`;
- no JIT override and the exact production page policy;
- exact output, retirement accounting, generated activity, artifact, guest,
  browser, chronology, and non-overlap proofs;
- exactly 33,554,432 P9 write bytes, at least 33,554,432 read bytes, and
  maximum 4,096-byte read/write requests for every shared-9P repetition.

The result directory
`target/bench/r125-wanix-singleflight/formal-pairs` did not exist at freeze.

## Immutable decision

For shell, Python, SHA-256, and shared 9P, compute the median of three samples
inside each browser, seven paired control/candidate speedups, their paired
median, and the exact paired-bootstrap 95% interval.

The gate passes only when all integrity checks pass and, for every row:

1. paired-median speedup is at least `0.99x`;
2. the interval upper bound is at least `1.00x`, so it does not establish a
   regression;
3. each timed phase has interval lower bound at least `1/1.10`;
4. every within-browser three-sample spread is at most `1.25x`; and
5. Python and SHA-256 generated coverage is at least 90%.

Every shared-9P sample must also last at least two seconds and satisfy the
exact byte/transfer-size proofs. A timeout, crash, protocol violation, or
incorrect sample invalidates the run; it is not replaced.

This gate supplies no new native or Chromium credit. A pass advances the exact
R124 candidate to the untouched 117-trial modern three-way scorecard. A valid
protected-row failure rejects it. An invalid run stops for diagnosis. No
threshold, artifact, tool, sample count, or order changes after this freeze.
