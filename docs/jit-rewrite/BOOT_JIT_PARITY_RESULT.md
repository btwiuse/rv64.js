# Boot JIT parity and lifecycle result

Date: 2026-08-14 America/Phoenix

Disposition: achieved for the Boot-focused objective; the older all-row
copy/v86 parity objective remains open because Compile and several RV64GCV
vector-heavy rows still lose.

## Objective and conclusion

The objective was to make JIT-enabled modern Boot equal to or faster than the
same build with JIT disabled, using architecture-general Tier-0/JIT lifecycle
changes, while preserving exact correctness and avoiding a speed regression in
either complete scalar or RV64GCV JIT scorecard.

That objective is achieved by the release Wasm artifact with SHA-256
`90d543ac510efeb8627ec3c806c3c36f01602aaf1609921d0b588d8001fddc71`:

- scalar Boot is `1.057x` faster with the JIT enabled, with paired 95%
  confidence interval `[1.042, 1.065]`;
- RV64GCV Boot is `1.079x` faster with the JIT enabled, with paired 95%
  confidence interval `[1.069, 1.085]`;
- the complete scalar and RV64GCV artifact A/B matrices contain no
  scorecard-class regression, and no row has a statistically demonstrated
  slowdown after focused reruns of the two initially suspicious rows;
- the final authoritative scalar scorecard improves from 11/13 to 12/13
  non-losses against copy/v86 by moving Boot from LOSS to MATCH;
- the final authoritative RV64GCV scorecard improves from eight wins to eight
  wins plus a Boot MATCH, or 9/13 non-losses; and
- the strict eight-stage correctness matrix ends in `ALL STAGES PASSED`.

The scorecard runner still prints `Rewrite goal NOT MET` for both final
cross-v86 reports. That message evaluates the older requirement to match or
beat copy/v86 on all thirteen rows. It does not contradict this narrower and
subsequently authorized Boot JIT-on/JIT-off objective.

## Promoted architecture-general mechanisms

### Integrated generated-code fallback

Generated code often exits into the scalar interpreter for a short unsupported
or privileged bridge before reaching another generated entry. Previously this
fallback returned to the general per-instruction path, including an indirect
compiled-entry callback after every instruction.

The promoted path now:

- uses the existing integrated scalar Tier-0 loop for generated-code fallback;
- preserves exact post-instruction generated-entry stopping;
- preserves first non-sequential target observation for control-entry policy;
- exposes the generated dispatch table as a read-only, strided
  `CompiledEntryMap`, allowing its one-load PC-tag test to inline into Tier-0;
- retains device synchronization, WFI, direct-SBI, exception, budget, and
  retirement semantics; and
- executes at most two instructions through the compact scalar bridge before
  transferring a longer gap to integrated Tier-0. The choice depends only on
  observed bridge length.

The direct-map view is valid only for the duration of one synchronous Wasm
interpreter call. Async publication cannot run during that call, and policy
observation does not mutate or reallocate the dispatch table.

### Privileged heat admission

The ordinary user-page threshold remains `131072` retired instructions.
S/M-mode pages now require `64x` that heat instead of `32x` before Tier-2
admission. Privilege mode and page heat are architectural/runtime state, not
workload identity.

In the five-pair RV64GCV artifact comparison, this changed cold Boot from eight
privileged candidates and sixteen host modules to three privileged candidates
and eleven modules. Median host compile work fell from roughly `216 ms` to
`166 ms`. Truly persistent privileged code can still tier up; transient
firmware and kernel pages remain in the faster integrated Tier-0 path.

The multiplier remains visible through the public statistics/configuration
contract so controlled policy A/Bs are reproducible.

## Same-artifact Boot proof

Each row below compares JIT disabled and production JIT enabled in fresh
processes using the exact same `90d543...c71` Wasm. Control/candidate order was
balanced for eleven pairs. Lower duration is better.

| Population | JIT disabled | JIT enabled | Independent speedup | Paired median | Paired 95% CI |
| --- | ---: | ---: | ---: | ---: | ---: |
| scalar modern | 1,598.35 ms | 1,512.02 ms | `1.057x` | `1.056x` | `[1.042, 1.065]` |
| RV64GCV | 1,648.36 ms | 1,527.68 ms | `1.079x` | `1.076x` | `[1.069, 1.085]` |

Both reports are measurement-valid, have empty problem lists, use identical
kernel/initramfs identities within each pair, and record the same Wasm hash on
both sides.

Evidence:

- scalar JSON:
  `target/bench/boot-no-jit-penalty/minimal-threshold64-scalar-jit-on-vs-off-v11/config-ab-2026-08-14T08-40-52-641Z.json`
  (`b33ce9b0033f70b31186d11dfa3a52ce803479733bc28edb2497590ceb5bbe2a`);
- RV64GCV JSON:
  `target/bench/boot-no-jit-penalty/minimal-threshold64-gcv-jit-on-vs-off-v11/config-ab-2026-08-14T05-27-58-604Z.json`
  (`615877fc0ce176b0a92b67cf64aaa5ad84cdb81a7539b8d79d20e96c33fdd7b1`).

## Complete artifact regression guard

The control is the frozen pre-lifecycle JIT artifact
`f3147807fb6654226256752acb4afdf2e89327c23d64d93d0e5b5c6305f6f761`.
The candidate is exact final artifact `90d543...c71`. Each complete matrix used
five balanced fresh-process pairs. Values are control duration divided by
candidate duration, so values above one favor the final artifact.

| Benchmark | Scalar | RV64GCV |
| --- | ---: | ---: |
| ALU | `0.996x` | `1.008x` |
| Mixed | `1.000x` | `0.999x` |
| Boot | `1.042x` | `1.088x` |
| Python | `1.008x` | `1.018x` |
| Compile | `1.034x` | `1.074x` |
| Numeric Sort | `0.996x` | `0.995x` |
| String Sort | `1.006x` | `1.001x` |
| Bitfield | `1.008x` | `1.000x` |
| FP Emulation | `0.998x` | `1.000x` |
| Fourier | `1.001x` | `1.000x` |
| Assignment | `0.999x` (15-pair follow-up) | `0.992x` |
| IDEA | `0.997x` | `1.001x` |
| Huffman | `1.002x` | `1.004x` (15-pair follow-up) |

All point results meet the scorecard's `0.95x` MATCH floor. Every five-pair
paired interval included parity except the initial RV64GCV Huffman interval.
Its preregistered-style fifteen-pair follow-up reversed the small point loss:
`1.005x` paired with interval `[0.999, 1.008]`. Scalar Assignment was highly
bimodal in both artifacts; its fifteen-pair medians were `482.86` and
`483.54 ms`, a `0.999x` result with interval `[0.933, 1.053]`. This does not
claim zero sub-percent microvariance; it demonstrates no scorecard-class loss
and no statistically supported regression.

Evidence:

- scalar complete matrix:
  `target/bench/boot-no-jit-penalty/minimal-threshold64-scalar-regression-v5/config-ab-2026-08-14T06-12-39-495Z.json`
  (`1eae14407f9ab43ff0fbfd6d574d368fb3052d65364405594c588bdc359724c2`);
- RV64GCV complete matrix:
  `target/bench/boot-no-jit-penalty/minimal-threshold64-gcv-regression-v5/config-ab-2026-08-14T05-59-38-270Z.json`
  (`358c577bb2cfcc5b10dfc4b3c5e64669fffd7b015bd2e93146f513c4eba89839`);
- scalar Assignment follow-up:
  `target/bench/boot-no-jit-penalty/minimal-threshold64-scalar-assignment-v15/config-ab-2026-08-14T06-14-47-861Z.json`
  (`499721745dd1ef0102721b25659ad3c5c7ff257bce024f20a688d89332e68cfe`);
- RV64GCV Huffman follow-up:
  `target/bench/boot-no-jit-penalty/minimal-threshold64-gcv-huffman-v15/config-ab-2026-08-14T06-02-02-157Z.json`
  (`d15f178b928c4b56e7972744ebba6d65064c4ac2d26c2967da8d90a086cbf5c8`).

## Final authoritative cross-v86 scorecards

The scalar three-way report is measurement-valid with an empty problem list.
Rewrite wins ten rows, matches Boot and Assignment, and loses only Compile:

- Boot: rewrite `1,532.2 ms`, copy/v86 `1,589.7 ms`, MATCH `1.04x`;
- Compile: rewrite `802.7 ms`, copy/v86 `734.5 ms`, LOSS `1.09x behind`;
- total: 12/13 rows at or above the copy/v86 parity floor.

Evidence:

- JSON:
  `target/bench/boot-no-jit-penalty/final-minimal-scalar-authoritative-v3-rerun/scorecard-v2-2026-08-14T08-37-12-846Z.json`
  (`9fd36d786968e138ed30fb85a53ec7276bfeb03b3d049add302ffd043936f3f6`);
- Markdown:
  `target/bench/boot-no-jit-penalty/final-minimal-scalar-authoritative-v3-rerun/scorecard-v2-2026-08-14T08-37-12-846Z.md`
  (`7b4c40d06977f8973e11c5f227ea9985cf2c2d02aecc7ee9c94b7e58528237cf`).

The RV64GCV two-way report is also measurement-valid with an empty problem
list. It wins eight rows, matches Boot, and retains four pre-existing losses:
Compile, String Sort, FP Emulation, and Assignment.

- Boot: rewrite `1,502.8 ms`, copy/v86 `1,531.1 ms`, MATCH `1.02x`;
- prior Boot: rewrite `1,657.6 ms`, copy/v86 `1,556.1 ms`, LOSS `1.07x behind`;
- Compile improved from `877.4` to `803.1 ms`, reducing its gap from `1.21x`
  to `1.07x behind`;
- total: eight wins plus one match, or 9/13 non-losses.

Evidence:

- JSON:
  `target/bench/boot-no-jit-penalty/final-minimal-gcv-authoritative-v3/scorecard-v2-2026-08-14T08-49-13-292Z.json`
  (`f72076215a90540b004b6f55686c7e1f5132ae7b5336740fea8ea529121cabb4`);
- Markdown:
  `target/bench/boot-no-jit-penalty/final-minimal-gcv-authoritative-v3/scorecard-v2-2026-08-14T08-49-13-292Z.md`
  (`2792d09c69587664ed92111e5edc114c25d06677032e9cb68ad9d1c0dbb0f5b2`).

## Correctness gate

The exact benchmarked source rebuilt to the same `90d543...c71` Wasm after the
strict command:

```sh
nix develop --command env REQUIRE_ALL=1 ARTIFACTS=target/bench tests/run-all.sh
```

The uninterrupted final rerun ended in `ALL STAGES PASSED`. It includes:

- the complete workspace and guest-integration Rust suites;
- bit-identical QEMU guest differentials;
- all 8,724 RVV interpreter executions against QEMU;
- 134/134 `riscv-tests`, 109/109 Spike lockstep tests, and 193/193 matching
  architecture-test signatures;
- all scalar integer, M/A, FP, memory, system-memory, TLB, WFI, T2, and
  randomized atomic JIT differentials;
- all 8,724 hot RVV JIT executions, plus vector fault, full-system, and
  fractional-LMUL memory tests;
- Wasm API, Worker, networking, 9P, Wasm smoke, generated-module, and
  standalone Wasmtime checks;
- direct and OpenSBI modern Linux boots, stable public page-policy Linux, FP
  context switching, and AMO differential; and
- the modern Virt smoke guest through `SMOKE_OK`.

The first strict attempt exposed two stale assertions that still expected the
old public multiplier `32`. Both were updated to `64`; their focused tests
passed, and the complete strict matrix was then rerun from the beginning and
passed without failure.

## No-special-casing audit

The final production diff contains no benchmark name, row name, guest symbol,
guest-binary hash, kernel/initramfs identity, input marker, or fixed guest-PC
recognizer. It does not recognize Boot or any scorecard workload.

The only selection inputs added by this work are:

- whether the current PC tag is present in the general generated dispatch map;
- whether control flow was sequential;
- how many instructions a generated/interpreter bridge has executed;
- architectural privilege mode; and
- general per-page retired-instruction heat.

These mechanisms apply to arbitrary RV64 programs. The threshold value was
tuned and validated using the requested Boot and complete scorecards, but no
runtime benchmark identity was introduced.

An entire-source text audit does find older comments in `rv64-wasm` that name
nbench, CPython, and tcc while explaining historical policy decisions. Those
comments and their general runtime-signal policies predate this change and are
present on both sides of the frozen artifact comparison. This work neither
uses those names at runtime nor adds another such selector. Benchmark-informed
threshold tuning is being reported explicitly; workload recognition is not
being disguised as a general optimization.

Several broader experiments were rejected and removed before the final build:
single in-flight compilation, privilege-specific leader caps, candidate seed
snapshots, and a completion pump. They either changed unrelated region geometry
or failed the scalar/RV64GCV guards. Production retains the prior two-job
in-flight limit, full leader cap, and ordinary asynchronous lifecycle.

## Remaining work outside this objective

This result does not claim all-row copy/v86 parity. Scalar Compile remains
about 9% behind. RV64GCV Compile, String Sort, FP Emulation, and Assignment
also remain behind. They require separate architecture-general work and must
not be hidden by relabeling this Boot result.
