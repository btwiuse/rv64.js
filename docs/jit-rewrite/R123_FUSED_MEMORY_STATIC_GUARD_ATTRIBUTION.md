# R123 Fused-Memory Static-Guard Attribution

Date: 2026-08-10  
Status: diagnostic complete; no product candidate admitted

## Question

R054 promoted interpreter consumption of fused load/store capabilities, but
retained its exact diagnostic enable flag. Production sets the flag true, and
every `Cpu::ld`/`Cpu::st` still loads and branches on it before probing the
fused row. Can replacing that runtime flag with a compile-time Bus capability
remove enough current modern-Boot cost to support a verified net 1% gain?

This mechanism is independent of R122's opcode semantics: it uniformly removes
one obsolete production guard from every scalar data-memory helper while
preserving the accepted fused tag/offset proof and miss fallback.

## Evidence and method

An exploratory read of the exact R119 optimized native bodies identified the
three-instruction flag blocks. Before any source edit or timing, a deterministic
analyzer must authenticate and close the preserved data:

- perf data `24028310d77f...`;
- `ld1` DSO `6bb9919ffc99...`;
- `ld4` DSO `475f06161e1b...`;
- `ld8` DSO `5096d4c3a9e9...`;
- `st4` DSO `0e8f127f7d20...`; and
- `st8` DSO `109df5d60641...`.

For each body, verify the exact `interpreter_fused_memory` load, comparison
with enabled, and conditional branch in these half-open symbol-relative bands:

| Body | Guard band |
|---|---:|
| `ld1` | `[0x0033,0x004f)` |
| `ld4` | `[0x0700,0x0718)` |
| `ld8` | `[0x035e,0x0376)` |
| `st4` | `[0x04ee,0x0506)` |
| `st8` | `[0x02b1,0x02c9)` |

Perf/JIT logging perturbs execution, so elapsed time is excluded. Samples may
skid; the complete guard basic blocks are an exposure bound, not causal timing.

## Admission rule

Admit the compile-time Bus specialization only if the complete authenticated
guard population owns at least 1.25% of uninstrumented main-thread period and
removing the guard plus the once-per-host-call setter has a realistic
construction-adjusted projection of at least `1.01x` whole Boot. The outer
setter is one field store per `run_system_jit` invocation, versus one guard per
guest scalar memory access; it cannot be combined with unrelated scheduler
work to cross the floor.

If the exposure fails, stop without changing Bus, CPU layout, exports, the
scorecard diagnostic API, or product bytes. This is not a runtime result and
does not weaken the standing verified-one-percent acceptance policy.

The frozen result is recorded in
`R123_FUSED_MEMORY_STATIC_GUARD_ATTRIBUTION_RESULT.md`.
