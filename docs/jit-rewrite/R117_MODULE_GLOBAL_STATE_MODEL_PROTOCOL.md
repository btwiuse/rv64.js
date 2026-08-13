# R117 module-global architectural-state model

Date: 2026-08-10  
Status: closed at frozen Gate A; no native capture or product change

## Question

Can module-owned mutable Wasm globals provide cheaper spill storage for
long-lived architectural state than V8's large native stack frames, while
remaining inside one generated module/function? R110 measures an 8.87%
whole-Compile ceiling in optimized guest-body frame traffic; R113 shows that
traffic is distributed through general/control body code. R111's function
partition and R116's provably free cold-state subset are closed.

This mechanism keeps every CFG edge and function boundary unchanged. It is
not source-local reuse, a carried call ABI, a selected register set, or a
workload/PC policy. A module instance already belongs to one emulator and is
not invoked reentrantly, so its private unexported globals have the same
instance isolation as its function locals.

## Frozen model

Emit two deterministic modules from one Rust builder. Each exports
`run(memory_base:i32, iterations:i32) -> i64`, initializes 31 RV64-like i64
architectural values from linear memory, executes the same bounded structured
integer/control kernel, writes the 31 values back, and returns the same
checksum.

- control stores the 31 long-lived values in ordinary function locals;
- candidate stores those exact values in 31 defined mutable i64 globals;
- candidate declares only the two i64 temporaries and one i32 loop local; it
  does not retain dummy architectural locals;
- loop/control/temporary locals, memory, instruction order, constants,
  iteration count, output writes, and checksum are otherwise identical;
- globals are private, unexported, and initialized by `run` on every call;
- no imported helper, engine flag, forced tier, or benchmark-specific input is
  present.

The kernel uses every state value before and after a structured conditional
phase so values remain broadly live rather than forming 31 independent tiny
loops. The builder validates both modules, records indexed state-access and
operator counts, and emits normalized function-operator streams that must be
byte-identical after replacing architectural local access with global access.

## Frozen measurement

Run 15 alternating fresh Node/V8 pairs on CPUs 8--15. Each process compiles and
instantiates one immutable module, verifies first-call state/checksum, performs
eight warm calls separated by event-loop turns so ordinary tiering may publish,
then records seven fixed-work steady samples. Pair inputs and useful iteration
counts are exact. Do not stop early, replace an outlier, force Liftoff or
TurboFan, change the kernel, or try a partial-global width.

The exact memory base is 4,096. The exact call schedule is one 4,096-iteration first call, eight 16,384-
iteration warm calls, and seven 65,536-iteration steady calls. Every call
starts from the same 31-value memory image. Pair 0 runs control then candidate,
pair 1 reverses that order, and the order continues alternating. CPU assignment
cycles through 8--15. Each leg is bracketed by the frozen host probe; side
spread is computed across the 15 per-process medians of the seven steady calls.

Gate A passes only if:

1. module validation, output, final state, work, and deterministic-byte proofs
   all pass;
2. the candidate contains exactly 31 mutable i64 globals and replaces every
   architectural `local.get/set` with the corresponding `global.get/set`;
3. host probe spread is at most 1.10x and each side's sample spread is at most
   1.25x;
4. steady paired median speedup is at least 1.15x with 95% median-interval
   lower endpoint at least 1.00x; and
5. first-call paired median is at least 0.99x.

The 1.15x local threshold is prospective: applying it only to R110's measured
8.87% upper-bound population projects about 1.013x whole Compile before
construction. Model bytes and compile time are reported but are not standalone
vetoes.

If Gate A passes, collect natural optimized-native code for both model bodies
and require at least a 20% frame-byte and explicit-stack-operator reduction
before any product work. If either gate fails, stop without a product edit or
representation variant.

## Result

The builder emitted two deterministic, validating modules and independently
decoded their shapes. After normalizing the state-storage instruction, their
963-operator function streams are byte-identical. The control has 33 i64
locals and no globals; the candidate has two i64 locals and exactly 31 private
mutable i64 globals. All 160 indexed state reads, 95 indexed state writes, 31
memory reads, and 31 memory writes match. The candidate is 2,083 bytes versus
1,926 bytes for control; size is diagnostic only.

All 30 CPU-pinned fresh-process legs and 210 steady calls completed with exact
schedule and output/state fingerprints. Host-probe spread is `1.027687x`,
control/candidate side spreads are `1.019694x`/`1.087485x`, and maximum
within-process spread is `1.109024x`. All are inside their frozen limits.

Control/candidate steady medians are 1.252402/1.290324 ms. The paired candidate
speedup is `0.970330x` with exact 95% median interval
`[0.967047,0.973196]`. First-execution paired speedup is `0.826972x`
`[0.814919,0.842724]`. The global form is therefore about 3% slower after
ordinary tiering and about 17% slower on first execution; it misses every
performance condition in Gate A.

Report `target/bench/r117-module-global/gate.json` has SHA-256
`2fddcfd93b8157112f7b89e2c388fe3f809b8fad8a17844f0a8333bf6f71e2e7`.
Stop before native-code capture, product implementation, construction debit,
browser, WANIX, or scorecard work. Do not try a partial-global width or select
registers after this result. The live product remains exact `d9f686a9...`.

## Product path if admitted

Freeze one default-off same-artifact implementation that places all generated
architectural x/f/fcsr state—never PC, retirement, fuel, selectors, memory
proofs, or SSA temporaries—in private module globals. Preserve precise exits,
helper synchronization, module-instance isolation, tail-chain state transfer,
and every ABI. Then follow complete differentials, direct/OpenSBI Linux, R107
construction debit, 15 native Boot/Compile/Python pairs, normalized work,
Chromium, qualified WANIX including `/shared/bench.py`, and the untouched
scorecard. R104's verified-1% and protected-row rules decide promotion; size
alone does not.
