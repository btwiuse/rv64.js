# R094 Long Shared-9P Guard Qualification Protocol

Date: 2026-08-09  
Status: qualified for future candidates; exact-R085 null gate passed

## Purpose

The R092 and R093 product guards used a 4 MiB shared-9P phase consisting of
only 1,024 4 KiB guest reads and writes. R093's seven paired medians ranged
from 0.674x to 1.636x even though its candidate/control median was favorable at
1.040x. Individual samples lasted only 0.388--0.651 seconds. The resulting
`[0.730,1.580]` interval could not establish non-regression and stopped R093
under its valid frozen rule.

R094 prospectively qualifies a longer guard for future candidates. It does not
reopen, rerun, or promote R093. It changes no emulator source, JIT policy,
Python workload, SHA-256 workload, modern scorecard artifact, or official
score. Any future candidate must still pass this qualified guard before its
untouched three-way scorecard.

## Frozen workload and identities

Create two versioned comparison pages from the exact R085 WANIX page. Both
pages bind the same immutable R085 archive
`0b953be67610e130f79a852f86542c8400ad3a235001ec450fbdffc29ed3a61a`
containing Wasm
`efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010`
and loader `2cbb264f4dac...`. After normalizing the archive query token, the two
pages must be byte-identical.

Change only the versioned pages' `shared_io` fixed work from 4 MiB to 32 MiB.
Keep its 1 MiB Python block, unbuffered file objects, unique filename, complete
write/read/unlink sequence, phase synchronization, and reported elapsed-time
boundary unchanged. Python remains 250,000 iterations and SHA-256 remains
32 MiB. Do not modify the ordinary public comparison page.

## Qualification run

Run seven alternating pairs on CPUs 8--15. Every leg owns a fresh Chrome
process/profile, WANIX Worker, exact R085 VM, and Linux 6.12.7 / Alpine 3.22.5
integration guest. Run three synchronized `shared9p` repetitions per browser;
exclude shell time and all setup from the phase values. Record immutable page,
archive, inner-Wasm, loader, adapter, root, browser, guest, JIT-policy, and
generated-execution identities.

Every repetition must:

- complete with the existing shared-9P correctness marker;
- report exactly 33,554,432 P9 write bytes and a maximum 4,096-byte write;
- report at least 33,554,432 P9 read bytes and a maximum 4,096-byte read;
- execute nonzero generated guest instructions with exact retirement
  accounting; and
- last at least 2.0 seconds, proving the short 4 MiB timing was not reused.

Qualification passes only if all 42 samples satisfy those proofs, each
browser's three-sample max/min spread is at most 1.25x, and the seven paired
control/treatment median speedups meet all of:

- median in `[0.97, 1.03]`;
- exact paired-bootstrap 95% interval contains 1.0;
- interval lower bound at least `1 / 1.10`; and
- interval upper bound at most `1.10`.

This is a null comparison, not a performance claim. If it fails, record the
failure and do not alter work size, repetition count, interval method, or
thresholds after observing the samples. R093 remains rejected regardless of
the result.

## Result

The frozen qualification passed without a retry or rule change. All fourteen
fresh-browser legs and all 42 synchronized samples completed. Every sample
lasted 23.765--25.757 seconds, wrote exactly 33,554,432 P9 bytes, read
33,611,024--33,660,176 P9 bytes, used 4,096-byte maximum transfers, executed
nonzero generated instructions, and satisfied exact retirement accounting.
The largest three-sample spread within any browser was 1.068x.

The seven paired control/candidate medians were 1.0165x, 1.0317x, 0.9984x,
1.0000x, 1.0075x, 0.9960x, and 1.0004x. Their median is 1.0004x and the exact
paired-bootstrap 95% interval is `[0.9984,1.0165]`, satisfying every frozen
null bound. The gate report is
`target/bench/r094-long-shared9p/gate.json`
(`760567deb6f7f2ae7b45d27cb0f709b90c663e26c98aee20f1d170ff6d489f1d`);
its immutable run protocol is
`target/bench/r094-long-shared9p/qualification/protocol.json`
(`4e7d211a5bd629c119d8412098eb96bbc6df15a35c2e20025674760905de70db`).

Therefore future independently admitted candidates may use this exact R094
long shared-9P guard. This is a harness qualification only: it changes no
official score, product artifact, or prior decision, and R093 remains closed.
