# rv64.js

A RISC-V (RV64) emulator for the browser — [TinyEMU](https://bellard.org/tinyemu/)'s
scope with [copy/v86](https://github.com/copy/v86)'s architecture: Rust CPU core
compiled to WebAssembly, plain-JS device/browser layer.

**Status: phase 1 — RV64I interpreter running end-to-end (Rust → wasm → JS).**
See [DESIGN.md](DESIGN.md) for the architecture and roadmap.

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
