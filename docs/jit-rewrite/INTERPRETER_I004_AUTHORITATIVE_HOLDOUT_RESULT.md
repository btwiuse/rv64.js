# I004 Authoritative Sealed Holdout Result

Date: 2026-08-11 (America/Phoenix)  
Status: measurement valid; holdout transfer passed; overall goal open

## Frozen execution

Exact I004 ran the sealed holdout only after development-informed tuning was
closed, the strict correctness matrix passed, and the one-time stock-musl run
was retained and documented. This was the first and only execution of the
holdout population. The command frozen in advance was:

```sh
taskset -c 8-15 env \
  ARTIFACTS=/home/darren/src/arm64.js/target/bench \
  INTERPRETER_HOLDOUT_ROOT=/home/darren/src/jit/target/bench/interpreter-holdouts-v1 \
  INTERPRETER_HOLDOUT_OUTPUT=/home/darren/src/jit/target/bench/interpreter-final/holdout-v1-i004 \
  AUTHORITATIVE=1 REPS=3 \
  node tests/vs-v86/interpreter-holdout.mjs
```

The holdout runner forces interpreter execution for both workers and injects
only the frozen initramfs paths. There was no runtime-Wasm override, profiler,
diagnostic selector, sample extension, replacement, or rerun. Every sample was
retained in the balanced order selected by the harness. The immutable manifest
passed both before and after execution.

## Result

The report is authoritative and measurement-valid with an empty `problems`
array. Exact I004 wins all four unseen transfer workloads:

| Workload | I004 interpreter | copy/v86 interpreter | I004 / v86 |
| --- | ---: | ---: | ---: |
| BusyBox gzip | 4,566.1 ms | 8,675.0 ms | `1.8999x` |
| BusyBox sort | 7,850.5 ms | 14,319.4 ms | `1.8240x` |
| BusyBox SHA-256 | 18,146.0 ms | 31,158.6 ms | `1.7171x` |
| OpenSSL AES-256-CTR | 13,676.7 ms | 34,750.8 ms | `2.5409x` |

All 24 fresh-process trials are measurement-eligible and every trial proves
inactive JIT. No JIT activity field is nonzero, and all twelve v86 trials
record the explicit disabled-JIT proof. Every row/side pair has exactly
repetitions 0, 1, and 2; every trial has FIRST, PRIME, and STEADY; and every
phase checksum matches the sealed contract. There are no policy problems,
diagnostic trials, or worker stderr. Host-probe spread is `1.023339` and the
maximum scored within-side sample spread is `1.072049`, both below the frozen
`1.25` limits.

The exact rewrite identity is release Wasm
`7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`.
The comparator is pinned copy/v86 commit
`2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`, source tree
`ca8afd71c1444a56c20b1ab63939569329fd5369a1a75760b5dce53fc3ba00f8`,
and Wasm `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1`.
The holdout initramfs hashes are
`aa6cc4197f8aa685b46f9d8d5c2d6cdf10f6bb03fca0a26377a6825c1978b30a`
for RV64 and
`34cae1e01a1fa421b3af4eb76ba0a1209e3d167bd1b8a95f689c99a0259e2a24`
for x86.

The raw report is
`target/bench/interpreter-final/holdout-v1-i004/interpreter-holdout-2026-08-11T09-43-27.120Z.json`
(`dc71535f627e246ef83620c168b7f20857f4412f74460e151e104fb61213561a`).
Its generated Markdown is the adjacent `.md` file
(`03c37c895ed37ee65c58f6805cdaf43c5cfd053d8f3198a72cf52ab7ab3010cc`).

## Decision

The report's `goalMet=true` refers only to the four-row holdout contract. It is
strong transfer evidence for the retained architecture-general I004 changes,
but it cannot override the preregistered all-population acceptance rule. I004
already failed development String at `0.3186x` and fair stock String at
`0.3076x`. Therefore the overall pure-interpreter parity goal remains not met.

The holdout is now opened and permanently ineligible as a tuning input. No
product edit was made from its contents, and exact I004 remains unchanged.
