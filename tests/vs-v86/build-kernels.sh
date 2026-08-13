#!/usr/bin/env bash
# Build the fixed-work benchmark kernels for both ISAs into $OUT/xbench/, the
# layout the compare*.mjs harnesses expect (ARTIFACTS=$OUT). Reproducible; run in the
# nix dev shell:  nix develop -c tests/vs-v86/build-kernels.sh [outdir]
#
#   alu.rv64 / rvbench_fs.rv64  freestanding riscv64 (raw ecalls, -nostdlib)
#   rvbench.rv64               libc riscv64 (newlib; prints isum=, for compare.mjs)
#   alu.i386 / rvbench_fs.i386 freestanding i386 for v86 (-m32 -nostdlib)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-$PWD}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
XB="$OUT/xbench"; mkdir -p "$XB"
command -v riscv64-none-elf-gcc >/dev/null || { echo "run inside: nix develop -c $0"; exit 1; }

RVCC=riscv64-none-elf-gcc
RVF="-static -O2 -march=rv64gcv_zicsr_zifencei -mabi=lp64d"

# riscv64 freestanding (self-contained _start + raw ecalls, no libc)
$RVCC -nostdlib $RVF -o "$XB/alu.rv64"        "$HERE/alu.c"
# amo.rv64: every AMO* form the JIT compiles, checksummed — tests/amo-diff.mjs
# compares JIT vs interpreter vs superblock on it. CLEANUP_PLAN flagged this
# artifact as having no producer, so amo-diff SKIPped in every run.
# -mno-relax: amo.c has a PLAIN C _start (no naked gp setup like alu.c), so
# linker relaxation to gp-relative addressing would store through an unset
# gp — measured: StoreAccessFault at 0xfffffffffffff820, no output at all.
$RVCC -nostdlib $RVF -mno-relax -o "$XB/amo.rv64"        "$HERE/../guest/amo.c"
# -ffp-contract=off: i386 (SSE2, no fma) cannot contract, so the riscv64
# build must not either or the FP folds diverge (fmadd single-rounds).
$RVCC -nostdlib $RVF -ffp-contract=off -o "$XB/rvbench_fs.rv64" "$HERE/rvbench_fs.c"
# riscv64 libc (newlib): -ffp-contract=off so gcc emits mul+add, not FMADD —
# the JIT translates those bit-exactly (no wasm fma) and it matches v86's build.
$RVCC          $RVF -ffp-contract=off -o "$XB/rvbench.rv64" "$HERE/rvbench.c"

# i386 freestanding for v86 (nixpkgs gcc has no 32-bit glibc headers, so the
# benchmarks stay -nostdlib; that's why the mixed v86 build is rvbench_fs, the
# freestanding twin of rvbench.c — identical work).
# -fno-stack-protector is REQUIRED: the nixpkgs cc-wrapper enables the
# stack protector by default, whose %gs:0x14 canary reads fault in a
# freestanding (no-TLS) binary.
# -msse2 -mfpmath=sse is REQUIRED for cross-ISA comparability: without
# -mfpmath=sse, gcc keeps double math on x87 whose 80-bit intermediates
# produce a different FP fold than the riscv64 IEEE-double build (and v86
# executes the x87 pattern of this kernel ~10x slower). SSE2 doubles are
# bit-identical to riscv64 doubles, so the Mixed checksum cross-check holds
# for freshly built artifacts on BOTH sides.
I386F="-m32 -msse2 -mfpmath=sse -nostdlib -static -no-pie -O2 -fno-stack-protector"
gcc $I386F -o "$XB/alu.i386"        "$HERE/alu.c"
gcc $I386F -o "$XB/rvbench_fs.i386" "$HERE/rvbench_fs.c"

echo "built into $XB:"; ls -1 "$XB" | sed 's/^/  /'
