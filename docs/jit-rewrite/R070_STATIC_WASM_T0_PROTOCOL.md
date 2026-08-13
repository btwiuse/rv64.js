# R070 Static Hand-Shaped Wasm T0 Protocol

Date: 2026-08-09  
Status: Phase A passed; Phase B rejected at the frozen Boot gate

R070's original decision remains final under this protocol. A later
project-wide change to the economics of small cumulative gains is evaluated as
the separate R071 experiment using entirely fresh evidence; see
`R071_CUMULATIVE_GAIN_CONFIRMATION_PROTOCOL.md`. It does not retroactively
turn the three R070 pairs into a pass.

## Question

Can one small, architecture-complete, hand-emitted WebAssembly interpreter
core execute the ordinary RV64 scalar path materially faster than the
LLVM-shaped Rust interpreter, without compiling guest pages during execution?

R067 proves that the accepted Boot residual is an execution-core problem:
R066's fully integrated Rust scalar drivers own 62.19% of sampled Boot time,
while 111.363 million instructions still execute in T0. R066 nevertheless
improved the complete Boot row by only 7.4%. Its two VirtBus drivers are
6,138/6,331 native bytes, have 28/30 locals and 22 branch tables, and retain
LLVM's representation of the Rust decoder and `Cpu` state.

R070 changes that representation. It emits one small auxiliary Wasm module
once, installs its `run(i32)` entry in the existing shared function table, and
calls it Wasm-to-Wasm. The module dynamically fetches guest instructions but
uses a deliberately flat instruction selector, loop-local PC/retirement/fuel,
one compact outcome word, and direct memory capabilities. Slow or faulting
instructions exit before retirement to the unchanged architectural Rust
interpreter.

This is distinct from the closed mechanisms:

- R023/R066 asked LLVM to inline or integrate the Rust decoder;
- R025-R027/R045 cached decoded instructions but retained a dispatched
  per-instruction executor and architectural state updates;
- R047/R053 precompiled exact opcode sequences or pair handlers;
- R046/R069 compiled guest-page-specific Wasm while Linux was running; and
- R058 changed only the non-inlined `Cpu::step` result ABI.

The R070 module has no guest PC, symbol, binary hash, workload, browser, or
scorecard selector. It is compiled once per emulator instance, not once per
guest page or hot region.

## Frozen semantic surface

The fast core must implement every ordinary integer/control/scalar-memory
instruction admitted by R066:

- RV64I integer, control, and all scalar load/store widths;
- RV64M, including divide edge cases and all high-multiply forms; and
- every legal integer RV64C form.

F/D, A, FENCE, SYSTEM/CSR, compressed floating point, access failures, and any
unrecognized encoding exit before the instruction and execute through the
existing complete interpreter. An isolated exit executes exactly one
instruction there. If the next static-core invocation immediately encounters
another unsupported instruction without retiring a fast instruction, the
interpreter owns a fixed 64-instruction stretch; this opcode-independent rule
amortizes crossings through unsupported regions. Family membership and the
64-instruction constant may not be reduced or retuned after timing.

The initial opportunity implementation uses user-mode flat memory so the
execution shape can be compared against the exact current interpreter without
mixing in Sv39 or device behavior. A system candidate, if admitted, must use
the existing execute and fused data-TLB capabilities, return for interrupt and
device boundaries, stop exactly at a generated entry, and retain all current
page-policy sampling and WFI/SBI behavior.

## Phase A: independent opportunity gate

Before changing the system scheduler:

1. deterministically emit and validate identical auxiliary-module bytes;
2. statically confirm one defined driver, no guest-specific functions, no
   indirect handler dispatch, and the frozen scalar family surface;
3. compare candidate and current interpreter on directed RV64I/M edge cases,
   all 49,152 actual compressed-prefix encodings, randomized register/memory
   states, unaligned and boundary accesses, x0 writes, branches and links;
4. compare complete user-mode guest exit, output, PC, GPR/FPR/FCSR, retirement,
   and memory fingerprints with the JIT disabled; and
5. run at least seven alternating fresh-process measurements over independent
   ALU/control, scalar-memory, mixed-RVC, and held-out general-program corpora.

The auxiliary module's compile plus instantiate median must be at most 25 ms.
The paired geometric throughput improvement over the exact current interpreter
must be at least 1.30x with a paired-bootstrap 95% lower bound at least 1.20x
on every broad corpus; no corpus may regress more than 3%. These margins are
intentionally higher than the simple 1.20x local projection because the system
form adds translation, interrupt and policy boundaries.

Failure removes the emitter/runtime hooks without a dispatch-layout,
instruction-family, warmup, engine, or corpus-weight sweep.

## Phase B: default-off modern-system candidate

Only a passing Phase A admits system integration. Both A/B legs use one
candidate main Wasm. The auxiliary module is constructed before the measured
guest phase in both legs so the control cannot hide or add lifecycle work;
only its table invocation differs.

Correctness must cover the complete core, DBT, Wasm, full-system memory,
Sv39/MPRV, FP/FS, atomic, WFI, T2, direct-Linux and OpenSBI-Linux matrices. In
particular, a fast instruction may retire only after all fetch/data proofs
succeed, a store must preserve generated-code invalidation, and an interrupt
or exact generated-entry transition must observe committed PC and retirement.

Run one Boot gross screen and stop on a correctness difference or greater than
5% regression. Otherwise run three alternating fresh-process pairs for Boot,
Compile and Python. Advancement requires:

- Boot paired median speedup at least 1.10x with lower bound at least 1.00x;
- Compile and Python each at least 0.90x;
- exact input/output/retirement and generated-execution fingerprints; and
- valid unchanged host-stability probes.

A passing candidate then requires five Boot/Compile pairs, the browser
`/shared/bench.py` guard, and the untouched three-way scorecard. Promotion is
allowed only if no row regresses 10%. The thread goal remains unachieved until
both Boot and Compile meet copy/v86 parity.

## Frozen result

Phase A passed without changing the semantic surface. The authoritative
seven-pair report is
`target/bench/r070-static-t0-phase-a/static-t0-user-2026-08-09T04-52-09-450Z.json`.
Paired throughput was ALU/control 2.154x `[2.088,2.225]`, scalar memory
2.445x `[2.355,2.527]`, independent mixed RVC 1.873x `[1.840,1.893]`, and the
held-out integer/FP program 1.624x `[1.619,1.631]`. Construction was about
0.10 ms. All 49,152 actual compressed prefixes, 65,536 broad scalar cases,
complete architectural fingerprints, memory boundaries, and deterministic
module checks passed. The admitted flat auxiliary remains SHA-256
`f0e0540c53e6ddc77a385fbe91d75b55a4fcfa81340bf7ad4d7774b5584df818`
at 7,741 bytes.

The default-off full-system form then passed 32 core, 56 DBT, 76 system,
Wasm smoke, memory, Sv39/MPRV, A, FP/FS, WFI, T2, and modern direct/OpenSBI
Linux gates. It also exposed and fixed an independent legacy page-policy bug
that discarded a real WFI yield and could spin a caller's full budget.

The decisive same-main-Wasm report is
`target/bench/r070-static-t0-phase-b-screen/config-ab-2026-08-09T05-28-06-677Z.json`.
It is valid, uses three alternating fresh pairs on CPUs 8-15, has exact
inputs/outputs, and records 1.015x host spread. Paired results are:

- Boot 1.047x `[1.001,1.061]`;
- Compile 0.995x `[0.931,1.035]`; and
- Python 1.052x `[1.050,1.063]`.

R070 therefore fails the required 1.10x Boot median despite a real positive
effect and passing both guards. Do not relax the gate, change the 64-insn slow
stretch, select instruction subfamilies, or run the five-pair/browser/full
scorecard escalation. Candidate Boot executes only 4.89M-6.86M fast R070
instructions while 105M+ page-policy-sampled instructions remain on the Rust
interpreter path. A successor may address that separately only by preserving
the policy's exact architecture-general observations, not by suppressing or
retuning sampling.
