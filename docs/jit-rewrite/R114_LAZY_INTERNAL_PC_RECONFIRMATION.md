# R114 Lazy Internal-PC Reconfirmation

Date: 2026-08-10  
Status: rejected at frozen Gate B; candidate removed; exact baseline restored

## Question

Does deferring the architectural-PC local assignment across covered structured
CFG edges produce a verified net Compile gain of at least 1% on the exact
current product, without regressing Boot, Python, WANIX, or the authoritative
three-way scorecard?

R014 is admission evidence only. Its balanced Compile point estimates were
approximately +1.1% FIRST, +0.2% PRIME, and +3.0% STEADY, but its control
alternated between two generated-coverage strata and it bundled lazy PC with
the old sparse-safepoint experiment. No R014 timing or artifact is reused in
the decision. R102 subsequently reconstructed the fuel-check axis and tied;
R114 therefore tests only architectural-PC materialization.

The abandoned block-parameter proposal is not part of R114. A source audit
showed that it would mostly re-express the R039 local-state and R103 carried-
state mechanisms rather than remove an independent operation.

## Immutable control and scope

The control is:

- core CPU source
  `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`;
- Wasm runtime source
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- release Wasm
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`,
  exactly 4,279,380 bytes;
- loader
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- modern Linux 6.12.7 / Alpine 3.24.1 inputs, R087 public cadence, and the
  corrected scorecard contracts; and
- the authoritative R087 score, 13/13 versus legacy and 11/13 versus v86.

Target native row: Compile. Protected native rows: Boot and Python.

The sole candidate is architecture-general and may do exactly this:

1. In `RegisterStructured` generated functions, retain each completed
   member's exact `next_pc` SSA local without immediately copying it to the
   architectural `state.pc` local.
2. Omit that copy on a statically covered internal fallthrough or branch.
3. Materialize the exact `next_pc` only on the taken path before:
   - a fuel/hop-cap exit from the structured function;
   - a statically external successor;
   - a dynamic successor that returns to the outer PC dispatcher; or
   - the defensive structured fallthrough exit.
4. Leave precise side exits unchanged: they already store their exact side-
   exit PC directly to architectural memory before returning.
5. Leave the common final state commit, cross-module chain lookup, retirement,
   fuel policy, selector lowering, CFG structure, and all non-structured modes
   unchanged.

The first artifact contains a default-off same-Wasm causal switch. It may add
one public debug option/setter solely for the experiment. The switch chooses
only the lowering above; it may not inspect a guest PC, instruction, symbol,
binary, privilege, benchmark, output, engine, tier, or observed timing.

R114 may not coalesce fuel checks, alter the 128-instruction scheduling bound,
add block parameters, carry GPRs across function boundaries, split regions,
change leader/page/coverage policy, change local allocation, add a br_table,
or compose any previous candidate. No alternate materialization placement is
permitted after timing starts.

## Gate A: structural and semantic proof

Before performance timing:

1. Add directed generated-Wasm tests for an internal fallthrough, internal
   unconditional branch, mixed internal/external conditional, two external
   arms, dynamic successor, fuel exit, no-fuel hop-cap exit, precise memory
   side exit, cross-module chain exit, and defensive fallthrough.
2. For the internal-only case, decode the generated function and prove that
   candidate mode removes the member-boundary `local.get next_pc; local.set
   state.pc` pair while control retains it. For every leaving path, prove by
   execution that architectural PC, registers, memory, retirement, and fuel
   match control/interpreter state exactly.
3. Run all rv64-dbt/core/system units and the complete scalar, M, A, FP,
   memory, Sv39/MPRV, WFI, T2 atomic/lifecycle/multientry, public API, Worker,
   and raw-Wasm differentials in both relevant modes.
4. Fresh direct and OpenSBI Linux 6.12.7 boots must reach their exact readiness
   markers, execute a shell command, and prove nonzero generated execution.
5. Capture the deterministic real-region corpus in control and candidate
   modes. Input manifests, module population, entry sets, state modes, helper
   ABI, imports, exports, memory contract, and translation outcomes must match.
   Report aggregate and per-module Wasm/operator deltas; sizes are diagnostics,
   not acceptance thresholds.

A correctness repair may restore only the frozen semantics and must add a
directed regression. It may not change placement or performance scope.

## Gate B: construction-debited native timing

Freeze the exact default-off causal Wasm, source archive, loader, harness, guest/input
hashes, pair order, Node/V8 identity, affinity, outputs, and gate implementation
before the first candidate sample.

1. Use that same Wasm artifact and exact loader for both arms; select only the
   off/on raw-Wasm switch after construction. Run R107's 15 alternating
   fresh-process real `await RV64Debug.create(wasmBytes)` pairs on CPUs 8--15.
   Let causal `D` be the nonnegative upper 95% paired-bootstrap bound of the
   median candidate-minus-control construction delta. Although both paths are
   byte-identical, retaining this conservative noise debit prevents a lucky
   construction sample from helping the candidate.
2. Run 15 alternating fresh-process pairs for Boot, Compile, and Python on
   CPUs 8--15. All legs use exact fixed work, R087 one-slice cadence,
   production policy, modern inputs, and complete artifact/output/generated-
   execution/host proofs. No leg is replaced and there is no extension.
3. Charge `D` once to every candidate row sample before analysis.

R114 advances only if all of these prospectively frozen R104 rules pass:

- adjusted Compile paired-median speedup is at least `1.01x`;
- its deterministic paired-bootstrap 95% lower bound is at least `1.00x`;
- adjusted fixed-work normalized Compile MIPS agrees at at least `1.01x`;
- Boot and Python adjusted paired medians are each at least `0.99x`, and
  neither confidence interval establishes a regression; and
- all identities, work, output, cadence, policy, generated-execution,
  construction, and host-stability checks pass.

A favorable single leg, unadjusted point estimate, source byte reduction, or
historical R014 sample is not a pass. If the fixed 15 pairs cannot resolve the
target, the result is inconclusive and the candidate is removed without a
sample extension.

## Gate C: browser, WANIX, and authority

Only after Gate B passes:

1. Build one clean default-on product artifact with no proof switch or R114
   test exports in its public product ABI; repeat focused and strict
   correctness.
2. Run 15 exact-baseline/clean-product R107 construction pairs and then 15
   exact-baseline/clean-product native Boot/Compile/Python pairs with no
   configuration diagnostic. Charge the product construction debit to every
   candidate sample and require the same Compile target and Boot/Python
   protected rules as Gate B. This prevents the same-artifact causal harness
   from hiding main-module code, export, or default-selection cost.
3. Collect 15 alternating fresh Chromium Worker pairs with both execution-
   only and construction-to-ready Boot clocks. Boot is protected under the
   same `0.99x`/no-established-regression rule.
4. Run the R094-qualified seven-browser-by-three fixed-work WANIX guard,
   including unchanged `python /shared/bench.py`; every protected phase must
   retain at least `0.99x` paired median with no established regression.
5. Run the untouched corrected-cadence 117-trial legacy/rewrite/v86 scorecard.

Promotion requires 13/13 versus legacy, at least 11/13 versus v86, retention
of the verified Compile gain, and no protected or `python /shared/bench.py`
regression. Passing every gate promotes the unique clean artifact as the new
baseline. Stop at the first failed gate, archive immutable evidence, remove
the candidate and proof plumbing, and restore the exact control; do not tune a
second placement from the observed result.

## Result

Gate A passed. The implementation preserved each structured member's exact
next-PC SSA local across covered edges and materialized it on every external,
dynamic, safety, chain, and defensive exit. The complete candidate-on
correctness matrix passed, including 53/53 DBT units, randomized scalar/M/A/FP,
system memory/MMIO/bulk-copy, Sv39/MPRV, WFI, T2 multi-entry/atomic tests,
standalone Wasmtime modules, and direct plus OpenSBI Linux 6.12.7 boots. The
direct/OpenSBI candidate boots retired 411,039,821/423,541,174 instructions,
including 380,211,159/382,710,711 generated instructions.

The deterministic corpus contains 56 real regions, 6,258 members, and 392
modules across its seven captured modes. All 56 control/candidate structured
pairs validate with exact imports, exports, function population, calls, and
inputs. Candidate lowering converts exactly 8,542 safety `br_if` operators to
`if`/`br`/`end` paths and adds 2,052 `local.get` plus 2,052 `local.set`
operators on cold leaving paths. Aggregate structured bytes are
8,124,154/8,158,014. These byte and operator changes are diagnostics, not a
decision gate. Shape report `generated-shape.json` is
`970d6e5ebd7e...`; operator census `operator-census.tsv` is
`aea2fd484771...`.

The frozen same-artifact causal Wasm is `471a34059e4b...` (4,282,394 bytes).
Fifteen real fresh-construction pairs measured control/candidate medians of
20.539/20.424 ms and a paired candidate-minus-control median of -0.242 ms with
95% interval `[-0.610,1.432]`. R107 therefore charges the conservative upper
bound, 1.432463 ms, once to every candidate row. Construction report
`construction.json` is `c42402d12a5...`.

The immutable 90-leg native report used 15 alternating fresh pairs for each
row, CPUs 8--15, the public one-slice cadence, exact modern guest/work/output
identities, and production policy. Host-probe spread is 1.067722x; the report
is `bc4f93c1b0ed...` and the mechanical gate is `9d0884032446...`.

| Row | Debit-adjusted paired median (95%) | Adjusted normalized work | Gate |
|---|---:|---:|---|
| Boot | `0.99474x` `[0.97790,1.00430]` | `0.99471x` | protected row passes |
| Compile | `0.98579x` `[0.95202,0.99629]` | `0.98577x` | target fails all three rules |
| Python | `1.00438x` `[0.99406,1.02019]` | `1.00440x` | protected row passes |

Compile's interval upper bound is below 1.00, so this is an established
regression rather than an unresolved small benefit. Generated coverage does
not explain it: control/candidate median coverage is 0.955438/0.953634 for
Compile, 0.396831/0.396752 for Boot, and 0.978293/0.978438 for Python. Thus the
old R014 92%/95% coverage bimodality did not recur.

Gate B rejects the sole frozen placement. Per protocol there is no additional
sample, placement variant, product build, Chromium run, WANIX run, or full
scorecard. All live candidate code, test exports, and harness switches are
removed. Restored tests pass (53/53 DBT units, public API, raw Wasm smoke, T2
multi-entry, and both modern Linux firmware paths), and the active identities
are again exact CPU `aec4b31434a6...`, runtime source `1da35e70bc9c...`,
loader `2cbb264f4dac...`, and release Wasm `d9f686a9ce4f...` at 4,279,380
bytes. `target/bench/r114-lazy-internal-pc/SHA256SUMS` authenticates the
retained evidence.
