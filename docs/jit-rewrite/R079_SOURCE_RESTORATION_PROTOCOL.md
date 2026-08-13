# R079 Source-Built R054 Restoration Protocol

Date: 2026-08-09  
Status: protocol, artifacts, evaluator, and thresholds frozen before collection

## Question

R078 proved that exact accepted R054 is 1.178x faster on Boot than the
post-R077 default-off artifact, while also improving Compile and Python by
1.037x and 1.031x. The causal source audit found the complete rejected
R070-R074 static-T0 decoder and sampled/backoff machinery still linked into
the production main Wasm and selected through branches in the page-policy hot
loop even when every runtime flag was false.

R079 tests the required source repair. It does not install the archived R054
binary. The candidate removes the static-T0 emitter, auxiliary-module
lifecycle, writable controls, counters, machine hooks, backoff set, and hot
branches, while preserving the independently accepted WFI-yield contract from
D047. Stable APIs reject the removed write controls and retain only an
explicit `supported: false`, all-zero read-only compatibility record for old
report readers.

The source-built candidate is 4,270,092 bytes, 2,425 bytes smaller than exact
R054. Its WebAssembly import set is identical to R054's. Its export set is an
exact superset containing only four later, non-static diagnostics:
`jit_page_policy_fingerprint`, `jit_set_page_policy_fingerprint`,
`user_memory_len`, and `user_memory_ptr`.

## Frozen comparison

Run five alternating fresh-process pairs for Boot, Compile, and Python on
CPUs 8-15 with the scorecard's Linux 6.12.7 / Alpine 3.24.1 artifacts, normal
production page policy, timer/cadence, outputs, and host probes.

- control: exact accepted R054
  `4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`;
- candidate: source-built cleaned Wasm
  `e43fd0a9f02a7b21b38888f5e64aa12467db1bbf37f1ebfc0e3e4791ab62363a`;
- loader:
  `f6a16b0274d6f097322312bf5a16604f133418dec88cf9987f50e6796f11642c`.

The control alone uses `SCORECARD_V2_REWRITE_WASM` to select the archived
artifact. The candidate is the ordinary build output and is therefore the
measurement-eligible side. Retain all 30 legs. There is no retry, outlier
removal, row replacement, threshold adjustment, or pooling with R078.

## Frozen gates

Every leg must prove exact artifact/loader/guest/input/output identity,
generated-code execution, complete phases, zero static-T0 activity, and the
unchanged 1.25x sample and host spread limits. Performance requires:

- candidate/control paired median at least 0.97x on Boot, Compile, and Python;
- Boot's exact paired-bootstrap 95% lower bound at least 0.95x.

A pass proves source-built restoration to the accepted R054 performance
envelope and admits the complete correctness matrix followed by the untouched
authoritative three-way scorecard. It does not itself claim copy/v86 parity.
A fail rejects this source repair and requires profiling the cleaned artifact
before any production or scorecard promotion.

Frozen harness identities:

- A/B harness `74d624e8b40111cd79d4598acfc6284f8e94ab93c710194dec3e95c478196d79`;
- worker `cb9413c923db9d28e7fed4c1044a306d8bc09c366a6179a6e8cef42ebbaea3d9`;
- scorecard library `8681b09f81f3c71e30945d5770486517d993c712b2394c017b4980d481e31c61`;
- evaluator `76476b1a772d85d92fc30e0fc5f99db7c23724c9b32dbccaeb51101de32786bb`;
- gate `224bc8b282c6c0bc52a491662de956559bf71677a88b8232abf807c104042f25`;
- rejection selftest `8ac4681543c763220f6d9a6ff3c1250eb3460c39ce4082cd2686b743cd20879f`.

The selftest accepts one valid synthetic matrix and independently rejects
Boot median inferiority, Boot confidence inferiority, a Compile guard loss,
loader identity drift, nonzero static activity, a missing trial, and CPU
affinity drift. It passed before collection. The removed experimental source
is preserved only as a non-production archive at
`target/jit-policy-traces/r078-pre-removal-source/static-t0-runtime-source.tgz`
with SHA-256
`38050d3694737e166d3f47db92a5117df1b2ef72fcd0d61bd07d0109cd2c5153`.

## Frozen result

All 30 preregistered legs completed without replacement. The immutable report
is
`target/bench/r079-source-restoration/config-ab-2026-08-09T13-30-39-529Z.json`
(SHA-256
`6a26e3ebc0ec68264a037c1895cb43468bce9112a76638d2c846ae0d463938df`),
and the frozen evaluator result is
`target/bench/r079-source-restoration/gate.json` (SHA-256
`809d45a15874a27963d16027077de8139885195cbdc67807dd7d2e68a29541c5`).
Artifact, loader, guest, output, generated-execution, zero-static-activity,
affinity, and spread checks pass. Performance does not:

- Boot: R054 2,314.859 ms, candidate 2,737.942 ms, paired 0.845x with
  interval `[0.809,0.885]`;
- Compile: 1,056.351/1,102.979 ms, paired 0.952x `[0.918,1.024]`;
- Python: 3,037.438/3,131.019 ms, paired 0.968x `[0.949,1.026]`.

R079 is rejected. Removing the large static emitter/runtime was necessary
cleanup but did not restore the source build to R054. A subsequent exact
session-history audit found smaller R070/R072 residues that this candidate had
incorrectly classified as independent diagnostics: a fetch-context update on
every JIT entry, static-core interrupt helpers, an altered M/Bare fetch probe,
two user-memory exports, and an ordered-fingerprint branch in the page-policy
hot loop. R080 removes that complete residue set prospectively.
