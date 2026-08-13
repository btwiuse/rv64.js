# R062 loop-carried instruction backing protocol

Date: 2026-08-08  
Status: default-off product prototype; timing frozen

## Mechanism and distinction

Carry one execute-proved 4 KiB instruction-page backing in local state for the
duration of `Cpu::run` or `Cpu::run_until`. A successful translation yields
either a stable direct RAM offset or the ordinary physical-address difference.
Subsequent instructions on the same virtual page use that backing without
another TLB/tag/table lookup. The backing is discarded on page change,
mapping-generation change, execute-privilege-context change, interrupt, or
exception. A 32-bit instruction split at page offset `0xffe` translates its
second half independently.

This is not another R020/R055 cache. Those candidates stored a capability in
`Cpu` and probed its tag/context on every scalar fetch. R062 follows the exact
copy/v86 execution contract: resolve once at entry, carry the backing in the
surrounding interpreter loop, and re-resolve only at a semantic boundary. It
adds no address, opcode, guest, workload, or engine selector.

## Frozen artifacts and controls

- Accepted source baseline:
  `4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`
- Same-Wasm prototype:
  `target/bench/r062-carried-fetch/rv64-carried-fetch.wasm`
- Prototype SHA-256:
  `1bca1e1207fcb4b162be6724aedfebc55f6e43ac47c2c230e9ab416cdd961978`
- Control: `SCORECARD_V2_INTERPRETER_CARRIED_FETCH=0`
- Candidate: `SCORECARD_V2_INTERPRETER_CARRIED_FETCH=1`

The default is off. No policy threshold, page size, capacity, refill heuristic,
or instruction-family switch exists.

## Correctness gate

Before product timing:

1. core tests must compare enabled/disabled state across compressed execution,
   a 32-bit `0xffe` page split, successor-page refill, exact `run_until` stop,
   and direct backing;
2. standard full-system memory, Sv39/MPRV, atomic/T2, and direct/OpenSBI Linux
   tests must pass with the candidate enabled; and
3. Wasm import/export ABI, JIT retirement, generated-module validation, and
   modern Linux boot must remain exact.

Any mismatch removes the prototype before timing.

## Frozen product gate

First run two alternating fresh-process Boot/Compile pairs only as a gross
screen. Stop immediately for a correctness problem, more than 5% regression
on either row, or less than 1.05x improvement on both rows.

Otherwise run five total pairs on CPUs 8-15 using the immutable same-Wasm
configuration harness. Advancement requires:

- valid host probes and exact guest/input/output/JIT-policy fingerprints;
- Boot or Compile paired median speedup at least 1.10x with paired-bootstrap
  95% lower bound above 1.05x; and
- no more than 3% regression on the other failing row.

Promotion then requires the complete correctness matrix, an exact rebuilt
default-on artifact A/B, the untouched 13-row three-way scorecard, and the
fresh Chrome `/shared/bench.py` guard. Final goal completion still requires
both Boot and Compile parity with copy/v86.
