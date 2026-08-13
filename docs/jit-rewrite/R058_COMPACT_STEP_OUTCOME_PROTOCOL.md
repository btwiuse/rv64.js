# R058 compact interpreter step-outcome protocol

Date: 2026-08-08  
Status: rejected at the frozen engine-shape gate; production unchanged

## Question

Can the interpreter return its overwhelmingly common `continue` outcome in a
Wasm scalar instead of materializing Rust's
`Result<Option<StopReason>, Exception>` in linear memory after every
instruction?

Accepted R054's `Cpu::step` has a 24-byte structure-return ABI. On the normal
path the callee stores the outer `Ok` tag and inner `None` tag, then
`Cpu::run_until` reloads both fields immediately after the direct call. A
candidate would return a compact integer outcome (`continue`, `ecall`,
`break`, `wfi`, or `exception`) and write exception payload to CPU sidecar
fields only on the rare exception path. It must retain the separate decoder
call, exact per-instruction generated-entry check, interrupt cadence, trap
semantics, public `step` behavior, and all architectural state.

This is distinct from R023, which forced the entire large decoder into the run
loop and lost to code growth, and R056, which changed only the following
callback boundary. R058 tests only the step-result ABI across the existing
direct call.

## Frozen attribution and leverage

The production control is accepted R054 Wasm SHA-256
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
The post-R054 profile is
`target/bench/r055-post-r054-engine-profile/profile-analysis.json`.

Boot sampled 2,279.363 ms. `Cpu::step` self time was 1,137.088 ms and
`Cpu::run_until` self time was 298.626 ms. Their deliberately generous
combined upper-bound fraction is 62.9875%. Improving the whole Boot row 1.10x
therefore requires at least 1.1687x local speedup:

```text
f = (1137.088 + 298.626) / 2279.363 = 0.629875
s = f / (1 / 1.10 - (1 - f)) = 1.1687
```

The ABI cannot truly own every cycle in either function, so the frozen
engine-shape admission threshold is 1.20x median with a 1.15x bootstrap lower
bound. A result below that level closes the mechanism without production
work. Passing is only permission to build a real product prototype, not an
expected whole-row result.

## Frozen engine-shape upper bound

Emit deterministic `sret` and `compact` modules with identical exported
memory, state evolution, loop, direct step-call boundary, executed step body,
rare outcome mix, checksums, and exports.

- The `sret` step takes a result pointer and writes the accepted hot-path
  layout: outer `Ok` as an i64 zero at offset 0 and inner `None` as i32 -1 at
  offset 8. Its caller reloads both fields after the call. Stop and exception
  correctness paths populate deterministic payload fields.
- The `compact` step returns one i32. It writes sidecar exception kind/value
  only when returning the exception code. Its caller tests the returned scalar.
- Both step bodies contain the same state loads, integer transform, stores,
  outcome branch, and more than 500 wire bytes. The padding is identical and
  semantically inert. Node 26.5/V8 14.6 reports a 500-byte maximum Wasm
  inlining size; static inspection and V8's inlining trace must confirm that
  the driver retains the direct step call in both variants. The padding makes
  this a best-case ABI upper bound without accidentally benchmarking an
  inlined function.
- Timed calls use only the ordinary `continue` path. Separate untimed probes
  cover zero work, six instruction counts, clean stop, and exception payload,
  and compare the complete exported state bytes.

Run seven alternating paired fresh Node 26.5/V8 14.6 processes on affinity
8-15 under the repository benchmark lock. Each process records synchronous
compile and instantiation, a first call, four warm calls, and five steady
calls. Use 16,777,216 iterations per steady call. Do not change padding,
iterations, state transform, result layout, process count, order, or thresholds
after the first timed pair.

## Admission gates

A production prototype is admitted only if all of the following hold:

1. both modules regenerate byte-identically, validate, and preserve the exact
   frozen static shape;
2. static disassembly and the optimizing-tier inlining trace confirm one
   separate direct step call in each driver and no step inlining;
3. every correctness result and complete state snapshot is identical;
4. warm and steady results are stable, per-process warm spread is at most
   1.25x, and the global host CPU-probe spread is at most 1.25x;
5. paired compact/sret steady throughput is at least 1.20x and the fixed-seed
   bootstrap median lower bound is at least 1.15x; and
6. compact compile plus instantiation and paired cold delta are each below
   25 ms.

Failure rejects compact outcomes before a production edit. Do not weaken the
bound, remove the direct call, time only a favorable rare outcome, or add its
projection to a rejected mechanism.

## Product and promotion gates if admitted

Implement one architecture-wide internal compact outcome. Preserve the public
`Cpu::step -> Result<Option<StopReason>, Exception>` contract through a thin
adapter; `run`, `run_until`, `run_until_observed`, and `run_traced` must retain
exact behavior. Store and reconstruct the full exception variant and trap
value without loss. The rule contains no guest PC, opcode, binary, workload,
checksum, browser, engine, or scorecard selector.

First run the core/public/full-state, randomized differential, atomic/T2,
generated-module, and direct/OpenSBI modern Linux correctness gates. Then run
five alternating fresh-process same-Wasm Boot/Compile pairs against exact
R054. Require exact inputs, outputs, JIT-policy fingerprints, host/sample
stability, Boot paired median at least 1.10x with bootstrap lower bound at
least 1.00x, and Compile retention at least 0.90x. A failed leg is not replaced.

Only a passing product artifact proceeds to the untouched authoritative
13-row three-way scorecard and the five-pair Chrome `/shared/bench.py` guard.
Promotion requires no non-target regression of 10%, preserved browser
non-inferiority, 13/13 legacy wins, and improvement or parity on both remaining
v86 misses. Otherwise revert every production change and retain the frozen
negative evidence.

## Result and decision

The immutable report is
`target/bench/r058-compact-step-outcome-corpus.json`; static disassemblies and
optimizing-tier traces are under
`target/bench/r058-compact-step-outcome-shape/`. Both modules regenerate
byte-identically and validate. The sret module is 1,100 bytes
(`599eb15fb289...`); compact is 1,027 bytes (`1f6c85d82a84...`). All six
ordinary counts, clean stop, exception payload, complete memory snapshots,
first/warm/steady checksums, engine, and affinity match. Each step contains
640 nops and one direct driver call. V8 reports 777/751-byte step bodies,
compiles both step and driver with TurboFan, and explicitly declines the call
with `not enough inlining budget`.

Seven alternating fresh Node 26.5/V8 14.6 pairs on CPUs 8-15 measured:

- sret: 370.972 million non-inlined steps/second;
- compact: 177.587 million non-inlined steps/second;
- paired compact/sret throughput: 0.477x with bootstrap median interval
  `[0.469,0.650]`; and
- compact cold construction: 0.129 ms, with -0.008 ms paired delta.

Host-probe spread was only 1.015x. Warm stability also failed: six compact
legs clustered at about 94-96 ms per steady call while one ran at 30.8 ms;
no leg was discarded or replaced. The favorable isolated leg does not rescue
the median and is inconsistent with the required frozen non-inline shape.

Post-decision diagnostics do not alter the gate. Three runs with synchronous
tier-up kept compact at 94.4-94.6 ms versus sret at 43.4-48.2 ms. Explicit
Liftoff and forced-top-tier checks also retained the reversal. Ten additional
inlining-traced compact processes all reported `not enough inlining budget`
and clustered at 94.4-96.3 ms. Native print-code confirms a separate call in
both optimized drivers; the compact loop is smaller, but V8's observed scalar
return dependency is still slower than the store/load convention across that
call boundary. That last dependency explanation is an inference from the
code shape and timing, not an engine guarantee.

R058 therefore fails the independent 1.20x median, 1.15x lower-bound, and
warm-stability gates by wide margins. Do not refactor `Cpu::step`, build a
sidecar exception path, force inlining of the 10.9 KiB production decoder,
replace the outlier, or retune warmup. Production was never edited and remains
exact accepted R054 SHA `4160333352b18b...`.
