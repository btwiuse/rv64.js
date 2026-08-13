# R097 Tail-Chain Dispatch-Metadata Reuse Protocol

Date: 2026-08-10  
Status: closed at the frozen opportunity gate; no product implementation

## Question and distinct mechanism

Can a successful generated cross-module tail transfer reuse the generation and
table-index values it already loaded, instead of reading each dispatch-line
field a second time?

R096 showed that removing the adjacent diagnostic counter does not improve
Compile. R097 does not alter that counter. It targets the architectural
transfer proof itself: production currently loads `gen` for equality and again
for the unverified sentinel check, then loads `idx` for sign validation and
again for the transfer. The candidate uses `local.tee` on each first load and
reuses two i32 locals that are dead after the structured member dispatcher. It
therefore adds no locals and changes four metadata loads to two while retaining
the complete PC, live-generation, non-sentinel, non-negative-index, progress,
fuel, tag-mask, and `return_call` sequence.

The rule is architecture-wide. It may not depend on guest PC, opcode, symbol,
binary, workload, benchmark phase, browser, or compiler identity. There is no
code-size gate; sizes and cold construction are measured rather than proxied.

## Frozen opportunity screen

Generate one deterministic Wasm corpus containing both exact proof shapes with
identical function signatures and local declarations. A separate Wasm instance
mutates the shared memory between probes, matching the cross-instance aliasing
boundary and preventing invalid load hoisting across iterations. Test exact hit,
PC miss, generation miss, unverified generation, negative index, and tagged
region index results.

Run seven alternating fresh-process pairs in both Liftoff-only and ordinary
Node/V8 modes. Advance one product prototype only when:

- all outputs, barrier counts, bytes, validation, and regeneration hashes are
  exact;
- Liftoff paired median is positive and its 95% median lower bound is at least
  1.0; and
- ordinary tiered V8 has a positive paired median and a lower bound of at least
  0.98.

No fixed percentage reward floor applies because the source change removes two
loads with no new state, branch, or policy. The corpus projection is diagnostic
only; real Linux wall time decides promotion. If either engine direction fails,
stop without a local-layout, packed-field, sentinel, or engine-specific variant.

## Product gate if admitted

Add one emission-time diagnostic switch solely for a same-main causal A/B,
include it in the emission configuration signature, and reuse the existing
selector/hop locals. First prove exact operator differences and run the complete
focused correctness matrix plus modern direct/OpenSBI Linux.

Then run seven alternating fresh Boot/Compile/Python pairs using one main Wasm,
modern Linux 6.12.7 / Alpine 3.24.1, CPUs 8--15, and public one-slice cadence.
Advance when Compile's paired median is positive with lower bound at least 1.0,
Boot/Python elapsed ratios are at most 1.02, and identities, fingerprints,
generated execution, tail transfers, fuel, retirement, and policy all pass.

If causal timing passes, make the lowering unconditional, compare the immutable
product with R085, run Chrome Boot and the qualified R094 WANIX Python/SHA/32
MiB shared-9P guards, then run the untouched 117-trial scorecard. Promotion may
reduce an open-row gap cumulatively but does not redefine completion: parity
with copy/v86 on both Boot and Compile remains the objective.

On the first failed gate, archive exact evidence, remove all product switches
and candidate lowering, restore R085 executable behavior, and try no variant.

## Result

The 314-byte corpus and 72-byte cross-instance barrier regenerate
byte-identically, validate, and match all hit, PC-miss, generation-miss,
unverified, negative-index, tagged-index, checksum, and barrier-count results.
The exact report is `target/bench/r097-tail-chain-metadata-opportunity/
opportunity.json` at `b05217f9088d...`.

Seven fresh pairs show a real engine split:

- Liftoff-only: cached/reload paired speedup 1.649x with interval
  `[1.525,1.743]`, saving 2.864 ns per modeled hop and projecting 2.48% of the
  Compile row if every measured transfer remained baseline-compiled; and
- ordinary tiered V8: 0.998x with interval `[0.992,1.005]`, with no measurable
  saved time per hop.

The ordinary-V8 paired median is not positive, so the prospectively frozen gate
rejects the product prototype. The likely explanation is that TurboFan already
common-subexpression-eliminates the duplicate same-line loads, while Liftoff
does not. Do not implement an engine/tier selector, force functions to remain in
Liftoff, change warmup, or tune local/packed metadata after seeing this result.
No product source, runtime ABI, generated lowering, or scorecard harness was
changed; R085 executable behavior remained active throughout.
