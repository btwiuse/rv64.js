# rv64.js vs v86 — cross-emulator performance comparison

A like-for-like benchmark of **rv64.js** against **[copy/v86](https://github.com/copy/v86)**
(a mature, well-regarded x86-to-wasm emulator), run on the same machine.

## What v86 measures, and what we borrowed

v86's own benchmarks (`v86/tests/benchmark/`) are:
- **arch-bytemark.js** — `nbench` (BYTEmark) in booted Arch Linux; INTEGER/FP/MEMORY index scores.
- **linux-boot.js** — boot Linux to a shell prompt; wall-clock ms.
- **arch-python.js** — `time python fib(30)`.

They rely on the benchmark self-timing via the guest clock, which is unreliable
under emulation (the guest clock rate vs wall-clock differs per emulator, and
each emulator calibrates it differently). So instead we use the same *spirit*
(portable compute kernels) but a **cleaner metric**:

> **Fixed work, identical source, measured by host wall-clock, checksum-verified.**

Two freestanding benchmarks compiled for both ISAs (x86 for v86, riscv64 for
rv64.js), run to completion, timed between `BENCH_START` and `BENCH_DONE` on the
serial console:

- `rvbench_fs.c` — mixed: integer insertion-sort + FP recurrence + strided
  memory (nbench-like).
- `alu.c` — pure register ALU (xorshift), the JIT's best case (no FP, no
  data-dependent branches).

The checksum printed by each run confirms every build does identical work
(integer parts match bit-for-bit across x86/riscv64; the mixed FP checksum
differs only by 32-bit-vs-64-bit rounding, with identical operation counts).

## Running it — one command

`compare.mjs` is the reusable driver: it runs **both** emulators fresh on the
same machine (no hardcoded baselines), each in interpreter and JIT mode, and
prints the table below.

```sh
# SC points at the dir holding xbench/ (the compiled benchmark binaries);
# V86DIR defaults to $SC/v86 (a built copy/v86 checkout — see "Reproducing").
SC=<scratchpad> nix develop -c node tests/vs-v86/compare.mjs
# JIT-vs-JIT only (skip the slow interpreter runs):  SKIP_INTERP=1
```

If v86 isn't present it measures rv64.js alone and says so. The interpreter runs
are slow (tens of seconds to ~1.5 min each); the JIT runs are best-of-3.

## Results (WORK=1, one machine, host wall-clock, JIT roadmap 3a–3e complete)

Fresh `compare.mjs` run (2026-07-23). **rv64.js's JIT now beats v86's** on both:

| Workload | rv64 interp | **rv64 JIT** | v86 interp | **v86 JIT** | rv64 JIT vs v86 JIT |
|---|---:|---:|---:|---:|---:|
| ALU (xorshift) | 85.8 s | **1.48 s** (100% cov) | 86.4 s | 2.82 s | **0.53× — 1.9× faster** |
| Mixed (int+FP+mem) | 18.1 s | **0.57 s** (100% cov) | 21.7 s | 1.42 s | **0.40× — 2.5× faster** |

JIT speedup over each emulator's own interpreter: ALU rv64 **58×** / v86 31×;
mixed rv64 **32×** / v86 15×.

Cross-emulator correctness: the ALU checksum is **bit-identical** across ISAs
(rv64 `0xf858aba6` == v86 `0xf858aba6`). The mixed checksum differs only by
FP width (riscv64 double vs i386 SSE rounding), same integer operation counts.

### Takeaways

- **Interpreters are peers** (ALU 85.8 vs 86.4 s; mixed rv64 ~1.2× faster).
- **rv64.js's JIT now leads** — 1.9× (ALU) to 2.5× (mixed) faster than v86's,
  at 100% coverage. This is the payoff of ROADMAP items 3a–3e: registers in
  wasm locals, structured-CFG loop compilation (nested loops + breaks as one
  wasm function), and inline FP arith/compare/load/store. Earlier this harness
  measured the *reverse* (v86 ~11–18× ahead) before that work landed; riscv64
  turned out to be an easier JIT target than x86 (no condition-flag machinery,
  fixed-width decode, clean IEEE F/D).

## Reproducing

Everything runs through the nix dev shell. Build the benchmarks (into `$SC/xbench/`):

```sh
# riscv64 (rv64.js): alu.rv64 freestanding; rvbench.rv64 via musl for the mixed
CC=$(command -v riscv64-unknown-linux-musl-gcc)
$CC -nostdlib -static -O2 -march=rv64gc -mabi=lp64d -Wl,-Ttext-segment=0x10000 -o alu.rv64 alu.c
$CC          -static -O2 -march=rv64gc -mabi=lp64d -ffp-contract=off -o rvbench.rv64 rvbench.c
# x86 (v86): both freestanding, -m32
gcc -m32 -nostdlib -static -no-pie -O2 -Wl,-Ttext-segment=0x8048000 -o alu.i386        alu.c
gcc -m32 -nostdlib -static -no-pie -O2 -Wl,-Ttext-segment=0x8048000 -o rvbench_fs.i386 rvbench_fs.c
```

(`-ffp-contract=off` keeps riscv64 gcc from emitting FMADD — no wasm fma — so
the JIT can translate the FP bit-exactly, and it matches v86's fma-less i386
build.) Build the rv64 wasm first:
`cargo build --release -p rv64-wasm --target wasm32-unknown-unknown`.

v86 (clone github.com/copy/v86 into `$V86DIR`; build its wasm with the
*unwrapped* clang — the nixpkgs clang wrapper injects glibc headers that break
the freestanding wasm C objects):

```sh
# in the v86 checkout, with clang-unwrapped on PATH:
make build/softfloat.o build/zstddeclib.o build/v86.wasm
cp build/v86.wasm build/v86-debug.wasm     # src/main.js loads the debug name
```

`compare.mjs` drives v86 via `v86-compute.mjs`, which boots
`buildroot-bzimage68.bin` (i686), injects the benchmark over 9p via
`create_file`, runs it from tmpfs, and times `BENCH_START`→`BENCH_DONE`
(`BIN` selects `alu.i386` vs `rvbench_fs.i386`). `v86-boot.mjs` is the separate
boot-time benchmark (linux4.iso): v86 JIT ~3.4 s, interpreter ~12.4 s here.
Images come from `https://i.copy.sh/` (`linux4.iso`, `buildroot-bzimage68.bin`).
