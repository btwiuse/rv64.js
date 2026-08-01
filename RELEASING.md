# Releasing rv64.js

This repository currently ships as source rather than as independently
published crates or an npm package. The Rust crates use path dependencies and
the web package is marked `private`; changing either policy is a separate
release decision.

## Release checklist

1. Start from a clean worktree and record the release commit.
2. Enter the reproducible environment with `nix develop`.
3. Ensure the modern-system kernel has been built at least once with
   `tests/virt-smoke/run.sh`.
4. Run the strict validation gate:

   ```sh
   REQUIRE_ALL=1 tests/run-all.sh
   ```

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
- Boot the modern OpenSBI/Debian image.
- Exercise virtio block, console, 9p, and HTTP/HTTPS proxy paths.
- Confirm that no credentials, generated disk images, benchmark artifacts, or
  machine-specific paths are tracked.

Vendored code under `reference/tinyemu/` and `vendor/` retains its upstream
license and notices.
