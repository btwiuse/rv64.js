#!/bin/sh
# Build (if needed) and run the official riscv-tests ISA suites against
# rv64-isa-test. Requires a bare-metal riscv64 cross gcc and git; set
# RISCV_PREFIX if yours isn't riscv64-unknown-elf- (the nix dev shell
# exports riscv64-none-elf-).
#
# Expected result: 134/134 pass.
set -e
cd "$(dirname "$0")"

if [ ! -d riscv-tests ]; then
    git clone -q --depth 1 --recurse-submodules --shallow-submodules \
        https://github.com/riscv-software-src/riscv-tests.git
fi
make -k -C riscv-tests/isa -j"$(nproc)" RISCV_PREFIX="${RISCV_PREFIX:-riscv64-unknown-elf-}" \
    rv64ui rv64um rv64ua rv64uc rv64ud rv64uf rv64mi rv64si >/dev/null 2>&1 || true

cargo build --release -p rv64-system --manifest-path ../Cargo.toml
../target/release/rv64-isa-test $(ls riscv-tests/isa/rv64*-p-* | grep -v dump)
