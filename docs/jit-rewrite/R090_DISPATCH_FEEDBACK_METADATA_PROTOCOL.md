# R090 Dispatch Feedback Metadata Protocol

Date: 2026-08-09  
Status: diagnostic complete; metadata prototype rejected before implementation

## Question

R088 assigns 16.898% of corrected-cadence Compile self time to scheduler work.
After every non-region generated return, exact R085 currently probes both the
authoritative compiled-block map and the indirect-specialization map before it
can decide whether successor feedback is needed. Can immutable metadata in the
existing 16-byte direct dispatch line remove those probes broadly enough to
produce a reproducible three-to-five-percent whole-workload improvement?

This is distinct from the closed dispatch-count experiments: it does not omit,
batch, or move a dispatch, generated-entry probe, policy observation, or
interrupt boundary. It changes the representation of information already
known when a direct dispatch line is filled.

## Frozen control and diagnostic

The immutable control is exact promoted R085:

- runtime Wasm
  `efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`;
- loader
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- corrected authoritative report
  `1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7`;
  and
- `crates/rv64-wasm/src/lib.rs`
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`.

Build one instrumentation-only artifact. It adds six monotonically increasing
diagnostic counters, exposed as `jit_stat(134..139)`, around the existing
feedback decision:

1. non-region generated returns;
2. explicit generated guard misses;
3. ordinary two-map feedback checks;
4. returns classified as one-body;
5. returns whose owner already has embedded indirect targets; and
6. ordinary successor observations.

The diagnostic must preserve the exact evaluation order and all state
transitions. Its wall times are invalid and must never be compared with R085.
Run fresh modern Boot and Compile processes under production page policy and
R087's `public-one-slice-per-turn` cadence. Require exact input/output identity,
nonzero generated execution, and these accounting invariants:

- ordinary checks plus explicit misses equal non-region returns;
- embedded-present and ordinary observations do not exceed ordinary checks;
- ordinary observations do not exceed one-body returns; and
- the counters are monotonic across FIRST, PRIME, and STEADY snapshots.

## Admission rule

Restore exact R085 after collecting the diagnostic. Admit one implementation
only if ordinary two-map checks cover at least 50% of all generated dispatches
in Compile STEADY and at least 25% in Boot. Those deliberately conservative
dynamic floors establish that the mechanism reaches both remaining rows; they
do not claim a wall-time gain.

The single frozen implementation may pack the exact T1 static length (bounded
by the architecture-wide 128-instruction T1 limit) and a specialization state
into otherwise unused high bits of the dispatch-line table index. The actual
table index, region tag, and blacklist sign must remain disjoint. It must:

- preserve every feedback observation and explicit guard miss;
- preserve the 16-byte dispatch-line ABI and exact full-PC/map-generation
  validation;
- apply uniformly to every guest PC, opcode, address space, workload, and Wasm
  engine;
- add no selector, threshold sweep, or runtime switch; and
- mask metadata before every host or generated table call.

Before performance, require static layout/index proofs, the complete strict
correctness matrix, direct and OpenSBI modern Linux boot, and at least three
fresh scorecard Boots with exact output and generated execution.

Run five alternating fresh-process native pairs for Boot, Compile, and Python
using exact R085 as control. A candidate advances with paired median speedup of
at least 1.03x on Boot or Compile, a 95% lower bound of at least 0.98x on the
winning row, and no other row slower than 1.03x by side median. Stop and restore
at the first failed gate; do not tune bit allocation, feedback population, or
generated-code shape after observing timings. Browser `/shared/bench.py` and
the complete three-way scorecard remain mandatory promotion gates.

## Result and decision

The 4,279,723-byte instrumentation-only artifact is
`target/bench/wasm-candidates/r090-feedback-inventory-085adb2da85d.wasm`,
SHA-256
`085adb2da85d1e116bac5f47aa5a617b453b32f66ea39eb825b3e7fd11beaf55`.
It used exact loader `2cbb264f4dac...`; the modern guest, production page
policy, public one-slice cadence, inputs, Compile MD5, and generated-execution
proof all match.

The proof-only report is
`target/bench/r090-dispatch-feedback/report/scorecard-v2-2026-08-09T20-40-07-083Z.json`,
SHA-256 `4b5e076ad4d05bbf507c35e6c7de8674b75168ae4a7fec7b2236183a67541495`.
Its only problems are the expected declarations that diagnostic code entered
Boot and Compile measurements; no elapsed duration is evidence.

The independent gate is `target/bench/r090-dispatch-feedback/gate.json`,
SHA-256 `dc33820fefcb9c399c68b8702be09e3d6143ef16cffbba83d50cfa72bb41de`.
All accounting and identity checks pass, but the dynamic opportunity is zero:

- Boot FIRST: 575,382 generated dispatches and zero non-region returns,
  ordinary checks, explicit misses, or observations;
- Compile FIRST: 733,451 generated dispatches and the same all-zero feedback
  inventory;
- Compile PRIME: 572,534 generated dispatches and the same all-zero inventory;
  and
- Compile STEADY: 612,605 generated dispatches and the same all-zero inventory.

Production page policy therefore enters only region functions at the outer
generated-dispatch boundary in these rows. The `is_region` branch bypasses the
suspected maps completely. Packing feedback metadata cannot affect either
remaining workload, so no implementation or timing variant is admitted.

All instrumentation and the temporary harness fields were removed. Release
Wasm is restored byte-exact R085 at `efd7830307ef...`, the scorecard library is
restored to `8681b09f81f3...`, and both scorecard self-tests pass. This narrows
Compile's 16.898% scheduler category to the region call/return loop and its
boundary work rather than indirect-target feedback.
