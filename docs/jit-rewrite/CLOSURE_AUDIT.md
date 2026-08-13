# JIT rewrite performance closure audit

Date: 2026-08-08  
Accepted artifact: `4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`

## Purpose

This document maps the exact R050 Compile cost decomposition to every causal
experiment already completed. It prevents a new diagnostic label from
reopening an old mechanism, as happened briefly in R051. A family being closed
means that its tested implementation shape cannot be repeated without new
structural evidence; it does not claim that every imaginable implementation is
impossible.

The authoritative modern Linux 6.12.7 / Alpine 3.24.1 scorecard remains 11/13
against copy/v86 and 13/13 at parity or better against legacy. The open rows
are Boot 2,260.5/1,525.8 ms and Compile 1,060.9/718.5 ms. The exact QEMU
workload count is 290,675,966 RV64 versus 231,421,773 i386 guest instructions,
or 1.256x. Consequently, the current 1.477x raw Compile gap contains a 1.176x
per-guest-instruction cost gap after accounting for ISA instruction count;
parity requires about a further 15% reduction in rewrite cost per guest
instruction if the workload counts remain fixed.

## R050 component map

| STEADY component | CPU samples | Exact dynamic evidence | Mechanisms already tested | Closure |
| --- | ---: | --- | --- | --- |
| Generated-module subtree | 43.86% | 300.88M generated instructions; 2.45 sampled ns/instruction | Structured scheduling/PC/fuel thinning (R013-R014), generated-memory variants (R008-R019, R037-R038, R043, R049, R052), SSA local/stack lowering (R011, R039), module/PIC/frontend geometry (R003, R006, R048) | Local forms and repacking of the existing translation proof are closed. Whole-function optimized native-code cost has not been completely decomposed; only a mechanism that removes a broad operation remains admissible. |
| Policy-sampled interpreter | 19.22% | R029: warming 10.141M, pending 7.626M, queued 0.846M interpreted instructions | Heat and privileged multipliers (R005, R022, R036), synchronous tiny tier (R035), asynchronous compact privileged tier (R046), precompiled exact triples (R047) | Threshold, queue, ordinary per-page compile, decoded-handler, and exact-triple forms are closed. A future cold tier must avoid per-page runtime Wasm compilation and prove cross-workload transfer before implementation. |
| Final-outcome interpreter | 14.98% | R029: attempted-not-installed 3.916M, new entries 1.812M, installed-missing 0.135M, plus generated side exits | Sparse scheduling/coverage (R013-R014), interpreted-work entry ranking (R032-R034), exact re-entry probe removal (R040-R041), R051 independent site corroboration | Entry ordering, leader-cap effects, and blind probe thinning are closed. More coverage is not itself a speedup. |
| Scheduler plus cache hashing | 11.86% | About 361 sampled ns per outer dispatch; 545-618 guest instructions per generated call | Direct/structured dispatch and tail chaining are already production; R013-R014 removed about 10% of dispatches for only 3% wall benefit; R040 changed interpreter probe cadence and regressed STEADY | No local scheduler edit has enough remaining ceiling: even impossible elimination is only 11.86%. Reopen only if a new mechanism removes scheduler work together with another dominant component. |
| Translation / issue | 5.46% | 6.39-7.65 sampled ms per attempt, but only 5-6% of total CPU | Physical/PIC reuse (R003, R006), policy/registration volume (R004-R005), oversized-function/frontend removal (R048) | Closed as a standalone target: perfect elimination cannot pass the 10% whole-row gate. |

## Generated-execution subclosures

R015 measured the dynamic structured instruction mix: 42.94% ALU/other,
24.26% loads, 19.96% stores/AMO, and 12.84% control. The following boundaries
must be respected:

- Per-access, invocation, persistent, member, whole-region, stable-component,
  and carried two-page translation proofs are covered by R008-R009, R012,
  R016-R017, R019, and R049. R016's reproducible 8.3% STEADY improvement was
  the strongest form, but it missed the fixed gate; broader carried forms
  subsequently regressed 12-41%.
- Interleaved SIMD TLB lookup improved one screen 6.4% (R037) but missed the
  gate. The shared scalar packed-proof replacement catastrophically destroyed
  hit shape (R038). R052 then preserved separate load/store rows and exact hit
  shape, but native VPN/context reconstruction made the frozen uncached path
  43.9% slower and the cached path 11.5% slower. Layout and compression of the
  current per-access proof are closed; fewer table loads is not a sufficient
  opportunity metric.
- R011 showed that load/store immediate rematerialization is optimized away by
  V8. R039 removed most apparent SSA/local traffic and emitted bytes but
  regressed optimized STEADY, so Wasm-source local count is not a valid proxy.
- R043 found only 0.0115% exact safe redundant generated loads. Conventional
  local memory-SSA forwarding has no useful opportunity in these members.
- R013-R014 bound PC/fuel/dispatch thinning at about 3% causal wall benefit.
  The current structured CFG and tail-chain path already amortizes outer calls.
- R048 removed 57-62% of steady generated bytes in its final form and tied.
  Wasm byte count and late TurboFan completion are not scored throughput
  proxies.

## Boot subclosures

Boot remains predominantly runtime/T0 execution. R021 removed the complete
compressed-instruction expansion pass and improved Boot 11.6%. R054 is the
latest promoted change: exact fused JIT-TLB capability hits bypass the standard
TLB and physical-RAM dispatch for scalar interpreter memory, improving the
untouched Boot row another 13.35%. Decoder inlining (R023), instruction/block predecode caches
(R025-R027), cold-family outlining (R042), decoded page execution (R045), heat
thresholds (R022/R036), compact asynchronous page Wasm (R046), selected exact
triples (R047), and exhaustive exact pairs (R053) are closed in their tested
forms. R046 is especially constraining: adding 41-42 small cold
modules raised asynchronous compile latency from about 0.4 s to 6.8 s and
regressed Boot 45.7%.

R055 refreshed the post-R054 profile and tested instruction-fetch pointer
fusion only after a deterministic mixed-RVC corpus showed 1.989x local
throughput. The real five-pair same-Wasm Linux result was 0.962x Boot
`[0.935,1.152]`, so the complete candidate was removed. Together with R020's
translation-only fetch cache, this closes one-page instruction-fetch caches in
the current scalar interpreter even when the cache is a direct live-pointer
capability.

R056 then isolated the remaining exact re-entry callback boundary without
omitting probes. Monomorphization improved the frozen local shape 1.494x, but
the complete `run_until` category is only 13.101% of Boot and requires 3.27x
local speed for a 10% row gain. Its best-case projection is 4.5%, so no
production edit was admitted. R023, R040-R041, and R056 close decoder inlining,
blind probe thinning, measured next-entry indexing, and callback
monomorphization as standalone loop-boundary changes.

R057 separately froze the exact accepted ten-module Boot stream and tested the
current two-promise compiler against a two-Worker compiler pool while identical
Wasm ran in the foreground. Worker-local construction was faster, but
transfer/message/module-clone latency made Boot modules ready only 0.489x as
fast and foreground call/wall time tied at 0.998x/1.002x. Compile's larger
stream became ready 1.073x faster but its foreground still tied. This closes a
separate compiler Worker as a throughput mechanism for the accepted policy;
moving compilation between isolates does not remove the dominant T0 work.

R058 tested the broadest remaining call-boundary representation inside T0.
Although `Cpu::step` visibly writes a 24-byte result and `run_until` reloads
its tags, a frozen non-inlined V8 corpus measured a compact i32 outcome at only
0.477x sret throughput `[0.469,0.650]`. Exact state, stop/exception payload,
module bytes, call shape, and host stability passed; warm stability also failed
independently. Synchronous tiering and ten additional non-inlining traces
retained the reversal. Together with R023, this closes both giant-decoder
inlining and compact scalar returns across the existing step boundary.

R059 flattened the one-plus-three jump tables visible in the complete RV64C
dispatch. The architecture-balanced upper bound was locally strong at 1.592x
`[1.592,1.617]`, but one measured warm sequence spread 1.301x and failed its
predeclared stability gate. Even assigning the full ratio to all compressed
`Cpu::step` work projects only 1.130x whole Boot; real handler work narrows it.
The result is retained without a production rewrite, yield retuning, family
weighting, or replacement sample.

No further Boot production candidate is currently admitted. R054 demonstrates
that reducing broad T0 work can clear the gate without another dynamic-code
tier. A future proposal must follow the post-R054 residual profile, execute
broad privileged code without many runtime `WebAssembly.compile` jobs, and
avoid a large function-per-sequence indirect target space. Bundling pages,
opcode pairs, or addresses selected from scorecard populations would violate
the generality gate and is not admissible.

## Corrected next gate

R051's dominant exact fallback site was the same attempted-not-installed cause
already diagnosed in R032 and rejected causally in R034. R052 then used frozen
architecture-general modules and native V8 output to test the clearest new
generated-memory idea: separate one-load packed Sv39 proofs. Exactness gates
passed, but the generated path regressed 43.9% uncached and 11.5% cached. It was
removed before a scorecard. R053 subsequently tested an architecture-complete
precompiled pair tier and measured a 0.879x kernel regression despite halving
dispatch calls; no production change was made. R054 then crossed the gate with
an architecture-general fused interpreter-memory capability: 3.030x frozen
local throughput, 1.151x final-artifact Boot A/B, and 13.35% lower Boot plus
4.73% lower Compile in the valid untouched scorecard. The bounded sequence is
now:

1. Do not reopen current-proof layout, SIMD, scalar packing, stack carry,
   scheduler thinning, entry ranking, module geometry, or precompiled exact
   pair/triple families. R037, R038, R047-R049, R052, and R053 supply causal
   closures.
2. Use R055's fresh exact accepted-R054 Boot and Compile profiles; do not reuse
   R045/R050 percentages that predate the material T0-memory change.
3. Do not reopen instruction-fetch translation/direct-pointer caching or exact
   re-entry callback layout: R020, R055, and R056 close those forms in the
   current scalar interpreter.
4. Do not treat a dedicated compiler Worker as foreground acceleration. R057
   shows no Boot isolation benefit and about twice the publication latency;
   use it only as an explicit embedder/UI isolation option if needed.
5. Do not replace `Cpu::step`'s sret with a compact scalar or force the decoder
   into its callers. R058 shows that V8's non-inlined scalar-return shape loses
   despite smaller Wasm/native code; R023 already rejects the inline form.
6. Do not reopen RV64C dispatch flattening from family frequency or a different
   warmup. R059 records strong isolated throughput but fails its independent
   stability gate and has only a narrow deliberately overstated whole-row
   ceiling.
7. Bound a genuinely different broad mechanism from the refreshed residual. For
   generated memory this means removing or amortizing the per-access proof
   through an exact sparse/radix/mirrored mapping, not compressing it; for
   another category it means eliminating comparable dynamic native work across
   general modules.
8. Demonstrate an address-independent opportunity ceiling of at least 10% of
   the complete Compile row. If no such mechanism exists, record a performance
   plateau rather than reopen a closed family.
9. Any admitted implementation must pass the DBT/system differential matrix,
   an alternating same-Wasm A/B with exact output and stable host probes, Boot
   and all-row guards, and the browser `/shared/bench.py` non-regression gate.

R054 is the accepted production baseline; R055-R059 are rejected and no
successor candidate is currently admitted. The performance goal remains open,
but another edit without a new residual bound would be activity rather than
evidence-driven progress.
