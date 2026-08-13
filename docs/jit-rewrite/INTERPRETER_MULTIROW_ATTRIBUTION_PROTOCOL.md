# I009 Multi-Row Pure-Interpreter Attribution Protocol

Date frozen: 2026-08-10 (America/Phoenix)  
Status: diagnostic complete; no performance credit and no candidate admitted

## Question

After retaining I004 and rejecting I005-I008, does one distinct,
architecture-general host operation still account for material cost across a
diverse set of JIT-off development workloads?

This diagnostic exists to prevent another source-level guess from being timed
against one favorable benchmark. It may admit at most one subsequent I009
candidate. The diagnostic itself changes no product source and earns no
performance credit.

## Immutable product and inputs

- live I004 `cpu.rs`:
  `d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`;
- live I004 release Wasm:
  `7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`;
- loader:
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- development population: `scorecard-v2-modern` only; and
- execution mode: interpreter, with the existing negative JIT-activity proof.

The frozen rows are String, Bitfield, Matched Boot, Compile, Python, and Mixed.
They deliberately include the remaining loss, the recently resolved loss,
startup/system work, compiler work, high-level-language work, and a simple
compute row. Exactly one fresh profile is collected for each scored phase at a
250-microsecond interval. FIRST/PRIME/STEADY phase elapsed values are
measurement-ineligible and must not be compared as benchmark results.

The `stock-musl-v1` population and all sealed holdouts remain forbidden.

## Attribution and admission rule

Reconstruct every sampled stack and report exclusive leaf self time by
operation family. Also report exact optimized Wasm function names so broad
categories are auditable. A family can admit a candidate only when all of the
following hold:

1. it is defined by ISA semantics or ordinary machine/runtime state, with no
   benchmark, process, ELF, symbol, guest-PC, exact instruction sequence, or
   observed opcode-frequency selector;
2. it owns at least 5% of String STEADY and at least 2% in two other frozen
   rows, or at least 5% in four frozen rows;
3. the removable subset and added work support a realistic local gain rather
   than treating the whole family as free;
4. it is distinct from the closed I005 interrupt-local, I006 fetch-width, I007
   M-helper, I008 decoded-cache, R083 full-system specialization, and earlier
   fetch/cache/driver-layout families; and
5. exactly one implementation shape, correctness matrix, target rows,
   protection rows, and rejection rule are recorded before source editing.

Opcode counts, guest PCs, symbols inside guest binaries, and exact sequences
may not be collected. If no family qualifies, I009 closes with no edit.

Any admitted candidate must still pass the anti-overfit protocol. This
diagnostic cannot authorize a scalar-loop accelerator, REP emulation for
RISC-V, benchmark-aware helper, or a variant of a previously rejected shape.

## Frozen collection command

```sh
ARTIFACTS=/home/darren/src/arm64.js/target/bench \
SIDES=rewrite \
ROWS=mixed,boot,python,compile,string,bitfield \
REPS=1 \
SCORECARD_V2_EXECUTION_MODE=interpreter \
SCORECARD_V2_REWRITE_WASM="$PWD/target/bench/interpreter-general/i004/artifacts/candidate-7e7cee94.wasm" \
SCORECARD_V2_ENGINE_PROFILE_DIR="$PWD/target/bench/interpreter-general/i009/profiles" \
SCORECARD_V2_ENGINE_PROFILE_INTERVAL=250 \
SCORECARD_V2_OUTPUT="$PWD/target/bench/interpreter-general/i009/report" \
node tests/vs-v86/scorecard-v2.mjs
```

The report must mark the profile run measurement-ineligible. Profile files,
the scorecard report, a deterministic analysis, and SHA-256 identities are
retained whether or not a candidate is admitted.

## Result

All sixteen requested phase profiles completed from exact I004. Every worker
proves interpreter mode and zero JIT activity. The scorecard-level report is
intentionally `measurementValid=false`: its only six problems are that a
proof-only profiler run entered the normal measurement list. This is the
expected exclusion mechanism, not a guest or profiling failure. Its JSON is
`target/bench/interpreter-general/i009/report/interpreter-scorecard-v2-2026-08-11T06-43-10-016Z.json`
(`afb4c1df7b8d5f3e60ee8446ecb6b722b655e9bd17bf3322bdcbdc914fedea6d`).

The existing deterministic closure analyzer produced
`target/bench/interpreter-general/i009/closure-analysis-preliminary.json`
(`8bb2a91ddf9423bd041239ffa6185cef75046704c1130f6a2fc86bc0a2cfe700`).
V8 attributes the following share of sampled phase time directly to the one
optimized `Cpu::run_integrated_scalar_t0<VirtBus, CHECK_STOP=false>` body:

| Phase | Integrated body |
| --- | ---: |
| String STEADY | 96.4954% |
| Bitfield STEADY | 95.6244% |
| Boot FIRST | 92.5672% |
| Compile STEADY | 97.1591% |
| Python STEADY | 97.6054% |
| Mixed STEADY | 66.3787% |

Mixed separately spends 20.9677% in the complete `Cpu::step` fallback and
10.6043% in `Cpu::op_fp`, consistent with its floating-point instruction mix.
In the other rows, no independently named operation inside the fully inlined
integrated body is visible to the engine profiler. External slow paths are all
well below the admission thresholds; for example String STEADY assigns about
0.208% to `ld_slow`, while interrupt polling plus `irq_lines` totals about
2.97% and is already closed by I005.

The only cross-row qualifying frame is therefore the complete integrated
interpreter itself. That is not a removable operation and its driver/layout,
fetch, memory, interrupt, and decoded-cache variants are already closed. I009
admits no implementation. No elapsed value from this profile run is used as
performance evidence, and no sealed input was executed.
