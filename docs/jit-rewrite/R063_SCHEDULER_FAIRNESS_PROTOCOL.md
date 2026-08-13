# R063 Scheduler-Fairness Protocol

## Question

Does the scorecard delay rewrite JIT publication by driving several large
guest slices without returning to the JavaScript event loop, and does matching
the public browser scheduler materially improve broad performance?

This is a harness/runtime-lifecycle question, not a guest-work or opcode
specialization. The exact pinned copy/v86 runner calls `cpu.main_loop()` once
and schedules the next call with `setImmediate`. The public rewrite scheduler
likewise runs one 2,000,000-instruction slice and schedules its next call
through `hostYield`. The scorecard instead calls rewrite
`runVirtSystem(2_000_000n)` synchronously and historically yields only on pump
iterations 1, 5, 9, and so on. An async `WebAssembly.instantiate` completion can
therefore remain unpublished while roughly eight million additional guest
instructions execute.

## Frozen control and intervention

The control is accepted R054 Wasm SHA-256
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
No Rust, generated module, guest, policy, threshold, or workload byte changes.

The sole candidate switch is
`SCORECARD_V2_YIELD_EVERY_PUMP=1`. It makes the common scorecard wait loop
return to the event loop after every pump. The switch is applied to every
engine and makes a result diagnostic/ineligible. For copy/v86 the pump is a
no-op because its own event-driven runner already owns CPU scheduling; for both
RV64 engines this makes the manual driver cadence one guest slice per event-loop
turn.

## Predeclared experiment

1. Verify source evidence in the exact pinned v86 checkout and current public
   rewrite scheduler.
2. Run three alternating fresh-process pairs on Matched Boot, Compile, and
   Python using the exact same Wasm in both legs.
3. Preserve input hashes, guest output/checksums, retired guest work, generated
   execution proof, raw samples, host probes, and sample spreads.
4. If the screen shows at least a 1.10x paired median improvement on Boot or
   Compile, with the other two rows no worse than 0.90x, rerun five pairs.
5. Only then make every-pump yielding the harness default and run the complete
   three-way 13-row scorecard. A harness promotion changes the comparison
   baseline; it is not credited as a JIT code-generation optimization.

## Anti-reward-hacking constraints

- The intervention is scheduler-wide: no PC, opcode, symbol, phase, benchmark,
  browser, or guest-ISA exception.
- Timed serial markers and guest commands are unchanged.
- Guest instruction counts and output fingerprints must remain identical per
  row. Any change is a correctness/work-equivalence failure.
- The old cadence remains available only long enough to produce the immutable
  paired diagnostic. If promoted, the new cadence applies to all future
  scorecards, including regressions.
- `/shared/bench.py` remains a separate browser guard. A scorecard improvement
  cannot advance the production objective if that guard regresses by 10% or
  more.

## Result

The three-pair screen is valid at
`target/bench/r063-yield-every-pump-screen-valid/config-ab-2026-08-09T02-01-24-637Z.json`.
Host-probe spread was 1.019 and output/input correctness fingerprints matched.
Paired candidate speedups were:

- Matched Boot: 1.008x, interval `[0.946, 1.016]`;
- Compile STEADY: 1.060x, interval `[1.058, 1.076]`;
- Python STEADY: 1.246x, interval `[1.156, 1.254]`.

The cadence materially changes when code becomes available: Python
interpreter retirement fell from roughly 56.5-57.5 million to 29.8-33.7
million instructions and landed page functions rose from 26 to 62-65. This is
direct evidence that publication delay exists. The predeclared advancement
gate nevertheless required a 1.10x Boot or Compile improvement; neither row
passed. R063 is therefore not promoted and the accepted scorecard baseline is
unchanged. The evidence admits a distinct product-level experiment (R064):
return the runtime slice promptly when a page module is actually pending,
rather than changing polling cadence unconditionally.
