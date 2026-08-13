# R054 Interpreter Fused-Memory Protocol

Status: promoted; new accepted production baseline  
Date: 2026-08-08

## Question and measured ceiling

Can the T0 interpreter consume the fused JIT-TLB capability it already
publishes, directly accessing proven RAM on a hit, instead of repeating the
standard translation lookup, physical-bus range/dispatch path, and fused-row
publication for every data access?

The accepted R045 Boot CPU profile attributes 232.979 ms (8.501%) to complete
`Cpu::ld` subtrees and 223.267 ms (8.146%) to complete `Cpu::st` subtrees: a
combined 16.647% whole-Boot ceiling. A mechanism must remove at least 60.1% of
that subtree to have a 10% whole-row opportunity, equivalent to at least 2.51x
subtree throughput under an optimistic Amdahl bound.

This is distinct from prior memory work. R008-R019 and R037-R052 change
generated-code translation; R020 changes instruction fetch; R025/R045 change
decoded instruction dispatch. R054 changes the interpreter's data-memory
execution path and reuses an existing exact capability.

## Frozen comparison

`emit_interpreter_fused_memory_corpus.rs` emits two deterministic standalone
modules executing identical mixed 64-bit load/store streams:

- `control.wasm` models a successful standard TLB probe, physical RAM checks,
  store dirty-page test, RAM access, and fused-row publication;
- `fused.wasm` first checks the exact fused tag and directly uses its proven
  linear/native pointer, with the complete control path retained as a cold
  miss fallback.

The corpus conservatively gives the control the same precomputed permission
context cell as the candidate rather than modeling its more expensive
`mstatus`/effective-mode reconstruction. It retains the page-crossing guard and
all candidate tag/offset loads. Memory is exported and reset outside timed
calls; state and the mutated data window are independently hashed.

The harness must regenerate both modules twice, validate and hash them, prove
identical state/memory at multiple iteration counts, and use seven alternating
paired fresh Node/V8 processes. Compile, instantiate, first, warm, and steady
times remain separate.

## Predeclared decision

Admit a default-off production prototype only if:

1. bytes are deterministic and valid;
2. state and mutated memory are exact and stable;
3. paired median mixed-memory throughput is at least 2.51x control, the
   optimistic minimum needed for 10% whole-Boot leverage;
4. the lower bootstrap median bound is at least 2.25x, ruling out a fragile
   tiering artifact; and
5. candidate median compile plus instantiate time and its paired delta are
   each below 25 ms.

Failure closes direct interpreter consumption of the current fused rows
without a hit-rate, width, opcode, address, or workload selector. Passing only
licenses correctness implementation and focused Boot/Compile A/B; it does not
promote a production change.

## Frozen corpus result

The immutable report is
`target/bench/r054-interpreter-fused-memory-corpus.json`. Seven alternating
fresh Node 26.5/V8 14.6 pairs produced 3.030x median candidate throughput with
a 3.024-3.041 deterministic bootstrap interval. Exact state and mutated-memory
fingerprints match, and median candidate cold construction was 0.222 ms with a
-0.017 ms paired delta. The corpus therefore passes every gate above and
licenses a default-off production prototype. It is not evidence of a Linux
row improvement.

## Preregistered production screen

The production prototype changes only interpreted scalar data loads/stores.
Generated memory, instruction fetch, decoding, page policy, thresholds, and
module geometry remain unchanged. The exported diagnostic switch permits both
legs to execute the exact same Wasm bytes.

Before inspecting any modern-Linux timing, run five alternating fresh-process
pairs for both `boot` and `compile`, pinned to CPUs 8-15. Boot uses the `first`
phase; Compile uses `steady`, after the existing PRIME and settle barriers.
Retain all samples. The report must verify exact Wasm and input hashes, equal
guest fingerprints, per-side sample spread no greater than 1.25, and host-probe
spread no greater than 1.25. Paired speedups are summarized with the fixed-seed
4,096-resample median interval used elsewhere in the project.

The candidate advances only if:

1. its paired median speedup is at least 1.10x on Boot or Compile;
2. the lower paired-bootstrap bound for that advancing row is at least 1.00x;
3. the other row is no worse than 0.90x by paired median;
4. all functional, artifact, sample-spread, and host-stability gates pass.

A passing same-Wasm result licenses an artifact A/B against accepted
`d93345139c5a...`; it still does not permit promotion. That second screen must
include all candidate code-shape cost and meet the same performance and
correctness gates. Only then may the full 13-row three-way scorecard and the
browser `/shared/bench.py` non-regression guard run. Failure at either focused
screen removes the production path without threshold, address, opcode, or
workload tuning.

## Production result and decision

Every preregistered gate passed:

- Same-Wasm report
  `target/bench/r054-interpreter-fused-memory-same-wasm-ab/config-ab-2026-08-08T21-04-11-879Z.json`
  measured Boot at 1.161x paired speedup with interval 1.113-1.181 and Compile
  at 1.022x with interval 0.968-1.058.
- Exact final-artifact report
  `target/bench/r054-final-artifact-ab/config-ab-2026-08-08T21-11-28-235Z.json`
  compared accepted `d93345139c5a...` with final default-on
  `4160333352b1...`. Boot measured 1.151x with interval 1.130-1.168; Compile
  measured 1.070x with interval 0.993-1.079. Inputs, outputs, sample spreads,
  and host probes were valid.
- The first complete authoritative scorecard is retained but invalid solely
  because legacy String Sort spread was 1.28x. No sample was replaced. The
  complete untouched rerun is
  `target/bench/r054-final-three-way-rerun/scorecard-v2-2026-08-08T23-01-30-777Z.json`;
  it is authoritative, valid, and has no problems.
- Relative to the prior accepted scorecard, rewrite Boot fell from 2,608.9 to
  2,260.5 ms, a 13.35% time reduction. Compile fell from 1,113.6 to 1,060.9
  ms, a 4.73% reduction. No other rewrite row regressed by 10%.
- Five alternating fresh Chrome 150 pairs are retained in
  `target/jit-policy-traces/wanix-r054-416033-chrome-20260808/analysis.json`.
  Paired geometric RV64/v86 time ratios were 0.841 for Python, 0.619 for
  SHA-256, and 0.608 for shared 9P; every upper 95% bound was below 0.89.

Exact directed coverage includes 1/2/4/8-byte and unaligned accesses, context
tag changes, Sv39+MPRV, cross-page and MMIO fallback, and generated-code-page
store invalidation. The wider Rust/Wasm/randomized/T2 and both modern-Linux
boot matrices pass. Promote final Wasm
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`
as the new accepted baseline. The diagnostic setter remains only to reproduce
the exact disabled path.

R054 advances the objective but does not complete it. The valid scorecard is
still 11/13 against copy/v86, with Boot at 2,260.5/1,525.8 ms and Compile at
1,060.9/718.5 ms. Any next candidate starts from `416033...` and must retain
the same correctness and browser gates.
