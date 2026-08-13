# R113 Native Stack-Slot and Control-Context Protocol

Date: 2026-08-10  
Status: closed at frozen form gate; no local model or product change

## Question

Is R110's optimized native-stack cost concentrated in one architecture-general
native form with enough distributed whole-Compile exposure to justify a new
local model, or is it diffuse spill traffic with no bounded mechanism? R113 is
read-only. It reuses the exact R110 dump and samples, recollects nothing, and
does not infer guest/Wasm semantics from native addresses.

## Frozen inputs and closure

- perf data:
  `79abdf40afbf492c2718bb5832758df1ed2e137d18329d543c79c5c0c9196809`;
- jitdump:
  `2d10666fd9f9279cec8b9c2e580c05d925a177f05ff626e4262bf30a7999acd4`;
- R110 report:
  `4aab78cbb7a5dc2045b42abe420b42ed6ad9262a267b4e7029f137ce94ce971f`;
- R088 generated share of Compile STEADY: `0.40684`.

The analyzer must reproduce R110's total, main-thread, generated, tier, role,
native-family, guest-family, per-load period, and sample totals exactly before
its classifications are usable. It uses the same newest-containing-load join,
same code bytes, and same instruction-boundary rule. The shared 128-byte tail
trampoline and all Liftoff code are reported but excluded from admission.

## Frozen stack access forms

For every sampled TurboFan guest-body instruction whose explicit memory
operand is based on `%rbp` or `%rsp`, decode the signed displacement, base,
mnemonic, operand width when explicit, and R110 read/write/read-modify-write
direction. Assign exactly one form in this precedence order:

1. `immediate-compare-test`: `cmp`, `test`, or `bt` with an immediate and a
   stack memory operand;
2. `register-reload`: a pure move from a stack memory source to a register;
3. `register-spill`: a pure move from a register source to a stack memory
   destination;
4. `immediate-spill`: a pure move from an immediate source to stack memory;
5. `stack-rmw`: a non-move instruction that writes a stack memory operand;
6. `other-stack-read`; or
7. `other-stack-write`.

No displacement is named as a guest register, selector, PC, or proof value.
Aggregate slots are keyed by `(base, signed displacement, access width)` only;
per-load slot keys additionally include code index. Report top slots and the
top-one/top-five/top-eight concentration, but do not treat matching offsets in
different functions as the same semantic value.

## Frozen native context

Assign each stack sample one context, in this precedence order:

1. `entry-prefix`: native offset is below 512 bytes;
2. `call-neighborhood`: within eight decoded instructions before or after a
   direct or indirect call, without crossing another call, conditional branch,
   unconditional branch, return, or trap;
3. `control-neighborhood`: within two decoded instructions before or after a
   conditional or unconditional branch, without crossing a call, return, or
   trap; or
4. `general-body`.

The context is static native proximity, not a claim about dynamic control flow.
The report also records distance/direction and forms a Cartesian
`form/context` partition. Every stack period must close exactly across form,
context, form/context, slot, code load, and frame-size bucket.

## Admission gate

Predeclare these seven form families and four contexts; do not combine or
rename them after the census. A single form family can admit one separately
frozen architecture-general local model only if all of the following hold:

1. its optimistic whole-Compile exposure,
   `family_period / complete_R110_JIT_path_period * 0.40684`, is at least
   `0.02` (twice the verified 1% target);
2. it appears in at least 10 distinct TurboFan guest code loads;
3. it has at least 500 mapped hardware-cycle samples;
4. no one code load owns more than 35% of its period; and
5. its top five code loads own no more than 70% of its period.

Entry/call/control contexts are attribution only and cannot be pooled to make
a form pass. If a form passes, its local model must preserve identical useful
work and use ordinary tiered V8; passing R113 does not admit product code. If
no form passes, close native-form specialization. If multiple forms pass,
advance only the one with the largest exposure; ties use the form order above.

Two fresh analyzer invocations must produce byte-identical JSON. Synthetic
self-tests cover signed displacements, each form, context precedence, barrier
crossing, instruction boundaries, slot keys, and concentration math.

R113 cannot recollect perf, select a code load/workload/PC, infer Wasm source,
reopen R111 partition parameters or earlier closures, change product
`d9f686a9...`, or earn scorecard credit.

## Result

Three fresh analyzer invocations are byte-identical, SHA-256
`46990882175460e430183d84b6a62e3146822fbd0cbcb9d3bd16d99ad4accf91`.
The analyzer closes exactly against all R110 totals and classifies all 897
TurboFan guest-body stack samples / 1,958,954,615 period.

The traffic is not call-boundary overhead:

| Context | Stack fraction | Optimistic whole-Compile exposure |
| --- | ---: | ---: |
| entry prefix | `14.293%` | `1.160%` |
| call neighborhood | `0%` | `0%` |
| control neighborhood | `32.656%` | `2.650%` |
| general body | `53.051%` | `4.306%` |

The frozen form results are:

| Form | Samples / loads | Exposure | Gate result |
| --- | ---: | ---: | --- |
| immediate compare/test | `201 / 46` | `1.817%` | exposure and sample gates fail |
| register reload | `464 / 70` | `4.196%` | 500-sample gate fails |
| register spill | `205 / 60` | `1.858%` | exposure and sample gates fail |
| immediate spill | `6 / 2` | `0.055%` | fails |
| stack RMW | `0 / 0` | `0%` | fails |
| other stack read | `21 / 18` | `0.190%` | fails |
| other stack write | `0 / 0` | `0%` | fails |

Register reloads are broad rather than selected: the largest load owns 9.56%
and the top five own 27.85%. They nevertheless miss the frozen minimum at 464
samples. R113 therefore admits no native-form model; the threshold is not
relaxed or rounded after observing the result. This does not reject a measured
runtime gain—no candidate exists—and the optimistic 4.196% exposure is not a
claim that reloads are removable.

The dominant Cartesian rows explain the shape: register reloads in the general
body own 43.63% of stack period, immediate comparisons next to control own
21.83%, and entry-prefix register spills own 12.28%. These are symptoms of
distributed live-state pressure, consistent with R110/R111, rather than one
call-boundary operation.

Close native-form specialization without timing or product code. Preserve the
result as attribution; do not change the 500-sample rule, combine forms, select
loads, or infer guest semantics from frame slots. Exact `d9f686a9...` remains
the product baseline.
