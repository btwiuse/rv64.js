#!/bin/sh
# Build (if needed) and run the official riscv-tests ISA suites against
# rv64-isa-test. Requires: gcc-riscv64-unknown-elf, git.
#
# Known failures (13): rv64uf/ud fadd, fcmp, fcvt, fcvt_w, fdiv, fmadd, fmin
# — all fflags checks. The F/D implementation uses the host FPU: results
# are IEEE-correct but fflags are only approximated (see DESIGN.md;
# softfloat port is the planned fix). Everything else passes: 121/134.
set -e
cd "$(dirname "$0")"

if [ ! -d riscv-tests ]; then
    git clone -q --depth 1 --recurse-submodules --shallow-submodules \
        https://github.com/riscv-software-src/riscv-tests.git
fi
make -k -C riscv-tests/isa -j"$(nproc)" RISCV_PREFIX=riscv64-unknown-elf- \
    rv64ui rv64um rv64ua rv64uc rv64ud rv64uf rv64mi rv64si >/dev/null 2>&1 || true

cargo build --release -p rv64-system --manifest-path ../Cargo.toml
../target/release/rv64-isa-test $(ls riscv-tests/isa/rv64*-p-* | grep -v dump)
