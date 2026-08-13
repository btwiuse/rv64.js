# R075 Pre-Boot Sampled Static-T0 Lifecycle Protocol

Date: 2026-08-09  
Status: Gate A passed; Gate-B artifacts and tooling frozen before formal timing

## New causal hypothesis

R074 is rejected and remains rejected. Its valid browser candidate was enabled
only after the shell appeared. That deliberately strict test started every
measured workload with an empty short-entry map, whereas an actual default-on
product would execute and learn through Linux boot, including the same 9P and
virtio kernel paths used by the shared-I/O guard.

Post-rejection Chrome CPU profiles use the same exact R074 archive and the
formal Python -> SHA-256 -> shared-9P order. Across the three shared phases,
control policy-interpreter subtrees consume 697/577/530 ms. Candidate
policy-interpreter plus static-T0 subtrees consume 650/533/524 ms. Candidate
scheduler hashing adds only 16/8/16 ms and the whole `run_system_jit` subtree
adds 16/17/12 ms. These profiles are attribution-only, not timing evidence,
but they reject the claim that the accelerated decoder or tuple lookup alone
must account for the observed 52 ms unprofiled median difference.

R075 changes no decoder rule, threshold, entry key, instruction family, page
policy, module geometry, scheduler cadence, guest, or benchmark. It tests the
missing product lifecycle: prepare the same auxiliary module and configure the
exact R074 control or candidate immediately after `RV64.create` and before
`vm.start`. Candidate short-entry state then starts empty at reset and may be
learned only from ordinary architectural execution beginning with the first
firmware/kernel instruction.

## Exact configurations and embedding

Add one adapter-only option removed from the guest kernel command line:

- `rv64.static-t0=control` calls `configureJit` before start with residual
  static off, sampled static off, and backoff off;
- `rv64.static-t0=sampled-backoff` calls it at the same point with residual
  static off, sampled static on, and backoff on.

Both legs therefore prepare one identical guest-independent module before
boot. WANIX captures launch arguments before the harness can modify a started
page, which an untimed candidate-only smoke exposed without producing a
comparison result. The formal harness therefore uses two immutable pages whose
only byte-level difference is the exact `rv64.static-t0` value already present
in the RV64 VM `append` attribute; it never calls `configureJit` at the shell.
The main Wasm and every guest/archive byte are identical between legs. This
adapter option is a product lifecycle surface, not a scorecard, PC, binary,
browser, phase, device-request, output, or workload selector.

R072-R074 measurements motivate R075 but may not be pooled into an R075 timing
statistic. R074's failed verdict cannot be changed by this experiment.

## Gate A: adapter and lifecycle correctness

Before timing:

1. add parser tests proving both values are accepted, removed from the guest
   command line, and reject every other value;
2. prove control and candidate prepare exactly one equal static module before
   start, with the expected enable bits;
3. reach the exact modern direct Linux shell in both legs and prove candidate
   sampled retirement, short marks, and bypasses are already nonzero at the
   shell, while all corresponding control counters are zero and errors are
   zero;
4. rerun the R074 q1/q32/q1024, dirty/reset, generated-entry, WFI, public API,
   direct/OpenSBI, and strict lower-level correctness gates; and
5. verify that default pages and users that do not request the new adapter
   value retain all-off behavior.

A correctness repair must add a directed regression and cannot change the
candidate, threshold, key, activation point, or later performance rules.

Gate A passed. Go/Wasm parser tests prove both exact values, removal from the
kernel command line, duplicate/invalid rejection, and default-off behavior.
Independent control and candidate Chrome smokes reached Linux 6.12.7 and the
matched WANIX Alpine 3.22.5 shell. Both prepared module index 822 before start.
Control retained all static counters and enable bits at zero; candidate reached
the shell with 64.186M sampled retirement, 66,060 samples, 224 short marks,
88,945 bypasses, and zero errors. The page diff contains exactly the one
`rv64.static-t0=control` versus `sampled-backoff` attribute change.

`REQUIRE_ALL=1 tests/run-all.sh` then passed all eight release stages: 32/56/76
core/DBT/system tests, 134 ISA cases, 109 Spike locksteps, 193 architecture
signatures, the complete Wasm/system/T2 matrix, direct Linux, stable public
page-policy Linux, and modern OpenSBI virt smoke. Public and Worker APIs pass.
An explicit sampled-backoff direct/OpenSBI rerun retired 53.938M/53.447M
sampled instructions, recorded 253/260 marks and 537,972/639,824 bypasses, and
reported zero errors. The adapter's `GOOS=js GOARCH=wasm` test suite passes.

## Gate B: prospective browser product A/B

After Gate A, build and hash one archive and two immutable flag-specific pages.
Freeze a new schema, runner, analyzer, verifier, Chrome/V8 identity, CPUs 8-15,
and exact artifact hashes before collection. Run seven alternating fresh
process/profile/guest pairs, with three synchronized Python, SHA-256, and
shared-9P repetitions per leg. No leg may be replaced.

Every leg must prove exact guest/checksum/artifact/browser/affinity identity,
production page policy, generated execution, one prepared module, and the
pre-start lifecycle. Candidate must have nonzero sampled retirement,
marks/bypasses already at the shell and in every phase, residual static off,
and zero errors. Control must have zero sampled/backoff activity.

The prospective statistics are:

- browser shell speedup is paired `control/candidate`; its median must be at
  least 1.10x and exact paired-bootstrap 95% lower bound at least 1.00x;
- for each benchmark phase, take each leg's median of three and the paired
  median of seven candidate/control elapsed ratios;
- Python, SHA-256, and shared 9P must each be at most 1.03x, with exact
  paired-bootstrap 95% upper bound at most 1.10x; and
- exact correctness/lifecycle failure invalidates the experiment rather than
  becoming performance failure.

No raw shared-9P spread cap is used; established controls already prove it is
incompatible with this subsecond workload. The confidence bound, triplicate
leg medians, and seven pairs remain the uncertainty guard. There is no retry,
threshold/key tweak, shell activation fallback, sample pooling, or rule change
after a complete valid unfavorable result.

### Frozen Gate-B identities

- main Wasm: `28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c`;
- shared R075 archive: `e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c`;
- control page: `a2f43c06f86507c267c36fb3922079d11d44072ab0622526e5577f69448e976f`;
- candidate page: `7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413`;
- preboot WANIX harness: `525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545`;
- shared runner / R075 wrapper: `bf83e993e62481acb743dbc3d5397eefab2b74b896b330ace8eca3bd44200049` /
  `3223259d4892f32f326b5f3b8e8f2ec51bebfd4b6d46982c6e0ece1ac9914dea`;
- shared analyzer / R075 wrapper: `dd37d6d262666fceb506476f4e566b2752b63e19f2bc9ed0b28b58844a761ed0` /
  `47fcadd74e2bd6d797ed8d6f55d2c842af72fe40b3d2472ac917e53956355291`;
- analyzer selftest / verifier: `031132af0832c2e1894f5d18a9f52e0f43f1b7c98332cbb7f2321b0b4986dbf9` /
  `9f21db6ed38206b201924887051c66cb96e11bb4b4d94d01b2a7f1a0090bbef8`;
- browser: Chrome `150.0.7871.186`, revision
  `@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d`, V8 `15.0.245.21`;
- matched WANIX roots: RV64 `274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb`,
  x86 `09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320`;
- frozen v86 archive: `7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771`;
- CPUs: `8-15`; schema: 5; result directory:
  `target/jit-policy-traces/wanix-r075-preboot-28ceaf7b-chrome150-20260809-config-ab`.

The schema-5 analyzer selftest proves a valid pass, insufficient shell gain,
4% shared-9P regression, and missing shell lifecycle proof take the expected
paths. The extended shared analyzer reproduces the immutable R073 and R074
reports byte-for-byte at SHA-256 `9bbaf2cd89cc...` and `d3cf3a102966...`.

## Gate C: product guard and promotion

Only if Gate B passes, run a fresh seven-by-three candidate/v86 browser guard
with the established 1.10 upper-confidence limit. Then make this exact
pre-start lifecycle the candidate product default, rebuild/hash it, rerun the
complete strict correctness suite, and execute the untouched 13-row
rewrite/legacy/v86 scorecard with three alternating fresh processes per side.

Promotion requires 13/13 versus legacy, no reduction from R054's 11/13 versus
v86, no row more than 5% slower than R054, Boot at least 1.10x faster than the
same-core control, and no `/shared/bench.py` regression. Promotion still does
not complete the thread goal unless raw Boot and Compile both reach copy/v86
parity.
