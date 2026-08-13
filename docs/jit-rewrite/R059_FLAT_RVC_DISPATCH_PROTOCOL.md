# R059 flattened RV64C dispatch protocol

Date: 2026-08-08  
Status: rejected at the frozen stability gate; production unchanged

## Question

Can the direct compressed-instruction interpreter replace its nested
quadrant-then-funct3 dispatch with one complete combined-selector dispatch,
removing one Wasm/native jump-table operation from every RV64C instruction?

Accepted R054 computes `c & 3`, executes a three-way `br_table`, then computes
or reuses `(c >> 13) & 7` and executes one of three eight-way `br_table`s. The
candidate computes `((c & 3) << 3) | ((c >> 13) & 7)` and executes one
24-way match. All 24 quadrant/funct3 families remain present; their operands,
legality checks, memory operations, PC updates, exceptions, and retirement are
unchanged. This is a complete ISA rule, not a popular-opcode selector.

R042 already rejected outlining cold instruction families and R045 rejected a
separate handler-dispatched decoded executor. R059 neither adds a call nor a
new interpreter. It tests only the nested control shape still visible inside
the accepted monolithic `Cpu::step`.

## Frozen attribution and leverage

The production control is accepted R054 Wasm SHA-256
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
The fresh post-R054 profile
`target/bench/r055-post-r054-engine-profile/profile-analysis.json` sampled
1,137.088 of 2,279.363 Boot ms in `Cpu::step`. The independent exact R041
trace counted 71,295,025 compressed instructions out of 115,272,067 total
interpreted instructions, or 61.8494%.

Assigning the complete compressed share of `Cpu::step` to dispatch is an
intentional upper bound:

```text
f = (1137.088 / 2279.363) * (71295025 / 115272067) = 0.308543
s = f / (1 / 1.10 - (1 - f)) = 1.41772
```

Thus even this generous category must improve 1.418x locally for a 1.10x
whole-Boot opportunity. The frozen corpus gate is 1.45x median with a 1.40x
bootstrap lower bound. Passing admits one production prototype; it is not a
prediction that dispatch owns the whole 30.85% category.

## Frozen engine-shape upper bound

Emit deterministic `nested` and `flat` modules with identical exported
selector memory, state, loop, 24 handlers, checksums, and exports.

- The selector stream traverses all 24 legal combined values in a fixed
  architecture-balanced permutation. It contains no Boot/Compile opcode
  weights, PC, symbol, guest binary, compiler, checksum, or engine identity.
- `nested` uses exactly one three-way quadrant `br_table` plus three eight-way
  funct3 `br_table`s. `flat` uses exactly one 24-way `br_table`.
- Every handler performs the same small state transform parameterized by a
  distinct family constant. Identical executed handler work prevents V8 from
  merging cases while making this a favorable upper bound on dispatch removal.
- Exported memory prevents the engine from treating selectors as immutable.
  Static disassembly must confirm four versus one `br_table` and no
  `call_indirect` or helper call. Optimizing-tier traces must show both driver
  functions reach TurboFan before steady measurement.
- Untimed correctness probes cover zero work, six operation counts, more than
  one complete stream wrap, and externally mutated selectors; result and full
  memory snapshots must match.

Run seven alternating paired fresh Node 26.5/V8 14.6 processes on affinity
8-15 under the repository benchmark lock. Each process records compile and
instantiation, one first call, eight fixed prewarm calls of 4,194,304
operations, an event-loop yield after every prewarm plus sixteen final yields,
four measured warm calls, and five steady calls of 16,777,216 operations.
Measured warm/steady calls also yield between calls. The yields are frozen
before timing because R057/R058 exposed background tier-publication variance;
they are outside every timed call and are identical for both variants.

Do not change the selector permutation, handler work, yields, iterations,
sample count, order, or thresholds after the first timed pair.

## Admission gates

A production prototype is admitted only if all of the following hold:

1. both modules regenerate byte-identically, validate, and retain the exact
   four-versus-one static dispatch shape;
2. all immutable and externally mutated correctness results plus complete
   memory snapshots match;
3. optimizing traces show both drivers reach TurboFan; warm and steady results
   are stable and each process's measured-warm spread is at most 1.25x;
4. the global host CPU-probe spread is at most 1.25x;
5. paired flat/nested steady throughput is at least 1.45x and its fixed-seed
   bootstrap median lower bound is at least 1.40x; and
6. flat compile plus instantiation and paired cold delta are each below 25 ms.

Failure closes combined compressed dispatch before a production edit. Do not
replace the balanced stream with measured Boot frequencies, specialize only
common families, merge projections from other rejected work, or tune the
selector after seeing engine output.

## Product and promotion gates if admitted

Replace only the complete RV64C match selector in `step_compressed`. Preserve
all 24 family bodies and exact illegal encodings. The rule has no PC, opcode
subset, workload, binary, checksum, compiler, browser, or engine selector.

Run exhaustive RV64C differential tests, core/system/full-state randomized
differentials, memory/atomic/T2 tests, generated-module validation, and direct
plus OpenSBI modern Linux. Then run five alternating fresh-process same-Wasm
Boot/Compile pairs against exact R054. Require exact inputs, outputs, JIT
fingerprints, host/sample stability, Boot median at least 1.10x with bootstrap
lower bound at least 1.00x, and Compile retention at least 0.90x. No failed leg
is replaced.

Only a passing artifact proceeds to the untouched 13-row three-way scorecard
and five-pair Chrome `/shared/bench.py` guard. Promotion requires no non-target
regression of 10%, preserved 13/13 legacy wins and browser non-inferiority, and
improvement or parity on both remaining v86 misses. Otherwise restore exact
R054 and retain the negative evidence.

## Result and decision

The immutable report is
`target/bench/r059-flat-rvc-dispatch-corpus.json`; static disassembly and tier
traces are under `target/bench/r059-flat-rvc-dispatch-shape/`. Both modules
regenerate byte-identically, validate, and match all seven ordinary counts,
three externally mutated-selector probes, full memory snapshots, and every
timed checksum. Nested is 4,738 bytes (`a5b74de3578f...`) with exactly four
`br_table`s; flat is 4,695 bytes (`23754ae27565...`) with exactly one. Neither
contains a Wasm helper call. V8 compiled both 543/504-byte functions with
Liftoff and TurboFan before the paired run.

Seven alternating fresh Node 26.5/V8 14.6 pairs on CPUs 8-15 measured:

- nested: 603.125 million dispatches/second;
- flat: 960.376 million dispatches/second;
- paired flat/nested throughput: 1.592x with bootstrap median interval
  `[1.592,1.617]`; and
- flat cold construction: 0.122 ms, with +0.002 ms paired delta.

Exactness, static/optimizing shape, the 1.45x median, 1.40x lower bound, cold
cost, and 1.012x host-spread gates pass. The predeclared warm-stability gate
does not: the first measured warm call in one flat leg was 5.678 ms, followed
by 4.369, 4.368, and 4.366 ms, a 1.301x spread against the fixed 1.25x limit.
That leg remains in the report. All five later steady samples in that process
were stable near 17.47 ms, and all seven paired steady ratios favor flat, but
the protocol does not permit deleting the warm observation or overriding the
aggregate gate.

Even the favorable 1.592x microkernel ratio projects only 1.130x whole-Boot
speed while assigning the complete 30.85% compressed-step upper-bound category
to dispatch. Real compressed handlers retain fetch, operand, arithmetic,
memory, exception, PC, and retirement work, so product leverage must be lower.
The narrow best-case margin does not justify violating the independent
stability gate. Do not implement the source rewrite, alter prewarm/yields,
weight common families, or rerun into a replacement report. Production remains
exact accepted R054 SHA `4160333352b18b...`.
