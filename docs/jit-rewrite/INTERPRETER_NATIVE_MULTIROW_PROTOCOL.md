# I010 Native Multi-Row Interpreter Attribution Protocol

Date frozen: 2026-08-10 (America/Phoenix)  
Status: diagnostic complete; no product candidate admitted

## Why a second-resolution diagnostic is needed

I009's sixteen phase-isolated V8 profiles completed, but the optimizer inlines
ordinary fetch, decode, integer semantics, and fused memory into one
`Cpu::run_integrated_scalar_t0<VirtBus, CHECK_STOP=false>` function. The engine
profiler consequently assigns 92%-98% of most rows to one leaf and cannot
separate operations inside it. I009 correctly admits no source edit.

I010 asks a narrower diagnostic question: do native hardware samples inside
that exact optimized function expose one shared architecture-general operation
with material cost across String, Compile, and Python? It changes no product
source and gives no elapsed-time credit.

## Frozen inputs and collection

The runtime, loader, guest population, execution mode, and I004 identities are
exactly those frozen by I009. The only rows are String, Compile, and Python.
For each row, run one fresh exploratory rewrite-only scorecard process under
Linux `perf` at 1,999 `cycles:u` samples per second, pinned to CPUs 8-15, with
V8 `--perf-prof` JIT metadata enabled. Preserve raw and JIT-injected perf data,
JIT dumps/ELFs, the scorecard report, command, and hashes. JIT remains disabled
inside the emulator and the report must prove zero guest-code generation and
dispatch.

The profiler changes tiering and timing. Every elapsed value is excluded even
if the harness mechanically labels a worker measurement-eligible. A failed
setup before guest execution may be corrected once without replacing a
sample; an eligible collection may not be rerun based on its contents.

`stock-musl-v1` and all sealed holdouts remain forbidden.

## Frozen classification

For each row, locate the optimized native DSO for the exact integrated
interpreter and close all of its sampled period into source-auditable native
PC bands. Bands may describe only:

- slice entry, budget, retirement, and interrupt polling;
- executable-page capability and exact byte fetch;
- instruction-length and top-level ISA decode;
- architectural register read/write mechanics;
- fused scalar load/store mechanics;
- complete slow-family/fault transitions; and
- semantic opcode bodies as one aggregate residual.

Guest PCs, guest symbols, binaries, exact instruction sequences, and opcode
frequencies may not be collected. Individual opcode-case blocks may be listed
only to prove closure; they cannot admit an optimization or reorder dispatch.

## Admission rule

I010 may admit at most one subsequent candidate, and only if:

1. the same non-semantic operation owns at least 5% of integrated-body period
   in all three rows;
2. source and native disassembly identify a concrete removable subset with a
   realistic gain after replacement cost;
3. the mechanism is not a variant of I005-I008, R083, or another closed
   fetch/cache/driver/layout family; and
4. its sole implementation shape, complete correctness matrix, target and
   protection rows, and no-variant rejection rule are frozen before editing.

If no operation qualifies, I010 closes without an implementation. Native
samples are attribution evidence only and cannot establish a speedup.

## Completed result

All three frozen collections completed against exact I004 Wasm
`7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`.
The first String process rejected `--perf-prof-path` before loading the guest;
the one protocol-permitted setup correction replaced it with Node's supported
`--perf-prof` flag. That setup-error data is preserved separately and was not
used as a sample. No eligible collection was repeated.

The String, Compile, and Python collections contain 39,314, 26,076, and 92,831
hardware samples. Their scorecard reports are mechanically valid and prove
zero generated guest instructions and zero generated dispatches, but every
profiled elapsed time is excluded by this protocol. The integrated interpreter
accounts for 93.88%, 93.30%, and 96.30% of main-thread period respectively;
TurboFan owns more than 99.8% of the samples inside that body in every row.

Only two common 64-byte native bands clear the frozen 5% threshold in all
three rows:

- `turbofan:0x580`: 8.65% String, 9.68% Compile, and 8.58% Python;
- `turbofan:0x200`: 5.25% String, 5.78% Compile, and 5.56% Python.

The optimized symbol begins at ELF text address `0x80`. Consequently the
first band maps to ELF `0x600`-`0x63f`: the retained executable-page
capability hit, exact halfword load, and instruction-length classification in
`scalar_fetch16`/`scalar_t0_step`. I001 and I004 already retain the profitable
capability and miss-path shapes; I006 rejected a one-read fetch-width variant,
and I008 rejected decoded-block caching. The samples identify no distinct
removable subset outside those closed fetch/cache mechanisms.

The second band maps to ELF `0x280`-`0x2bf`: loop state, the per-instruction
interrupt countdown materialization, and the beginning of executable-page tag
formation. I005 already tested and rejected carrying that countdown in a
slice-local, while the adjacent fetch work belongs to the same closed family
as the first band. It likewise identifies no new removable operation.

The next two common bands miss the preregistered threshold in Compile
(`0x3900` at 4.99% and `0x140` at 4.95%) and cannot admit a candidate. Broad
instruction categories such as register moves, integer arithmetic, native
stack traffic, and branches are closure accounting, not concrete removable
operations. I010 therefore closes without an implementation or elapsed-time
claim. Product source remains exact I004, and neither stock-musl nor any sealed
holdout was executed.

The deterministic analysis is
`target/bench/interpreter-general/i010/native-analysis.json`
(`ace8d069375f128a424f2ead9b91a28e35d0f234cc994558511b511eac20897c`).
Its analyzer is
`tests/vs-v86/interpreter-native-multirow-analyze.mjs`
(`2ac90a7a919dd4c494f476b5c8ae2701c09f6db9a131026d0d777e62ba1a51e5`).
The scorecard reports and hashes are:

- String:
  `target/bench/interpreter-general/i010/string/report/interpreter-scorecard-v2-2026-08-11T06-51-25-719Z.json`
  (`a4e69e8c11f2d19011d48699d7a91c8183eaa3641472b8c80d5944b5c36030bd`);
- Compile:
  `target/bench/interpreter-general/i010/compile/report/interpreter-scorecard-v2-2026-08-11T06-52-07-822Z.json`
  (`1fb2d9a86f1d159836ea6f7ac8d74a9a081f8dae6ec00f6f0fdf675735fd2f0d`);
- Python:
  `target/bench/interpreter-general/i010/python/report/interpreter-scorecard-v2-2026-08-11T06-53-14-168Z.json`
  (`e65533da30a5889574ae35d64713deb3b920744bb7e8d1bc6fe8f979978b39f1`).
