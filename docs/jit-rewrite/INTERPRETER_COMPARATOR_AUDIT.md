# Pure-Interpreter Comparator Audit

Date: 2026-08-10 (America/Phoenix)  
Status: complete; copy/v86 has no comparable benchmark recognizer; goal open

## Question and conclusion

This audit asks whether pinned copy/v86 obtains its JIT-off scorecard results
with the same kind of benchmark-derived optimization that invalidated the
rewrite's former 12-win/1-match claim.

It does not. The audited copy/v86 source contains no benchmark, process,
symbol, guest-PC, or exact multi-instruction workload recognizer. Its important
String mechanism is the complete implementation of the architecturally defined
x86 REP string-instruction family. In particular, a permission- and
page-bounded `REP MOVS` chunk can call host `memcpy`, and `REP STOSB` can call
host `memset`. That behavior is available to every x86 guest program which
executes those instructions. It is not analogous to recognizing a scalar loop
from nbench or musl after observing a benchmark loss.

The rewrite's removed bulk-copy, `strncmp`, offset-adjust, bit-run,
assignment-scan, and numeric-heapsort recognizers did match exact
multi-instruction workload bodies. Those were benchmark overfitting even
though they preserved architectural results. copy/v86's REP implementation
does not excuse restoring them.

## Audited comparator identity

- source commit:
  `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`;
- source-tree SHA-256 recorded by the scorecard:
  `ca8afd71c1444a56c20b1ab63939569329fd5369a1a75760b5dce53fc3ba00f8`;
- Wasm SHA-256 recorded by the scorecard:
  `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1`;
- local `src` status against the pinned commit: clean;
- `src/rust/cpu/cpu.rs` SHA-256:
  `9268cad1019fef8339ce4eb7e1e4e809713df86e958574102e4d4e0d6cf24bf8`;
- `src/rust/cpu/string.rs` SHA-256:
  `15e7b96264a839bd8d572eedc171f953f5cf255cf35b152dec37a5bcf6cb53aa`;
- `src/rust/cpu/memory.rs` SHA-256:
  `b6e72b1ec56cf98f74a9988802f46400fa145a76e9a84a0f01556e3e67a20571`.

A source search found no nbench, BYTEmark, String Sort, Bitfield, fixed guest
PC, or benchmark-name selector under the pinned `src` tree.

## JIT-off execution proof

The scorecard worker constructs v86 with `disable_jit: true`, explicitly calls
`set_jit_config(0, 1)`, and rejects the worker if `get_jit_config(0)` is not
one. Interpreter-mode validation also requires all module-finalization, byte,
cache-size, and workload-module activity counters to remain zero.

The clean report at
`target/bench/interpreter-clean/baseline-a2f42e55-r1/interpreter-scorecard-v2-2026-08-11T04-05-08-202Z.json`
passes those checks for all copy/v86 trials. The String trial records
`disabled=1`, zero instantiations, zero finalized modules and bytes, zero cache
size, and `inactiveProof=true`. Thus the REP result below belongs to the
ordinary interpreter, not generated code hidden behind the JIT-off label.

## Ordinary interpreter mechanisms

copy/v86's JIT-off path still uses architecture-general interpreter
infrastructure whose names include `jit` because the same dispatcher also
serves JIT-enabled execution:

1. `get_phys_eip` retains the translated physical offset for the current
   virtual code page and performs a new translation after a page change.
2. `jit_run_interpreted` fetches the current opcode directly from guest RAM,
   calls the generated complete x86 opcode dispatcher, and continues across a
   same-page interpreted stretch until an architectural block boundary, page
   transition, or bounded backward branch.
3. The instruction counter advances once for each x86 instruction dispatched
   by that loop. A REP instruction remains one decoded x86 instruction even
   when its architectural count register describes many elements.

These mechanisms are page-, state-, and ISA-driven. They do not select a
measured workload.

## REP string implementation

`src/rust/cpu/string.rs` uses one generic `string_instruction` implementation
for MOVS, LODS, STOS, SCAS, CMPS, INS, and OUTS; byte, word, and dword sizes;
16- and 32-bit address sizes; forward and backward direction; and REP/REPE/
REPNE behavior.

The fast path is selected only from architectural and memory conditions:

- the instruction is REP and has compatible alignment/address-size behavior;
- source and destination virtual addresses translate successfully;
- the physical range is ordinary RAM rather than MMIO;
- the chunk ends at the first source or destination page boundary;
- MOVS overlap and direction preserve x86 sequential-copy behavior; and
- a destination code page is dirtied through the general JIT coherency path
  before the write.

Within those bounds, MOVS calls
`memory::memcpy_no_mmap_or_dirty_check`, which is Rust `ptr::copy`, and STOSB
calls `memory::memset_no_mmap_or_dirty_check`, which is Rust
`ptr::write_bytes`. Compare and I/O forms retain their element-wise semantic
checks. Incomplete page chunks reset the instruction pointer to re-enter the
same REP instruction for the next page. This is a complete x86 instruction
family, including faults, page boundaries, direction, overlap, count updates,
and compare termination—not an extracted guest loop.

## Cross-ISA work amplification

The clean development String trial reports:

| Engine | STEADY wall time | Reported guest-instruction delta |
| --- | ---: | ---: |
| rewrite RV64 | 7,177.898 ms | 796,661,853 |
| copy/v86 i686 | 1,722.637 ms | 96,485,320 modulo 2^32 |

The reported RV64 count is about 8.26 times the reported x86 count. The
counters are not equal-work metrics across ISAs: one x86 REP dispatch may move
or compare a page-bounded chunk, while an RV64 binary must express comparable
work as scalar load, store, pointer-update, count, and branch instructions.
They nevertheless explain the direction of the result. The clean RV64 engine
already retires roughly 111 million reported guest instructions per wall
second in this trial versus roughly 56 million x86 instructions per second for
v86, yet loses elapsed time because it executes far more architectural
instructions. I004 improves the RV64 engine further without changing that
guest count, but its chained String projection is still only about `0.308x`
versus v86.

This does not relax the wall-time goal. It means that closing the remaining
gap with a pure interpreter requires a genuinely general way to reduce RV64
fetch/decode/dispatch cost or a standardized RV architectural bulk operation.
Replacing selected scalar loops with host helpers would merely recreate the
withdrawn error.

## Separate workload-input problem

The development scorecard itself has an asymmetry on the RV64 side, not a
hidden copy/v86 optimization. `tests/vs-v86/mk-bench-bins.sh` links
`tests/vs-v86/nbench-extras/fastmem.c` only into the RV64 nbench binaries and
disables builtin memcpy/memmove there. The source comment explicitly identifies
String Sort as its motivation. The i686 binary uses its target-selected musl
implementation.

The existing workload contract discloses the two implementations and embeds
the extra source in both archives for provenance, but equal data widths and
work counts do not make the memory implementations identical. The 13-row
population therefore remains development evidence only. Final acceptance is
still gated on the already frozen, never-executed `stock-musl-v1` population
and the sealed holdouts described in
[INTERPRETER_ANTI_OVERFIT_PROTOCOL.md](INTERPRETER_ANTI_OVERFIT_PROTOCOL.md).

No stock-musl or sealed-holdout workload was executed for this audit.
