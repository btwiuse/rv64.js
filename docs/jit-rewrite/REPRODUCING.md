# Reproducing the JIT Rewrite Gates

This document separates correctness/release verification from benchmark
measurement. Run commands from the repository root. The recorded delivery tree
was based on upstream commit
`96aa93896e7bb6fa561d1f977c9bf23cd909a100`.

## Prepare the environment and artifacts

The Nix development shell supplies the Rust/Wasm toolchain, RISC-V compilers,
QEMU, Spike, architecture-test tools, filesystem utilities, and Node used by the
strict matrix. The first image build downloads a SHA-256-pinned Alpine
minirootfs and realizes the kernel/OpenSBI derivations.

```sh
nix develop -c tests/vs-v86/build-kernels.sh target/bench
nix develop -c web/prepare-images.sh
```

Those commands produce:

- `target/bench/xbench/{alu,amo,rvbench,rvbench_fs}.rv64`;
- `target/bench/alpine-riscv64/`, including the real-corpus musl loader;
- `target/bench/alpine-riscv64.ext4`;
- `web/images/alpine/{Image,opensbi.bin,alpine.ext4}`.

The standalone gate needs Wasmtime 47 or newer. Put it on `PATH` or set
`WASMTIME` to its executable. Browser measurements similarly discover Chrome
and Firefox on `PATH`, or accept explicit `CHROME` and `FIREFOX` paths.

## Strict release matrix

`REQUIRE_ALL=1` converts every missing optional tool or artifact into a failure.
Preparing the images first also realizes `virt-kernel-fast`, so stage 8 cannot
silently skip.

```sh
nix develop -c env \
  ARTIFACTS="$PWD/target/bench" \
  REQUIRE_ALL=1 \
  WASMTIME=/path/to/wasmtime \
  tests/run-all.sh
```

Success ends with `ALL STAGES PASSED`. The eight stages cover workspace tests,
guest builds, QEMU differentials, `riscv-tests`, Spike lockstep,
`riscv-arch-test`, Wasm/JIT tests (including standalone and both modern boot
paths), and the modern OpenSBI/Linux Virt smoke guest.

The current matrix also runs the complete RV64GCV differential twice: 8,310
interpreter executions against QEMU and 8,310 repeated hot-JIT executions
against the interpreter, followed by focused vector fault and full-system
tests.

## RV64GCV JIT scorecard

Build the frozen stock-musl RV64GCV/i686 population from the pinned nbench
archive. The preparation step also realizes and verifies the population-owned
RVV-capable Linux kernel:

```sh
nix develop -c tests/vs-v86/prepare-interpreter-stock-nbench.sh \
  target/bench /path/to/nbench-byte-2.2.3.tar.gz
cargo build --release -p rv64-wasm --target wasm32-unknown-unknown
```

Run the authoritative JIT scorecard with the pinned copy/v86 checkout. Do not
set an artifact override: overrides are proof-only and make a measurement
ineligible.

```sh
ARTIFACTS=target/bench \
V86DIR=/path/to/pinned/v86 \
SCORECARD_V2_EXECUTION_MODE=jit \
SCORECARD_V2_INPUT_POPULATION=scorecard-v2-rv64gcv-v1 \
SCORECARD_V2_OUTPUT=target/bench/rv64gcv-jit-authoritative-v1 \
AUTHORITATIVE=1 REPS=3 \
node tests/vs-v86/scorecard-v2.mjs
```

The unchanged scalar three-way regression scorecard uses
`SCORECARD_V2_INPUT_POPULATION=scorecard-v2-modern` and the default rewrite,
legacy, and v86 sides. See
[RVV_JIT_RESULT.md](RVV_JIT_RESULT.md) for the measured artifact and report
hashes.

The async T2 publication regression can be stressed independently with the
older Node supplied by the Nix shell:

```sh
nix develop -c bash -lc \
  'for run_index in 1 2 3 4 5; do node tests/jit-t2-multientry-differential.mjs || exit 1; done'
```

Focused modern and randomized gates are:

```sh
nix develop -c node tests/jit-t2-atomic-random.mjs
nix develop -c node tests/jit-modern-linux.mjs
nix develop -c tests/virt-smoke/run.sh
```

## Frozen synthetic backend measurements

The Node runner regenerates the exact 11-module corpus before timing. Each
reported sample is a fresh process; paired variant order alternates.

```sh
node tests/jit-backend-corpus-bench.mjs \
  --samples=7 \
  --output=target/jit-backend-node-report.json
```

The browser runner uses a fresh process and profile per sample, serves modules
with `Cache-Control: no-store`, and supports Chromium CDP plus current Firefox
WebDriver BiDi.

```sh
CHROME=/path/to/chrome \
FIREFOX=/path/to/firefox \
node tests/jit-backend-browser-bench.mjs \
  --engines=chrome,firefox \
  --samples=7 \
  --output=target/jit-backend-browser-report.json
```

Do not run benchmark workers concurrently on the same host. Preserve every raw
trial; do not replace the reported median with best-of-N.

## Real compiler-generated region corpus

The default generator inputs are the just-built RV64 benchmark and the Alpine
musl loader. It emits five backends at seven page/leader geometries.

```sh
cargo run --release -q -p rv64-dbt \
  --example emit_real_region_corpus -- \
  target/jit-real-region-corpus
```

Custom ELF64 little-endian RISC-V inputs use `label=path` arguments after the
output directory:

```sh
cargo run --release -q -p rv64-dbt \
  --example emit_real_region_corpus -- \
  target/jit-real-region-corpus \
  app=/path/to/app.rv64 loader=/path/to/ld-musl-riscv64.so.1
```

Frontend reports regenerate the corpus unless `--skip-generate` is supplied:

```sh
node tests/jit-real-region-bench.mjs \
  --samples=8 \
  --output=target/jit-real-region-node-report.json

CHROME=/path/to/chrome \
FIREFOX=/path/to/firefox \
node tests/jit-real-region-browser-bench.mjs \
  --engines=chrome,firefox \
  --samples=7 \
  --output=target/jit-real-region-browser-report.json
```

The report and `manifest.tsv` contain source/module SHA-256 hashes. A result is
comparable only when those hashes and the engine versions match.

## Standalone Wasmtime

This executes the 11 synthetic modules through generated imports/drivers and
then AOT-compiles every eager real region in a fresh CLI process:

```sh
WASMTIME=/path/to/wasmtime \
node tests/jit-standalone-wasmtime.mjs \
  --real-corpus=target/jit-real-region-corpus \
  --output=target/jit-standalone-wasmtime-report.json
```

The ordinary strict suite executes the 11 exact-state modules even when the
optional `--real-corpus` AOT sweep is omitted.

## Live modern-Linux region policy

### Async page admission policy

Build the deterministic guest and current Wasm first:

```sh
(cd guests/syscompute && cargo build --release)
cargo build --release --target wasm32-unknown-unknown -p rv64-wasm
```

Capture mapping traces and run the offline policy simulator:

```sh
node tests/jit-policy-trace.mjs --mode=direct --phase=boot \
  --output=target/jit-policy-traces/direct-boot.json
node tests/jit-policy-trace.mjs --mode=opensbi --phase=boot \
  --output=target/jit-policy-traces/opensbi-boot.json
node tests/jit-policy-sim.mjs \
  --output=target/jit-policy-traces/simulation.json \
  target/jit-policy-traces/direct-boot.json \
  target/jit-policy-traces/opensbi-boot.json
```

Calibrate the exact direct interpreter. Each sample is a new Node process:

```sh
node tests/jit-policy-calibrate.mjs --samples=3 \
  --output=target/jit-policy-traces/calibration-medium-node.json
```

Sweep thresholds with one fresh process/Linux VM per point. No workers should
run concurrently on the benchmark host:

```sh
node tests/jit-page-policy-sweep.mjs \
  --samples=3 --quantum=1024 \
  --thresholds=262144,524288,1048576,2097152 \
  --workloads=alu1,alu5,mix20 \
  --output=target/jit-policy-traces/page-policy-threshold-sweep-q1024.json
```

The real browser A/B launches a fresh Chromium process and profile for every
interpreter/policy sample and alternates process order:

```sh
CHROME=/path/to/chrome node tests/jit-page-policy-browser-bench.mjs \
  --samples=3 --threshold=1048576 --quantum=1024 \
  --workloads=alu1,alu5,mix20 \
  --output=target/jit-policy-traces/page-policy-browser-chrome.json
```

Gate the selected policy through both modern boot paths and all deterministic
payload sizes:

```sh
node tests/jit-page-policy-modern.mjs --mode=direct \
  --threshold=1048576 --quantum=1024 \
  --workloads=alu1,alu5,alu,mix20,mix
node tests/jit-page-policy-modern.mjs --mode=opensbi \
  --threshold=1048576 --quantum=1024 \
  --workloads=alu1,alu5,alu,mix20,mix
```

The generated-module callback is useful for structural assertions but should
not retain bytes in a headline timing run. `wasmCompileElapsedMs` is summed
promise elapsed time, including scheduling/contention; it is not a direct
measurement of compiler CPU consumption.

### Earlier region geometry policy

Leader caps are measured in alternating order, one fresh Node process and VM
per cap/sample. Include 32 explicitly; the script's generic diagnostic default
starts at 64.

```sh
node tests/jit-modern-region-policy.mjs \
  --caps=32,64,128,256,512 \
  --page-cap=3 \
  --samples=3 \
  --output=target/jit-modern-region-policy-report.json
```

Isolate page count while holding the selected leader cap constant:

```sh
node tests/jit-modern-region-policy.mjs \
  --caps=32 --page-cap=1 --samples=3 \
  --output=target/jit-modern-region-pages1-report.json

node tests/jit-modern-region-policy.mjs \
  --caps=32 --page-cap=2 --samples=3 \
  --output=target/jit-modern-region-pages2-report.json

node tests/jit-modern-region-policy.mjs \
  --caps=32 --page-cap=3 --samples=3 \
  --output=target/jit-modern-region-pages3-report.json
```

These are production-mode measurements. Engine diagnostic tier flags can help
explain a result but must not replace the fresh default-engine reports.

## Current modern three-way scorecard and R054 gate

The accepted rewrite artifact is
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
Rebuild it and verify the hash before a comparison:

```sh
nix develop -c cargo build --release -p rv64-wasm \
  --target wasm32-unknown-unknown
sha256sum target/wasm32-unknown-unknown/release/rv64_wasm.wasm
```

R054's frozen engine-shape gate regenerates both modules and measures seven
alternating fresh processes. The output path must not name a retained report
unless that report is intentionally being reproduced elsewhere:

```sh
node tests/jit-interpreter-fused-memory-corpus-bench.mjs \
  --samples=7 \
  --output=target/bench/r054-interpreter-fused-memory-corpus-repro.json
```

The production Linux A/B compares the diagnostic switch in the same Wasm
artifact, one fresh guest per leg. Use a new output directory:

```sh
ARTIFACTS="$PWD/target/bench" \
ROWS=boot,compile REPS=5 \
CONTROL_CONFIG='{"SCORECARD_V2_INTERPRETER_FUSED_MEMORY":"0"}' \
CANDIDATE_CONFIG='{"SCORECARD_V2_INTERPRETER_FUSED_MEMORY":"1"}' \
SCORECARD_V2_OUTPUT="$PWD/target/bench/r054-fused-memory-repro-ab" \
taskset -c 8-15 node tests/vs-v86/scorecard-v2-config-ab.mjs
```

Only after focused promotion gates pass, run the untouched authoritative
matrix. Defaults select all three sides and all 13 rows; `AUTHORITATIVE=1`
enforces that scope, odd `REPS>=3`, and v86 generated-execution proof:

```sh
ARTIFACTS="$PWD/target/bench" \
REPS=3 AUTHORITATIVE=1 \
SCORECARD_V2_OUTPUT="$PWD/target/bench/scorecard-v2-authoritative-repro" \
taskset -c 8-15 node tests/vs-v86/scorecard-v2.mjs
```

Accept only a report with `authoritative=true`, `measurementValid=true`, an
empty `problems` array, exact modern guest/input hashes, and the intended Wasm
hash. If any leg or sample-spread gate fails, retain the complete invalid run
and preregister a full-matrix rerun; never replace one sample.

## Previous JIT versus rewrite

The comparison builds the previous implementation from the same commit in an
isolated target directory. Do not inspect or copy its compiler source if the
clean-room boundary matters. Start with a nonexistent destination rather than
overlaying an earlier archive:

```sh
comparison_commit=96aa93896e7bb6fa561d1f977c9bf23cd909a100
comparison_repo="$PWD"
comparison_root="$PWD/target/jit-old-new/previous"
test ! -e "$comparison_root"
mkdir -p "$comparison_root"
git archive "$comparison_commit" | tar -x -C "$comparison_root"

nix develop "$comparison_repo#" -c cargo build \
  --release -p rv64-wasm --target wasm32-unknown-unknown

(
  cd "$comparison_root"
  nix develop "$comparison_repo#" -c cargo build \
    --release -p rv64-wasm --target wasm32-unknown-unknown
)
```

Prepare the current guest artifacts and images as described at the top of this
document. The accepted headline command uses nine paired samples and omits
capture work so its host-stability decision covers only emulator trials:

```sh
node tests/jit-old-new-bench.mjs \
  --samples=9 \
  --capture-workloads= \
  --output=target/jit-old-new-headline.json
```

On Linux, constrain the complete process tree to a fixed multi-core set as a
scheduler/affinity sensitivity check (select an otherwise idle set appropriate
to the host):

```sh
taskset -c 8-15 node tests/jit-old-new-bench.mjs \
  --samples=9 --workloads=user-alu,user-mixed --capture-workloads= \
  --output=target/jit-old-new-pinned-user.json
```

Capture is an untimed, separate pass. It records every dynamically generated
module and then compiles/instantiates the exact bytes in fresh processes:

```sh
node tests/jit-old-new-bench.mjs \
  --samples=7 \
  --frozen-samples=5 \
  --output=target/jit-old-new-comparison.json
```

Reject a report when `validity.valid` is false; do not relax the recorded 1.25x
CPU-probe threshold after observing data. The console and JSON define speedup
as `old_time / new_time`, so a value greater than one favors the rewrite.

V8 tier diagnostics propagate the selected engine flag to every child process:

```sh
node --liftoff-only tests/jit-old-new-bench.mjs \
  --samples=5 --workloads=user-alu,user-mixed --capture-workloads= \
  --output=target/jit-old-new-liftoff.json

node --no-liftoff tests/jit-old-new-bench.mjs \
  --samples=5 --workloads=user-alu,user-mixed --capture-workloads= \
  --output=target/jit-old-new-turbofan.json

node tests/jit-old-new-bench.mjs \
  --samples=5 --workloads=user-alu --capture-workloads= \
  --user-quantum=1000000 \
  --output=target/jit-old-new-alu-reentry.json
```

Those flags are explanatory only. The untouched-engine report remains the
headline. Locally installed Node-compatible runtimes can provide supporting
cross-engine diagnostics:

```sh
deno run -A tests/jit-old-new-bench.mjs \
  --samples=3 --workloads=user-alu,user-mixed --capture-workloads= \
  --output=target/jit-old-new-deno-pilot.json

bun tests/jit-old-new-bench.mjs \
  --samples=3 --workloads=user-alu,user-mixed --capture-workloads= \
  --output=target/jit-old-new-bun-pilot.json
```

The interpretation and recorded artifact hashes are in
[COMPARISON.md](COMPARISON.md).

## Dedicated compiler Worker opportunity gate

R057 uses exact generated modules from accepted R054 and is diagnostic-only.
Generate the deterministic foreground Wasm if it is absent, then capture Boot
and Compile through the worker directly. Capture is downstream of lifecycle
timing and the result must report `measurementEligible: false`:

```sh
cargo run --release -q -p rv64-dbt \
  --example emit_monomorphic_reentry_corpus -- \
  target/jit-monomorphic-reentry-corpus

ARTIFACTS=/home/darren/src/arm64.js/target/bench \
SCORECARD_V2_CAPTURE_JIT_MODULES="$PWD/target/bench/r057-current-boot-modules" \
  taskset -c 8-15 node --max-old-space-size=4096 \
  tests/vs-v86/scorecard-v2-worker.mjs rewrite boot

ARTIFACTS=/home/darren/src/arm64.js/target/bench \
SCORECARD_V2_CAPTURE_JIT_MODULES="$PWD/target/bench/r057-current-compile-modules" \
  taskset -c 8-15 node --max-old-space-size=4096 \
  tests/vs-v86/scorecard-v2-worker.mjs rewrite compile
```

The frozen harness verifies the manifest and every module hash before timing,
acquires the ordinary scorecard lock, and alternates seven fresh-process pairs:

```sh
ARTIFACTS=/home/darren/src/arm64.js/target/bench \
  taskset -c 8-15 node tests/jit-compiler-worker-corpus-bench.mjs --samples=7
```

It writes `target/bench/r057-compiler-worker-corpus.json` with exclusive-create
semantics. The exact corpus hashes, fixed foreground work, thresholds, and
negative result are in
[R057_DEDICATED_COMPILER_WORKER_PROTOCOL.md](R057_DEDICATED_COMPILER_WORKER_PROTOCOL.md).

## Compact step-outcome opportunity gate

R058 is a deterministic engine-shape test; it does not edit or rebuild the
production runtime. The harness emits both modules twice, validates exact
bytes and state, verifies one direct call through static disassembly and V8's
non-inlining trace, then holds the scorecard lock for seven alternating fresh
process pairs:

```sh
ARTIFACTS="$PWD/target/bench" \
  taskset -c 8-15 node \
  tests/jit-compact-step-outcome-corpus-bench.mjs --samples=7
```

It exclusively creates
`target/bench/r058-compact-step-outcome-corpus.json` and the static/optimizing
shape logs under `target/bench/r058-compact-step-outcome-shape/`. Do not delete
the failed warm leg, replace a pair, add yields, force a tier, or rerun into the
same output. The exact 1.20x/1.15 gate, broad-cost bound, immutable result, and
post-decision diagnostic interpretation are in
[R058_COMPACT_STEP_OUTCOME_PROTOCOL.md](R058_COMPACT_STEP_OUTCOME_PROTOCOL.md).

## Flattened RV64C dispatch opportunity gate

R059 emits an architecture-balanced complete 24-family selector twice, checks
four nested versus one flat `br_table`, proves both drivers reach TurboFan
after the same fixed-yield prewarm, then holds the scorecard lock for seven
alternating fresh-process pairs:

```sh
ARTIFACTS="$PWD/target/bench" \
  taskset -c 8-15 node \
  tests/jit-flat-rvc-dispatch-corpus-bench.mjs --samples=7
```

The harness exclusively creates
`target/bench/r059-flat-rvc-dispatch-corpus.json` and shape logs under
`target/bench/r059-flat-rvc-dispatch-shape/`. Preserve the failed warm-spread
leg; do not replace it, change the fixed yields, or rerun into the same output.
The positive local throughput and negative aggregate admission decision are
both part of
[R059_FLAT_RVC_DISPATCH_PROTOCOL.md](R059_FLAT_RVC_DISPATCH_PROTOCOL.md).

## WANIX matched-root browser smoke

Build the two architecture-matched namespace archives and the two RV64 runtime
archives used by the three-way page:

```sh
make -C integrations/wanix comparison
```

Install `wanix-linux-{x86,rv64}.tgz`, `rv64-{legacy,jit}.tgz`, and
`v86-rv64-three-way.html` as described in `integrations/wanix/README.md`, then
serve the WANIX tree. The focused smoke starts only copy/v86 in a fresh Chrome
process/profile, checks the guest identity, and runs `/shared/bench.py`:

```sh
WANIX_URL=http://127.0.0.1:8765/examples/v86-rv64-three-way.html \
  node tests/wanix-v86-matched-smoke.mjs
```

This is a functional gate for kernel, console, Python/native-extension, and 9P
paths. Do not interpret its single run as a controlled emulator comparison.

## WANIX matched-root parity measurement

Rebuild and deploy `dist/rv64-jit.tgz` and the comparison HTML before measuring.
Confirm that the served archive equals the build archive and contains the
current release Wasm:

```sh
make -C integrations/wanix dist/rv64-jit.tgz
sha256sum integrations/wanix/dist/rv64-jit.tgz /path/to/site/rv64/rv64-jit.tgz
tar -xOzf integrations/wanix/dist/rv64-jit.tgz rv64_wasm.wasm | sha256sum
sha256sum target/wasm32-unknown-unknown/release/rv64_wasm.wasm
```

Use a hash-named copy of the page and RV64 archive for a retained performance
claim. The harness hashes the page and all four bound archives before and after
every leg and the analyzer requires one identical snapshot, but immutable names
also prevent another deployment from invalidating a long experiment.

Run on an otherwise idle host. Select an isolated CPU set after inspecting host
topology and background activity; the accepted runs used CPUs 8–15. The fixed
runner writes `protocol.json` before the first sample, alternates VM order,
holds the global benchmark lock for the whole experiment, and creates a new
browser process, profile, and guest for every leg. The results directory must
not already exist:

```sh
export WANIX_URL=http://127.0.0.1:8765/examples/v86-rv64-three-way-ARTIFACT.html
export WANIX_CPU_AFFINITY=8-15
results_dir=target/jit-policy-traces/wanix-parity-chrome

node tests/run-wanix-pairs.mjs "$results_dir" \
  --pairs=5 --max-slowdown=0.10

node tests/analyze-wanix-pairs.mjs "$results_dir" \
  --max-slowdown=0.10 --output="$results_dir/analysis.json"
```

The pair, not a repetition inside one guest, is the resampling unit. The runner
removes `WANIX_JIT_CONFIG`, `WANIX_JIT_POLICY`, and `WANIX_JIT_PROFILE` from
every child and stops on the first failed leg; it never adds a replacement
sample after seeing a result.

The harness discovers Chrome by default. To reproduce the Microsoft Edge leg,
set `CHROME` on the complete fixed runner and use a new result directory; no
other command or policy setting changes:

```sh
results_dir=target/jit-policy-traces/wanix-parity-edge
CHROME=/path/to/microsoft-edge-stable \
  node tests/run-wanix-pairs.mjs "$results_dir" \
    --pairs=5 --max-slowdown=0.10
node tests/analyze-wanix-pairs.mjs "$results_dir" \
  --max-slowdown=0.10 --output="$results_dir/analysis.json"
```

The analyzer rejects missing pairs, mixed browser engines, guest-identity or
checksum failures, JIT overrides, unexpected shipped page/leader/control/tail-
chain defaults, zero tail-chain activity, generated coverage below 90%,
protocol chronology or overlap violations, changed artifacts/CPU affinity, or
a phase whose paired geometric-mean 95% upper bound exceeds 1.10. It also
reports every raw ratio, ratio of medians, paired median, and exact paired-
bootstrap intervals. Do not discard an outlier or replace this report with
best-of-N.

For a quick stability check inside one fresh guest, not the headline
fresh-process comparison, use three repetitions with the same phase barrier:

```sh
env -u WANIX_JIT_CONFIG -u WANIX_JIT_POLICY \
  WANIX_VM=rv64-jit WANIX_BENCH_REPETITIONS=3 \
  WANIX_BENCH_PHASE_SYNC=1 WANIX_SUMMARY_ONLY=1 \
  node tests/wanix-v86-matched-smoke.mjs
```

## Statistical fields

All new reports use `tests/statistics.mjs`. For each metric they retain raw
values, minimum, conventional median (the mean of the two middle values for an
even sample count), p95, maximum, and a deterministic fixed-seed 4,096-resample
bootstrap 95% interval for the median. Paired ratios are formed within the same
fresh process/profile sample before being summarized.
