# Pure-Interpreter Post-G003 Feasibility Boundary

Date: 2026-08-11 (America/Phoenix)  
Status: blocked under the fixed comparison constraints; parity goal not achieved

## Purpose

This document records where the honest pure-interpreter campaign stops after
G001, G002, and the final G003 build screen. The filename is retained because
it was frozen after G002; this revision adds G003 without changing any earlier
result. It is not a parity claim and not a theorem that no future
interpreter can ever be faster. It is an evidence boundary: under the fixed
guest binaries, fixed ISA contracts, JIT disabled on both sides, and prohibition
on workload-derived recognizers, the audited source and measured execution
representations identify no remaining mechanism with credible leverage to
close the String gap.

The correct current result is still exact I004: eleven wins, one match, and
one loss in both authoritative 13-row populations. The goal remains open and
not reached.

## Fixed comparison and remaining gap

The comparison keeps:

- exact I004 RV64 production source and Wasm;
- pinned copy/v86 commit `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`;
- guest JIT disabled and independently proved inactive on both sides;
- the same fixed benchmark work and output checks; and
- no guest PC, binary, symbol, benchmark, measured opcode sequence, or
  workload identity in an optimization decision.

On the authoritative development String row, exact I004 takes
`5,496.369485 ms` for `796,532,587` reported RV64 instructions. copy/v86 takes
`1,751.212069 ms` for `96,490,537` reported x86 instructions modulo `2^32`.
The counts are not equivalent cross-ISA work units, but they expose the causal
shape:

- the RV64 binary executes about `8.25497x` as many reported instructions;
- I004 is already `2.630x` faster per reported instruction;
- matching wall time at unchanged RV64 work needs another `3.13861x` complete
  development-row speedup; and
- the fair stock-musl String result is farther behind at `0.307555x`, or about
  `3.25x`.

The pinned comparator audit found no workload recognizer. copy/v86's material
String acceleration implements the complete architecturally defined x86 REP
string-instruction family. One `REP MOVS` instruction may move a permission-
and page-bounded chunk through host `memcpy`; it is not recognition of a
multi-instruction benchmark loop. The RV64 guest expresses the corresponding
work as many scalar instructions, even in the already-customized development
`fastmem.c` path.

## Measured representation boundary

The campaign did not infer failure from code size or intuition. It tested the
broad mechanism classes prospectively and retained or removed them according
to frozen gates.

| Mechanism | Architecture/workload scope | Relevant result | Disposition |
|---|---|---:|---|
| I001 executable-page capability | every direct executable page | retained | live in I004 |
| I003 data slow-path outlining | every scalar load/store miss/fault | retained | live in I004 |
| I004 fetch-refill outlining | every executable-page miss | retained | live baseline |
| I002 data-page local capability | every scalar load/store | target/protection gate failed | removed |
| I005 interrupt countdown locality | every integrated scalar instruction | gate failed | removed |
| I006 one-read fetch | every 32-bit scalar fetch | gate failed | removed |
| I007 M-family outlining | complete architectural M family | gate failed | removed |
| I008 exact-revalidated decoded blocks | complete ordinary scalar class | String `0.6950x`, Bitfield `0.6803x` | removed; cache variants closed |
| R053 exhaustive operation-pair dispatch | all 3,844 ordered operation pairs | `0.879x` local throughput | sequence-dispatch family closed |
| R066 integrated complete scalar driver | complete RV64I/M scalar path | real Boot `1.074x`, below gate; protected rows regressed slightly | removed historically; current integrated form already reflects later accepted work |
| R083 full-system const specialization | complete user/system interpreter bodies | Boot `0.888x` | removed |
| I009/I010 native multi-row attribution | String, Compile, Python, no opcode selection | only retained/closed fetch, countdown, and cache bands qualified | no candidate |
| G001 `FENCE.I`-coherent decoded execution | complete balanced ordinary scalar class | favorable standalone `1.662022x [1.644338, 1.680755]` | failed `3.75x`/`3.50x`; no production edit |
| G002 complete local GPR file | all `x1`--`x31`, balanced dynamic operands | favorable state-only `0.098116x [0.097818, 0.098492]` | about `10.19x` slower; no production edit |
| G003 standard LLVM source levels | O1/O2/Os/Oz versus O3 on the complete balanced direct interpreter | O1/O2 byte-identical and `~1.00x`; smaller Os/Oz `~0.97x` | all fail `3.25x`/`3.00x`; no production build |
| R070/R071 hand-emitted static Tier 0 (historical transfer) | complete RV64I/M, integer RVC, and scalar-memory surface; no guest selector | isolated corpora `1.624x`--`2.445x`; full-system Boot `1.047x`, independent confirmation `1.024x` | rejected and removed |
| R072--R078 sampled static Tier 0 lifecycle (historical transfer) | same complete core entered by architecture-wide page-policy rules | native Boot up to `1.209x`; browser guards failed; R077 raw scorecard Boot `0.986x` versus baseline; dormant removal restored `1.178x` | rejected; machinery removed |
| R082 separated external Tier 0 (historical transfer) | guest-independent emitter/compiler and complete scalar executor | dormant-capable Boot `1.005x [0.967, 1.092]`; missed frozen confidence floor before active timing | rejected and removed |
| R095/R115 prebuilt and same-instance Tier 0 (historical transfer) | complete guest-independent scalar executor, with no runtime compiler or workload selector | external Boot `0.944x [0.933, 0.949]`; same-instance boundary recovered `1.03413x`, but embedded enabled/disabled Boot was `0.97987x [0.95574, 0.99572]` | executor rejected; no product change |

G001 removed byte fetch and field decode from repeated execution under a
standardized `FENCE.I` coherence rule. Even its isolated balanced model reached
only about half the preregistered leverage floor. No capacity, hash, layout,
validation, boundary, or warmup retry is admissible.

G002 removed all per-instruction register-memory traffic and intentionally
omitted every unrelated interpreter cost. It therefore overcredited local
residency. Nevertheless, static Wasm local indices required three complete
dynamic selectors per modeled instruction, making it roughly ten times slower
than two direct state loads and one guarded store. No local-count, compact-bank,
hybrid, selector-tree, SIMD-lane, or predecoded-register retry is admissible.

Together with the exact native body censuses, these results close the two
remaining representation-scale hypotheses and the remaining standard compiler
level axis without observing another guest benchmark. G003 also confirms that
ordinary LLVM inlining/size policy cannot supply the missing leverage: three
levels emit exact control bytes, while the sole smaller shape is measurably
slower.

The R070--R115 rows are supporting transfer evidence, not additional
pure-interpreter scorecard results. Their full-system product screens retained
the ordinary generated page/region JIT, so their ratios cannot be pooled with
I004 or used to claim JIT-off parity. They are relevant because they already
implemented the strongest architecture-general alternative to the
LLVM-shaped Rust interpreter: a flat, hand-emitted, guest-independent executor
covering the complete ordinary scalar surface. Its most favorable isolated
corpus reached `2.445x`, still below the fair String deficit, while every
integrated lifecycle either delivered only a small gain, failed a protected
browser/product gate, or regressed. R115 also removes the strongest unresolved
packaging excuse: avoiding the cross-instance boundary helped, but the exact
embedded executor remained slower than its disabled control.

## Why the apparent shortcut is ineligible

The withdrawn artifact did close the wall-time gap. It did so by recognizing
the exact measured RV64 copy, compare, bit, assignment, and heapsort loops and
replacing hundreds of millions of their scalar retirements with bespoke host
operations. That proves the missing leverage is obtainable only when many
guest instructions are collapsed at once; it does not prove a general
interpreter optimization.

Restoring those helpers, recognizing slightly generalized forms of the same
loops, selecting guest PCs, or building a library of measured instruction
sequences would repeat the same reward hacking. Calling it an RV64 counterpart
to x86 REP would also be false: REP is one standardized x86 instruction whose
semantics copy/v86 implements for every guest, whereas the removed helpers
recognized multi-instruction compiler output after seeing benchmark losses.

An automatic system that discovers and executes arbitrary multi-instruction
regions is dynamic translation. If it emits or compiles host code it is the
JIT explicitly disabled by this comparison. If it caches decoded regions and
interprets them, it is the closed I008/G001 representation unless it also
recognizes and fuses semantics; that fusion then needs an independently
justified architecture-general rule rather than the known loops.

## Final requirement and blocked audit

The completion audit checks the original objective directly rather than
substituting the successful intermediate work for parity:

| Requirement | Authoritative evidence | Status |
|---|---|---|
| Remove benchmark-derived direct-interpreter specializations | current `cpu.rs` is exact clean I004 `d8d1322f...`; the recognizers, counters, negative cache, and dedicated tests are absent | complete |
| Correct the invalid parity record | `STATUS.md` and `INTERPRETER_EXERCISE_REPORT.md` withdraw the old `goalMet=true` interpretation and identify the tuned artifact as ineligible | complete |
| Pin and audit copy/v86 | exact commit `2f1346b0...`, source tree `ca8afd71...`, Wasm `4a1b966e...`; comparator audit finds no workload recognizer | complete |
| Run the authoritative development comparison with both JITs disabled | 78/78 eligible trials, all inactive proofs true, empty problems; I004 is `0.3186125x` on String | complete, parity fails |
| Run the fair stock-musl comparison once | 78/78 eligible trials, all inactive proofs true, empty problems; I004 is `0.3075551x` on String | complete, parity fails |
| Run the sealed holdout once | 24/24 eligible trials, all inactive proofs true, empty problems; I004 wins all four rows | complete, transfer passes |
| Reach architecture-general pure-interpreter parity | both required 13-row populations lose String by more than `3x` | **not achieved** |
| Identify an eligible next implementation with credible leverage | I001--I010, G001--G003, and the reconciled R053/R066/R070--R115 representation families leave no open candidate near the required leverage | **blocked** |

The JIT subsystem still contains its earlier whole-loop bulk-copy lowering.
It is not silently counted here: all interpreter trials report zero generated
instructions, dispatches, translation attempts, registered modules, region
activity, and bulk-copy-helper activity. The anti-overfit protocol classifies
that JIT-only lowering as an exact compiler-idiom recognizer and requires it to
be removed or independently transfer-qualified before any future JIT-enabled
parity claim. This audit therefore claims only that the pure-interpreter
comparison is clean; it does not relabel the paused JIT-enabled artifact as
anti-overfit-qualified.

The blocking condition is the conjunction of the unchanged scalar `rv64gc`
guest, pinned x86 guest with architectural REP, JIT disabled on both sides,
and prohibition on workload-derived multi-instruction fusion. Under that
contract, exact I004 executes about `8.255x` the reported String instruction
work and already runs each reported instruction about `2.630x` faster, leaving
a `3.139x` whole-row gap. No measured architecture-general interpreter
representation supplies that leverage. Further work requires one of the
explicit constraint changes below or a genuinely new portable primitive that
can be selected and qualified independently of every opened workload.

## Legitimate ways to reopen the objective

At least one comparison constraint must change before another implementation
has a credible route to the missing `3.14x`--`3.25x` complete-row gain:

1. **Enable the existing architecture-general JIT/DBT.** This changes the
   execution-mode question and belongs to the paused JIT-enabled scorecard, not
   the pure-interpreter result.
2. **Change the guest ISA and rebuild both the contract and validation
   populations.** A standardized RV vector or future bulk-memory operation
   could express work that the current scalar `rv64gc` binary cannot. This is
   a new comparison, not an improvement of the frozen binaries.
3. **Admit a genuinely new portable execution primitive.** It must be selected
   from ISA/runtime semantics before benchmark observation, have an independent
   newly sealed transfer population, and show standalone leverage consistent
   with the full remaining gap before production work.
4. **Change the fairness rule explicitly.** Disabling or degrading x86 REP,
   changing only the RV64 benchmark library, or accepting known-loop
   recognizers could produce a different score, but none establishes the
   requested general pure-interpreter parity.

Small honest local gains remain scientifically possible, but the audited
native exposure and failed broad representations provide no basis for claiming
that a collection of unrelated percent-level edits will remove `68.139%` of
complete String time. Such a collection may not be assembled from known
scorecard feedback and reported as an independently validated route.

## Current decision and preserved state

No G001, G002, or G003 production code was implemented. No development,
stock-musl, holdout, or Embench guest was run for any candidate. The G001
Embench images remain unexecuted, but they cannot become a result-driven
successor input.

The preserved live identities are:

- `crates/rv64-core/src/cpu.rs`:
  `d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`;
- `.cargo/config.toml`:
  `252a344de3e565c134906a497e33f88795eae1a29f1357bbfb05ffea911bc267`;
- release Wasm:
  `7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`;
  and
- loader:
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.

The JIT-enabled scorecard stays paused. The pure-interpreter goal is **not
reached** and is blocked under the fixed comparison constraints. With the
present constraints there is no eligible next production candidate, and
inventing one from the known String loss would violate the anti-overfit rule
rather than solve the objective.
