# R122 Current-Product Interpreter-Body Attribution Result

Date: 2026-08-10  
Status: diagnostic complete; no product candidate admitted; exact product restored

## Outcome

R122 does not admit a product edit. The immutable native profile confirms that
the two semantic bodies are large—30.2562% of main-thread period together—but
no independent architecture-wide removable operation satisfies both frozen
requirements: at least 1.25% authenticated native exposure and a realistic
construction- and overhead-adjusted projection of at least `1.01x` whole Boot.

This is not a rejection of a measured small gain. No candidate was timed. Code
size was not considered. The result is an attribution stop: every qualifying
native block is required dispatch, belongs to an already closed mechanism, or
contains only a much smaller removable subset.

## Stage A: immutable native closure

The deterministic analyzer consumes the preserved uninstrumented R119 perf
capture and optimized `Cpu::step` DSO. Two independent final reports are byte
identical at SHA-256 `40f70f2eaa6b...`. All `5,440,912,104` sampled
`Cpu::step` period closes exactly; the function owns 51.4460% of main-thread
period.

The original draft reports (`e7d9cebabb1d...`) used the same immutable perf
capture but a permissive objdump parser incorrectly treated five byte-only
continuation lines as instructions. The corrected analyzer rejects lines whose
parsed mnemonic is itself a two-digit byte. It changes decoded-instruction
count from 4,311 to 4,228, but does not change a symbol boundary, basic-block
boundary, sampled period, operation band, hot block, exposure, or decision.
Both draft reports remain authenticated as history; the `native-census-final-*`
reports and `native-census-final.mjs` are authoritative.

| Native operation | Main-thread period |
|---|---:|
| entry and execute TLB | 7.8920% |
| physical fetch | 2.5552% |
| length/RVC dispatch | 4.9618% |
| compressed semantic body | 16.1968% |
| RV32 semantic body | 14.0594% |
| both architectural-PC stores | 0.2930% |
| both common result/outcome tails | 3.7573% |
| both retirement tails | 1.0001% |
| both return sequences | 0.7305% |

Only five semantic basic blocks independently reach the 1.25% native evidence
floor:

| Offset | Main-thread period | Classification and decision |
|---:|---:|---|
| `0x188a` | 1.7523% | Required 32-bit assembly and already-dense opcode dispatch; no removable independent operation. |
| `0x0be4` | 1.5673% | Required `bit12` selection inside the `C.JR/MV/EBREAK/JALR/ADD` family; another selector/order rewrite is closed by R118. |
| `0x094f` | 1.4117% | Compressed address formation and scalar memory path; fused interpreter memory is already the accepted R054 implementation. |
| `0x1112` | 1.3996% | Quadrant-1 register, immediate, and family dispatch combined; only a minority subset is potentially avoidable. |
| `0x0d35` | 1.2717% | Required `rd == 0` legality/family selection inside the same quadrant-2 group; not an architecture-wide removable operation. |

The quadrant-1 census makes the partial-decode ceiling explicit. Only
7,281,249 of 24,170,756 quadrant-1 attempts (30.1242%) are `C.J`, `C.BEQZ`, or
`C.BNEZ` forms that do not consume the common six-bit immediate. Even charging
that fraction the *entire* `0x1112` block gives an intentionally impossible
0.4216% main-thread ceiling; real removal is smaller and moving the decode into
cases adds duplicated code. It cannot project a 1% Boot gain.

The result/outcome population is already closed by R058's compact scalar
return experiment, whose local result was `0.477x` of the ordinary ABI. The
1.0001% retirement population is below R122's evidence floor and the broader
integrated scalar-loop form is closed by R105, which improved Boot 5.88% but
failed protected Compile/Python. PC stores and return sequences are smaller.

## Stage B: one modern-Boot counter census

The sole counter-observed scorecard run uses Linux 6.12.7, Alpine 3.24.1,
riscv64, the production page policy, and public one-slice-per-turn cadence.
The harness labels the run measurement-ineligible and the report marks every
elapsed field excluded. The instrumented Wasm is
`1dfb3eacb282...`; the report is `4e93b96caa5c...`.

All independent closures pass:

- 108,693,790 interpreted plus 71,704,074 generated retirements equals
  180,397,864 total guest retirements;
- 67,576,293 compressed plus 41,117,497 32-bit retirements equals the harness's
  interpreter total; and
- 91,387,416 sequential plus 17,306,374 non-sequential successors equals the
  same interpreter total.

The observed interpreter mix is 62.1713% compressed and 37.8287% 32-bit.
Decoded non-retiring attempts are only 140 and 165 respectively. Generated
execution is proved and owns 39.7477% of Boot retirement; the interpreter owns
60.2523%.

The common GPR-write helper executes 66,630,535 times, but only 730,168 calls
(1.0958%) discard `rd == x0`. This does not support a cheaper general x0
representation. Always writing and restoring x0 would add one store to all
108.7 million interpreted retirements; a 33rd scratch slot still needs a
comparison/select/address sequence on all 66.6 million writes. Both add more
general work than the current highly predictable test/branch they replace.

Opcode and compressed-family counts are retained as descriptive closure only.
They did not select an opcode-specialized candidate, family ordering, binary,
guest PC, workload, or engine variant.

## Target-scope note

The R122 census itself used only the approved modern Linux 6.12.7 / Alpine
3.24.1 path. The generic Wasm smoke command was also run while the diagnostic
artifact was present; that command includes an unrelated legacy BBL/TinyEMU
leg. Its output and elapsed behavior are explicitly outside R122 evidence and
outside the project target. Raw-Wasm ABI, generated-module validity, exact
retirement, and the modern scorecard Boot contract passed independently.

## Evidence and restoration

Evidence is under `target/bench/r122-interpreter-body/`:

- authoritative `native-census-final-a.json` and
  `native-census-final-b.json`, plus the corrected analyzer snapshot;
- superseded draft `native-census-a.json` and `native-census-b.json`, retained
  to make the parser correction auditable;
- `counter-census/counter-census.json`;
- the exact instrumented Wasm and source snapshot; and
- a verified `SHA256SUMS` manifest.

All hot counters, the temporary export, and harness plumbing were removed.
The 32 core tests and scorecard harness self-test pass. Rebuilt live identities
are exact:

- core source `aec4b31434a6...`;
- Wasm source `1da35e70bc9c...`;
- loader `2cbb264f4dac...`; and
- release Wasm `d9f686a9ce4f...` (4,279,380 bytes).

R122 stops before construction timing, native A/B, Chromium, WANIX,
`/shared/bench.py`, or the scorecard because there is no admitted candidate.
Official status remains 13/13 versus legacy and 11/13 versus copy/v86, with
Boot and Compile parity still open.
