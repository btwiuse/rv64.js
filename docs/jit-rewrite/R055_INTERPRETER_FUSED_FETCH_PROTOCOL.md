# R055 interpreter fused-fetch protocol

Date: 2026-08-08  
Status: rejected at the full-system promotion gate; production removed

## Question

Can the scalar interpreter fetch instruction halfwords through a live,
execute-permission-checked RAM pointer capability, eliminating both the
ordinary fetch-TLB lookup and the physical-bus RAM dispatch on an exact hit?

This is not R020's rejected one-page *translation* cache. R020 retained the
physical fetch path after caching virtual-to-physical translation and produced
no Boot gain. R055 applies the mechanism R054 subsequently proved for data:
the authoritative execute translation publishes a native/Wasm pointer offset,
and later exact hits consume that capability directly. A miss, page crossing,
permission failure, or non-RAM target uses the unchanged fetch path.

## Frozen control and residual attribution

The production control is Wasm SHA-256
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`.
The valid modern Linux 6.12.7 / Alpine 3.24.1 scorecard is
`target/bench/r054-final-three-way-rerun/scorecard-v2-2026-08-08T23-01-30-777Z.json`.

Fresh post-R054 profiles are under
`target/bench/engine-profile-r055/phase/`; the proof-only scorecard is
`target/bench/r055-post-r054-engine-profile/scorecard-v2-2026-08-08T23-22-30-399Z.json`
and the machine-readable attribution is
`target/bench/r055-post-r054-engine-profile/profile-analysis.json`.

The exact accepted rewrite Boot profile sampled 2,279.36 ms:

- runtime Wasm: 93.59%; generated Wasm: 4.77%;
- complete policy-interpreter subtrees: 78.88%;
- `Cpu::step` self time: 1,137.09 ms, or 49.89% of the whole row;
- `Cpu::ld` plus `Cpu::st` self time: 197.65 ms, down from 371.27 ms in the
  pre-R054 profile; and
- generated-module, scheduler-self, translation/issue, and scheduler-hashing
  subtrees: 4.64%, 4.06%, 3.93%, and 1.09% respectively.

The prior exact R041 interpreter trace is still architecture-count evidence:
115,271,767 Boot interpreter instructions contained 43,977,042 32-bit and
71,295,025 compressed instructions. Correct fetch therefore performed
159,249,109 halfword reads. The same trace contained 40,136,235 scalar data
loads/stores, so instruction-fetch halfwords were 3.968x as frequent as the
data operations whose pointer fusion R054 promoted. R054 reduced untouched
Boot time 13.35%; a fetch hit retaining only 18.9% of that measured
per-operation benefit has a 10% whole-Boot opportunity ceiling. This admits a
frozen engine-shape test, not production code.

Compile STEADY independently shows the same direction. The rewrite profile is
50.59% runtime / 47.90% generated, while v86 is 21.50% / 74.45%. In absolute
sample time the generated subtrees are 692.80 versus 675.45 ms; most of the
remaining rewrite-v86 difference is runtime/interpreter/scheduler work rather
than generated-Wasm execution.

## Frozen engine-shape gate

Before changing production, emit deterministic control and fused-fetch Wasm
modules. Both must execute an identical mixed compressed/32-bit fetch stream,
including sequential same-page hits, nonsequential same-page targets, a page
transition/refill, and a 32-bit instruction split across the last page
halfword. The control performs the exact hot standard fetch proof, physical
RAM range/offset path, and halfword load. The candidate performs one exact
page/context capability comparison and direct halfword load, with the complete
control path on a miss.

Measure seven alternating paired fresh Node 26.5/V8 14.6 processes. Admit a
production prototype only when all of the following are true:

1. regenerated modules are byte-deterministic and `WebAssembly.validate`;
2. every tested iteration count produces identical state and fetched-byte
   checksum in both variants;
3. paired fused/control steady throughput is at least 1.50x and its bootstrap
   median 95% lower bound is at least 1.35x; and
4. fused cold compile plus instantiation and its paired delta are each below
   25 ms.

Do not tune table size, stream population, or thresholds after timing. Failure
closes direct one-page pointer fusion together with R020 for instruction fetch.

## Production shape if admitted

Use one recent fetch capability, not another 4,096-entry CPU bank:

- one tag stores exact virtual page plus execute permission context;
- one offset maps the virtual address directly to the live RAM pointer;
- publication occurs only after `translate(..., Access::Fetch)` succeeds and
  `Bus::jit_fast_off` proves stable RAM backing;
- `fetch16` first probes that exact capability, otherwise executes the current
  translation and bus path and may publish the next capability;
- global and matching-page `SFENCE.VMA`, SATP/mapping flushes, and backing
  invalidation clear it; a different privilege/context cannot match its tag;
  and
- the 0xffe 32-bit split translates and proves each page independently.

The rule is complete-architecture and contains no PC, opcode, page address,
kernel symbol, workload, checksum, compiler, browser, or engine selector.

## Correctness and promotion gates

Before timing Linux, require directed tests for same-page compressed and
32-bit fetch, page-end compressed fetch, 0xffe split fetch, execute-context
mismatch, selective/global fence invalidation, remapping, non-RAM fallback,
and instruction-access/page faults. Run the existing core/system, randomized
full-state, generated-memory, Sv39/MPRV, T2/atomic, and direct/OpenSBI modern
Linux gates with the candidate both disabled and enabled where supported.

Then use five alternating fresh-process same-Wasm pairs on Boot and Compile.
The candidate advances only if:

- Boot paired speedup median is at least 1.10x and its bootstrap 95% lower
  bound is at least 1.00x;
- Compile speedup is at least 0.90x;
- exact guest/input/output/JIT-policy fingerprints match; and
- every sample and host-probe spread passes the existing fixed gates.

If admitted, archive and compare the exact final default-on artifact against
`416033...`, repeat the full correctness matrix, run the untouched authoritative
13-row legacy/rewrite/v86 scorecard, and run the five-pair Chrome
`/shared/bench.py` guard. Promotion requires no non-target scorecard regression
of 10% and preserves every existing browser non-inferiority decision. No
individual failed or noisy leg may be replaced.

## Result and decision

The deterministic corpus
`target/bench/r055-interpreter-fused-fetch-corpus.json` passed its frozen gate.
Control and fused modules regenerated byte-identically, validated, and matched
state and memory at every requested iteration count plus the 0xffe page split.
Seven alternating fresh Node 26.5/V8 14.6 pairs measured 974.779 versus
1,939.601 million halfword fetches/second. The paired fused/control median was
1.989x with bootstrap median interval `[1.984,1.993]`; fused cold construction
was 0.206 ms and its paired delta was -0.026 ms. That evidence admitted exactly
one production prototype.

The prototype then passed 35 core tests, 76 system tests, the Wasm smoke suite,
bare-memory and Sv39/MPRV differentials with the feature both off and on, and
both direct and OpenSBI modern Linux boots. It used no workload selector. The
same-Wasm artifact was
`0386864ea02a63c1f5a89073a210072639783a743ec29500907bdb5b872e8a5a`.

The decisive five-pair report is
`target/bench/r055-interpreter-fused-fetch-same-wasm-ab/config-ab-2026-08-08T23-45-27-090Z.json`.
It is measurement-valid, has no problems, uses exact matching guest inputs and
Compile MD5, and has 1.023x host-probe spread. Results were:

- Boot control 3,310.8 ms versus candidate 3,420.1 ms; aggregate speedup
  0.968x and paired median 0.962x with interval `[0.935,1.152]`;
- Compile STEADY control 1,154.6 ms versus candidate 1,141.5 ms; aggregate
  speedup 1.011x and paired median 1.008x with interval `[0.982,1.053]`.

Boot therefore failed both the 1.10x median and 1.00 lower-bound requirements.
The isolated engine shape did not survive the real interpreter's control flow,
context changes, invalidations, and surrounding code layout. No refill policy,
cache count, tag encoding, page selection, or workload-specific rescue was
attempted.

The CPU fields, pointer probe, publication path, invalidation hooks, Wasm
export, runtime synchronization, worker switch, and candidate-only differential
controls were removed. After restoration, 32 core tests pass and the release
module is byte-for-byte the accepted R054 artifact
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`
at 4,272,517 bytes. Retain only the frozen emitter, corpus harness, protocol,
and raw reports as reproducible negative evidence. R020 and R055 jointly close
one-page instruction-fetch translation/pointer caching in the current scalar
interpreter shape.
