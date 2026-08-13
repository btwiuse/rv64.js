# R076 Candidate-v86 WANIX Guard Protocol

Date: 2026-08-09  
Status: correctness gate passed; implementation, identities, result path, and
performance gates frozen before formal timing

## Purpose

R076's execution-only modern Boot A/B passed its prospective cumulative-gain
gate at 1.175x `[1.167,1.189]`, with normalized MIPS 1.174x
`[1.167,1.189]`. Its protocol requires a fresh candidate-versus-copy/v86
WANIX guard before default promotion. This experiment is that Gate C guard; it
does not change or rescore the R075 launch-to-shell rejection or reuse R075's
elapsed samples.

The guard answers one question: do the exact R076/R075 candidate bytes remain
noninferior to copy/v86 on every `/shared/bench.py` phase when both use matched
WANIX roots? It is not a Boot measurement and its 10% product-noninferiority
margin is not an optimization-acceptance threshold.

## Frozen artifacts and execution contract

Every leg uses the immutable comparison page:

`http://127.0.0.1:8765/examples/v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html`

Its SHA-256 is
`7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413`
and its size is 12,045 bytes. The page selects
`rv64.static-t0=sampled-backoff` before the RV64 adapter creates the VM.

The other frozen archives are:

- RV64 candidate archive: SHA-256
  `e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c`,
  1,831,853 bytes, containing main Wasm
  `28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c`;
- copy/v86 archive: SHA-256
  `7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771`,
  1,680,818 bytes;
- matched RISC-V root: SHA-256
  `274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb`,
  28,069,154 bytes;
- matched i686 root: SHA-256
  `09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320`,
  28,785,067 bytes.

Both roots identify as Alpine 3.22.5 with Python 3.12.13 and run the same
architecture-independent `/shared/bench.py`. This WANIX guard intentionally
uses those already-matched roots. The authoritative promotion scorecard remains
the Linux 6.12.7 / Alpine 3.24.1 test required by the thread goal.

The candidate must prove at shell and in every repetition that residual static
T0 is disabled, sampled static T0 and short-sample backoff are enabled, the
equal auxiliary module exists, sampled retirement/samples/polls/marks/bypasses
are nonzero, generated execution is active, production page-policy constants
are unchanged, and all static errors are zero. The v86 leg must expose complete
external 9P accounting. Static inspection of the exact v86 archive must prove
the JIT configuration/cache exports, conditional-only disable path, and
generated-module instantiation path remain present. This WANIX wrapper does not
expose a trustworthy runtime dispatch counter, so the later authoritative
scorecard—not this guard—must again prove JIT enabled, generated modules
installed, and generated dispatch actually reached. Promotion cannot complete
without that independent runtime proof.

## Correctness and harness gate

Before formal collection:

1. source/synthetic selftests must enforce the immutable URL, artifacts,
   candidate preboot lifecycle, exact browser, CPUs, pair order, phase set,
   repetition count, correctness markers, JIT proof, and statistical rule;
2. the existing R075 candidate smoke and v86 matched-guest evidence may be used
   as semantic evidence, but no elapsed value may be pooled into this guard;
3. all syntax/selftests must pass and runner/analyzer/verifier hashes must be
   recorded before formal collection; and
4. no result directory or benchmark lock may preexist.

Any correctness repair before collection adds a directed regression. It may
not change candidate bytes, page, guest roots, phases, repetitions, sample
count, ordering, or thresholds.

## Prospective measurement and decision rule

Run seven alternating fresh Chrome process/profile/VM/guest pairs on CPUs
`8-15`, RV64 first in odd pairs and v86 first in even pairs. Each guest performs
three phase-synchronized repetitions of, in order, Python, SHA-256, and shared
9P. No leg may be replaced. The per-leg observation is the median of its three
repetitions; the paired ratio is RV64/v86 elapsed time.

For each phase, both conditions must hold:

- paired median RV64/v86 ratio at most 1.10; and
- exact paired-bootstrap 95% upper bound at most 1.10.

Every correctness, identity, lifecycle, generated-code, chronology, or
artifact invariant must also pass. There is no within-side spread cap; raw
values remain visible. After a complete valid result there is no retry,
outlier deletion, alternate statistic, threshold adjustment, or pooling with
R075.

The immutable result directory is:

`target/jit-policy-traces/r076-wanix-v86-28ceaf7b-chrome150-20260809`

Only a passing result permits default promotion, rebuild, the complete strict
correctness matrix, and the untouched authoritative three-way scorecard.

## Correctness result and frozen tool identities

The fixed-mode source/artifact selftest passes. It proves the wrapper admits
only one result-directory argument; the shared runner selects seven pairs,
three repetitions, the immutable candidate URL and configuration, and preboot
configuration only for the RV64 leg. It rehashes the page and all four
archives, extracts and rehashes the main RV64 Wasm, verifies copy/v86's
`get_jit_config`, `set_jit_config`, `jit_get_cache_size`, and
`codegen_finalize_finished` exports, verifies its conditional-only disable and
generated-module instantiation paths, and confirms the server returns the
frozen page.

The synthetic analyzer/verifier selftest passes and independently rejects a
1.11 ratio, disabled preboot lifecycle, incomplete repetition set, and mixed
artifact. The historical schema-2 analyzer selftest still passes after the
backward-compatible schema-3 extension.

Frozen tool identities are:

- shared runner: `8d37e7b20186253a0b7e71e5b7c28f3d8ee3b34a49a7eb4374553c5b80ee4e80`;
- fixed runner wrapper: `15cf55e647c285558046367fcff4f6a083cd7b3c47de540585f41b45d84b4220`;
- shared analyzer: `fc28c39c6a1fd73905595230908cdbb366df211dc2b7eddb685e8fc770f2949e`;
- fixed analyzer wrapper: `1093c816e76cbdca54c34154bb5d97f0628c905a4b436cadab527962c35bb427`;
- analyzer/verifier selftest: `2bb1c093d6a192910c06a7500de07a0d397e07918a624e5f6f2994c270217cf7`;
- source/artifact selftest: `355a8622ff69b4a7ca03146d7220c38ae2f48fd24f0970a4b73b7824853a8b18`;
- final verifier: `cc61995a097c3033affe558f87dc825578e42a85970493bffb054a6537093ab5`;
- immutable WANIX harness: `525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545`.

Formal collection uses exact Chrome 150.0.7871.186, revision
`@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d`, V8 15.0.245.21, and CPUs
`8-15`. No formal elapsed value existed when these identities were recorded.

## Pre-analysis 9P snapshot correction

All fourteen formal legs completed, but the frozen analyzer stopped before
creating `analysis.json` or exposing any elapsed sample. Its first failure was
an implementation assertion requiring every phase-end external-9P snapshot to
have equal request/reply counts and zero pending requests. Pair 3 x86
`shared9p1` recorded 2,152 requests, 2,151 replies, and one pending request.
Inspection was restricted to 9P accounting fields in all seven x86 logs. The
pair-3 whole-leg record is 14,398 requests, 14,398 replies, zero pending, and
zero tag collisions; every other whole leg is likewise exactly balanced. The
pending reply crossed the instantaneous phase snapshot and completed before
the later whole-leg snapshot.

This is an analyzer-contract defect, not a failed workload or missing reply.
Before reading any elapsed values, the semantic rule is corrected narrowly:

- every whole x86 leg must still have equal requests/replies, zero pending, and
  zero tag collisions;
- every shared-9P phase must have nonzero requests, zero tag collisions,
  replies no greater than requests, `requests - replies == pending`, and at
  most one pending request at its instantaneous end snapshot; and
- all existing guest correctness, repetition, artifact, chronology, candidate
  lifecycle, performance thresholds, raw legs, and the no-retry rule remain
  unchanged.

A synthetic regression must exercise both the valid one-reply boundary case
and rejection of a larger/inconsistent pending count. The corrected analyzer,
selftest, and verifier receive new hashes before analysis; the already-frozen
runner, wrapper, harness, artifacts, protocol JSON, and collected legs do not
change. No leg is replaced and no timing is recollected.

The corrected semantic tests pass before elapsed analysis. Corrected shared
analyzer SHA-256 is
`8d243a2e5433e25f77eef88a33b9fe58865cf90a86982c52b0c7f8c61a431c60`;
corrected analyzer/verifier selftest SHA-256 is
`0265c30816ee67054115b9a0bf070f88a1d73e4f551c59fdc5030bef401b345c`.
The fixed analyzer wrapper remains
`1093c816e76cbdca54c34154bb5d97f0628c905a4b436cadab527962c35bb427`
and the verifier remains
`cc61995a097c3033affe558f87dc825578e42a85970493bffb054a6537093ab5`.

## Formal result

The corrected semantic analyzer and unchanged performance verifier both pass
on the original fourteen legs; no leg was replaced or recollected. Protocol
SHA-256 is
`0bbf0988fb190de585687357fea4160df2ee3b83ae790f5dcf6796b8e78871ec`;
analysis SHA-256 is
`41583f18dacc57a9c70a51ecd32194a67b6fa3f2156ffbce133b869dd51cc400`.
All checksums, repetitions, artifacts, browser identities, candidate lifecycle
proofs, zero-error checks, whole-leg 9P balances, and chronology pass.

Paired RV64/v86 medians and exact 95% intervals are:

- Python: 0.891 `[0.876,0.924]`, side medians 1.744/1.960 s;
- SHA-256: 0.632 `[0.614,0.638]`, side medians 2.870/4.587 s;
- shared 9P: 0.669 `[0.577,0.830]`, side medians 0.681/1.008 s.

Every upper bound is below 1.10. The fresh candidate-v86 product guard passes
and admits default promotion, the complete strict correctness matrix, and the
authoritative three-way scorecard.
