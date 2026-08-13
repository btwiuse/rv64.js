# R056 monomorphic interpreter re-entry protocol

Date: 2026-08-08  
Status: rejected at the frozen engine-shape gate; production unchanged

## Question

Can the exact generated-entry predicate used after every interpreted
instruction be monomorphized through the system/interpreter call stack, so V8
sees and inlines the direct dispatch-tag load instead of executing a Wasm
`call_indirect` for every probe?

This does not omit, delay, batch, or approximate a probe. R040 already rejected
checking only at inferred boundaries, and R041 rejected an ahead-of-time
next-entry index at its opportunity gate. R023 already rejected inlining the
large decoder. R056 preserves the current per-instruction check and the
separate `Cpu::step` function; it tests only the dynamic callback boundary that
remains visible in accepted executable bytes.

## Frozen attribution and required local leverage

The control is accepted R054 Wasm SHA-256
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
The fresh profile and analysis are under
`target/bench/engine-profile-r055/phase/` and
`target/bench/r055-post-r054-engine-profile/profile-analysis.json`.

Boot sampled 2,279.363 ms in total and 298.626 ms of self time in
`Cpu::run_until`, a 13.101% whole-row fraction. Accepted Wasm disassembly of
that function contains the expected `call_indirect` after `Cpu::step`. R041's
exact trace recorded 72.574 million warm probes, 84.74% of them sequential
misses. The measured self-time budget is therefore about 4.12 ns per probe,
including loop/result bookkeeping.

Even perfect removal of this category can improve the whole row only 1.151x.
To reach the fixed 1.10x advancement gate, the local path must improve at least
3.27x:

```text
1 / ((1 - 0.13101) + 0.13101 / 3.27) = 1.10
```

Compile STEADY places only 42.859 of 1,545.614 sampled ms in `run_until`, so it
is a guard row, not the motivating opportunity.

## Frozen engine-shape gate

Emit deterministic control and candidate modules with identical memory,
callback, table, direct step-call, loop, PC evolution, tag lookup, result
mixing, and exports. The control reaches the exact dispatch-tag predicate
through `call_indirect`; the candidate spells the same predicate inline. The
memory is exported so the engine cannot treat tags as immutable. Test both
miss and externally populated exact-hit states and require identical results.

Measure seven alternating paired fresh Node 26.5/V8 14.6 processes. A
production edit is admitted only when all of the following hold:

1. both modules regenerate byte-identically and validate;
2. miss and exact-hit results match at every requested iteration count;
3. inline/control steady throughput is at least 3.30x and its bootstrap median
   95% lower bound is at least 3.00x; and
4. candidate compile plus instantiation and the paired cold delta are each
   below 25 ms.

The 3.30x requirement is intentionally slightly above the 3.27x Amdahl bound.
Do not change the tag population, iteration count, callback signature, or gate
after timing. Failure closes callback monomorphization without production code.

## Production and promotion gates if admitted

Propagate concrete callback types from `run_system_jit` through
`SystemJitMachine`, `VirtMachine::run_slice_sampled_until`, and
`Cpu::run_until`; do not inline `Cpu::step`, omit exact checks, or alter page
policy. The rule is architecture-wide and contains no PC, opcode, workload,
binary, checksum, engine, or browser selector.

Because monomorphization changes compile-time Wasm shape, archive immutable
control and candidate artifacts and run five alternating fresh-process
Boot/Compile pairs. Require exact inputs and output/JIT-policy fingerprints,
all sample/host spread gates, Boot paired median at least 1.10x with bootstrap
lower bound at least 1.00x, and Compile at least 0.90x. A failed leg is not
replaced.

If admitted, run the complete workspace and differential matrix, direct and
OpenSBI modern Linux, the untouched authoritative 13-row three-way scorecard,
and the five-pair Chrome `/shared/bench.py` guard. Promotion requires no
non-target row regression of 10% and preserves the current browser
non-inferiority decisions.

## Result and decision

The report is
`target/bench/r056-monomorphic-reentry-corpus.json`. Both modules regenerated
byte-identically, validate, expose externally mutable memory/table state, and
produce identical results for all miss counts plus an externally installed
exact hit. The indirect module is 258 bytes
(`393200f31c3e...`); the inline module is 271 bytes
(`9b6263deb230...`).

Seven alternating fresh Node 26.5/V8 14.6 pairs measured:

- indirect: 753.976 million probes/second;
- inline: 1,126.763 million probes/second;
- paired inline/indirect median: 1.494x, bootstrap median interval
  `[1.492,1.500]`; and
- inline cold construction: 0.175 ms, paired delta -0.028 ms.

The callback boundary is measurable, but 1.494x fails both the frozen 3.30x
median and 3.00x lower-bound gates. Applied to the complete 13.101% Boot
category, its optimistic Amdahl projection is only 1.045x whole-row speedup:

```text
1 / ((1 - 0.13101) + 0.13101 / 1.494) = 1.045
```

No production prototype, artifact A/B, or policy variation is justified.
Retain the emitter, harness, protocol, and report as negative evidence. R023,
R040-R041, and R056 now close decoder inlining, blind re-entry thinning,
next-entry indexing under the measured population, and exact callback
monomorphization as standalone ways to meet the Boot advancement gate.
