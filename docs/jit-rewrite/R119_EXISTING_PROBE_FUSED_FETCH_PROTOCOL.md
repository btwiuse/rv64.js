# R119 Existing-Probe Fused Execute-TLB Protocol

Date: 2026-08-10
Status: rejected at the frozen native gate; evidence archived and exact
baseline restored

## Question

Can the scalar full-system interpreter reuse its existing execute-TLB tag
lookup to fetch directly from the permission-checked live RAM capability,
instead of converting the hit to a physical address and then repeating the
physical-bus RAM range, offset, and bounds path?

This is deliberately narrower than changing instruction decoding and
deliberately different from R020/R055. Those experiments put another recent-
page fetch cache/probe in front of the ordinary execute TLB. R055's additional
probe was locally fast but made real Boot slower. R119 may not add a second
tag comparison, another fetch-tag bank, a page-replacement rule, or a tunable
switch. It changes the payload consumed after the tag comparison that already
exists in every full-system `Cpu::step`.

The execute row remains keyed by the complete virtual page and architectural
fetch-permission context. On a successful slow translation, the row may be
published only when `Bus::jit_fast_off(va, pa, false)` proves that the entire
page is backed by stable live RAM. Its payload is then the native/Wasm pointer
offset rather than `pa - va`. A tag hit performs one unaligned little-endian
halfword load through that capability. A miss, page crossing, non-RAM mapping,
permission failure, or unsupported full-system bus follows the unchanged
authoritative translation and `Bus::fetch16` path.

No guest PC, physical page, opcode family, kernel symbol, binary identity,
benchmark phase, browser, or measured frequency may select the fast path.

## Frozen control and preliminary native evidence

The exact product/control identities are:

- `crates/rv64-core/src/cpu.rs`
  `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`;
- `crates/rv64-wasm/src/lib.rs`
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- loader `web/rv64.js`
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- runtime Wasm
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`,
  4,279,380 bytes;
- Linux 6.12.7 image
  `57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2`;
  and
- Alpine 3.24.1 initramfs
  `cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808`.

The preserved R110 collection uses these exact identities and is reused only
as read-only attribution. Its optimized `Cpu::step<VirtBus>` image is 17,280
native bytes and receives 3,621 samples / 7,877,928,712 period-weighted cycles:
24.8636% of main-thread period and 11.0581% of the full mixed Compile
collection. Exact disassembly boundaries partition that function as follows:

| Native PC band | Share of sampled `Cpu::step` period |
| --- | ---: |
| entry and existing execute-TLB path | 14.5705% |
| physical-bus first-halfword fetch | 6.5785% |
| length / compressed dispatch transition | 13.6652% |
| compressed instruction bodies | 32.7926% |
| 32-bit instruction bodies and exits | 32.3932% |

Hardware samples can skid across a nearby branch, so these bands establish
where current native work resides, not removable-cycle precision. Counting
only the first two bands gives a conservative diagnostic population of
5.2584% of R110 main-thread period. R119 does not claim that all of it is
removable and does not use R110's instrumented elapsed time as performance
evidence. It is enough exposure to justify one exact product candidate under
R104's verified-one-percent rule.

## Frozen attribution collection

Before editing product code, run exactly one fresh current-control Boot worker
under Linux perf, pinned to CPUs 8--15:

```text
perf record -k 1 -F 1999 -e cycles:u -g -- \
  taskset -c 8-15 node --perf-prof --max-old-space-size=4096 \
  tests/vs-v86/scorecard-v2-worker.mjs rewrite boot
```

Preserve the raw `perf.data`, V8 JIT dump, injected data and JIT ELF images,
worker output, command, and SHA-256 manifest. The worker must report the exact
identities above, public one-slice-per-turn cadence, modern guest identity,
successful boot output, nonzero generated execution, settled compilation, and
`measurementEligible !== false`. Do not rerun or replace this diagnostic based
on its result.

Report the complete optimized `Cpu::step<VirtBus>` share and the same native
bands above. This collection is attribution only: perf/JIT instrumentation
perturbs tiering and no elapsed milliseconds become promotion evidence.

## Attribution result and admission decision

The first profiler launch omitted the required `ARTIFACTS` environment
variable. The worker rejected it at module initialization, before creating an
emulator or producing a benchmark result. Its 172 startup samples and complete
error log are preserved as `setup-error-*`; they are excluded rather than
silently deleted. The single eligible launch then ran with
`ARTIFACTS=/home/darren/src/jit/target/bench` and completed successfully.

That eligible worker is `measurementEligible: true`, uses the exact frozen
runtime, loader, kernel and initramfs, and reports Linux 6.12.7 / Alpine 3.24.1,
the public scheduler cadence, settled compilation, 180,350,959 guest
instructions, 71,580,490 generated instructions, and 592,838 generated
dispatches. Its reported Boot duration is excluded from performance evidence
as frozen.

The authoritative diagnostic is
`target/bench/r119-existing-probe-opportunity/opportunity.json`
(`b4b695d11c864962541c10d580b821ff52a637681e31717094cd779f325a739d`).
The analyzer is
`tests/vs-v86/r119-existing-probe-native-census.mjs`
(`1b787772a3df1e517eefb8faa886634967de822d569e6b6614bd4310faeca468`).
It partitions all 2,474 optimized `Cpu::step` samples exactly:

| Native PC band | `Cpu::step` | all sampled cycles | main-thread cycles |
| --- | ---: | ---: | ---: |
| entry and existing execute TLB | 15.3403% | 5.7306% | 7.8920% |
| physical-bus fetch | 4.9668% | 1.8554% | 2.5552% |
| length / RVC dispatch | 9.6446% | 3.6029% | 4.9618% |
| compressed bodies | 38.1892% | 14.2660% | 19.6469% |
| 32-bit bodies and exits | 31.8590% | 11.9013% | 16.3902% |

Optimized `Cpu::step` itself owns 37.3562% of all sampled cycles and 51.4460%
of main-thread cycles. The physical-bus band alone has a 1.8554% whole-
collection impossible-elimination ceiling, so a 1% Boot result requires
removing about 53.9% of that band. The candidate removes its ordinary direct-
RAM range/offset/bounds path while retaining the existing tag proof; this is
plausible but not guaranteed. Sample skid and unavoidable load cost prevent a
speedup prediction.

This prospectively admits exactly the frozen candidate below. It does not
admit a second fetch cache, tag probe, decoder/layout edit, or follow-up tuning
if the product result fails.

## Frozen implementation

Implement exactly one default-on source candidate:

1. In full-system mode only, the existing `tlb_tag[Fetch]` remains the sole
   hot fetch-tag probe.
2. A published fetch hit stores a direct pointer offset in the existing
   `tlb_diff[Fetch]` slot. Load/store slots retain their current physical-diff
   meaning.
3. A slow successful fetch translation publishes the tag only when
   `jit_fast_off` proves a stable readable page. Publish the offset before the
   tag. A non-capable mapping remains uncached and uses `Bus::fetch16`.
4. Normal `Cpu::step` obtains both halfwords through the same complete helper,
   so the 0xffe split independently proves its second virtual page.
5. Diagnostic physical-address consumers (`run_traced` and mapping
   verification) use the authoritative slow fetch translation rather than
   interpreting the direct-pointer payload as a physical diff.
6. User-mode `FlatMemory`, data TLBs, fused data JTLBs, generated memory,
   scheduling, policy, interrupts, retirement, traps, and guest-visible state
   are unchanged.

Do not add a runtime A/B branch, new cache size, alternate indexing policy,
second-halfword shortcut, decoder change, predecode, or adjacent cleanup. Code
and object size are recorded costs, never standalone rejection criteria.

## Correctness and shape gate

Before timing, require:

- formatting and complete rv64-core, rv64-system, and rv64-dbt unit suites;
- deterministic release builds and `WebAssembly.validate`;
- public API, Worker API, interpreter-bypass, lifecycle, and WFI suites;
- existing integer/M/A/FP, memory, Sv39/MPRV, randomized state, dirty-code,
  and generated-execution differentials;
- directed execute-row tests for a same-page hit, tag collision, privilege
  mismatch, selective/global SFENCE.VMA, SATP remap, non-RAM fallback,
  instruction access/page faults, a compressed instruction at page offset
  `0xffe`, and a 32-bit instruction split at `0xffe`; and
- direct and OpenSBI modern Linux 6.12.7 / Alpine 3.24.1 boots.

Static/native inspection must confirm that optimized full-system `Cpu::step`
has one execute-tag comparison on its ordinary hit and a direct halfword load,
with no physical-bus range/bounds subtree on that hit. The candidate may grow
or shrink; bytes alone neither pass nor fail it.

### Gate A result

The implementation is confined to `crates/rv64-core/src/cpu.rs`, whose frozen
candidate SHA-256 is
`7121814faf1379b0904e95e43cad1c576f2eadb00a5abe9bc7355748e04d0eba`.
It adds no TLB bank, CPU field, runtime switch, selector, or benchmark-specific
condition. Five new directed tests make bus fetches observable and cover a
same-page capability hit, a non-capable fallback, a compressed instruction at
page offset `0xffe`, a 32-bit split fetch that proves both virtual pages, and
privilege-tag/SFENCE invalidation. Existing translation, tag-collision, SATP,
fault, and differential suites supply the remaining matrix above.

`cargo test -p rv64-core`, `cargo test -p rv64-system -p rv64-dbt`, and the
strict Nix invocation
`nix develop --command env REQUIRE_ALL=1 tests/run-all.sh` all passed. The
strict run includes 37 core, 76 system, and 53 DBT tests; 134/134 ISA tests;
109/109 Spike lockstep cases; 193/193 architecture signatures; the public and
Worker APIs; raw Wasm; integer/M/A/FP, memory, Sv39, randomized-state,
generated-execution, lifecycle, WFI, and page-policy coverage; plus direct and
OpenSBI Linux 6.12.7 / Alpine 3.24.1 boots. It ended with `ALL STAGES PASSED`.
The earlier non-Nix `REQUIRE_ALL=1` attempt is not counted because that shell
lacked the required RISC-V GCC and Spike tools.

Two independent fresh release builds are byte-identical:

- candidate SHA-256
  `41b94faa9020b3c900919505a754c8ffe362d79fe5b68e68dce796e17f85def5`;
- candidate size 4,282,567 bytes; and
- control size 4,279,380 bytes, making the recorded growth 3,187 bytes.

Both candidates pass `WebAssembly.validate`. The growth is included in R107's
construction debit and is not a correctness or performance verdict.

One separately instrumented candidate Boot was used only for native-shape
inspection and reports `measurementEligible: false`; none of its elapsed
timing is admissible. Its optimized full-system `Cpu::step<VirtBus>` is 17,472
native bytes. On the first halfword, the existing execute-tag comparison is at
offset `0x108`; its hit reaches offset `0x172`, reads the existing Fetch
payload at `0x172`, and performs the direct halfword load at `0x180` before
decode at `0x185`. The second halfword has the corresponding single comparison
at `0x1731`, payload read at `0x179b`, and direct load at `0x17a9`. Physical-bus
range and bounds checks remain only on the miss subtrees beginning at `0x18a`
and `0x17b3`. Thus the emitted native shape is the prospectively specified
one-probe path; it does not reproduce R055's extra cache probe.

The native decision implementation and self-test were also frozen before
timing. Their SHA-256 identities are
`4a9759c59b7da11e38221531bb062e46304c16d5106ed055674147b889169436`
and
`52a5cec80776bd428d9fea7fed8acddfad2e8d8c857e1412236a6c615a3971e2`.
The self-test reports
`R119 native gate selftest: verified target and protected-row decisions`.

## Construction and native promotion gate

Archive exact control and two independently byte-identical candidate builds,
plus source and harness identities, before timing. Apply R107's amortized main-
runtime construction debit to every candidate sample. Run exactly 15
alternating fresh-process pairs on Boot, Compile, and Python with the frozen
public cadence and CPUs 8--15. Retain all 90 legs; no retry, replacement,
threshold change, or historical pooling is allowed.

Boot is the sole target row. R104 admits the candidate only if:

- adjusted paired Boot speedup median is at least `1.01x` and its paired
  bootstrap 95% lower bound is at least `1.00x`;
- normalized fixed-work evidence agrees in direction;
- Compile and Python paired medians are each at least `0.99x`, and neither
  confidence interval establishes a regression;
- every identity, output, work, generated-execution, scheduler-cadence,
  affinity, and host-stability guard passes; and
- the candidate passes the complete correctness/shape gate above.

A point estimate over 1% without the confidence requirement is inconclusive,
not an acceptance. A verified 1--5% result is accepted; there is no 10%, 20%,
or byte-size veto.

## Browser, WANIX, and scorecard qualification

Only a passing native candidate advances. Run the fixed Chrome execution-Boot
gate and the qualified WANIX comparison including unchanged
`python /shared/bench.py`, SHA-256, and shared-9P rows. Then run the untouched
117-leg modern legacy/rewrite/copy-v86 scorecard. All exact artifact/root/output
and generated-coverage guards must pass; protected browser, WANIX, and the
other eleven scorecard rows may not regress under R104.

Promotion requires the qualified default-on build to remain positive after
its construction debit and all product guards. Otherwise archive the complete
result, remove the candidate, and restore exact
`d9f686a9ce4f...` before selecting another mechanism.

## Frozen native result

The one permitted construction run is valid and assigns a conservative
1.258935 ms candidate debit. The one permitted 90-leg runtime report is also
valid, retains every leg, reports no problems, uses CPUs 8--15, and has host-
probe spread 1.067733x. After the debit, its fixed 15-pair results are:

| Row | Adjusted paired median | Paired-bootstrap 95% interval | Normalized fixed work |
| --- | ---: | ---: | ---: |
| Boot | `1.012411x` | `[0.997859,1.015302]` | `1.012351x` |
| Compile | `0.984634x` | `[0.947863,1.016480]` | `0.984642x` |
| Python | `1.001649x` | `[0.981848,1.029909]` | `1.001612x` |

Boot clears the 1% point and normalized-work floors but misses the frozen
parity lower bound. Compile misses the protected 0.99 median floor, although
its interval does not establish a regression. Python passes its protection
rules. All integrity checks pass.

The prospectively frozen decision is therefore
`reject-at-native-gate-and-restore-baseline`. This is not a rejection because
the gain is only 1.24% or because the candidate grew 3,187 bytes. It is a
rejection because that target gain is not confidence-verified and the same
candidate's protected Compile median is 1.54% slower. Do not extend the sample,
retry an ordering, tune the capability helper, or advance to Chromium, WANIX,
or the scorecard.

The result is recorded separately in
`docs/jit-rewrite/R119_EXISTING_PROBE_FUSED_FETCH_RESULT.md`. Candidate source
and Wasm remain archived. The live CPU source and release Wasm are restored
byte-exact to `aec4b31434a6...` and `d9f686a9ce4f...`.
