# Final RV64-to-WebAssembly JIT Rewrite Exercise Report

> **Historical closure record.** This report describes the repository and
> owner-directed stop on 2026-08-10. Work was subsequently reauthorized,
> checkpointed, extended with RV64GCV lowering, and continued with a focused
> Boot JIT-lifecycle objective. Its statements that no more work was authorized,
> that the rewrite was uncommitted, and that Boot remained a loss are not the
> current repository status. See
> [BOOT_JIT_PARITY_RESULT.md](BOOT_JIT_PARITY_RESULT.md) and
> [STATUS.md](STATUS.md) for the current result. The chronology below is
> intentionally preserved rather than rewritten.

Date: 2026-08-10  
Disposition: concluded by owner; implementation preserved; parity objective not achieved

## Executive conclusion

The clean-room RV64-to-WebAssembly JIT rewrite produced a capable, tested
emulator and a substantially better measurement system. It did **not** prove
the requested final outcome of parity with copy/v86 on the complete modern
Linux 6.12.7 / Alpine 3.24.1 scorecard.

The last valid authoritative full scorecard remains R087: 13 of 13 rows at or
above the legacy parity floor and 11 of 13 at or above the copy/v86 parity
floor. Matched Boot and Compile remain the two copy/v86 losses. The later
fixed-register-bank candidate has strong focused and WANIX results, but every
attempt to run its full three-way scorecard was invalid or intentionally
stopped before a report. It is therefore promising, not promoted.

No more optimization, benchmark, or scorecard work is authorized. No process
is running. The persistent parity goal must remain unachieved/blocked rather
than be relabeled complete.

## What was built

- The former `rv64-jit` crate was deleted from the active tree. Its Git history
  remains recoverable, but its implementation was not used as a design input.
- New clean-room crate `rv64-dbt` implements typed RV64-to-Wasm translation,
  precise architectural side exits, structured CFG/SCC lowering, bounded
  traces and loops, register-resident multi-entry batches, and sparse
  multi-page regions.
- Scalar RV64GC integer, M, A, C, F/D, CSR, reservation, flat-memory, and
  full-system memory paths are compiled or precisely returned to the
  interpreter. Guest exceptions never depend on host Wasm traps.
- The runtime implements hotness collection, page/region admission, code-page
  generations, self-modifying-code invalidation, fuel-bounded execution,
  stable table publication, asynchronous compilation of large generated Wasm,
  and publication-time revalidation.
- The public loader supports main-thread and browser-Worker execution,
  production JIT policy configuration, lifecycle counters, generated-code
  coverage, and diagnostic profiling without requiring a bundler.
- The modern Virt machine boots Linux 6.12.7 / Alpine through direct host SBI
  and OpenSBI paths. TinyEMU/BBL remain compatibility material and were not
  scorecard inputs.
- A three-way harness now runs rewrite, an isolated legacy modern-Virt adapter,
  and copy/v86 with matched riscv64/i686 Linux 6.12.7 / Alpine 3.24.1 guests,
  active-generated-execution proof, exact artifact/workload identities,
  fresh-process repetitions, phase-separated cold/prime/steady timing, CPU
  affinity, host probes, fixed outputs/work, and explicit eligibility.
- A WANIX external-9P stall exposed by the benchmark was corrected with FIFO
  single-flight delivery at the WANIX adapter endpoint. Generic emulator 9P
  concurrency remains intact.

## Terminal repository state

| Item | Terminal value |
| --- | --- |
| repository | `/home/darren/src/jit` |
| branch | `main` |
| Git HEAD | `96aa93896e7bb6fa561d1f977c9bf23cd909a100` |
| last committed subject | `fix: pin demo site to image release` |
| rewrite commit/merge | none |
| worktree summary before this report | 40 modified, 4 deleted, 297 untracked status entries |
| active scorecard/JIT benchmark processes | none |

The worktree is intentionally large and uncommitted. The rewrite, tests,
research, experiment records, and harnesses are **not** present in Git HEAD and
must not be inferred from the branch name or last commit.

The ordinary release build currently contains the final experimental R124
candidate, because R126 proved that selecting it through the explicit artifact
override makes an authoritative run diagnostic-only:

| Item | SHA-256 | Bytes/status |
| --- | --- | ---: |
| live release Wasm | `d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59` | 4,281,786 |
| live `rv64-dbt/src/lib.rs` | `e4be5025e892f417fdde56dfc0c4c5ead632b474de9ea849558b9ed7ffbed795` | exact candidate |
| live `rv64-dbt/src/wasm.rs` | `f60b8ae438cf1fcec7dc22215e6e3da5caf1755a7d5c214ec09b5928eb54d96e` | exact candidate |
| unchanged Wasm runtime source | `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339` | exact |
| public loader | `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385` | exact |
| deterministic candidate source archive | `6497fe464b64113525620f0f1ae4ac767a2137fd09b7d5cb843ba893f5627829` | 168,543 |

This candidate is not scorecard-promoted and was not committed. The last
internally accepted pre-candidate control Wasm is preserved as
`d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`
(4,279,380 bytes). Exact control DBT source backups are preserved with hashes
`ba497233...` and `b5e9c11e...`. No automatic restoration was performed at
closure because the owner requested conclusion and preservation, not a
destructive rollback.

## Closure correctness evidence

The following non-performance checks were rerun against the terminal live
candidate on 2026-08-10:

- `cargo test --workspace`: **177 passed, 0 failed**;
- `node tests/vs-v86/scorecard-v2-selftest.mjs`: passed;
- `node tests/vs-v86/r126-r124-scorecard-gate-selftest.mjs`: passed;
- `node tests/analyze-r125-wanix-selftest.mjs`: passed; and
- `go test ./...` in `integrations/wanix/adapter`: package builds successfully
  (no Go test files selected).

Before R124 timing, the exact candidate also passed the wider recorded matrix:
60 randomized integer programs, M/A/FP and system differentials, randomized
atomic/T2 and multi-entry/T2 cases, public/Worker/raw-Wasm ABI checks, 338
generated-module validations, and direct/OpenSBI Linux 6.12.7 / Alpine 3.24.1
boots with generated execution. Those wider suites were not rerun during
closure; their immutable exact-candidate evidence remains under R124.

Correctness evidence does not substitute for the missing complete performance
scorecard.

## Last valid full three-way scorecard

R087 report
`target/bench/r087-authoritative-three-way/scorecard-v2-2026-08-09T19-59-20-960Z.json`
is measurement-valid, authoritative, has no problem, and contains all 117
scheduled trials (13 rows x 3 sides x 3 fresh processes). Its SHA-256 is
`1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7`.

It used rewrite Wasm `efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`,
legacy Wasm `274aaab5799386956a8c509434961c4a426066f8fc9f520e994c210affd61709`,
and copy/v86 Wasm `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1`
from v86 commit `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`.

| Row | Rewrite median | copy/v86 median | rewrite/v86 speed ratio | Outcome |
| --- | ---: | ---: | ---: | --- |
| Matched Boot | 2,176.495 ms | 1,563.646 ms | `0.718424x` | below parity |
| Compile (`tcc -c`) | 954.483 ms | 728.859 ms | `0.763616x` | below parity |

All other eleven rows meet the `0.95x` copy/v86 parity floor, and all thirteen
meet the legacy floor. Consequently the final official score is **13/13 versus
legacy and 11/13 versus copy/v86**. R087's `goalMet` is correctly `false`.

## Final candidate evidence

### R124 native product gate

The exact `d017a10f...` candidate was compared with exact control
`d9f686a9...` in 15 alternating fresh-process pairs for modern Linux 6.12.7 /
Alpine 3.24.1 Boot, Compile, and Python. After the conservative 0.168840 ms
construction debit:

| Row | Paired-median speedup | Paired-bootstrap 95% interval |
| --- | ---: | ---: |
| Boot | `1.018471x` | `[1.000973, 1.035481]` |
| Compile | `1.083675x` | `[1.037357, 1.112250]` |
| Python | `1.200538x` | `[1.180720, 1.220097]` |

Normalized fixed-work Compile is `1.083602x`. The gate passed every frozen
integrity, target, protected-row, work, active-JIT, affinity, and host check.
Gate report SHA-256: `74d962c2d17458f03592012d274d2ff992c1db696b39d1571f9e871604c898ab`.

### R124 natural Chromium confirmation

Seven fresh Chrome pairs on the same modern guest measured execution-only Boot
at `1.018971x [1.001431,1.037301]` and construction-to-ready at
`1.017013x [0.998068,1.029593]`. The prospective gate passed. Report SHA-256:
`c3d5b975ea09219ffb62948b03154d8167ad505c4af045b102b694b56f2e6243`.

### R125 WANIX qualification

R125 is a complete, measurement-valid fourteen-browser run using Linux 6.12.7
/ Alpine 3.22.5 / Python 3.12.13 and exact public `/shared/bench.py` work. It
proved active generated execution and completed three synchronized samples of
each workload per browser:

| Row | Paired-median speedup | Paired-bootstrap 95% interval |
| --- | ---: | ---: |
| Shell ready | `0.996257x` | `[0.989513, 0.998082]` |
| Python `/shared/bench.py` | `1.079692x` | `[1.070039, 1.111911]` |
| SHA-256 | `1.021724x` | `[1.018486, 1.037770]` |
| shared 9P | `1.009359x` | `[1.003915, 1.016363]` |

The immutable R125 analyzer reports `gatePassed=false` because its original
zero-established-regression rule rejects the measured 0.374% shell slowdown.
The owner subsequently selected a one-percent materiality rule: protected
medians at or above `0.99x` do not veto independently verified gains unless a
regression larger than 1% is established. Under that prospective product
policy, R125 is accepted as WANIX qualification, not as final product
promotion. Report SHA-256:
`9e051d1fd4b23c7b440134778e54a500c5fe6eb0cdc058e0d8bf3db359491868`.

This valid result supports the no-regression requirement for the public Python
guard. It cannot prove the broader final objective without a valid
`d017a10f...` modern three-way scorecard.

## Why no post-R125 score exists

| Run | Terminal classification | Evidence consequence |
| --- | --- | --- |
| R126 | invalid harness/input admission | 117 scheduled, zero eligible, 135 problems; diagnostic artifact override invalidated RV64 and the matched x86 kernel was absent |
| R127 | externally terminated | repaired admission passed, but the interactive tool session killed the parent during Python; no report |
| R128 | owner-requested stop | detached admission worked; ALU, Mixed, and Boot orchestration reached `ok`, Python began, then the exact process group was terminated; no report |

R126 report `ac096fec...` must never be interpreted despite its displayed
`goalMet=true`; its own `measurementValid=false` and zero eligible population
are authoritative. R127 and R128 have no formal report. No process from any of
these runs may be pooled, reconstructed, replaced, or reused.

The post-R126 harness repair is still useful: it restores the exact matched
i686 kernel, preflights every v86 kernel/BIOS/runtime input before scheduling,
keeps explicit artifact overrides diagnostic-only, and provides a frozen
incremental adjudicator. R128 proved that the repaired admission and detached
ownership worked, but not candidate performance.

## Benchmark-methodology conclusions

The final harness addresses the original measurement concerns as follows:

- modern matched guests only: Linux 6.12.7 / Alpine 3.24.1 riscv64 and i686;
- no TinyEMU kernel, BBL, or unrelated root in the scorecard;
- active JIT and installed/executed generated-code proof for both rewrite and
  copy/v86 rather than trusting configuration flags;
- separate first, prime, and steady phases, including generated-Wasm
  compile/instantiate/publication accounting;
- fresh Node/browser process boundaries instead of silently warm module or V8
  code caches;
- fixed work, output checksums, guest instruction accounting, exact input and
  implementation hashes, public scheduler cadence, and CPU affinity;
- all repetitions retained, host-stability checks, explicit validity, and no
  post-result sample replacement; and
- prospective target/protected-row policies with code size treated only as a
  diagnostic fact.

The main methodological lesson is that a Wasm-targeting DBT is a two-compiler
system. Guest translation quality, generated-module size/topology, host Wasm
frontend latency, background tier-up, publication timing, scheduler cadence,
and useful execution all have to be measured independently. A fast generated
loop can still lose end-to-end if it compiles too early, too synchronously, or
into modules that the host engine optimizes too late.

## Accepted technical findings

- The rewrite decisively exceeds the isolated legacy comparator on the valid
  modern scorecard, establishing that RV64 compiler output is not inherently a
  blocker for this emulator architecture.
- Modern steady compute is generally competitive with or faster than copy/v86;
  the remaining problem separates into interpreter/runtime-dominated Boot and
  generated-code/lifecycle-dominated Compile.
- Bounded asynchronous page/region compilation is preferable to synchronous
  eager compilation. Compile storms and repeated page rebuilding are directly
  observable regressions.
- Structured SCC lowering, register-resident state, precise Wasm-to-Wasm hot
  edges, inline typed translation proofs, and conservative fallback are sound
  foundations. Many plausible local mechanisms failed because their whole-row
  exposure was too small or because host-engine code generation offset the
  guest-side saving.
- R124's architecture-general fixed RV64C register bank is the strongest final
  unpromoted candidate: it reproduced byte-for-byte, passed broad correctness,
  improved focused native and Chromium measurements, and improved public
  Python/SHA/9P. Its missing independent full scorecard is decisive.
- The WANIX 9P hang was an integration defect, not a JIT speed result. Adapter
  single-flight fixes that endpoint without serializing the generic emulator.
- A one-percent verified gain with no protected row worse than the one-percent
  material boundary is the final owner-selected economic policy. Code-size
  growth or a former 10%/20% hurdle is not a standalone rejection criterion.

## Unfinished requirements and terminal decision

The original performance objective remains false/unproven because:

1. exact final candidate `d017a10f...` has no valid full 117-trial modern
   three-way report;
2. Boot and Compile have not been shown at copy/v86 parity in a common valid
   population;
3. the candidate has not passed the frozen incremental scorecard adjudicator;
4. no accepted source state has been selected between control `d9f686a9...`
   and experimental candidate `d017a10f...`; and
5. the rewrite worktree has not been intentionally staged, committed, merged,
   or published.

The owner has chosen to conclude rather than resume those steps. This is a
terminal project decision, not a technical claim that the missing evidence
would have passed or failed.

## Preservation and any future recovery

The authoritative human-readable history is in `docs/jit-rewrite/`, with raw
artifacts under `target/bench/`. `FINAL_EXERCISE_SHA256SUMS` seals this report,
terminal source identities, key valid reports, invalid/paused records, and raw
R128 logs.

If a future effort independently chooses to resume, it must first preserve or
commit the dirty worktree, explicitly choose candidate versus accepted-control
source, rebuild reproducibly, and run all 117 scorecard processes from zero.
R126--R128 cannot supply a single sample. That is recovery guidance only; this
exercise is closed and no continuation is scheduled.
