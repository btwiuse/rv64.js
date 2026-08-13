# R072 Exact Sampled Static-T0 Protocol

Date: 2026-08-09  
Status: rejected at frozen browser nonregression gate; candidate remains default-off

## Hypothesis

R071 proves that residual-only activation is too narrow: candidate Boot runs
only 4.878M-7.873M static instructions, while the unchanged page-policy sampler
attributes roughly 105M-108M retired Boot instructions to Rust execution. The
static decoder is already architecture-complete for ordinary RV64I/M/integer
RVC/scalar memory and is 1.624x-2.445x faster on independent local corpora.

Execute the ordinary instructions *inside* each existing page-policy sample
through that decoder while preserving the exact policy observations. This
changes the execution representation, not the compilation selector. It adds no
guest-dependent module generation and no page, PC, symbol, binary, workload,
browser, or scorecard special case.

## Frozen semantic contract

Production remains the 131,072-instruction, q1024 async page policy. For every
sampled chunk the candidate must preserve, in order:

1. the starting `(pc, pa, satp, mode)` captured after the normal machine
   device synchronization;
2. no more than the unchanged 1,024 retired instructions attributed to that
   start;
3. exact post-instruction stop at a generated dispatch entry;
4. the first mapped non-`pc+2`/`pc+4` target not already generated, emitted
   after the main sample as `(pc, pa, satp, mode, retired=0,
   control_entry=true)` when the production control-entry policy requests it;
5. exact interrupt-poll countdown and trap timing; and
6. direct-SBI, exception, WFI, device synchronization, dirty-page, and
   generated-code invalidation behavior.

The auxiliary module may carry sample sidecars and call one main-Wasm interrupt
helper at the architectural 32-instruction poll boundary. A first control
target may perform the same execute-translation probe as the Rust observer.
F/D, A, FENCE, SYSTEM/CSR, fetch/data failures, and MMIO still exit before the
instruction. During a control-observed sample, slow instructions execute one
at a time until the first target is known; otherwise the unchanged R070
64-instruction opcode-independent slow stretch remains allowed. No instruction
family or constant may be changed after timing.

Both timing legs construct the same one auxiliary module before the measured
guest phase. The control executes the current Rust sampler and the candidate
executes the hybrid sampled-static path. R070/R071 reports and their decisions
remain immutable.

## Gate A: correctness and observation equivalence

Before any product timing:

- validate deterministic auxiliary bytes and its one-driver/no-guest-code
  shape;
- compare complete architectural state and memory against the interpreter for
  the existing scalar, memory, Sv39/MPRV, A, FP/FS, WFI, T2, direct-Linux, and
  OpenSBI-Linux matrices;
- compare the ordered page-policy observation fingerprint and count at q1,
  q32, and q1024 over deterministic branch/fallthrough, compressed, scalar
  memory, slow-family, exception/trap, generated-entry, and WFI programs;
- prove the candidate retires sampled static instructions, records zero
  internal errors, and leaves page thresholds, candidates, issued/landed
  mappings, generated execution, and output fingerprints unchanged; and
- run the complete core, DBT, system, and Wasm suites.

Any mismatch rejects the mechanism. It may be fixed only as a semantic bug,
with a directed regression added before timing; sample contents, ordering,
family coverage, and policy constants may not be weakened.

Gate A passed before product timing. The complete strict suite passed all eight
stages, including 134/134 ISA cases, 109/109 Spike lockstep cases, 193
architectural signatures, the Wasm differential matrix, modern Linux direct
and OpenSBI boots, and modern virt smoke. The exact same-main-Wasm Gate B
artifact is
`target/bench/wasm-candidates/r072-sampled-static-cb7ea81685b3cb96.wasm`
with SHA-256
`cb7ea81685b3cb9605f6e01b619f3c15005f4ef77ca00c258c20c7a519bb6df6`.
The prospectively frozen verifier is
`tests/vs-v86/r072-sampled-static-gate.mjs`.

Before any browser sample, the browser control surface, counters, pair runner,
and analyzers were frozen. Both legs use immutable page SHA-256
`28957e0d5ce381184addb291805ba26a6e64d421a51882c4ae56e0512a82cd3d`
and RV64 archive SHA-256
`159fc55c4337345a685252e384d64be39fc50c743b4478e2b864289ad8bb8690`;
the archive was independently unpacked and contains the exact Gate B main
Wasm SHA-256 `cb7ea81685b3...`. The candidate/control runner and analyzer are
`tests/run-wanix-r072-pairs.mjs` and
`tests/analyze-wanix-r072-pairs.mjs`. The subsequent candidate/v86 run uses
the established generic pair harness with the same fixed candidate
configuration and is checked by
`tests/vs-v86/r072-browser-v86-gate.mjs`.

The first candidate/control browser sample is retained as invalid because an
accidentally added 1.25x within-side cap rejects the inherently variable
shared-9P phase (1.385x/1.433x), even though every paired-median timing rule
passed. Accepted pre-R072 R054 evidence has 1.612x shared-9P spread, proving
the new cap was not part of the established browser method. The independent,
non-pooled correction is prospectively frozen in
`R072_BROWSER_CONFIRMATION_PROTOCOL.md`; the original analyzer schema and
report remain unchanged and invalid.

## Gate B: product timing

Use the cumulative-gain track prospectively. First run one Boot gross screen
and stop on a correctness difference, candidate error, or slowdown greater
than 5%. Otherwise run five fresh alternating same-main-Wasm pairs for Boot,
Compile, and Python on CPUs 8-15 with the exact Linux 6.12.7 / Alpine 3.24.1
artifacts.

Advance only if Boot's paired median is at least 1.03x with paired-bootstrap
95% lower bound at least 1.00x; Compile and Python medians are each at least
0.97x; host/sample spreads stay within 1.25x; and all fingerprints and policy
proofs pass. No repeat, quantum/slow-batch/family adjustment, or sample pooling
is allowed after a valid unfavorable result.

## Gate C: browser and authoritative promotion

A passing native candidate must pass five fresh-browser candidate/control
`/shared/bench.py` pairs with no phase more than 3% slower, then the existing
rewrite/v86 browser guard. Only then make it default-on, rebuild and archive
the artifact, rerun full correctness, and run the untouched authoritative
13-row rewrite/legacy/copy-v86 scorecard. Promotion requires 13/13 against
legacy, no new v86 loss, no decrease from 11/13 v86 wins, and no accepted row
regression greater than 5%.

R072 is only an intermediate product improvement. The thread goal remains
open until both Boot and Compile reach copy/v86 parity without regressing
`python /shared/bench.py`.

## Frozen result

Gate A passed the complete eight-stage correctness suite. Gate B then passed
on five fresh same-main-Wasm pairs. Report
`target/bench/r072-sampled-static-native/config-ab-2026-08-09T06-23-00-619Z.json`
measures Boot 1.209x `[1.190,1.239]`, Compile 1.021x `[0.978,1.107]`, and
Python 1.049x `[1.008,1.065]`, with 1.020x host spread, exact fingerprints,
roughly 106M sampled Boot instructions, and zero static errors. The frozen
verifier returns `R072_GATE_B_PASS`.

The first browser sample remains invalid exactly as described above and in
`R072_BROWSER_CONFIRMATION_PROTOCOL.md`. Its separately preregistered,
entirely fresh five-pair confirmation is
`target/jit-policy-traces/wanix-r072-cb7ea816-chrome-20260809-config-ab-confirmation/analysis.json`
with SHA-256
`50e36b53eb1037fb76a81648ba6c20e7f2a491262a4066ab3472f9e25b58a4cb`.
All artifact, chronology, guest-output, policy, generated-coverage, and static
execution proofs pass. Candidate/control paired medians are 1.029x for Python,
0.990x for SHA-256, and 1.041x for shared 9P. Shared 9P therefore misses the
frozen maximum 1.03x ratio. Its exact paired-median interval `[0.829,1.268]`
shows the phase is noisy, but interval width was not the frozen decision rule.

R072 is rejected without pooling the invalid predecessor, replacing a leg,
running the candidate/v86 browser gate, enabling the mechanism by default, or
running the authoritative scorecard. The architecture mechanism and evidence
remain default-off research assets. Accepted production and the authoritative
goal status remain R054: 11/13 against v86 and 13/13 against legacy.
