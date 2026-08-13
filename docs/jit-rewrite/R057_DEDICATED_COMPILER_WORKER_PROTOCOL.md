# R057 dedicated compiler Worker protocol

Date: 2026-08-08  
Status: rejected at the frozen-corpus opportunity gate; production unchanged

## Question

Does moving the existing asynchronous generated-Wasm builds into a dedicated
two-Worker compiler pool reduce interference with foreground Wasm enough to
justify a product prototype, without delaying publication or harming the warm
Compile workload?

This experiment does not change what guest code is selected, translated, or
emitted. It does not change the two-build in-flight limit. The control is the
current `WebAssembly.compile(bytes)` path. The candidate transfers the same
owned bytes to one of two compiler Workers, constructs a
`WebAssembly.Module` there, and structured-clones the completed module back.
Instantiation, table publication, ticket generation checks, and invalidation
would remain in the emulator realm in a later product prototype.

R046 makes this question material: cold Boot grew from 10 modules and about
0.42 seconds of summed asynchronous compile latency to 51-52 modules and
6.77-6.94 seconds, regressing Boot 45.7%. R057 is not a retry of that rejected
admission policy. It tests whether the accepted ten-module compiler activity
itself contends with foreground Wasm under the current policy.

## Frozen inputs

The production control is accepted R054 Wasm SHA-256
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
The module-capture hook is downstream of lifecycle timing and capture runs are
diagnostic-only.

Two exact current module streams are frozen:

- Boot manifest
  `target/bench/r057-current-boot-modules/manifest.json`, SHA-256
  `ef848ebf739b57346d49c7fa229f93d6d725276ba4d9ccdaffd7c7859ce88e3a`.
  Select tickets 1 through 10 in ticket order: 10 unique modules and
  3,974,380 bytes. This matches the number and bytes recorded before readiness
  in that capture. Later settlement modules are not inserted into the timed
  Boot corpus.
- Compile manifest
  `target/bench/r057-current-compile-modules/manifest.json`, SHA-256
  `e26c628443828ab16e19e7dff95847569cfb05d9b251261561476871f15c4fcf`.
  Select every module attributed to STEADY in ticket order: 15 unique modules
  and 5,745,513 bytes, including the naturally occurring 2,946,434-byte
  module. FIRST and PRIME modules are not substituted.

The foreground engine load is the deterministic R056 inline module
`target/jit-monomorphic-reentry-corpus/inline.wasm`, SHA-256
`9b6263deb230...`. It is warmed before measurement and executes fixed work:
512 calls of 4,194,304 iterations for Boot and 256 such calls for Compile.
Each call result contributes to a checksum. The same exact calls and module
streams execute in both modes.

## Frozen measurement

Run seven alternating paired fresh Node 26.5/V8 14.6 processes under host CPU
affinity 8-15 and the repository benchmark lock. Each process creates the
compiler service before timing, warms the foreground module, then runs Boot
followed by Compile. The control maintains two concurrent
`WebAssembly.compile` promises. The candidate maintains two compiler Workers,
one synchronous module construction per Worker. It transfers input
`ArrayBuffer`s and clones returned `WebAssembly.Module`s; port transfer and
clone latency are inside the timed compiler-ready interval.

For each corpus, launch at most two builds, execute the fixed foreground calls
with one event-loop yield between calls, and launch the next build only when a
prior build completes. Record foreground Wasm call time, foreground wall time,
last-module-ready time, total makespan, per-module latency, Worker-local build
time, module descriptors, checksums, and all raw samples. File I/O, compiler
service startup, and foreground warmup are outside the concurrent interval but
their costs remain reported.

No pool-size, chunk-size, module subset, queue order, construction API, or
threshold is changed after the first timed pair.

## Admission gates

A production prototype is admitted only if all of the following hold:

1. every raw byte hash matches its manifest, every module validates, both
   modes return all modules, and structured-cloned module import/export
   descriptors match exactly;
2. foreground checksums are identical across every sample and warmup spread
   within each process is at most 1.25x;
3. the global host CPU-probe spread is at most the scorecard's 1.25x limit;
4. on Boot, paired control/candidate foreground-call and foreground-wall
   medians are each at least 1.10x, with bootstrap median lower bounds at least
   1.00x;
5. on Boot, candidate last-module-ready time is no more than 1.10x control;
6. on Compile, foreground-call time, foreground-wall time, last-module-ready
   time, and total makespan each retain at least 0.90x control performance;
7. two-Worker startup plus clone feature check has median cost below 100 ms.

Failure rejects the service before any production loader, worker, runtime, or
Wasm change. A strong Worker-local compile number alone is insufficient.

## Product and promotion gates if admitted

Feature-test module cloning and add a portable compiler-service callback; do
not require `SharedArrayBuffer`. Bytes are transferred, completed modules are
instantiated only in the emulator realm, and existing ticket/generation checks
remain authoritative. Unsupported Worker/module-clone environments fall back
to the unchanged `WebAssembly.compile` path. The rule contains no guest PC,
opcode, binary, workload, checksum, browser, or engine selector.

Run lifecycle cancellation, stale-ticket, failure-fallback, table-growth,
direct/OpenSBI Linux, and full differential tests. Then run five alternating
same-Wasm Boot/Compile pairs. Require exact inputs and outputs, host/sample
stability, Boot speedup at least 1.10x with lower bound at least 1.00x, and
Compile at least 0.90x. A failed leg is not replaced.

Only an admitted product artifact proceeds to the complete 13-row
authoritative three-way scorecard and five-pair Chrome `/shared/bench.py`
guard. Promotion requires no non-target regression of 10%, no browser guard
regression, and preserved 13/13 legacy wins. Worker startup, publication
latency, and fallback use must remain visible in reports.

## Result and decision

The immutable report is
`target/bench/r057-compiler-worker-corpus.json`. Seven alternating paired
fresh Node 26.5/V8 14.6 processes ran under affinity 8-15. All 25 exact module
hashes validated, both modes returned every module, structured-cloned module
descriptors matched in the Worker and emulator realms, checksums were stable,
and host-probe spread was 1.071x.

Boot foreground Wasm did not improve: paired control/Worker call time was
0.998x with bootstrap median interval `[0.997,1.008]`; foreground wall time
was 1.002x `[1.001,1.013]`. The ten-module stream became ready only 0.489x as
fast as control `[0.465,0.632]`, meaning the Worker route took about twice as
long. Worker-local construction was faster in isolation (about 14.5 ms summed
versus 25.7 ms for current-realm promises), but transfer, message delivery,
and module clone raised summed end-to-end latency to about 50.1 ms and erased
that advantage.

Compile's larger stream did clone successfully and its ready-time ratio was
1.073x `[1.071,1.119]`; foreground call/wall ratios were 1.005x/1.008x.
That is a small mechanism-specific win, not enough to affect the foreground
row, and R048 already showed that eliminating most oversized frontend work
does not improve Compile wall time.

Four of fourteen premeasurement warmup sequences also exceeded the frozen
1.25x spread gate while V8 completed foreground tiering. The later fixed-work
ratios were stable, but the protocol does not discard or replace those legs.
More importantly, the independent Boot foreground and ready-time gates fail
decisively. Do not implement the product service, alter warmup, batch messages,
share bytes, change pool size, or select only large modules. Retain the capture
fix, frozen corpus, harness, protocol, and report as negative evidence. The
accepted production Wasm remains exact R054 SHA `4160333352b18b...`.
