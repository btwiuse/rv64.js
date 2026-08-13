# R102: bounded structured fuel-check coalescing

Date: 2026-08-10  
Status: candidate rejected at native gate, archived, and cleanly removed

## Motivation

R101 counted a conservative, architecture-general set of structured members
whose post-member fuel comparison can be merged with the next comparison or
function return without changing the scheduler's existing 128-instruction
granularity bound. Compile STEADY dynamically executes 14,873,571 such members
(37.210% of 39,972,369 entries), representing 59,494,284 Wasm operators:
`local.get retired`, `local.get fuel`, `i64.ge_u`, and `br_if`. Boot exposure is
only 9.219%, so Compile is the motivated row and Boot is a non-regression row.

This mechanism is selected from control-flow and static retirement counts. It
does not use guest PCs, benchmark identity, opcode classes, address ranges,
privilege, host engine, or scorecard phase.

## Frozen control and environment

- exact clean R085-equivalent control Wasm:
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`
  (4,279,380 bytes), archived at
  `target/bench/r098-interrupt-deadline/artifacts/control-d9f686a9ce4f.wasm`;
- loader:
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- modern Linux 6.12.7 / Alpine 3.24.1 guest artifacts only (no TinyEMU kernel
  and no BBL);
- production page policy and public one-slice-per-event-loop-turn cadence;
- CPUs 8--15, existing host-probe/spread rules, and frozen workload/output
  identities.

The candidate artifact and source archive are immutable once the first
performance command starts. Artifact and CODE-section size are recorded and
attributed but are not acceptance thresholds.

The immutable production candidate for this experiment is
`0a0dbb4bafe3556a113392e3ee72cbcc7d6d4f1718637e5ba7f538777cbcedd2`
(4,294,003 bytes, +14,623 bytes versus control). The size delta is diagnostic;
only the direct construction and workload measurements below decide whether it
advances.

## Frozen implementation

Use R101's deterministic independent-set plan unchanged. A member is selected
only when all of these hold:

1. every successor is statically enumerable;
2. the member has no local self-edge and no variable-trip acceleration;
3. its static retirement count is at most 128;
4. no selected member is adjacent on an internal CFG edge; and
5. its retirement count plus every internal successor's count is at most 128.

Ordering is deterministic: statically terminating members first, then lowest
undirected CFG degree, shortest retirement count, and member index. At each
selected emitted occurrence, omit exactly the ordinary four-operator
post-member fuel comparison. Do not alter retirement accumulation, precise
side exits, state commit, successor selection, generated cross-module fuel
checks, the outer scheduler, fuel refresh cadence, interrupt quantum, region
geometry, thresholds, compilation lifecycle, or any other lowering.

The R101 profiling ABI is removed from the production candidate after its
diagnostic artifact/source are archived. It must not be used to claim elapsed
performance.

## Safety proof and correctness gates

Before timing:

1. Unit tests must prove deterministic selection, rejection of dynamic
   successors, self-loops, variable-trip members, over-128 members, adjacent
   selections, and pairs whose combined retirement exceeds 128.
2. A generated-Wasm shape test must prove selected occurrences lose exactly
   one `local.get retired`, one `local.get fuel`, one `i64.ge_u`, and one
   `br_if`, while unselected occurrences retain them.
3. A bounded-fuel execution test must enter every member in directed acyclic,
   conditional, cyclic, external-exit, and precise-side-exit graphs across
   fuel values around every member/pair boundary. Candidate architectural
   state must match the reference and retirement before a generated boundary
   must never exceed fuel by more than 127 instructions.
4. Run DBT/core/workspace units, all focused JIT differentials (integer/RVC,
   M, A/random atomics, FP, memory, Sv39/context, bulk copy, WFI, lifecycle,
   public/worker APIs), and modern direct plus OpenSBI Linux boots.

Any semantic, bound, validation, output, or lifecycle failure rejects R102.

These gates completed before performance timing:

- all 54 `rv64-dbt` unit tests passed, including deterministic selector,
  rejection, adjacency, combined-retirement, and emitted-shape assertions;
- the feature-gated execution proof passed all 260 entry/fuel combinations
  (four entries times fuel 0 through 64), with reference state equality and
  the 127-instruction overshoot bound;
- workspace/core and the complete focused JS/Wasm JIT matrix passed;
- ISA differential passed 134/134, Spike lockstep passed 109/109 with 24,103
  matched writebacks, and architecture signatures matched 193/193; and
- modern Linux 6.12.7 direct, OpenSBI, and virt-smoke boots all passed. The
  feature-only proof export is absent from the production artifact.

## Construction and native causal gates

First run seven alternating fresh-process cold compile+instantiate pairs over
the immutable control and candidate. Record full-module and executable-CODE
identity. Candidate/control median construction time must be <=1.03; size is
diagnostic only.

Then run seven fresh alternating control/candidate pairs for Boot, Compile,
and Python, all 42 legs in one predeclared report. Require:

- exact artifact/loader/guest/policy/cadence/input/output identity and all legs;
- host spread <= the existing 1.10 limit;
- Compile paired median speedup >=1.03 and paired-bootstrap 95% lower bound
  >=1.00;
- Compile normalized-MIPS ratio >=1.03;
- candidate/control elapsed median <=1.03 for Boot and every Python phase.

No retry, selector threshold, alternate ordering, larger segment, special-case
PC, mixed policy, or code-shape variant is allowed after seeing these results.
Failure stops before browsers and restores exact control source/artifact.

## Browser, WANIX, and scorecard escalation

Only after the native gate passes:

1. Run the existing fresh-Chrome direct execution-Boot guard.
2. Run the R094-qualified long fixed-work WANIX guard for shell, Python,
   SHA-256, and 32 MiB shared 9P. Every phase must satisfy its frozen paired
   non-regression and confidence rules; `/shared/bench.py` may not regress.
3. Run the untouched authoritative 13-row, three-way, odd-repetition
   scorecard. Require no correctness failures, no new v86/legacy loss, no row
   regression beyond the standing limits, and an increased or unchanged total
   parity score.

Promotion requires every stage. Otherwise archive the exact evidence, document
the failed causal gate, remove R102 product code, and restore the exact
R085-equivalent baseline.

## Results

The seven-pair fresh-process construction gate passed. Control construction
was 5.523684 ms and candidate construction was 5.622481 ms at the median, a
1.017886 candidate/control ratio below the frozen 1.03 limit. The candidate's
14,623-byte module growth and 12,768-byte CODE-section growth therefore caused
about 0.10 ms of measured construction latency here; neither size delta was a
rejection criterion. The report is
`target/bench/r102-structured-fuel/cold-compile.json`, SHA-256
`9a327d30a4c952dc71d7c1077aa73851ed1522cc985c1d5557dae9f48fc2906a`.

The complete 42-leg native report is measurement-valid. All artifact, loader,
modern guest, production policy, public cadence, generated-execution,
guest-work, input, and output identities passed; host-probe spread was 1.071x.

| Row | Control median | Candidate median | Paired speedup | 95% paired interval | Required result |
|---|---:|---:|---:|---:|---|
| Boot FIRST | 2,172.921 ms | 2,166.984 ms | 1.007x | `[0.987,1.036]` | guard passed |
| Compile STEADY | 942.820 ms | 940.407 ms | 0.997x | `[0.978,1.083]` | target failed |
| Python STEADY | 2,388.775 ms | 2,317.880 ms | 1.016x | `[0.999,1.033]` | guard passed |

Compile's paired normalized-MIPS ratio is also 0.997x. Thus the elapsed point
medians happen to differ by 0.26% in the candidate's favor, but paired and
instruction-normalized evidence both show a tie, not the frozen 3% gain. The
native report is
`target/bench/r102-structured-fuel/native-ab/config-ab-2026-08-10T05-21-14-520Z.json`,
SHA-256 `847463625d23829c18a2444faade746501853033bde0ae6ccf5958d77740e4e3`;
the gate is `target/bench/r102-structured-fuel/native-gate.json`, SHA-256
`24fb325d45b62d7ab366cdd3cafed95e5a84bf0457445c16c1c9e907b86779fc`.

## Decision

Reject R102 at its first failed causal gate. Do not escalate it to Chrome,
WANIX, or the authoritative scorecard, and do not tune its independent-set
ordering, segment size, successor policy, or Wasm spelling after seeing this
result. R101's 59.5 million removable operators establish dynamic opportunity,
but not measurable optimized-engine cost.

The immutable candidate, source archive, and reports remain under
`target/bench/r102-structured-fuel/`. All live selector, lowering, feature, and
test-export code was removed. The release rebuild is byte-exact control
`d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`;
restored DBT units pass 53/53. Official parity remains 13/13 versus legacy and
11/13 versus copy/v86, with Boot and Compile open.
