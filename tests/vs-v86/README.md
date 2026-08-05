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

## Benchmark policy (anti-overfitting)

The operational procedure is [METHODOLOGY.md](METHODOLOGY.md). It is mandatory
for performance claims: immutable Wasm control, parallel rejection screen,
serial fresh-process A/B, authoritative 13-row promotion, then full
correctness. Effects below 10% are ties.
Current priorities, rejected approaches, and the append-only result ledger are
in [PERFORMANCE_PROGRESS.md](../../PERFORMANCE_PROGRESS.md).

The bar is WIN or MATCH on **every** row, which removes the temptation to
optimize a favorite subset — but benchmark-driven tuning can still overfit.
Rules, per the 2026-07-24 review:

- **Labels.** ALU and Mixed are *microbenchmarks* (fixed-work synthetic
  kernels; ALU is the JIT's best case). Boot, python, compile, and the eight
  nbench kernels are *macro rows*. Never present microbenchmark wins as
  evidence of general parity — the macro rows carry that claim.
- **Tuning vs held-out.** An optimization motivated by row X must be
  re-validated on ALL rows before landing (the scorecard's all-rows verdict
  enforces this); a row that regresses is a blocker, not a footnote.
  Workloads are never added to or removed from the scorecard in reaction to
  a loss.
- **Diagnose, then fix; pin what you matched.** Pattern-matching
  optimizations (copy-loop shapes, superblock triggers) must carry unit
  tests pinning the exact shipped code shapes they claim to match, so a
  toolchain change that breaks the match is a visible test failure, not a
  silent perf cliff.
- **Report honestly.** Cold vs warm behavior, which JIT tier each row runs
  (basic blocks / loop regions / superblocks / bulk-copy), and single-run
  noise (this host shows double-digit variance and 2x outliers; borderline
  rows require interleaved median-of-N, REPS=N).
- **Provenance.** Every scorecard JSON records the git revision, wasm build
  hash, and config, so any number can be traced to the exact code that
  produced it.

## The scorecard (one command, full picture)

`scorecard.mjs` runs the whole suite system-mode and prints **one table**, then
saves a timestamped `scorecard-<ts>.md` + `.json` so before/after perf work is
directly comparable. This is the entry point — use the individual harnesses
below only for focused runs.

```sh
nix develop -c tests/vs-v86/setup.sh target/bench  # DEBIAN=1 for python + v86 compile/nbench

# Exploratory, not a promotion result:
ARTIFACTS=target/bench SB=1 REPS=1 \
  nix develop -c node tests/vs-v86/scorecard.mjs

# Authoritative promotion result:
AUTHORITATIVE=1 ARTIFACTS=target/bench NBENCH=1 SB=1 \
  REPS=3 NBREPS=3 nix develop -c node tests/vs-v86/scorecard.mjs
```

It covers ALU, Mixed, Boot, python fib(30), and **compile (tcc -c)** — all as
rv64-JIT-vs-v86-JIT head-to-heads — plus the **nbench** BYTEmark table (rv64 vs
v86) under `NBENCH=1`. Needs a built `copy/v86` at `$SC/v86`.

The compile + nbench rows run the **same source on both sides**, one build per
ISA: `w.c` (compile-bench/gen_c.py) through `tcc@d9d02c5`, and nbench-byte-2.2.3.
The cross-ISA binaries come from `mk-bench-bins.sh`, which builds static musl
`tcc.i386`/`tcc.rv64`/`nbench.i386` with `zig cc` — no 32-bit-glibc host
toolchain needed (the old blocker; see below). `mk-v86-bench.sh` bakes tcc +
nbench + `w.c` into one Debian-i386 v86 initramfs that dispatches on a `bench=`
cmdline token (python | tcc | nbench); the rv64 compile side boots `cc-bench.img`.

Every authoritative wall row runs in a fresh child process on both emulators,
with paired side order alternating each repetition. The whole nbench table uses
the same protocol. The JSON retains raw trials, host probes, JIT counters,
runtime configuration, v86 revision, and hashes for Wasm and guest artifacts.
An exclusive artifact-directory lock prevents concurrent benchmark
orchestrators.

The Boot row is matched separately from the workload rows. Both sides use
Linux 6.12.7 from this flake, Alpine 3.24.1 `alpine-base`, the same package-name
manifest and init script, 512 MiB RAM, and an uncompressed `newc` initramfs.
This removes the old root-disk and storage-controller mismatch. rv64.js boots
through OpenSBI 1.4; v86 boots through SeaBIOS because its Linux `bzImage`
loader is a SeaBIOS option ROM. Timings begin after VM creation and report
kernel-banner and userspace-ready milestones separately.
The RISC-V payload is an uncompressed `Image`; v86's normal Linux loader
requires an x86 `bzImage`, so kernel decompression remains an unavoidable
platform-path difference. The shared initramfs is uncompressed on both sides.

```sh
nix develop -c tests/vs-v86/prepare-matched-boot.sh target/bench
ARTIFACTS=target/bench REPS=5 \
  node tests/vs-v86/compare-boot.mjs
```

## Automated setup (build everything once)

`setup.sh` builds the wasm, the benchmark kernels, and the nbench rootfs into one
artifacts dir, ready for the harnesses — reproducibly, in the nix dev shell:

```sh
nix develop -c tests/vs-v86/setup.sh target/bench          # + DEBIAN=1 for python
ARTIFACTS=target/bench nix develop -c node tests/vs-v86/compare-sys.mjs
```

Pieces (each runnable alone): `build-kernels.sh` (the fixed-work C kernels for
both ISAs), `mk-nbench-rootfs.sh` (fetch + build nbench, bake into the buildroot
rootfs), `mk-debian-rootfs.sh` (Debian riscv64 rootfs with Python plus the
curl/OpenSSL/networking tools used by `tests/virt-proxy`). The v86 side still
needs a built `copy/v86` checkout at
`<outdir>/v86` (see "Reproducing").

## Running it — one command

`compare.mjs` is the reusable driver: it runs **both** emulators fresh on the
same machine (no hardcoded baselines), each in interpreter and JIT mode, and
prints the table below.

```sh
# SC points at the dir holding xbench/ (the compiled benchmark binaries);
# V86DIR defaults to $SC/v86 (a built copy/v86 checkout — see "Reproducing").
ARTIFACTS=<scratchpad> nix develop -c node tests/vs-v86/compare.mjs
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
ARTIFACTS=<scratchpad> nix develop -c node tests/vs-v86/compare-sys.mjs   # SKIP_INTERP=1 for JIT-only
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

**v86-side head-to-head is now wired up** (`scorecard.mjs` with `NBENCH=1`). The
old blocker — nbench for i386 needs 32-bit glibc (`gnu/stubs-32.h`), absent from
this nix env — is sidestepped by building a **static musl** `nbench.i386` with
`zig cc -target x86-linux-musl` (see `mk-bench-bins.sh`); it runs in v86's
Debian-i386 guest with no lib deps. rv64 stays the newlib buildroot build below;
both are the same nbench-byte-2.2.3 source, self-timed, so the per-kernel
iterations/sec compare directly (libc differs but nbench barely touches it).

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
ARTIFACTS=. ROOT_NBENCH=root-nbench.bin node tests/vs-v86/nbench.mjs
```

## Debian riscv64 userland (real python, glibc) — arch-python

To run v86's arch-python (`time python fib(30)`) and anything else needing a
real userland, we assemble a minimal **Debian riscv64** rootfs — no building
from scratch, just prebuilt `.debs` — that boots under our JIT-capable machine
(bbl64 + kernel-riscv64). `mk-debian-rootfs.sh` does it with no root and no
binfmt: `fakeroot debootstrap --foreign` (host-side download + first-stage
unpack), then `dpkg-deb -x` the requested packages. The default includes
Python, curl, OpenSSL, CA roots, iproute2, and a static busybox `udhcpc`; the
builder supplies the CA bundle and DHCP lease script that the foreign
debootstrap stage cannot generate. It then uses `mke2fs -d` to produce the
ext4 image. The virtio-blk disk is read on demand, so it fits under wasm's 4 GB
(image + 512 MB RAM). Boot with `init=/binit.sh` (mounts proc/dev/sys → shell).

```sh
nix develop -c tests/vs-v86/mk-debian-rootfs.sh target/bench
# -> target/bench/deb-riscv64.ext4
ARTIFACTS=<outdir> nix develop -c node tests/vs-v86/deb-python.mjs
nix develop -c tests/virt-proxy/run.sh
```

**arch-python — fib(30), APPLES-TO-APPLES** (same Debian trixie python 3.13 on
both emulators, both JIT; result 832040). The v86 side boots a Debian **i386**
rootfs (same `mk-debian-rootfs.sh`, `ARCH=i386`) as an initramfs under a stock
i386 kernel — because v86 has only IDE disks and its buildroot kernel lacks ATA,
so booting the rootfs *as* an initramfs needs no block device. `mk-v86-debian.sh`
builds those artifacts; `compare-python.mjs` runs both:

| | rv64 (riscv64) | v86 (i386) | winner |
|---|---:|---:|---:|
| fib(30), JIT | 15.4 s | **2.7 s** | **v86 ~5.6× faster** |

This is the honest flip side of the compute-loop result: rv64's structured-loop
JIT wins on tight numeric loops (alu/mixed ~2× *faster* than v86), but v86's
mature register-allocating JIT wins big on CPython's branchy, indirect-call-heavy
eval loop. Different JITs excel at different workload shapes.

```sh
nix develop -c tests/vs-v86/mk-debian-rootfs.sh <out>            # riscv64 rootfs
ARCH=i386 nix develop -c tests/vs-v86/mk-debian-rootfs.sh <out>  # i386 rootfs
nix develop -c tests/vs-v86/mk-v86-debian.sh <out>               # v86 kernel + initramfs
ARTIFACTS=<out> nix develop -c node tests/vs-v86/compare-python.mjs     # (v86 checkout at <out>/v86)
```

This Debian rootfs is also a glibc environment, so a glibc-built nbench there
*can* read a `-cCMD` command file (unlike the newlib build above, since riscv64
Linux has no `open`, only `openat`, which newlib doesn't wire).

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
