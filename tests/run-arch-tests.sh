#!/bin/sh
# riscv-arch-test signature comparison: compile the official architecture
# tests once (shared env in tests/arch-env), run each on rv64.js AND on
# Spike (the RISC-V golden model), and require bit-identical signatures.
# This is the substance of RISCOF compliance with Spike as reference.
#
# Requires (all in `nix develop`): riscv cross gcc (RISCV_PREFIX), spike.
# Suites: I M A C F D Zifencei privilege (rv64i_m).
set -u
cd "$(dirname "$0")"
PREFIX="${RISCV_PREFIX:-riscv64-unknown-elf-}"
SPIKE="${SPIKE:-spike}"
ARCH="${ARCH_TEST_DIR:-riscv-arch-test}"
SUITES="${SUITES:-I M A C F D Zifencei privilege}"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if [ ! -d "$ARCH/riscv-test-suite" ]; then
    git clone -q --depth 1 --branch 3.9.1 \
        https://github.com/riscv-non-isa/riscv-arch-test.git "$ARCH"
fi
cargo build --release -q -p rv64-system --manifest-path ../Cargo.toml

PASS=0; FAIL=0; CFAIL=0
for suite in $SUITES; do
    for src in "$ARCH/riscv-test-suite/rv64i_m/$suite/src/"*.S; do
        [ -e "$src" ] || continue
        name=$(basename "$src" .S)
        elf="$WORK/$name.elf"
        if ! "${PREFIX}gcc" -march=rv64gc_zicsr_zifencei -mabi=lp64d \
            -static -mcmodel=medany -fvisibility=hidden -nostdlib \
            -nostartfiles -DXLEN=64 -DFLEN=64 \
            -I arch-env -I "$ARCH/riscv-test-suite/env" \
            -T arch-env/link.ld "$src" -o "$elf" 2>"$WORK/$name.cc.err"; then
            echo "CFAIL $suite/$name (compile)"
            CFAIL=$((CFAIL + 1))
            continue
        fi
        ../target/release/rv64-isa-test --signature "$WORK/$name.rv64.sig" \
            "$elf" >/dev/null 2>&1
        "$SPIKE" --isa=rv64gc_zicsr_zifencei "+signature=$WORK/$name.ref.sig" \
            +signature-granularity=4 "$elf" >/dev/null 2>&1
        if [ -s "$WORK/$name.rv64.sig" ] && [ -s "$WORK/$name.ref.sig" ] &&
            cmp -s "$WORK/$name.rv64.sig" "$WORK/$name.ref.sig"; then
            PASS=$((PASS + 1))
            echo "SIG-OK  $suite/$name"
        else
            FAIL=$((FAIL + 1))
            echo "SIG-BAD $suite/$name"
        fi
    done
done
echo "--- arch-test signatures: $PASS match, $FAIL mismatch, $CFAIL compile-skips"
[ "$FAIL" -eq 0 ]
