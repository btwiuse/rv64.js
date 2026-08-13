# R068 Batched Privileged T0 Opportunity Protocol

## Purpose

R067 shows that raw Boot parity requires moving a broad part of the remaining
111.363M interpreted instructions into generated Wasm. R046 proved that an
early 200,000-instruction privileged page tier has enough static coverage, but
its 51-52 independent runtime modules accumulated 6.77-6.94 seconds of async
availability latency and made Boot 45.7% slower. R068 tests whether packaging
several unrelated privileged pages into one host compilation can retain that
coverage while removing the per-page compile storm.

This is an offline opportunity and module-geometry test. It does not publish or
execute a candidate module and cannot produce a scorecard result. No runtime
implementation is permitted unless the primary geometry clears every frozen
gate below.

## Frozen inputs and trace

- Guest: the exact modern Linux 6.12.7 / Alpine 3.24.1 direct-Boot scorecard
  artifact and readiness marker.
- Runtime: a diagnostic build derived from accepted R054. Generated execution
  is fully bypassed while tracing.
- Trace slice: 16,384 guest instructions, identical to the snapshotted R046
  opportunity capture.
- Eligibility threshold: 200,000 physical-page instructions. This is the exact
  v86 threshold and the exact R046 early-tier threshold, not a new sweep.
- Eligible pages: the hottest observed virtual mapping of each physical page,
  restricted to mappings whose dominant architectural mode is not User.
- Ordering: ascending first threshold-crossing instruction count, then physical
  page number. No PC, symbol, binary hash, process, phase, or row label enters
  selection.
- Entry seeds: observed non-fallthrough/exception/context entry events for the
  selected physical/virtual mapping, ranked by event count then PC. Code bytes
  are snapshotted during the JIT-disabled run; any observed byte conflict makes
  that page ineligible.

The diagnostic adds capture/emission data only. It must never call a generated
function or the async publication path. Its Wasm hash is measurement-ineligible
and the production R054 bytes must be restored after the decision.

## Frozen geometry

The primary candidate uses:

- eight pages per module;
- 64 discovered leaders per page;
- `MultiEntryState::Memory`, which emits bounded member functions plus one
  dispatcher instead of one register-union giant function;
- deterministic consecutive groups in threshold-crossing order. A duplicate
  virtual page starts the next group so one module never contains two bodies
  for the same architectural PC. If the deterministic tail contains one page,
  it is appended to the preceding group (which may therefore contain nine).

Four- and sixteen-page groups are sensitivity diagnostics only. A
register-structured eight-page form is a shape control only. Neither can rescue
a failed primary gate or become the runtime choice after inspecting results.

Each unique module is compiled once in a fresh Node/V8 process after guest
execution has stopped. Process startup is outside the interval. The harness
records translation time, module bytes, function count, largest encoded
function body, fresh-process `WebAssembly.compile` time, and validation. This
avoids same-process module caching and separates guest execution from host
compiler contention.

## Coverage and readiness model

For every page in a module, the earliest possible issue point is the latest
200,000-instruction threshold crossing among its members. The measured fresh
compile duration is converted to guest instructions using the JIT-disabled
trace's observed instructions per millisecond and added to that issue point.
The credited window ends at the accepted production privileged threshold of
4,194,304 page instructions: execution after that point is already eligible
for R054's generated tier and cannot be claimed by the early tier. A page's
incremental opportunity is therefore `min(final_heat, 4,194,304)` minus its
latest quantized heat at readiness, floored at zero and never beginning before
200,000. This is then multiplied by the module's exact observed-entry coverage.

The projected time saved uses the independently measured R067 rates:

```text
saved_ms = ready_covered_instructions * (1 / 62e6 - 1 / 661e6) * 1000
```

This is deliberately an optimistic admission bound: it assigns every covered
post-readiness instruction generated-tier cost and charges no execution-side
dispatcher overhead. A runtime candidate must still prove its wall result.

## Frozen admission gates

The eight-page memory-state primary is admitted only if all are true:

1. every emitted module validates and compiles in a fresh V8 process;
2. at least 95% of observed entry events on included pages are represented by
   emitted entries;
3. projected ready covered instructions are at least 15.5M, corresponding to
   at least 226 ms or 10% of accepted R054 Boot under the frozen rate model;
4. no more than 13 compile jobs cover the eligible set, at least a fourfold
   reduction from R046's 51-52 live modules;
5. no module exceeds 1 MiB and no encoded function body exceeds 256 KiB,
   avoiding R048's multi-megabyte-function late-tiering failure;
6. total emitted bytes do not exceed the corresponding 64-leader R046
   one-page total by more than 10%;
7. no trace overflow, code conflict, invalid module, or missing exact input
   fingerprint occurs.

Passing these gates admits one default-off runtime implementation. It does not
promote it. The implementation must use the same architecture-only threshold,
queue order, batch size, leader cap, and memory-state representation, retain
all existing mapping/dirty-page proofs, and then pass differential/full-system
correctness, alternating same-Wasm Boot/Compile/Python A/B, the full three-way
scorecard, and browser `/shared/bench.py` guards.

If the primary fails any gate, R068 closes batched cold-page packaging. Do not
retune the threshold, batch size, leader cap, state mode, queue ordering,
in-flight limit, or module bounds from the observed result.

## Result

The corrected report is
`target/bench/r068-batched-privileged-t0/opportunity.json`, SHA-256
`7decd1563eb17931801a98dcc7e3421442b61f4924e72db938cf7e0f5cc7b1d2`.
It uses diagnostic Wasm `97a4eb738cf5...`, exact kernel `57d077974820...`,
exact initramfs `cbb75afb016d...`, Node 26.5.0/V8 14.6, and a complete
183.000M-instruction trace with zero dropped or out-of-RAM events. One eligible
physical page reported 1,622 code conflicts and was excluded under the frozen
snapshot rule; 82 stable privileged pages remained.

The eight-page memory-state primary passes every gate:

- 11 modules cover all 82 pages, versus R046's 51-52 runtime modules;
- 26,323,551 of 26,576,717 entry events are covered (99.047%);
- the corrected readiness model credits 73,621,771 incremental instructions
  in the 200,000-to-4,194,304 early window, projecting 1,076.1 ms under the
  deliberately optimistic frozen rate model;
- total emitted bytes are 5,083,761, the largest module is 615,820 bytes, and
  the largest of 4,976 encoded function bodies is only 15,825 bytes;
- all modules validate and compile in fresh V8 processes; summed fresh compile
  time is 9.900 ms and the maximum is 1.125 ms;
- translation takes 108.807 ms after the guest has stopped and no emitted page
  carries a code conflict.

The shape controls reinforce the frozen choice. Four-page memory state retains
79.120M ready covered instructions but needs 21 jobs. Sixteen-page memory state
uses six jobs but its largest module is 1,222,786 bytes, above the 1 MiB bound.
Eight-page register-structured state emits only 11 functions but makes its
largest function 834,421 bytes and grows total bytes to 7,231,140, reproducing
the giant-function risk that R048 closed.

An earlier completed calculation credited all post-readiness execution,
including instructions already eligible for R054's normal generated tier. It
reported an impossible 139.600M newly covered instructions against only
111.363M remaining interpreted and is invalid. It was rejected before the
admission decision. The corrected model caps every page at the existing
4,194,304 privileged threshold, can only reduce opportunity, and is the sole
R068 result.

## Decision

R068 admits one default-off runtime candidate with the exact primary
configuration: 200,000 early threshold, privileged mappings only, FIFO groups
of eight, 64 leaders per page, memory-state multi-entry modules, and the
existing in-flight limit. This is an admission result, not a performance win.
The runtime candidate must preserve mapping and dirty-page proofs and clear the
full R069 correctness and paired product gates before promotion.
