# RV64GCV Pure-Interpreter Final Report

Date: 2026-08-12  
Disposition: complete; authoritative pure-interpreter goal achieved; RVV JIT work not started

## Conclusion

The interpreter baseline is now RV64GCV with VLEN=128 and ELEN=64. The
interpreter implements the complete mandatory ratified RVV 1.0 base extension
required by that baseline, and the controlled JIT-disabled scorecard matches
or beats pinned copy/v86 on every row.

The authoritative result has 13 rows, three fresh-process repetitions per
side, 78 eligible trials, an empty problem list, and explicit inactive-JIT
proof for every trial. Its immutable fields are
`authoritative=true`, `measurementValid=true`, and `goalMet=true`.

This milestone stops at the interpreter as requested. No RVV instruction is
lowered by `rv64-dbt`; vector instructions continue through the interpreter
when the ordinary JIT is enabled.

## ISA boundary

“Full RVV” here means the mandatory base V extension in the
[ratified RISC-V vector specification](https://docs.riscv.org/reference/isa/v20260120/unpriv/v-st-ext.html)
for the selected `rv64gcv` architecture:

- VLEN=128, ELEN=64, and SEW 8/16/32/64;
- all legal LMUL values from `mf8` through `m8` where the operand EMUL is
  representable;
- vector configuration and CSRs, integer and fixed-point arithmetic, masks,
  reductions, permutations, multiply/divide, widening and narrowing;
- vector floating point for FP32 and FP64, including widening/narrowing,
  conversions, reductions, fused operations, and estimate instructions; and
- unit-stride, strided, indexed ordered/unordered, segmented, whole-register,
  mask, and fault-only-first memory operations, including architectural
  overlap, masking, tail, fault, and `vstart` restart rules.

Optional vector extensions not named by `RV64GCV`, such as vector crypto,
extra vector bit-manipulation, or half-precision packages, are not claimed.
That is an ISA boundary, not a benchmark-derived subset of base V.

## Implementation work

The main implementation is in `crates/rv64-core/src/cpu/vector.rs`. It adds
the complete base-V decode and semantic families, legality checks for mixed
EEW/EMUL register groups, precise restart and fault-only-first behavior,
sticky `vxsat` with all `vxrm` modes, vector FP exception behavior, and vector
architectural state/CSR handling. `crates/rv64-core/src/softfp.rs` also fixes
the architectural FP flag bit assignment used by scalar and vector FP.

The retained performance mechanisms are architecture-general:

- cache the already decoded current `VectorConfig` until a vector-config
  instruction changes it;
- inline general vector element/register accessors;
- use a permission-checked direct-memory path for ordinary constant-stride
  vector transfers when the complete byte range is on one proved RAM page;
- retain precise generic page/fault handling for all other memory cases; and
- dispatch common single-width integer OP-V operations through a compact hot
  path while keeping the full uncommon families available.

None selects a program, benchmark, symbol, guest PC, loop signature, or opcode
sequence. There is no String Sort, BYTEmark, libc, or workload recognizer in
the vector implementation. Unlike the withdrawn scalar-loop recognizers,
these mechanisms apply to every guest program solely from architectural
instruction fields and general memory permissions.

## Compiled workload baseline

The benchmark inputs are compiled from the controlled C sources. The fixed
BYTEmark source is nbench-byte-2.2.3 plus the existing equal-data-width and
fixed-work patches. Both targets are statically linked against their ordinary
target musl with `-O2`, `-fno-builtin-memcpy`, and
`-fno-builtin-memmove`. The RV64 binary is compiled by Zig for
`riscv64-linux-musl` with `-mcpu=baseline_rv64+v+zifencei`; the i386 binary is
compiled for `x86-linux-musl`.

`readelf -A nbench-fixed.rv64` records `v1p0`, `zve64d1p0`, and
`zvl128b1p0`, and disassembly confirms real compiler-emitted vector
configuration, strided memory, vector FP, conversion, and reduction
instructions. The scored population is `stock-musl-rv64gcv-v1`; neither side
uses the old RV64-only replacement memory source.

Pinned input identities:

| Input | SHA-256 |
| --- | --- |
| RV64GCV nbench ELF | `6363d56006d334394371066bbd5be74221b7e8407bb358967a304cf36c74eb2c` |
| i386 nbench ELF | `0bcdb4594d083a7540b1b87e13944a190130c0b99ebd96f7e8b6e132f02500dc` |
| RV64 initramfs | `5ebfe65a24e17252c60b42e766659b73291691fc9e50854df1ccd4e598b146f6` |
| i386 initramfs | `fa63ca47a99bd776489980e24ec54acb0fa693a0a3039eb7397da123aa4b4329` |

## Authoritative JIT-disabled scorecard

The table reports the median host duration in milliseconds. A larger ratio is
better for the rewrite because it is `v86 time / rewrite time`.

| Benchmark | RV64GCV interpreter | copy/v86 interpreter | Result |
| --- | ---: | ---: | ---: |
| ALU | 44,896.1 | 84,291.1 | WIN `1.8775x` |
| Mixed | 21,433.8 | 21,758.0 | MATCH `1.0151x` |
| Matched Boot | 1,644.8 | 2,941.1 | WIN `1.7881x` |
| Python fib(30) | 13,380.2 | 37,769.9 | WIN `2.8228x` |
| Compile (`tcc -c`) | 3,218.5 | 4,739.4 | WIN `1.4726x` |
| Numeric Sort | 1,677.6 | 3,061.8 | WIN `1.8251x` |
| String Sort | 6,806.1 | 8,833.3 | WIN `1.2978x` |
| Bitfield | 2,401.8 | 2,926.1 | WIN `1.2183x` |
| FP Emulation | 9,189.0 | 10,251.4 | WIN `1.1156x` |
| Fourier | 3,840.2 | 6,076.5 | WIN `1.5823x` |
| Assignment | 5,467.4 | 8,846.3 | WIN `1.6180x` |
| IDEA | 3,966.7 | 10,059.2 | WIN `2.5359x` |
| Huffman | 4,418.7 | 8,589.2 | WIN `1.9438x` |

The maximum host-probe spread is `1.0193`; the maximum scored-side sample
spread is `1.1952`, below the frozen `1.25` stability ceiling. All 78 workers
requested JIT-disabled execution and independently record `inactiveProof=true`
with zero generated instruction dispatch.

Evidence:

- JSON:
  `target/bench/rv64gcv-interpreter-authoritative-full-rvv-v2/interpreter-scorecard-v2-2026-08-12T21-34-53-584Z.json`
  (`35403df0dadd61bdb66e599459b7fb2bf7c4806fc26dce55a4990164c0283a42`);
- rendered report:
  `target/bench/rv64gcv-interpreter-authoritative-full-rvv-v2/interpreter-scorecard-v2-2026-08-12T21-34-53-584Z.md`
  (`12a6d42074be851a3f252f0de8e174d188035559c72f4521d9ca886b1405a2b7`);
- measured release Wasm:
  `39092beab711e2875692983c65a0304d50efa3bc75a0ae29bd5955e575e5e34d`.

The repository has a pre-existing corrupt Git index, so optional Git status
collection printed an error and the report records `gitStatus=unknown`. That
does not affect measurement eligibility: the report records exact executable
and input hashes, all trials are eligible, and `problems` is empty.

## Correctness and independence evidence

The final measured source passes:

- `cargo test --workspace`: all 189 selected tests pass, including core,
  DBT, Linux guest, full-system, boot/network, and Wasm suites;
- `tests/rvv-interpreter-differential.mjs`: all 8,310 QEMU comparisons pass
  (1,385 instruction/configuration cases times six adversarial state profiles,
  62,648 guest instructions per profile);
- `cargo fmt --all -- --check`;
- optimized `wasm32-unknown-unknown` release build;
- `tests/jit-disabled-bypass.mjs`: 100,000 instructions in one direct
  interpreter slice; and
- `tests/wasm-smoke.mjs`: all ABI, user-mode, generated-module, Linux boot,
  virtio, and proxy smoke checks pass.

The differential inventory covers every named mandatory base-V opcode, every
legal SEW/LMUL configuration, widening/narrowing EMUL boundaries, masked and
`vstart` execution, legal and illegal overlap patterns, every vector memory
mode and segment width, all `vxrm`/`frm` modes, vector CSRs, and unboxed FP32
scalar inputs. Targeted Rust regressions separately exercise illegal/fault and
precise restart behavior that cannot be compared as an ordinary successful
QEMU user-mode process.

Final source identities:

| Source | SHA-256 |
| --- | --- |
| `crates/rv64-core/src/cpu/vector.rs` | `16478662ae81e5b613fb3d9855efff6b15719494ce35ef6056690e5452ac6d07` |
| `crates/rv64-core/src/softfp.rs` | `c60599971a560e6fbcd19686109a58bb960fc9a4c4405ce94de4f6f46fb138e1` |
| `tests/rvv-interpreter-differential.mjs` | `d41d9ede87e7eb2c6d54b06a0d2ca6d2daa34b7c71dd37dbdce293869fa93316` |

## Stop boundary

The pure-interpreter objective is achieved. Work stops here before adding RVV
lowering to the JIT. The next distinct project, if authorized, is to teach
`rv64-dbt` to compile the same complete RV64GCV architectural surface while
retaining precise interpreter side exits for unsupported optional extensions.
