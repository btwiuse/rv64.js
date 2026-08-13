# R128 Exact-R124 Scorecard Paused Result

Date: 2026-08-10  
Status: user-requested pause; incomplete and ineligible

## What ran

R128 launched exact candidate `d017a10f...` through the frozen detached
protocol. The manifest and both harness selftests passed, the real copy/v86
generated-execution preflight passed, and the detached parent was confirmed as
PID 525150 in its own session under PID 1.

The raw orchestration log reached:

```text
[alu] ... ok
[mixed] ... ok
[boot] ... ok
[python] 1r 1l
```

The owner then requested a pause. The exact isolated process group 525150
(launcher, scorecard parent, and active legacy-Python worker) received
`SIGTERM`. All three processes exited and no scorecard process remains.

## Evidence status

There is no formal JSON, Markdown report, or runner exit record. Row markers
show scheduling/completion control flow but do not expose or preserve the
formal population. Do not calculate or infer a candidate result from them, do
not invoke the adjudicator, and do not reuse any R128 process in a future run.

R128 is neither a candidate pass nor a candidate failure. It is an
intentionally stopped partial run. Its raw stdout/stderr, PID, immutable
manifest, protocol, and launcher remain preserved under
`target/bench/r128-r124-scorecard/`.

## Paused repository boundary

The worktree and release build currently contain exact uncommitted R124
candidate `d017a10f...`; this was necessary to avoid the diagnostic-only
artifact override exposed by R126. It has not passed the modern scorecard and
has not been committed or promoted as the accepted product. Exact control
source backups remain preserved for restoration if the experiment is later
rejected.

Official evidence therefore remains the accepted pre-R126 state: 13/13 rows
versus legacy and 11/13 versus copy/v86, with Boot and Compile open. R125 is
accepted as WANIX qualification under the owner's one-percent materiality
policy, while its original immutable analyzer still reports the stricter
zero-regression gate failure.
