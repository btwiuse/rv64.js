#!/bin/sh
# rv64.js full test suite. Run from anywhere; skips stages whose external
# tools are missing (each skip is reported). Set REQUIRE_ALL=1 for a release
# gate where a skipped tool-dependent stage is a failure. Exit 0 means every
# stage that ran passed, and in strict mode that every stage was available.
#
# Stages:
#   1. cargo tests        unit + guest-integration tests
#   2. guest builds       Rust->riscv64gc-musl test binaries
#   3. qemu differential  guests bit-identical on qemu-riscv64
#   4. riscv-tests        official ISA suite, 134 tests (cross-gcc needed)
#   5. spike lockstep     per-instruction register-writeback diff vs Spike
#   6. riscv-arch-test    official architecture tests, signatures vs Spike
#   7. wasm build + smoke + JIT differential fuzzer through Node
#   8. virt-smoke        slim modern OpenSBI/Linux boot regression test
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
cd "$(dirname "$0")/.." || exit 1
FAILED=0
note() { printf '\n=== %s\n' "$*"; }
skip() {
    echo "SKIP ($*)"
    if [ "${REQUIRE_ALL:-0}" = 1 ]; then FAILED=1; fi
}

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
    node tests/rvv-interpreter-differential.mjs || FAILED=1
else
    skip "qemu-riscv64 not found"
fi

note "4/8 riscv-tests ISA suite"
PREFIX="${RISCV_PREFIX:-riscv64-unknown-elf-}"
if command -v "${PREFIX}gcc" >/dev/null 2>&1; then
    RISCV_PREFIX="$PREFIX" tests/run-isa-tests.sh || FAILED=1
else
    skip "${PREFIX}gcc not found; set RISCV_PREFIX or enter nix develop"
fi

note "5/8 spike lockstep differential"
if command -v spike >/dev/null 2>&1 && [ -d tests/riscv-tests/isa ]; then
    python3 tests/lockstep.py $(ls tests/riscv-tests/isa/rv64u[imac]-p-* \
        tests/riscv-tests/isa/rv64ud-p-* tests/riscv-tests/isa/rv64uf-p-* \
        2>/dev/null | grep -v dump) | tail -3 || FAILED=1
else
    skip "spike or built riscv-tests missing; run stage 4 first"
fi

note "6/8 riscv-arch-test signatures vs Spike"
if command -v spike >/dev/null 2>&1 && command -v "${PREFIX}gcc" >/dev/null 2>&1; then
    tests/run-arch-tests.sh | tail -1 || FAILED=1
else
    skip "spike or ${PREFIX}gcc not found"
fi

note "7/8 wasm build + smoke"
if command -v node >/dev/null 2>&1; then
    node tests/vs-v86/harness-selftest.mjs || FAILED=1
    node tests/vs-v86/amortized-cold-cost-selftest.mjs || FAILED=1
    node tests/analyze-wanix-pairs-selftest.mjs || FAILED=1
    cargo build --release -q -p rv64-wasm --target wasm32-unknown-unknown || FAILED=1
    node tests/public-api.mjs || FAILED=1
    node tests/worker-api.mjs || FAILED=1
    node tests/http-relay.mjs || FAILED=1
    node tests/p9-concurrency.mjs || FAILED=1
    node tests/wanix-p9-singleflight.mjs || FAILED=1
    node tests/wasm-smoke.mjs || FAILED=1
    node tests/jit-differential.mjs || FAILED=1
    node tests/jit-memory-differential.mjs || FAILED=1
    node tests/jit-m-differential.mjs || FAILED=1
    node tests/jit-forward-trace-differential.mjs || FAILED=1
    node tests/jit-fp-differential.mjs || FAILED=1
    node tests/jit-fp-fastpath-differential.mjs || FAILED=1
    node tests/jit-vector-differential.mjs || FAILED=1
    node tests/jit-vector-fault-differential.mjs || FAILED=1
    node tests/jit-system-vector-differential.mjs || FAILED=1
    node tests/jit-a-differential.mjs || FAILED=1
    node tests/jit-system-memory-differential.mjs || FAILED=1
    node tests/jit-system-memory-exits.mjs || FAILED=1
    node tests/jit-system-bulk-copy-differential.mjs || FAILED=1
    node tests/jit-disabled-bypass.mjs || FAILED=1
    node tests/jit-system-wfi-yield.mjs || FAILED=1
    node tests/jit-page-policy-wfi-yield.mjs || FAILED=1
    node tests/jit-page-policy-smoke.mjs || FAILED=1
    node tests/jit-page-policy-multipage.mjs || FAILED=1
    node tests/jit-policy-trace-smoke.mjs || FAILED=1
    node tests/jit-system-sv39-differential.mjs || FAILED=1
    node tests/jit-tlb-context-probe.mjs || FAILED=1
    node tests/jit-system-a-differential.mjs || FAILED=1
    node tests/jit-system-fp-differential.mjs || FAILED=1
    node tests/jit-t2-multientry-differential.mjs || FAILED=1
    node tests/jit-t2-atomic-random.mjs || FAILED=1
    node tests/jit-standalone-wasmtime.mjs || FAILED=1
    node tests/jit-modern-linux.mjs || FAILED=1
    node tests/jit-page-policy-public-modern.mjs || FAILED=1
    ARTIFACTS="${ARTIFACTS:-target/bench}" node tests/fp-context-switch.mjs || FAILED=1
    ARTIFACTS="${ARTIFACTS:-target/bench}" node tests/amo-diff.mjs || FAILED=1
else
    skip "node not found"
fi

note "8/8 virt-smoke (modern-system boot)"
# Only run when the kernel is already realized in the store; building it is a
# ~15 min one-off. `nix path-info` checks presence without triggering a build.
if command -v nix >/dev/null 2>&1 && nix path-info .#virt-kernel-fast >/dev/null 2>&1; then
    tests/virt-smoke/run.sh || FAILED=1
else
    skip "virt-kernel-fast not built; run tests/virt-smoke/run.sh once to build+cache it"
fi

printf '\n'
if [ "$FAILED" -eq 0 ]; then
    echo "ALL STAGES PASSED"
else
    echo "SUITE FAILED"
fi
exit "$FAILED"
