# R078 Dormant Static-T0 Artifact Regression Audit

Date: 2026-08-09  
Status: protocol and verifier frozen before collection

## Question

R077 correctly restored sampled/static T0 to default-off after production
promotion failed, but the current main Wasm still contains the complete R070-
R074 experimental runtime machinery. It is 4,329,839 bytes versus accepted
R054's 4,272,517 bytes. More importantly, the hot page-policy loop still loads
the disabled flags, queries sampled-static support, selects extra branches,
and carries the short-entry set even when no auxiliary module is prepared.

The valid R076/R077 Chrome controls take roughly 2.73 seconds, while exact R054
historically took 2.26 seconds in the authoritative Node scorecard. The single
post-rollback 2.72-second smoke is correctness-only and cannot decide a
performance claim, but it makes a direct artifact regression audit mandatory
before adding another optimization to the slower control.

The hypothesis is that removing the dormant post-R054 machinery restores at
least 5% of raw Boot without regressing Compile or Python. This is an artifact
audit, not a proposal to ship an old binary: a passing result admits a clean
source removal followed by rebuild and full correctness/performance gates.

## Frozen comparison

Use the current loader and scorecard worker for both sides. Main-module creation
remains outside workload timing. Both sides use the production page policy,
default-off sampled/static T0, Linux 6.12.7, Alpine 3.24.1 riscv64, identical
inputs, fresh processes and guests, and the scorecard's normal timer/cadence.

- control: current default-off Wasm
  `28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c`;
- candidate restoration: exact accepted R054 Wasm
  `4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`;
- loader:
  `f1d56b133c39cbaf174604830edbb8f351ad71f574c4f735f66271f23bb889c2`.

Run five alternating pairs for Boot, Compile, and Python on CPUs 8-15. Retain
all 30 legs. There is no retry, outlier removal, old-report pooling, threshold
change, or per-row replacement after collection. The R054 override makes those
legs intentionally ineligible for the authoritative cross-ISA scorecard; the
fixed A/B report is diagnostic evidence only.

## Frozen gates

Every leg must have the exact artifact/loader/guest/input/output identity,
generated-code execution, complete phases, and zero sampled/static activity.
The report and each side must satisfy the unchanged 1.25x host/sample spread
limits. Performance requires:

- paired Boot restoration at least 1.05x with exact paired-bootstrap 95% lower
  bound at least 1.00x; and
- paired Compile and Python medians at least 0.97x.

A pass admits removal of the dormant static-T0 implementation from source,
not direct installation of the archived R054 binary. A fail closes this
explanation and sends the accepted default-off artifact to fresh profiling.

Frozen tool identities before timing:

- A/B harness `74d624e8b40111cd79d4598acfc6284f8e94ab93c710194dec3e95c478196d79`;
- worker `cb9413c923db9d28e7fed4c1044a306d8bc09c366a6179a6e8cef42ebbaea3d9`;
- scorecard library `8681b09f81f3c71e30945d5770486517d993c712b2394c017b4980d481e31c61`.

The independent evaluator, final gate, and synthetic rejection selftest pass
and are frozen before collection:

- evaluator `e00257fccdcf37e1873530c6fdebe51374d35f45e229a69970a0859025b75231`;
- gate `146bf008f7af5ed3f2a2023bb8c01942b53c8a8e523238eb96ffd93a04f94b4d`;
- selftest `7cb76b50128219476d2bbd0c5f24b3baa2d4db30ca5c09f20b7c15db7a6ef6f3`.

The selftest requires one acceptance and independently rejects insufficient
Boot median, a regressing confidence lower bound, a Compile guard loss,
identity mismatch, nonzero static activity, an incomplete trial set, and CPU-
affinity drift. These files and thresholds cannot change after timing begins.

## Frozen result

All 30 legs completed once and passed identity, output, generated-execution,
zero-static-activity, affinity, and spread checks. The report is
`target/bench/r078-dormant-static-regression-audit/config-ab-2026-08-09T13-02-26-919Z.json`
(SHA-256
`3ebd8867c623502b1717d8ffb3d4d0f30a2deaa76e7cbdb3fe951596f7ec0d5d`)
and the frozen gate is `gate.json` in the same directory (SHA-256
`df620d7af12066244134ddc64fff520a7bec0210aae904c020c9f155aabf0504`).

Exact R054 improves Boot 1.178x `[1.116,1.190]`, Compile 1.037x
`[0.966,1.085]`, and Python 1.031x `[0.991,1.043]` relative to the post-R077
default-off artifact. The audit therefore passes and proves that dormant
rejected machinery materially regressed production even when disabled. This
admits source removal; it does not authorize shipping the archived binary.
