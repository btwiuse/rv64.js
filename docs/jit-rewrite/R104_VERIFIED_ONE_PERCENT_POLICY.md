# R104 Verified One-Percent Cumulative-Gain Policy

Date: 2026-08-10  
Status: accepted prospectively; no candidate or parity credit

## Decision

The former 3% whole-row promotion floor is too coarse for the mature rewrite.
General improvements compound, and a fixed byte count or a requirement that
every change recover a large fraction of the remaining parity gap has no
causal basis. Future independently admitted candidates may therefore use a
verified 1% cumulative-gain track.

This changes the economic floor, not the correctness, generality, evidence, or
product-integration standards. A point estimate above 1% is not by itself an
accepted gain.

## Frozen native rule

Before timing, every experiment names exactly one target row and all protected
rows. The implementation, pair order, initial and maximum pair counts,
statistic, confidence construction, host limits, artifact identities, and
escalation order are immutable once the first candidate sample begins.

A candidate clears its native target only when:

1. the order-balanced paired-median speedup is at least `1.01x`;
2. the deterministic paired-bootstrap 95% median interval has lower bound at
   least `1.00x`;
3. normalized throughput agrees with elapsed time for a fixed-work CPU row;
4. every protected native row has paired-median speedup at least `0.99x`, and
   no confidence interval establishes a regression larger than 1% (its upper
   endpoint must be at least `0.99x`); and
5. every input, output, generated-execution, cadence, policy, host-stability,
   and correctness proof passes.

The one-percent protected-row allowance is a noise/non-inferiority boundary,
not performance credit. A candidate that trades more than 1% of Boot for
Compile, or vice versa, is not called net positive without a separately frozen
product weighting policy. No such weighting policy currently exists.

## Power and inconclusive results

Five or seven fresh pairs can establish a large low-variance effect, but recent
Compile paired log-ratio variation is too high to resolve every true 1% effect.
For a prospective 1--3% candidate, use control/control evidence collected
before candidate timing to choose one of these paths:

- lengthen the exact fixed work while preserving inputs and output proofs;
- preregister enough fresh pairs for the desired power; or
- freeze one automatic extension from an initial count to a maximum count.

The extension decision may depend only on the prospectively stated rule. It
may not replace a leg, change order, pool an older experiment, or stop at the
first favorable interval. If the maximum sample remains unresolved, record
the candidate as **inconclusive**, not as an accepted gain and not as proof
that the mechanism is intrinsically neutral.

## Proxy-gate guardrail

Static exposure, operator counts, local microbenchmarks, native shape, and
artifact size are useful for deciding whether an implementation is plausible
and for explaining its result. They are not substitute promotion metrics. In
particular, a 5%, 10%, or 20% proxy threshold must not recreate the former
coarse runtime floor and veto a candidate that can still credibly deliver a
verified 1% end-to-end product gain.

A premeasurement gate may stop work when it proves a complete removable-cost
ceiling below 1%, exposes a correctness or architecture violation, or shows
that the proposed mechanism cannot be measured faithfully. Otherwise, when a
bounded general implementation remains practical, prefer the actual product
measurement and decide it with the frozen native and product rules below.
Proxy improvements and byte reductions remain evidence, never performance
credit; proxy misses alone do not establish a runtime regression.

## Product rule

Native causality remains only the first performance gate. Promotion still
requires, in order:

1. complete focused and strict correctness;
2. measured cold construction/instantiation with code and section sizes kept
   as diagnostics rather than hard rejection proxies;
3. fresh Chromium execution-Boot confirmation when Boot can be affected;
4. the R094-qualified fixed-work WANIX guard, including
   `python /shared/bench.py`;
5. no protected browser phase more than 1% slower by paired median and no
   confidence evidence of a regression larger than 1%; and
6. the untouched corrected-cadence 117-trial legacy/rewrite/v86 scorecard,
   retaining 13/13 versus legacy, at least 11/13 versus v86, and the measured
   target gain without a protected-row regression.

No guest PC, symbol, binary, compiler output, benchmark, checksum, browser,
engine identity, or observed result may select the optimization. Actual cold
latency, memory, lifecycle, and execution effects decide whether added code is
acceptable; source or Wasm byte count alone does not.

## Historical audit

The audit does not retroactively promote an old artifact.

- R071 is the clearest old threshold casualty: Boot was `1.0241x` with exact
  interval `[1.0003,1.0498]`, Compile `0.9984x`, and Python `1.0320x`. Under
  this rule it would have advanced beyond the native screen. Its linked static
  implementation is not a current product candidate because R078 later proved
  that the surrounding dormant machinery materially poisoned the baseline.
- R014's balanced Compile result was about 3%, but coverage strata were
  bimodal and it lacks current product guards. It is admission evidence only.
- R016 and R037 were independently reconstructed as R092 and R100. R092
  regressed shared 9P materially; R100 produced only an unresolved `1.017x`
  Compile point estimate. Neither is promoted by this policy.
- R056 was implemented later as R089 and clearly regressed Boot. R064, R066,
  R089, and R098 each damaged a protected row. R063's real scheduler-cadence
  benefit was ultimately incorporated as the R087 harness correction.

R093 is handled separately by R105. It is not retroactively accepted: R105
freezes one fresh current-baseline reconstruction and uses no historical timing
in its decision.

## Historical-audit closure through R115

The audit found real experiments that the former coarse gate stopped despite
small favorable point estimates, principally R071 and R014. That establishes
that the old rule was too blunt; it does not establish that either old artifact
was promotable.

- R071's fresh native Boot result would have advanced under this policy, but
  R078 later showed that its surrounding dormant implementation contaminated
  the baseline. Its old artifact therefore cannot be accepted as a clean net
  gain.
- R014 was the strongest independently reconstructable threshold casualty.
  R114 rebuilt only its lazy architectural-PC component on the exact current
  product, with matched generated coverage and no reuse of old timing. The
  debit-adjusted Compile result is `0.98579x` with interval
  `[0.95202,0.99629]`, an established regression. The candidate was removed.
- R115 removed R095's cross-instance confound without altering its already
  exhaustive-tested executor. Same-instance ownership recovers a verified
  3.4% Boot versus the external instance, but enabled versus disabled on the
  identical embedded artifact is `0.97987x` Boot with upper interval `0.99572`.
  Thus the packaging diagnosis is real while the underlying executor still
  fails product non-regression.

Accordingly, no historical artifact is now known to deserve retroactive
promotion. The prospective rule remains: accept a measured, end-to-end net
gain above 1% when its lower confidence bound is nonnegative, normalized work
agrees, and protected product paths pass. Do not reject such a candidate for
being only 1--5%, for source/Wasm size alone, or for failing to close a large
fraction of the remaining parity gap.

## Historical-audit closure through R118

R059 was a further testing-method casualty, although not a percentage-threshold
casualty. Its flat RV64C model had stable favorable steady calls, but the old
gate rejected the entire result because a warmup call crossed V8's tier-
publication boundary. Current methodology separates FIRST, PRIME/tier-up, and
STEADY, so R118 reconstructed the exact architecture-wide selector on the
current product without crediting R059's timing.

R118 passes deterministic shape, exhaustive RVC semantics, full strict
correctness, and modern Linux. Its fresh product result is nevertheless
negative: construction-debited Boot is `0.982183x` with interval
`[0.968403,0.993110]` and normalized work `0.982098x`. The candidate was
removed. This confirms that historical gate flaws should trigger fair modern
retests, not automatic promotion.

The rewrite-era implemented-candidate audit still finds no old artifact that
should now be accepted.
R063's scheduler finding was incorporated as the later R087 harness correction;
R071/R014/R059 were retested or causally closed by R078/R114/R115/R118; and
the remaining small favorable points either regress a protected product path,
fail browser/WANIX/scorecard confirmation, or remain statistically unresolved.

## Expanded ledger audit after R118

The broader ledger includes the deleted legacy-backend E-series as well as the
rewrite R-series. That expansion found two further percentage-cutoff casualties
which the earlier wording did not call out:

- E005b disabled the legacy per-trace TLB page cache and reported a `1.045x`
  Compile point result in one serial pair. It was labeled a tie solely because
  it missed the old 10% threshold.
- E006b lowered the legacy ordinary tier threshold from 64 to 32 and reported a
  `1.026x` Compile point result in one serial pair. It too was stopped because
  it missed 10%; neighboring thresholds 128 and 16 were negative.

Those observations prove that useful-looking small leads were discarded, but
neither is verified promotion evidence: each has one pair, no confidence
interval, no protected-row matrix, and no browser/WANIX/scorecard guard. Both
also belong to the removed `rv64-jit` backend. The rewrite's production page
policy does not use the old ordinary tier-up path, and its current invocation
cache is not the old per-trace cache. Therefore do not transplant either knob
or retroactively accept either artifact.

If current attribution identifies a genuinely equivalent *active* cost, the
historical result may motivate one independently frozen current-baseline test.
That test receives no credit from the old timing and is decided entirely by
the verified-one-percent product rule. This correction changes the scope of
the audit, not its present product conclusion: no archived artifact is known
to be promotable now.

## Exact R100 closure through R120

R100 was the remaining exact rewrite artifact whose old result most directly
straddled the new economic floor: five pairs gave a `1.017x` Compile point, but
the then-current rule required `1.03x`. R120 therefore authenticated and built
the immutable interleaved fused-TLB candidate twice, froze a fresh 15-pair
decision in advance, charged R107 construction, and excluded every old sample.

The valid new result is not positive. Debit-adjusted Compile is `0.992069x`
with interval `[0.952178,1.015084]`, and normalized work is `0.992040x`.
Protected Python is `0.982841x [0.954431,1.033982]`; Boot is
`1.003286x [1.001075,1.013737]`. The candidate fails all Compile target rules
and Python's protected median, so it stops before product reapplication.

This closes the exact R100 artifact without weakening the policy. It deserved
remeasurement because the former percentage floor was too coarse, but its old
small point was not repeatable. The standing rule remains a verified net 1%
gain—not automatic acceptance of any observed point above 1%.

## Owner clarification after R125

R125 exposed that the previous implementation of "no confidence evidence of
regression" was stricter than the stated one-percent materiality boundary. Its
shell paired median was `0.996257x`, safely above `0.99x`, yet the analyzer
rejected it because the interval upper endpoint `0.998082x` was below exact
parity. That rule made every consistently measurable sub-1% slowdown a veto
and rendered the separate `0.99x` protected median ineffective.

The owner has clarified the intended cumulative-gain policy before R124's
untouched scorecard: a verified target improvement of at least 1% is retained
when correctness and construction hold and no protected row regresses more
than 1%. A smaller confidence-established slowdown is reported but is not an
automatic veto. For confidence evidence to establish a material regression,
the interval upper endpoint must fall below `0.99x`.

This does not modify R125's frozen protocol, samples, analyzer output, or
historical verdict. It is an explicit product-policy supersession. R125's
valid integration evidence is accepted for escalation, and exact R124
candidate `d017a10f...` advances unchanged to the still-untouched scorecard.
