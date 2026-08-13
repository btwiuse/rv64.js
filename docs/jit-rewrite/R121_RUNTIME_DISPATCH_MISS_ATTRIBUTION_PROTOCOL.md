# R121 Runtime Dispatch-Miss Attribution Protocol

Date: 2026-08-10
Status: diagnostic complete; dispatch-miss variants closed below the verified
1% net-gain floor; exact product restored

## Question

Does the current full-system JIT lose at least one percent of modern Compile
to avoidable outer-dispatch misses, and, if so, are those misses caused by
global mapping-generation changes rather than new code, direct-table
collisions, invalid code mappings, or ordinary cold execution?

This is a diagnostic experiment. It may identify one architecture-general
candidate, but neither profiler time nor instrumented elapsed time is
performance evidence. In particular, the visible size of the inlined
`HashMap::get` implementation is not evidence that it is dynamically costly.

## Frozen product and evidence

The unchanged product/control is:

- `crates/rv64-core/src/cpu.rs`
  `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`;
- `crates/rv64-wasm/src/lib.rs`
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- runtime Wasm
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`;
- loader
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- Linux 6.12.7
  `57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2`;
  and
- Alpine 3.24.1 scorecard initramfs
  `cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808`.

Phase A reuses only R110's immutable exact-product data:

- `perf.data`
  `79abdf40afbf492c2718bb5832758df1ed2e137d18329d543c79c5c0c9196809`;
- injected perf data
  `6b976f6ebe1b38b6a41b451eff4178cc9313e13a8fc5b29d5588f2bda24d162b`;
- optimized Virt `run_system_jit` image
  `5febd1122f8c440958607f119082656b6a0d9ed91fdc9f6564208fccab46a8d6`;
  and
- R110 worker result, which proves exact modern inputs, public one-slice
  cadence, correct Compile MD5 in all phases, settled compilation, and
  generated execution.

R110 sampled the complete Compile process under perf/JIT logging, so its
elapsed durations remain excluded. The native samples are attribution only.

## Phase A: exact native closure

Parse every `cycles:u` sample from R110 and identify the one optimized Virt
`run_system_jit` symbol. Validate its 66,432-byte size and partition every
sample in that symbol by the following prospective symbol-relative bands:

| Begin | End | Meaning |
| ---: | ---: | --- |
| `0x0000` | `0x351c` | setup, invalidation, policy, and pre-dispatch work |
| `0x351c` | `0x3652` | fuel plus direct dispatch-line lookup |
| `0x3652` | `0x3fbf` | authoritative cache lookup, mapping proof, and line refill |
| `0x3fbf` | `0x4072` | ordinary block-call entry |
| `0x4072` | `0x51b2` | ordinary-block feedback path |
| `0x51b2` | `0x5273` | production region-call entry |
| `0x5273` | `0x5a73` | common post-call and diagnostic bookkeeping |
| `0x5a73` | `0x5c0a` | dispatch break/exit |
| `0x5c0a` | `0x10380` | remaining interpreter, policy, and lifecycle work |

The boundaries come from control-flow destinations in the preserved native
image, not from sample density. The analyzer must close sample count and
period exactly across bands, report fractions of the complete collection and
main thread, and retain one-second temporal bins only as a distribution check.
The bins must not be named as benchmark phases because the perf stream has no
authenticated phase markers. Hardware skid means adjacent instructions are
attribution, not a removable-cycle claim.

## Phase B: exact miss-cause census

Only if Phase A leaves at least a one-percent plausible late-run main-thread
ceiling, make one diagnostic-only source build. Add monotonically increasing
`u64` counters, with no behavior switch, for exactly:

1. dispatch loop visits;
2. verified direct-line hits;
3. empty-line PC misses;
4. occupied-line PC collisions;
5. matching-PC unverified-generation misses;
6. matching-PC stale-generation misses;
7. authoritative-cache compiled-block hits, blacklist hits, and absent-key
   misses after a line miss;
8. successful and failed self-page mapping proofs;
9. region-page proof visits and failures; and
10. successful dispatch-line refills.

Read the counters before and after FIRST, PRIME, and STEADY using the existing
scorecard counter boundary. Also add the already-exported mapping/SATP/TLB/
SFENCE counters 89--95 to that boundary; reading them after a phase is outside
the timed guest interval and is a permanent harness correction, not a product
optimization.

Run one fresh diagnostic Compile worker on CPUs 8--15 with the exact modern
workload and public cadence. It must retain exact output MD5, guest identity,
generated-execution proof, settled compilation, and eligible workload status.
Counter instrumentation may affect time, timers, tiering, and sample shape, so
all diagnostic elapsed values are excluded and the run may not be repeated
based on its result. Archive the instrumented source, Wasm, worker result,
commands, and SHA-256 manifest, then restore and rebuild the exact product
identities above before any product experiment.

Required counter identities include:

- loop visits = direct hits + empty misses + collision misses + unverified
  misses + stale-generation misses;
- every fallback = compiled-block hit + blacklist hit + absent-key miss;
- every successful fallback cache hit ends in a mapping failure or refill;
- refill count = successful self proof with all required region proofs; and
- all phase deltas are nonnegative and close independently.

## Admission rule

R121 can admit at most one product mechanism. Admission requires all of:

- the measured fallback path has a credible whole-Compile exposure above the
  verified 1% promotion floor; a point estimate at or below 1% is insufficient;
- stale-generation cache hits dominate the avoidable fallback population;
- failed mappings, cold cache misses, and direct-table collisions do not
  explain the exposure;
- the proposed mechanism preserves authoritative VA-to-PA validation after
  SATP/SFENCE and dirty-code events;
- it is general across address spaces and guest software, with no PC, page,
  opcode, symbol, binary, phase, browser, or workload selector; and
- an explicit Amdahl projection based on measured exposure can reach at least
  1.01x without assuming the complete path becomes free.

If the native ceiling or counter population is too small, close dispatch-miss
elimination without implementing or timing a product variant. If admitted,
freeze a separate correctness, construction-debit, native A/B, browser/WANIX,
and untouched-scorecard protocol before editing product code. Code size is
recorded as a diagnostic cost and is never a veto.

## Result

### Phase A: native exposure

The deterministic analyzer is
`tests/vs-v86/r121-runtime-dispatch-native-census.mjs`
(`e81b5f3b64705b7e1cdcb4716fc79aa578851fcb7aed1ae902f6c6cbc76abe8a`).
Its two independent outputs are byte-identical. The authoritative report is
`target/bench/r121-runtime-dispatch/native-census.json`
(`a215043e272d3f5415059c3849a08ec33b16bb5142529aa712c0ab78ec53989c`).
It reproduces all 34,753 R110 samples, `71,241,130,879` total period, and
`31,684,540,743` main-thread period, then closes all 381 optimized-scheduler
samples and `825,328,724` scheduler period exactly across the frozen bands.

Optimized Virt `run_system_jit` owns 1.1585% of the complete mixed-process
period and 2.6048% of main-thread period. Its relevant bands are:

| Native band | Scheduler | Main thread |
| --- | ---: | ---: |
| fuel and direct-line lookup | 8.3749% | 0.2182% |
| fallback cache/mapping/refill | 28.7183% | 0.7481% |
| ordinary call plus feedback | 0.7922% | 0.0206% |
| production region-call entry | 6.7972% | 0.1771% |
| common post-call work | 1.5846% | 0.0413% |

Even making all five executed dispatch bands free has only a 1.2052%
main-thread ceiling in this complete collection. The actual proposal can
remove only part of the 0.7481% fallback band: the line lookup, generated call,
mapping proof, and required post-call work remain. One-second bins reach
1.03--1.42% fallback share late in the process, which satisfied the frozen
condition for cause counting. They are not authenticated FIRST/PRIME/STEADY
boundaries and therefore cannot establish a target-row exposure or admit a
candidate.

The two hottest scheduler PCs are both in the first authoritative-cache Swiss
table lookup (`+0x3721` and `+0x37a5`), jointly 14.18% of sampled scheduler
period. This proves where miss work lands; it does not make that work large
enough end to end.

### Phase B: miss causes

The sole diagnostic build is Wasm
`a6685df5fe127678a2c4d11becf81140a5a784fd6ee71e42bcecaeadf992698e`
(4,280,217 bytes). The frozen collector is
`tests/vs-v86/r121-dispatch-counter-census.mjs`
(`381a47a21a653bcdfa51389060e64b691d7e179885e987d62040fcf37706f384`).
The authoritative counter report is
`target/bench/r121-runtime-dispatch/counter-census/counter-census.json`
(`0170353c869781955822d215a98317089b2d5563052ddb2ad871cf599358895b`).

The worker is an eligible exact modern Compile workload: Linux 6.12.7,
Alpine 3.24.1 riscv64, public one-slice cadence, exact MD5
`24eedf7e06beffd4d3ba1945585588db` in all three phases, settled compilation,
904,236,746 generated instructions, and 1,828,555 generated dispatches. Its
instrumented elapsed values are excluded. Every per-phase visit, miss reason,
cache outcome, self proof, region proof, refill, and drop identity closes.

| STEADY dispatch cause | Count | Share |
| --- | ---: | ---: |
| all outer visits | 924,436 | 100% |
| verified direct hits | 353,961 | 38.289% |
| all fallbacks | 570,475 | 61.711% |
| empty-line miss | 349,171 | 61.207% of fallbacks |
| stale-generation miss | 213,625 | 37.447% |
| occupied-line collision | 7,498 | 1.314% |
| unverified publication | 181 | 0.032% |

The authoritative lookup finds a compiled block 213,808 times and no block
356,667 times; no blacklist is encountered. Of those 213,808 compiled-block
proofs, 213,807 refill successfully and exactly one mapping is actually
dropped. Region proofs visit 51,690 pages with zero failure. Thus correctness
validation is effective but almost never rejects during this workload.

STEADY records 3,143 mapping invalidations, of which 3,118 are selective page
SFENCEs; changed SATP is only nine. The global `map_gen` stamp consequently
causes about 68 stale compiled-line refills per invalidation. That explains
the stale population but does not enlarge its measured native ceiling.

### Decision and restoration

Reject a negative absence line, a parallel-PA generation refresh, and a
combined richer dispatch row before product implementation. A negative line
could avoid only some of the absent-key hash lookups and would require
publication invalidation plus a negative-aware interpreter re-entry predicate.
A parallel PA/region capability could avoid some stale hash work but must keep
the actual mapping proof. Combining both can at most approach the complete
0.7481% fallback band in the authenticated full collection, before its extra
loads, table footprint, collisions, first misses, and mandatory proofs. The
unauthenticated late-bin point estimate is too close to 1% to support the
required verified net 1.01x result.

This is an exposure rejection, not a code-size rejection. No product variant
was implemented or timed. The diagnostic source and Wasm are archived, all
counter code is removed, and current source/release identities are restored
exactly to `1da35e70...` / `d9f686a9...`; loader remains `2cbb264f...`.

The scorecard harness permanently reads already-exported MMU counters 89--95
at phase boundaries. Those reads occur outside the measured guest interval
and improve future causal reports without changing the emulator. R121 does
not close the broader post-dispatch policy/interpreter region; it requires a
separate operation-level attribution rather than another dispatch variant.
