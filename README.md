# rv64.js

A RISC-V (RV64) emulator for the browser — [TinyEMU](https://bellard.org/tinyemu/)'s
scope with [copy/v86](https://github.com/copy/v86)'s architecture: Rust CPU core
compiled to WebAssembly, plain-JS device/browser layer.

**Status: boots Linux.** rv64gc interpreter (I/M/A/F/D/C + Zicsr + full
privileged arch with sv39/sv48 MMU) boots TinyEMU's stock Linux 4.15 +
buildroot image to an interactive busybox shell — natively (~50 Minsn/s)
and in the browser. User-mode emulation (qemu-user style) runs static
riscv64 musl binaries. JIT-to-wasm pipeline is proven end-to-end (v1).
See [DESIGN.md](DESIGN.md) for architecture and remaining work.

```sh
# boot Linux in the browser
web/get-images.sh
cargo build -p rv64-wasm --target wasm32-unknown-unknown --release
python3 -m http.server -d . 8000    # open http://localhost:8000/web/system.html

# boot Linux natively
cargo build --release -p rv64-system
target/release/rv64-boot web/images/bbl64.bin web/images/kernel-riscv64.bin web/images/root-riscv64.bin

# run a static riscv64 Linux binary (qemu-user style)
cargo run --release -p rv64-run -- <static-elf> [args...]
```

## Build & test

```sh
# core unit tests
cargo test -p rv64-core

# wasm module
cargo build -p rv64-wasm --target wasm32-unknown-unknown --release

# browser demo (needs the wasm build above)
python3 -m http.server -d . 8000   # then open http://localhost:8000/web/

# native TinyEMU oracle (differential testing)
make -C reference/tinyemu CONFIG_FS_NET= CONFIG_SDL= CONFIG_X86EMU= CONFIG_SLIRP=
```

## Layout

- `crates/rv64-core` — portable CPU core (`no_std`, generic over a `Bus` trait)
- `crates/rv64-wasm` — `extern "C"` wasm export surface (no wasm-bindgen)
- `web/` — JS loader + demo page
- `reference/tinyemu/` — vendored TinyEMU (MIT, Fabrice Bellard): spec map & test oracle

## License

MIT. `reference/tinyemu/` retains its own MIT license and copyright
(Fabrice Bellard); see `reference/README.md`.
