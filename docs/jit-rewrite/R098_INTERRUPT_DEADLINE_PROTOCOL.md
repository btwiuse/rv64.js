# R098 interrupt-poll deadline protocol

Date frozen: 2026-08-10

Status: rejected at the frozen native gate; exact evidence archived and the
R085-equivalent source build restored

## Question

Can the full-system interpreter preserve the existing interrupt observation
schedule while replacing its hot per-instruction countdown update with an
absolute retired-instruction deadline?

This is an architecture-wide runtime representation change.  It must not
change the 32 complete instructions between ordinary bus-line samples, and it
must retain the immediate post-instruction recheck caused by traps, xRET, and
CSR writes that can change interrupt eligibility.

## Evidence admitting one implementation

The exact R085-equivalent source build executes 108,874,500 interpreted
instructions during the profiled Boot row.  Its concrete `Cpu::run_until`
Wasm body loads `irq_poll_cd`, branches on zero, and on the ordinary path
subtracts one and stores the countdown for every instruction.  The corrected
R088 profile attributes 12.719% of Boot to the exact-reentry interpreter loop
and another 2.514% to the ordinary interpreter loop.  Compile also retains
15.94M--38.98M interpreted instructions per phase.

The candidate replaces the countdown with the low 32 bits of an absolute
`insn_count` deadline.  This retains the existing 32-bit cell and complete
`Cpu` layout.  An ordinary poll at retired count `n` arms `n + 33`, which is
exactly the current cadence: the poll instruction plus 32 complete intervening
instructions.  A half-range modular comparison is unambiguous because the
deadline is only 33 instructions ahead, including across low-word and complete
counter wraparound.  A trap, xRET, or CSR write arms the current count, so the
next loop iteration polls even if the current instruction retires.  No poll
interval, scheduler quantum, page/JIT policy, workload, guest identity, or
output contract changes.

This admits exactly one implementation.  Do not sweep intervals, selectively
arm by privilege or workload, weaken immediate CSR/xRET handling, or combine
the change with another interpreter/JIT mechanism.

## Required structural and semantic proof

1. Unit tests cover ordinary cadence, a pending interrupt, exception/trap
   re-entry, xRET, CSR interrupt-enablement, and `u64` retirement wraparound.
2. A baseline/candidate disassembly report identifies all four interpreter
   drivers and proves that the ordinary candidate path has no countdown
   decrement/store while retaining a call to `check_interrupts`.
3. Workspace tests, exhaustive ISA tests, Spike differentials, architectural
   signatures, focused JIT memory/FP/atomic/Sv39/WFI/lifecycle tests, public
   and Worker API tests, and direct/OpenSBI modern Linux boot pass.
4. Guest-visible outputs, checksums, instruction accounting, JIT policy, and
   generated execution remain exact in every performance leg.

## Performance sequence

Use exact R085-equivalent source as control and immutable candidate artifacts.
Record complete hashes and section/function shape; bytes are diagnostic, not
an acceptance threshold.

1. Five alternating fresh-process native pairs on the frozen Boot, Compile,
   and `/shared/bench.py` rows, with host spread and paired bootstrap intervals.
2. Continue only if Boot has at least a 1.03x paired median improvement and a
   non-regressing lower confidence bound; Compile and Python must each have a
   non-regressing lower bound.  This is the standing D073/D083 cumulative
   track, not a code-size gate.
3. If native passes, run fresh Chrome, the R094-qualified long shared-9P guard,
   and the untouched authoritative three-way scorecard in that order under
   their already frozen contracts.

Stop at the first failed gate, preserve exact evidence, remove the candidate,
and verify executable restoration.  A pass is a product candidate only after
the complete sequence; a static operator reduction alone is not a result.

## Result

The final candidate retained the existing 32-bit `Cpu` cell and complete
layout, using the low retired-count word as the modular deadline.  An earlier
proof build used a 64-bit cell, which Rust moved within its default-layout
struct; it was archived as `pre-shape-u64-layout-2bd6e287bebc.wasm` and was
never timed.  This pre-shape correction prevents unrelated field offsets from
contaminating the causal candidate.

The immutable candidate is 4,279,332 bytes at
`b2e2831bb7851f6ce0c2cd58fba6a9f6f78e77a9e7c428192ed3270968553453`;
the source-equivalent control is 4,279,380 bytes at
`d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`.
The 48-byte reduction is diagnostic only.  Shape report
`target/bench/r098-interrupt-deadline/shape-v2.json` (`c8ccaac68fbe...`)
finds the same six `run`, one `run_until`, one `run_until_observed`, and one
`run_traced` concrete bodies and identical direct/indirect call counts.  Every
control body has one interrupt-cell load and two stores; every candidate body
has one load, one rare arm store, and the modular deadline comparison.

Five focused cadence/trap/xRET/CSR/wrap tests passed before measurement.  The
complete strict Nix matrix then passed: 134 ISA tests, 109 Spike lockstep
tests, 193 architecture signatures, all workspace and Wasm/JIT differentials,
direct and OpenSBI Linux 6.12.7, and native virt-smoke.

The frozen 30-leg report is
`target/bench/r098-interrupt-deadline/native-ab/config-ab-2026-08-10T03-49-10-852Z.json`
at `6bda539f619e...`; host spread is 1.014x and every artifact, input, output,
production-policy, cadence, generated-execution, and modern-guest proof
passes.  Paired results are:

- Boot: 2,197.65 -> 2,158.28 ms, 1.020x `[0.988,1.032]`;
- Compile: 952.58 -> 944.85 ms, 1.008x `[0.974,1.089]`; and
- Python: 2,331.57 -> 2,444.74 ms, 0.948x `[0.930,1.053]`.

Gate `target/bench/r098-interrupt-deadline/gate.json`
(`dd90ff8dd414...`) is integrity-valid but rejects: Boot misses both its
1.03x median and 1.00 lower-bound rules, and Python misses the 3% elapsed
guard.  Stop before Chrome, WANIX, or scorecard; do not sweep poll cadence,
counter width, comparison spelling, privilege selection, or helper layout.
The source archive is `6a0a83298113...`.

All candidate CPU code and candidate-only unit tests were removed.  The release
rebuild is byte-exact control `d9f686a9...` at 4,279,380 bytes, and its 32 core
tests pass.  Historical protocol, shape/gate scripts, artifacts, and reports
remain for reproducibility.  R098 shows that the countdown store has a small
Boot cost, but it is not a safe standalone product gain and does not explain
the remaining parity gap.
