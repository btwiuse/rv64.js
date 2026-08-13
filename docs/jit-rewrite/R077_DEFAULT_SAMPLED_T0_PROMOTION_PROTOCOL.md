# R077 Default Sampled-T0 Promotion Protocol

Date: 2026-08-09  
Status: rejected at frozen authoritative promotion gate; default restored off

## Admitted evidence

R076 measured the exact sampled-static plus short-sample-backoff mechanism at
the authoritative Chrome modern-Boot boundary: 1.175x execution speedup
`[1.167,1.189]` and 1.174x normalized-MIPS gain `[1.167,1.189]`, with nearly
identical guest instruction counts and zero errors. Its fresh seven-pair,
three-repetition candidate-v86 WANIX guard passes Python, SHA-256, and shared
9P at 0.891, 0.632, and 0.669 paired elapsed ratios, with every 95% upper bound
below 0.93. This admits production promotion under R076 Gate C.

## Production integration

The mechanism and main Wasm remain unchanged. Promotion centralizes one
guest-independent production helper in `web/rv64.js`. After any full-system
machine is assembled, but before its first guest instruction, the helper:

1. calls `jit_static_t0_system_prepare` and requires one valid auxiliary-module
   table index;
2. sets residual static T0 off;
3. sets sampled static T0 on; and
4. sets short-sample backoff on.

The stable local API invokes this helper automatically for direct Linux,
OpenSBI/virt, and legacy full-system boots. Reset reconstructs the pointer-bound
module and reapplies the production default. Bare-metal mode remains unchanged
because it has no full-system page-policy sampling path. Explicit
`configureJit` calls remain authoritative and can disable or alter the three
switches after creation.

The authoritative scorecard's rewrite worker invokes the same helper after its
low-level full-system boot and treats it as production, not a diagnostic. Its
default result must prove exactly one prepared module, residual off, sampled
and backoff on, sampled retirement/marks/bypasses nonzero, and zero errors.
Explicit diagnostic environment overrides remain available for controlled
historical comparisons but make a run measurement-ineligible as before.

The WANIX adapter needs no benchmark flag for the new default. Its existing
`rv64.static-t0=control|sampled-backoff` option remains an explicit diagnostic
override; the no-option path must expose the sampled/backoff production state
at shell.

This integration has no guest PC, symbol, opcode list, binary checksum,
compiler, benchmark, or browser selector.

## Frozen production identities

The production integration and its complete correctness gate are now fixed at:

- main Wasm `28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c`;
- production loader `d949d8641dd4048ed031c7293ddf9d7b7c911dbc89aa9fa0c29487c21687718b`;
- WANIX adapter `ad765bf6d618baf67b9c363b2ac0f84979fada0430fca3da389386dc10d315e0`;
- rebuilt `rv64-jit.tgz`
  `9f28e71af658fef6a32da9c3682f7a8b4a34c83049515dd44fd6df756ab1ead6`;
- no-override WANIX page
  `9199655ff6df7eb5f7d6077c2f85bf8a327a885fcdc270130ec57285145e4dd8`;
- final-smoke wrapper/harness
  `058188b28e86ffdceb4bf919423651eebadbfe4b3bcaf3d11237c1e8eb1357a7` /
  `525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545`;
- matched WANIX root
  `274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb`;
- scorecard worker/main/library `52ea4d91a17428f9ba851d54743922e01c09e36e8ad81722ed23e33e3f94ff57` /
  `3c55f6e34c8b41e4bb2d877f9a8a5303a40cce5dfa7eb01a2a49f552d7e28608` /
  `3ec440d9e86f5ecf6e524dcc6f0d0edc7899a2892106b76cb945c7afca9f14de`.

The R077 Chrome confirmation uses seven alternating fresh
process/profile/module-Worker/guest pairs on CPUs 8-15 and the exact R076
timer boundary. Both legs use the same production loader, main Wasm, helper,
and one untimed auxiliary-module registration. The control then disables
sampled/backoff execution before the initial counter snapshot; production
leaves the helper's default untouched. Frozen identities are:

- page `23b74fad059e2a9bbf0bce452bb1a5298af444d886f5fb92738318644133a0c5`;
- Worker `a9ea4021697a29ea771d3e9d57c3439e0e9289f9a8d25df8f6abfbf83c40d45b`;
- timing library `05edc84de1e4b8df83b6b3ea0ba8474230f0f8cb61d2c0a0807392c3a42d4d57`;
- host `c533f528d69d7f43075e84ced676fdc8491ee29c90de16ebe2872a9657f60843`;
- harness selftest `a89b42c94fcb0eeb4408eeb97277009935a7ed4e3f6d82a3f306010ba3b1f18e`;
- runner `952e122a1fcee3865bf144cb7730373ccf5927427bf1b3bbe47e0e61f750ebdc`;
- analyzer `b5ec4b92bd85dd0cb7ae2b6f224e522679a20bb4cb78b51a28f635e00af3ba77`;
- analyzer selftest `a81a4916eff56f58f308150016036028cf4da6791c20f5aba4bed355d67bf4c6`;
- independent verifier `b7c8def33c421ad4d91ae43e2ed5150c46eadf39ca5c921f685be011dd5a6134`;
- Chrome `150.0.7871.186`, revision
  `@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d`, V8 `15.0.245.21`;
- result directory
  `target/jit-policy-traces/r077-production-default-28ceaf7b-chrome150-20260809-config-ab`.

The correctness-only final WANIX smoke writes exactly one result to
`target/jit-policy-traces/r077-wanix-default-smoke-9f28e71a-chrome150-20260809.json`.
It runs one fresh Chrome/Worker/guest on CPUs 8-15, uses no adapter or runtime
JIT configuration override, requires the exact page/archive/root identities,
and validates sampled/backoff state both at shell and during one correct Python
phase. Its elapsed fields cannot be used as promotion performance evidence.

The first correctness-only invocation completed the guest but the wrapper
looked for `python1`, the three-repetition key used by R076, while the frozen
WANIX harness deliberately calls a single repetition `python`. It failed
before writing the result path. Before rerun, the wrapper was corrected to the
source-defined single-repetition key and now asserts the harness's exact naming
expression; its updated hash is the one frozen above. No runtime, artifact,
workload, threshold, or timing rule changed, and neither invocation is
performance evidence.

Before these identities were frozen, an unpinned smoke exposed that the copied
CDP collector retried a completed Worker error until timeout. R077 now
propagates completed errors immediately and has a synthetic regression that
requires one poll. The same smoke also removed an invalid attempt to read
stable-API configuration fields from the low-level debug counter object. No
formal pair or protocol directory existed, and no smoke timing entered any
decision. Corrected control/production lifecycle smokes then passed with
180.35M/180.30M instructions, zero control activity, and production activity
of 93.54M sampled instructions, 328 marks, 279,339 bypasses, and zero errors.

## Correctness gates

Before performance:

1. directed stable-API and Worker tests prove the default immediately after
   creation, explicit disable, reset reconstruction, and one-module lifecycle;
2. adapter tests prove no-option production default and explicit control/
   candidate overrides without leaking adapter-only arguments into Linux;
3. the static-T0 randomized/system differential and lifecycle suites pass;
4. the complete strict repository matrix passes, including all core/DBT/system
   tests, ISA/Spike/signature suites, Wasm/API/Worker gates, T2/atomic tests,
   direct Linux, and OpenSBI virt; and
5. the default-on artifact is rebuilt and all loader/Wasm/archive identities
   are recorded.

All five correctness requirements pass. The stable and Worker APIs prove
creation/disable/reset behavior and one registration per machine assembly;
the adapter's Go/Wasm tests pass; the directed sampled-static, page-policy,
WFI, generated-entry, dirty/reset, and T2 lifecycle tests pass; explicit
sampled/backoff direct/OpenSBI Linux boots retire 50.58M/58.14M sampled
instructions with 249/258 marks, 558,320/509,613 bypasses, and zero errors;
and `nix develop -c env REQUIRE_ALL=1 tests/run-all.sh` passes all eight stages,
including 134 ISA tests, 109 Spike locksteps, 193 architecture signatures,
the complete Wasm/system matrix, and modern virt smoke.

Any correctness defect is fixed generally with a directed regression before
timing. No performance threshold, guest, workload, or sample count changes.

## Performance and promotion gates

After correctness, run a same-artifact production-default confirmation at the
exact R076 Chrome execution-only Boot boundary. It must retain at least a 1.05x
speedup over R054-equivalent control with a paired 95% lower bound at least
1.00 and no normalized-MIPS regression. Then run the untouched authoritative
13-row rewrite/legacy/v86 scorecard with three alternating fresh processes per
side on Linux 6.12.7 / Alpine 3.24.1.

Promotion requires:

- at least 11/13 versus copy/v86 and 13/13 versus legacy;
- no rewrite row more than 5% slower than R054;
- raw Boot at least 5% faster than R054's 2,260.5 ms;
- exact outputs, production-policy/default proof, generated execution, copy/v86
  enabled/generated-dispatch proof, artifact identities, and valid host spread;
- the already-passing fresh R076 candidate-v86 `/shared/bench.py` guard remains
  applicable to the same mechanism; and
- the final rebuilt WANIX archive receives a short fresh default-on smoke to
  prove the adapter no longer depends on the experiment-only page flag.

Passing promotion advances the accepted baseline but does not finish the thread
goal unless raw Boot and Compile both reach copy/v86 parity.

Before collecting the authoritative scorecard, the independent semantic
promotion evaluator was self-tested and frozen. It does not consume the
producer's `goalMet` bit: it revalidates the complete 117-trial matrix, exact
configuration and artifact identities, production sampled-T0 lifecycle and
runtime counters in every rewrite trial, generated-code execution, the v86
execution preflight, match counts, every row's R054 regression limit, and the
raw Boot speedup. Its fixed inputs and identities are:

- accepted R054 report
  `target/bench/r054-final-three-way-rerun/scorecard-v2-2026-08-08T23-01-30-777Z.json`,
  SHA-256 `603507e2a54729b490a87965a4c8012aa8b58ff49143a414d665ded8fcce516d`;
- evaluator library
  `82cd8183e92ac906a6d44048c8650dc7491fdea09c0fabca6f3bfe9ea6963f72`;
- independent gate
  `e339bd68134be55d77900c15fa4e4dfa678eddeaab20cb6f5aed9f8ea95356ca`;
- synthetic accept/reject selftest
  `971764ed0c4095d585d49db2d18c197c35b2edc2362c86df307c8987e3bfc2f9`.

The selftest requires a clean accept and directed rejection of a score-count
drop, any row over the 5% regression cap, Boot below 1.05x, missing lifecycle,
nonzero static errors, a rewrite identity mismatch, missing v86 execution
proof, and a baseline-hash mismatch. These files and thresholds are frozen
before timing and cannot be revised in response to the candidate result.

## Chrome confirmation result

All fourteen frozen legs completed in order without replacement. The semantic
analyzer and independent verifier both pass. Immutable result files are under
`target/jit-policy-traces/r077-production-default-28ceaf7b-chrome150-20260809-config-ab`:

- protocol JSON SHA-256
  `05668a4c06f546321b3571f53cf6d8a16ac8f5f110f544edfc44c64b73635e9a`;
- analysis SHA-256
  `46ba77ef6ea918f43372c8f187359035b45894a7eed8cac499b2fc68177790f3`.

Production-default execution is 1.163x faster `[1.136,1.191]`, with
control/production side medians 2,733.1/2,357.7 ms. Normalized MIPS is 1.163x
`[1.136,1.191]`, with side medians 65.99/76.48 MIPS. Median retired work is
180,349,181/180,310,567 instructions. Every control static counter is zero;
production records 95,104,963 sampled retirement, 179,109 samples, 316 short
marks, 286,906 bypasses, and zero errors. The gain therefore comes from
execution throughput rather than reduced guest work, and the frozen 5% Chrome
promotion gate passes.

The corrected final WANIX smoke also passes. Result SHA-256 is
`c014cb2a5f736cab072720317672d6684dcb68429b4c18a610fa0b899c5b3bd7`.
It binds the exact no-override page, production archive, and matched RV64 root;
reports an empty runtime configuration and no page-policy override; and reaches
the shell with 63,609,976 sampled instructions, 65,705 samples, 216 marks,
104,866 bypasses, and zero errors. Its correct Python phase adds 853,172 sampled
instructions, 5,787 samples, 192,328 bypasses, and generated execution. As
frozen, this is lifecycle/correctness evidence only.

## Authoritative scorecard result and rejection

The untouched authoritative run completed the v86 generated-dispatch preflight
and all 117 prescribed trials exactly once, with no retry or replacement. Its
raw report is
`target/bench/r077-authoritative-three-way/scorecard-v2-2026-08-09T12-38-31-920Z.json`,
SHA-256 `1f00a609bbcf07fcbb73d97b6ca8d90420a20561cb97418449881fd57f310b4f`;
the rendered Markdown SHA-256 is
`df837b752c2bc23514d735a69aba129b4d59f3121fee442275729be3b06ae6b4`.

The report is not admissible as a new baseline. Legacy HUFFMAN's three STEADY
samples are 12,641.231, 13,233.133, and 18,017.064 ms, a 1.425x spread above
the unchanged 1.25x stability cap. Every other report invariant passes; the
largest trial host-probe spread is 1.049x. The complete invalid report is
retained and no HUFFMAN leg or whole-scorecard sample is replaced.

The already-frozen independent evaluator wrote
`target/bench/r077-authoritative-three-way/r077-promotion.json`, SHA-256
`85d0b462fc3af8ac9ebfcf3f4f54b2bb737593aa28b934b30a93b434825ff53a`,
and rejects promotion for two independent reasons:

- the candidate report is measurement-invalid; and
- raw rewrite Boot is 2,293.093 ms versus R054's 2,260.485 ms, a 0.986x
  baseline-to-candidate ratio rather than the frozen minimum 1.05x speedup.

The descriptive, non-baseline aggregates retain 11/13 matches against
copy/v86 and 13/13 against legacy, and every rewrite row remains within the
5% R054 regression cap. Assignment is the closest at 4.85% slower relative to
R054; Compile is 4.64% slower. Those facts do not override either failed
gate. In particular, the valid same-artifact Chrome A/B result proves the
mechanism accelerates the isolated browser execution boundary, but the
authoritative default-on scorecard does not reproduce a raw Boot gain.

R077 is therefore rejected without threshold changes, per-leg replacement, or
a favorable rerun. Accepted scorecard status remains R054: 11/13 against
copy/v86 and 13/13 against legacy, with Boot and Compile still below parity.

## Default-off rollback proof

Before rebuilding, the exact rejected loader and archive were preserved under
`target/jit-policy-traces/r077-rejected-production-artifacts/` with their
original SHA-256 identities `d949d864...` and `9f28e71a...`. Stable full-system
assembly and the scorecard were then restored to default-off behavior; the
sampled/backoff mechanism remains available only through explicit diagnostic
configuration. The current default-off identities are:

- loader `f1d56b133c39cbaf174604830edbb8f351ad71f574c4f735f66271f23bb889c2`;
- unchanged main Wasm
  `28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c`;
- WANIX adapter
  `ad765bf6d618baf67b9c363b2ac0f84979fada0430fca3da389386dc10d315e0`;
- rebuilt default-off `rv64-jit.tgz`
  `378219063e1b9858443f9e4c45d7c37c88831ab4a843e8dcc39a8b8d59d42b66`;
- no-override comparison page
  `63028a1dcc430d21b12ed346808014389de15076b4d39d5690fcaf670e8e147a`.

Stable public/Worker lifecycle tests and the scorecard selftest pass after the
rollback. A measurement-valid, correctness-only Linux 6.12.7 / Alpine 3.24.1
rewrite-Boot smoke is retained at
`target/bench/r077-default-off-scorecard-smoke/scorecard-v2-2026-08-09T12-47-18-053Z.json`,
SHA-256 `c2f5ab880b77ccfc86d64335323c36045c09d2a25010066608d6f217d244791b`;
it reports generated execution and no static lifecycle or proof.

The final no-override WANIX rollback smoke is
`target/jit-policy-traces/r077-default-off-rollback-37821906-chrome150-20260809.json`,
SHA-256 `ef0b031fa3936621bb38ae7f3ac2e9cbbca11fc3ad47129c8e3dcaeb6ca8d1b8`.
It reaches the riscv64 Alpine shell, completes correct Python, retires
566,429,264 generated instructions, and proves module index -1 plus exact zero
sampled/static-T0 activity at both shell and workload boundaries. No elapsed
field from either rollback smoke is performance evidence.
