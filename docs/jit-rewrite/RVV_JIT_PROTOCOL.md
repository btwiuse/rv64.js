# RVV JIT protocol

## Goal

Implement complete mandatory RVV 1.0 execution support at JIT instruction
boundaries for the repository's RV64GCV baseline (VLEN=128, ELEN=64), while
preserving exact interpreter behavior and avoiding regressions in the existing
scalar JIT scorecard.

The implementation must be architecture-general. It may dispatch by decoded
RVV opcode or instruction family, but must not identify benchmark binaries,
guest PCs, symbol names, input sizes, or measured scorecard rows.

## Frozen measurements

Two scorecards are kept distinct:

1. `scorecard-v2-rv64gcv-v1` runs the frozen stock-musl RV64GCV binaries and
   their pinned i686 counterparts with JIT enabled. Its authoritative sides are
   rewrite and copy/v86.
2. `scorecard-v2-modern` remains the unchanged scalar three-way JIT scorecard.
   It is the regression gate for non-vector workloads.

The first RV64GCV JIT run made from checkpoint `9850777` is the performance
baseline. Candidate comparisons use the same initramfs, kernel, workload,
scheduler cadence, row order, phases, and repetition rules.

The population owns `rv64gcv-linux-Image`; it must come from
`.#virt-kernel-fast` and enable both `CONFIG_RISCV_ISA_V` and
`CONFIG_RISCV_ISA_V_DEFAULT_ENABLE`. The legacy `web/images/alpine/Image` is
not a valid fallback because that artifact has the V extension disabled.

## Semantic and direct lowering contract

The DBT IR represents an RVV instruction as an ordered architectural effect.
Before that effect, generated code commits every pending scalar register,
floating-point register, `fcsr`, PC, and retired-count value required to
reconstruct the exact pre-instruction state. It then invokes a typed Wasm
helper for the concrete user or system machine.

Each effect also carries an optional direct description decoded solely from
architectural instruction fields. Generated Wasm may use SIMD or a scalar
Wasm loop only after runtime guards prove the current vtype, vl, vstart,
masking, register grouping, privilege state, and memory capability are exact
for that lowering. A guard miss invokes the typed helper.

Successful direct effects reconcile precise pre-effect publications with the
function's cached scalar/FP locals. Scalar- or FP-producing vector moves
reload canonical state. System execution enforces and dirties mstatus vector
and FP state exactly as the interpreter does.

On helper failure, generated code returns at the pre-instruction PC without
retiring it; the interpreter re-executes the instruction and produces the
architectural exception. A partially completed vector memory operation keeps
its `vstart` state, so restart behavior remains precise. Direct memory is used
only after the entire selected range and direct-RAM translation capability are
proved; page crossings, MMIO, code-dirty stores, missing translations, and
fault candidates retain the helper.

Direct FP arithmetic is forbidden unless rounding and accrued exception flags
can be reproduced exactly. Flag-free bit operations may lower directly.

## Promotion gates

- Every mandatory RVV instruction accepted by the interpreter is accepted at
  a JIT boundary in both user and system layouts.
- Interpreter/JIT differential tests cover vector state, scalar and FP side
  effects, masks, `vstart`, memory, and faults.
- Existing workspace, Wasm, and scorecard harness tests pass.
- The authoritative RV64GCV JIT scorecard is measurement-valid.
- The unchanged scalar JIT scorecard shows no material regression.
- Any family with an ambiguous first performance result receives a frozen
  paired A/B, and is removed if it establishes a material regression.
