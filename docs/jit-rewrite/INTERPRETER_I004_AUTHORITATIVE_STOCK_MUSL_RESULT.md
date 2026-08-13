# I004 Authoritative Stock-musl Scorecard Result

Date: 2026-08-11 (America/Phoenix)  
Status: measurement valid; fair-input gate failed on String; goal open

## Frozen execution

Exact I004 ran the complete 13-row `stock-musl-v1` population only after
development-informed tuning was closed and the pre-unseal correctness matrix
passed. This was the first and only execution of the sealed stock population.
The command frozen in advance was:

```sh
taskset -c 8-15 env \
  ARTIFACTS=/home/darren/src/arm64.js/target/bench \
  SCORECARD_V2_EXECUTION_MODE=interpreter \
  SCORECARD_V2_INPUT_POPULATION=stock-musl-v1 \
  INTERPRETER_AUDIT_RV64_INITRAMFS=/home/darren/src/jit/target/bench/interpreter-stock-musl-v1/interpreter-stock-musl-riscv64.cpio \
  INTERPRETER_AUDIT_X86_INITRAMFS=/home/darren/src/jit/target/bench/interpreter-stock-musl-v1/interpreter-stock-musl-x86.cpio \
  AUTHORITATIVE=1 REPS=3 V86_EXECUTION_PREFLIGHT=0 \
  SCORECARD_V2_OUTPUT=/home/darren/src/jit/target/bench/interpreter-final/stock-musl-v1-i004 \
  node tests/vs-v86/scorecard-v2.mjs
```

There was no runtime-Wasm override, profiler, diagnostic selector, altered row
list, sample extension, replacement, or rerun. All eligible samples were
retained in the order emitted by the harness. The input manifest passed both
before and after execution.

The driver exited with status 1 because the valid report sets
`goalMet=false`; this is the expected scorecard failure exit, not an execution
failure. The known corrupt local Git index produced provenance warnings and
leaves Git status unknown. Artifact, source-tree, guest, and workload hashes
are recorded independently in every trial.

## Fair input

The fixed-work BYTEmark binaries use the same nbench 2.2.3 source,
fixed-data32/fixed-work patches, Zig `-O2 -static` options, and
`-fno-builtin-memmove` / `-fno-builtin-memcpy` on both architectures. Neither
binary links a replacement memory implementation. In particular, the
development population's RV64-only `fastmem.c` is absent.

All trials record the same cross-ISA hashes for the workload contract
(`cc3f0116f4a6cf56c0a4bfc703e629db0571aa465ecb7cb124057f63d62ffe28`),
workload transforms
(`0a1f284f3cea9a8ed3498003d0e6ac45830463de03b500461469f8d452b14e2a`),
and implementation sources
(`145f86cd1ece2f6af0d4e0c58c984a6904e897872e32e013821e719518111f6b`).
The architecture-specific nbench hashes are `1ada7fb2cbd4...` for RV64 and
`0bcdb4594d08...` for i386; the initramfs hashes are `e75b6df497c8...` and
`4901c733a0c3...`, respectively.

## Result

The report is authoritative and measurement-valid with an empty `problems`
array. It records eleven wins, one match, and one loss against copy/v86:

| Row | I004 interpreter | copy/v86 interpreter | I004 / v86 |
| --- | ---: | ---: | ---: |
| ALU | 45,652.9 ms | 85,541.2 ms | `1.8737x` |
| Mixed | 21,074.2 ms | 21,451.1 ms | `1.0179x` match |
| Matched Boot | 1,647.3 ms | 2,944.2 ms | `1.7873x` |
| Python | 13,699.4 ms | 37,534.0 ms | `2.7398x` |
| Compile | 3,198.1 ms | 4,772.2 ms | `1.4922x` |
| Numeric Sort | 1,703.0 ms | 3,038.9 ms | `1.7844x` |
| String Sort | 28,366.8 ms | 8,724.3 ms | **`0.3076x` loss** |
| Bitfield | 2,361.8 ms | 2,912.2 ms | `1.2331x` |
| FP Emulation | 4,852.3 ms | 10,279.3 ms | `2.1185x` |
| Fourier | 4,039.4 ms | 5,880.9 ms | `1.4559x` |
| Assignment | 5,005.5 ms | 8,809.2 ms | `1.7599x` |
| IDEA | 3,940.7 ms | 9,795.4 ms | `2.4857x` |
| Huffman | 4,316.5 ms | 8,550.4 ms | `1.9809x` |

All 78 fresh-process trials are measurement-eligible and every trial proves
inactive JIT. Every row/side pair has exactly repetitions 1, 2, and 3. There
are no policy problems or diagnostic trials. The six trials without
FIRST/PRIME/STEADY are exactly the three Boot pairs, which correctly score
FIRST only. Maximum per-trial host-probe spread is `1.013977`; maximum scored
within-side sample spread is `1.160265`, both below the frozen `1.25` limits.

The exact rewrite identity is release Wasm
`7e7cee94eb5811b1582fb814a1cac567cfe323759564ce85a78eb063a919c4e9`.
The comparator is pinned copy/v86 commit
`2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`, source tree
`ca8afd71c1444a56c20b1ab63939569329fd5369a1a75760b5dce53fc3ba00f8`,
and Wasm `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1`.

The raw report is
`target/bench/interpreter-final/stock-musl-v1-i004/interpreter-scorecard-v2-2026-08-11T09-16-08-726Z.json`
(`bb4b277196d003f7c864bcb1d8570b5d43a5af798d9bda9b99616046dcf7e0c7`).
Its generated Markdown is the adjacent `.md` file
(`1a3670229a7770d46c22ba33bec2ea2ba1136754e47c7cb243069c4f36377095`).

## Decision

The fair stock-libc population confirms that the remaining String deficit is
not an artifact of the development population's RV64-only memory replacement.
I004 is `3.25x` behind copy/v86 on stock String and fails the preregistered
`0.95x` floor. The pure-interpreter parity goal is not met.

This sealed result cannot select or shape another product edit. At this
checkpoint exact I004 remained unchanged and the one-time holdout was next in
the previously frozen sequence. Whatever the holdout reported had to be
retained and could not rescue the already failed development and stock gates.

That later one-time execution is recorded separately in
[INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md](INTERPRETER_I004_AUTHORITATIVE_HOLDOUT_RESULT.md).
