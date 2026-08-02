# Releasing rv64.js

This repository ships as source rather than as independently published crates
or an npm package. Every Rust crate sets `publish = false`, the crates use path
dependencies, and the web package is marked `private`; changing that policy
requires a deliberate manifest and release-process change.

## Release checklist

1. Start from a clean worktree and record the release commit.
2. Enter the reproducible environment with `nix develop`.
3. Ensure the modern-system kernel has been built at least once with
   `tests/virt-smoke/run.sh`.
4. Run the strict validation gate:

   ```sh
   REQUIRE_ALL=1 tests/run-all.sh
   ```

   CI covers formatting, Clippy, guest fixtures, debug/release workspace
   tests, the browser-module build, and self-contained JavaScript tests. The
   strict local gate remains required for QEMU, Spike, architecture-signature,
   image-dependent Wasm, and modern-system boot coverage.

5. Run `cargo clippy --workspace --all-targets` and review every warning.
   Existing warning debt is tracked as release debt; do not introduce new
   warnings silently.
6. Run `cargo fmt --all -- --check`. The repository has pre-existing format
   drift that must be resolved before this becomes a mandatory gate.
7. Verify the browser demo from a clean build and downloaded image set.
8. Update `README.md`, `ROADMAP.md`, and `CHANGELOG.md` for user-visible
   changes, then tag the validated commit.

## Required manual checks

- Boot the stock browser image to a shell.
- Boot the modern direct-boot Alpine image and run `apk update`.
- Exercise virtio block, console, 9p, and HTTP/HTTPS proxy paths.
- Confirm that no credentials, generated disk images, benchmark artifacts, or
  machine-specific paths are tracked.

Vendored code under `reference/tinyemu/` and `vendor/` retains its upstream
license and notices.

## Browser demo assets

The GitHub Pages site is code-only. Guest firmware, kernels, disks, and the
compiled Wasm core are published under the versioned `demo-images-v2` release
tag so large generated binaries never enter Git history.

Build and verify the complete asset set before creating or replacing that
release:

```sh
nix develop -c tools/build-demo-assets.sh
RV64_UNTIL='~ #' node examples/boot-linux.mjs fast
RV64_UNTIL=ALPINE_READY node examples/boot-linux.mjs modern
```

Upload every file in `target/demo-images-v2/`, including `SHA256SUMS`. The
fixed tag is part of the page's public configuration; use a new tag when an
asset change is not backward-compatible.
