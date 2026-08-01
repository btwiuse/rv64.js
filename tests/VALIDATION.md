# Validation status

Last strict full run: 2026-07-31. Reproduce it with
`REQUIRE_ALL=1 tests/run-all.sh` inside `nix develop`; strict mode fails if a
required tool, benchmark artifact, or modern-system kernel is unavailable.
The run passed all eight stages with no skips: release workspace tests, guest
builds, three QEMU differentials, 134 ISA tests, 109 Spike lockstep tests, 193
architecture signatures, Wasm smoke/JIT/FP/AMO differentials, and the modern
OpenSBI/Linux boot smoke test.

## Official riscv-tests (ISA suites, p-variants, 134 tests)

| | rv64.js | Spike 1.1.1-dev (golden model) |
|---|---|---|
| pass | **134/134** | 132/134 |

rv64.js passes the complete suite. F/D is now a softfloat implementation
(softfp.rs, ported from TinyEMU's softfp) with exact IEEE 754 exception
flags and all five rounding modes — the earlier host-float fflags
deviation is gone.

- The **2 Spike failures** are Spike-configuration artifacts, not rv64.js
  bugs:
  - `rv64mi-p-zicntr`: passes on Spike with `--isa=rv64gc_zicntr`
    (counters not in its default ISA string); rv64.js implements Zicntr.
  - `rv64ui-p-ma_data`: rv64.js supports misaligned data accesses in
    hardware (a spec-legal choice, same as TinyEMU); default Spike traps.

Runner: `tests/run-isa-tests.sh` (needs gcc-riscv64-unknown-elf).

## Differential testing

- **qemu-riscv64 (user-mode)**: all three guests (hello-std, fpu-test,
  bench) produce bit-identical stdout and exit codes.
- **TinyEMU (`temu`, native)**: boots the same guest images; used as the
  machine-model reference during bring-up.
- **Spike**: built from riscv-isa-sim source; available as golden-model
  oracle for future lockstep/state-diff work.

## JIT

- Wasm emitter validation: 338/338 modules emitted by `translate_block`
  from every halfword offset of a real guest binary (i.e. mostly garbage
  input) are structurally valid wasm.
- Interpreter-vs-JIT: guests produce bit-identical results with the JIT
  enabled; Linux boots to an interactive shell with system blocks live.

## riscv-arch-test signature compliance (vs Spike golden model)

`tests/run-arch-tests.sh`: official riscv-arch-test 3.9.1 suites
(I M A C F D Zifencei privilege) compiled once against a shared HTIF/CLINT
environment (tests/arch-env/) and executed on both rv64.js and Spike —
**193/193 signatures bit-identical, 0 compile skips**. Same tests, golden
model, and pass criterion as RISCOF.

## Spike lockstep differential

`tests/lockstep.py`: per-instruction x-register writeback streams
(rv64-isa-test --trace vs spike --log-commits) — **109/109 riscv-tests
lockstep-identical, ~24,000 writebacks compared** (rv64ui-p-ma_data
skipped: documented spec-legal misaligned-access divergence). Suite stage
5 in tests/run-all.sh.

## FP fast path

FADD/FSUB/FMUL/FDIV use native FP (wasm f32/f64 instructions in the
browser build) when rm=RNE and NX is already set — conditions under which
no new flag information is possible. Verified by a 600k-iteration
differential fuzz against softfp (bit-identical values; softfp confirmed
to set nothing beyond NX in every eligible case) — see
`fp_fastpath_tests` in cpu.rs. riscv-tests remain 134/134.

## Bugs found by this validation (all fixed)

1. 32-bit AMO min/max compared full 64-bit registers (rv64ua-p-amomin*_w).
2. `mstatus.FS` forced dirty; FP ops didn't trap when FS=Off (rv64mi-p-csr).
3. `minstret`/`mcycle` not writable (rv64mi-p-instret_overflow).
4. JIT emitter had wrong opcodes for I64_GE_S/GE_U — BGE/BGEU blocks
   miscompiled (found by review during integration).
5. Missing: PMP CSRs, mstatus.TVM/TW/TSR enforcement, U-mode WFI/SFENCE
   traps, trigger CSR stubs (rv64mi-p-pmpaddr/illegal/breakpoint).
