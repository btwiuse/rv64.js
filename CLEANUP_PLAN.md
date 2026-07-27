# Repository cleanup plan

Recorded: 2026-07-25. Status refreshed: 2026-07-27.

This document captures the current repository-organization audit and a proposed
cleanup sequence. The goal is to simplify development and make validation
trustworthy without losing test coverage or benchmark history.

## Progress since the audit

Several correctness holes found by this audit have already been closed:

- `tests/run-all.sh` now preserves failures through the lockstep and
  architecture-test pipelines instead of allowing `tail` to hide them, and
  the missing ISA/lockstep/architecture stages run again.
- `tests/vs-v86/build-kernels.sh` now produces `xbench/amo.rv64`, and the AMO
  differential test executes when the prepared artifact directory is supplied.
- The HTTP relay has direct protocol coverage and runs in the Wasm suite.

The structural work below remains open: strict test tiers and explicit
PASS/FAIL/SKIP summaries, pinned and centralized assets, shared JavaScript
harnesses, benchmark consolidation, CI, and documentation rationalization.

## Current assessment

The production-code layout is generally sound:

- `crates/` separates the CPU core, JIT, Linux user-mode support, runners,
  system emulation, and wasm interface.
- `guests/` contains RISC-V guest programs.
- `web/` contains the browser runtime and demos.
- `reference/tinyemu/` is an intentional reference implementation.

Most of the organizational debt is in test, benchmark, artifact, and project
history management.

### Test and setup problems

- `tests/run-all.sh` still reports `ALL STAGES PASSED` when all non-skipped
  stages pass, even if important checks did not run. Pipeline failures are no
  longer hidden, but missing prerequisites remain permissive skips.
- Its comment claiming 23 Cargo tests is stale and should not embed a count
  that changes whenever coverage grows.
- Test prerequisites and artifacts are produced through several unrelated
  paths.
- `riscv-arch-test` is pinned to version 3.9.1, while `riscv-tests` is cloned
  from the current default branch and can drift.
- The external test repositories are cloned inside `tests/`, producing
  confusing nested Git repositories.
- Heavy tests such as `tests/virt-proxy` are manual and easy to miss.
- Some integration tests return successfully when their images or guest
  binaries are absent, so Cargo alone cannot prove that they actually ran.
- There is no root CI workflow, formatting check, or Clippy stage.
- A repository-wide `cargo fmt --all -- --check` currently reports formatting
  drift in existing core/JIT files; establish and commit a deliberate baseline
  before making that command a required gate.

### Benchmark problems

- Benchmark setup, workload construction, execution, comparison, reporting,
  and historical analysis are mixed together.
- Multiple JavaScript runners duplicate Linux boot, shell synchronization,
  binary injection, JIT/interpreter selection, marker timing, and checksum
  logic.
- Several benchmark harnesses overlap or appear orphaned and need review:
  - `tests/ab-syscompute.mjs`
  - `tests/bench-sys.mjs`
  - `tests/vs-v86/bench-jit-fast.mjs`
  - `tests/vs-v86/xbench-user.mjs`
  - `tests/vs-v86/v86-boot.mjs`
  - `tests/vs-v86/tstamp.py`
- Some scripts hard-code `/home/darren/src/arm64.js` instead of deriving the
  repository root.
- Correctness validation and performance measurement do not have clearly
  separated canonical entry points.

### Documentation problems

- `ROADMAP.md`, `ISSUES.md`, `tests/BASELINE.md`, `tests/VALIDATION.md`, and
  `HANDOFF.md` overlap as plans, status reports, investigation logs, and
  historical records.
- The current README points at the main test runner but does not explain its
  skip semantics or enumerate the manual/nightly checks.

## Desired developer interface

Provide one stable developer-facing entry point. A small shell dispatcher fits
this repository because it must coordinate Rust, Node, Python, Nix, external
compilers, Linux images, and third-party emulators.

Example interface:

```sh
./dev setup
./dev assets ensure test
./dev assets ensure bench

./dev test fast
./dev test full
./dev test nightly
./dev test one rv64-core cpu::tests::addi_add_sub

./dev bench smoke
./dev bench full
./dev bench --only alu,mixed
```

The existing scripts can remain as internal implementations initially. Users
should not need to understand their locations, environment variables, or setup
ordering.

## Suggested target layout

```text
crates/                         Rust production code and Cargo tests
guests/                         RISC-V test and benchmark programs
web/                            Browser runtime and demos

tools/
  dev                           Canonical command dispatcher
  test/
    conformance.sh
    differential.sh
    wasm.mjs
    system/
  js/
    rv64-harness.mjs
    linux-console.mjs
    benchmark-runner.mjs
  bench/
    scorecard.mjs
    workloads/
    v86/
      internal/

docs/
  testing.md
  benchmarking.md
  architecture.md
  history/

target/test-assets/
  manifest.json
  sources/                      Pinned external test sources
  images/
  guests/
  benchmarks/
  reports/
```

Rust unit tests should remain next to the source they exercise, and Cargo
integration tests should remain under their crates. The proposed movement is
for orchestration, standalone harnesses, generated artifacts, and documentation.

## Migration plan

### Phase 1: make the suite honest

This should be the first, small cleanup change. Avoid moving or deleting test
harnesses in this phase.

1. Add the `./dev` dispatcher.
2. Define explicit validation tiers:
   - `fast`: formatting, Clippy, Rust tests, wasm build, and a short
     deterministic JIT differential run.
   - `full`: all ordinary correctness checks, with missing prerequisites
     treated as failures.
   - `nightly`: expensive kernel realization, Debian proxy integration, and
     other heavyweight checks.
3. Permit skips only through an explicit option such as `--allow-skip`.
4. Print a final per-stage table containing `PASS`, `FAIL`, or `SKIP`.
5. Keep stage numbering accurate and remove hard-coded test counts.
6. Add:

   ```sh
   cargo fmt --check
   cargo clippy --workspace --all-targets -- -D warnings
   ```

7. Include `tests/virt-proxy` in the known inventory, classified as nightly.
8. Add `docs/testing.md` as the authoritative test matrix.

Acceptance criteria:

- `./dev test full` cannot report success if a required stage skipped.
- `./dev test fast` runs from a clean checkout after documented setup.
- Every test tier prints exactly which checks ran.

### Phase 2: centralize and pin assets

Implement:

```sh
./dev assets ensure test
./dev assets ensure bench
```

The asset manager should:

1. Fetch the TinyEMU Linux images and verify recorded hashes.
2. Fetch exact revisions of `riscv-tests` and `riscv-arch-test`.
3. Initialize and verify the `riscv-tests/env` dependency.
4. Build all RISC-V guest binaries.
5. Build every required benchmark binary, including `amo.rv64`.
6. Place generated and downloaded material under `target/test-assets/`.
7. Write a manifest containing:
   - source URLs and revisions;
   - image hashes;
   - compiler and tool versions;
   - build flags;
   - artifact paths and hashes.

The source revisions should live in a small tracked lock file or pinned Nix
definitions. Fresh validation runs must not depend on the current upstream
default branch.

Acceptance criteria:

- No external Git repository is created inside tracked source directories.
- A fresh asset preparation produces the same source revisions and reports the
  same hashes.
- Every artifact-dependent test has a declared producer.

### Phase 3: share JavaScript harness code

Extract the repeated machinery currently spread across smoke tests and
benchmarks:

- locating the repository and wasm module;
- loading TinyEMU images;
- creating and configuring an RV64 VM;
- booting Linux and waiting for a real output marker;
- disabling terminal echo safely;
- injecting binaries into the guest;
- selecting interpreter or JIT mode;
- timing serial-output marker pairs;
- enforcing timeouts;
- checking exit codes, output, and checksums;
- collecting JIT statistics.

Possible modules:

```text
tools/js/rv64-harness.mjs
tools/js/linux-console.mjs
tools/js/artifacts.mjs
tools/js/benchmark-runner.mjs
```

All repository paths should be derived from `import.meta.url`; remove absolute
path constants.

Acceptance criteria:

- Correctness and benchmark runners share the same boot and console protocol.
- No active script contains `/home/darren/src/arm64.js`.
- Marker matching cannot accidentally succeed on terminal input echo.

### Phase 4: consolidate benchmarks

Make the system-mode scorecard the canonical performance suite and add workload
filters:

```sh
./dev bench full
./dev bench --only alu,mixed
./dev bench --only boot
./dev bench --only python
./dev bench --only compile
./dev bench --only nbench
```

The canonical scorecard should:

- separate correctness verdicts from performance verdicts;
- require all requested workloads and artifacts;
- record repository revision, wasm hash, configuration, host information, and
  workload hashes;
- use repeated/interleaved samples where appropriate;
- write machine-readable JSON and a human-readable summary;
- distinguish incomplete, invalid, failed, and successful runs.

Audit the apparently redundant or unreferenced scripts listed earlier. For each
one, either:

- migrate unique coverage into the canonical runner;
- document it as an intentional specialized tool;
- move it to `docs/history/` with its results; or
- remove it after confirming it carries no unique coverage.

Keep required `v86-*.mjs` helpers under an internal directory rather than
presenting them as top-level benchmark entry points.

Acceptance criteria:

- There is one documented full performance command.
- Focused commands use the same workload implementations as the full scorecard.
- No historical benchmark is mistaken for a current correctness gate.

### Phase 5: add tiered CI

Add three workflows.

Pull-request workflow:

- `cargo fmt --check`;
- Clippy;
- Rust workspace tests;
- wasm build;
- short deterministic JIT differential run.

Nightly or manual correctness workflow:

- QEMU differential;
- official `riscv-tests`;
- Spike lockstep;
- architecture signature comparison;
- full wasm smoke;
- modern `virt-smoke`;
- Debian `virt-proxy`.

Manual performance workflow:

- build or restore pinned benchmark assets;
- run the scorecard;
- upload JSON and Markdown reports;
- always enforce workload checksums;
- avoid gating on raw timing until a controlled runner and stable thresholds
  exist.

Acceptance criteria:

- Fast CI provides useful feedback without large downloads or multi-minute
  guest boots.
- The scheduled suite demonstrates that all expensive validation actually ran.
- Performance reports are retained as artifacts with provenance.

### Phase 6: rationalize documentation

Assign one purpose to each document:

- `README.md`: project overview and minimal quick start.
- `docs/testing.md`: complete validation matrix, setup, tiers, filters, and
  troubleshooting.
- `docs/benchmarking.md`: current workload definitions, measurement rules, and
  scorecard usage.
- `DESIGN.md` or `docs/architecture.md`: current technical architecture.
- `ROADMAP.md`: future work only.
- `ISSUES.md`: unresolved technical issues only.
- `docs/history/`: completed investigations, old baselines, performance
  narratives, and superseded decisions.

Decide whether `HANDOFF.md` is:

- a temporary local working document;
- a generated status summary;
- or material that should be folded into active issues and the roadmap.

Avoid maintaining it indefinitely as a second roadmap.

Acceptance criteria:

- A new contributor can discover every validation tier from `README.md`.
- Current instructions do not require reading historical benchmark narratives.
- Completed investigations remain available without obscuring active work.

## Proposed cleanup changesets

Keep changes reviewable and behavior-preserving:

1. **Test infrastructure honesty**
   - Add `./dev`.
   - Add strict tier semantics and summaries.
   - Add formatting and Clippy.
   - Add `docs/testing.md`.
   - Require the now-produced AMO artifact in full mode.

2. **Pinned assets**
   - Introduce the asset manifest and manager.
   - Move external sources and generated artifacts under `target/test-assets`.
   - Pin all external source revisions.

3. **Shared harnesses**
   - Extract common JavaScript boot, console, injection, timeout, and result
     logic.
   - Remove hard-coded paths.

4. **Benchmark consolidation**
   - Establish the canonical scorecard and workload filters.
   - Archive or remove redundant runners after verifying coverage.

5. **CI and documentation**
   - Add fast, nightly, and performance workflows.
   - Split current operational documentation from history.

Before each changeset, run and record the currently applicable full suite.
Afterward, demonstrate equivalent or stronger coverage. File movement and
runner consolidation should not be accepted on the basis of exit status alone;
the before/after inventory must show which tests actually executed.

## Immediate next step

The best first implementation is the **test infrastructure honesty** changeset.
It creates a trustworthy baseline for every later move:

1. introduce `./dev`;
2. wrap the existing runners without relocating them;
3. make full-mode skips fatal;
4. add formatting and Clippy checks;
5. add a structured result summary;
6. document all fast, full, nightly, and manual checks.

Once that is in place, script consolidation and directory cleanup can proceed
with much lower risk.
