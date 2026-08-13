# R084 Closure-Aware R080 Residual Audit

Date: 2026-08-09  
Status: diagnostic protocol frozen before residual classification

## Question

After removing the rejected R082 external compiler and R083 full-system
specialization, does exact R080 still contain one architecture-general dynamic
operation with enough untested end-to-end leverage for the standing cumulative
3% track? The audit must distinguish a real residual from samples belonging to
an already-tested mechanism. It may admit one implementation, but it is not a
performance result itself.

The retained production identities are:

- runtime Wasm
  `e5415db83b27b32a1f525af2aa19e93539332a274068e389a1e28ebba41d8095`;
- loader
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- browser archive
  `414a174542161f9d52d6814d1deaf9fbdd56e4fa152d11fa80d7167e76a45aa5`.

The objective remains the untouched modern Linux 6.12.7 / Alpine 3.24.1
three-way scorecard. R080 is 13/13 versus modern legacy and 11/13 versus
copy/v86; Boot and Compile are the only open rows. `/shared/bench.py` remains a
hard non-regression guard.

## Frozen evidence

Use R081's already-collected proof-only profiles. Do not recollect or use their
inspector-perturbed wall times as performance measurements:

- analysis `target/bench/r081-r080-engine-profile/profile-analysis.json`,
  SHA-256 `ed942beb6e6d4da111510fcbb2d99e54d0a638bb07b5e23a627ec1cd7e489d13`;
- Boot profile `rewrite-boot-first.cpuprofile`,
  SHA-256 `ca4ea0c1d60a23be6031dcaaf4cf5e703034229ecfd4c69afa49dd5b059b3081`;
- Compile FIRST profile, SHA-256
  `3e4fbdef64b5e3613f8166853118b2482271b13e92a9d2822f8d59e2365cefa3`;
- Compile PRIME profile, SHA-256
  `a0cf7903d620563fb95b9182ccb7995bbf44b14f7e47fb6bf8c16ff8ab75b249`;
- Compile STEADY profile, SHA-256
  `d030935da62483d6a12bcbaa7fd1ebe36e027341583fe811fab7e84e8af6fc7b`.

R081's scheduler component table labels only the immediate child below
`run_system_jit`. That is correct for subtree ownership but is not a complete
operation inventory: for example, hashing reached through page-policy helpers
is owned by those helper subtrees. R084 therefore reconstructs every sampled
stack and attributes leaf self time to complete operation families as well as
to the first caller below the scheduler.

## Classification method

For Boot and Compile STEADY, classify every runtime-Wasm self sample in
descending order until at least 95% of the complete phase is explained. For
Compile FIRST and PRIME, repeat the classification as a lifecycle cross-check.
For each family report:

1. exclusive sampled microseconds and fraction of the complete phase;
2. the first caller below `run_system_jit` and the nearest non-library caller;
3. whether the mechanism was already tested, with its experiment identifier;
4. the optimistic Amdahl ceiling if the operation were free;
5. a realistic projection based only on a selector-free local measurement or
   an already-preserved same-mechanism result.

Do not add unrelated leaf frames to manufacture a larger opportunity. A
candidate may combine multiple costs only when one representation necessarily
removes them together, such as a specialized state table replacing both its
hash computation and probe. Guest PCs, symbols, binaries, workloads, URLs,
checksums, browser identity, compiler output, and scorecard row may diagnose
concentration but may not select behavior.

The closure ledger includes, at minimum: cache-capacity and entry ranking;
module geometry, thresholds, synchronous tiny tiers and compiler workers;
fetch caching and helper partitioning; decoder inlining/local-state scalar
drivers; blind scheduler cadence/re-entry thinning; structured local
allocation and fuel layouts; existing generated-memory proof packing/SIMD;
stack-root proof carry; static/sampled Tier-0 and its lifecycle variants; and
R083's `Option<SysCsrs>` specialization. A previously tested mechanism is not
reopened merely because its samples remain visible. A materially changed
retention policy may justify an independent confirmation only when the old
candidate measured a general 3% or larger target-row gain and was rejected
solely by the superseded 10% advancement rule, not by regression or a failed
product guard.

## Admission rule

Admit at most one next implementation from this audit. It must satisfy all of:

- architecture-general semantics and no forbidden selector;
- a distinct causal mechanism, or an explicitly identified independent
  confirmation allowed by the final closure-ledger rule above;
- an optimistic whole-row ceiling above 3%;
- a realistic projected target-row improvement of at least 3%;
- no projected Python regression and no new execution tier/lifecycle unless
  the realistic projection is at least 5%.

Before editing product code, append the selected mechanism, exact semantics,
correctness matrix, measurement order, stopping rules, and restoration path to
this document or freeze a separate R085 protocol. If no mechanism qualifies,
record a measured local-architecture plateau and move to a larger execution
representation rather than varying a closed threshold or helper boundary.

## Performance path for an admitted candidate

The default cumulative path is:

1. directed semantics and complete relevant differentials;
2. release construction, Wasm validation, public/Worker API checks, and the
   complete strict repository matrix;
3. seven alternating fresh-process cold `WebAssembly.compile` trials, with no
   greater than 5% candidate median regression;
4. five alternating fresh R080/candidate native pairs for Boot, Compile, and
   Python, exact modern inputs/outputs and host-spread checks;
5. retain only with at least 1.03x paired target-row speedup, a bootstrap
   interval that excludes a material regression, and non-target medians no
   worse than 1.03x elapsed;
6. immutable Chrome candidate/control `/shared/bench.py`, SHA-256, shared-9P,
   and execution-Boot guards;
7. only then run the untouched 117-trial legacy/rewrite/v86 scorecard and
   promote if it improves the fixed parity count without a guarded regression.

Every stopped candidate is archived with source, artifact, report, and reason,
then removed completely. Rebuilding must restore the exact R080 identities
above.

## Result

The corrected complete-operation accounting finds one previously untested
general residual. Rust `RandomState` hashing owns 117.512 ms, or 5.062% of
R080 Boot; distinct `hashbrown` probe self time owns another 35.380 ms (1.524%).
The same exclusive categories own 59.377/12.838 ms, or 3.619%/0.783%, of
Compile STEADY. Lifecycle consistency is strong: hashing alone owns 13.436%
of Compile FIRST and 5.354% of PRIME. Most Boot samples are tuple-u64 page
policy state lookups reached through `run_slice_sampled_until`,
`page_policy_observe`, and scheduler cache probes. Small DBT-internal maps are
visible but are not needed to meet the admission bound.

This is not an alias of the direct-mapped dispatch cache: that cache already
avoids a `HashMap` on every generated dispatch. The residual consists of the
many architecture-general policy/lifecycle maps keyed by virtual/physical page,
address space, PC, or function-table index. No earlier experiment changed their
hash algorithm as one coherent representation.

The proof-only local corpus is
`target/bench/r084-hash-opportunity/opportunity.json`, SHA-256
`c49602b9bfa02fa1faaccd5090eb619648c9f0bea3dc5fa5550a9d8c8ef24dd3`.
It compiles the candidate and `RandomState` forms into the same deterministic
125,098-byte Wasm (`fa00c927ded0...`) and executes a frozen mix of tuple-u64
hashes plus six representative map/set operations. Seven alternating fresh
Node 26.5.0/V8 14.6 processes measured:

- hashing: 5.508x paired median, 95% median interval `[5.498,5.510]`;
- complete state-map mix: 3.021x, interval `[2.989,3.023]`;
- exact map-state equivalence in every correctness and timed sample.

Applying only the hash speedup to exclusive hash self time, and crediting none
of the table-probe improvement, projects 1.043x Boot, 1.124x Compile FIRST,
1.046x PRIME, and 1.031x STEADY. This clears the 3% cumulative floor in both
open target rows without combining unrelated leaves. The map-family projection
is similar (1.046x Boot and 1.030x STEADY), so the decision does not depend on
optimistically eliminating all probe work.

R084 therefore admits one product candidate: replace only rv64-wasm JIT
policy/lifecycle integer-key `HashMap`/`HashSet` builders with the measured
seeded avalanche hasher. R085 freezes the implementation and gates. All other
residual families remain closed.
