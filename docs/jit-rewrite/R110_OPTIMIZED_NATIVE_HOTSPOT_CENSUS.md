# R110 Optimized Native Hotspot Census

Date: 2026-08-10  
Status: diagnostic complete; one proof-only local model admitted; no product change

## Question

Which native operations consume cycles inside the optimized generated-Wasm
functions during the modern Compile row? R088 established that generated guest
execution plus inline dispatch owns 57.6% of Compile STEADY, but its V8 CPU
profile cannot resolve a program counter within a Wasm function. Wasm operator
counts have repeatedly been misleading (R039, R048, R097, R101--R103), so this
experiment samples actual TurboFan machine code before selecting another
product mechanism.

This is an attribution run, not elapsed-time performance evidence. Perf and
V8 JIT logging perturb construction, tiering, and execution. No duration from
this run may be compared with R087 or used to claim a gain.

## Frozen input

- source baseline:
  - `crates/rv64-core/src/cpu.rs`
    `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`;
  - `crates/rv64-wasm/src/lib.rs`
    `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- runtime Wasm:
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`;
- loader:
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- Linux 6.12.7 image:
  `57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2`;
- Alpine 3.24.1 scorecard initramfs:
  `cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808`;
- Node/V8: `v26.5.0` / `14.6.202.34-node.24`;
- workload: the unchanged scorecard-v2 Compile input and public one-slice per
  event-loop-turn cadence.

## Frozen collection

First validate the V8/perf pipeline on the existing R103 two-module model.
The probe must produce separate JIT ELF images for both optimized
`wasm-function[1]` bodies and `perf annotate` must retain instruction offsets
and samples. Probe results only validate the instrument.

Then run exactly one fresh rewrite Compile worker under Linux perf, pinned to
CPUs 8--15. Record user-mode hardware cycles at 1999 Hz with call graphs while
Node writes its JIT dump. The worker must report:

- the exact identities above;
- MD5 `24eedf7e06beffd4d3ba1945585588db` in FIRST, PRIME, and STEADY;
- nonzero generated retirement and dispatch;
- public scheduler cadence;
- all generated compilation settled; and
- `measurementEligible !== false` (the underlying workload is authentic even
  though R110 excludes its perturbed time).

Inject the JIT dump into a copy of `perf.data`; preserve the raw data, dump,
injected data, generated ELF images, worker output, commands, and SHA-256
manifest. Do not rerun or replace the collection based on its hotspots.

## Frozen analysis

Each generated module defines exactly one function. Its index is the number of
function helpers imported before it (`fp_exec`, reservation, `tlb_fill`, bulk
copy, chain, and tail chain), currently zero through five. The main runtime's
defined functions begin at index 13, so optimized generated bodies are the
distinct low-index JIT images named
`JS:wasm-function[N]-N-turbofan`, `0 <= N <= 5`. Report both aggregate and
per-image cycles and validate this classification against the emitter's helper
count. Do not merge Liftoff and TurboFan code, runtime-Wasm functions, JS
wrappers, or V8 compiler threads into the generated-body denominator.

For every sampled generated-body instruction, classify the x86-64 mnemonic and
addressing form into these architecture-independent cost families:

1. guest arithmetic, comparison, and data movement;
2. guest-memory data access;
3. translation/capability proof and bounds checks;
4. architectural state load/store or spill/reload;
5. PC, retirement, fuel, structured selector, and internal dispatch;
6. cross-module tail transfer and indirect-target checks;
7. prologue, epilogue, trap, stack, or other engine machinery; and
8. unresolved.

Static disassembly may help label a sampled range, but only sampled cycles
establish dynamic exposure. Preserve per-instruction evidence and require at
least 90% of generated TurboFan cycles to be classified. A family can admit a
new experiment only if all of the following are true:

- it owns at least 5% of whole Compile STEADY after multiplying by R088's
  40.684% generated-execution share (or it jointly removes independently
  measured inline-dispatch work);
- the proposed removal is not an implementation already closed through R109;
- it is architecture-general and cannot select a guest PC, binary, symbol,
  workload, browser, or observed hotspot; and
- an ordinary optimized-V8 local model can prospectively demonstrate enough
  improvement to clear R104's verified 1% whole-row gate.

If no family qualifies, close the attribution without a product edit. Any
surviving candidate receives a separate frozen correctness/construction/native
protocol before implementation; R110 itself earns no parity credit.

## Collection result

The frozen worker ran once under:

```text
perf record -k 1 -F 1999 -e cycles:u -g -- \
  taskset -c 8-15 node --perf-prof --max-old-space-size=4096 \
  tests/vs-v86/scorecard-v2-worker.mjs rewrite compile
```

The worker is authentic and eligible: it records the exact runtime, loader,
kernel, initramfs, guest, and public cadence above; FIRST, PRIME, and STEADY all
produce MD5 `24eedf7e06beffd4d3ba1945585588db`; all compilation settles; and the
run executes 909,201,860 generated instructions with 1,862,759 dispatches.
STEADY retires 311,403,071 of 325,726,925 guest instructions in generated code
(`95.6024%`) and performs 8,536,570 chain hops.

Perf records 34,753 cycle samples. The analyzer maps 4,506 samples, with
9,819,517,893 period-weighted cycles, to the sampled low-index JIT path. The
collection covers the full Compile worker (boot plus FIRST, PRIME, and STEADY),
and perf/JIT logging perturbs it. Its reported milliseconds are excluded, as
required; no collection was rerun based on the observed native shape.

## JIT-dump and classification corrections

Node/V8 14.6 writes two malformed `JIT_CODE_DEBUG_INFO` record sizes in this
run. Stock `perf inject --jit` stops after 2,517 code loads, before most guest
modules. The direct reader validates each record and resynchronizes at exactly
two nearby timestamp-monotonic headers: `-6` bytes at offset 3,263,634 and
`-5` bytes at 3,269,278. It then parses the complete 84,465,372-byte dump:
5,985 records, 3,295 code loads, and 233 low-index generated/JIT-path loads.

The frozen low-index rule also included one non-guest function: the 128-byte
table-owning shared tail trampoline is named
`JS:wasm-function[0]-0-turbofan` too. Its disassembly has the table bounds and
signature checks followed by `jmp *(%r10,%rdx,8)`, so the final report labels
it separately rather than charging it to a guest body. The analyzer also keeps
long x86 instructions on one objdump line, splits AT&T operands outside address
parentheses, and treats memory-form `cmp`/`test` as reads. The initial
`native-census.json` and intermediate `native-census-v2.json` remain preserved
but are superseded by `native-census-v3.json`; they were analysis corrections,
not replacement collections.

The final analyzer partitions every mapped period across native opcode/address
families and asserts exact period equality across samples, code loads, roles,
and guest-body families. It is
`tests/vs-v86/r110-perf-native-census.mjs`
(`2bdfeaf8299aa3e7e7d72f2a4bc4ad392c31ea69d09482b7c7dff973cc4407a7`).
The authoritative report is
`target/bench/r110-optimized-native-census/native-census-v3.json`
(`4aab78cbb7a5dc2045b42abe420b42ed6ad9262a267b4e7029f137ce94ce971f`).

## Native result

TurboFan owns `91.7575%` of the sampled JIT-path period and Liftoff `8.2425%`.
The shared tail trampoline owns `2.7883%`; actual guest bodies own `97.2117%`.
Inside guest bodies, the largest exclusive native families are:

| Family | Guest-body share |
| --- | ---: |
| register moves | 18.588% |
| explicit native-stack reads | 16.568% |
| conditional branches | 14.635% |
| non-stack memory reads | 12.881% |
| address generation | 12.154% |
| integer arithmetic | 10.714% |
| explicit native-stack writes | 5.867% |

Explicit `%rbp`/`%rsp` reads plus writes therefore own `22.4351%` of sampled
guest-body cycles. This excludes `push`, `pop`, entry, and return machinery,
so it is a conservative measure of explicit frame/spill traffic rather than a
count of all host-stack activity. TurboFan guest bodies show essentially the
same `22.4230%` share. Their period-weighted frame is 323.84 bytes; frame size
and local stack-cycle share have a positive weighted correlation of `0.3840`.
Bodies with 129--256, 257--384, and over 512 frame bytes spend respectively
`18.445%`, `23.749%`, and `31.016%` of their sampled periods on explicit stack
traffic.

Applying the guest-stack share of the complete JIT path (`21.8096%`) to R088's
independently measured `40.684%` generated-execution share gives an `8.8730%`
whole-Compile exposure ceiling. This is not a predicted speedup: eliminating
all frame traffic is impossible, hardware-cycle samples skid, this collection
mixes phases, and a transformation adds boundary work. It does establish more
than the frozen 5% exposure required to ask whether a general mechanism can
remove enough optimized-native cost to clear 1%.

The trampoline's optimistic standalone ceiling is only about `1.13%` of
Compile. Replacing it with per-module table ownership is already closed by the
measured V8 instance-publication/construction failure recorded in D054, so R110
does not reopen that axis. Likewise, R039 already rejected source-local reuse
and R103 rejected a full-GPR cross-module ABI. Native spill evidence does not
retroactively make either implementation positive.

## Decision

R110 makes no product edit and earns no scorecard credit. It admits one R111
ordinary-V8 local model of architecture-general multi-function partitioning:
split a generated region by bounded CFG/liveness structure inside one Wasm
module, preserve direct same-module edges, and measure whether smaller
optimized frames reduce explicit stack cycles by enough to outweigh the added
function boundaries. The model may not select a guest PC, symbol, binary,
workload, engine tier, or observed hotspot, and it may not carry the fixed full
architectural state rejected by R103.

Product implementation remains forbidden unless the prospective model:

1. preserves exact work and output under ordinary tiering;
2. demonstrates smaller native frames and stack exposure, not just fewer Wasm
   locals or bytes;
3. projects at least a verified 1% whole-Compile gain after boundary cost; and
4. freezes one selector-free mechanism and all R104/R107 correctness,
   construction, native, browser, WANIX, and scorecard gates before timing.
