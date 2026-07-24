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

## System mode — the comparison that counts (v86 has no user mode)

v86 is a full-system emulator; it has no user mode. The fair comparison boots a
full Linux on **both** sides and runs the same fixed-work kernel *inside the
guest*, host-wall-clock timed between the guest's serial `BENCH_START` /
`BENCH_DONE` markers. `compare-sys.mjs` is the driver:

```sh
SC=<scratchpad> nix develop -c node tests/vs-v86/compare-sys.mjs   # SKIP_INTERP=1 for JIT-only
```

Fresh run (2026-07-23, after roadmap item 3f ported the loop + FP JIT into
system mode). **rv64.js's JIT beats v86's here too:**

| Workload | rv64 interp | **rv64 JIT** | v86 interp | **v86 JIT** | rv64 JIT vs v86 JIT |
|---|---:|---:|---:|---:|---:|
| ALU | 78.8 s | **1.42 s** | 83.2 s | 2.79 s | **0.51× — 2.0× faster** |
| Mixed | 22.1 s | **0.66 s** | 25.7 s | 1.41 s | **0.47× — 2.1× faster** |

JIT over each emulator's own interpreter: ALU rv64 **56×** / v86 30×; mixed
rv64 **34×** / v86 18×. Checksums cross-verify (ALU `0xf858aba6` on both; mixed
low-32 `16c84da4` matches).

Before 3f this was the reverse — v86 was ~10–12× ahead — because rv64.js's JIT
wins (structured/nested loops, FP-in-blocks) were user-mode gated. 3f enabled
them for system blocks (mid-loop bail reports the live iteration count; the
system FP file drives FP-in-blocks), and now rv64.js leads v86 in *both* modes.

## nbench (BYTEmark) — v86's actual arch-bytemark compute benchmark

v86's flagship compute benchmark (`tests/benchmark/arch-bytemark.js`) runs the
real **nbench** (BYTEmark, Uwe Mayer's Linux port) inside a booted Linux. We run
the *same* nbench source, built for riscv64, inside our full-system Linux
(`nbench.mjs`). It exercises 10 kernels grouped into INTEGER / FP / MEMORY
indices (NUMERIC SORT, STRING SORT, BITFIELD, FP EMULATION, FOURIER, ASSIGNMENT,
IDEA, HUFFMAN, NEURAL NET, LU DECOMPOSITION).

**The clock caveat (important):** nbench *self-times* via the guest clock. Our
default clock is instruction-counted (deterministic — see "Clock" below), which
would make nbench's scores identical JIT-vs-interp. So `nbench.mjs` enables the
opt-in **wall-clock** time source (`sys_set_wallclock`), making the self-timed
scores reflect real throughput exactly as they do under v86.

Our results (riscv64, wall-clock, JIT vs our interpreter; "New Index" =
×AMD-K6/233, v86's own scale). The JIT's per-kernel effect is what BYTEmark is
designed to expose:

| Kernel | interp iters/s | JIT iters/s | JIT speedup |
|---|---:|---:|---:|
| NUMERIC SORT | 33.6 | 65.4 | 1.9× |
| STRING SORT | 0.63 | 23.2 | **37×** |
| BITFIELD | 8.7e6 | 5.5e8 | **64×** |
| FP EMULATION | 4.08 | 12.4 | 3.0× |
| FOURIER | 741 | 728 | 1.0× (libm sin/cos/pow — not JIT'd) |
| ASSIGNMENT | 0.86 | 5.15 | **6.0×** |
| IDEA | 142 | 353 | 2.5× |
| HUFFMAN | 67 | 71 | 1.1× |

**v86-side head-to-head is not yet wired up** — two environment blockers:
building nbench for i386 needs 32-bit glibc (`gnu/stubs-32.h`), absent from this
nix env (our freestanding kernels use `-nostdlib`, but nbench needs full libc);
and v86's own arch-bytemark needs its Arch-Linux image + snapshot, not
downloaded here. Options to close it: fetch v86's Arch image
(`tests/benchmark/fetch-download.js`) and run its real arch-bytemark, or get a
32-bit libc to build `nbench.i386` and inject it into v86's buildroot.

Build + bake (riscv64 side), for `nbench.mjs`:

```sh
# newlib cross-gcc; needs pointer.h=`#define LONG64`, a popen-free hardware()
# stub, and stub sysinfo.c/sysinfoc.c (see git history for the exact stubs).
riscv64-none-elf-gcc -DLINUX -O2 -static -march=rv64gc -mabi=lp64d \
  -o nbench.rv64 nbench0.c nbench1.c sysspec.c misc.c emfloat.c hwstub.c -lm
riscv64-none-elf-strip -s nbench.rv64
# bake into a copy of the ext2 rootfs (nbench has no in-guest open() — riscv64
# Linux only has openat and newlib's open isn't wired — so no command file;
# lower MINIMUM_SECONDS in nmglobal.h at build time instead of via -cCMD):
cp web/images/root-riscv64.bin root-nbench.bin
debugfs -w -R 'write nbench.rv64 nbench' root-nbench.bin
debugfs -w -R 'sif /nbench mode 0100755' root-nbench.bin
SC=. ROOT_NBENCH=root-nbench.bin node tests/vs-v86/nbench.mjs
```

## Clock: instruction-counted (default) vs wall-clock (opt-in)

Our CLINT `mtime` is derived from `insn_count` — deterministic "virtual time"
(QEMU's `-icount`), which our lockstep/differential testing vs Spike relies on.
`sys_set_wallclock(1)` opts into a real host-wall-clock source instead (like v86
and real hardware), so `gettimeofday`/`clock` and self-timing benchmarks reflect
real time. Both are correct emulator behaviour; the default stays deterministic
so validation is reproducible. See the commit adding `Machine::wall_ns`.

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
