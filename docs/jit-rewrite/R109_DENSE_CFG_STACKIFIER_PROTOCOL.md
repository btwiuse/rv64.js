# R109 Dense CFG Stackifier Protocol

Date: 2026-08-10  
Status: completed; rejected at Gate D; candidate removed; baseline restored

Result: `docs/jit-rewrite/R109_DENSE_CFG_STACKIFIER_RESULT.md`.

## Hypothesis and scope

Replace only the stackifier's ordered tree graph/set representation with a
dense bit matrix and dense bit sets. Region members are already numbered
`0..N`, production caps `N` at 512, and the synthetic entry can use index `N`.
Successor iteration remains ascending, duplicates are removed, SCC traversal
and subgroup order remain identical, and the emitted `Structure` tree must be
exactly equal to the current implementation.

This is architecture-general compiler working storage. It may not inspect a
guest PC, opcode, symbol, binary, privilege, benchmark phase, output, host
engine, or observed result. It does not alter region thresholds, leader/page
caps, CFG discovery, duplication limit, state strategy, Wasm instruction
selection, function/module geometry, async publication, or generated runtime
behavior. The R108 typed-instruction-sink idea is explicitly excluded.

Target: Boot. Protected native rows: Compile and Python. The exact control is
core `aec4b31434a6...`, Wasm source `1da35e70bc9c...`, release Wasm
`d9f686a9ce4f...`, loader `2cbb264f4dac...`, and R087 public cadence.

## Gate A: measurement-ineligible production corpus

Temporarily instrument `stackify` behind an off-by-default diagnostic switch.
Capture every raw `successors` and `entries` input from one exact modern Boot
and all FIRST/PRIME/STEADY phases of one exact Compile guest. Capture overhead,
instrumented Wasm, and elapsed values are ineligible for performance evidence.
Retain the guest/input/runtime hashes, output fingerprints, production-policy
proof, generated execution, phase label, binary corpus, parser, and corpus
SHA-256. Remove the hot capture branch and restore exact baseline source after
the corpus is frozen.

No graph may be selected, discarded, or weighted by candidate timing. Boot is
the target corpus; Compile is a protected/generalization corpus. Every valid
captured graph is retained.

## Gate B: local dense representation proof

Implement the current ordered-tree algorithm and exactly one dense bit-matrix
algorithm side by side in a standalone Wasm model over the frozen corpus. Add
exhaustive small-graph and deterministic randomized graphs through 512 nodes.
Both implementations must serialize byte-identical `Structure` results for
every graph. Then collect seven alternating fresh V8 process pairs, retaining
first-call and naturally tiered steady timing, input/output hashes, Node/V8,
affinity, order, and every raw observation.

Admission to product work requires all of:

1. exact structure equality over the complete production, exhaustive, and
   randomized corpus;
2. Boot-corpus dense/tree paired median speedup at least `1.80x` and lower 95%
   bound at least `1.50x` for both first and steady regimes;
3. no Compile-corpus dense/tree paired median below `1.00x` in either regime;
4. identical graph count and per-graph node/edge/entry accounting; and
5. no workload/PC-selected fast path or post-result representation variant.

R088 gives the Boot structure closure a 2.316% whole-row share. A 1.80x local
speedup projects approximately 1.04% whole-Boot improvement before R107's
small construction debit. This projection admits one product test but awards
no performance credit.

## Gate C: product candidate and correctness

Only after Gate B passes, replace production `structure.rs` with the exact
dense implementation. Keep a reference implementation in tests only. Require:

- exact generated Wasm bytes for every frozen production translation input;
- exhaustive/random structure differentials;
- all rv64-dbt/core/workspace units;
- raw Wasm smoke and 100 randomized scalar differential cases;
- M/A/FP, memory, Sv39/MPRV, WFI, T2 atomic/lifecycle/multientry;
- public and Worker APIs; and
- fresh direct and OpenSBI Linux 6.12.7 boots.

Any semantic repair adds a directed regression but cannot change the candidate
scope or later thresholds.

## Gate D: construction-debited native timing

Run R107's 15 alternating real `RV64Debug.create` pairs and compute the frozen
one-time debit `D`. Then collect 15 alternating fresh-process pairs for Boot,
Compile, and Python, pinned to CPUs 8--15 with exact host probes, public cadence,
production policy, guest/output identities, generated execution, translation
counters, and no replacement legs.

Apply `D` to every candidate sample. Boot must have adjusted paired median at
least `1.01x`, lower 95% bound at least `1.00x`, and adjusted normalized MIPS
in agreement. Compile and Python must each have adjusted paired median at least
`0.99x` and no interval establishing regression. Raw and adjusted results are
both retained. If the fixed maximum remains unresolved, the candidate is
inconclusive and is not promoted or declared intrinsically neutral.

## Gate E: browser, WANIX, and authority

Only after Gate D passes:

1. collect 15 fresh Chromium Worker pairs with both execution-only Boot and
   construction-to-ready clocks; Boot must satisfy the same 1% target and
   confidence rules on both clocks;
2. run the R094-qualified seven-browser-by-three fixed-work WANIX guard,
   including unchanged `python /shared/bench.py`, with every protected median
   at least `0.99x` and no established regression;
3. build the clean default product, rerun strict correctness, and run the
   untouched corrected-cadence 117-trial legacy/rewrite/v86 scorecard.

Promotion retains 13/13 versus legacy, at least 11/13 versus v86, an adjusted
Boot gain of at least 1%, and no protected-row regression. Only raw scorecard
parity counts toward the thread goal. Stop at the first failed gate, archive
all evidence, remove the candidate, and restore the exact baseline; do not try
row width, hybrid tree/bit sets, alternate SCC algorithms, or instruction-sink
composition after observing a result.
