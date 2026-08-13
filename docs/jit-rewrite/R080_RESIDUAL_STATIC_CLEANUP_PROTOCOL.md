# R080 Residual Static-T0 Cleanup Protocol

Date: 2026-08-09  
Status: hypothesis, candidate, evaluator, and gates frozen before timing

## Question

Why does the R079 cleaned source build remain materially slower than exact
accepted R054? The session log gives a bounded causal answer that binary-symbol
inspection alone could not: R069 rebuilt byte-exact R054 immediately before
R070, and several R070/R072 support edits survived R079 because they did not
contain the final static scheduler's names.

The surviving production effects are:

1. `Cpu` still carries and refreshes `jit_fetch_tlb_context` on every
   `sync_jit_tlb_context` call, exposes static-core fetch and interrupt pointer
   helpers, and makes `jit_probe_fetch` publish an extra M/Bare-mode fetch row.
   These edits entered at session timestamps `04:58:15.880Z` and
   `05:57:15.808Z` and have no remaining caller after static-T0 removal.
2. `page_policy_observe` still tests a mutable fingerprint-enable global on
   every sample and retains the associated state/reset/exports. This was added
   at `06:03:30.227Z` solely to prove R072 sample-stream equivalence.
3. `user_memory_ptr`/`user_memory_len` and the loader's deterministic
   `onRandom` hook remain from R070's rejected differential harness. No
   supported or standard test path consumes them.

The independently required WFI-yield contract remains unchanged. The loader's
read-only `staticT0.supported=false` compatibility record also remains because
it has no execution-path branch and lets immutable historical reports be read.

Pre-edit identities are R079 Wasm
`e43fd0a9f02a7b21b38888f5e64aa12467db1bbf37f1ebfc0e3e4791ab62363a`,
CPU source `17c555b580f8f56c5eb9a4803222bd4111e2d54f9d01904bf40224bc364ccb5d`,
Wasm source `d66df74682bafdbe41c7dd60c85ad3b1be8cd1abaea80280b57c88e4d7c59391`,
and loader `f6a16b0274d6f097322312bf5a16604f133418dec88cf9987f50e6796f11642c`.

## Candidate and attribution

Remove exactly the residue above. Build and archive an intermediate artifact
after the core residue and another after the page-policy fingerprint removal;
these are diagnostic attribution points only. The eligible R080 candidate is
the predetermined fully cleaned build, including removal of the unused
user-memory exports and loader entropy hook. Intermediate timing cannot select
or retune the final source.

Before timing, require formatting, core/system/DBT tests, Wasm validation,
public and Worker API tests, JIT-disabled bypass, WFI lifecycle, memory/FP/A/
Sv39 differentials, and modern Linux direct/OpenSBI boots. Record exact source,
loader, artifact, guest, harness, and evaluator hashes.

## Frozen performance gate

Use exact accepted R054 as control and the fully cleaned ordinary source build
as candidate. Run five alternating fresh-process pairs for Boot, Compile, and
Python on CPUs 8-15 with the frozen Linux 6.12.7 / Alpine 3.24.1 scorecard
artifacts. Retain all 30 legs; do not retry, replace, pool with R078/R079, or
change a threshold after collection.

Every leg must prove exact inputs/outputs, generated execution, complete
phases, zero static activity, correct affinity, and the existing 1.25x sample
and host-spread limits. Restoration requires:

- candidate/control paired median at least 0.97x for Boot, Compile, and Python;
- Boot paired-bootstrap 95% lower bound at least 0.95x.

A pass establishes a source-built R054 performance envelope, not copy/v86
parity. It admits the untouched three-way scorecard. A fail keeps R054 as the
accepted artifact and moves to a separately frozen redesign of the WFI-yield
implementation; it does not authorize restoring rejected static code.

## Frozen identities

The predetermined fully cleaned source build is
`target/bench/wasm-candidates/r080-residual-clean-e5415db83b27.wasm`, SHA-256
`e5415db83b27b32a1f525af2aa19e93539332a274068e389a1e28ebba41d8095`,
4,272,559 bytes. It has exactly R054's 112 types, 13 imports, 3,704 defined
functions, 170 exports, 819 table elements, and two data segments. Its code
section is only 38 bytes larger and its name section four bytes larger; those
are the retained WFI-yield contract. R080 does not use binary equality as a
performance claim.

Final source identities are CPU
`aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`,
Wasm runtime
`b4146802f13239f3c5079d08160271173ad574889a3cd4276702db42bf80ef6c`,
and loader
`2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.
The diagnostic core-only and core-plus-fingerprint artifacts are retained as
`0d5fe534bf46...` and `f6c3021b8323...`; neither is promotion-eligible.

Frozen measurement machinery is:

- configuration A/B harness
  `74d624e8b40111cd79d4598acfc6284f8e94ab93c710194dec3e95c478196d79`;
- worker
  `cb9413c923db9d28e7fed4c1044a306d8bc09c366a6179a6e8cef42ebbaea3d9`;
- scorecard library
  `8681b09f81f3c71e30945d5770486517d993c712b2394c017b4980d481e31c61`;
- evaluator library
  `9117c0fba6c31388ec6e9dbe37e47d03346715591c6e57e02038797d726c999f`;
- evaluator entry point
  `61f6e9c53caef0bfe1d1cac1351f457d3ff05273714ae90772a284855a40bd79`;
- rejection selftest
  `2e1069507ffbbc99712edc3df55c065054a697017cdaac33d6c5334b5ea951b2`.

The selftest passed before measurement and independently rejects Boot median
or confidence inferiority, Compile inferiority, loader drift, nonzero static
activity, a missing trial, and affinity drift.

## Frozen result and promotion decision

All 30 preregistered R054-control/R080-candidate legs completed once, without
replacement. The immutable report is
`target/bench/r080-residual-cleanup/config-ab-2026-08-09T13-53-23-904Z.json`
(SHA-256
`7ce4ed478f12fa07b02410e9827efe47f3a020aea1a70a2393987987e0f44085`)
and the frozen gate result is
`target/bench/r080-residual-cleanup/gate.json` (SHA-256
`9e36141b3ea5fc6160b99137a5f73e01993ed4e868f9e10c5b2eb59acef9323c`).
Every identity, output, generated-execution, zero-static-activity, affinity,
sample-spread, and host-spread check passed; maximum host spread was 1.065x.

- Boot: R054 2,318.913 ms, R080 2,260.547 ms, paired speedup 1.033x with
  interval `[0.982,1.042]`;
- Compile: 1,070.107/1,030.149 ms, paired speedup 1.013x
  `[0.974,1.072]`; and
- Python: 3,067.136/3,005.392 ms, paired speedup 1.011x
  `[0.981,1.034]`.

The restoration gate passes. The complete strict repository matrix then
passed: 32 core, 53 DBT, and 76 system tests; QEMU differentials; 134 ISA
cases; 109 Spike locksteps; 193 architecture signatures; the complete
Wasm/JIT, WFI, Sv39, A, FP, T2, direct-Linux, OpenSBI, page-policy, AMO, and
virt-smoke matrix; public/Worker APIs; and adapter Go/Wasm tests.

The untouched authoritative three-way report is
`target/bench/r080-authoritative-three-way/scorecard-v2-2026-08-09T14-47-31-985Z.json`
(SHA-256
`09ff8ffa27640d6992500c024fccb5f6438bb84967b6e70df1381dfbec2f2378`).
It is authoritative and measurement-valid, reports no problem, and retains
all 117 trials. R080 beats the modern legacy comparator on all 13 rows and
wins or matches copy/v86 on 11 of 13. The remaining losses are Matched Boot
(2,338.368 versus 1,562.155 ms, 1.497x slower) and Compile (1,058.842 versus
718.276 ms, 1.474x slower). Python is a 1.099x rewrite win.

Finally, the rebuilt WANIX archive is
`414a174542161f9d52d6814d1deaf9fbdd56e4fa152d11fa80d7167e76a45aa5`;
its main Wasm is exact R080 `e5415db83b27...`. Five alternating fresh Chrome
150/V8 15.0 rewrite/v86 pairs on CPUs 8-15 completed without replacement.
The protocol and analysis are under
`target/jit-policy-traces/wanix-r080-e541-chrome-20260809/` with SHA-256
`50356970a8f1...` and `8cf5ca76a36e...`. Paired geometric-mean elapsed ratios
and exact bootstrap 95% intervals are Python 0.875 `[0.863,0.888]`, SHA-256
0.608 `[0.592,0.630]`, and shared 9P 0.655 `[0.551,0.747]`; lower is faster.
Exact artifact/root hashes, outputs, guest identity, and required generated
coverage all pass.

Promote R080 as the clean, source-built baseline. It restores ordinary source
reproducibility and preserves the independent WFI fix, but it does not complete
the parity objective: the standing result remains 11/13 versus copy/v86, with
Boot and Compile open.
