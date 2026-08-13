# R096 Optional Tail-Chain Accounting Protocol

Date: 2026-08-10  
Status: rejected at the frozen same-artifact causal gate; implementation
removed and accepted R085 behavior restored

## Question

Does the exact diagnostic increment currently emitted before every successful
cross-module structured-region tail transfer impose a reproducible cost on the
modern Compile row?

This is a general generated-code cleanup, not a workload specialization. Every
successful transfer currently executes the same `i64.load`, add, and
`i64.store` against one runtime counter. The counter is observational only: it
does not participate in architectural state, mapping validation, dispatch,
fuel, interrupts, or JIT policy. Existing exact-R085 evidence records roughly
8.1--8.3 million transfers in a Compile phase and roughly 37.3 million in a
Python phase. Boot records about 0.1 million. Those counts establish frequency,
not a performance result.

No byte-count gate applies. Main-module and generated-module section sizes are
recorded for attribution only. A size change matters only if measured cold
construction, browser startup, or end-to-end execution regresses.

## Frozen mechanism

1. Add one emission-time boolean controlling only the tail-transfer diagnostic
   increment. The transfer guards, dispatch-line proof, index masking, and
   `return_call` remain byte-for-byte identical between modes.
2. Read the boolean while generating a module. Do not add a runtime branch to
   a generated tail-transfer site.
3. Include the boolean in the DBT emission-configuration signature so a page
   template generated in one mode cannot be reused in the other.
4. Expose one diagnostic setter. The same main Wasm artifact must support both
   A/B legs; the worker sets the mode before any guest module is generated.
5. Preserve exact accounting as an opt-in diagnostic. Production may default
   it off only after the causal and product gates pass.
6. Do not change region formation, thresholds, page policy, tail-chain
   eligibility, scheduler cadence, guest code, or any benchmark input.

## Correctness and shape gate

Before performance timing:

- prove modules in both modes validate and differ only by the counter sequence
  and the expected configuration identity;
- execute the directed two-module tail cycle in both modes, requiring identical
  GPRs, PC, retirement, and fuel behavior; accounting-on must report nineteen
  transfers and accounting-off must report zero;
- retain the table-independent single-trampoline import shape;
- pass DBT/Rust units and the focused generated/interpreter, memory, FP, atomic,
  Sv39, WFI, lifecycle, public, and Worker differentials; and
- boot modern Linux 6.12.7 through direct SBI and OpenSBI before promotion.

## Same-artifact causal gate

Run seven alternating fresh-process pairs for Boot, Compile, and Python on CPUs
8--15 with the modern Linux 6.12.7 / Alpine 3.24.1 guest and public one-slice
scheduler cadence. Both legs use one exact Wasm artifact. Control explicitly
enables accounting; candidate explicitly disables it before compilation.

The report must be measurement-valid, use identical input hashes and output
fingerprints, prove generated execution and tail-chain enablement, and have a
host-probe spread at most 1.10x. Advance when:

- Compile's paired candidate-speedup median is greater than 1.0 and its exact
  paired 95% median interval lower bound is at least 1.0;
- Boot and Python candidate/control elapsed medians are each at most 1.02x; and
- no architectural, generated-retirement, dispatch, policy, module-lifecycle,
  or workload-output invariant differs beyond ordinary nondeterministic JIT
  landing counts.

There is deliberately no fixed 3%, 5%, 10%, 20%, or code-size reward gate for
this removal. Statistical separation from no improvement supplies the benefit
criterion; the implementation removes hot work and retains the diagnostic
behind one generic switch. If the confidence interval includes regression, the
mechanism is not demonstrated and is removed without threshold or workload
variants.

## Product and promotion gates

If the causal gate passes, make accounting off the ordinary production mode
and compare that artifact with immutable R085 in five alternating fresh
Boot/Compile/Python pairs. Require the causal Compile direction to persist with
the same non-regression bounds. Then run fresh Chrome Boot, the exact R094
32 MiB WANIX shared-9P guard, Python and SHA-256 guards, and the complete
untouched 117-trial three-way scorecard.

Promotion requires complete correctness, valid browser/product reports, no
protected-row regression, no scorecard row more than 3% slower than R085, and
either an improved v86 parity count or a reproducible reduction of an open-row
gap. A partial Compile gain does not redefine the thread objective: Boot and
Compile parity with copy/v86 remain required.

On failure, preserve exact source/artifact/report hashes, remove the candidate
switch and harness field, and restore the accepted R085 executable behavior.

## Result and removal

The implementation passed its exact shape test: after deleting the one
six-operator counter sequence, accounted and unaccounted generated functions
had identical complete operator streams. Both modes validated and the directed
two-module cycle produced identical GPR, PC, retirement, and fuel results with
nineteen versus zero recorded hops. DBT/workspace units; public and Worker
APIs; interpreter/generated, memory, FP, atomic, Sv39, WFI, and lifecycle
differentials; and modern direct/OpenSBI Linux boots all passed. The candidate
artifact was 4,279,671 bytes at `05daf545b519...`.

The one allowed seven-pair run was valid with host spread 1.022x. Results were:

- Boot: 2,196.248 ms accounted versus 2,176.976 ms unaccounted; paired 1.005x,
  interval `[0.999,1.009]`;
- Compile: 948.163 ms versus 960.465 ms; paired 0.991x,
  interval `[0.959,1.028]`; and
- Python: 2,365.151 ms versus 2,368.111 ms; paired 0.994x,
  interval `[0.973,1.031]`.

The Compile point estimate is unfavorable and its interval does not establish
an improvement, so the frozen gate rejects R096. No product, browser, WANIX,
or scorecard run follows, and no sampling/counter variant is permitted. The
exact report is `058b6183c223...`, gate `a669dc9db3b...`, candidate artifact
`05daf545b519...`, and source archive `46da89385883...`, all under
`target/bench/r096-tail-chain-accounting/`.

The emission switch, configuration-signature bit, exported setter/stat,
worker field, public diagnostic, directed candidate assertions, and generated
conditional are removed. Historical protocol/gate code and immutable evidence
remain. The product path once again always emits the accepted exact hop counter
sequence; no active runtime or harness selector remains.
