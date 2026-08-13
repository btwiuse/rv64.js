# R127 Exact-R124 Scorecard Invalid Result

Date: 2026-08-10  
Status: invalid external termination; no performance credit

## Result

R127 did not produce a scorecard report. The sole formal parent was launched
with the frozen command and passed its generated-v86 preflight. It completed
the ALU, Mixed, and Boot rows and had launched the final legacy process in the
Python row when the interactive execution session carrying the parent was
externally aborted during an agent continuation handoff.

After the handoff:

- neither `scorecard-v2.mjs` nor a scorecard worker process existed;
- the formal session identifier was unknown to the process-control service;
- no formal JSON or Markdown report existed; and
- the R127 formal output directory had not been created.

The benchmark itself emitted no worker error, timeout, failed validation, or
performance verdict before termination. Process-health checks during the long
legacy Python legs showed the active worker CPU-bound at roughly one core.
This is therefore external run invalidation, not evidence for or against the
candidate.

## Adjudication

Do not invoke the frozen scorecard adjudicator: there is no complete report to
adjudicate. Do not reconstruct medians from terminal markers, and do not reuse,
pool, replace, or count any R127 process. Candidate source, release Wasm,
scorecard tools, workloads, external inputs, baseline, R125 evidence, and the
incremental promotion rule remain unchanged.

The replacement must rerun all 117 processes from zero under a new frozen
identifier. Launch the formal parent independently of the interactive tool
session, redirect its raw stdout/stderr into the replacement artifact tree,
and record its operating-system PID. Detachment is a measurement-reliability
repair only; it changes no benchmark input, cadence, timeout, or gate.
