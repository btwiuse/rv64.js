# R108 DBT Frontend Residual Attribution

Date: 2026-08-10  
Status: diagnostic complete; dense CFG model admitted; typed sink closed standalone

## Exact production evidence

R088's corrected-cadence CPU profiles are measurement-ineligible attribution,
but they retain the exact R085-equivalent runtime, production policy, modern
Linux/Alpine inputs, output proofs, and generated execution. The complete
closure report is
`target/bench/r088-r085-public-cadence-profile/closure-analysis.json`.

DBT translation and issue owns 5.917% of Boot and 5.548% of Compile STEADY.
The authoritative R087 counters independently record 111--134 ms of synchronous
RV64-to-Wasm translation over 14--15 Boot attempts and 73--82 ms over 9--13
Compile STEADY attempts. Dynamic module compilation is additional measured
work and is not part of these translation counters.

The next representation must be selected from leaves that are both live and
large enough for R104's 1% floor:

| Closure | Boot sampled | Compile STEADY sampled | Optimistic free ceiling |
|---|---:|---:|---:|
| structured CFG + B-tree operations | 51.466 ms (2.316%) | 29.065 ms (1.139%) | 1.0237x / 1.0115x |
| all wasm-encoder leaves | 40.604 ms (1.827%) | 48.711 ms (1.910%) | 1.0186x / 1.0195x |
| generic `Instruction::encode` alone | 12.400 ms (0.558%) | 22.325 ms (0.875%) | 1.0056x / 1.0088x |

The structured graph consists of dense member indices 0 through at most 511,
plus one synthetic entry. Its current `BTreeMap<usize, BTreeSet<usize>>`
representation pays ordered-tree allocation/search even though the key space
is already dense and bounded. A bit-matrix/set implementation that is at least
1.80x faster locally would conservatively project beyond 1% on Boot. Compile
is protected rather than claimed as a target.

## Typed instruction-sink proof

`wasm-encoder` 0.255 exposes a typed `InstructionSink` that writes each opcode
directly. The current emitter constructs an `Instruction` enum and calls its
general encoder at every site. A byte-exact 86,016-instruction model confirms
that this is real local overhead:

- native release model: approximately 2.25x faster;
- seven alternating fresh V8 process pairs, first large call:
  `1.377x` `[1.303,1.433]`;
- naturally tiered V8 steady calls: `1.273x` `[1.253,1.354]`.

The immutable Wasm report is
`target/bench/r108-instruction-sink-opportunity/node-model.json`, SHA-256
`95e0008da81f74b11fa487e4283194c093dc17f72055e1aaa7625bed6296e1eb`.
Both paths produce exactly the same 176,070-byte function body; the model Wasm
is `6e6fa0125ee1...`, Node is 26.5.0 / V8 14.6.202.34-node.24, and CPUs are
8--15.

Despite the positive local result, the directly removable generic-encoder leaf
is below 1% in both open rows. Rewriting thousands of call sites cannot claim
the broader immediate-encoding leaves without a separate proof, so the typed
sink is closed as a standalone product candidate. It may be reconsidered only
after an independently useful emitter redesign, not bundled opportunistically
to push an unrelated point estimate over a threshold.

## Admission

Admit one R109 proof-only dense-CFG experiment. First capture exact production
CFG inputs with timing disabled, then compare the current ordered-tree
stackifier with one dense bit-matrix implementation. No product timing begins
unless output structures are exact and the frozen local speed gate is met.
