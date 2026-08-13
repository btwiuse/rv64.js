# R109 Dense CFG Stackifier Result

Date: 2026-08-10  
Status: rejected at the frozen native gate; candidate removed; exact baseline restored

## Outcome

R109 tested one architecture-general compiler change: replace the stackifier's
ordered `BTreeMap`/`BTreeSet` working graph with a fixed 513-bit dense graph and
sets. It did not inspect guest PCs, opcodes, binaries, privilege, workload,
phase, engine, or results, and it did not change region selection, thresholds,
duplication policy, emitted instruction selection, or generated execution.

The mechanism was real and locally large, but it did not clear the verified
whole-product rule. After the frozen 1.042 ms construction debit, Boot improved
`1.01522x` with interval `[0.99726,1.02755]`; its normalized-MIPS median agreed
at `1.01507x`. Compile measured `0.98926x` with interval
`[0.96237,1.01259]`, and Python measured `1.00453x` with interval
`[0.98732,1.02888]`.

Boot therefore passed the 1% point threshold but missed the required 1.00
lower confidence bound. Compile missed the protected `0.99x` median by
`0.000743x`. This is an inconclusive Boot effect plus a protected-row miss, so
R109 stops at Gate D. There is no Chromium, WANIX, or authoritative scorecard
run, no extra samples, no hybrid or width variant, and no composition with the
R108 instruction sink.

## Gate A: frozen production CFG corpus

An off-by-default, timing-ineligible capture was temporarily placed immediately
before the unchanged production tree stackifier. One modern Linux 6.12.7 /
Alpine 3.24.1 Boot and one Compile guest retained every FIRST/PRIME/STEADY graph.
The capture hooks were removed before candidate implementation.

- Boot: 15 graphs, 3,721 nodes, 4,779 edges, maximum 472 nodes.
- Compile: 118 graphs, 11,042 nodes, 12,862 edges, maximum 512 nodes.
- All production entries were retained; there were no invalid, external, or
  duplicate production edges.
- Instrumented Wasm: `016766ec7b6a...`, 4,278,800 bytes.
- Corpus manifest: `target/bench/r109-dense-cfg/corpus-manifest.json`, SHA-256
  `85cb086d069e3...`.

The parser, raw `.rvcfg` files, instrumented artifact, guest/input hashes,
phase accounting, compile MD5, generated-execution proof, and production-policy
proof are retained. No capture timing was used.

## Gate B: standalone Wasm admission model

The standalone model implemented the current ordered algorithm and exactly one
dense bit-matrix algorithm. It validated byte-identical serialized structures
over all production graphs plus exhaustive directed graphs through three nodes
and 256 fixed-seed graphs through 512 nodes: 14,931 graphs, 121,283 nodes, and
145,744 edges in total.

Seven alternating fresh-V8-process pairs measured:

- Boot first call: `5.264x` `[4.989,5.487]`;
- Boot tiered steady: `6.628x` `[6.615,6.651]`;
- Compile first call: `5.052x`;
- Compile tiered steady: `5.874x`.

This passed every pre-registered model threshold. Report:
`target/bench/r109-dense-cfg/model-gate.json`, SHA-256
`080c09151805...`; model Wasm `35c47490d6b9...`, 111,482 bytes.

## Gate C: product correctness and exact emission

The admitted implementation used the exact model representation: nine `u64`
words for real members `0..511` plus synthetic entry `N`, ascending bit
iteration, the same SCC traversal, subgroup order, duplication decision,
structure tree, and blockification.

- Dense versus ordered product tests passed exhaustive adjacency through three
  nodes, filtered/duplicate inputs, and fixed-seed graphs through the 512-member
  hard cap.
- A deterministic real-RV64 compiler corpus generated 280 modules from rvbench
  and Alpine musl across seven geometries and five state modes. Control and
  candidate manifests and every Wasm byte were exact. Report:
  `target/bench/r109-dense-cfg/generated-byte-gate.json`, SHA-256
  `e621283402ad...`.
- Workspace units, raw Wasm smoke, 60 randomized scalar programs, memory, M,
  A, FP, Sv39/MPRV, WFI, T2 multientry/lifecycle/atomic, public API, Worker API,
  standalone Wasmtime, direct Linux, and OpenSBI Linux passed.
- The outer `tests/run-all.sh` command failed only when the separate native
  virt-smoke launcher could not find `riscv64-none-elf-gcc`; the direct and
  OpenSBI Wasm Linux 6.12.7 gates in the same run passed.

The candidate main Wasm was `b335bb69559a...`, 4,230,802 bytes, which is 48,578
bytes smaller than control. This size was diagnostic and played no part in
admission or rejection.

## Gate D: construction-debited native timing

Fifteen alternating fresh-process `RV64Debug.create` pairs on CPUs 8--15 used
exact control `d9f686a9ce4f...` and candidate `b335bb69559a...`. Medians were
20.621/20.544 ms. The paired candidate-control median delta was -0.179 ms with
interval `[-0.411,1.042]`, giving the frozen debit `D=1.041935 ms`. Report:
`target/bench/r109-dense-cfg/construction.json`, SHA-256
`9835d26ab531...`.

The valid 90-leg native report used 15 alternating fresh guests for each of
Boot, Compile, and Python, public one-slice cadence, production policy, CPUs
8--15, exact guest/input/output identities, and no failed or replacement leg.
Host-probe spread was `1.074x`.

| Row | Raw paired median (95%) | Debit-adjusted median (95%) | Decision |
|---|---:|---:|---|
| Boot | `1.01572x` `[0.99774,1.02805]` | `1.01522x` `[0.99726,1.02755]` | lower bound fails |
| Compile | `0.99027x` `[0.96335,1.01372]` | `0.98926x` `[0.96237,1.01259]` | protected median fails |
| Python | `1.00498x` `[0.98775,1.02936]` | `1.00453x` `[0.98732,1.02888]` | passes |

Native report: `target/bench/r109-dense-cfg/native/config-ab-2026-08-10T08-11-41-969Z.json`,
SHA-256 `38655e53ea2b...`. Mechanical gate:
`target/bench/r109-dense-cfg/native-gate.json`, SHA-256
`fef266dade72...`.

## Removal and retained evidence

The rejected dense implementation and its test-only ordered selector were
removed from the product. The release rebuild is byte-exact control
`d9f686a9ce4f...` at 4,279,380 bytes; `crates/rv64-core/src/cpu.rs` is
`aec4b31434a6...`, `crates/rv64-wasm/src/lib.rs` is `1da35e70bc9c...`, and
the loader is `2cbb264f4dac...`. Restored DBT structure tests pass.

Candidate source/model snapshot:
`target/bench/r109-dense-cfg/source-candidate.tgz`, SHA-256
`c3397a5fa657...`. The corpus parser, model runner, byte comparator, and native
gate remain as reproducibility tools; none is linked into the product.

The authoritative status remains 13/13 versus legacy and 11/13 versus
copy/v86. Boot and Compile remain the two open v86 rows.
