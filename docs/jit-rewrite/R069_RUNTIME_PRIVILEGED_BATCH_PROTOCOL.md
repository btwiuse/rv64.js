# R069 Runtime Privileged Batch Protocol

## Purpose

R068 admits one runtime test of a cold privileged generated tier. R069 asks
whether its offline coverage survives live V8 compilation, guest/compiler CPU
contention, publication delay, mapping churn, and dirty-code invalidation. This
is a default-off product candidate built into the same Wasm as its control.

## Frozen mechanism

When enabled, and only for non-User page-policy samples:

- enqueue a cold-tier candidate at 200,000 sampled page instructions;
- preserve ordinary User policy unchanged;
- form candidates in FIFO threshold-crossing order;
- use one candidate's exact SATP as the batch address space and include only
  queued pages whose recorded `(VA, PA)` mapping is proven in that same SATP;
- issue exactly eight distinct virtual pages per module (no partial timeout or
  phase-end flush);
- discover at most 64 leaders per page and 512 total;
- emit `MultiEntryState::Memory`, giving bounded member functions plus one
  in-module dispatcher;
- retain the accepted two-build in-flight limit, asynchronous host compile,
  per-page physical mapping checks, code-page marking, dirty invalidation,
  generation checks, exact entry publication, and interpreter fallback;
- if a queued page reaches the accepted 4,194,304 privileged threshold before
  a valid batch forms, let the unchanged one-page production path compile it
  rather than strand it.

There is no PC, symbol, binary, process, workload, scorecard-row, elapsed-time,
or hash selector. The setter is applied before boot and may not change batch
size, threshold, leader count, state representation, ordering, concurrency, or
publication cadence.

## Correctness gates

The forced-on candidate must pass:

1. all core and DBT unit tests plus Wasm smoke with the switch both off and on;
2. randomized scalar memory, Sv39/MPRV, atomic/A, floating-point, WFI, T2,
   page-policy, and precise memory-exit differential suites;
3. modern Linux 6.12.7 direct-SBI and OpenSBI boots;
4. an exact forced-on modern-Boot proof that at least one eight-page batch is
   issued, landed, and executes generated instructions;
5. dirty-page and mapping-generation tests demonstrating that one stale member
   prevents the shared module from executing until every page proof is valid.

Any correctness failure rejects the candidate before performance measurement.

## Frozen focused product gate

Use the existing alternating same-Wasm configuration A/B harness with three
fresh-process pairs, CPUs 8-15, exact modern artifacts, output fingerprints,
and host-probe validation. Control sets `jit_set_privileged_batch_t0(0)` and
treatment sets it to `1`. Measure Boot, Compile, and Python.

Advance only if:

- Boot paired point speedup is at least 1.10x and its paired-bootstrap lower
  bound is above 1.00;
- Compile and Python each retain at least 0.90x paired point speed and exact
  output;
- the treatment issues and lands eight-page batches, lowers cold compile-job
  count relative to an equivalent early one-page tier, and does not replace
  generated retirement with interpreter work;
- host-probe spread is at most 1.20x and no leg is retried or replaced.

Failure rejects and completely removes the candidate. Do not tune any frozen
mechanism from the observed rows.

## Promotion gate

Passing the focused gate admits the untouched full three-way 13-row scorecard
and the five-pair Chromium `/shared/bench.py` guard. Promotion requires no row
to regress by 10%, preserved 13/13 legacy wins, browser Python/SHA/shared-9P
non-inferiority, and a material move toward copy/v86 Boot/Compile parity. Raw
wall time remains authoritative; instruction-normalized rates are reported only
as anti-reward-hacking diagnostics.

## Result: rejected at the live lifecycle gate

Candidate Wasm `613d9627eec59144ec56c8b3a2d79651bec7472c350839ee42c6a96d572360f5`
(4,295,835 bytes) implemented the frozen mechanism. The synthetic switch-off/
switch-on gate proved one exact eight-page async batch could issue, land, pass
all-page mapping verification, and retire generated instructions. Core (32)
and DBT (53) unit tests, Wasm smoke, ordinary page-policy smoke/multipage
tests, formatting, scorecard selftests, and module construction passed.

The exact modern Linux forced-on proof then exposed a live lifecycle failure.
One standalone treatment boot reached readiness in 18,040 ms, issued 13 batch
modules covering 104 pages, spent 3,678 ms in host Wasm compilation, and
recorded only 296,979 generated instructions after a batch mapping
verification. The identical candidate Wasm with the switch off booted in
2,298 ms.

The preregistered three-pair Boot/Compile/Python A/B is retained at
`target/bench/r069-runtime-privileged-batch/ab/config-ab-2026-08-09T03-53-10-212Z.json`
(SHA-256 `0f44e2f14587704d49e4c88a3d1c7202bdafaaa75c30d07587707a64ceff4a65`).
It is invalid for performance promotion because eight of nine treatment legs
failed the existing 30-second JIT-settle requirement; no leg was retried or
replaced. All nine control legs completed and host-probe spread was 1.031x.
The one complete treatment Boot pair was 17,260 versus 2,279 ms, or 0.132x
speed. That leg issued and landed 13 batches/104 pages, emitted 18 total host
modules and 8.821 MB, spent 4,286 ms in host compilation, yet recorded only
257,844 verified batch-retired instructions.

R068's post-guest, future-entry-aware coverage and isolated compile samples did
not model live entry availability, compiler/guest contention, instantiation,
or the requirement to drain pending work. The runtime gate therefore rejects
the mechanism before the remaining forced-on differential matrix or any full
scorecard/browser run. Per the frozen rule, every runtime/export/test/harness
hook was removed without geometry or threshold tuning. The rebuilt production
module is byte-for-byte accepted R054
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`
(4,272,517 bytes).
