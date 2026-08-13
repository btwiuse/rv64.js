# Pure-Interpreter Anti-Overfit Protocol

Date frozen: 2026-08-10 (America/Phoenix)  
Status: frozen evaluation complete; overall goal not met

## Purpose

The earlier 12-win/1-match interpreter result is withdrawn. It used exact
multi-instruction recognizers selected after observing scorecard losses. This
protocol defines the evidence required for a replacement claim.

## Production eligibility

An interpreter optimization may depend on ISA semantics and ordinary machine
state: decoded opcode fields, privilege, page permissions, mapping generation,
alignment, and architecturally visible registers or memory. It may not select
behavior from:

- a benchmark, process, ELF, symbol, or guest executable identity;
- a measured guest PC or a list/range of PCs;
- an exact multi-instruction byte sequence from a workload;
- a scorecard row, phase, output, or host-side benchmark setting; or
- a negative/positive cache populated specifically for a measured sequence.

General decode, TLB, permission-proved memory, interrupt-polling, and
architecture-complete instruction-family improvements are eligible. A dynamic
decoded-block or loop mechanism is eligible only if its selection rule is
architecture-wide, its complete semantic class is tested, and it transfers to
the sealed workloads. Preserving guest state is necessary but not sufficient:
post-hoc workload specificity still disqualifies a result.

## Comparator and development population

The pinned copy/v86 comparator is commit
`2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`; its `src` tree matches the commit.
Its interpreter contains no benchmark-name, fixed-PC, or measured opcode-body
recognizer. Its REP string fast path implements one architecturally defined x86
instruction.

The existing 13-row scorecard is development evidence. It has a pre-existing
guest-input asymmetry: only RV64 BYTEmark links the custom
`tests/vs-v86/nbench-extras/fastmem.c`, whose own comment names String Sort as
the motivation. Therefore final evidence must also include a stock-libc or
same-portable-implementation cross-ISA population.

That audit population is now frozen as `stock-musl-v1`. Both fixed-work
BYTEmark binaries use the same nbench 2.2.3 source, fixed-data32/fixed-work
patches, Zig `-O2 -static` build, and `-fno-builtin-memmove` /
`-fno-builtin-memcpy`; neither links a replacement implementation. Its exact
identities are:

| Input | SHA-256 |
| --- | --- |
| RV64 nbench | `1ada7fb2cbd4974dda4d7759e35f84f1933bd194d46f457f50d7ba8d068001eb` |
| i386 nbench | `0bcdb4594d083a7540b1b87e13944a190130c0b99ebd96f7e8b6e132f02500dc` |
| RV64 initramfs | `e75b6df497c8a19b590146f1cc00f04837338c6feb97ec51cc219188b79284f7` |
| x86 initramfs | `4901c733a0c3d2f048b196c71f5a8f8af79909fac1a9391850a2e874b67d22e4` |

The complete manifest is
`target/bench/interpreter-stock-musl-v1/SHA256SUMS`. Archive overlay parsing
proves that these binaries replace the former scored executables. At the
freeze, neither emulator had executed this population and it was forbidden as
a tuning input.

After development tuning was closed at exact I004 and the strict correctness
matrix passed, the population was executed once under the frozen authoritative
three-repetition protocol. All 78 trials are eligible and JIT-inactive, but
I004 loses stock String at `0.3076x` (`3.25x` behind). The goal is not met and
this opened population cannot select another edit. See
[INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md).

The clean development baseline uses Wasm
`a2f42e55070478dd162ded55e58a7d4be2b050d859da7a0d7b48a94352336095`.
Its valid one-repetition all-row report wins 11 rows and loses String Sort
(`0.2400x`) and Bitfield (`0.8178x`). It is not an authoritative parity result.

The subsequently frozen architecture-general I004 artifact
`7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`
has now completed the authoritative three-repetition development gate. It wins
eleven rows, matches one, and loses String at `0.3186x`; all 78 workers prove
JIT inactivity and the report is measurement-valid. I004 therefore fails
before stock-musl or holdout execution. Exact evidence is in
[INTERPRETER_I004_AUTHORITATIVE_DEVELOPMENT_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_DEVELOPMENT_RESULT.md).

The post-result mechanism audit now closes development-informed tuning at
exact I004 before any sealed execution. It freezes one final stock-musl run and
one authoritative holdout run for comparison evidence, with no subsequent
optimization permitted from their contents. See
[INTERPRETER_STRING_PLATEAU_AND_UNSEAL_PROTOCOL.md](INTERPRETER_STRING_PLATEAU_AND_UNSEAL_PROTOCOL.md).

## Sealed holdouts

The holdouts reuse the paired Linux 6.12.7 / Alpine 3.24.1 root filesystems and
their stock BusyBox/OpenSSL package binaries. A deterministic overlay adds the
same scripts and data to both architectures. The four workloads are:

1. BusyBox gzip, four fixed 1 MiB compressions;
2. BusyBox `LC_ALL=C` sort, 75,000 fixed records;
3. BusyBox SHA-256, 32 fixed 1 MiB hashes; and
4. OpenSSL AES-256-CTR, sixteen fixed 1 MiB encryptions.

The immutable input hashes are:

| Input | SHA-256 |
| --- | --- |
| holdout overlay | `5723a78e39f687171ff7f1ba9645dbbf8adee25da1a345d23934e5edcacc0d37` |
| RV64 initramfs | `aa6cc4197f8aa685b46f9d8d5c2d6cdf10f6bb03fca0a26377a6825c1978b30a` |
| x86 initramfs | `34cae1e01a1fa421b3af4eb76ba0a1209e3d167bd1b8a95f689c99a0259e2a24` |
| data generator | `a8362ed225f6826716290d6f753ef4ee9a51fb1e0f9052541442a442797514ee` |

The complete source and artifact manifest is
`target/bench/interpreter-holdouts-v1/SHA256SUMS`; the work/result contract is
`target/bench/interpreter-holdouts-v1/contract.json`. The inputs were built and
their archive structure verified before either emulator executed a holdout.
They could not be profiled, screened, or consulted while selecting or tuning a
candidate.

After exact I004 was frozen and the stock result was retained, the holdout was
executed once at `AUTHORITATIVE=1`, `REPS=3`. All 24 trials are eligible,
JIT-inactive, and checksum-correct. I004 wins all four rows by `1.7171x` to
`2.5409x`, so the holdout-local gate passes. The population is now opened and
permanently forbidden as a tuning input. The overall goal remains not met
because development and stock String failed. See
[INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md).

## Evaluation and acceptance

After a candidate and all thresholds are frozen:

1. rerun formatting, unit tests, the JIT-disabled bypass proof, and all
   differential suites;
2. run the complete existing 13-row interpreter scorecard with odd `REPS>=3`;
3. run the fair stock/same-implementation population with odd `REPS>=3`; and
4. run `tests/vs-v86/interpreter-holdout.mjs` exactly once with
   `AUTHORITATIVE=1` and odd `REPS>=3`.

Every worker must prove JIT inactivity and exact input/output identity. Every
development, fair-input, and holdout row must meet the frozen `0.95x` match
floor; sample and host-probe spreads must remain within the scorecard limits.
Any holdout loss is a failed candidate, not permission to inspect the workload
and add another specialization.

The JIT-only whole-loop bulk-copy lowering is unreachable in interpreter mode
and receives no credit here. It is an exact compiler-idiom recognizer and must
be separately removed or independently transfer-qualified before a future
JIT-enabled parity claim.
