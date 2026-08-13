# R074 Per-Entry Short-Sample Backoff Protocol

Date: 2026-08-09  
Status: rejected at valid browser Gate C; default remains off

## Causal hypothesis

R073 proves that exact sampled static T0 is useful during cold Linux execution
but is not safe as an unconditional persistent tier. Five native pairs improve
Boot 1.157x `[1.137,1.187]`. Its valid seven-by-three Chrome report leaves
Python and SHA-256 neutral at 0.994x and 0.999x candidate/control, but shared
9P is 1.058x `[0.932,1.114]` and fails the frozen guard.

The retained architecture counters identify an execution-shape boundary rather
than a guest workload identity:

- native Boot retires a median 106.957M sampled instructions in 476,959
  samples, or 224.3 instructions per sample;
- native Compile FIRST/PRIME/STEADY averages 88.5/67.6/97.9 and native Python
  109.5/117.9/118.2 instructions per sample;
- the formal browser shared-9P median is 15.740M sampled instructions in
  430,157 samples, or 36.6 instructions per sample;
- compared with control, shared 9P executes a median 8.0% more total guest
  instructions, issues seven rather than five modules, and records 82.5 rather
  than 48.0 ms host compilation. Host 9P service time changes only 1.5%.

The candidate therefore treats the static decoder as a per-entry Tier 0, not a
permanent policy. The first eligible policy sample for an exact mapped entry
uses static T0. If that sample retires fewer than 64 instructions, later policy
samples starting at the same `(virtual page, physical page, entry PC)` use the
accepted sampled interpreter. A sample of 64 or more leaves the entry eligible.

The value 64 is not swept. It reuses the existing architecture-wide
`JIT_ON_THRESHOLD` and represents the minimum observed stretch that can repay
a tier transition. The backoff is cleared on machine/JIT reset and for a dirty
physical code page. It uses no guest PC value, binary, symbol, privilege mode,
opcode family, device request, wall clock, benchmark phase, output, or browser
identity. The entry PC participates only as an opaque cache key.

## Exact configurations

Both legs use one newly built main Wasm and prepare the same auxiliary static
module exactly once:

- control: residual static off, sampled static off, short-sample backoff off;
- candidate: residual static off, sampled static on, short-sample backoff on.

R072/R073 samples are causal evidence only and are not pooled into any R074
timing statistic. R073 remains rejected regardless of this result.

## Gate A: semantics and mechanism proof

Add a default-off API switch and counters for newly marked short entries and
subsequent interpreter bypasses. Extend the q1/q32/q1024 full-state and ordered
page-policy fingerprint differential so candidate and accepted interpreter
produce the same observation sequence. Add a directed program whose first
mapped-entry sample retires fewer than 64 instructions and prove:

1. the first sample executes through static T0;
2. the exact entry is marked once;
3. its next eligible sample bypasses static T0;
4. a distinct or at-least-64-instruction entry remains eligible;
5. dirty-page and reset lifecycles remove stale backoff state;
6. generated-entry handoff and partial-progress WFI remain exact.

Then run direct and OpenSBI modern Linux, the public/Worker API tests, scorecard
selftests, and the strict lower-level system-memory/Sv39/A/FP/T2 matrix. Any
semantic mismatch rejects R074. A correctness repair must add a directed
regression and may not change the frozen selector or threshold.

Gate A passed. The first directed run caught and fixed a tuple-order error in
the opaque backoff lookup before any timing; q1/q32 now exercise marks and
bypasses, while q1024 proves long samples stay eligible. All configurations
match complete architectural state and the exact ordered policy fingerprint.
Dirty-page invalidation clears stale entries, reset relearns them, and
generated-entry and partial-WFI handoffs are exact.

Direct/OpenSBI Linux reach the shell and execute 52.422M/55.327M sampled
instructions, with 250/276 short marks, 594,074/635,869 bypasses, and zero
errors. The strict eight-stage Nix suite passes 134 ISA cases, 109 Spike
locksteps, 193 architecture signatures, all Cargo/Wasm/system/T2 tests, and
virt smoke. The exact archived Gate-B Wasm is
`target/bench/wasm-candidates/r074-short-backoff-28ceaf7bcf63b726.wasm`,
SHA-256 `28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c`.

## Gate B: same-Wasm native product timing

Run two alternating gross Boot pairs first. Stop if candidate Boot regresses
more than 5%, exact output/lifecycle differs, static errors are nonzero, the
module count differs, or the backoff never marks and bypasses an entry.

Otherwise run five entirely fresh alternating pairs for Boot, Compile, and
Python on CPUs 8-15 using exact Linux 6.12.7 / Alpine 3.24.1 inputs. Advance
only if:

- Boot paired-median speedup is at least 1.10x and exact paired-bootstrap 95%
  lower bound is at least 1.00x;
- Compile and Python paired medians are at least 0.97x;
- candidate sampled retirement, short marks, and bypasses are nonzero, while
  control sampled retirement/marks/bypasses are zero;
- residual static stays disabled, module lifecycle matches, outputs are exact,
  errors are zero, host spread is at most 1.25x, and within-side native spread
  is at most 1.25x.

No threshold, key, retry, affinity, row, or statistic changes after a complete
valid unfavorable result.

Gate B passed. The two-pair gross screen is
`target/bench/r074-short-backoff-gross-valid/config-ab-2026-08-09T08-11-14-427Z.json`,
SHA-256 `2674479ca8ef61dfd877a2a9e223c7a27223187faf460f4fffa970962d1945b2`;
Boot is 1.148x `[1.131,1.165]`, host spread is 1.010x, output and lifecycle
are exact, and candidate marks/bypasses are nonzero with zero errors. An
earlier invocation under `target/bench/r074-short-backoff-gross/` pointed its
artifact root at the wrong directory and every worker failed preflight before
running a guest. It is permanently invalid and contributes no timing sample.

The five entirely fresh pairs are
`target/bench/r074-short-backoff-native/config-ab-2026-08-09T08-17-04-186Z.json`,
SHA-256 `efc35eb25e289022f23872742554874fbf63501b7b297ef2bdb81d13a0f0b7a5`.
The preregistered verifier returns `R074_NATIVE_GATE_PASS`; paired speedups are
Boot 1.154x `[1.145,1.177]`, Compile 0.979x `[0.948,1.054]`, and Python
0.999x `[0.981,1.061]`, with 1.024x host spread, exact fingerprints and
lifecycle, nonzero candidate activity, and zero errors.

## Gate C: prospective browser nonregression

Only after Gate B passes, freeze a new hash-named archive/page. Use seven
alternating fresh Chrome processes/profiles/guests per side and three
phase-synchronized repetitions per unchanged `/shared/bench.py` phase. The
candidate is enabled when the harness obtains the emulator at the shell; this
starts with an empty backoff map and is stricter than a production-default
lifecycle that has already learned during Boot.

Every repetition must prove exact guest/checksum/artifact/browser/affinity
identity, production page policy, generated coverage, one equal prepared
module, candidate sampled retirement/marks/bypasses, zero corresponding
control activity, residual static off, and zero errors. Use each leg's median
of three and the paired median of seven ratios. Python, SHA-256, and shared 9P
must each have elapsed ratio at most 1.03 and exact paired-bootstrap upper
bound at most 1.10. No raw spread cap and no replaced leg.

A pass runs the same seven-by-three candidate/v86 guard with the established
1.10 upper-confidence limit.

The prospective browser sample is frozen as follows before collection:

- URL
  `http://127.0.0.1:8765/examples/v86-rv64-three-way-r074-917ddcad15a15fa6.html`;
- comparison-page SHA-256
  `bdc4827f2a9b86eee1ce4443a9914eae4ef8e5c4ff8329b81973343feccb1a64`;
- RV64 JIT archive SHA-256
  `917ddcad15a15fa6560c480b9b19ccc2d39ec52ceed65030c94c79f0805df2a9`,
  whose inner main Wasm is the exact Gate-B
  `28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c`;
- modern RV64 root, matched x86 root, and v86 archive SHA-256 values
  `274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb`,
  `09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320`,
  and `7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771`;
- Chrome `150.0.7871.186`, revision
  `0fcdce5f4fdec8d442d7df760cb541f1ca6e446d`, V8 `15.0.245.21`, and CPUs
  8-15;
- runner `tests/run-wanix-r074-pairs.mjs`, analyzer
  `tests/analyze-wanix-r074-pairs.mjs`, and verifier
  `tests/vs-v86/r074-browser-gate.mjs`; their shared runner, wrapper, shared
  analyzer, wrapper, and verifier SHA-256 values are respectively
  `fee36c4fe9f277c99820c1bccf5fca09d7379a52f4cb7436f199baae8b88dc09`,
  `8f5bcf452072ec825e776dc250fa975f8a5e74c71c4af0613e3a4ab20f18d9ca`,
  `e7c8e16cd8189191dbb314e1a51fdaf787ec48aca6ccfff7d6723b5b8e6a0a10`,
  `8c7d429361ab9d50e03fe2a01a7bb3d2b97fc748546ec86396ca75b51bca4c8b`,
  and `c71b6e68f86fcbfcc5bc7b9f09419b3838fe37a4ffec48e468a0b5bb105abc03`;
- WANIX smoke harness SHA-256
  `c3f32eee15012ecc53da541bb3e3b1bda798ae1983d2b3a1e1bcd90dcb4e7495`,
  which the runner checks before launching a leg; and
- immutable result directory
  `target/jit-policy-traces/wanix-r074-28ceaf7b-chrome150-20260809-config-ab/`.

One candidate-only functional browser smoke may be run before the formal
sample to prove the new public switch and counters are wired through the
archived Worker. It is not timing evidence and may not change the frozen
candidate, artifacts, browser method, rules, or sample after it succeeds or
fails.

The smoke completed all nine synchronized phase repetitions with the exact
guest and artifacts. Every phase recorded candidate backoff enablement,
nonzero marks and bypasses, and zero errors. The formal 14 legs then completed
without replacement, timeout, artifact drift, or semantic failure. Its report
is
`target/jit-policy-traces/wanix-r074-28ceaf7b-chrome150-20260809-config-ab/analysis.json`,
SHA-256 `d3cf3a102966b2b1d05d126e3c97697fe4353b6d864441605dd76762efb83572`;
the immutable protocol SHA-256 is
`76e610c097e4af151bdae05edf725f9e8e1744ccca7e411d68d080286423ec2f`.

The result is measurement-valid but fails Gate C:

- Python 1.010x candidate/control, interval `[0.995,1.021]`;
- SHA-256 1.002x, interval `[0.986,1.010]`; and
- shared 9P 1.068x, interval `[0.959,1.122]`.

Shared 9P exceeds both frozen limits. The backoff itself was active: compared
with R073's side medians it reduced shared sampled-static retirement from
15.740M to 8.213M and static sample calls from 439,866 to 29,656, while
recording 391,360 subsequent bypasses. It did not remove the lifecycle
divergence. R074 candidate/control side medians still execute 49.937M/46.210M
guest instructions and issue 10/6 page modules; host 9P service is only
1.006x. This localizes the remaining cost to guest/page-JIT scheduling and
module-admission feedback rather than external 9P service or unconditional
continued static decoding.

R074 is rejected without rerun, threshold/key tuning, candidate/v86, default
promotion, or an authoritative scorecard. Gate D is not run.

## Gate D: promotion and parity

Only after Gates A-C pass may this exact candidate become default. Rebuild the
default-on artifact, rerun the complete strict correctness suite, then run the
untouched 13-row three-way scorecard with three alternating fresh processes
per rewrite/legacy/v86 side.

Promotion requires a valid report, 13/13 versus legacy, at least R054's 11/13
versus v86 with no new loss, no row more than 5% slower than R054, and Boot at
least 1.10x faster than the same-Wasm control. Promotion still does not finish
the thread goal unless raw Boot and Compile both reach copy/v86 parity and the
browser benchmark remains nonregressed.
