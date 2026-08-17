# Releasing rv64.js

This repository ships as source rather than as independently published crates
or an npm package. Every Rust crate sets `publish = false`, the crates use path
dependencies, and the web package is marked `private`; changing that policy
requires a deliberate manifest and release-process change.

## Release checklist

1. Start from a clean worktree and record the release commit.
2. Enter the reproducible environment with `nix develop`.
3. Ensure the Alpine kernel has been built at least once with
   `tests/virt-smoke/run.sh`.
4. Run the strict validation gate:

   ```sh
   REQUIRE_ALL=1 tests/run-all.sh
   ```

   CI covers formatting, Clippy, guest fixtures, debug/release workspace
   tests, the browser-module build, and self-contained JavaScript tests. The
   strict local gate remains required for QEMU, Spike, architecture-signature,
   image-dependent Wasm, and Alpine-system boot coverage.

5. Run `cargo clippy --workspace --all-targets` and review every warning.
   Existing warning debt is tracked as release debt; do not introduce new
   warnings silently.
6. Run `cargo fmt --all -- --check`. The repository has pre-existing format
   drift that must be resolved before this becomes a mandatory gate.
7. Verify the browser demo from a clean build and downloaded image set.
8. Update `README.md`, `ROADMAP.md`, and `CHANGELOG.md` for user-visible
   changes, then tag the validated commit.

## Required manual checks

- Boot the direct-boot Alpine image to a shell and run `apk update`.
- Exercise virtio block, console, 9p, and HTTP/HTTPS proxy paths.
- Confirm that no credentials, generated disk images, benchmark artifacts, or
  machine-specific paths are tracked.

Vendored code under `reference/tinyemu/` and `vendor/` retains its upstream
license and notices.

## Browser demo assets

The GitHub Pages site is code-only. Guest firmware, kernels, disks, and the
compiled Wasm core are published under versioned `demo-images-vN` release tags
so large generated binaries never enter Git history. Because the runtime Wasm
is part of this bundle, a Wasm-only source change still requires a new demo
asset version even when the guest kernel and disk inputs are unchanged.

Demo assets are built and published only by GitHub Actions from a pushed tag.
Do not upload locally built images. After the release-source commit is on
`main`, create and push the next versioned tag:

```sh
git tag demo-images-vN
git push origin demo-images-vN
```

The `Demo images release` workflow builds the kernel, Alpine disk, and Wasm
module in a clean runner; verifies checksums; boots Alpine and runs a real
`apk update`; and creates the GitHub Release for that exact tag. A failed build
or validation does not create a release.

Tags and releases are immutable inputs to the Pages site. Never move or replace
an existing `demo-images-vN` tag. Use a new version when any asset changes.
After the workflow has successfully published the new release, update
`web/site-config.js` to consume its immutable URL; do not point Pages at an
asset version that does not exist yet.

The safe publication order is: merge the validated source commit to `main`,
create the next `demo-images-vN` tag, wait for its validated release, update
the Pages asset pin, and then create the intended `vN.N.N` library tag from the
merged release source. Never publish either release from an unmerged review
branch.
