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

## Results (WORK=1, one machine, host wall-clock)

**Mixed** (integer + FP + memory):

| Emulator / mode | Time | vs native |
|---|---:|---:|
| Native (host) | 0.17–0.23 s | 1× |
| v86 JIT (i386, full-system) | **1.54 s** | ~9× |
| v86 interpreter | 26.8 s | ~160× |
| rv64.js wasm-JIT (user-mode) | **17.1 s** | ~76× |
| rv64.js wasm-interpreter | 19.5 s | ~86× |
| rv64.js native-rust interp (full-system) | 15.6 s | ~69× |

**Pure ALU** (xorshift):

| Emulator / mode | Time | JIT vs own interp |
|---|---:|---:|
| Native | 1.02 s | — |
| v86 JIT | **2.87 s** | 35× |
| v86 interpreter | 100.8 s | — |
| rv64.js wasm-JIT | **50.5 s** | 1.95× |
| rv64.js wasm-interpreter | 98.6 s | — |

### Takeaways

- **Interpreters are peers.** On ALU rv64.js (98.6 s) ties v86 (100.8 s); on the
  mixed workload rv64.js is ~1.4× faster. Our core interpreter is competitive.
- **v86's JIT is far ahead — ~11× (mixed) to ~18× (ALU).** v86's mature,
  register-allocating JIT gets 17–35× over its own interpreter and lands within
  ~2.8× of native on ALU. rv64.js's JIT is naive (no register allocation —
  ROADMAP item 2b) and compiles no FP (ROADMAP item 3), so it gets only 1.1–2×
  over its interpreter. The gap *is* the open JIT roadmap, quantified.

## Reproducing

Everything runs through the nix dev shell. Build the benchmarks:

```sh
# riscv64 (rv64.js) — via the flake's musl cross-gcc; x86 via gcc -m32
CC=$(command -v riscv64-unknown-linux-musl-gcc)
$CC -nostdlib -static -O2 -march=rv64gc -mabi=lp64d -Wl,-Ttext-segment=0x10000 -o alu.rv64 alu.c
gcc -m32 -nostdlib -static -no-pie -O2 -Wl,-Ttext-segment=0x8048000 -o alu.i386 alu.c
```

rv64.js (build the wasm first: `cargo build --release -p rv64-wasm --target wasm32-unknown-unknown`):

```sh
node xbench-user.mjs alu.rv64          # wasm interpreter + JIT, host-timed
```

v86 (clone github.com/copy/v86; build its wasm with the *unwrapped* clang —
the nixpkgs clang wrapper injects glibc headers that break the freestanding
wasm C objects):

```sh
# in the v86 checkout, with clang-unwrapped on PATH:
make build/softfloat.o build/zstddeclib.o build/v86.wasm
cp build/v86.wasm build/v86-debug.wasm     # src/main.js loads the debug name
node v86-compute.mjs                        # JIT;  DISABLE_JIT=1 for interpreter
```

`v86-compute.mjs` boots v86's `buildroot-bzimage68.bin` (i686), injects the
benchmark over 9p via `create_file`, runs it from tmpfs, and times
`BENCH_START`→`BENCH_DONE`. `v86-boot.mjs` is the boot-time benchmark
(linux4.iso): v86 JIT ~3.4 s, interpreter ~12.4 s here.

Images come from `https://i.copy.sh/` (`linux4.iso`, `buildroot-bzimage68.bin`).
