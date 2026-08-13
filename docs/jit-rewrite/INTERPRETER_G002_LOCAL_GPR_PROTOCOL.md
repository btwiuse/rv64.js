# G002 Complete Local-GPR Interpreter Protocol

Date opened: 2026-08-11 (America/Phoenix)  
Status: rejected at the frozen opportunity gate; no production edit

## Objective

Test the one materially distinct pure-interpreter representation left by the
post-G001 source audit: retain the complete RV64 integer register file in Wasm
locals for an interpreter slice instead of loading and storing `Cpu::x` in
linear memory for every guest instruction.

This mechanism is architecture-wide and workload-blind. It retains every
writable integer register `x1` through `x31`; reads of `x0` return zero and
writes to `x0` are discarded. It does not select a compact-register bank,
register popularity, opcode, guest PC, binary, symbol, process, or benchmark.
It does not cache, fuse, recognize, translate, or generate guest instructions.

The obstacle is architectural rather than workload-specific: a Wasm local has
a static index, while each decoded RV64 `rd`, `rs1`, and `rs2` is dynamic. The
sole G002 representation therefore uses one complete 32-way `br_table`
selector for each dynamic read or write. No tree, hybrid local/memory bank,
SIMD lane form, predecoded register combination, or register-count variant may
follow the result.

## Relationship to existing closures

G002 is not a decoded-cache successor to I008 or G001. Both sides consume the
same already-normalized dynamic register-access records; there is no code
fetch, decoding, block cache, invalidation, or `FENCE.I` behavior in the
model. G001's failed result cannot choose a G002 parameter and the untouched
G001 Embench guests remain forbidden.

R103, R116, R117, and R124 concern architectural-state representation inside
generated guest modules. G002 instead asks whether locals can replace the
direct interpreter's dynamic linear-memory register file. R117's global/local
result and R124's fixed compact bank are historical context only and supply no
G002 performance credit.

## Frozen control and production boundary

The live production identities remain:

- `crates/rv64-core/src/cpu.rs`:
  `d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`;
- `.cargo/config.toml`:
  `252a344de3e565c134906a497e33f88795eae1a29f1357bbfb05ffea911bc267`;
- release Wasm:
  `7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`;
  and
- loader:
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.

G002 begins as a standalone model only. Failure changes no production source
and authorizes no scorecard, stock-musl, holdout, or Embench execution.

## Frozen favorable opportunity model

One deterministic Wasm module exports `run_control` and `run_treatment`. Both
functions consume the same immutable 1,024-record schedule from module linear
memory, start from the same 32-word register image, execute the same loop and
integer kernel, commit the same final register image, and return the same
checksum.

For record index `i`, let `lo = i & 31` and `hi = i >> 5`. The record is fixed
as:

```text
rd    = lo
rs1   = (hi + 5*lo + 1) mod 32
rs2   = (7*hi + 13*lo + 3) mod 32
shift = ((17*hi + 11*lo) mod 63) + 1
salt  = (0x5a3*hi + 0x31d*lo + 0x155) mod 2048
```

The fields are packed into one little-endian `u32`. Every register number
appears exactly 32 times in each of the `rd`, `rs1`, and `rs2` roles. For each
`rd`, `rs1` traverses all 32 register numbers exactly once. The schedule is
mathematical and fixed here; no guest trace or opcode frequency informs it.

Each modeled instruction performs exactly:

1. one packed-record load;
2. two dynamic GPR reads;
3. `rotl64((a + zero_extend(packed)) xor b, shift)`;
4. one dynamic GPR write, discarded only for `rd == 0`; and
5. one identical checksum update.

The control performs dynamic GPR accesses in canonical linear memory. The
treatment loads all `x1`--`x31` once on entry, uses exactly one complete
32-way `br_table` for each of the two reads and the write, and commits all 31
locals once on exit. Both compute their final checksum from the committed
linear-memory image.

This deliberately omits instruction fetch, decode, semantic dispatch, guest
data memory, interrupts, and other common interpreter work. It therefore gives
local GPR residency an unrealistically favorable complete-runtime share. A
failure here is stronger than a failure in a full interpreter; a pass is only
permission to consider production feasibility, not performance credit.

No record count, access ratio, selector spelling, local count, warmup, work,
or threshold may change after the first timing result.

## Static and correctness preparation

Before timing, generate the module twice and require byte identity and Wasm
validation. Deterministic inspection must prove:

- exactly 1,024 records and exact role balance described above;
- one module with the two named execution functions and one memory;
- treatment declares exactly 31 more `i64` state locals than control;
- control performs exactly two dynamic state loads and at most one state store
  per loop iteration, with `x0` writes discarded;
- treatment performs no state-memory access inside its record loop and has
  exactly three complete 32-target selectors there;
- treatment performs exactly 31 entry loads and 31 exit stores;
- neither function imports or calls another function; and
- normalized operator streams differ only in the frozen state-access
  representation and the treatment entry/exit transfer.

Fresh instances must match complete committed register state and return value
for zero work, every one of the 1,024 records independently, one full round,
257 full rounds, and nonzero schedule windows that cross the record-array
midpoint. Any mismatch stops before timing.

Preparation may repair a correctness or inspection bug without observing
performance. Once the preparation artifact identities and timing harness are
recorded below, there is no second timed artifact.

### Frozen preparation result and identities

The final preparation ran without a performance clock and passed every frozen
check. It generated the module twice with byte-identical Wasm, records,
schedule, and shape reports. The module validates and exports exactly the two
drivers plus reset/inspection functions. All 1,024 records match the formula;
each of the three register roles has 32 occurrences of every register number,
and `rs1` is a complete permutation for each `rd`.

The control driver declares six `i32` and four `i64` locals, one record load,
34 static `i64` loads (two dynamic state reads plus 32 final checksum loads),
one guarded dynamic state store, no `br_table`, and no call. The treatment
declares six `i32` and 35 `i64` locals, one record load, 63 static `i64` loads
(31 entry plus 32 final checksum loads), 31 exit stores, exactly three
33-entry tables (32 register targets plus the frozen duplicate-`x0` default),
and no call. Thus the treatment has exactly 31 additional state locals and no
state-memory operator in its record loop.

Zero work, every single record, one and 257 complete rounds, and both frozen
cross-midpoint windows produce exact return and complete committed-register
state equality with `x0 == 0`.

The immutable preparation is
`target/bench/interpreter-g002-model-v1`:

- model generator source:
  `0d477252985d1a5681f2fa358d535c71a758efd9b6d27be43be70b28681f7f13`;
- preparation harness:
  `b0f9bc11d708716770faee0666113b04f3687b73b4558050b40097e4a9891372`;
- deterministic 7,416-byte Wasm:
  `d3dd92bad1792340d8bf618a4c30595af87f49b76ab81de9343f32c59e764a56`;
- packed records:
  `480732a812a3271e5059217cffc949dcbaaef97f3d63e8391bb64210d30be1a5`;
- disclosed schedule:
  `d3e7f193990b28097a93c9623946ab1a1b591a7aae3ad711172461144bd424a3`;
- shape report:
  `52a6c64bf1e06963cbfc28a227a9f7f925d081c78a1e61a699e332a4a832e069`;
- normalized record schedule:
  `145986294e0a7eee2d6d0b43874d9a46d1c6bfe9a1791a59880df0dee24f1b28`;
- untimed freeze report:
  `4d4ff1105e69944c9fd4c37b598bf308d92f1ecea2fb50fe1b73c72b858d1ab5`;
  and
- frozen timing harness:
  `eefee53c710f15f212cc889e397ba04efc141d9c830560fc38cd79ea1a5808af`.

An earlier `interpreter-g002-model-dev-a` directory was a generator compile
check before the final preparation harness ran. It emitted the same model,
records, schedule, and shape bytes and was never performance-timed; it is not
an alternative artifact or eligible result.

## Frozen opportunity timing

Use seven alternating fresh-process pairs on CPUs 8 through 14, one immutable
module per process, ordinary Node/V8 tiering, and no engine flags. Pair zero
runs control then treatment, pair one reverses the order, and so on.

Each process performs:

- one untimed prewarm call of one complete round;
- three warm calls of 4,096 complete rounds; and
- seven steady calls of 16,384 complete rounds.

Every steady call represents exactly 16,777,216 modeled instructions. Reset
the complete register image before each call, retain every issued sample, and
yield one event-loop turn between calls. Report compile, instantiate, first,
warm, steady, affinity, state/output fingerprints, host probes, and all raw
durations.

G002 passes only if all of these prospectively frozen conditions hold:

- deterministic module, schedule, work, output, and complete-state checks;
- host, side, and every within-process spread at most `1.25x`;
- paired treatment/control steady throughput at least `3.75x`;
- fixed-seed bootstrap 95% lower bound strictly above `3.50x`; and
- compile plus instantiate time below 25 ms in every treatment process.

The known String row requires about `3.14x` complete-row speedup while common
interpreter work omitted by this favorable model would dilute any state-only
gain. The `3.75x`/`3.50x` gates therefore test whether this representation has
credible leverage for the actual goal rather than whether it can produce a
small microbenchmark improvement.

## Conditional product path

Only a passing opportunity result may admit one production feasibility pass.
Before any product edit, freeze a new independent cross-architecture transfer
population; the development scorecard, stock-musl population, opened holdout,
and G001 Embench population may not select or tune the implementation.

The sole production shape would keep all 31 GPRs local across the complete
integrated scalar slice and use the same complete selectors everywhere. It may
not add decoded blocks, register-pair handlers, opcode/register combinations,
a compact-bank fallback, or workload-derived choices. It must first pass the
complete scalar/M/A/FP, compressed, memory, Sv39/MPRV, interrupt, fault,
budget, direct/OpenSBI Linux, JIT-bypass, and cross-engine correctness matrix.

Only after a new transfer gate passes may the known scorecards be used once as
final acceptance tests. Overall completion still requires every development
and stock row at or above `0.95x` versus pinned copy/v86. A failed standalone,
correctness, transfer, or final gate removes G002 without a representation
variant. The parity goal remains open unless those final conditions actually
pass.

## Frozen opportunity result

The first and only seven-pair timing completed all 14 fresh processes in the
frozen order. Every identity, schedule, output, complete-state, `x0`, affinity,
within-process, side-spread, host-spread, and compile-cost check passed. Host
spread was `1.059965x`, control/treatment side spreads were
`1.025655x`/`1.014773x`, and the largest treatment compile-plus-instantiate
time was `0.201943 ms`.

For 16,777,216 modeled instructions per steady call, control and treatment
side medians were `26.085727 ms` and `265.656926 ms`. Complete local residency
therefore produced paired throughput `0.098116x`, with fixed-seed 95% median
interval `[0.097818x, 0.098492x]`: it was about `10.19x` slower than direct
linear-memory GPR access. Only the preregistered `3.75x` paired-median and
`3.50x` lower-bound checks failed.

The result is mechanistic. Every dynamic local access pays a complete selector
because Wasm local indices are static; three selectors cost far more than the
two direct loads and one guarded store they replace. The model omits all other
interpreter work and thus already gives local residency more leverage than a
real implementation could receive.

The immutable report is
`target/bench/interpreter-g002-opportunity-v1/gate.json`, SHA-256
`a6924fb9b123bb4ee41412dbaff065db4939d82d95b565362c9da15de9313039`.
Its decision is `close-g002-before-production-edit-without-successor`.
Consequently no local-count, selector, compact-bank, hybrid-memory, SIMD,
predecoded-register, or register-combination variant is admissible. No
production file or guest population was touched. Exact I004 remains live with
`cpu.rs`
`d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`,
and the overall parity goal remains open.
