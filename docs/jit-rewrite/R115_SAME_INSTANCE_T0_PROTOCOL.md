# R115 same-instance Tier-0 protocol

## Question

R095's guest-independent hand-emitted scalar Tier-0 passed exhaustive ISA and
modern-Linux correctness, but its product candidate ran from a second
`WebAssembly.Instance` and lost 5.6% on Boot. R115 asks one bounded question:
was that loss caused materially by the cross-instance call boundary?

This is an architecture-general packaging experiment. It does not specialize
for a benchmark, guest binary, guest PC, kernel, or root filesystem.

## Frozen mechanism

The diagnostic linker appends the four already-frozen R095 auxiliary
functions and its passive 65,536-entry RVC expansion table to the R095 main
module. It performs only index-space relocation:

- auxiliary types are appended to the main type space;
- auxiliary direct calls to its eight imports are remapped to the matching
  existing main-module exports;
- its four defined functions are appended to the main function and code
  sections;
- its passive data segment is appended and `memory.init` is remapped to that
  data index;
- `r115_static_t0_run` and `r115_static_t0_initialize` expose the appended
  functions for the diagnostic publisher.

No instruction decoder, execution semantic, slow-path batching rule, policy,
threshold, guest input, or ordinary generated-code path changes. No variants
will be tried inside R115.

Frozen artifacts:

- embedded main: 4,439,939 bytes,
  `a4c0c34f67e4e78a57af372746ea1917f78ea69d8bf5094378cb96b3ee0dfe82`;
- source R095 main:
  `24d9e0ac94896ba88530c0340a7ec8fcc6db859e7c5b94284bdba82d5166467e`;
- source R095 auxiliary:
  `2be7aab637b0105fbe1e8a8b515263c0fa1f7b419b8329b829ebdd5b7ea563f5`;
- diagnostic loader:
  `4dad0ea90b4d6829ec8d5308fc720cf5c00ede8954af79db5fb4284ebfb9d636`;
- diagnostic worker:
  `9442e485f1f4efc72aaf74c172c0c850d979173ec55aa8cae167984f9af2dcd9`.

Both timing legs use the same embedded main, loader, precompiled auxiliary,
modern Linux 6.12.7 kernel, Alpine 3.24.1 initramfs, runtime policy, and
ordinary JIT. Both enable Tier-0. The sole leg difference is which exact
`run` function is published into the same main table slot:

- control: the R095 function from the auxiliary instance;
- candidate: its relocated byte-equivalent function in the main instance.

The auxiliary is loaded and compiled on both legs, so lifecycle construction
is balanced and excluded from the scored phase.

## Frozen boundary screen

Run Boot, Compile STEADY, and Python STEADY in alternating order with seven
fresh-process pairs per row. Preserve the scorecard's native host probe,
modern-guest identity checks, exact input hashes, output fingerprints,
generated-execution proof, work counters, and maximum-spread checks.

The boundary hypothesis is established only if:

1. the report is measurement-valid and host spread is at most 1.10x;
2. all 42 legs complete with identical inputs and outputs;
3. both sides execute Tier-0 with zero errors and comparable retired work;
4. Boot's paired candidate speedup median is at least 1.01x and its exact
   nonparametric 95% median interval lower endpoint is at least 1.00x;
5. neither Compile nor Python has a paired median below 0.99x.

This screen proves only the boundary mechanism. It cannot promote the old
R095 artifact. If it passes, R115 proceeds to a clean reconstruction against
the exact current baseline, followed by the normal correctness, modern-Linux,
construction-debited native, Chromium/WANIX, and full scorecard gates. Product
admission uses the project-wide rule: at least 1% verified net gain with a
95% lower bound at or above parity, no protected-row regression, and no
correctness failure. Artifact size is reported but is not an acceptance gate.

## Pre-freeze smoke observation

One unscored pair motivated the powered run: external 2,332.645 ms versus
same-instance 2,207.291 ms on Boot (1.057x), with roughly 107 million Tier-0
fast instructions and zero errors on each leg. This observation is not used
in the seven-pair decision sample.

## Frozen executor screen

If the boundary screen passes, run a second seven-pair Boot/Compile/Python
screen on the same embedded main, loader, and auxiliary lifecycle. This time
both legs select the embedded function and vary only
`SCORECARD_V2_EXTERNAL_T0_ENABLED`: zero for control, one for candidate.

The executor advances to clean current-baseline reconstruction only if all
integrity and work checks pass, neither protected row has a paired median
below 0.99x, and at least one still-open row (Boot or Compile) has a paired
median at least 1.01x with its 95% median-interval lower endpoint at or above
1.00x. Construction is identical between these two legs. A later exact-
baseline product gate must still debit any candidate-only construction cost.

This rule was written before collecting the executor-screen sample.

## Result

The linker produced a valid 4,439,939-byte main module. Direct and embedded
smoke boots both reached the modern Alpine marker, each retiring about 107
million instructions through the exact Tier-0 executor with zero errors.

The boundary report
`target/bench/r115-same-instance-proof/boundary-screen/config-ab-2026-08-10T11-14-11-804Z.json`
(`e3e738305021...`) is measurement-valid. All 42 legs completed, host spread
was 1.072156x, paired guest work differed by less than 0.04%, and inputs,
outputs, policies, ordinary generated execution, and Tier-0 execution proofs
matched. Same-instance versus external-instance results were:

- Boot `1.03413x`, interval `[1.01796,1.04451]`;
- Compile `1.02349x`, interval `[0.99847,1.03595]`; and
- Python `1.00661x`, interval `[0.95828,1.02686]`.

The frozen boundary screen therefore passes. R095 paid a real, broad
cross-instance execution tax; its old rejection did not prove the emitted
Tier-0 body itself was 5.6% slow.

The executor report
`target/bench/r115-same-instance-proof/executor-screen/config-ab-2026-08-10T11-21-26-472Z.json`
(`03d22b571b47...`) then compared the embedded executor enabled versus disabled
on the identical artifact. All 42 legs completed with correct work and zero
errors; host spread was 1.037174x. One Compile control sample made that row's
spread 1.28577x, just beyond the 1.25 report limit, so the report cannot admit
a positive candidate. The complete Boot subset is independently stable
(1.06284x/1.03101x side spreads) and decisively negative:

- Boot `0.97987x`, interval `[0.95574,0.99572]`;
- Compile `0.99715x`, interval `[0.94090,1.02718]`; and
- Python `1.00955x`, interval `[0.96845,1.01703]`.

No open row clears the verified-1% rule, and Boot's upper interval endpoint is
below parity. The executor is rejected without reconstruction on the current
product, construction, browser, WANIX, or full-scorecard work. This rejection
is based on execution, not the 160,559-byte net diagnostic-module increase.

Gate `target/bench/r115-same-instance-proof/gate.json`
(`2f8319b5ab3f...`) records
`cross-instance-tax-established; embedded-executor-rejected`. Authenticated
source, artifacts, and raw reports are under
`target/bench/r115-same-instance-proof/`. The live source and release product
were never changed and remain exact `d9f686a9...`.
