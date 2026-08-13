# R118 flat RV64C current-baseline result

Date: 2026-08-10  
Status: rejected at frozen native Gate B; candidate removed

## Outcome

The current-baseline reconstruction of R059 does not improve Boot. Its exact
15-pair Boot subset is stable, but the candidate is slower and the entire 95%
paired-median interval is below parity. R118 therefore stops before Chromium,
WANIX, `/shared/bench.py`, or the authoritative three-way scorecard. There is
no rerun, sample replacement, selector/layout variant, or threshold change.

This rejection is unrelated to source or Wasm size. The candidate's diagnostic
release-Wasm growth is only 1,354 bytes and was never a gate.

## Frozen candidate and correctness

- Control: `d9f686a9ce4f...`, 4,279,380 bytes.
- Candidate: `0501207f314f...`, 4,280,734 bytes. Two independent builds are
  byte-identical.
- Candidate CPU source: `a0ac3777e5e7...`.
- Shape report: `3d2f10034f8e...`. All six `Cpu::step` specializations replace
  initial table lengths `4,9,9,9` with one length-25 table, remove exactly
  three `br_table` operators, preserve call topology, and leave every other
  defined function body byte-identical.
- The exhaustive 65,536-encoding direct-RVC/reference test passes.
- The strict release suite passes 134/134 ISA, 109/109 Spike lockstep, 193/193
  architecture-signature tests, every differential/API/raw-Wasm test, direct
  and OpenSBI Linux 6.12.7 with generated execution, and virt-smoke.

The first structural build without an explicit compressed-prefix precondition
retained a cold `panic_fmt` call and was disqualified before timing. Making the
already-existing caller invariant explicit removed the call while preserving
the impossible fallback. Both superseded artifacts are retained and marked
forbidden from performance use.

## Construction debit

The valid 15 alternating fresh-process `RV64Debug.create` pairs use CPUs 8--15
and the exact loader `2cbb264f4dac...`:

- control median: 20.672769 ms;
- candidate median: 20.559315 ms;
- paired candidate-minus-control median: +0.178117 ms;
- paired 95% interval: `[-0.405668,1.225862]` ms; and
- conservative R107 debit: 1.225862 ms.

Candidate construction receives no credit for its lower unpaired median; the
positive upper confidence endpoint is charged once to every candidate row.
The report is `construction.json`, SHA-256 `ad5d1fb1cb47...`.

## Native result

The immutable run retains all 90 legs: 15 alternating fresh-process pairs for
Boot, Compile, and Python under the production page policy and one public
slice per event-loop turn. Artifact, guest, input, output, generated-execution,
affinity, and host proofs pass. Host-probe spread is 1.019653x. One control
Compile sample makes that side's spread 1.297506x, above the frozen 1.25x
limit, so the complete report is invalid for positive admission.

The debit-adjusted mechanical gate reports:

| Row | Paired median | 95% interval | Normalized work | Decision role |
|---|---:|---:|---:|---|
| Boot | 0.982183x | [0.968403, 0.993110] | 0.982098x | target fails decisively |
| Compile | 0.996783x | [0.987467, 1.031246] | 0.996706x | protected point passes; report has spread violation |
| Python | 1.022471x | [0.997670, 1.040330] | 1.022405x | protected passes; favorable point unresolved |

Boot's control/candidate medians before debit are 2,170.919/2,206.083 ms and
its raw paired result is `0.982719x [0.968926,0.993667]`. Thus the target
subset alone establishes regression; the Compile outlier cannot plausibly
hide an R118 win and does not justify a rerun.

The native report is `native/config-ab-2026-08-10T12-39-36-568Z.json`,
SHA-256 `d89bc21c8ddf...`. The mechanical gate is `native-gate.json`, SHA-256
`d0817ba5cdd7...`.

## Decision and restoration

Reject the unique flat 24-family selector. Do not reorder families, use an
opcode-frequency layout, change the selector spelling, or combine it with
another mechanism after seeing these results. R059's old local model exposed a
real methodology flaw—tier publication must not be treated as steady-state
instability—but its fresh full-product hypothesis is false for current Boot.

The candidate source has been removed. Live identities are restored exactly:

- `crates/rv64-core/src/cpu.rs`: `aec4b31434a6...`;
- `crates/rv64-wasm/src/lib.rs`: `1da35e70bc9c...`;
- `web/rv64.js`: `2cbb264f4dac...`; and
- release Wasm: `d9f686a9ce4f...`, 4,279,380 bytes.

The historical audit therefore still finds no old implementation that merits
retroactive promotion. It does validate the prospective rule: accept a
reproducible net end-to-end gain of at least 1% with confidence and protected
rows intact; do not reject it for small code growth or because it is below a
formerly convenient 10%/20% threshold.
