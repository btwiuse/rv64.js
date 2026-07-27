# rv64.js patch

This is `rustls-pki-types` 1.15.1, vendored from its crates.io release under
its original MIT OR Apache-2.0 license. The original `.crate` SHA-256 is
`2f4925028c7eb5d1fcdaf196971378ed9d2c1c4efc7dc5d011256f76c99c0a96`.

Upstream's `web` feature implements `UnixTime::now()` through `web-time`,
which brings wasm-bindgen and externref-table imports into the final module.
rv64.js deliberately instantiates a plain `wasm32-unknown-unknown` module with
a small raw `env` import table and does not ship the wasm-bindgen runtime.

The local change keeps the `web` feature and public API intact, but gets Unix
milliseconds from the existing raw `host_unix_ms() -> f64` host import. Native
targets continue to use `std::time::SystemTime`.

When updating this crate, audit the release module's imports. It must contain
`host_unix_ms` and must not contain `__wbindgen`, `externref`, or `web_time`
imports.
