# Changelog

All notable user-visible changes to rv64.js will be recorded here. The project
uses [Semantic Versioning](https://semver.org/) once a release is tagged.

## Unreleased

- RV64GC interpreter and WebAssembly JIT for user-mode and full-system use.
- TinyEMU-compatible Linux machine and modern OpenSBI virt machine.
- Virtio console, block, 9p, and network devices.
- Native and browser HTTP proxy paths, including HTTPS CONNECT interception.
- Stable JavaScript networking modes (`fetch`, `wisp`, `wsproxy`, `inbrowser`, `external`,
  and `none`), with the HTTP proxy enabled by default for Linux machines.
- Firmware-free direct Linux boot through emulator-provided SBI services.
- Alpine 3.24 release guest with automatic proxy/CA setup and a tested `apk`
  package-index path.
- Correct large HTTPS response delivery through the guest proxy; rustls output
  is drained incrementally instead of truncating at its plaintext buffer limit.
- Differential ISA, architecture-signature, lockstep, Wasm, and boot
  validation suites.
