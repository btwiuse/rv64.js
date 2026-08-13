# R124 WANIX Product Gate Result

Date: 2026-08-10  
Status: invalid/incomplete; no WANIX or product credit

The sole frozen run stopped during pair 5 control.  Pairs 1--4 completed in
their preregistered order with zero stderr, but exact control archive
`9d0bf45c...` timed out waiting for the first complete `shared9p` sample in
pair 5.  Its terminal reached the modern Linux 6.12.7 / Alpine 3.22.5 shell,
completed Python in 1.678 seconds with the exact checksum, completed SHA-256
in 2.865 seconds with the exact digest, printed
`__WANIX_PHASE_READY_shared9p__`, and then produced no shared-9P result before
the fixed 600-second phase deadline.

The failed leg's stdout is empty because the harness writes its structured
summary only after all phases close.  Its stderr is
`target/bench/r124-rvc-bank-hybrid/wanix-pairs/pair-5-control.stderr.log`,
SHA-256 `596dddb7df01798c1770e10df11a26435008241aa35f4278c51a2d9ef4f540de`.
The immutable protocol is `6d165654fcab...`.  No leg was replaced, no partial
four-pair ratio is promotion evidence, and no `wanix-gate.json` was produced.

This does not establish an R124 performance regression: the timed-out side is
the exact pre-R124 control.  It does establish that R124 has not passed its
frozen WANIX requirement.  Per the freeze, archive exact candidate
`d017a10f...`, restore exact product `d9f686a9...`, and stop before the
three-way scorecard.

Diagnose the failure outside performance measurement.  The terminal boundary
leaves two primary hypotheses: the phase-release console line did not reach
Python's `input()`, or an external 9P request/reply stalled.  A diagnostic-only
run may add a visible release token and periodic P9/instruction snapshots, but
its elapsed values are ineligible.  Any replacement product gate must be
qualified and frozen prospectively; it may not reuse or replace these samples.

## Post-gate diagnosis

The console hypothesis is now closed.  A diagnostic-only harness observed the
exact visible release token in the terminal before every long-I/O attempt.  In
the reproduced failure, all 33,554,432 write bytes and the complete read work
had crossed the emulator, guest instructions continued to retire, and one host
request remained pending indefinitely.  Its decoded packet was `T_UNLINKAT`
(type 76), tag 0, directory fid 30, flags 0, for the benchmark's temporary
file.  The request received no WANIX reply for more than 64 seconds.

The failing archive retained the exact control core.  A single-flight loader
diagnostic then completed six of six independent fresh-browser 32 MiB runs
with exact byte proofs and no stall.  This isolates multiplexed delivery into
WANIX's stream-backed Go 9P endpoint as the prerequisite, not R124 execution,
the console handshake, or incomplete benchmark work.  These diagnostic times
remain ineligible and none of the invalid R124 gate samples will be reused.

A general FIFO single-flight queue is therefore being qualified in the WANIX
adapter only.  The emulator's generic external-9P bridge remains concurrent
for endpoints that support it.  A replacement gate will use new immutable
archives, new pages, a new protocol, and entirely fresh samples.
