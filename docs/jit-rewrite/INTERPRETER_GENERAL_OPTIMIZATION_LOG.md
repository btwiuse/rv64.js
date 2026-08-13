# Architecture-General Interpreter Optimization Log

Date opened: 2026-08-10 (America/Phoenix)  
Baseline Wasm: `a2f42e55070478dd162ded55e58a7d4be2b050d859da7a0d7b48a94352336095`

This log operates under
[INTERPRETER_ANTI_OVERFIT_PROTOCOL.md](INTERPRETER_ANTI_OVERFIT_PROTOCOL.md).
The stock-musl and sealed holdout populations may not be run during candidate
selection or tuning.

## I001: slice-local executable-page capability

Status: retained as an intermediate architecture-general gain; goal still open

The integrated scalar interpreter currently keeps PC and retirement in Wasm
locals, but every ordinary instruction reloads the executable-page capability,
mapping generation, and effective fetch context from `Cpu` memory. Ordinary
scalar instructions cannot change the address-space generation or execution
privilege; those changes pass through interrupt, trap, or complete-decoder
boundaries already visible to the driver.

I001 will carry one executable-page capability and its fetch context in the
integrated driver. Each instruction will still reread its exact bytes. A page
change performs the existing permission-checked translation/fill. Any
interrupt poll, scalar exception, stop, or slow-family fallback invalidates the
local capability before execution continues. The final capability is published
back to `Cpu` only for reuse by a later slice.

Selection may depend only on virtual page, effective fetch privilege, mapping
generation at a visible boundary, and direct-linear-memory availability. I001
adds no opcode, PC, binary, loop, or workload selector and no decoded-byte
cache.

Required correctness covers exhaustive scalar differential behavior,
self-modifying code, page-split instructions, TLB/SFENCE/SATP invalidation,
Sv39/MPRV, full-system memory, atomic fallback, and the explicit JIT-disabled
proof. The development timing gate is a reproducible improvement on at least
one unresolved row without making any existing winning row a loss. Failure
removes I001 rather than retuning it from row results.

Candidate Wasm is
`83321fe6137d7cd8a4103fac526fabfe88a120621338f4773e7198f9511e48f0`;
the exact control was reconstructed byte-for-byte as baseline `a2f42e55...`.
The Rust matrix and six differential suites pass. Three alternating target
pairs report Bitfield `1.097x` with interval `[1.094,1.111]` and String
`0.985x` with interval `[0.983,1.016]`. Two alternating protection pairs
report Boot `0.996x`, Compile `1.000x`, and Python `1.028x`. Both reports are
valid:

- `target/bench/interpreter-general/i001/target-ab/config-ab-2026-08-11T04-30-15-935Z.json`
  (`f2ee363219cbd0511816c86f56e13893887f166b99274fe5c6a52eee79695adc`);
- `target/bench/interpreter-general/i001/protected-ab/config-ab-2026-08-11T04-35-37-578Z.json`
  (`654ede780716a33afeda91c71d08ee35ab67e58c00b4a424973fa594be581279`).

I001 is retained because it materially improves an unresolved row and Python,
keeps existing winners protected, and is wholly architecture-general. It does
not establish parity: the projected clean Bitfield ratio remains below the
`0.95x` floor and String remains far behind.

## I002: slice-local data-page capabilities

Status: rejected and removed; exact I001 source/artifact restored

Every scalar load/store currently probes the 4,096-entry fused TLB in `Cpu`
memory, even when adjacent operations repeatedly use the same source and
destination pages. I002 will carry exactly one permission-proved load page and
one store page in the integrated driver's Wasm locals. It applies uniformly to
all scalar widths and opcodes. Page-crossing accesses, misses, MMIO, faults,
and the first access after a boundary continue through the authoritative
`ld`/`st` paths; a successful fused mapping refreshes the corresponding local.

Interrupts, traps, complete-decoder fallback, mapping/context changes, and
slice entry invalidate or re-establish the local proof. Stores retain the
existing no-generated-code-page condition. I002 adds no loop recognition,
guest-PC/opcode-sequence selector, decoded bytes, or benchmark state.

Correctness must cover all widths, unaligned and cross-page accesses, Sv39
aliases and non-contiguous pages, MPRV/SUM/MXR contexts, MMIO/fault exits,
atomics through fallback, and JIT-disabled execution. The same development
target/protection rule as I001 applies; failure removes I002 without variant
tuning.

The frozen implementation carried one load-page and one store-page capability
only in the integrated scalar slice and applied them uniformly to every scalar
width in the ordinary and compressed decoders. Its Wasm was
`12e5906336001b72cf3c9b74a9d37d1c265ed5cb5e26d57d3c416cdd70d41917`;
its `cpu.rs` was
`5473affe9c716e1f6673f7f6d9596bd3cec040b3f2a732503d100341e97404a8`.
The full workspace and six focused gates passed: the exact JIT-disabled bypass,
60-program full-state differential, all flat-memory widths and bounds fault,
fused system-memory modes, Sv39/MPRV with hardware A/D updates, and atomic
fallback.

The valid three-pair target A/B rejected it decisively. Relative to exact I001,
String was `0.935x` with interval `[0.931,0.962]`, and Bitfield was `0.885x`
with interval `[0.870,0.929]`; host-probe spread was `1.016`. The report is
`target/bench/interpreter-general/i002/target-ab/config-ab-2026-08-11T04-52-46-783Z.json`
(`25bc081bff7097ad317216ae599e3229d009c64490de03ea39b76e7f47490faa`).
Because both unresolved rows regressed, the protected-row screen was neither
needed nor run. No cache-count, refresh, invalidation, or row-specific variant
was attempted. The live source and release Wasm were restored byte-for-byte to
I001 (`c729969c...` and `83321fe6...`). The sealed stock-musl and holdout
populations remain unexecuted.

## I003: out-of-line authoritative data-memory slow paths

Status: retained as an architecture-general intermediate; goal still open

The integrated scalar body currently force-inlines both halves of every
`ld`/`st`: the accepted permission-proved fused hit and all page-crossing,
translation, refill, MMIO, bus-dispatch, and fault work. The latter remains
necessary, but it need not be duplicated inside the already-large scalar
driver after every memory-width dispatch.

I003 keeps the existing fused tag/offset probe and direct access byte-for-byte
in the inline wrapper. A full-system page-crossing access or fused miss calls
one non-inlined authoritative width-specific helper containing the current
translation/refill/bus logic. It adds no cache, threshold, PC, instruction
sequence, opcode subset, or workload identity. The selector is exactly the
architecture-wide permission/context/page proof already accepted for every
scalar memory operation. The complete decoder and JIT fallback use the same
semantic wrapper.

The implementation is frozen as one split: no `cold` weighting, width
coalescing, hit-path rewrite, page policy, or post-result inline threshold.
Correctness must pass the same complete matrix as I002, including fused-memory
disabled mode and page crossing. Three alternating target pairs compare exact
I001 on String and Bitfield. It advances only if neither row has confidence
evidence of regression and at least one improves; otherwise it is removed
without a layout variant. Existing winning rows receive the same protection
screen only after that target gate passes.

Candidate Wasm is
`b9ddb41e1f2fcf444512fea6792099514038e61ffc6393eb3fdf50f473cfd85d`;
candidate `cpu.rs` is
`7dfb28b7e81e1c31f0726c540232d0ca8760ba7c2770f0d9faef56688e2fb8be`.
The split reduced the release Wasm from 4,418,051 to 4,303,473 bytes. The full
workspace and the same six focused differential/JIT-disabled gates passed.

The valid three-pair target report measured String `1.096x` with interval
`[1.092,1.100]` and Bitfield `0.999x` with interval `[0.995,1.055]`; host
spread was `1.020`. The valid two-pair protection report measured Boot
`1.085x`, Compile `1.076x`, and Python `1.228x`, with every lower bound above
parity and host spread `1.020`. Evidence:

- `target/bench/interpreter-general/i003/target-ab/config-ab-2026-08-11T05-06-37-699Z.json`
  (`79bac477eace4273c785abd27c4c1ac7e49f3aafa055dbc40a9847df79be6a56`);
- `target/bench/interpreter-general/i003/protected-ab/config-ab-2026-08-11T05-11-13-418Z.json`
  (`58be310b757169101c76302b1a52142d073d2eb6ba346baac64163c8bd28ff3c`).

I003 is retained. It is a broad code-shape improvement and contains no new
dynamic selector. It still does not establish parity: chained from the clean
development baseline, String projects to about `0.259x` and Bitfield to about
`0.896x` versus copy/v86. No sealed population has been executed.

## I004: out-of-line executable-page miss/refill path

Status: retained as an architecture-general intermediate; goal still open

I001's local executable-page capability makes the overwhelmingly common fetch
case a tag comparison and exact halfword reread, but `scalar_fetch16` still
force-inlines translation, direct-RAM refill, physical-bus fetch, and fault
propagation into every integrated scalar monomorphization. I004 preserves the
capability-hit branch and byte read exactly, while a tag miss calls one
non-inlined authoritative refill helper containing the existing miss body.

This is not another fetch cache: there is no new state, capacity, threshold,
address rule, decoded byte, or reuse policy. Selection remains virtual page
plus architectural fetch context, and mapping/trap invalidation is unchanged.
The implementation is frozen without `cold` weighting, miss specialization,
page-size variants, or a post-result inline threshold.

Correctness and timing gates are identical to I003, with exact I003 as control.
Failure removes I004 without changing the helper boundary. The stock-musl and
holdout populations remain forbidden during this decision.

Candidate Wasm is
`7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`;
candidate `cpu.rs` is
`d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`.
The full workspace and six focused gates passed. Three target pairs measured
String `1.188x` with interval `[1.165,1.198]` and Bitfield `1.201x` with
interval `[1.160,1.206]`; host spread was `1.031`. Two protection pairs
measured Boot `1.124x`, Compile `1.094x`, and Python `0.976x`. Python's
`[0.968,0.985]` interval is evidence of a 2.4% regression, but it remains far
above copy/v86 and therefore does not turn an existing winning row into a loss.
All identities, outputs, and JIT-inactivity proofs passed. Evidence:

- `target/bench/interpreter-general/i004/target-ab/config-ab-2026-08-11T05-17-36-921Z.json`
  (`2548843bf4ec87c69d9db7567d6930145ba7e9ef565f7daa8336212a8005a53f`);
- `target/bench/interpreter-general/i004/protected-ab/config-ab-2026-08-11T05-21-49-252Z.json`
  (`18866456f216c2097e02a5394aa724f551df52c7edf0a58c424984d71d50e885`).

I004 is retained with the Python tradeoff explicitly recorded. Chained from
the clean development baseline, Bitfield projects to about `1.076x` and thus
clears the parity floor; String projects to only about `0.308x`. This reduces
the unresolved development population to String alone, but does not establish
the final result and does not permit opening the sealed populations.

## I005: slice-local interrupt-poll countdown

Status: rejected and removed; exact I004 source/artifact restored

The integrated scalar driver carries PC, retirement, and fetch proof in Wasm
locals, but still loads and stores `Cpu::irq_poll_cd` on every full-system
instruction. I005 carries exactly that existing countdown in one scalar-slice
local. It publishes the value before interrupt delivery, complete-decoder
fallback, trap conversion, architectural stop, callback stop, or slice return,
and reloads it after any operation allowed to reset the architectural polling
deadline. The polling interval, delivery priority, instruction boundary, bus
sampling, and deadline remain unchanged.

This is an architecture-wide state-materialization removal. It has no new
threshold, cadence, PC, opcode, loop, workload, or binary selector. It changes
neither interrupt sampling frequency nor guest time. The implementation is
frozen without interval changes, batching, speculative polls, or a user/system
mode variant.

Correctness adds WFI/yield and interrupt-deadline coverage to the complete I004
matrix. Exact I004 is the control for three String/Bitfield target pairs and,
only after a target pass, the existing Boot/Compile/Python protection screen.
Failure removes I005 without changing the interval or publication boundaries.
The sealed inputs remain forbidden.

Candidate Wasm was
`6c74e4efaa6c67f8fc4e97ce25d45945efe998391a80b74157f0be0441165f30`;
candidate `cpu.rs` was
`1310477ad1267c06d0ac42f4e265806139c828885b6a20b73317516edf591e4d`.
The full workspace, six focused gates, exact JIT-disabled proof, and both
system/page-policy WFI-yield tests passed.

The valid three-pair target screen rejected the frozen shape: String measured
`0.933x` with interval `[0.924,0.936]`, and Bitfield measured `0.923x` with
interval `[0.914,0.932]`; host spread was `1.079`. The report is
`target/bench/interpreter-general/i005/target-ab/config-ab-2026-08-11T05-28-45-782Z.json`
(`973e0443cfd65cd97ff45e23b784341fe9baf8e7a739a4026a7158939399b62d`).
No protection screen was run. The interval, boundary set, and system/user
shape were not changed after observation. Live `cpu.rs` and release Wasm were
restored byte-for-byte to I004 (`d8d1322f...` and `7e7cee94...`). The sealed
populations remain unexecuted.

## I006: one-read direct instruction window

Status: rejected and removed; exact I004 source/artifact restored

On a direct executable-page hit, the integrated decoder currently performs a
16-bit load, classifies instruction length, and performs a second 16-bit load
for every ordinary 32-bit instruction. I006 performs one unaligned 32-bit host
load when all four bytes lie inside the already permission-proved executable
RAM page. It returns the exact low halfword plus a prefetched high halfword;
compressed execution ignores the latter. At page offset `0xffe`, and on every
non-direct bus path, the existing translated halfword behavior is unchanged.

This is a fetch-width optimization, not decoded-byte caching: each dispatch
rereads memory, publishes no byte or opcode state, and applies uniformly to
every compressed and 32-bit instruction. It adds no PC list, opcode subset,
loop, binary, workload identity, or adaptive parameter. The single shape is
frozen without alternative widths, alignment assumptions, or page-end
speculation.

Correctness must explicitly preserve self-modifying code observation and the
`0xffe` split-page fetch contract in addition to the complete I004 matrix.
Exact I004 is the control for the same target and protection sequence. Failure
removes I006 without testing a width or boundary variant. Sealed inputs remain
forbidden.

Candidate Wasm was
`ee09a998fc3aa4d150642418ee2a16b78956dd861824cfe5a175ccfccfd9199a`;
candidate `cpu.rs` was
`8250fd5f2b16fb691529ce6d3bb15425dace70eb311019508ce4f4361b2146b6`.
The full workspace, six focused differential gates, exact JIT-disabled proof,
the explicit `0xffe` straddling-instruction case, and the self-modifying-code
fetch-capability test passed.

The valid three-pair target screen rejected the frozen shape. String measured
`0.942x` with interval `[0.932,1.109]`, while Bitfield measured `0.907x` with
interval `[0.907,0.926]`; host spread was `1.022`. The report is
`target/bench/interpreter-general/i006/target-ab/config-ab-2026-08-11T05-42-10-218Z.json`
(`0d3b29e029e663f8d2fdacf6d10c21095692da2d4a852a7186e349bab0cec9ec`).
No protection screen was run. No alternate width, boundary, or alignment shape
was tested after observation. Live `cpu.rs` and release Wasm were restored
byte-for-byte to I004 (`d8d1322f...` and `7e7cee94...`). The sealed populations
remain unexecuted, and the interpreter-parity goal remains open.

## I007: out-of-line complete M-extension arithmetic

Status: rejected and removed; exact I004 source/artifact restored

The hot integrated scalar decoder currently embeds every RV64 M-extension
multiply, high-multiply, divide, and remainder implementation in both the
64-bit OP and 32-bit OP-W dispatch bodies. Those operations include wide
integer lowering and all architectural divide edge cases even for ordinary
base-integer instructions.

I007 moves the complete M-extension semantic class into exactly two
non-inlined authoritative helpers, one for OP and one for OP-W. The inline
decoder retains the architectural `funct7 == 0x01` classification and calls
the corresponding helper; all base-integer cases, results, traps, retirement,
and instruction fetch remain unchanged. This is an ISA-family code-layout
change. It adds no PC, instruction sequence, register, process, binary,
workload, frequency, or adaptive selector, and it does not make multiply or
divide execute fewer guest instructions.

The implementation is frozen as the whole M family, without separating
multiply from divide, choosing individual funct3 values, adding `cold`, or
changing inlining after observation. Exact I004 is the control. Correctness
must pass the full workspace, the six focused differential/JIT-disabled gates,
and the M-extension differential coverage. The three-pair target remains
String and Bitfield. Only after a target pass, the protection screen includes
ALU and Mixed in addition to Boot, Compile, and Python because those compute
rows could expose a helper-call cost. Any confidence evidence of target
regression removes I007 without a family subset or helper-boundary variant.
The sealed populations remain forbidden.

Candidate Wasm was
`f08b862b14babcedc2099b850b8a4202b5c9b5e44c34c25e704aaa94596c05de`;
candidate `cpu.rs` was
`d47ea42d6d05ba1db954b52bb9a59f5fd0972a880d8286f3a124982799e5c06d`.
The full workspace, six focused differential/JIT-disabled gates, and the
dedicated complete M-extension differential passed.

The valid three-pair target screen rejected the frozen helper boundary.
String measured `0.992x` with interval `[0.977,0.998]`, and Bitfield measured
`0.962x` with interval `[0.948,0.965]`; host spread was `1.037`. The report is
`target/bench/interpreter-general/i007/target-ab/config-ab-2026-08-11T05-52-34-809Z.json`
(`2dcbbbe39fac481719bd17b3fce75b98a9835a4642e35307ac60762c04111586`).
Both target rows contained confidence evidence of regression, so the frozen
ALU/Mixed/Boot/Compile/Python protection screen was not run. No M-family
subset, inlining hint, or helper-boundary variant was tested after observation.
Live `cpu.rs` and release Wasm were restored byte-for-byte to I004
(`d8d1322f...` and `7e7cee94...`). The sealed populations remain unexecuted,
and the interpreter-parity goal remains open.

## I008: exact-revalidated decoded scalar blocks

Status: rejected and removed; exact I004 source/artifact/build config restored

The remaining String gap is too large for another instruction-family layout
split. I008 introduces a bounded decoded-block interpreter for the same
ordinary scalar semantic class already handled by integrated Tier 0. Every
direct executable-RAM entry is eligible under one uniform rule. RV64I/M and
the supported integer/scalar-memory RV64C forms are normalized once into
decoded fields, then executed by the same architectural category dispatcher
without re-extracting opcode, register, and immediate fields on each trip.

The cache shape is fixed at 64 direct-mapped entry-PC slots and at most 32
instructions per slot; 32 is the existing architectural interrupt-poll
cadence, not a measured workload length. There is no heat threshold, PC list,
opcode-sequence recognizer, adaptive size, replacement training, process,
binary, or benchmark identity. Blocks end after every store, direct or
conditional control transfer, stop, illegal/unsupported family, page end, or
the fixed bound. Thus a guest store can never be followed by a cached
instruction without a new validation. Non-direct fetches and a 32-bit
instruction straddling `0xffe` use the exact I004 fetch/decoder path.

Every activation first establishes the existing permission- and
context-proved direct executable-page capability, requires the current mapping
generation, and compares every cached low/high halfword with current RAM before
execution. The cache stores no host pointer. Interrupt checks, traps,
complete-decoder fallbacks, nonsequential control, and slice boundaries discard
the active cursor. These rules preserve self-modifying code observation while
amortizing field decode and per-instruction fetch dispatch only within an
exactly validated straight-line stretch.

The implementation is frozen without capacity, block-length, hashing,
validation-width, boundary, or decoded-field variants after timing. Exact I004
is the control. Correctness must pass the full workspace, exhaustive scalar and
compressed differentials, self-modifying and `0xffe` fetch tests, the six
focused JIT-off/system gates, and JIT inactivity proof. Three target pairs use
String and Bitfield. If they pass, every other development row is a protection
row before any final candidate decision. Failure removes I008 without a cache
shape variant. The stock-musl and sealed holdout populations remain forbidden
during selection.

The first release build exposed a pre-execution Wasm allocation failure: the
fixed cache enlarged by-value `Cpu` construction beyond wasm-ld's default
stack reservation. Before any timing, the build configuration was frozen with
a two-MiB stack reservation; the 64-by-32 cache shape and all execution rules
were unchanged. With that storage fix, the full workspace, six focused
differential/JIT-disabled gates, dedicated M differential, explicit `0xffe`
case, all 49,152 compressed encodings, and same-slice self-modifying-store test
passed.

The timed candidate Wasm was
`b13f604207a15d419a3f0f899519c5814ce4ca52d98fa6735dd1c5e9bf69456d`;
candidate `cpu.rs` was
`e7abc9e16e204b415346bf9c39b7877f14334ae16d9170c0a0b2ee666eac4896`;
the frozen candidate `.cargo/config.toml` was
`9ddcb7fcd544d2e75d6a9ec8d3ca9332a042bbf8398e514040784bece9d1a7c8`.

The valid three-pair target screen rejected the mechanism decisively. String
measured `0.695x` with interval `[0.694,0.696]`, and Bitfield measured `0.680x`
with interval `[0.658,0.681]`; host spread was `1.007`. The report is
`target/bench/interpreter-general/i008/target-ab/config-ab-2026-08-11T06-25-20-938Z.json`
(`36169f22972e687c5f51fec20201e57fa417bf614c341c42057a34954eda8422`).
No protection screen was run. No capacity, block length, validation width,
storage, hashing, or boundary variant was tested after observation. Live
`cpu.rs`, release Wasm, and `.cargo/config.toml` were restored byte-for-byte to
I004/default (`d8d1322f...`, `7e7cee94...`, and `252a344d...`). The sealed
populations remained unexecuted at this I008 checkpoint, and the
interpreter-parity goal remained open.

## Comparator audit after I008

The post-I008 source audit of pinned copy/v86 is recorded in
[INTERPRETER_COMPARATOR_AUDIT.md](INTERPRETER_COMPARATOR_AUDIT.md). Its JIT-off
dispatcher uses generic page-local physical fetch and complete x86 opcode
dispatch. Its page-bounded REP string path is an implementation of one
architecturally defined x86 instruction family and contains no nbench, PC, or
multi-instruction guest-loop recognizer.

The clean development String trial reports 796,661,853 RV64 instructions and
96,485,320 x86 instructions modulo 2^32, an approximately 8.26x work-count
difference. That diagnostic does not make cross-ISA instruction counts
equivalent and does not weaken the wall-time goal. It does rule out restoring
the removed RV64 scalar-loop recognizers as a purported counterpart to REP.
The stock-musl and sealed holdout populations had not been executed at this
comparator-audit checkpoint.

## I009: multi-row residual attribution

Status: diagnostic complete; no candidate admitted

The one-shot diagnostic protocol is
[INTERPRETER_MULTIROW_ATTRIBUTION_PROTOCOL.md](INTERPRETER_MULTIROW_ATTRIBUTION_PROTOCOL.md).
It profiles exact I004 in interpreter mode across String, Bitfield, Boot,
Compile, Python, and Mixed. Profiled durations are excluded. At most one
distinct operation common across several rows can admit a prospectively frozen
candidate; guest PCs, opcode sequences, guest symbols, frequency-selected
families, sealed stock-musl inputs, and sealed holdouts remain unavailable.

All sixteen frozen phase profiles completed with exact I004 and zero JIT
activity. The report is intentionally measurement-invalid because profiling
makes each worker proof-only; its six listed problems are exactly those
exclusions. V8 assigns 92.57%-97.61% of String, Bitfield, Boot, Compile, and
Python directly to the one fully inlined integrated interpreter function;
Mixed assigns 66.38% there and the remainder primarily to complete FP fallback.
No independently named removable operation clears the cross-row rule.

The report is
`target/bench/interpreter-general/i009/report/interpreter-scorecard-v2-2026-08-11T06-43-10-016Z.json`
(`afb4c1df7b...`), and the deterministic closure report is
`target/bench/interpreter-general/i009/closure-analysis-preliminary.json`
(`8bb2a91ddf...`). I009 closes without a product edit. The exact I004 source,
Wasm, and build configuration remain live; sealed inputs remain unexecuted.

## I010: native multi-row interpreter attribution

Status: diagnostic complete; no candidate admitted

I009 cannot resolve work inside the optimized integrated interpreter body.
[INTERPRETER_NATIVE_MULTIROW_PROTOCOL.md](INTERPRETER_NATIVE_MULTIROW_PROTOCOL.md)
therefore freezes one higher-resolution native collection on String, Compile,
and Python. Only a non-semantic fetch/decode/register/memory/driver operation
owning at least 5% of the integrated body in all three rows may admit one
prospective candidate. Opcode cases, guest PCs, guest symbols, exact sequences,
stock-musl, and sealed holdouts cannot select it. Profiler-perturbed elapsed
times receive no credit.

All three collections completed with exact I004 and zero guest-JIT activity.
The integrated body owns 93.88% of String, 93.30% of Compile, and 96.30% of
Python main-thread period. Two 64-byte bands clear the common 5% exposure rule.
The larger band (8.65%, 9.68%, and 8.58%) is the retained executable-page
capability hit, exact halfword fetch, and length classification. The other
(5.25%, 5.78%, and 5.56%) contains loop/interrupt-countdown state and the start
of fetch-tag formation. Those are precisely the already retained or closed
I001/I004/I005/I006/I008 fetch, cache, and driver mechanisms; neither band
contains a distinct removable operation. The next two bands miss the frozen
threshold in Compile and are ineligible.

The deterministic analysis is
`target/bench/interpreter-general/i010/native-analysis.json`
(`ace8d069375f...`); collection identities and full hashes are recorded in
[INTERPRETER_NATIVE_MULTIROW_PROTOCOL.md](INTERPRETER_NATIVE_MULTIROW_PROTOCOL.md).
All profiled elapsed values are excluded. I010 closes without a product edit;
exact I004 remains live, and no sealed input was executed.

## Frozen I004 authoritative development scorecard

Status: measurement valid; String gate failed; goal still open

With I010 closed and no further source edit admitted, exact I004 ran the full
13-row development scorecard at `AUTHORITATIVE=1`, three alternating
fresh-process repetitions per side, and interpreter execution mode. All 78
trials are eligible and prove zero JIT activity. There are no report problems;
maximum host-probe spread is `1.014548` and maximum within-side sample spread
is `1.096179`.

I004 records eleven wins, one match, and one loss. Mixed matches at `1.0301x`;
Bitfield advances from the clean baseline loss to a valid `1.1229x` win.
String remains `0.3186x`, or `3.14x` behind copy/v86, and fails the frozen
`0.95x` floor. The report is
`target/bench/interpreter-general/frozen-i004-authoritative-dev/interpreter-scorecard-v2-2026-08-11T08-09-12-664Z.json`
(`3639b051bd30...`). Full identities and the decision are recorded in
[INTERPRETER_I004_AUTHORITATIVE_DEVELOPMENT_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_DEVELOPMENT_RESULT.md).

I004 remains the live architecture-general intermediate, not a parity
candidate. Because it fails the complete development gate, `stock-musl-v1`
and the unseen holdouts remain unexecuted rather than being consumed as tuning
inputs for a successor.

## String plateau and final unseal decision

Status: development tuning closed at exact I004; strict correctness passed

The authoritative String medians correspond to 796,532,587 RV64 instructions
in 5,496.369 ms and 96,490,537 x86 instructions modulo `2^32` in 1,751.212 ms.
I004 is already 2.630x faster per reported instruction, but executes 8.255x
the reported work. Matching wall time requires another 3.139x complete-row
speedup or removal of 68.139% of I004 time.

The complete current/historical mechanism ledger finds no open
architecture-general operation with that leverage. In particular, I008's
decoded executor regressed both target rows, and changing its validation,
store boundary, or invalidation after timing is the prohibited cache-variant
rescue. Restoring benchmark loop recognizers is ineligible.

The decision and exact final sequence are frozen in
[INTERPRETER_STRING_PLATEAU_AND_UNSEAL_PROTOCOL.md](INTERPRETER_STRING_PLATEAU_AND_UNSEAL_PROTOCOL.md).
After the strict correctness matrix passes, `stock-musl-v1` and the holdouts
will be executed once to complete the requested comparison. Their results
cannot select another product edit.

The complete pre-unseal correctness matrix passed: formatting, 109 Rust
library tests, scorecard self-test, exact JIT bypass, 60-program full-state,
user memory/M/A/FP, fused system memory, Sv39/MPRV/A-D, system A/FP, precise
memory exits, and WFI gates. Exact I004 identities remain unchanged.

## Frozen I004 authoritative stock-musl scorecard

Status: measurement valid; fair-input String gate failed; goal still open

The first and only sealed `stock-musl-v1` run completed after tuning was
closed. Exact I004 ran all 13 rows at `AUTHORITATIVE=1`, three fresh-process
repetitions per side, with neither a runtime override nor any diagnostic. All
78 trials are eligible and prove inactive JIT; the problem list is empty,
maximum host-probe spread is `1.013977`, and maximum scored within-side sample
spread is `1.160265`.

I004 wins eleven rows, matches Mixed at `1.0179x`, and loses String at
`0.3076x`, or `3.25x` behind copy/v86. The stock binaries use the same portable
implementation contract on both architectures and exclude the development
RV64-only `fastmem.c`. The manifest passed before and after execution. No
sample was replaced or rerun, and the result admits no source edit.

The raw report is
`target/bench/interpreter-final/stock-musl-v1-i004/interpreter-scorecard-v2-2026-08-11T09-16-08-726Z.json`
(`bb4b277196d0...`). Full identities and the decision are recorded in
[INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md).
At this checkpoint the one-time sealed holdout was next under the already
frozen sequence.

## Frozen I004 authoritative holdout

Status: measurement valid; transfer gate passed; overall goal still open

The first and only sealed holdout execution completed after the failed stock
result was retained. Exact I004 ran all four rows at `AUTHORITATIVE=1`, three
fresh-process repetitions per side, without a runtime override, diagnostic, or
sample replacement. All 24 trials are eligible, prove inactive JIT, and match
every sealed phase checksum. The problem list is empty, host-probe spread is
`1.023339`, and maximum scored within-side sample spread is `1.072049`.

I004 wins BusyBox gzip at `1.8999x`, BusyBox sort at `1.8240x`, BusyBox
SHA-256 at `1.7171x`, and OpenSSL AES-256-CTR at `2.5409x`. This clean transfer
does not rescue the failed development and stock String gates; the overall
goal remains not met, and the opened holdout admits no source edit.

The raw report is
`target/bench/interpreter-final/holdout-v1-i004/interpreter-holdout-2026-08-11T09-43-27.120Z.json`
(`dc71535f627e...`). Full identities and the decision are recorded in
[INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md).
The frozen final audit sequence is complete with exact I004 unchanged.

## G001: FENCE.I-coherent decoded interpreter

Status: rejected before production integration; exact I004 remains live

After the opened I004 populations were permanently excluded from candidate
selection, G001 froze a new architecture-defined decoded representation under
[INTERPRETER_G001_FENCEI_DECODE_PROTOCOL.md](INTERPRETER_G001_FENCEI_DECODE_PROTOCOL.md).
It replaces eager instruction-byte revalidation with a hart-local
instruction-fetch generation advanced by architectural `FENCE.I`. The design
has one uniform 64-by-32 direct-mapped cache, a complete ordinary scalar class,
no workload identity or opcode subset, and an independently sealed 19-program
Embench transfer population. Neither Embench guest was executed.

Before any production edit, the frozen architecture-balanced standalone model
passed deterministic-module, complete-state, all 62 32-bit operation, all 19
compressed-family, every-single-operation, 129-entry wrap, page-straddle,
complete-key, and old-before/new-after-`FENCE.I` checks. Its one permitted
seven-pair timing was stable and valid, but delivered only `1.662022x` with
fixed-seed 95% interval `[1.644338x, 1.680755x]`. That is far below the frozen
`3.75x` median and `3.50x` lower-bound leverage requirements. The report is
`target/bench/interpreter-g001-opportunity-v1/gate.json`
(`1e2d6daa1706...`).

G001 therefore closes without touching production source and without a cache,
storage, hash, boundary, or warmup successor. Exact I004 `cpu.rs` remains
`d8d1322f...`; the pure-interpreter parity goal remains open.

## G002: complete local-GPR interpreter representation

Status: rejected before production integration; exact I004 remains live

The post-G001 source audit found one distinct non-cache representation worth
an upper-bound test: retain all writable architectural integer registers in
Wasm locals across a direct-interpreter slice. G002 froze the complete
architecture-defined set `x1`--`x31`, with `x0` fixed at zero, and one complete
32-way `br_table` selector for every dynamic `rs1`, `rs2`, and `rd`. It uses no
register subset, opcode, trace, guest PC, decoded block, or workload input.

[INTERPRETER_G002_LOCAL_GPR_PROTOCOL.md](INTERPRETER_G002_LOCAL_GPR_PROTOCOL.md)
defines an intentionally favorable standalone model: its 1,024-record
mathematical schedule gives every register exactly 32 appearances in every
role and every `rd` a complete `rs1` permutation. It performs only two state
reads, one small ALU kernel, and one state write per modeled instruction,
omitting fetch, decode, guest data memory, interrupts, and every other common
interpreter cost. The control uses direct linear-memory state; the treatment
uses 31 locals and exactly three complete selectors.

Untimed preparation passed deterministic bytes and shape, zero work, every
single record, long-run, windowed, complete-state, and `x0` equality. The first
and only seven-pair timing then passed every identity, correctness, affinity,
stability, host, and construction check but measured only `0.098116x` paired
throughput with interval `[0.097818x, 0.098492x]`. Treatment was about `10.19x`
slower because static Wasm local indices require costly dynamic selectors;
direct memory needs only two loads and one guarded store.

The report is
`target/bench/interpreter-g002-opportunity-v1/gate.json`
(`a6924fb9b123...`). It fails only the frozen `3.75x`/`3.50x` leverage checks
and closes before production, transfer guests, or known benchmarks. No local
count, selector shape, compact bank, hybrid, SIMD lane, or predecoded-register
variant follows. Exact I004 remains unchanged and the goal remains open.

## G003: standard LLVM source optimization levels

Status: finite standalone screen rejected; exact I004 remains live

The compiler audit found that R060 had closed standard Binaryen post-link
levels and R061 had closed Wasm SIMD, but the ordinary LLVM source levels had
not received an architecture-balanced comparison. G003 prospectively froze
`O1`, `O2`, `Os`, and `Oz` against current `O3`, with no custom pass, flag,
target feature, guest input, or profile. It reused only G001's already-frozen
balanced direct-interpreter source and executed `run_control`; no decoded-cache
path ran.

Untimed preparation built every level twice, required exact bytes and complete
state across every operation and long streams, and found only two executable
artifacts. O1/O2/O3 are byte-identical; Os/Oz are byte-identical to each other
and 1,757 bytes smaller. A v1 checker omitted three existing diagnostic exports
and failed only its manifest; v2 corrected those names before timing and is the
authoritative preparation.

The complete frozen 56-process screen retained every sample. O1 measured
`0.999569x [0.997283, 1.044406]`, O2 `1.000312x [0.969077, 1.006095]`, Os
`0.970757x [0.961987, 0.978101]`, and Oz
`0.967120x [0.910545, 0.996385]`. One O2 process failed spread, but O2's exact
byte identity with control independently rules out a causal change; it was not
rerun. The valid Os screen establishes that the sole smaller artifact is
slower.

No level approaches the frozen `3.25x`/`3.00x` full-gap gates. The report is
`target/bench/interpreter-g003-opportunity-v1/gate.json`
(`26580d5c1ad2...`). No production build or guest execution follows. Standard
LLVM optimization-level choice closes, exact I004 remains live, and the goal
remains open.

## Post-G003 historical static-Tier-0 reconciliation

Status: audited; no new candidate, no product change, goal blocked under fixed constraints

The final mechanism ledger was checked against the earlier R070--R115
hand-emitted scalar Tier-0 line. This was the strongest prior
architecture-general alternative to the LLVM-shaped Rust interpreter and must
not be omitted merely because it predates the pure-interpreter campaign.

R070's guest-independent executor covered the complete ordinary RV64I/M,
integer-RVC, and scalar-memory surface. Its isolated balanced corpora measured
`1.624x`--`2.445x`, while full-system Boot measured only
`1.047x [1.001, 1.061]`; the independent R071 confirmation was
`1.024x [1.000, 1.050]`. R072--R077 sampled-entry successors reached stronger
native or browser boundaries, but failed frozen browser/product promotion;
R078 then proved that leaving the rejected machinery dormant materially
regressed Boot and admitted its removal. R082 stopped at its dormant-artifact
confidence gate before active execution timing.

R095's prebuilt external executor passed exhaustive semantics but measured
Boot `0.944x [0.933, 0.949]`. R115 later isolated the packaging question:
same-instance ownership recovered `1.03413x [1.01796, 1.04451]` relative to
the external instance, but the exact embedded executor enabled versus disabled
was still `0.97987x [0.95574, 0.99572]` on Boot. Thus the instance tax was
real and the executor still was not a product improvement.

These historical full-system experiments retained the ordinary page/region
JIT. Their timings are not pooled with I004 and are not called JIT-off
scorecard evidence. They are retained only as architecture-general transfer
evidence: even the most favorable isolated hand-emitted shape stayed below the
fair String deficit, and its integrated forms did not provide a safe product
gain. Exact I004 remains unchanged, the JIT-enabled scorecard remains paused,
and the pure-interpreter goal is blocked under the fixed comparison
constraints. It is not complete: development and stock String remain at
`0.3186x` and `0.3076x`. A new run requires a genuinely new independently
qualified primitive or an explicit constraint change; known-loop fusion is
ineligible.
