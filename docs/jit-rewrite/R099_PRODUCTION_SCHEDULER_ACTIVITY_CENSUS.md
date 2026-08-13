# R099 Production Scheduler Activity Census

Date: 2026-08-10  
Status: complete diagnostic; no emulator change

## Question

Before changing the scheduler again, which source-visible policy paths actually
execute under the public one-slice cadence, production page policy, modern
Linux 6.12.7 / Alpine 3.24.1, and exact R085-equivalent executable code?

The clean source-built runtime is
`d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`.
The loader is `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`.
R099 only added already-exported `jit_stat(32..45)` values to the modern
scorecard result. Counter reads remain outside each timed interval; no runtime,
generated module, policy, or workload changed.

## Result

The measurement-valid one-process opportunity report is
`target/bench/r099-scheduler-opportunity/scorecard-v2-2026-08-10T04-02-12-181Z.json`.
It records exact output identities and ordinary generated execution.

Across Boot FIRST and Compile FIRST/PRIME/STEADY, every one of these counters
is zero:

- measured region exits and exit-without-profile/in-region outcomes;
- queued, issued, cooldown-deferred, and targetless region extensions;
- extension drain visits and address-space misses;
- region demotions;
- compatibility batches and batch members; and
- indirect-cache extensions.

Boot executes 581,658 outer generated dispatches. Compile executes
696,209/533,310/590,001. Thus their surrounding tests are dynamically reached,
but the policy bodies do no work. Production page policy deliberately has
measured-region extension disabled, and every generated return observed by
R090 is already a region return.

## Decision

Do not treat region-exit sampling, batching, feedback tables, or extension
draining as a broad remaining opportunity. Removing a dormant branch may be a
small layout experiment, but it has no measured operation-count leverage and
R091 already showed that optimizing the complete scheduler loop does not
convert its sampled attribution into product wall time. Move to a component
with both dynamic volume and prior local conversion evidence.

