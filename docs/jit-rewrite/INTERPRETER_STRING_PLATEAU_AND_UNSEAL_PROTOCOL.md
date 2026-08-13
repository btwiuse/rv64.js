# Pure-Interpreter String Plateau and Final Unseal Protocol

Date frozen: 2026-08-11 (America/Phoenix)  
Status: final sealed sequence complete; overall goal not met

## Purpose

Exact I004 completed the authoritative development scorecard and still loses
String at `0.3186125x`. This protocol decides, before inspecting stock-musl or
any holdout, whether another development-informed implementation is admissible
and fixes the final audit sequence. It does not redefine parity and does not
turn a failed candidate into a pass.

## Exact remaining gap

The median I004 String trial takes `5,496.369485 ms` for `796,532,587`
reported RV64 guest instructions. The median copy/v86 trial takes
`1,751.212069 ms` for `96,490,537` reported x86 instructions modulo `2^32`.
Therefore:

- reported guest-work amplification is `8.25497x`;
- I004 already sustains `144.919` million reported instructions/s;
- copy/v86 sustains `55.099` million reported instructions/s, so I004's
  per-reported-instruction rate is already `2.630x` higher;
- equal wall time at the current RV instruction count would require `454.843`
  million RV instructions/s, another `3.13861x` over I004; and
- equivalently, a successor must remove `68.139%` of complete I004 String wall
  time while preserving every protected row.

The counters are not equivalent cross-ISA work units. The disparity is still
causal evidence: the development RV64 binary already uses its custom unaligned
eight-word `fastmem.c` copy, while x86 expresses page-bounded bulk movement in
the architecturally defined REP string family.

## Closed architecture-general mechanisms

The current clean series prospectively tested and retained the slice-local
executable-page capability (I001), out-of-line data slow paths (I003), and
out-of-line fetch refill (I004). It tested and rejected, without post-result
variants, slice-local data-page capabilities (I002), a local interrupt
countdown (I005), one-read instruction fetch (I006), complete M-family
outlining (I007), and exact-revalidated decoded blocks (I008).

I008 is especially relevant to the required scale. Its architecture-wide
decoded executor was correct, but three pairs measured String `0.6950x` and
Bitfield `0.6803x` versus I004. Its frozen rules end blocks at stores and exact
control transitions and compare current instruction bytes on every activation.
Changing capacity, length, validation, storage, hashing, or block boundary
after that result was explicitly prohibited. Allowing stores through a newly
versioned page cache or removing revalidation would be such a result-driven
cache/invalidation variant, not an independent candidate.

The multi-row I009/I010 diagnostics then found no new removable operation.
Their only common native bands above the frozen exposure threshold are the
already retained/closed fetch capability, instruction-length, loop, and
interrupt-countdown mechanisms. Historical architecture-wide decoded-page
handler dispatch tied the ordinary decoder (R045), exhaustive operation-pair
handlers regressed to `0.879x` throughput (R053), full-system const
specialization regressed Boot to `0.888x` (R083), and the complete interpreter
body census admitted no opcode-independent residual (R122).

No remaining source-audited operation has a credible `3.13861x` complete-row
projection. Combining unrelated optimistic leaves would not create one causal
mechanism. Recognizing the observed RV64 copy/compare/sort loops, selecting
guest PCs or instruction sequences, changing the RV-only benchmark library,
or enabling generated guest code would violate the frozen comparison.

## Tuning decision

Development-informed interpreter tuning is closed at exact I004. This is a
plateau decision, not a parity claim: the goal is still not met. No production
edit may be selected, shaped, or revised from the stock-musl or holdout
results below. A future project may introduce a genuinely new standardized ISA
operation or execution representation, but it must use newly sealed validation
inputs and cannot reinterpret this population as unseen.

The final-audit identities are:

- `crates/rv64-core/src/cpu.rs`:
  `d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`;
- `.cargo/config.toml`:
  `252a344de3e565c134906a497e33f88795eae1a29f1357bbfb05ffea911bc267`;
- release Wasm:
  `7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`;
- pinned copy/v86 commit:
  `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`.

## Frozen final sequence

Before opening a sealed input, rerun formatting, the full Rust workspace,
scorecard self-test, explicit interpreter bypass proof, full-state scalar
differential, system-memory differential, Sv39/MPRV differential, and the
dedicated M/A/FP differentials used to qualify I004. Any failure stops before
unsealing.

If those pass, execute in this order:

1. `stock-musl-v1`, all 13 rows, both interpreter sides, `AUTHORITATIVE=1`,
   `REPS=3`;
2. `tests/vs-v86/interpreter-holdout.mjs` exactly once with
   `AUTHORITATIVE=1`, `REPS=3`, and the frozen holdout initramfs identities.

Retain every eligible sample. Do not extend, replace, or rerun a valid leg or
population based on its value. A setup failure may be corrected only if it
occurs before either guest consumes the relevant sealed input; preserve the
failure and make at most one mechanical correction. Profiler, counter,
diagnostic, runtime-Wasm override, altered row list, or changed workload is
forbidden.

The development failure already prevents a parity result. Stock and holdout
execution is nevertheless required to finish the user's requested honest
comparison after tuning has closed. Any loss is reported as a loss and cannot
authorize another specialization.

## Pre-unseal correctness result

The frozen gate passed against exact release Wasm `7e7cee94eb58...` before
either sealed guest was executed:

- formatting passed;
- 33 `rv64-core` and 76 `rv64-system` library tests passed;
- the scorecard-v2 self-test passed;
- the explicit disabled-JIT bypass retired exactly 100,000 interpreted
  instructions with zero generated activity;
- 60 randomized full-state interpreter/JIT programs matched;
- user-mode integer memory, M, A, FP, and FP-fast-path differentials passed;
- fused system-memory hit/miss/refill/hashed, cross-page, MMIO, and
  compiled-code-store exits passed;
- Sv39/MPRV with hardware A/D updates passed; and
- system A, system FP, cold/warm WFI, and page-policy WFI tests passed.

No source or release artifact changed during the matrix. The stock-musl
population was still unexecuted at this checkpoint and was next in the frozen
sequence.

## Stock-musl execution record

Exact I004 subsequently completed the first and only sealed `stock-musl-v1`
execution, all 13 rows with three fresh-process repetitions per side. The
report is authoritative and measurement-valid; all 78 trials are eligible and
prove inactive JIT, its problem list is empty, maximum host-probe spread is
`1.013977`, and maximum scored within-side sample spread is `1.160265`.

I004 wins eleven rows, matches Mixed at `1.0179x`, and loses String at
`0.3076x`, or `3.25x` behind copy/v86. The fair stock input excludes the
development population's RV64-only `fastmem.c`; both sides record identical
workload-contract, transform, and implementation-source hashes. The manifest
passed before and after execution. The goal remains not met, and no product
edit may be based on this result.

The immutable result and complete audit are recorded in
[INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md).
At this checkpoint the one-time holdout remained sealed and was next in the
sequence fixed above.

## Holdout execution record

Exact I004 then completed the first and only sealed holdout execution. The
report is authoritative and measurement-valid; all 24 trials are eligible,
prove inactive JIT, and match every sealed phase checksum. Its problem list is
empty, host-probe spread is `1.023339`, and maximum scored within-side sample
spread is `1.072049`.

I004 wins all four unseen rows: BusyBox gzip at `1.8999x`, BusyBox sort at
`1.8240x`, BusyBox SHA-256 at `1.7171x`, and OpenSSL AES-256-CTR at `2.5409x`.
The holdout-local goal passes, providing clean transfer evidence. It does not
rescue the failed development and stock String gates, so the overall goal
remains not met. The opened holdout cannot select another product edit.

The immutable result and complete audit are recorded in
[INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md).
The final sequence frozen in this protocol is complete with exact I004 still
unchanged.
