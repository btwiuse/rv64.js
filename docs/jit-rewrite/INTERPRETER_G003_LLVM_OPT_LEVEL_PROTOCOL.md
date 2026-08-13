# G003 Standard LLVM Optimization-Level Protocol

Date opened: 2026-08-11 (America/Phoenix)  
Status: finite screen complete; all standard levels rejected before production

## Objective

Test the remaining workload-blind build representation not covered by the
source mechanisms or Binaryen post-link campaign: can a standard LLVM source
optimization level produce a radically faster direct RV64 interpreter than
the current release `-O3` shape?

The exact candidates are the four other standard optimized Clang/Rust levels:
`-O1`, `-O2`, `-Os`, and `-Oz`. Control is `-O3`. There is no custom LLVM
argument, pass list, pass ordering, target CPU, target feature, profile-guided
input, function annotation, or workload-specific flag. No level may be added,
removed, or rebuilt differently after timing.

## Relationship to prior work

R060 tested Binaryen 125 `-O1` through `-O4` after Rust/LLVM had already linked
the production module. G003 changes LLVM's source optimization, inlining, and
code formation before Wasm linking; it is a distinct compiler boundary. R061
separately rejected `+simd128`. Those results provide no G003 performance
credit and no candidate choice.

G003 reuses the immutable architecture-balanced G001 model source solely as a
complete direct-interpreter corpus. Every timing leg calls `run_control`, which
fetches bytes, classifies instruction length, decodes fields, dispatches, and
executes semantics on every operation. No leg calls G001's decoded-cache
`run_treatment`, and no cache result or geometry selects a compiler level.

The source is already frozen independently of G003 at SHA-256
`27bfc111495af24e39a4f2c3e7233ac690a20e2456099dd2cab3a1e2453a0128`.
Its stream contains all 62 admitted 32-bit operation kinds once, all 19
compressed quadrant/funct3 families ten times, every compressed semantic form,
balanced taken/not-taken control, and balanced page bands. It contains no guest
trace, benchmark weight, guest PC, binary, symbol, or scorecard input.

## Production boundary

G003 constructs standalone Wasm only. The exact live production identities
remain:

- `crates/rv64-core/src/cpu.rs`:
  `d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`;
- `.cargo/config.toml`:
  `252a344de3e565c134906a497e33f88795eae1a29f1357bbfb05ffea911bc267`;
- release Wasm:
  `7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`;
  and
- loader:
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.

No guest population, known scorecard, G001 Embench image, or production source
is executed or changed unless a standard level first passes the standalone
leverage gate.

## Frozen construction and correctness

Use exact Zig 0.16.0 `cc -target wasm32-freestanding -nostdlib` with the same
`--no-entry`, `--export-memory`, and `--strip-all` linker flags as G001. Build
each of `-O1`, `-O2`, `-O3`, `-Os`, and `-Oz` twice. Require byte identity per
level, Wasm validation, the exact export surface, and an `-O3` artifact
byte-identical to the frozen G001 model.

For every level, compare `run_control` with the fresh `-O3` control in separate
instances after identical resets. Complete return and state must match for:

- zero work;
- every one of the 252 scalar operations independently;
- one, three, and 257 complete rounds;
- two traversals of the 129-entry wrap stream;
- the page-straddling 32-bit fallback; and
- both mutation encodings through the uncached control path.

Inspect every level's normalized record stream and require the exact G001 raw
and normalized identities. Record whole-module bytes, CODE-section bytes,
function count, imports, exports, locals, calls, branch tables, and memory
operators. Static size is diagnostic only.

A preparation error may be repaired before timing without changing the five
levels or model source. Once artifact, preparation, and timing-harness hashes
are recorded below, there is no second candidate build or timing run.

### Frozen preparation result and identities

The first preparation under `interpreter-g003-model-v1` produced deterministic
valid artifacts and passed every stream and state comparison, but its checker
omitted the model's three existing `model_error_{index,expected,actual}`
diagnostic exports from the expected manifest. The sole failed check was
therefore `exactExports`. That untimed report is retained at SHA-256
`b609a600f3de530547896da7b89e5c98e85ae71c938f1532d880b2ac571ccdb3`;
its preparation harness was `2937b8cdeb52...`. No model performance was
executed or observed.

The v2 preparation adds exactly those three pre-existing names to the manifest
and changes no source, level, compiler argument, module, state comparison, or
schedule. It passes every frozen check. Zero work, all 252 single operations,
one/three/257 rounds, the 129-entry wrap stream, page straddle, and both
mutation words match complete `-O3` state at every level.

Static construction finds two executable shapes:

- `O1`, `O2`, and `O3` are byte-identical 14,475-byte modules at
  `63f2fb590d20260c01d55186c53d8b38f9722f6798cdba6a40846de87f400026`;
  and
- `Os` and `Oz` are byte-identical 12,718-byte modules at
  `128d7db59f57ac2053689408a5e5fda7ae2b89b9d53ec1a2ebdb83469ad2f55e`.

The finite screen remains exactly as frozen: all four named candidates receive
their scheduled pairs even where byte identity makes the causal expectation
obvious. No level is removed after construction.

The immutable preparation is
`target/bench/interpreter-g003-model-v2`:

- preparation harness:
  `8181cb607411c0a5b52980616ec426b4663ffab1f481456d5ccd4840ed180c8e`;
- exact build-command transcript:
  `e46482a9b7113e248162b5dbe3f39d6fbc101de8cad41343ac786917e6f33500`;
- untimed freeze report:
  `0ecceda6a5f519577da7ea4f09744cf5f34756663c216b299dda4491db8ed04c`;
- raw stream:
  `987912d44c5d5b1f25ca26f57ed298ba9d21c1f8e8bef6ae7e535b87a2315c0f`;
- normalized stream:
  `52ed0a9f402bc8e66a038d852f1afd65336b031d9233d15844c38cf320f5284a`;
  and
- frozen timing harness:
  `074f99a2dc2d6543587ff6fd79b8a2a889847c1018d2514a674411c2e34a3d6c`.

## Frozen finite timing screen

Each candidate receives seven alternating fresh-process pairs against a fresh
`-O3` process, for 56 total processes. CPUs cycle through 8--15. Candidate
level order is `O1,O2,Os,Oz` rotated left by the pair index; within each level,
even pairs run control then candidate and odd pairs reverse that order.

Each process instantiates only its immutable level artifact, calls
`run_control`, and performs:

- one untimed prewarm round;
- three warm calls of 4,096 rounds; and
- seven steady calls of 65,536 rounds.

Every steady call executes exactly 16,515,072 balanced scalar instructions.
Reset complete architectural state before every call, yield one event-loop
turn between calls, and retain every issued process and sample. No engine flag,
forced tier, early stop, sample replacement, or level-specific work is allowed.

For a level to pass:

- all deterministic identity, work, state, output, affinity, and schedule
  checks must pass;
- host, each level side, and every within-process spread must be at most
  `1.25x`;
- paired candidate/control throughput median must be at least `3.25x`;
- its fixed-seed bootstrap 95% lower bound must be strictly above `3.00x`; and
- every candidate compile-plus-instantiate duration must be below 25 ms.

The `3.25x` point floor matches the fair stock-musl String deficit. Unlike
G001 and G002, this model includes the whole balanced direct interpreter body,
so no additional partial-mechanism margin is added. A smaller compiler-only
gain cannot credibly achieve the requested end state and does not admit a
production scorecard experiment.

If more than one level passes, select exactly one by the highest paired lower
bound, breaking a tie within `0.02x` by the order `O2,O1,Os,Oz`. This rule is
frozen before timing. If none passes, standard LLVM optimization-level choice
is closed without a Rust production build, level retry, or compiler-flag
successor.

## Conditional production path

Only a passing standalone level may build one unchanged-source Cargo release
with `profile.release.opt-level` set to the corresponding standard Rust value.
Before any product timing, freeze a new independent transfer population and
pass deterministic artifact, import/export, strict architecture, interpreter
bypass, direct/OpenSBI Linux, and full-system correctness gates.

The new transfer suite must pass at `AUTHORITATIVE=1`, odd `REPS>=3`, with both
guest JITs inactive, exact work/output identity, no row below `0.95x` versus
pinned copy/v86, and no row below `0.95x` against exact I004. Only then may the
known development and stock-musl scorecards run once as final acceptance.
Completion still requires every row of both known populations at or above
`0.95x`; a local compiler win is not completion.

On any failure, remove the candidate artifact and restore exact I004. The
pure-interpreter parity goal remains open until the full condition is actually
proved.

## Frozen opportunity result

The first and only finite screen issued all 56 processes in the frozen order
and retained every sample. All global source, preparation, artifact, stream,
schedule, affinity, issue-order, worker-completion, and host checks pass. Host
spread is `1.044495x`. No guest or production artifact was executed.

The paired direct-interpreter results are:

| Level | Paired median | Fixed-seed 95% interval | Measurement status | Decision |
|---|---:|---:|---|---|
| `O1` | `0.999569x` | `[0.997283x, 1.044406x]` | valid | fails leverage |
| `O2` | `1.000312x` | `[0.969077x, 1.006095x]` | one retained process fails side/within spread | fails leverage and stability |
| `Os` | `0.970757x` | `[0.961987x, 0.978101x]` | valid | stable regression; fails leverage |
| `Oz` | `0.967120x` | `[0.910545x, 0.996385x]` | valid | fails leverage |

The O2 anomaly is not replaced or rerun. Its candidate is byte-for-byte the
O3 control, as are O1 and O3, so exact artifact identity already proves there
is no causal compiler candidate regardless of that process pause. Os and Oz
are likewise one identical smaller artifact; both complete screens favor O3,
and the valid Os interval establishes about a 2.9% regression.

No level comes remotely close to the frozen `3.25x` median or `3.00x` lower-
bound gate. The immutable report is
`target/bench/interpreter-g003-opportunity-v1/gate.json`, SHA-256
`26580d5c1ad24d7e9764b4f7fca8313e1cb17f2b93e1902219da6a0c53ffe517`.
Its decision is
`close-standard-llvm-opt-levels-before-production-without-successor`.

Consequently no Cargo production level is built, no compiler flag/pass/target
feature successor follows, and no transfer or known guest population runs.
Exact I004 source, configuration, release Wasm, and loader remain unchanged.
The pure-interpreter parity goal is still not achieved.
