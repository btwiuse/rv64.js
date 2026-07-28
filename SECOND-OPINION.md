# Second opinion requested: which rv64.js JIT commit is actually the best?

> **Status:** this is the historical evaluation brief. The benchmark harness
> was repaired after the review; use
> [`tests/vs-v86/METHODOLOGY.md`](tests/vs-v86/METHODOLOGY.md) for current
> testing and promotion rules. In particular, wall rows now run in isolated
> fresh processes, repeat symmetrically on both emulators, alternate paired
> order, and retain raw trials.
> Consolidated outcomes, rejected designs, current priorities, and the
> append-only experiment ledger now live in
> [`PERFORMANCE_PROGRESS.md`](PERFORMANCE_PROGRESS.md).

You are being asked for an INDEPENDENT evaluation. A long session (2026-07-26/27,
~14 hours, 23 commits) tried to win a 13-row benchmark against copy/v86 and
plateaued at 10/13. The author of that session (an AI agent) suspects some of its
own conclusions are contaminated by measurement noise and by at least one
confirmed comparison error. Your job is to re-measure the candidates and say
which tree is genuinely best — including "the pre-session baseline was fine and
most of this work was noise", if that is what the data says.

Do not trust the narrative below. Trust the numbers you reproduce.

---

## 1. What the benchmark is

`tests/vs-v86/scorecard.mjs` boots a full Linux under **both** emulators —
rv64.js (this repo, RISC-V→wasm JIT) and **copy/v86** (mature x86→wasm JIT) — and
runs the same workloads *inside the guest*. System emulation only; v86 has no
user mode, so user-mode comparisons are meaningless and must never be used.

**13 rows.** Five wall-clock (ALU, Mixed, Boot, `python fib(30)`, `compile` =
`tcc -c`) plus eight self-timed nbench/BYTEmark kernels (NUMERIC SORT, STRING
SORT, BITFIELD, FP EMULATION, FOURIER, ASSIGNMENT, IDEA, HUFFMAN).

**The bar (user directive): WIN or MATCH on EVERY row.** Speed ratio is
rv64/v86 where >1 = rv64 faster; MATCH = within 5% (≥0.95). The scorecard
self-invalidates if a fixed CPU probe drifts >1.25× across the run, and prints
`SCORECARD INVALID` with a reason. **A run marked INVALID is not evidence.**

Correctness is enforced separately and must stay green (see §6). Cross-ISA
checksums are compared automatically: the ALU checksum is bit-identical across
ISAs and the Mixed checksum folds to the same 64-bit value, so a miscompiled
row cannot be reported as a win.

---

## 2. THE TRAP THAT INVALIDATED MOST OF THE SESSION'S CLAIMS

**Several rows swing ±20-100% across boots of IDENTICAL code.** This is not host
noise — the host drift guard was green. It is a coverage race inside the JIT
(which pcs get hot first, which superblocks land, when).

Proof, from two runs of the **same commit `6ccf10a`**, same machine:

| row | run 1 | run 2 |
|---|---|---|
| IDEA | 1.22 | **2.72** |
| FP EMULATION | 1.86 | 2.24 |
| NUMERIC SORT | 1.42 | 1.15 |
| ASSIGNMENT | 0.89 | 0.75 |

The session author compared later work against **run 1** and reported "IDEA more
than doubled" and "FP EMULATION 1.86 → 2.23" as wins. **Both were false** — the
baseline already measured 2.72 and 2.24 in run 2. Treat every single-sample
claim in the historical narrative with suspicion for these rows: NUMERIC SORT, ASSIGNMENT,
HUFFMAN, IDEA, FP EMULATION, and `python fib(30)` (observed 3231/3279/3486/
3644/3701/3759 ms on effectively the same build).

**Therefore: any comparison you make needs ≥3 full runs per candidate, or
interleaved medians (`NBREPS=3`), and preferably both.** Rows that differ by
<10% between candidates should be called a tie.

---

## 3. Full historical record (every scorecard run, in order)

Speed ratios, rv64/v86, ≥0.95 passes. Raw JSON for all of these is in
`target/bench/scorecard-*.json` with git rev, wasm sha256 and config recorded.

```
timestamp            commit  pass  nbreps  NUM  STR  BITF  FP   FOUR ASGN IDEA HUFF py   cc
2026-07-26T15-12-10  6ccf10a 10/13  1      1.42 1.42 2.88  1.86 0.99 0.89 1.22 1.22 0.88 0.37
2026-07-26T17-44-31  6ccf10a 10/13  1      1.15 1.23 2.71  2.24 1.19 0.75 2.72 1.23 0.83 0.46  INVALID
2026-07-26T20-15-06  2d7d14e 10/13  1      1.04 1.24 2.72  2.40 1.12 0.78 2.81 1.29 0.70 0.47
2026-07-26T22-15-57  53ec078  9/13  1      0.89 1.21 2.73  2.28 1.16 0.76 2.78 1.13 0.70 0.45
2026-07-26T23-08-40  0a3143a  9/13  3      0.89 1.19 2.70  2.33 1.16 0.77 2.78 1.16 0.68 0.46  INVALID
2026-07-26T23-49-30  7391ca1 10/13  3      1.04 1.19 2.73  2.25 1.15 0.79 2.80 1.27 0.69 0.46  INVALID
2026-07-27T00-59-27  2bd9b17 10/13  3      1.02 1.21 2.72  2.26 1.17 0.77 2.79 1.28 0.68 0.46
2026-07-27T02-03-50  d7428f8 10/13  3      1.05 1.22 2.73  2.23 1.18 0.83 2.78 1.28 0.70 0.46
2026-07-27T03-43-23  318b55d 10/13  3      0.97 1.18 2.72  2.14 1.15 0.77 2.66 1.25 0.69 0.44
2026-07-27T06-30-44  1d1b107 10/13  3      0.79 1.28 2.72  2.19 1.39 0.77 2.66 1.20 0.95 0.45
2026-07-27T07-01-59  3855c72 10/13  3      0.98 1.26 2.72  2.17 1.39 0.76 2.67 1.21 0.85 0.46
```

ALU/Mixed/Boot are omitted: they won comfortably (1.37-1.44 / 1.44-1.89 /
1.62-1.91) in every run and are not in contention.

**No run ever exceeded 10/13.** (An "11/13" appeared in the old narrative from the
PREVIOUS session, 2026-07-25 — measured single-sample on a different, noisier
machine, before the drift guard and interleaved medians existed. It has never
been reproduced under the current methodology and should be treated as
unverified.)

**Note `6ccf10a` was never measured with `NBREPS=3`.** That is the single
biggest gap in the record and probably your highest-value first experiment.

---

## 4. Candidates to evaluate

### A. `6ccf10a` — pre-session baseline (the control)
The tree as it existed *before* the session's JIT work. Its best draw had the
two worst non-compile rows at **0.88 / 0.89** — closer to a hypothetical 12/13
than anything the session produced. Its NUMERIC SORT (1.15-1.42) was a
comfortable WIN, where every later tree has it borderline (0.79-1.05).
**Never measured with NBREPS=3.** Hypothesis worth testing seriously: *the
session's work was net-neutral or net-negative.*

### B. `3855c72` (or `a62142b`, docs-only on top) — session HEAD
Best FOURIER (1.39, from inline caches) and best typical python (0.85, one 0.95
draw). NUMERIC borderline (0.98) — could flip out on any draw.

### C. `d7428f8` — batch modules landed, before inline caches
Best ASSIGNMENT of the session (0.83) with NUMERIC still healthy (1.05) and
FOURIER 1.18. Arguably the best-balanced nbench profile; python is poor (0.70).

### D. `1d1b107` — batching enabled by default
Best python ever recorded (0.95 = MATCH) but worst NUMERIC (0.79). Included
because it is the only run where python passed. Suspected to be a lucky python
draw rather than a real flip — worth confirming or killing.

### E. Config space on HEAD (no rebuild needed — cheapest experiments)
HEAD exposes runtime knobs, so one binary can emulate much of the design space.
All are `jit_set_*` exports callable from the harness, and the focus/flip
scripts read them from env vars:

| env | effect |
|---|---|
| `TRACELVL=0..3` | 0 = classic basic blocks; 1 = branch side-exits; 2 = +call following; 3 = +return following & inline caches (default 3) |
| `ICTRIG=N` | inline-cache trace-extension trigger; `0` disables ICs (default 256) |
| `BATCH=0/1`, `BCAP=N` | batch modules (default off, cap 32) |
| `KEEPMIN=N` | region functions skip claiming traces ≥N insns (default 0) |
| `DEMOTE=0/1` | region demotion safety valve (default on) |
| `SBSPACE=N` | superblock build spacing in Minsns (default 16) |
| `SB=0/1` | page superblocks (scorecard runs `SB=1`) |
| `TLBFILL=0/1` | in-block TLB refill (default 0) |

**Specifically worth testing:** `TRACELVL=0` on HEAD. NUMERIC SORT dropped from
~1.15 to ~1.0 exactly when trace compilation landed and never recovered; if
disabling traces restores NUMERIC while keeping FOURIER's IC gain, HEAD may be
strictly improvable. (Untested — the session ran out of time here.)

---

## 5. How to run things

### 5.0 CRITICAL: the tree does not build from a clean checkout
The repo has **uncommitted in-flight work** (networking/proxy: `rv64-system`
`egress.rs`, `httpproxy.rs`, `virtio.rs`, `lib.rs`, plus `Cargo.toml`). The
committed JIT code depends on it: committed `rv64-system/src/lib.rs` has
`pub fs: Option<p9::Server>` while the **working tree** has
`pub fs: Vec<p9::Server>`, and the committed `rv64-wasm` is written against the
working-tree API. A fresh clone at HEAD **will not compile**.

=> **Work in this working tree. Do not `git stash`, `git checkout .`, or reset.**
(The session author stashed once and broke the build; it was recovered, but
don't repeat it.)

To evaluate an OLD commit, take only the two JIT files and keep everything else:
```sh
git checkout 6ccf10a -- crates/rv64-jit/src/lib.rs crates/rv64-wasm/src/lib.rs
# then two edits are needed against the current rv64-system API:
#   1. in rv64-wasm sys_boot: `fs: None`      -> `fs: vec![]`
#      and `Some(p9::Server::new(...))`       -> `vec![p9::Server::new(...)]`
#   2. re-add the getrandom shim if the build reports `undefined symbol:
#      __getrandom_custom`:
#        fn tls_random(buf: &mut [u8]) -> Result<(), getrandom::Error> {
#            unsafe { host_random(buf.as_mut_ptr(), buf.len()) } Ok(())
#        }
#        getrandom::register_custom_getrandom!(tls_random);
# restore with:  git checkout HEAD -- crates/rv64-jit crates/rv64-wasm
```
This recipe was used successfully during the session to A/B an older JIT.

### 5.1 Environment
Everything runs in the nix dev shell. The host's `~/.config/nix/nix.conf` holds
**stale GitHub tokens that 401 every flake fetch**; override per command:
```sh
export NIX_CONFIG="access-tokens = github.com=$(gh auth token)"
export ARTIFACTS=/home/darren/src/arm64.js/target/bench
```

### 5.2 Build
```sh
nix develop -c cargo build --release -p rv64-wasm --target wasm32-unknown-unknown
```
Artifacts (guest images, v86 checkout, tcc/nbench binaries) are already built in
`target/bench/`. If they are missing:
`DEBIAN=1 nix develop -c tests/vs-v86/setup.sh target/bench` plus a built
copy/v86 at `target/bench/v86` (see `tests/vs-v86/README.md`).

### 5.3 The authoritative measurement (~45 min)
```sh
ARTIFACTS=$ARTIFACTS NBENCH=1 SB=1 REPS=3 NBREPS=3 \
  nix develop -c node tests/vs-v86/scorecard.mjs
```
`REPS=3` medians the wall-clock rows; `NBREPS=3` runs the whole nbench leg three
times on **both** sides, interleaved (v86, rv64, v86, rv64...), and medians per
kernel. Writes `target/bench/scorecard-<ts>.{md,json}` with provenance. **Check
the printed host-probe spread and any `SCORECARD INVALID` line before believing
a row.** Run it at least 3× per candidate.

### 5.4 Fast screening (seconds to minutes)
```sh
node tests/vs-v86/flip.mjs python 6        # ~11s
node tests/vs-v86/flip.mjs compile 4       # ~7s
node tests/vs-v86/flip.mjs numeric 3       # ~5min
node tests/vs-v86/flip.mjs assignment 2    # ~5min
```
Measures ONE row on the rv64 side against the v86 number recorded in the newest
scorecard JSON (v86 is a control — our JIT cannot change it) and prints the same
WIN/MATCH/LOSS verdict. nbench single-kernel selection uses `CUSTOMRUN=T` +
`DO<KERNEL>=T` in a command file (nbench upper-cases the `-c` path, so the file
must be `/C`).

`tests/vs-v86/screen.mjs nb|cc|py K` runs K parallel boots and prints per-metric
medians for the whole nbench table.

**Both are SCREENS, not verdicts.** A kernel run alone differs from the same
kernel inside the full table, and parallel boots inflate wall-clock rows.
Confirm anything interesting with §5.3. The machine has 64 threads, so config
sweeps can be run concurrently (5 configs × 3 boots ≈ 5 min).

---

## 6. Correctness gates (must stay green for any candidate)
```sh
ARTIFACTS=$ARTIFACTS nix develop -c bash tests/run-all.sh
```
Expect: cargo tests; qemu differential; **riscv-tests ISA 134/134**; **Spike
lockstep LOCKSTEP-OK**; **riscv-arch-test 193 match / 0 mismatch**; wasm-smoke
(incl. budget contract, bit-equal retirement differential, emitter validity);
60-program JIT differential; fp-context-switch; AMO differential.
`virt-smoke` SKIPs unless its kernel is built — that is expected.

Watch for **SKIP** on stages 4/5/6: those stages were silently unavailable for
much of the session (a deleted `rv64-isa-test` binary) and `run-all.sh` lacked
`pipefail`, so failures could print and the suite still said ALL STAGES PASSED.
Both are fixed, but a SKIP there means you are not actually testing conformance.

---

## 7. What the session concluded (verify or refute)

1. **Inline caches at indirect jumps** (`c155cbd`) are the main claimed win:
   traces continue through `jalr` under a one-compare guard against an observed
   target. Measured: compile-row dispatches 25.5M → 17.4M (−32%),
   insns/dispatch 13.8 → 20.1, FOURIER 1.15 → 1.39. **This is the most solid
   claim in the session** (large effect, consistent across runs).
2. **Cutting dispatch count by 32% did not change compile's wall time at all.**
   If true, `compile` is not dispatch-bound and no block/dispatch restructuring
   can fix it — the residue is per-instruction code quality (an inline TLB probe
   on every memory op; no register allocation across block boundaries, which is
   costly because RISC-V has 31 GPRs to spill/reload at each edge vs x86's 8).
   **Worth independently verifying — it drives all future direction.**
3. **Nine structural designs measured neutral or negative** and are documented
   in `PERFORMANCE_PROGRESS.md`: three chaining architectures (the V8 shared-table import
   makes `table.set` O(importing instances) — quadratic registration), batch
   modules, page co-location, superblock spacing, definedness tracking,
   next-executing-tail formation. Re-deriving any of these is likely wasted
   effort, but the *reasoning* is worth a skeptical read.
4. **Batching cannot ship**: helps ASSIGNMENT (+9-21%, never past 0.944) and
   costs NUMERIC ~25%.

## 8. Questions we want answered

1. Re-measure `6ccf10a` and HEAD with `NBREPS=3`, ≥3 runs each, interleaved.
   **Is the session's work actually a net improvement?**
2. Is any candidate reliably better than 10/13, or is 10/13 the ceiling for all
   of them and the row identity just rotates with the draw?
3. Does `TRACELVL=0` (or 1/2) on HEAD recover NUMERIC SORT without giving back
   FOURIER's inline-cache gain? If so, HEAD is improvable for free.
4. Is `python fib`'s 0.95 in run `1d1b107` reproducible, or a draw?
5. Independent read on claim §7.2 — is `compile` really not dispatch-bound?

## 9. Ground truth files
- `PERFORMANCE_PROGRESS.md` — consolidated outcomes, measured dead ends,
  current plan, and append-only experiment ledger. The removed narrative
  remains available in Git history at `1ec9130`.
- `target/bench/scorecard-*.json` — every run, with provenance.
- `tests/vs-v86/README.md` — benchmark policy, anti-overfitting rules,
  reproduction instructions.
