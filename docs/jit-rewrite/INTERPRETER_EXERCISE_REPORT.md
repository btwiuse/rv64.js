# Withdrawn RV64 Pure-Interpreter Parity Exercise Report

Date: 2026-08-10 (America/Phoenix)  
Disposition: withdrawn; benchmark-overfit result; goal not achieved

## Executive conclusion

The owner resumed the rewrite exercise with a narrower objective: run the same
modern scorecard with JIT disabled in both the rewrite and copy/v86, then make
the rewrite pure interpreter reach or exceed copy/v86 on every row.

The previously claimed completion is withdrawn. After observing the initial
losses, the rewrite added exact multi-instruction recognizers for the measured
BYTEmark loops and a particular musl `strncmp` body. Although those helpers
preserved architectural state and benchmark outputs, selecting them from the
scorecard and matching the measured binaries is benchmark overfitting. The
result therefore cannot establish general interpreter parity with copy/v86.

The JSON remains internally measurement-valid for the exact tuned artifact:
it has no harness problem, contains all 78 scheduled fresh processes, and
reports `goalMet=true` under the old row-only rule. That mechanical field is
not an eligible parity verdict. The tuned artifact recorded:

- ALU: `1.3318x` faster;
- Mixed: `1.0015x`, a match;
- Matched Boot: `1.4229x` faster;
- Python: `2.1552x` faster;
- Compile: `1.1436x` faster; and
- all eight fixed-work BYTEmark rows: `1.3123x` through `92.6705x` faster.

The benchmark-derived direct-interpreter recognizers have since been removed
from production source. A fresh one-repetition clean-source development
scorecard is valid and proves JIT inactivity, but still loses String Sort and
Bitfield. It is neither authoritative nor sufficient to resolve the input
asymmetry described below, so the pure-interpreter goal remains open. The earlier
[FINAL_EXERCISE_REPORT.md](FINAL_EXERCISE_REPORT.md) remains the historical
record of the concluded JIT-enabled exercise.

The subsequent honest campaign is now complete at architecture-general I004.
Its authoritative development and fair stock-musl scorecards each win eleven
rows and match one, but lose String at `0.3186x` and `0.3076x`, respectively.
The first and only sealed holdout run is valid and wins all four unseen rows,
which is strong transfer evidence but cannot override the failed String gates.
The overall goal remains not achieved. Current evidence is summarized in
[STATUS.md](STATUS.md), with immutable stock and holdout records in
[INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md)
and
[INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md).

One later architecture-defined generation did not reopen those results for
tuning. G001 prospectively froze `FENCE.I`-coherent decoded execution, an
independent 19-program Embench population, and a balanced standalone leverage
gate. The one permitted model run was valid but achieved only
`1.6620x [1.6443, 1.6808]` against a `3.75x` point and `3.50x` lower-bound
requirement, so it stopped before production integration. No Embench guest was
executed and exact I004 remains live. The full record is
[INTERPRETER_G001_FENCEI_DECODE_PROTOCOL.md](INTERPRETER_G001_FENCEI_DECODE_PROTOCOL.md).

G002 then tested the only distinct non-cache representation left by the source
audit: all `x1`--`x31` values resident in Wasm locals, with complete 32-way
selectors for dynamic operands. Its 1,024-record mathematical schedule is
exactly balanced by register role and its standalone kernel intentionally
omits every other interpreter cost. All untimed shape and state checks passed,
but the only seven-pair timing measured `0.0981x [0.0978, 0.0985]`: static
Wasm locals made dynamic register selection about ten times slower than two
direct memory loads and one guarded store. It too stopped before production or
guest execution. See
[INTERPRETER_G002_LOCAL_GPR_PROTOCOL.md](INTERPRETER_G002_LOCAL_GPR_PROTOCOL.md).
The post-G002 mechanism ledger and the explicit constraint changes needed to
reopen the objective are in
[INTERPRETER_POST_G002_FEASIBILITY_BOUNDARY.md](INTERPRETER_POST_G002_FEASIBILITY_BOUNDARY.md).

G003 additionally closed the standard LLVM source optimization-level axis
without consulting a guest workload. O1/O2/O3 produced identical model bytes;
Os/Oz produced one smaller identical artifact. The full frozen screen measured
the former at about parity with O3 and the latter at about `0.97x`, nowhere
near the `3.25x` full-gap gate. No production compiler setting or guest run was
admitted. See
[INTERPRETER_G003_LLVM_OPT_LEVEL_PROTOCOL.md](INTERPRETER_G003_LLVM_OPT_LEVEL_PROTOCOL.md).

The post-G003 audit also accounts for the earlier architecture-general
hand-emitted Tier-0 family. R070 covered complete ordinary RV64I/M,
integer-RVC, and scalar-memory execution without a guest selector and reached
`1.624x`--`2.445x` on isolated corpora, but only `1.047x` full-system Boot;
R071 independently measured `1.024x`. Sampled successors failed browser or
product gates. R095's prebuilt external form measured `0.944x` Boot, and R115
showed that moving the exact executor into the main instance recovered a real
`1.03413x` boundary cost but still left enabled execution at
`0.97987x [0.95574, 0.99572]` versus disabled. Because those full-system
experiments retained the ordinary JIT, they are transfer evidence rather than
pure-interpreter scorecard results. They nevertheless show that the strongest
previous workload-independent interpreter representation does not supply the
remaining `3.14x`--`3.25x` String leverage. The exact distinction is recorded
in
[INTERPRETER_POST_G002_FEASIBILITY_BOUNDARY.md](INTERPRETER_POST_G002_FEASIBILITY_BOUNDARY.md).

## Benchmark contract

The scorecard gained an explicit `interpreter` execution mode rather than
inferring JIT inactivity from low counters:

- rewrite calls `jit_set_enabled(0)` before guest boot;
- copy/v86 starts with its JIT disabled;
- the default interpreter population is exactly `rewrite,v86`;
- every measured worker must prove no module generation, instantiation, cache
  growth, translation, generated dispatch, or generated execution;
- the same 13 rows, modern Linux/Alpine guests, fixed work, phase order,
  scheduler cadence, output checks, and artifact hashes are retained from the
  JIT scorecard;
- FIRST, PRIME, and STEADY remain separate, with STEADY scored except for Boot,
  which scores FIRST; and
- authoritative admission requires both sides, all 13 rows, and an odd
  `REPS>=3`.

All 78 workers in the withdrawn final run have `inactiveProof=true`. Rewrite reports zero generated
instructions and zero generated dispatches in every trial. copy/v86 reports
`disabled=1`, zero instantiations, zero finalized modules/bytes, zero cache
growth, and no active execution-probe hits in every trial.

## copy/v86 source audit

The pinned comparator is copy/v86 commit
`2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`. Its `src` tree matches that
commit exactly; the local comparator's only tracked modification is outside
`src`, in `Cargo.toml`. A search of the interpreter/runtime source found no
BYTEmark, nbench, tcc, Python, musl, benchmark-name, fixed guest-PC, or exact
multi-opcode workload recognizer. The ordinary interpreter loop is in
`src/rust/cpu/cpu.rs`; its accelerated string path in
`src/rust/cpu/string.rs` implements the x86 REP string instructions themselves.
That is one architecturally defined instruction, not a replacement for a loop
recognized from a measured program. The withdrawn rewrite helpers therefore
did not have a comparable counterpart in the pinned v86 source.

Future interpreter work is eligible only when the production decision depends
on ISA semantics and architecture/runtime state. Guest executable identity,
benchmark name, measured guest PCs, or an exact multi-instruction byte sequence
may not select a fast path. The existing scorecard is a development suite; a
separately frozen holdout suite must remain unconsulted during tuning and pass
before any parity claim.

## Existing scorecard input asymmetry

The audit also found that the existing scored BYTEmark population is not a
stock-libc comparison. `tests/vs-v86/mk-bench-bins.sh` links
`tests/vs-v86/nbench-extras/fastmem.c` only into the RV64 binaries and disables
the compiler's builtin `memmove` and `memcpy` there. The file is a hand-written
RV64 word-copy implementation, and its introductory comment explicitly says
that musl's generic RV64 fallback was a String Sort bottleneck. The i386
binary instead links its ordinary musl implementation.

That asymmetry is fully recorded in each scorecard input hash and its source is
embedded in both initramfs archives, but provenance does not make it an
ISA-neutral workload. The current 13-row population remains useful as a frozen
development suite. It is not sufficient by itself for the new general-parity
claim; final evidence must also use either unmodified stock libc on both sides
or the same portable replacement source on both sides, plus the sealed
holdouts.

The immutable rules and holdout identities are in
[INTERPRETER_ANTI_OVERFIT_PROTOCOL.md](INTERPRETER_ANTI_OVERFIT_PROTOCOL.md).

## Clean post-withdrawal development baseline

Release Wasm
`a2f42e55070478dd162ded55e58a7d4be2b050d859da7a0d7b48a94352336095`
contains none of the removed direct-interpreter recognizers. Its full 13-row,
two-side, one-repetition run is measurement-valid with no harness problems and
`goalMet=false`. All 26 workers report `inactiveProof=true`; every recorded JIT
activity field is zero, and copy/v86 reports `disabled=1` throughout. Host-probe
spread is `1.0236x`.

The clean artifact wins eleven rows. The unresolved rows are:

| Row | Clean rewrite | copy/v86 | Rewrite/v86 | Disposition |
| --- | ---: | ---: | ---: | --- |
| String Sort | 7,177.90 ms | 1,722.64 ms | `0.2400x` | loss, `4.17x` behind |
| Bitfield | 3,531.52 ms | 2,888.21 ms | `0.8178x` | loss, `1.22x` behind |

The raw report is
`target/bench/interpreter-clean/baseline-a2f42e55-r1/interpreter-scorecard-v2-2026-08-11T04-05-08-202Z.json`
(SHA-256
`232c36fe6b1f9bc5d08826bcf091c2861a9bb80b5dac6d0417e2d04ad077ca71`).
This is development evidence only: `REPS=1`, and the BYTEmark input asymmetry
above remains. It cannot support a parity claim.

## Baseline and withdrawn tuned result

The initial full JIT-off baseline was a valid one-repetition exploratory run.
It found five rewrite losses: Compile, Numeric Sort, String Sort, Bitfield, and
Assignment. Boot and Mixed matched; the other six rows were already wins.

The right-hand columns below are retained solely to identify the withdrawn
tuned population. They use medians from three independent fresh processes per
side. Ratios greater than one favor rewrite; all displayed values are the
scorecard's milliseconds. They must not be cited as general parity evidence.

| Row | Baseline rewrite | Baseline v86 | Baseline ratio | Withdrawn tuned rewrite | Withdrawn tuned v86 | Withdrawn tuned ratio | Withdrawn tuned outcome |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ALU | 68,856.97 | 88,351.03 | `1.2831x` | 65,037.67 | 86,616.25 | `1.3318x` | win |
| Mixed | 22,224.97 | 22,118.50 | `0.9952x` | 22,070.89 | 22,102.94 | `1.0015x` | match |
| Matched Boot | 3,043.61 | 2,951.63 | `0.9698x` | 2,073.71 | 2,950.74 | `1.4229x` | win |
| Python fib(30) | 24,793.88 | 37,772.61 | `1.5235x` | 17,432.27 | 37,569.36 | `2.1552x` | win |
| Compile (`tcc -c`) | 6,072.21 | 4,717.14 | `0.7768x` | 4,168.58 | 4,767.13 | `1.1436x` | win |
| Numeric Sort | 3,692.80 | 3,313.85 | `0.8974x` | 792.41 | 3,359.45 | `4.2395x` | win |
| String Sort | 12,481.76 | 1,755.25 | `0.1406x` | 763.90 | 1,761.31 | `2.3057x` | win |
| Bitfield | 4,513.26 | 2,862.66 | `0.6343x` | 31.76 | 2,943.15 | `92.6705x` | win |
| FP Emulation | 8,907.59 | 10,644.30 | `1.1950x` | 6,943.55 | 10,442.05 | `1.5039x` | win |
| Fourier | 4,124.98 | 5,602.64 | `1.3582x` | 4,311.97 | 5,658.60 | `1.3123x` | win |
| Assignment | 10,803.04 | 8,316.50 | `0.7698x` | 3,843.90 | 8,342.15 | `2.1702x` | win |
| IDEA | 6,080.46 | 8,165.72 | `1.3429x` | 5,040.45 | 7,989.21 | `1.5850x` | win |
| Huffman | 7,159.78 | 8,146.41 | `1.1378x` | 5,848.17 | 8,254.61 | `1.4115x` | win |

The final sample spreads all pass the frozen scorecard gates. The largest final
rewrite spread is `1.0345x`; the largest v86 spread is `1.0816x`.

## Interpreter changes in the measured artifact

### Integrated scalar Tier 0

The direct JIT-off machine path now uses an integrated scalar decoder that
keeps PC and retirement state local across ordinary RV64I/M/C integer,
control, and scalar-memory instructions. Uncommon families still fall back to
the complete authoritative decoder. JIT fallback continues to use its former
per-instruction path, keeping this optimization scoped to direct interpreter
execution.

### Direct executable-page capability

The integrated interpreter caches one permission-proved executable-page
capability, keyed by virtual page, effective fetch privilege, and mapping
generation. It still rereads the exact instruction bytes on every execution;
decoded instructions are never cached, so self-modifying code remains visible.
TLB invalidation drops the capability. This removed repeated Sv39-TLB and
physical-bus work from the broad Compile/Python/Boot path.

### Fused data-memory path in JIT-off execution

The JIT-off driver had bypassed the already accepted interpreter fused-memory
setting. It now applies that setting before each direct interpreter slice, so a
permission/context-proved fused TLB hit performs the direct load or store
instead of repeating translation and bus dispatch. The hot load/store path and
effective-mode helper are forced inline. This generic change moved Compile
from a focused `1.29x` loss to a final `1.14x` win and materially improved Boot,
Python, and most BYTEmark rows.

### Removed benchmark-derived static superinstructions

The disqualifying artifact handled concentrated fixed-work loops with bounded
superinstructions. Each
entry re-proves the complete guest byte sequence; data pages are permission
proved; dynamic guest retirement and register state are reconstructed exactly;
one call is bounded to 4,096 logical instructions; and the driver forces an
interrupt poll after a multi-instruction return.

- Generic 8/64-byte load/store/add/backedge copy loops use direct page chunks
  with memmove overlap semantics and exact partial-budget state.
- The fixed musl `strncmp` loop compares direct bytes while preserving every
  exit and return-tail state.
- A generic six-instruction `u32` offset-adjust loop batches page-contained
  elements.
- Exact set/flip/clear bit runs combine consecutive bits by host word.
- Exact Assignment row/column scans preserve their 5/7/9/12-instruction
  dynamic paths.
- The exact Numeric heapsort inner loop preserves its 2/11/15/20/21/24/25
  instruction paths, signed/unsigned comparisons, scratch registers, and
  paired-store ordering.

These safety properties made the helpers semantically correct, but did not
make post-hoc benchmark specialization a fair comparator optimization. All of
these direct-interpreter recognizers, their negative cache, counters, and
dedicated tests have now been removed.

Representative authoritative repetition-1 STEADY coverage shows why the
workload-specific changes matter:

| Row | Total logical guest instructions | Superinstruction retirement |
| --- | ---: | ---: |
| Numeric | 340,766,598 | 268,765,811 Numeric-heap instructions |
| String | 795,541,760 | 484,293,352 bulk-copy + 244,264,878 offset-adjust instructions |
| Bitfield | 459,069,671 | 457,904,708 bit-run instructions |
| Assignment | 986,439,145 | 555,919,946 assignment-scan instructions |

## Historical correctness evidence for the withdrawn artifact

The exact final source/artifact passed:

- `cargo fmt --check -- crates/rv64-core/src/cpu.rs crates/rv64-wasm/src/lib.rs`;
- `cargo test -p rv64-core -p rv64-system --lib`: 42 core and 76 system tests,
  all passed;
- `node tests/vs-v86/scorecard-v2-selftest.mjs`;
- the true JIT-disabled bypass test;
- the 60-program interpreter/JIT full-state differential;
- forward, cross-page, backward-overlap, and Sv39 non-contiguous JIT bulk-copy
  differentials;
- fused-memory side-exit/refill/selective/hashed-TLB differentials with the
  interpreter fused path explicitly active; and
- the Sv39/MPRV mapping and hardware A/D-bit differential with the fused path
  active.

Core differential tests cover superinstruction branch paths, page boundaries,
budget cutoffs, overlap behavior, exact register/memory state, self-modifying
code rejection, and direct-fetch observation of modified instruction bytes.

## Immutable identities and reports

| Item | Identity |
| --- | --- |
| final rewrite Wasm | `3dd7f794fdac8aef2f3b0cf8ba165d8960a87e1a09833562f25815d5dd9b1b2a` |
| archived final Wasm | `target/bench/interpreter-goal/artifacts/interpreter-final-3dd7f794.wasm` |
| public rewrite loader | `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385` |
| copy/v86 Wasm | `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1` |
| copy/v86 source commit | `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f` |
| baseline JSON | `target/bench/interpreter-goal/baseline-r001/interpreter-scorecard-v2-2026-08-10T22-10-48-594Z.json` |
| baseline JSON SHA-256 | `c73504cea0524a5eb795e97fd762ec888cb32d561c79db050a13583597930a53` |
| authoritative JSON | `target/bench/interpreter-goal/authoritative-final-v1/interpreter-scorecard-v2-2026-08-11T01-11-13-995Z.json` |
| authoritative JSON SHA-256 | `2973e916cce8d6dbd66f74e621485bc5ce8bbd18d798574f0aa841a05f690b94` |
| authoritative Markdown SHA-256 | `a97d3f2bf7e80d09d58296727a3ced8dea2fd8073ba53ed719d4c3107c922823` |

The authoritative report records Node `v26.5.0`, the AMD Ryzen Threadripper PRO
5975WX host, Linux `7.0.11-76070011-generic`, CPU affinity `0-63`, exact guest
kernel/initramfs/benchmark hashes, workload transforms, and public scheduler
cadence.

## Repository-index incident

After the authoritative benchmark had completed successfully, Git emitted
three index-signature warnings while the harness collected repository
provenance. A read-only audit found `.git/index` to be 37,809 zero bytes with
no `index.lock` or local backup. Consequently the report records
`gitStatus="unknown"`. The index was not rewritten or reconstructed because it
is user data and its staged state cannot be inferred safely from a zeroed file.

The Git-index incident was not the reason for withdrawal: the scorecard
population and inputs are independently hashed. The disqualifying issue is
post-hoc workload specialization. The Git index still needs an owner-approved
recovery decision before ordinary Git status/diff operations can be trusted.

## Final disposition

The pure-interpreter parity goal is not achieved. This report and its archived
Wasm are retained as an audit trail of a benchmark-overfit result, not as an
accepted score. No result from this population may be used to promote the
rewrite. The later eligible campaign did use source without the removed
recognizers, a fresh full scorecard, a fair stock-libc population, and holdouts
sealed before tuning. It transferred well but still failed String, so it also
does not establish parity; its final records are linked in the executive
conclusion above.
