# R124 WANIX Product Gate Freeze

Date: 2026-08-10  
Status: frozen before the first R124 WANIX timing sample

## Purpose

R124 has passed strict correctness, construction-debited native measurement,
and the dual-clock Chromium Boot guard.  This gate checks the exact candidate
in the public WANIX integration before the untouched three-way scorecard.  It
is a protected-workload gate, not another target-credit opportunity.

The Python and SHA-256 bodies are byte-identical to the existing
`/shared/bench.py` guard.  Shared 9P uses the prospectively qualified R094
32 MiB work size because the former 4 MiB phase was too short to distinguish
browser noise.  No guest PC, symbol, binary, result, or phase selects the JIT
mechanism.

## Immutable artifacts

- control archive `9d0bf45cdbcffcc06f68ac48a5e5692e548c5c9a4b310a236dc4bcbb8086a98d`,
  1,820,649 bytes, with Wasm `d9f686a9ce4f...`;
- candidate archive `76c7139ba38c2f658d981ebd24bbeeb0308e1acf0a5b593a9b0784d32f9127d8`,
  1,821,157 bytes, with Wasm `d017a10f00a8...`;
- loader `2cbb264f4dac...` and adapter `c40333c7355b...`, identical between
  archives;
- control page `ac3b9c63e67b...` and candidate page `f5086330565f...`, identical
  after replacing their one immutable archive URL;
- matched RV64 root `274a1e476646...`, v86 archive `7b2c1986bed2...`, and x86
  root `09735e00b02b...`; and
- Chrome `150.0.7871.186`, V8 `15.0.245.21`, revision
  `0fcdce5f4fdec8d442d7df760cb541f1ca6e446d`, on CPUs 8--15.

The two archives were independently assembled and reproduced byte-for-byte.
Both use the modern Linux 6.12.7 / Alpine 3.22.5 WANIX integration guest.  The
authoritative scorecard remains Linux 6.12.7 / Alpine 3.24.1.  Neither path
uses BBL, TinyEMU, or an old kernel/root.

## Frozen tools

- runner `82350bcc3b62...`;
- analyzer `c145b30828f0...`;
- analyzer mutation selftest `8f43174cb877...`;
- source/artifact selftest `08919d93122a...`; and
- unchanged WANIX automation harness `525b24160524...`.

Both selftests passed before freeze.  The mutation test proves that the
analyzer rejects an established confidence regression, incorrect P9 bytes,
inactive generated execution, and excessive within-browser spread.

## Frozen sampling and validity

Run seven alternating control/candidate pairs.  Every leg owns a fresh Chrome
process/profile, WANIX Worker, RV64 instance, and guest.  Run three synchronized
repetitions each of `python`, `sha256`, and `shared9p`; the per-browser value is
their median.  Shell time is retained as a separate protected row and is never
substituted for a phase value.

Every leg must preserve page/archive/member/root/browser/guest/policy
identities.  Every phase repetition must complete correctly, retire nonzero
generated code, close `generated + interpreted = instructions`, and prove the
exact production page policy.  Python and SHA-256 require at least 90%
generated coverage.  Every shared-9P sample must last at least 2 seconds,
write exactly 33,554,432 bytes, read at least that many bytes, and use 4,096
byte maximum transfers.  Each three-sample phase spread must be at most
1.25x.

For shell, Python, SHA-256, and shared 9P independently:

- paired-median control/candidate speedup must be at least `0.99x`; and
- the exact paired-bootstrap 95% interval upper bound must be at least `1.00x`,
  so the evidence does not establish regression.

Each benchmark phase additionally retains R094/R093's variance guard: its
interval lower bound must be at least `1 / 1.10`.  This bound is validity
protection, not permission for a 10% regression; the `0.99x` median rule still
decides protected performance.

## One authorized run

```sh
taskset -c 8-15 node tests/run-r124-wanix-browser-pairs.mjs \
  target/bench/r124-rvc-bank-hybrid/wanix-pairs

node tests/analyze-r124-wanix-browser-pairs.mjs \
  target/bench/r124-rvc-bank-hybrid/wanix-pairs \
  --output=target/bench/r124-rvc-bank-hybrid/wanix-gate.json
```

Do not replace, trim, extend, or rerun a leg after seeing results.  A pass
advances exact `d017a10f...` unchanged to the untouched 117-trial scorecard.
A failure archives R124 and restores exact `d9f686a9...`; it does not authorize
phase-specific tuning or a benchmark change.
