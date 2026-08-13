# R116 selective state-residency protocol

Date: 2026-08-10  
Status: Gate A frozen before static census

## Question

Can a structured generated function shorten optimized-native architectural
state live ranges without R111's cross-function boundaries? The current
backend eagerly retains the complete register union in Wasm locals from entry
to exit. R110 measured explicit optimized-native frame traffic at 8.87% of
Compile STEADY, and R113 showed that cost is distributed body pressure rather
than helper-call spill traffic.

R116 tests one architecture-general split already left open by D032: keep hot
or cross-member state resident and materialize cold single-member state. It is
not R039 temporary-local reuse, R103's all-GPR cross-function ABI, R111
partitioning, or a guest/workload-selected register list.

## Frozen rule

For each structured region and each architectural integer register except
`x0`, FP register, and `fcsr`, form the set of members that read or write it.
A member is cyclic when it belongs to a multi-member strongly connected
component or has a self-edge.

An architectural value is **cold** exactly when:

1. its reference set contains exactly one member; and
2. that member is not cyclic.

Every other required value is resident. PC, retirement, fuel, memory-proof
temporaries, selectors, and SSA temporaries remain unchanged. There is no
count, register-number, instruction-family, PC, binary, or workload threshold.

If implemented, a cold read loads the canonical architectural memory cell in
its sole member and a cold output stores that cell at member completion.
Precise side exits retain their existing SSA snapshots. Because a cold member
is acyclic, it can execute at most once per generated invocation. Therefore
each cold read/write adds no more dynamic architectural-memory operations than
the eager entry load / final commit it replaces, and paths that skip the member
perform fewer. No edge or function boundary is added.

## Gate A: static opportunity

Use the immutable deterministic 56-region / 6,258-member compiler-produced
corpus and its `member-shapes.tsv` from R111. Reconstruct exact successors,
SCCs, state masks, current structured-module local counts, and eager-byte
weights. Do not generate or time a candidate.

Gate A passes only if all of these predeclared conditions hold:

- all 56 regions and 6,258 members reproduce and every structured module has
  one function;
- at least 75% of eager-byte weight contains one or more cold values;
- eager-byte-weighted resident architectural state is at most 80% of the
  current union;
- eager-byte-weighted total declared-local footprint falls by at least 5%;
- zero cold values occur in cyclic members;
- for every region, the cold read/write incidence is no greater than the
  exact eager entry-load/final-store operations those values replace; and
- two analyzer runs are byte-identical.

Failure stops R116 without a model or product edit. Do not change the rule,
add a fanout threshold, select registers, or exempt a workload after seeing
the census.

## Gate B if admitted

Gate A admits one diagnostic backend mode, not product timing. Before editing
the emitter, freeze an ordinary-tiered V8 model using every real region. It
must preserve exact module ABI and architectural outcomes, demonstrate the
expected cold-load/store shape, reduce optimized native frame/stack exposure,
and improve fixed-work execution enough to project a construction-debited 1%
whole-Compile result. The model may not force Liftoff/TurboFan, choose modules,
or try another residency rule.

Only a Gate-B survivor may become one default-off same-artifact product
candidate. It then follows full differentials, direct/OpenSBI modern Linux,
R107 construction debit, 15 fixed native Boot/Compile/Python pairs, normalized
work, Chromium, qualified WANIX including `/shared/bench.py`, and the untouched
three-way scorecard. Product admission follows R104's verified-1% rule. Wasm
and source size remain diagnostics only.

## Result

Gate A fails before any emitter or product edit. Two independent analyzer runs
are byte-identical (`6c192c426bd9...`) and reproduce all 56 regions / 6,258
members. Forty regions contain cold state and represent 77.5156% of eager-byte
weight, so the rule is active and clears its breadth gate. It also proves zero
cyclic cold values and zero memory-operation-bound violations.

The removable population is too small:

- eager-byte-weighted whole architectural state: 32.4224 values;
- resident state after the frozen rule: 29.6979 values, ratio `0.917212`
  versus the required at-most `0.80`;
- current declared-local footprint: 131.5698 values;
- projected footprint: 128.8453 values, reduction `2.0376%` versus the
  required at-least `5%`.

The median region has only two cold values; the 95th percentile has eight.
R116 therefore stops without Gate B, candidate bytes, semantics work, or
elapsed timing. Do not expand cold state to multi-member or cyclic values,
select a register count, or choose corpus modules after this result. Such a
change would be a new spill/materialization trade-off without R116's static
non-increase proof.

Evidence is under `target/bench/r116-selective-state/`. The product remains
exact `d9f686a9...`.
