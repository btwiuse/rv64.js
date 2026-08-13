# R066 Integrated Scalar T0 Protocol

## Hypothesis

Boot is the remaining genuine emulator bottleneck. R065 measures nearly equal
guest work (180.816M RV64 versus 183.982M i686 instructions), but only 78.964
versus 120.827 MIPS. The accepted post-R054 profile attributes about half of
the whole row to non-inlined `Cpu::step`, with additional material self time in
`Cpu::run`/`run_until`.

R066 tests one broad execution-shape change: execute the complete ordinary
RV64 integer/control/scalar-memory families directly inside the interpreter
driver while carrying architectural PC and retirement in Wasm locals across
the stretch. Flush those locals before interrupt delivery, a slow-family
fallback, an exception, an architectural stop, or return to the caller. The
unchanged `Cpu::step` remains authoritative for F/D, A, FENCE, SYSTEM/CSR, and
compressed floating-point instructions.

This is distinct from earlier closures:

- R023 duplicated the same complete giant decoder but still materialized the
  ordinary per-step result/state contract;
- R025-R027 and R045 cached decoded instructions behind per-instruction
  handler dispatch;
- R042 only outlined cold families while preserving `Cpu::step` as the hot
  call boundary;
- R058 changed the non-inlined result ABI without integrating execution;
- R059 changed only compressed selector layout.

R066 caches no instruction, address, translation, PC, binary, symbol, browser,
or workload identity. Its family set is the ISA-defined RV64I/M scalar path,
including every integer load/store width and every integer RVC form. Slow
families always execute the pre-existing complete decoder.

## Opportunity gate

The independent R041 exact histogram counted 115,272,067 interpreted Boot
instructions. The proposed complete scalar set covers 114,121,267 (99.002%).
Integer/control forms that never touch guest data memory alone cover 64.183%;
ordinary scalar memory adds 34.818%. The post-R054 profile places 49.89% of
Boot in `Cpu::step` self before counting driver self time. Even the conservative
49.89% category needs only a 1.251x local speedup to project a 10% whole-row
gain. The dynamic coverage and profile attribution therefore admit one product
prototype; frequency-selected subsets and sub-family sweeps are forbidden.

## Frozen implementation and controls

- Control is accepted R054 Wasm SHA-256
  `4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
- The prototype is default-off in one candidate Wasm and selected only by
  `jit_set_integrated_scalar_t0(0|1)` before boot.
- Both legs use the same candidate Wasm, modern Linux 6.12.7 / Alpine 3.24.1,
  production page policy, and unchanged scorecard cadence. Generated module
  bytes and selection are not changed.

## Correctness gates

Before timing:

1. Add exhaustive/directed differentials for all fast 32-bit opcode families
   and all 65,536 compressed encodings, comparing PC, GPRs, memory, exception,
   stop, and retirement with `Cpu::step`.
2. Exercise fetch/data faults, page-straddling 32-bit instructions, unaligned
   and cross-page scalar memory, x0 writes, division edge cases, control links,
   interrupt delivery, and slow-family fallback.
3. Pass all `rv64-core` units, full-system memory/Sv39/MPRV/atomic/FP/T2
   differentials, Wasm smoke, and direct/OpenSBI modern Linux with the switch
   both off and on where applicable.

## Performance gates

1. Run one same-Wasm Boot engineering pair. Stop and remove immediately on a
   correctness difference or greater than 5% regression.
2. If non-regressing, run three alternating fresh-process pairs for Boot,
   Compile, and Python, preserving input/output fingerprints, guest retirement,
   generated execution proof, raw samples, and host probes.
3. Advance only if Boot paired median is at least 1.10x with paired-bootstrap
   lower bound at least 1.00x, while Compile and Python are each at least 0.90x.
4. Confirm five Boot/Compile pairs, then run the browser `/shared/bench.py`
   guard. No phase may regress 10%.
5. Only a passing candidate becomes default-on and earns the complete
   authoritative 13-row rewrite/legacy/v86 scorecard.

If the first product screen fails, remove every core/runtime/worker switch and
restore exact R054. Do not tune family membership, privilege mode, PCs,
fallback frequency, or workload-derived opcode combinations.

## Result

R066 is rejected at the frozen product gate and completely removed.

The default-off same-Wasm candidate is archived at
`target/bench/r066-integrated-scalar-t0/rv64-integrated-scalar-t0.wasm`, SHA-256
`d2ac8852eaf1950c55042cd8109a6e2f43aae2154a019ea6ee8c637aa628fdce`
(4,311,878 bytes). Before timing it matched the authoritative decoder for all
49,152 actual 16-bit compressed-prefix encodings, a 22,528-case randomized
corpus spanning every admitted 32-bit opcode, and directed M-extension edge
cases. All 35 candidate core tests passed. The complete Wasm smoke suite passed
with the switch both off and on. With the candidate forced on, the full-system
memory exits, bulk copy, Sv39/MPRV, atomic, FP/FS, WFI, page-policy, T2,
direct-Linux, OpenSBI-Linux, FP-context-switch, and AMO differentials all
passed.

The valid two-pair engineering screen is
`target/bench/r066-integrated-scalar-t0-engineering-valid/config-ab-2026-08-09T02-40-26-188Z.json`.
Boot improved from a 2,297.92 ms median to 2,214.88 ms, paired 1.039x with
interval `[1.002,1.075]`, so the non-regression rule admitted the frozen
three-row screen. An earlier report under
`target/bench/r066-integrated-scalar-t0-engineering/` is invalid because it
pointed `ARTIFACTS` at a directory without the modern initramfs; it contains no
guest measurements and is retained rather than silently replaced.

The valid three-pair report is
`target/bench/r066-integrated-scalar-t0-screen/config-ab-2026-08-09T02-43-43-436Z.json`.
It used identical candidate bytes in both legs, exact input/output
fingerprints, CPUs 8-15, and a 1.022x host-probe spread:

- Boot: 2,312.91 -> 2,177.30 ms by raw medians; paired speedup 1.074x,
  interval `[1.060,1.129]`;
- Compile STEADY: 1,055.83 -> 1,067.92 ms; paired speedup 0.974x,
  interval `[0.959,0.996]`;
- Python STEADY: 3,024.05 -> 3,063.13 ms; paired speedup 0.987x,
  interval `[0.947,0.999]`.

The Boot gain is real but misses the predeclared 1.10x advancement threshold;
Compile and Python also move slightly backward. Per protocol there is no
family-membership, privilege-mode, PC, or fallback-frequency tuning and no
five-pair/browser/full-scorecard escalation. Every core/runtime/export/worker
hook and candidate-only test switch was removed. The rebuilt production Wasm
is byte-identical to accepted R054 SHA
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`
at 4,272,517 bytes, and all 32 production core tests pass.
