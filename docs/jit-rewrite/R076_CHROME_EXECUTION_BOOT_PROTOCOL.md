# R076 Chrome Execution-Only Modern-Boot Confirmation Protocol

Date: 2026-08-09  
Status: Gate A passed; implementation, identities, result path, and performance
gates frozen before formal timing

## Why this is a new experiment

R075 remains rejected. Its preregistered `shellMs` clock begins before Chrome
publishes its debugging endpoint, before WANIX finishes loading and compiling
its host kernel, before the comparison page registers custom elements, and
before the RV64 archive and Go adapter create the emulator. The valid seven-pair
result precisely proves that the complete cold launch-to-shell product path is
unchanged at 0.997x `[0.993,1.002]`; it cannot prove that RV64 execution itself
is unchanged.

Post-rejection attribution-only 100-microsecond Chrome Worker CPU profiles make
that distinction material. Control/candidate complete-profile sampled CPU is
1,274.06/1,020.70 ms, while the `run_system_jit` subtree is
1,110.88/883.16 ms. Candidate replaces the control's 1,012.80 ms sampled
interpreter subtree with 686.26 ms in the static-T0 path plus 88.09 ms residual
sampled interpretation. Profile overhead and partial attachment make these
causal attribution, not timing evidence, but they show the frozen R075 clock
diluted a plausible emulator saving inside roughly 31 seconds of common host
startup.

The authoritative scorecard uses a different and explicit boundary: it loads
assets, creates the VM, configures the policy, calls `bootVirtLinuxDirect`, and
prepares the auxiliary module before timing; it starts immediately before the
first `runVirtSystem(2_000_000)` pump and stops on `SCORECARD_V2_READY`. R076
prospectively reproduces that boundary in a fresh Chrome module Worker using
the exact scorecard Linux 6.12.7 / Alpine 3.24.1 inputs. No R075 elapsed time is
pooled into R076, and R075's verdict cannot change.

## Frozen candidate and execution contract

Both legs use byte-identical files:

- loader `web/rv64.js`, SHA-256
  `2582e18ea207ad0f5ec154e82d3fe6208faf5742f26a9d8463d8d360f168b776`;
- main Wasm, SHA-256
  `28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c`;
- Linux `Image`, SHA-256
  `57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2`;
- scorecard RISC-V initramfs, SHA-256
  `cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808`.

After fetch and VM creation, both call `bootVirtLinuxDirect` with 512 MiB,
`console=ttyS0 rdinit=/init`, and the exact kernel/initramfs. Both prepare the
same guest-independent static module after reset and before timing:

- control: residual static off, sampled static off, backoff off;
- candidate: residual static off, sampled static on, backoff on.

The production page policy remains unchanged: threshold 131,072, privileged
multiplier 32, q1024, two builds in flight, two pages, 512 leaders, 10% control
gate, stable chain and tail chain on, privileged controls off, and region TLB
cache threshold four. The worker starts `performance.now()` immediately before
the first 2M-instruction pump, yields through `MessageChannel` after pump 1 and
then every fourth pump exactly like the authoritative Node scorecard, and stops
on the first complete `SCORECARD_V2_READY` marker.

The browser page, worker, runner, analyzer, verifier, Chrome/V8 identity, CPUs
8-15, input identities, query variants, sample count, order, and output paths
must be hashed and recorded after correctness but before collection. There is
no threshold, quantum, yield, page-policy, guest, or sample-count command-line
knob in the formal runner.

## Gate A: harness and semantic proof

Before timing:

1. add a deterministic worker selftest or source-level verifier proving all
   asset fetch, VM creation, machine boot/reset, policy/static preparation, and
   initial counter reads precede the timer;
2. prove the timer encloses only the pump-to-ready loop and exactly matches the
   authoritative scorecard's guest marker, quantum, and yield cadence;
3. verify Linux 6.12.7, Alpine 3.24.1, `riscv64`, input hashes, ready marker,
   generated execution, production policy, and complete instruction accounting;
4. prove both sides prepare one equal auxiliary module, candidate has nonzero
   sampled retirement/samples/polls/marks/bypasses and zero errors, and control
   has exactly zero corresponding activity; and
5. run at least one untimed control/candidate smoke without using its elapsed
   values for a decision.

Any correctness repair must add a directed regression and cannot change the
candidate, timer, cadence, guest, sample plan, or later thresholds.

### Gate A result and frozen collection identity

Gate A passed before formal collection. The boundary selftest proves setup is
outside the clock, the first 2M-instruction pump is inside it, yields occur
after pumps 1/5/9 and every fourth pump thereafter, and completion is the exact
`SCORECARD_V2_READY` marker. The analyzer selftest independently enforces a
passing sample, the 1.05 execution gate, the normalized-MIPS gate, and static
module lifecycle invalidation.

The first pre-timing worker smoke exposed a harness defect: it had not applied
the authoritative scorecard's production page-policy configuration, leaving
the candidate sampled path inactive. No formal timing had been collected. The
worker was corrected to make the same policy calls as the scorecard before
boot, and the boundary/source selftest now enforces those calls. The corrected
untimed smokes passed:

- control: exact guest/output, 180,351,284 retired instructions, generated
  execution present, and zero sampled/static marks/bypasses/errors;
- candidate: exact guest/output, 180,310,336 retired instructions, 96,293,916
  sampled-static instructions, 309 marks, 281,865 bypasses, generated
  execution present, and zero errors.

Both smokes add exactly one equal auxiliary module. Their elapsed values were
not printed, retained, or used for a decision.

Formal execution is pinned to CPUs `8-15` and exact Chrome
`150.0.7871.186`, revision
`@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d`, V8 `15.0.245.21`.
The immutable result directory is:

`target/jit-policy-traces/r076-chrome-modern-boot-28ceaf7b-20260809-config-ab`

Frozen file identities are:

- browser page: `bae81c20a73019dd57f5701330862f4a7b1c9d03feebbd1a7a718632e9c5d29b`
  (1,032 bytes);
- module Worker: `30c381a18b97ee9c24d54ee3a001c0232e4ff4d438036a9b782512417e60a883`
  (7,367 bytes);
- timing library: `8d39c83ad01e1d1003bf56ded2439d3475f925f7925f0731da64b90170f9f548`
  (1,131 bytes);
- browser host: `fb05697dff42a57636c8965842f721964f444faa0b5db520c2a587f4c8c9d857`;
- boundary/source selftest:
  `516c3d1b4371fb56de8a5823c37942c11ce60cb897245827c5a95d52a4cb8995`;
- untimed smoke: `ece1e4ea1e8a666c86d69b41c7bc2532b727ffa376b030c252415738711f1ccf`;
- formal runner: `d9b66a3b48a31a1154b827cb52c738319c99c5604bc98787d6c588451d024a70`;
- analyzer: `0dcea6d7dcbe2503fe56398c4e445ae4b4710ac87d407b02c833d95dbce1bae2`;
- analyzer selftest:
  `5984e19939a6d908b9b6f75e2708b8ef2bce10d3b8200dca5bc4961285a17728`;
- gate verifier: `b2dc21390c2dbce09ed1ba281a76123020d225931182a5db83c14b48fcc3d9d9`.

Input sizes are loader 90,264 bytes, main Wasm 4,329,839 bytes, Linux Image
4,205,056 bytes, and initramfs 64,383,488 bytes. Their SHA-256 identities are
the frozen values above.

## Gate B: prospective Chrome cumulative-gain A/B

Run seven alternating fresh Chrome process/profile/Worker/guest pairs, control
first on odd pairs and candidate first on even pairs. No leg may be replaced.
Use one complete modern Boot per leg; unlike the warmed subsecond WANIX phases,
there is no within-guest repetition or reused guest state.

For every leg retain execution milliseconds, retired instructions, MIPS,
generated/static/policy counters, browser identity, host CPU affinity, asset
hashes, guest contract, output fingerprint, and chronology. Exact semantic,
artifact, lifecycle, affinity, or order failure invalidates the sample rather
than becoming a performance loss.

The primary statistic is the paired control/candidate execution-time speedup.
This uses the standing cumulative-gain policy requested for mature
optimization:

- paired median speedup must be at least 1.05x;
- its exact paired-bootstrap 95% lower bound must be at least 1.00x; and
- candidate normalized MIPS must not regress: paired candidate/control MIPS
  median at least 1.00x with lower bound at least 0.97x.

No raw within-side spread cap is used; all values remain visible and the exact
paired interval is the uncertainty guard. There is no retry, outlier deletion,
candidate modification, alternate timer, cadence change, or R075 sample pooling
after a complete valid result.

R075's exact valid `/shared/bench.py` phase evidence remains an independent
nonregression guard for these unchanged candidate bytes: Python 1.014x
`[0.990,1.034]`, SHA-256 0.996x `[0.990,0.998]`, and shared 9P 0.871x
`[0.799,1.059]` candidate/control. R076 does not rescore or reinterpret its
failed launch-to-shell statistic.

### Gate B result

All fourteen frozen Chrome legs completed without replacement. The analyzer
and final verifier both pass. The immutable report is:

`target/jit-policy-traces/r076-chrome-modern-boot-28ceaf7b-20260809-config-ab/analysis.json`

Protocol SHA-256 is
`d991ff4d7d5b3fbcab3ce9412338f742a3f4b64bc29d935eff56e929bdd7cfa9`;
analysis SHA-256 is
`4263bdcd6542e3d40b2d1f138d5526e8623393ff8bfa5069b48edafbcf7cd985`.
The measurement is valid and Gate B passes:

- execution-time speedup: 1.175x `[1.167,1.189]`, control/candidate medians
  2,772.8/2,345.6 ms;
- candidate/control normalized MIPS: 1.174x `[1.167,1.189]`, side medians
  65.04/76.87 MIPS.

Control/candidate median retired instructions are 180,349,740/180,302,563, so
the result is a throughput gain rather than reduced guest work. Candidate
median sampled-static retirement is 97,862,176 with 187,565 samples, 317 short
marks, 281,311 bypasses, module index 822, and zero errors; every control static
counter is zero. This clears the prospective 1.05 cumulative-gain rule and
admits Gate C without changing R075's rejected launch-to-shell verdict.

## Gate C: product and parity escalation

Only if Gate B passes, run a fresh candidate/v86 WANIX seven-by-three guard,
then enable this exact candidate by default, rebuild/hash, rerun the complete
strict correctness matrix, and run the untouched authoritative 13-row
rewrite/legacy/v86 scorecard with three alternating fresh processes per side.

Promotion requires 13/13 versus legacy, no reduction from R054's 11/13 versus
v86, no row more than 5% slower than R054, a reproducible Boot improvement of
at least 5%, and no `/shared/bench.py` regression. A cumulative promotion does
not complete the thread goal unless raw Boot and Compile both reach copy/v86
parity.
