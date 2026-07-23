#!/bin/sh
# rv64.js full test suite. Run from anywhere; skips stages whose external
# tools are missing (each skip is reported). Exit 0 = everything that could
# run passed.
#
# Stages:
#   1. cargo tests        unit + guest-integration tests (23)
#   2. guest builds       Rust->riscv64gc-musl test binaries
#   3. qemu differential  guests bit-identical on qemu-riscv64
#   4. riscv-tests        official ISA suite, 134 tests (cross-gcc needed)
#   5. wasm build + smoke user-mode/JIT/Linux-boot through Node
#
# Under the nix dev shell (flake.nix) all tools are present.
set -u
cd "$(dirname "$0")/.."
FAILED=0
note() { printf '\n=== %s\n' "$*"; }

note "1/5 cargo tests"
cargo test --workspace --release -q || FAILED=1

note "2/5 guest builds"
for g in hello-nostd hello-std fpu-test bench; do
    (cd "guests/$g" && cargo build --release -q) || FAILED=1
done
cargo build --release -q -p rv64-run -p rv64-system || FAILED=1
# re-run guest integration tests now that guests surely exist
cargo test --release -q -p rv64-linux || FAILED=1

note "3/5 qemu differential"
if command -v qemu-riscv64 >/dev/null 2>&1; then
    for g in hello-std fpu-test bench; do
        B="guests/$g/target/riscv64gc-unknown-linux-musl/release/$g"
        qemu-riscv64 "$B" a b >/tmp/rv64-q.out 2>&1; QE=$?
        ./target/release/rv64-run "$B" a b >/tmp/rv64-r.out 2>&1; RE=$?
        if [ "$QE" -eq "$RE" ] && cmp -s /tmp/rv64-q.out /tmp/rv64-r.out; then
            echo "DIFF-OK $g"
        else
            echo "DIFF-FAIL $g (qemu=$QE rv64=$RE)"; FAILED=1
        fi
    done
else
    echo "SKIP (qemu-riscv64 not found)"
fi

note "4/5 riscv-tests ISA suite"
PREFIX="${RISCV_PREFIX:-riscv64-unknown-elf-}"
if command -v "${PREFIX}gcc" >/dev/null 2>&1; then
    RISCV_PREFIX="$PREFIX" tests/run-isa-tests.sh || FAILED=1
else
    echo "SKIP (${PREFIX}gcc not found; set RISCV_PREFIX or enter nix develop)"
fi

note "5/5 wasm build + smoke"
if command -v node >/dev/null 2>&1; then
    cargo build --release -q -p rv64-wasm --target wasm32-unknown-unknown || FAILED=1
    node tests/wasm-smoke.mjs || FAILED=1
else
    echo "SKIP (node not found)"
fi

printf '\n'
if [ "$FAILED" -eq 0 ]; then
    echo "ALL STAGES PASSED"
else
    echo "SUITE FAILED"
fi
exit "$FAILED"
