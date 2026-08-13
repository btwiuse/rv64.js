# R091 Hot Region Scheduler Outline Protocol

Date: 2026-08-09  
Status: rejected at frozen native product gate; exact R085 restored

## Hypothesis

R088 assigns 16.898% of corrected-cadence Compile STEADY self time to inline
work in `run_system_jit`. R090 proves this is not the visually expensive
indirect-feedback path: all 612,605 Compile STEADY outer generated dispatches
were region calls and the feedback inventory was exactly zero.

The executed region call/return loop currently lives inside a 33,230-byte
Wasm specialization of `run_system_jit`. R081's native-code capture shows the
corresponding full-system scheduler as an 83,840-byte Liftoff body, while the
hot `Cpu::step` naturally reaches TurboFan. The hypothesis is that the giant
scheduler combines hot region dispatch with cold invalidation, policy,
translation, profiling, and interpreter paths and therefore leaves repeated
region boundary work in baseline engine code.

Move the exact generated-chain loop into one non-inlined helper. This is code
layout, not scheduler thinning: every call, dispatch probe, map-generation
check, PA validation, feedback observation, region-exit sample, diagnostic
sample, retirement update, zero-progress exit, fuel refresh, and interrupt
boundary remains in the same order and at the same frequency.

## Frozen control and implementation

The immutable control is promoted R085 Wasm
`efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`;
loader is `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.
The corrected authoritative baseline is `1d822f1c1f37...`. Pre-edit source
identities include:

- `crates/rv64-wasm/src/lib.rs`
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- `crates/rv64-dbt/src/lib.rs`
  `fed2e33326d5e5c06d60341b013ff0a7fd63be4afd7c9fd7fdc3ebdf1095b262`;
  and
- `crates/rv64-dbt/src/wasm.rs`
  `b5e9c11ec1bfa1e92245e6bac003c4af0c6bac4b813d58344d8276940d6a1e99`.

Make one source candidate in `rv64-wasm` only:

1. extract the complete loop beginning with map-generation/state-pointer
   capture and ending after zero-retirement handling into a generic
   `#[inline(never)]` helper;
2. pass the exact round budget and return retirement plus dispatch count in a
   fixed scalar packing whose ranges are statically asserted;
3. leave bookkeeping after the loop in `run_system_jit`, unchanged; and
4. add no switch, selector, threshold, special PC/opcode, duplicated fast path,
   changed generated module, or additional host call.

Do not separately outline, inline, tune, or alter cold paths after observing
the result. R091 tests one function-boundary hypothesis.

## Shape and correctness gates

Before performance:

1. release Wasm must validate with unchanged imports and exports;
2. both legacy and Virt `run_system_jit` bodies must contain a direct call to
   their corresponding helper, and the Virt scheduler body must shrink at
   least 10% from 33,230 bytes;
3. the helper must contain the generated indirect call, dispatch-line loads,
   map-generation comparison, fuel update, retirement-cell load, and
   zero-retirement branch; no operation may remain duplicated in the caller;
4. one fresh production Compile run under `--print-wasm-code` must show a
   naturally emitted TurboFan body for the Virt helper. If it remains
   Liftoff-only, the proposed causal mechanism did not activate and stops;
5. cold main-module compile/instantiate median over seven alternating fresh
   pairs may regress by no more than 5%; and
6. formatting, core/system/DBT/Wasm/API/Worker tests and the complete
   `REQUIRE_ALL=1 tests/run-all.sh` matrix must pass, including Spike,
   signatures, randomized ISA/A/FP/Sv39/memory/T2, WFI, wasmtime, direct and
   OpenSBI Linux, and modern virt smoke. At least three additional fresh modern
   Boots must reach the exact shell with nonzero generated execution.

## Performance and promotion gates

Build the candidate once and archive it by hash. Run five alternating fresh
Node/V8 pairs for Boot, Compile, and Python on CPUs 8--15 with exact modern
artifacts, production page policy, and R087's public one-slice cadence. All 30
legs must retain exact identity/input/output/JIT proofs, host spread at most
1.10x, and ordinary sample limits.

The candidate advances if either Boot or Compile has paired median speedup at
least 1.03x and paired 95% lower bound at least 0.98x. The other target and
Python must each have candidate/control elapsed median no greater than 1.03x.
This explicitly accepts an honest three-to-five-percent whole-row gain; no
20% per-change requirement exists.

If native passes, run the standing fresh-Chrome execution-Boot and WANIX
`/shared/bench.py` guards. Only then run the untouched corrected-cadence
117-trial legacy/rewrite/v86 scorecard. Promotion requires 13/13 versus legacy,
no v86 parity-count loss, no rewrite row beyond the 3% guard, and exact browser
Python non-regression. Stop and restore exact R085 at the first failed gate;
retain every negative result without a helper-boundary variant.

## Result and decision

The only candidate source is archived at
`target/bench/r091-hot-region-outline/source-candidate.tgz`, SHA-256
`98c836075e281c598423388a4a11f4004e3a701e1166023dbc65ca2d996a24d9`.
Its `rv64-wasm/src/lib.rs` identity is `2efe1882b26b...`. The immutable
4,279,181-byte candidate is
`target/bench/wasm-candidates/r091-hot-region-outline-a8f14136e7d2.wasm`,
SHA-256 `a8f14136e7d217f4e71aec2c52020f749c476ee2531268f0bab7adfff2e42c75`.

The independent shape report is
`target/bench/r091-hot-region-outline/shape-gate-v2.json`, SHA-256
`2c168f372701138d4a2582790215b169ad17b230721f763051f3d5dc75a9276b`.
Imports and exports are exact. Both schedulers move their two generated
`call_indirect` operations into one helper and call that helper once. The Virt
scheduler falls from 33,230 to 26,922 executable Wasm bytes, a 18.98% reduction.
The exact production Compile capture is retained as `native-shape.log`, SHA-256
`71477d20dd12b58bba165b391414f3866435f1f07c0db49b7ab2b3d7a050c210`.
It observes the Virt helper at function index 29 first as a 12,860-instruction-
byte Liftoff body and later as an 11,028-byte TurboFan body. The same run uses
the candidate hash, modern guest, production policy, public cadence, exact
Compile MD5 in all phases, and proves 891,714,944 generated instructions.

The seven alternating fresh-process cold report is
`target/bench/r091-hot-region-outline/cold-compile.json`, SHA-256
`79aede89ff8014e55c43aac143231c32ba5e29d24491bd513baabc90d6e0bcab`.
Median main-module compile is 4.317 ms control and 4.271 ms candidate, a
0.989x candidate/control ratio; the 1.05x cap passes. The complete pinned
`REQUIRE_ALL=1 tests/run-all.sh` matrix passes all eight stages, including 134
ISA tests, 109/109 Spike locksteps, 193 architecture signatures, randomized
JIT/A/FP/Sv39/memory/T2 checks, Wasmtime, direct and OpenSBI Linux, and modern
virt smoke. Three additional fresh modern Boots pass in report
`target/bench/r091-hot-region-outline/boot-correctness/scorecard-v2-2026-08-09T21-07-58-840Z.json`,
SHA-256 `06fabf55ee7512332a458ba5732202cf715c671d3513e09861da4b7258dee695`.
Every process reaches the exact Linux 6.12.7/Alpine 3.24.1 shell with candidate
identity, production policy, public cadence, and nonzero generated execution.

The decisive valid 30-leg report is
`target/bench/r091-hot-region-outline/native-ab/config-ab-2026-08-09T21-12-56-255Z.json`,
SHA-256 `9a69bb475fb63a29067cdcb71b17b7e41021793a4be7732301efeba7ee3591d2`.
It retains all five pairs per row, has no input/output/policy/identity problem,
and records 1.019x host-probe spread. Paired results are:

- Boot 0.975x, interval `[0.960,1.020]`, with side medians
  2,188.091/2,212.678 ms;
- Compile 1.009x, interval `[0.950,1.070]`, with side medians
  943.425/932.571 ms; and
- Python 1.010x, interval `[0.978,1.038]`, with side medians
  2,402.676/2,423.445 ms.

The independent gate is `native-gate.json`, SHA-256
`0aaaee0ee680fe4ad0d4c839b155f532fcad9a93b78afcf7bc85385e42ac3781`.
All integrity and 3% elapsed guards pass, but neither Boot nor Compile reaches
the prospectively accepted 1.03x median. R091 is therefore rejected without a
helper-boundary/inlining variant, browser run, or full scorecard. The result is
causally useful: the large scheduler really was denied optimized-tier code,
and outlining really made its executed loop TurboFan-eligible, but that engine
transition changed neither remaining product row materially.

Production source is restored exactly to `1da35e70bc9c...`; the rebuilt release
Wasm is byte-identical R085 `efd7830307ef...`, and loader remains
`2cbb264f4dac...`. Goal status remains the corrected 13/13 versus legacy and
11/13 versus v86 baseline, with Boot and Compile open.
