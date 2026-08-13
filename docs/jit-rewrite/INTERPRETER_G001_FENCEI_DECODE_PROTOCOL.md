# G001 FENCE.I-Coherent Decoded Interpreter Protocol

Date opened: 2026-08-11 (America/Phoenix)  
Status: rejected at the frozen opportunity gate; no production edit

## Objective

Test one fundamentally different pure-interpreter representation that could
plausibly close the remaining cross-ISA work-amplification gap without
recognizing a benchmark, guest PC, binary, symbol, or measured instruction
sequence.

The official RISC-V Zifencei specification states that instruction fetch is
not required to observe prior stores to instruction memory until the hart
executes `FENCE.I`, and that fetches after `FENCE.I` must observe prior stores
visible to that hart:
[Zifencei specification](https://docs.riscv.org/reference/isa/unpriv/zifencei.html).
The current emulator implements a stronger, eagerly coherent model and treats
`FENCE.I` as a no-op. G001 tests an architecture-defined instruction-decode
cache whose coherence boundary is the standard instruction itself.

This remains a pure interpreter. It may cache normalized instruction fields
and execute them through a precompiled semantic dispatcher. It may not emit,
compile, instantiate, or dispatch guest-specific WebAssembly.

## Relationship to rejected I008

I008's exact-revalidated decoded blocks remain rejected. Its target timings
cannot admit G001, choose a parameter, or count as evidence for G001. The I008
campaign prohibited changing its capacity, length, validation, storage,
hashing, and boundaries after observing its result.

G001 is therefore admissible only as a new generation under all of these
conditions:

1. its complete design and geometry are frozen below before implementation;
2. the already opened development, stock-musl, and holdout populations are
   forbidden during implementation and candidate selection;
3. a new third-party transfer population is built and cryptographically frozen
   before production code changes;
4. only one implementation receives timing; there is no validation, boundary,
   capacity, hashing, or storage successor after any result; and
5. the candidate must pass the new population before the known scorecards are
   used solely as final acceptance tests.

If any condition cannot be met, G001 is an ineligible I008 rescue and stops
without a production edit.

## Frozen control

- `crates/rv64-core/src/cpu.rs`:
  `d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`;
- `.cargo/config.toml`:
  `252a344de3e565c134906a497e33f88795eae1a29f1357bbfb05ffea911bc267`;
- release Wasm:
  `7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`;
- public loader:
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- pinned copy/v86 commit:
  `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`; and
- copy/v86 Wasm:
  `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1`.

No runtime-Wasm override, profiler-derived selector, JIT activity, altered
guest library, or scorecard-specific build is eligible.

## Frozen G001 design

The candidate is exactly one direct-mapped cache of 64 entry-PC blocks, each
holding at most 32 normalized scalar instructions. These values are inherited
unchanged from I008 rather than selected from a result: 32 is the existing
architectural interrupt-poll cadence, and retaining 64 prevents a capacity
sweep.

The frozen slot function is
`((virtual_pc >> 1) ^ (virtual_pc >> 12)) & 63`. Each normalized record is the
following 32-byte array-of-structures layout: virtual instruction PC (`u64`),
sign-extended immediate (`i64`), operation (`u32`), `rd`, `rs1`, `rs2`, and
instruction length (four `u8` fields), and one reserved `u32` auxiliary field.
A slot contains the four `u64` key fields below, `u32` length and validity,
then exactly 32 records. These storage and hashing choices are part of G001;
they may not change after the opportunity result.

Every direct executable-RAM entry PC is eligible under the same rule. A key
contains:

- exact virtual entry PC;
- architectural fetch context;
- current address-translation generation; and
- a hart-local instruction-fetch generation advanced by every retired
  `FENCE.I`.

On a miss, the candidate fetches current bytes through the authoritative I004
permission and translation path and normalizes the complete ordinary scalar
class already handled by integrated Tier 0. A block ends at the 32-instruction
bound, executable-page end, control transfer, illegal/unsupported family, or
architectural stop. A scalar store does not end the block: without a following
`FENCE.I`, the ISA does not require a later instruction fetch to observe that
store.

On a hit, the candidate compares only the complete key. It does not reread or
compare instruction bytes. `FENCE.I` advances the generation before the next
fetch, making all older entries unreachable. SATP/SFENCE mapping changes and
fetch-context changes remain covered by their existing generation/context
keys. Page-straddling instructions and non-direct fetches use the authoritative
I004 path.

The normalized operation set, register/immediate extraction, legality rules,
and execution bodies must cover the whole I008 scalar class. There is no
opcode subset, sequence fusion, heat threshold, adaptive block size, PC list,
negative cache, process identity, or benchmark identity.

The existing eager generated-code-page invalidation remains unchanged. Any
generated path that encounters `FENCE.I` must side-exit before it so the
authoritative interpreter advances the instruction-fetch generation. G001 may
be enabled only for explicitly JIT-disabled execution until that cross-engine
contract passes.

## Fresh transfer population freeze

The population is all programs from the official Embench-IoT repository,
pinned before candidate code to commit
`09c2ed8c3b7008c95d08b038de4a3f6dc103ed70`:
[Embench project](https://www.embench.org/).

Both RV64 and i386 binaries must use the same source revision, portable harness
patch, fixed work, compiler family/version, optimization level, and libc
contract. No architecture receives a replacement memory routine or benchmark
source. Every upstream benchmark is included; no subset may be selected after
build or execution.

Before G001 source work, record a manifest covering the upstream tree, all
patches and harness sources, both compiler invocations, every binary, both
initramfs images, workload contract, and expected outputs. Building and archive
inspection are allowed. Executing either guest population is forbidden until
the candidate is complete and frozen.

### Frozen transfer identities

The population was built once, without booting an emulator or executing any
guest binary, under
`target/bench/interpreter-g001-embench-v1`. Static archive inspection confirms
that both images contain exactly the 19 named programs and the shared contract,
and no `/opt/scorecard` population.

- upstream commit: `09c2ed8c3b7008c95d08b038de4a3f6dc103ed70`;
- deterministic upstream `git archive`:
  `90d8c3efef1a7c8c19748b465757c79c711990ea1be94753ff5a78c1217d0969`;
- Zig 0.16.0 executable:
  `190a19fb057d44a1ed3b44bff68a218b29d99fc6dec8dd2353878c0e46f18c91`;
- build script:
  `dc31edd8c6a9ca75bd40ec1821ab32cdbb98478249528ad43f53f9c2912d9054`;
- guest init:
  `815ff9d7fd401f3291da8cbbd3005276307bc60f20739e35c4216cae9be7e8ac`;
- workload contract:
  `c43af9d86c8537e4bb8036f89acaf465a3e9d9c9dffbac57efa6aa60c098c975`;
- exact 38-command transcript:
  `c4bf7e66e01e5b0e612d6b1fa18b8aed832c7968a38a14e355126a8de715fa51`;
- complete artifact `SHA256SUMS`:
  `09f01a28c4620f8c8f233a8e7de50a3f325f1fe2888a407f5f5106b12f5d368c`;
- RV64 initramfs:
  `a47d548615955e715d66d0942724010e26b3876474275e4cdbf9b951bca66f1b`;
  and
- i386 initramfs:
  `c8c21430a3dc6f438faa4a3f595f83f8f673838cab44921c16c89f2f5c20aa6f`.

The exact build uses unmodified upstream `support/main.c`, `support/beebsc.c`,
the upstream native `boardsupport.c`, `GLOBAL_SCALE_FACTOR=1`,
`WARMUP_HEAT=0`, Zig `cc -O2 -static`, target-default musl, and one target
switch only. There is no architecture-specific source or library replacement.
Expected benchmark output is empty and expected status is zero through each
upstream `verify_benchmark`. The manifest and both archives are now immutable
inputs; a rebuild is not a replacement for either archive.

## Architecture-balanced opportunity gate

Before production integration, build one standalone deterministic Wasm model
with two implementations over the same architecture-balanced scalar stream:

- `control` performs exact byte fetch, length classification, field decode,
  and semantic dispatch for every operation; and
- `treatment` consumes the corresponding normalized operations through the
  fixed 64-by-32 cache representation and semantic dispatcher.

The stream must traverse every admitted 32-bit category and every admitted
compressed quadrant/funct3 family equally in a fixed pseudorandom permutation.
Registers, immediates, taken/not-taken controls, page positions, and memory
widths are balanced without workload-derived weights. Both variants perform
identical architectural state and memory updates and must match complete state
for zero work, each single operation, page crossings, more than one cache wrap,
externally mutated code followed by `FENCE.I`, and the full timed stream.

Freeze the emitter, stream hash, iterations, prewarm, process count, ordering,
and thresholds before the first timing. Run seven alternating fresh-process
pairs on CPUs 8--15. Admission requires:

- deterministic valid modules and exact complete-state equality;
- no host or within-process spread above `1.25x`;
- paired treatment/control throughput at least `3.75x` with a fixed-seed 95%
  bootstrap lower bound above `3.50x`; and
- treatment compile plus instantiate time below 25 ms.

The `3.75x` local floor is deliberately demanding: the known failing row needs
roughly a three-times whole-row gain while the integrated interpreter already
owns nearly all of its runtime. A smaller isolated improvement does not provide
a credible route to the requested end state. Failure stops G001 before a
production edit; do not change geometry, weighting, representation, or warmup.

### Frozen standalone identities and schedule

Static preparation produced a deterministic 14,475-byte module twice and ran
correctness only; it collected no timing. The final eligible identities are:

- model source:
  `27bfc111495af24e39a4f2c3e7233ac690a20e2456099dd2cab3a1e2453a0128`;
- preparation harness:
  `a127049217c86340db91f414f0a12afa37e79f457e4b053bcad2ddd9e39c12ee`;
- deterministic Wasm:
  `63f2fb590d20260c01d55186c53d8b38f9722f6798cdba6a40846de87f400026`;
- untimed preparation report:
  `5d80474a040c70ed907eefe9798d65df396fee70c7eb6ea64af8a005310496da`;
- raw stream:
  `987912d44c5d5b1f25ca26f57ed298ba9d21c1f8e8bef6ae7e535b87a2315c0f`;
- normalized stream:
  `52ed0a9f402bc8e66a038d852f1afd65336b031d9233d15844c38cf320f5284a`;
  and
- timing harness:
  `2de8dd16be05678b3abba0a986d22be8f208ef8d9b7d78920421008f8c4d5983`.

The deterministic stream has 252 scalar instructions in 44 blocks. Each of
the 62 admitted 32-bit operation kinds occurs once. Each of the 19 admitted
compressed quadrant/funct3 families occurs ten times, and every one of the 32
compressed semantic forms occurs at least once. Conditional controls are
exactly 13 taken and 13 not taken. Entry PCs occupy all 16 fixed 256-byte page
bands with counts differing by at most one, and the 44 main blocks map to 44
unique slots. The separate correctness stream traverses 129 entry PCs, more
than two complete direct-map capacities. Preparation passed zero work, every
single operation, full streams, page straddle, cache wrap, complete-key
invalidation, and stale-before/new-after-`FENCE.I` full-state comparisons.

"Balanced" here is prospective balance within the two architecture-defined
partitions, not a claim that RV64C has byte and halfword memory encodings that
do not exist. The disclosed dynamic memory-width counts per round are 3 byte,
3 halfword, 43 word, and 42 doubleword operations: the 32-bit class contributes
each legal operation kind once, while the equally weighted compressed families
contribute only their architectural word/doubleword forms. Word and doubleword
compressed counts are equal; the single aggregate difference is the ISA's
additional unsigned word load. Duplicating selected byte/halfword opcodes to
force four equal totals would break operation-family balance and is forbidden.
No count comes from a benchmark trace.

The schedule is now immutable: seven fresh-process pairs (14 processes), CPUs
8 through 14 in round-robin order from the frozen CPU set 8--15, alternating
control/treatment order, one untimed prewarm round, three 4,096-round warm
calls, and seven 65,536-round steady calls. Each round is exactly 252 scalar
instructions. Every call resets architectural state but not the decoded
cache. The report uses each process's steady median, paired control/treatment
ratios, and the repository's fixed 4,096-resample bootstrap seed expression
`0x9e3779b9 xor sample_count`. Host, side, and within-process spread limits are
`1.25x`; paired median must be at least `3.75x`, its 95% lower bound must be
strictly above `3.50x`, and every treatment compile-plus-instantiate time must
be below 25 ms.

An earlier preparation-only `interpreter-g001-model-v1` artifact was never
timed. Static review found that its compressed BNEZ cases were split 7/3 rather
than balanced. It is ineligible. The only change in v2 preserved every design,
geometry, operation count, and semantic path while assigning the already
frozen ten BNEZ cases five to each outcome. This correction occurred before
the timing harness and schedule were frozen and was driven by a failed static
balance check, not a performance result. There may be no v3 after timing.

## Correctness gates

If the opportunity gate passes, implement exactly the frozen production shape
and pass all of the following before any guest timing:

1. formatting and the complete Rust workspace;
2. exhaustive admitted 32-bit and all 49,152 compressed-encoding comparisons
   against the authoritative decoder;
3. exact registers, PC, memory, exceptions, retirement, and stop reasons at
   every budget boundary;
4. instruction-page crossing, Sv39 aliases, SATP and scoped SFENCE.VMA,
   privilege changes, fetch faults, MMIO, and unaligned/cross-page data;
5. self-modifying compressed and 32-bit code that is old before `FENCE.I` and
   necessarily new after it, including virtual aliases and same-slice stores;
6. the official Zifencei architecture suite plus the full I/M/A/C/F/D and
   privilege suites;
7. JIT-disabled bypass, full-state, memory, M, A, FP, WFI, and system
   differentials; and
8. generated execution side-exiting on `FENCE.I`, with no stale decoded entry
   after returning to the interpreter.

Any mismatch removes G001. A test may correct an implementation bug without
timing, but it may not change the frozen cache, boundary, or coherence design.

## One-shot transfer and final acceptance

After correctness and exact candidate identities are frozen, execute all
Embench rows once under an authoritative odd `REPS>=3` comparison with both
JITs disabled. Every trial must prove zero JIT activity, exact input/output
identity, and stability. Every row must meet the existing `0.95x` floor against
pinned copy/v86. A causal same-Wasm candidate/control view must have no row
below `0.95x` and a geometric-mean improvement above `1.10x`.

Failure removes G001 without a successor and permanently opens Embench as a
non-tuning input. If it passes, run the known development and fair stock-musl
13-row scorecards only as final acceptance tests, each exactly once at
`AUTHORITATIVE=1`, `REPS=3`, with all eligible samples retained. Overall
success still requires every row in both populations at or above `0.95x`.
The old holdout may be reported as an opened regression suite but cannot be
called unseen and cannot select an edit.

Only those final all-population results can establish the original parity
goal. A local, correctness-only, Embench-only, or partial String improvement is
not completion.

## Frozen opportunity result

The first and only seven-pair execution completed with all 14 fresh processes,
exact affinity, deterministic complete state, exact cache hit/miss proof, and
no worker failure. Host-probe spread was `1.021235x`; control-side spread was
`1.157101x`, treatment-side spread was `1.030174x`, and every within-process
spread passed `1.25x`. The largest treatment compile-plus-instantiate time was
`0.374810 ms`, well below 25 ms.

The decoded treatment improved the balanced standalone stream by only
`1.662022x`, with fixed-seed 95% median interval
`[1.644338x, 1.680755x]`. It therefore fails both preregistered leverage gates:
the point is below `3.75x` and the lower bound is below `3.50x`. These are the
only failed checks.

The immutable report is
`target/bench/interpreter-g001-opportunity-v1/gate.json`, SHA-256
`1e2d6daa1706bc75c0519da97109db6e245815e423a327561915af7ae4315877`.
Its decision is `close-g001-before-production-edit-without-successor`.
Consequently no cache, hash, record, geometry, warmup, or validation variant is
permitted; the production interpreter was never modified, and no Embench guest
binary was executed. Exact I004 remains live with `cpu.rs`
`d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`.
The overall parity goal remains open.
