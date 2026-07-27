# JIT correctness, benchmarking, and generalization issues

This document tracks concerns found during a review of `crates/rv64-jit` and
its integration and benchmark harnesses on 2026-07-24.

The review found no literal benchmark recognition in the JIT: there are no
workload names, checksum constants, or guest-PC-specific fast paths. The
optimizations are real JIT techniques. However, some current performance gains
come from violating execution-budget semantics, and the system-vs-v86 timing
methodology favors rv64.js. These issues must be resolved before claiming that
the JIT generally matches or beats copy/v86.

Priority meanings:

- **P0:** invalidates correctness or headline performance conclusions
- **P1:** serious architectural or measurement defect
- **P2:** production-readiness or generalization risk
- **P3:** cleanup or lower-risk hardening

## P0 — Compiled loops do not honor the caller's instruction budget

### Problem

`crates/rv64-jit/src/lib.rs` uses a fixed:

```rust
const LOOP_CAP: u64 = 1 << 24;
```

Generated functions do not receive the caller's remaining instruction budget.
A structured loop can therefore retire roughly 16.8 million instructions even
when `user_run` or `sys_run` was asked to execute one instruction.

A direct runtime probe reproduced this:

```text
runUser(1) retired 16,777,224 guest instructions
```

System mode amplifies the problem. `crates/rv64-wasm/src/lib.rs` chains up to
`JIT_CHAIN_CAP == 1024` compiled calls before applying `retired_sum` to
`remaining` or checking interrupts. A persistent compiled loop can therefore
execute approximately:

```text
1024 * 16,777,216 ~= 17.2 billion guest instructions
```

before yielding.

This invalidates the documented fuel contract, produces unbounded interrupt
latency, and gives tight-loop benchmarks a large advantage by eliminating host
and dispatcher boundaries.

Relevant code:

- `crates/rv64-jit/src/lib.rs`: `LOOP_CAP`, `translate_loop`
- `crates/rv64-jit/src/lib.rs`: `translate_superblock`
- `crates/rv64-wasm/src/lib.rs`: user and system JIT chain loops

### Required fix

- Give generated functions access to a dynamic retirement deadline or remaining
  fuel.
- Limit a compiled call by `min(remaining, interrupt_quantum)`.
- Bound system chaining by dynamically retired instructions, not only by the
  number of block calls.
- Apply retired counts and service pending interrupts before re-entering a
  capped structured loop.

### Acceptance criteria

- After tier-up, `user_run(1)` advances `insn_count` by no more than one.
- `sys_run(N)` never retires more than `N` instructions, apart from any
  explicitly documented single-instruction trap behavior.
- An infinite compiled loop services a pending timer/external interrupt within
  a documented instruction bound.
- Tests cover budgets around `0`, `1`, loop-body length, `LOOP_CAP`, and
  `LOOP_CAP + 1`.
- The performance scorecard reports the interrupt quantum and uses the corrected
  implementation.

## P0 — rv64 system benchmark timing excludes work that v86 includes

### Problem

rv64 system console output is forwarded to JavaScript only when `sys_run`
returns. `tests/vs-v86/compare-sys.mjs` and `tests/vs-v86/scorecard.mjs` inspect
the accumulated output and timestamp `BENCH_START` after a synchronous
`runSystem()` call completes.

Therefore, the first rv64 slice containing `BENCH_START` is excluded from the
measurement. With the loop budget overrun above, that slice may execute a large
fraction or all of the benchmark.

The v86 runner timestamps serial bytes as they arrive, so v86 includes the
actual interval between `BENCH_START` and `BENCH_DONE`.

`compare-sys.mjs` also runs an untimed rv64 warmup before the reported run,
whereas each v86 runner starts a fresh workload. This excludes rv64 compilation
and tier-up cost without applying the same policy to v86.

Relevant code:

- `crates/rv64-wasm/src/lib.rs`: output forwarded at the end of `sys_run`
- `tests/vs-v86/compare-sys.mjs`: `runOne`
- `tests/vs-v86/scorecard.mjs`: `rvRunBench`
- `tests/vs-v86/v86-compute.mjs`: live serial-byte timestamps

### Required fix

Use a symmetric timing protocol on both emulators. Acceptable approaches
include:

- measure from immediately before launching the command until verified
  completion on both sides, including equivalent tier-up policy; or
- stream rv64 serial output during execution and timestamp the marker callback;
  or
- use a dedicated host-visible benchmark marker/hypercall with timestamps taken
  at the event.

Do not infer marker times by polling buffered output after a large execution
slice.

### Acceptance criteria

- rv64 and v86 use the same cold or warm policy.
- Both include or both exclude JIT compilation and tier-up.
- A test benchmark that emits START, performs a known delay, and emits DONE
  reports the delay even when both markers occur during one `runSystem` call.
- Results use interleaved median-of-N trials rather than fixed-order single
  samples.
- Old system ALU/mixed headline numbers are marked invalid or replaced.

## P1 — System-mode FP JIT ignores `mstatus.FS` and does not mark FP state dirty

### Problem

The interpreter:

- traps on FP instructions when `mstatus.FS == Off`; and
- marks FP state Dirty after instructions that modify FP state.

The JIT layout exposes `f[]` and `fcsr`, but not `mstatus` or an equivalent FP
state guard. Compiled FLD, FP arithmetic, comparisons, and moves can execute
with FS Off and do not perform the required Dirty transition.

This can break lazy FP enablement and FP context switching. It is especially
risky because compiled code survives context switches and physical code pages
may be shared by processes with different FP-state status.

Relevant code:

- `crates/rv64-core/src/cpu.rs`: `fp_check`, `fp_dirty`
- `crates/rv64-jit/src/lib.rs`: `JitLayout`, FP emitters
- `crates/rv64-wasm/src/lib.rs`: system `JitLayout` construction

### Required fix

Either:

- add the required privileged FP state to `JitLayout`, guard every compiled FP
  instruction, and perform the architecturally correct Dirty transition; or
- disable system-mode FP compilation until this can be done correctly.

### Acceptance criteria

- A compiled FP instruction traps exactly like the interpreter with FS Off.
- Clean/Initial FP state becomes Dirty when required.
- Context-switch tests alternate two processes sharing the same FP code page
  and verify independent FP register state.
- Tests cover FP loads, arithmetic, compares, moves, static rounding, dynamic
  rounding, and FP fast-path bailouts.

## P1 — Loop and superblock bailouts undercount retired instructions

### Problem

When `Ctx::retired_local` is present, `Ctx::bail` stores only the runtime
`ITER` value and ignores the static count passed to it.

Structured loops keep instructions since the last branch/scope boundary in a
separate compile-time `seg` accumulator. If a TLB probe or FP guard bails in the
middle of that segment, the instructions already executed in the segment are
not reported.

Superblocks have the same problem: `emit_simple(..., len)` can bail after `len`
instructions in the current entry body, but `retired_local == ITER` discards
`len`.

This permanently corrupts:

- `insn_count` and `minstret`
- instruction-derived clocks
- fuel accounting
- JIT coverage
- reported Minsn/s

Relevant code:

- `crates/rv64-jit/src/lib.rs`: `Ctx::bail`
- `crates/rv64-jit/src/lib.rs`: `translate_loop` and `seg`
- `crates/rv64-jit/src/lib.rs`: `emit_super_body`

### Required fix

At every possible mid-block bailout, publish:

```text
completed_runtime_segments + instructions_completed_in_current_segment
```

Do not increment the success path twice. One approach is to let bailout
emission accept an additional compile-time retired count used only inside the
bail branch.

### Acceptance criteria

- Interpreter and JIT `insn_count` are identical after forced TLB misses at
  every instruction position in a loop.
- Repeat for FP eligibility/result bailouts and page-crossing memory accesses.
- Repeat for every superblock entry-body position.
- Guest `rdinstret` observes identical values under interpreter and JIT.

## P1 — The scorecard does not enforce correctness or completeness

### Problem

`tests/vs-v86/scorecard.mjs` records elapsed time for rv64 compute rows without
extracting or validating the checksum. A miscompiled benchmark that reaches
`BENCH_DONE` can be reported as a win.

The scorecard also silently reduces its scope:

- Python and compile rows disappear when artifacts are absent.
- Null or failed v86 rows are excluded from the scored denominator.
- Nbench iterates only the rows that happened to be parsed.
- Nbench is optional, and missing individual kernels are not rejected.
- It can report "ALL ROWS PASS" for an incomplete subset.
- Main rows use one fixed-order sample despite documented host noise.

### Required fix

- Define an explicit manifest of required rows and expected outputs.
- Treat missing artifacts, process failures, parse failures, timeouts, and
  checksum mismatches as failures.
- Require every expected nbench kernel.
- Include trial samples and dispersion in the JSON output.
- Interleave rv64/v86 order and report median plus a robust spread statistic.
- Record exact emulator revisions, wasm hash, guest binary hashes, compiler
  versions, flags, and host details.

### Acceptance criteria

- Deliberately corrupting a benchmark checksum fails the scorecard.
- Deleting any required artifact produces a nonzero exit.
- Making a v86 child exit without `RESULT` produces a nonzero exit.
- Missing one nbench kernel produces a nonzero exit.
- The scorecard cannot print an all-pass verdict unless the full required
  manifest was run successfully.

## P1 — Add architectural differential testing for emitted JIT code

### Problem

`rv64-jit` currently has four unit tests. They check translation metadata and
wasm headers but do not execute emitted modules or compare architectural state.

The wasm smoke test validates that arbitrary emitted modules instantiate, but
structural wasm validity does not establish RISC-V correctness. The official
ISA, architecture, and lockstep suites exercise the interpreter rather than
the runtime-generated wasm JIT.

End-to-end guest checksums are insufficient for detecting incorrect traps,
flags, retired counts, interrupt timing, and privileged state transitions.

### Required fix

Build a differential runner that executes the same generated block/region in:

1. the interpreter; and
2. the emitted wasm JIT;

then compares:

- PC
- every GPR and FPR
- `fcsr` and privileged FP state
- memory writes
- exception/trap information
- retired count
- consumed fuel

### Acceptance criteria

- Exhaustive edge tests for every supported instruction encoding.
- Randomized basic blocks and structured CFGs.
- Register-alias cases such as `rd == rs1`, `rd == rs2`, and x0.
- Division/remainder zero and signed-overflow cases.
- Memory alignment, bounds, page crossing, TLB hit/miss, MMIO, and faults.
- FP normal, zero, subnormal, infinity, qNaN, sNaN, rounding, and flag cases.
- Differential tests run in CI on every JIT change.

## P2 — Code-cache lifecycle is unsafe and benchmarks hide it

### Problem

Fresh wasm instances are used for most benchmarks, hiding long-lived cache
behavior.

Current issues:

- `sys_boot` replaces the machine without clearing `SYS_JIT`; a second boot in
  the same wasm instance can reuse code generated from the previous guest.
- `jit_set_enabled(0)` changes only the tier-up threshold. Existing compiled
  blocks continue to execute, despite the API being described as disabling the
  JIT.
- Every compilation grows the wasm function table.
- Cache invalidation removes Rust metadata but cannot reclaim table entries or
  compiled wasm code.
- Reboots, address-space churn, self-modifying code, and repeated user ELF loads
  can grow the table indefinitely.

Relevant code:

- `crates/rv64-wasm/src/lib.rs`: `sys_boot`, `jit_set_enabled`, `JitState::clear`
- `web/rv64.js`: `host_jit_register`

### Required fix

- Clear all system JIT state on `sys_boot`.
- Define whether disabling JIT means "no new compilation" or "execute no JIT
  code"; name and implement the API accordingly.
- Add a bounded code cache with eviction/reuse or a documented safe reset
  mechanism.
- Reset per-machine statistics when loading/booting a new guest.

### Acceptance criteria

- Booting two different systems in one wasm instance cannot execute code from
  the first.
- Disabling JIT after tier-up produces zero subsequent JIT dispatches if the API
  retains its current name/contract.
- A long-running recompilation stress test has bounded table/code-cache growth.
- Cache eviction and self-modifying-code tests preserve correctness.

## P2 — Current performance results are overly coupled to the tuning workloads

### Problem

The loop optimizer is general within its supported subset, but that subset
matches the headline benchmarks unusually well:

- maximum 128-instruction scan
- outer back-edge targets the entry PC
- properly nested natural loops
- forward if-then and loop-exit branches
- no calls, indirect control flow, switches, or irreducible CFG
- many unsupported ISA and FP operations still terminate regions

The ALU benchmark is a single tight natural loop. The mixed benchmark's nested
insertion-sort loops and breaks directly motivated the structured-loop work.
The roadmap records each feature taking these workloads toward 100% coverage.

This is legitimate benchmark-driven engineering, but the results describe
best-case structured-loop performance, not broad JIT performance. The
superblock path intended for branchy code remains disabled by default.

### Required fix

Adopt an anti-overfitting benchmark policy:

- separate tuning and held-out workload sets;
- do not add a held-out workload to the tuning set after observing a loss;
- report cold and warm performance;
- report worst-case regressions as well as aggregate performance;
- track compilation time, generated-code size, cache memory, interrupt latency,
  and invalidation cost;
- keep ALU/mixed as clearly labeled micro/best-case rows rather than headline
  proof of general parity.

Suggested held-out categories:

- CPython and another language runtime
- a real compiler
- compression and cryptography
- SQLite or another branchy data workload
- libc-heavy and syscall-heavy programs
- memory working sets from KiB through hundreds of MiB
- code churn/self-modifying workloads
- boot and interactive latency

### Acceptance criteria

- Benchmark documentation labels microbenchmarks, best cases, and macro
  workloads separately.
- Performance changes must improve a predeclared aggregate without unacceptable
  held-out regressions.
- Results identify which JIT mode each workload used: basic block, structured
  loop, or superblock.
- Coverage includes bail reasons and time spent compiling, interpreting,
  dispatching, and executing generated code.

## P2 — Cross-ISA benchmark equivalence needs tightening

### Problem

`tests/vs-v86/rvbench_fs.c` defines:

```c
typedef unsigned long u64;
```

This is 32-bit in the i386 build and 64-bit in the RV64 build. The i386 build
also uses x87 under the current `-m32 -O2` flags, while RV64 uses its regular
double-precision register ISA. The programs run the same source-level loop
counts, but they are not equivalent instruction or numerical workloads.

Cross-ISA source-level comparisons are still useful as end-to-end emulator
measurements, but they cannot by themselves isolate JIT implementation quality.

### Required fix

- Use `uint64_t` or an explicit 64-bit typedef where 64-bit behavior is intended.
- Make numerical-width and FP-model differences explicit.
- Verify the generated assembly and record dynamic guest instruction counts.
- Report each emulator's JIT speedup over its own interpreter alongside direct
  rv64-vs-v86 wall time.
- Keep native execution time as an additional normalization point.

### Acceptance criteria

- Checksums are comparable and automatically verified across architectures
  where the algorithm is meant to be bit-identical.
- Any intentional cross-architecture numerical difference is documented per
  field.
- Build flags and disassembly summaries are recorded in scorecard artifacts.

## P2 — Direct-mapped interpreter hot counters are untagged

### Problem

`JitState::interp_hot` is indexed using the same low-PC direct-map slot as the
dispatch cache, but unlike the dispatch cache it has no PC tag. Different PCs,
processes, or address spaces that alias the same slot share a saturating hot
count.

Once one PC makes a slot hot, unrelated cold PCs at the same index may be
immediately forced into compilation. This can cause compile storms and makes
tier-up behavior workload- and address-layout-dependent.

Relevant code:

- `crates/rv64-wasm/src/lib.rs`: `JitState::interp_hot`
- `crates/rv64-wasm/src/lib.rs`: `JitState::dslot`
- `crates/rv64-wasm/src/lib.rs`: `run_slice_until` callback

### Required fix

Tag hot-counter entries with the full PC or use a bounded associative structure
with explicit replacement semantics.

### Acceptance criteria

- Two PCs mapping to the same direct-map slot maintain independent counts.
- Address-space switches do not transfer heat between unrelated code.
- A collision stress test does not cause excessive cold-block compilation.

## P2 — Restore the `sys_insn_count` wasm export

### Problem

`sys_pc` has duplicate `#[no_mangle]`/`#[allow]` attributes, while
`sys_insn_count` has none. A fresh release wasm build therefore does not export
`sys_insn_count`.

`node tests/bench.mjs --json` currently fails with:

```text
TypeError: this.ex.sys_insn_count is not a function
```

Relevant code:

- `crates/rv64-wasm/src/lib.rs`: `sys_pc`, `sys_insn_count`
- `web/rv64.js`: `sysInsnCount`

### Acceptance criteria

- `WebAssembly.Module.exports` includes `sys_pc` and `sys_insn_count`.
- `node tests/bench.mjs --json` completes.
- The wasm smoke test asserts the complete public export surface.

## P3 — Harden instruction-encoding validation

### Problem

Some JIT paths accept the opcode/funct3 while failing to reject reserved
funct7/funct6 or other required fixed fields. Examples include OP-IMM shifts,
OP-IMM-32 shifts, JALR's required funct3, and fixed fields in some FP moves.

The interpreter currently shares some of these decoding gaps, so ordinary
interpreter-vs-JIT differential testing will not catch all of them.

### Required fix

- Centralize complete encoding validation for every supported instruction.
- Compare invalid/reserved encodings against an independent architectural
  oracle, not only the local interpreter.

### Acceptance criteria

- Reserved encodings trap as illegal instructions.
- JIT scanners and emitters use the same complete validation functions.
- Invalid-encoding tests cover all supported opcode families.

## Validation performed during the review

The following passed:

- `cargo test --workspace --release`
- `node tests/wasm-smoke.mjs`
- standalone superblock execution: `sbtest == 55`
- 900 generated ALU/div/rem edge cases compared with a reference implementation
- JIT and interpreter produced identical output and instruction counts for the
  existing soft-FP `bench` guest

The following exposed defects:

- `runUser(1)` retired `16,777,224` instructions after loop tier-up
- `node tests/bench.mjs --json` failed because `sys_insn_count` is not exported

A fresh copy/v86 head-to-head run was not performed because the external v86
checkout and cross-ISA benchmark artifacts were not present in the workspace.

## Recommended fix order

1. P0 fuel/interrupt correctness
2. P0 symmetric benchmark timing
3. P1 system FP architectural state
4. P1 exact bailout retirement
5. P1 JIT differential test infrastructure
6. P1 scorecard correctness and completeness
7. P2 cache lifecycle and tier-up collision behavior
8. P2 benchmark diversification and cross-ISA normalization
9. P2/P3 export and decoder hardening

No "rv64 JIT is faster than v86" claim should be treated as established until
the two P0 issues are fixed and the corrected scorecard is rerun.

---

# STATUS ADDENDUM (2026-07-25)

Disposition of every item, with commits and covering tests:

- **P0 budget/fuel contract** — FIXED (a5f1620). FUEL_CELL per dispatch,
  INTERRUPT_QUANTUM=1M, chain bounded by retired instructions.
  `user_run(1)` worst overshoot measured 9 insns (was 16,777,224); enforced
  by a wasm-smoke budget test (<=128 over 2000 metered calls).
- **P0 symmetric timing** — FIXED (a5f1620, d1cb131). Console streamed per
  quantum during execution; markers timestamped in the onWrite stream on
  both sides; compile row windows symmetric (md5 outside); v86 legs run
  before rv64 legs (background tier-up pollution measured 4x and removed).
  Old ALU/Mixed headlines invalidated and re-measured (ALU "66ms/50x" was
  the artifact; honest ALU is ~1.5-1.9x).
- **P1 system FP state** — FIXED (a5dd523, f540317). Every compiled FP
  instruction is guarded by mstatus.FS == Dirty (hoisted to one per-block
  gate with zero-retire entry bail); FS=Off traps via the interpreter,
  Initial/Clean transition through fp_check/fp_dirty exactly.
  tests/fp-context-switch.mjs: two concurrent processes sharing FP code
  pages under JIT+superblocks both produce the exact checksum.
- **P1 exact bailout retirement** — FIXED (a5dd523). Ctx::bail reports
  ITER + segment-relative count; translate_loop passes segment-relative
  positions; superblock bodies pass body-relative counts. Differential:
  user-mode insn_count bit-equal (230,000,332) between JIT and interpreter.
- **P1 differential testing** — DONE (caec17b). tests/jit-differential.mjs:
  synthesized random programs across every compiled instruction family,
  edge-value seeding, tier-up mid-run, full state compared (pc, exit,
  insn_count, fcsr, x1-31, f0-31). 250-program soak ALL PASS. In
  run-all.sh stage 7.
- **P1 scorecard enforcement** — DONE (412fd82, ae1e5c1, + retry/timeout
  fixes). Required-row manifest, cross-ISA checksum validation (caught a
  real broken artifact in production), nonzero exit, REPS=N medians,
  provenance (git rev, wasm sha256, config) in every JSON.
- **P2 cache lifecycle** — FIXED (a2c88e8, 8607ed8). sys_boot clears JIT
  state + stats; jit_set_enabled(0) executes no JIT code; function-table
  growth bounded (JIT_TABLE_CAP) with graceful degradation.
- **P2 benchmark coupling / anti-overfitting** — POLICY ADOPTED (ae1e5c1;
  tests/vs-v86/README "Benchmark policy"). Micro/macro labels, all-rows
  re-validation, pinned shape tests, dispersion + provenance requirements.
  The scorecard grew python/compile/nbench macro rows this cycle.
- **P2 cross-ISA equivalence** — DONE (fdf25f1, 662531b). The u64 typedef
  is now 64-bit on both ISAs, FP flags reconciled (i386 -msse2 -mfpmath=sse;
  riscv64 -ffp-contract=off), and the Mixed checksum folds FULLY
  bit-identically cross-ISA (0x29c0709f16c84da4 on both, enforced at full
  64-bit equality by the scorecard); flags documented in build-kernels.sh;
  provenance recorded.
- **P2 untagged hot counters** — FIXED (412fd82). interp_hot slots carry a
  full-pc tag; aliasing pcs reset instead of inheriting heat.
- **P2 sys_insn_count export** — FIXED (a5f1620).
- **P3 encoding validation** — HARDENED (cebcd91). Reserved OP-IMM /
  OP-IMM-32 shift bits, JALR funct3, FMV/FSQRT fixed rs2 fields rejected in
  the single-authority predicates (scanners and emitters cannot desync);
  pin tests cover rejected and accepted neighbors. Full architectural
  oracle comparison for invalid encodings remains an interpreter-level
  effort (the JIT now never compiles them).

Performance claims are made only from the corrected harness. Every review
item above is closed with a commit and a covering test.

## SCORECARD PROGRESS SINCE THIS ADDENDUM

The scorecard stood at 8/13 rows win-or-match when the items above were
closed. What the remaining rows turned out to be, and what fixed them:

- **Superblocks were mostly not running.** `sys_sb_ready` re-probed the
  va->pa mapping from the microtask queue — an arbitrary guest moment,
  usually inside the kernel or another process — so 96% of finished page
  functions were dropped (landed=4 of 127). Entries now install with their
  recorded pa and no dispatch line; the first dispatch verifies the mapping
  in the address space that asked for it.
- **Regions were page-clamped**, so a loop straddling a page boundary ran as
  six 2-10 instruction blocks (NUMERIC SORT: 5.6 insns/dispatch). Superblocks
  now span up to 3 virtually contiguous pages, grown only where hot code sits
  within a block's reach of the edge. NUMERIC SORT: 128 -> 790 iter/s.
- **A page compiled its page function once**, from whatever handful of pcs
  was hot at the threshold; code that got hot later stayed on individual
  blocks forever. Tracing one pc showed IDEA's cipher_idea inner loop was
  never once a seed. Rebuilds are now allowed as long as they cover new hot
  code (only unproductive ones count against the allowance). IDEA: 1600 ->
  4600 iter/s, and the 1600-or-4400 bimodality is gone.
- **Missing instruction families.** FSGNJ.D (fabs/fneg/copysign/fmv — 76% of
  everything FOURIER handed the interpreter), the whole F extension (HUFFMAN
  is float code: 714 -> ~1900 iter/s), and AMO*. Each landed with a covering
  differential; the F extension and FSGNJ extended the user-mode fuzzer,
  atomics got an in-guest interpreter-vs-JIT-vs-superblock checksum test.
- **FP eligibility was stricter than the interpreter's.** `x + 0.0` inside
  musl's pow bailed 32M times per FOURIER run, and every FMA with a zero
  operand bailed (79M). Both now match the interpreter's exact rules; the
  zero-operand relaxation exposed a signed-zero bug (fma(+0,-0,-0)) that the
  existing fast-path fuzz test caught before it shipped.
- **The FP gate was per function**, so a page mixing integer and float code
  made every entry into the integer half bail. Gates are per body now, split
  into the state check (FS) and the rounding checks (frm/NX).
- **The harness was misreading nbench**: when nbench's own repeats disagree
  it prints a "NOT 95 % statistically certain" warning and moves the score to
  a later line. Three kernels were being recorded as missing. The parser
  follows the continuation, and a run nbench flags as uncertain is now
  INVALID — an unstable number is not a result.

Scorecard after this work: **11 of 13 rows win-or-match** (valid run, all
kernels reported, none flagged statistically uncertain). Two rows remain, both
diagnosed:

**nbench FOURIER**, ~1.1-1.2x behind. The kernel's cost is the exact FMADD
emulation: ~35 f64 operations (Dekker two-product, Knuth two-sum,
round-to-odd) per guest `fmadd.d`, because neither wasm nor JavaScript exposes
a fused multiply-add. The i386 build cannot contract into FMA at all, so v86's
guest executes a plain multiply and add — the asymmetry is real work, not a
harness artifact. Trimming the guards around the sequence moved it 345 -> 353
MIPS, confirming the sequence itself is the floor. Getting this row needs
either a cheaper exact algorithm or a wasm fma.

**compile (tcc -c)**, ~2.2x behind v86. Measured, not
guessed: the row is ~100% user-mode tcc, 340M guest instructions at 9 insns
per dispatch. It is not module-build cost (~4us each), not TLB-miss bails
(removing all 1.2M of them is a wash), and not superblock coverage (more page
functions cost more than the dispatches they save). It is that tcc's hot code
is a call graph spread across many pages, and a page-contiguous region cannot
hold a caller and a callee 16KB apart — so nearly every call and return is a
host dispatch. The fix is a region built from the call graph rather than from
address adjacency (a two-level dispatch over a sparse page set); that is
designed but not built.


## NEXT: INCREMENTAL REGION EXTENSION (the one project both remaining rows share)

Design, from this session's measurements (2026-07-25):

**Why.** compile (2.2x) is cross-page calls at 9 insns/host-dispatch; FOURIER
(~1.15x) is fmadd sites reachable only through cross-page libm calls, where
the shipped hardware-FMA path cannot help until the sites live inside big
functions. Every alternative is measured out: per-block tail calls (~1.2us/hop
cross-instance), page-contiguous regions (can't span tcc's call graph),
reachability-driven sparse regions (rebuild cost/codegen regressed FP rows),
build-count increases (always lose: 19 builds -> 3.1s, 52 -> 5.8s), node 22
(V8 12 tiering punishes many small modules: ASSIGNMENT 19 -> 2.6), relaxed
SIMD on node 20 (experimental flag taxes all compilation).

**What.** A region function must EXTEND when its misses say so, without
discarding V8's optimized code for the old version. Since wasm functions are
immutable, extension = compile the SUPERSET function asynchronously and
REPOINT entries when it lands (machinery exists: sys_sb_ready), but with two
changes from today's rebuild:
1. The OLD function keeps running until the new one LANDS (today: entries are
   already repointed to individual blocks during the gap — that is the
   FP EMULATION 2568 -> 550 MIPS cliff). Repoint only on landing; never
   un-install the old function early.
2. Extension is DRIVEN BY MEASURED EXITS: count, per region function, exits
   whose TPC resolved to a page NOT in the region (a cheap in-function
   counter per region, or host-side: attribute dispatch-line misses following
   a region exit). Extend with the top exit-target pages, weighted; cap total
   size by V8 compile cost (~4KB/page module bytes, measured ~15-40ms per
   8-page build).
3. Keep the sparse dispatch (one range check per contiguous run — commit
   5b69da6 on branch sparse-regions has run-level discovery + run-level
   resolve, both needed).
4. The sparse-regions branch also carries the miss-counter reset + per-page
   exponential cooldown; keep both, but key the cooldown to the REGION (its
   lead page), not each member page.

**Where.** Branch sparse-regions holds the working sparse translator
(translate_superblock_sparse, call-graph selection, run-level fixes). Main
holds the same plus the network-stack preservation. The 11/13-measured JIT
is commit 46736c3 (rv64-jit/rv64-wasm/web) if a baseline comparison is
needed.

**Acceptance.** The full battery (run-all stage 7 incl. amo-diff), then
ARTIFACTS=<sc> NBENCH=1 SB=1 REPS=3 scorecard: 13/13 win-or-match with no
nbench-flagged instability. FP EMULATION/ASSIGNMENT/IDEA must not regress
below their 11/13 values (235 iter/s / 19 / 4500-5000).


## OPEN (found 2026-07-25 session end): HUFFMAN collapse on the sparse tree — SUSPECTED MISCOMPILE

Every scorecard/nbench after the sparse-regions alignment reads HUFFMAN at
~131 iter/s (was 1756-2377). The JIT profile during the collapsed runs is
HEALTHY — 81 insns/dispatch, 100% coverage, tiny fallback counts — but the
kernel retires ~16x more guest instructions per self-timed iteration. A
healthy dispatch profile plus exploded per-iteration instruction counts means
the guest is EXECUTING DIFFERENT CODE PATHS, i.e. a suspected miscompile that
perturbs data-dependent loop trip counts without failing alu/mixed checksums,
the 150-program differential, amo-diff, or boot.

Prime suspect: the run-level sparse changes (originally branch commit
5b69da6, carried to main in the 0e64d5b alignment) — whole-run leader
discovery and run-level TPC resolve — interacting with F-extension bodies.
The last configuration with a GOOD full-scorecard HUFFMAN (1756) was the
cap-3 sparse tree WITHOUT the run-level changes (commit a19ea3b).

Next session, FIRST: bisect HUFFMAN between a19ea3b's JIT and HEAD (fast
check: SBFORCE=1 nb-rvonly, HUFFMAN row only), and extend a differential to
cover an in-guest compression loop (HUFFMAN-shaped: bit-packing over
data-dependent branches) so this class is caught by the battery, not by the
scorecard. Until it lands, scorecard results from this tree should not be
trusted for FOURIER/HUFFMAN comparisons either (FOURIER also reads unstable).


**Confirmation + reframe (last measurement of the session):** with
superblocks DISABLED on this tree, HUFFMAN returns to its no-SB shape
(688 iter/s — the miscompile is superblock-resident, confirmed), and
**FOURIER measures 8031 iter/s — ABOVE v86's 7335-7664**. FOURIER's loss was
never an engine/FMA floor: the current tree's superblocks actively cost it
~25%. Fixing the run-level superblock miscompile and the sparse FP
regressions is therefore the single path to 13/13: FOURIER wins once
superblocks stop hurting it (or bodies carry the shipped hw-FMA on a fixed
tree), and compile keeps its planned incremental-extension route.


**Bisect note:** SB-on + wallclock-off reads ALL kernels lower (guest time
becomes insn-derived; not comparable across configs) — that experiment does
NOT discriminate miscompile vs build-time-in-guest-clock. Two live theories,
in test order: (a) run-level sparse body/discovery miscompile — bisect
a19ea3b -> HEAD with SBFORCE=1 wallclock-ON HUFFMAN rows; (b) superblock
BUILD TIME (translate + leader analysis at quantum boundaries, several ms per
8-page region) charged to guest wall-time, compounding on rebuild-heavy
kernels — test by timing build_superblock host-side (add a jit_stat of
cumulative build ms) and correlating with the HUFFMAN deficit. If (b), the
fix is cheaper/incremental builds — the same incremental-extension project.


## MEASUREMENT INVALIDATION NOTICE (2026-07-25 late session)

Everything measured in the late session — the "HUFFMAN 9x collapse", the
"FP EMULATION 6x collapse", the FOURIER swings (2611 / 5107 / 6485 / 8031),
the IDEA bimodality — was taken while this shared host ran a concurrent
toolchain build (load average 29 on 24 cores). Proof: the KNOWN-GOOD JIT
(46736c3) re-measured in that window reads FP EMULATION 896 MIPS, against
2708 MIPS for the same commit that morning. A 3x host swing swamps every
effect being chased.

Consequences, in order of importance:

1. **The scorecard now guards against this** (commit 2c56094): a fixed CPU
   probe runs before the v86 legs, before the rv64 legs and at the end; the
   spread is printed and the run is marked INVALID above 1.25x. Because the
   harness runs all v86 legs before all rv64 legs, drift previously landed as
   a systematic bias against whichever side ran later.
2. **The "superblock miscompile" hypothesis is unproven and probably wrong.**
   The evidence for it (HUFFMAN 131 with SB on, 688 with SB off) came from
   that window. Before doing anything else, re-run the SB-on/SB-off pair
   BACK-TO-BACK on a quiet host.
3. **The bisect conclusions are unreliable** for the same reason, though the
   two settings they produced (3-page region cap, per-page discovery) match
   the configuration that independently measured well earlier.

### First actions next session (quiet host: `uptime` load < 4)

    # worktrees left in place for a back-to-back A/B:
    #   /tmp/good46  = 46736c3 (the JIT behind both 11/13 runs)
    #   /tmp/jitlab  = sparse HEAD
    ARTIFACTS=<sc> SBFORCE=1 node <sc>/nb-good-stats.mjs   # alternate
    ARTIFACTS=<sc> SBFORCE=1 node <sc>/nb-lab-stats.mjs    # these two
    # then, whichever wins, the authoritative run:
    ARTIFACTS=<sc> NBENCH=1 SB=1 REPS=3 node tests/vs-v86/scorecard.mjs
    # and CHECK the printed host-probe spread before believing any row.

Only after that does it make sense to resume the two open engineering items
(incremental region extension for compile; FOURIER's fmadd sites reaching
superblock bodies where the shipped hardware-FMA path applies).


**Automated verification pending:** `scratchpad/when-quiet-scorecard.sh` is
armed — it polls until this shared host's load drops below 6, then runs
`ARTIFACTS=<sc> NBENCH=1 SB=1 REPS=3 scorecard.mjs` and writes
`scratchpad/quiet-scorecard.txt`. Because the scorecard now guards its own
conditions (CPU probe, INVALID above 1.25x spread), that file is either a
trustworthy 13-row result or states why it is not. Read it before doing
anything else with the two open rows.


# SESSION 2026-07-26: trace JIT, measured region lifecycle, quiet-host truth

The armed quiet-scorecard and its artifacts did not survive /tmp cleanup;
everything was rebuilt from setup.sh (plus a fresh copy/v86 build — nix
GitHub tokens in ~/.config/nix/nix.conf were stale ghs_ tokens; overridden
per-command with `NIX_CONFIG="access-tokens = github.com=$(gh auth token)"`).

## Quiet-host verdicts on the open items

- **The HUFFMAN "superblock miscompile" does not exist.** On a quiet host
  (drift guard 1.17x) HUFFMAN WINS 1.22x with SB on. The collapse was host
  load, as the invalidation notice suspected.
- **FOURIER was never blocked on FMA.** It reached MATCH 0.99x on the
  morning baseline and WINS 1.15-1.19x after this session's work.
- **python fib is BIMODAL across boots** (3.3-6.7s on identical code, both
  old and new trees). Single runs and even median-of-3 are soft; treat any
  python row verdict within ~1.3x as a lottery until the coverage race is
  fixed. A/B claims made from single python runs this session were
  repeatedly wrong.
- **ASSIGNMENT/HUFFMAN/NUMERIC/STRING regressed between 46736c3 and the
  session start** (ASSIGNMENT 12.6 -> 9.5, HUFFMAN 1525 -> 959 measured
  with 46736c3's wasm on today's artifacts/host). Root cause found in
  build_superblock: a leftover bisect configuration ("SUB-BISECT(i):
  per-page discovery") ran leader discovery per page from only that page's
  own seeds, so pages without recorded hot pcs contributed no bodies and
  functions covered fragments. Whole-run discovery is restored (1ebecdc).
  ASSIGNMENT recovers only to ~8.4-8.9 — the rest of its gap is still open.

## What landed (commits e950060, <overwrite-commit>, 1ebecdc)

1. **Trace compilation**: individual blocks are extended basic blocks —
   side-exited conditional branches with exact per-exit retired counts,
   direct-call following both directions (link register is a compile-time
   constant), leaf-return following, and guarded stack-restored-return
   following (store-to-load forwarding of ra with a one-compare guard).
   Multi-page trace windows (cached 64-page gathers), span-based dirty/map
   verification through the regions table, entry fuel guards on long
   traces. This alone: compile 3677 -> 2926ms at SB=0.
2. **Measured region lifecycle**: per-landed-function exit sampling (stay
   lengths, out-of-region targets) drives demotion (functions that provably
   do not hold return their pcs to traces), extension along measured exit
   traffic (memory-direct bodies for short-stay regions), and a rebuild-
   pressure fix (only short uncovered blocks count as misses — traces mint
   hot pcs continuously and every one counted as miss pressure, reviving
   the rebuild-churn cliff).
3. **Correctness**: the sparse-superblock rewrite had dropped the
   emit_fp_flags call — hoisted FP-gate locals stayed zero and every
   per-body gate silently passed (FP bodies unguarded under non-RNE
   rounding / FS != Dirty). Restored. Trace FP gates emit AT the first FP
   instruction as a mid-trace side exit (a kernel-page trace sweeping one
   FP insn otherwise zero-retire-thrashed at 93M dispatches).
4. **Dispatch cost**: overwrite retirement contract (no host zero-store, no
   read-add-store per exit), fuel-cell store thinning in the sys chain loop.
5. **Config**: TRACE_KEEP_MIN=0 (functions claim every entry — mixed
   claiming fragmented execution), demotion stays as safety valve.

## Row status after this session (fresh v86 legs, quiet host)

WIN: ALU 1.37x, Mixed 1.44x, Boot 1.87x, STRING ~1.2x, BITFIELD 2.7x,
FP EMULATION 2.2-2.3x, FOURIER 1.19x, IDEA 2.7-2.8x, HUFFMAN 1.2-1.3x.
THIN: NUMERIC SORT ~1.04-1.13x (regressed from the trace work; wants
trace-keeping that every other row wants gone — unresolved conflict).
LOSS: ASSIGNMENT ~1.26x behind, python ~1.2-1.4x behind (lottery),
compile ~2.2-2.4x behind (3282ms vs 1382ms).

## Open engineering, in expected-value order

1. **compile**: per-dispatch overhead is ~85ns x 23-27M dispatches. The
   remaining structural ideas: full retired-in-return ABI (blocks return
   the count), same-instance block batching so chain transfers avoid the
   1.2us cross-instance tail-call tax, branch-direction profiles for
   longer executed traces. Together they model out to ~1.5s — MATCH range.
2. **ASSIGNMENT**: 46736c3 proves 12.6 exists. Its hot shape is rotated
   bottom-tested scan nests the loop detector rejects (backward exit
   branches); trace unrolling attempts measured net-negative because they
   displace region coverage. Either teach loop_region rotated nests or
   find what else the 46736c3 formation did differently (its rv64-wasm
   policies, not the translator — contiguous emission is equivalent
   post-gate-fix).
3. **python**: fix the coverage race behind the 3.3-6.7s bimodality before
   chasing the mean; suspects are dispatch-slot collisions under guest
   ASLR (2-way dispatch associativity is untried) and function-landing
   timing.

## SESSION 2026-07-26, closing addendum (afternoon)

Further landed (commits e950060, 1ebecdc, 53ec078 + the retirement-contract
commit): rotated-nest backward-exit loop regions, 2^18 dispatch lines, bulk
table growth in the JS shim, multi-function block modules (shared chain
helper), an online chain A/B controller, and the chain saga's conclusion:

**The V8 shared-table import quadratic** (the day's most important finding
for future work): return_call_indirect costs ~2ns/hop on node 20.18.1 (the
old 1.2us figure is obsolete), and fully-chained nbench runs produced
ASSIGNMENT 11.2 iter/s — a clear WIN over v86 — plus records on IDEA/
HUFFMAN/FOURIER. But any module importing __indirect_function_table makes
every subsequent table.set O(importing instances), so large block
populations (tcc 7.5k, CPython 10-20k) pay a quadratic registration cost
that no runtime gate can avoid (proved by kill-cell: 2.4x slowdown with
chain code emitted but never executed). Chaining ships default-off behind
RV_TAILCALL=1. The unlock for next session: a second chain-only table, or
transfers routed through a host-module chain_run export, so trace modules
never import the table. That is the highest-EV single project: it flips
ASSIGNMENT and materially helps python and compile.

**Benchmark lottery, quantified**: on identical binaries and a quiet host,
boot-to-boot coverage races swing NUMERIC SORT 320-467, HUFFMAN 724-1016,
ASSIGNMENT 7.6-8.9, python fib 3.4-7.8s. Config knobs (KEEPMIN 0/24/48,
DEMOTE_MIN_SAMPLES 16/64) all measure within that noise. Single-sample
nbench rows and REPS=3 python cannot be trusted to +/-15%. Any future
"win/loss" claim on those rows needs interleaved multi-boot medians, and
the scorecard should grow that capability before more tuning is attempted.

Authoritative full runs today (all valid by the drift guard, all at
provenance-recorded commits): 10/13 (morning baseline), 10/13 (traces +
demotion), 10/13 (discovery fix + claim-all), 9/13 (final; NUMERIC drew
360 vs v86 404). Rows that never lost today: ALU, Mixed, Boot, STRING,
BITFIELD, FP EMULATION, FOURIER, IDEA. HUFFMAN won in every full run.
Structural losses: compile (2.1-2.2x, best 2912ms vs 1352-1382ms), python
(1.2-1.4x, lottery), ASSIGNMENT (1.26-1.33x; 11.2 proven reachable with
chains). NUMERIC became draw-dependent this session (was a stable 568
pre-trace-work) — its stabilization is unfinished business tied to the
same demotion/coverage races.

## SESSION 2026-07-26/27, part 3: the dispatch-cost dead ends (all measured)

Machine changed mid-session to a 64-core Threadripper; `tests/vs-v86/screen.mjs`
(parallel K-boot medians, SCREENING-stamped) now gives ~8-minute A/B loops
against the serial scorecard's ~40 minutes. Every verdict below is an
INTERLEAVED multi-round A/B, the only method that survives the boot lottery.

**The compile row's ~100ns/dispatch overhead resists every structural fix
tried.** Six architectures, all rejected:

1. *Emitted return_call_indirect chaining* — ~2ns/hop, but modules importing
   the shared table make table.set O(importing instances): quadratic
   registration (tcc 2.4-3x slower, even with the code never executed).
2. *Shared per-module chain helper* (multi-function modules, ~15 bytes/site
   instead of ~80) — same import, same quadratic.
3. *env.chain_next host dispatch* (function import like tlb_fill; no table
   import, no quadratic) — mechanically perfect (tcc dispatches 25M -> 3.7M
   at 94 insns each) and SLOWER everywhere (ASSIGNMENT 8.3 -> 6.2, python
   4.6 -> 6.2s): the host loop is already wasm with no JS frame, so a hop
   that re-does its bookkeeping plus two call frames cannot win.
4. *Batch modules with DIRECT tail calls* (translate_batch: N bodies in one
   module, r0..rN-1, links guarded on dispatch-line pc + map generation +
   fuel) — no table import, O(1) registration, ~2ns/hop, 766 batches /
   2965 members on tcc, dispatches down 12%. Still loses 3 of 3 rounds
   (4356/4075/4069 vs 4558/4169/4336ms): only ~12% of exits stay in-batch,
   so ~88% pay the guard for nothing. Caps 32/64 are worse.
5. *Page-co-located batches* (test of the "V8 instance switch dominates"
   hypothesis: pack a page's hot pcs into one instance) — loses 2 of 2
   rounds at both caps. Instance switching is NOT the dominant cost.
6. *Superblock build spacing* — 16M insns is optimal; 4M and 1M lose
   (4244/4308/5083 and 4345/4467/4888), i.e. rebuild churn still dominates
   any coverage gain.

Also rejected this session: rotated-nest loop regions (net-negative on every
kernel), KEEPMIN 24/48 (0 is row-optimal; FP traces exempted either way).

**What the evidence now says about 13/13.** The three open rows are all
branchy, dispatch-bound code where the per-dispatch cost is the JIT's
*bookkeeping*, not any boundary we can remove: compile 2.19x, python 1.43x,
ASSIGNMENT 1.20x. Closing them needs a different class of change than
anything tried here — cross-block register allocation so exits don't spill
and reload the register union, i.e. a real trace-tree/region JIT with live
ranges, which is a multi-session project rather than a knob. The measured
upper bound for chaining-style fixes is ~12% of dispatches, worth a few
percent; the gap needs 2x.

Standing (authoritative, drift 1.01x, REPS=3 NBREPS=3, commit d7428f8):
10/13 win-or-match. WIN: ALU 1.41, Mixed 1.82, Boot 1.82, STRING 1.22,
BITFIELD 2.73, FP EMULATION 2.23, FOURIER 1.18, IDEA 2.78, HUFFMAN 1.28.
MATCH: NUMERIC SORT 1.05. LOSS: ASSIGNMENT 1.20, python 1.43, compile 2.19.

## SESSION 2026-07-27: validation restored, and the real diagnosis

**Four correctness gates were silently not running.** Fixed (4464a6f,
318b55d, 15be658): `rv64-isa-test` restored (deleted, unstaged, no
replacement — one API fix, `BootImages.fs` is now `Vec<p9::Server>`), which
un-skipped riscv-tests **134/134**, Spike lockstep **LOCKSTEP-OK**, and
riscv-arch-test **193 match / 0 mismatch** (it read 0/193 while the binary
was missing); `amo.rv64` given a producer AND `-mno-relax` (plain C `_start`
+ relaxation stored through an unset gp — instant fault, null checksums), so
**AMO DIFFERENTIAL: PASS (interp == jit == superblock)** runs for the first
time; `http-relay.mjs` no longer crashes the suite on Node 20's flagged
WebSocket; and `run-all.sh` got `pipefail` — stages 5/6 pipe into `tail`, so
a failing lockstep or signature comparison could print its failure and the
suite would still report ALL STAGES PASSED. `rv64-boot.rs`/`rv64-vboot.rs`
stay deleted: their HEAD versions predate the RTC/`--proxy` work.

**The sharpened diagnosis for the three open rows.** tcc's traces average
**15.5 instructions against a 256-instruction cap** — the cap is not
binding. Traces end on INDIRECT control flow: function pointers, switch
tables, and returns whose base register is not a traced constant. That is
why seven successive dispatch-cost architectures all measured neutral
(chaining x3, batch modules, page co-location, build spacing, definedness
tracking): they attack the COST of a dispatch when the binding constraint is
the COUNT, and the count is set by indirect branches. Batch formation was
even rebuilt around the observed successor chain (next-executing-tail, one
store per dispatch) — still neutral, because the successor of a
15-instruction trace is usually reached through exactly the indirect edge
the batch cannot span.

**Next lever, concretely:** inline caches for indirect targets — emit
`if (target == last_seen) <continue trace> else <exit>` at jalr sites, with
the cached target refreshed by the host. That extends traces through the
edges that currently end them, which is the only thing that moves the count.
Everything else in this area is now measured and closed.

Standing (authoritative, drift 1.00x, REPS=3 NBREPS=3): **10/13**.
