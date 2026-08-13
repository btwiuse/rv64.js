# I004 Authoritative Development Scorecard Result

Date: 2026-08-11 (America/Phoenix)  
Status: measurement valid; development gate failed on String; goal open

## Frozen execution

The complete existing 13-row development population was run after I010 closed,
without another product edit. The command was:

```sh
taskset -c 8-15 env \
  ARTIFACTS=/home/darren/src/arm64.js/target/bench \
  SCORECARD_V2_EXECUTION_MODE=interpreter \
  AUTHORITATIVE=1 REPS=3 V86_EXECUTION_PREFLIGHT=0 \
  SCORECARD_V2_OUTPUT=/home/darren/src/jit/target/bench/interpreter-general/frozen-i004-authoritative-dev \
  node tests/vs-v86/scorecard-v2.mjs
```

There was no runtime-Wasm override, profiler, diagnostic selector, sample
extension, or replacement. The live release artifact was exact I004
`7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`.
The comparator was pinned copy/v86 commit
`2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`, Wasm
`4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1`.

The driver exited with status 1 because its valid report sets `goalMet=false`;
this is the expected scorecard failure exit, not an execution failure. The
known corrupt local Git index produced provenance warnings and leaves Git
status unknown, but artifact, loader, source-tree, guest, and workload hashes
are recorded independently in every trial.

## Result

The report is authoritative and measurement-valid with an empty `problems`
array. It records eleven wins, one match, and one loss against copy/v86:

| Row | I004 interpreter | copy/v86 interpreter | I004 / v86 |
| --- | ---: | ---: | ---: |
| ALU | 45,465.3 ms | 89,426.4 ms | `1.9669x` |
| Mixed | 21,234.3 ms | 21,873.0 ms | `1.0301x` match |
| Matched Boot | 1,610.3 ms | 2,918.8 ms | `1.8125x` |
| Python | 13,530.9 ms | 36,663.6 ms | `2.7096x` |
| Compile | 3,175.1 ms | 4,721.1 ms | `1.4869x` |
| Numeric Sort | 1,956.6 ms | 3,398.4 ms | `1.7369x` |
| String Sort | 5,496.4 ms | 1,751.2 ms | **`0.3186x` loss** |
| Bitfield | 2,528.1 ms | 2,838.7 ms | `1.1229x` |
| FP Emulation | 5,021.3 ms | 10,510.0 ms | `2.0931x` |
| Fourier | 4,101.1 ms | 5,551.7 ms | `1.3537x` |
| Assignment | 5,334.8 ms | 8,393.0 ms | `1.5733x` |
| IDEA | 3,684.2 ms | 8,025.6 ms | `2.1784x` |
| Huffman | 4,064.7 ms | 8,101.0 ms | `1.9930x` |

All 78 fresh-process trials are measurement-eligible. Every trial reports
`executionMode=interpreter`, the appropriate interpreter-only requested
policy, and `jitProof.inactiveProof=true`. There are no policy problems and no
diagnostic runs. Every row/side pair has exactly repetitions 1, 2, and 3. The
maximum per-trial host-probe spread is `1.014548`; the maximum scored
within-side sample spread is `1.096179`, both below the frozen `1.25` limits.

The raw report is
`target/bench/interpreter-general/frozen-i004-authoritative-dev/interpreter-scorecard-v2-2026-08-11T08-09-12-664Z.json`
(`3639b051bd30c41f9e9491961a89148afc5e5d6ab5cbe3dec04cad87788eebe8`).
Its generated Markdown is the adjacent `.md` file
(`2a7d6cdd5e6df944c8c3325573a31ab2f361b8caa178c4e09614263aad8b85d1`).

## Decision

I004 establishes an honest Bitfield win but fails the preregistered `0.95x`
development floor on String by a large margin. It remains a useful
architecture-general intermediate baseline; it is not an accepted parity
candidate and the goal is not met.

The development input also retains the previously disclosed RV64-only
`fastmem.c` asymmetry, so even a hypothetical all-row pass here would not be
sufficient final evidence. Because I004 already fails the first complete
development gate, neither frozen `stock-musl-v1` nor any unseen holdout was
executed. Consuming either population cannot rescue I004 and would turn it into
a tuning input while successor development remains open. The subsequent
source/closure audit in
[INTERPRETER_STRING_PLATEAU_AND_UNSEAL_PROTOCOL.md](INTERPRETER_STRING_PLATEAU_AND_UNSEAL_PROTOCOL.md)
freezes tuning closed before either population is opened and fixes their
one-time final-audit execution; this development result itself used neither.

After the run, production identities remain exact: `cpu.rs`
`d8d1322fbb6e48981028707b665d655fc546858595df40384be10245aeac64af`,
`.cargo/config.toml`
`252a344de3e565c134906a497e33f88795eae1a29f1357bbfb05ffea911bc267`,
and release Wasm `7e7cee94eb58...`.

The later, prospectively frozen fair-input execution is recorded separately in
[INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_STOCK_MUSL_RESULT.md).
