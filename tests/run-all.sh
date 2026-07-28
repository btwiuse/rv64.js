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
#   5. spike lockstep     per-instruction register-writeback diff vs Spike
#   6. riscv-arch-test    official architecture tests, signatures vs Spike
#   7. wasm build + smoke + JIT differential fuzzer through Node
#   8. virt-smoke        modern-system (Debian-class) boot regression test
#
# Under the nix dev shell (flake.nix) all tools are present.
set -u
# Stages 5 and 6 pipe their runner into `tail`, so without pipefail the
# EXIT STATUS IS TAIL'S — a failing lockstep or signature comparison
# printed its failure and the suite still reported ALL STAGES PASSED.
# (POSIX sh has no pipefail; re-exec under bash when it is available, which
# it is in the dev shell and on every platform this suite supports.)
if [ -z "${RUNALL_PIPEFAIL:-}" ] && command -v bash >/dev/null 2>&1; then
    RUNALL_PIPEFAIL=1 exec bash "$0" "$@"
fi
(set -o pipefail) 2>/dev/null && set -o pipefail
cd "$(dirname "$0")/.."
FAILED=0
note() { printf '\n=== %s\n' "$*"; }

note "1/8 cargo tests"
cargo test --workspace --release -q || FAILED=1

note "2/8 guest builds"
for g in hello-nostd hello-std fpu-test bench; do
    (cd "guests/$g" && cargo build --release -q) || FAILED=1
done
cargo build --release -q -p rv64-run -p rv64-system || FAILED=1
# re-run guest integration tests now that guests surely exist
cargo test --release -q -p rv64-linux || FAILED=1

note "3/8 qemu differential"
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

note "4/8 riscv-tests ISA suite"
PREFIX="${RISCV_PREFIX:-riscv64-unknown-elf-}"
if command -v "${PREFIX}gcc" >/dev/null 2>&1; then
    RISCV_PREFIX="$PREFIX" tests/run-isa-tests.sh || FAILED=1
else
    echo "SKIP (${PREFIX}gcc not found; set RISCV_PREFIX or enter nix develop)"
fi

note "5/8 spike lockstep differential"
if command -v spike >/dev/null 2>&1 && [ -d tests/riscv-tests/isa ]; then
    python3 tests/lockstep.py $(ls tests/riscv-tests/isa/rv64u[imac]-p-* \
        tests/riscv-tests/isa/rv64ud-p-* tests/riscv-tests/isa/rv64uf-p-* \
        2>/dev/null | grep -v dump) | tail -3 || FAILED=1
else
    echo "SKIP (spike or built riscv-tests missing; run stage 4 first)"
fi

note "6/8 riscv-arch-test signatures vs Spike"
if command -v spike >/dev/null 2>&1 && command -v "${PREFIX}gcc" >/dev/null 2>&1; then
    tests/run-arch-tests.sh | tail -1 || FAILED=1
else
    echo "SKIP (spike or ${PREFIX}gcc not found)"
fi

note "7/8 wasm build + smoke"
if command -v node >/dev/null 2>&1; then
    node tests/vs-v86/harness-selftest.mjs || FAILED=1
    cargo build --release -q -p rv64-wasm --target wasm32-unknown-unknown || FAILED=1
    node tests/http-relay.mjs || FAILED=1
    node tests/wasm-smoke.mjs || FAILED=1
    node tests/jit-differential.mjs || FAILED=1
    node tests/fp-context-switch.mjs || FAILED=1  # SKIPs without ARTIFACTS
    node tests/amo-diff.mjs || FAILED=1  # SKIPs without ARTIFACTS
else
    echo "SKIP (node not found)"
fi

note "8/8 virt-smoke (modern-system boot)"
# Only run when the kernel is already realized in the store; building it is a
# ~15 min one-off. `nix path-info` checks presence without triggering a build.
if command -v nix >/dev/null 2>&1 && nix path-info .#virt-kernel >/dev/null 2>&1; then
    tests/virt-smoke/run.sh || FAILED=1
else
    echo "SKIP (virt-kernel not built; run tests/virt-smoke/run.sh once to build+cache it)"
fi

printf '\n'
if [ "$FAILED" -eq 0 ]; then
    echo "ALL STAGES PASSED"
else
    echo "SUITE FAILED"
fi
exit "$FAILED"
