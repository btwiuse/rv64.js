# R112 JIT Debug Source-Attribution Protocol

Date: 2026-08-10  
Status: closed; V8 emitted no debug record for sampled generated Wasm

## Question

Do the `JIT_CODE_DEBUG_INFO` records already preserved by R110 map enough
sampled TurboFan guest-body and explicit native-stack period to stable Wasm
source positions to support a later operator-level attribution? R112 is a
read-only diagnostic. It recollects no samples, changes no emulator or JIT
code, and treats no elapsed time as performance evidence.

## Frozen inputs

- R110 perf data:
  `79abdf40afbf492c2718bb5832758df1ed2e137d18329d543c79c5c0c9196809`;
- R110 Node/V8 jitdump:
  `2d10666fd9f9279cec8b9c2e580c05d925a177f05ff626e4262bf30a7999acd4`;
- authoritative R110 native census:
  `4aab78cbb7a5dc2045b42abe420b42ed6ad9262a267b4e7029f137ce94ce971f`;
- engine Node `v26.5.0`, V8 `14.6.202.34-node.24`;
- Linux jitdump layout from the local 6.12.7 `tools/perf/util/jitdump.h`.

The two R110 malformed debug-record lengths remain part of the input. The
validated R110 record resynchronization offsets (`-6`, `-5`) are reused; the
samples, code loads, and native instruction classification may not be changed.

## Frozen parser and join

Implement one deterministic analyzer that:

1. parses every jitdump record using the R110 validation and timestamp rules;
2. treats the actual next validated record offset as a debug record's physical
   end, while reporting any difference from its declared size;
3. decodes each debug entry as absolute native address, signed 32-bit line,
   signed 32-bit discriminator, and NUL-terminated filename; `0xff,0x00`
   repeats the previous filename exactly as defined by jitdump;
4. requires each debug record's declared entry count to consume its semantic
   payload exactly; the only remaining physical bytes may be zero-valued
   8-byte record-alignment padding (zero through seven bytes), validated and
   reported rather than silently ignored;
5. associates a debug record with the nearest timestamp-ordered code load at
   the same `code_addr`; address reuse is resolved by the same newest-load rule
   used for samples, and ambiguous associations are reported rather than
   guessed;
6. maps a native sample to the greatest debug-entry address not above its
   instruction address and below the next entry/load end; and
7. reuses R110's exact generated-function, tier, guest-body/trampoline, native
   family, and period partitions, requiring every total to close bit-exactly.

The report serializes record/entry validity, load association, mapped and
unmapped sample periods, filename/line/discriminator distributions, and the
top source positions separately for all guest-body period and explicit
native-stack period. It interprets neither line nor discriminator as a Wasm
operator unless the preserved data itself establishes that meaning. A
non-sentinel position has a non-empty resolved filename and a strictly
positive line number, matching jitdump's one-based line contract;
discriminator zero is valid.

The analyzer must include synthetic self-tests for filename repetition,
multiple entries, physical-versus-declared debug length, address boundaries,
address reuse, ambiguity rejection, and unmapped prefixes/suffixes. Two fresh
invocations over the frozen input must produce byte-identical JSON.

## Admission gate

R112 supports a later operator-level capture only if all conditions pass:

1. every physical debug payload parses exactly to its declared entry count;
2. at least 95% of sampled TurboFan guest-body period belongs to a load with an
   unambiguous debug record;
3. at least 90% of sampled TurboFan guest-body period maps to a non-sentinel
   source position;
4. at least 90% of its explicit native-stack period maps to a non-sentinel
   source position; and
5. mapped stack period spans at least two distinct source positions, so the
   data is not merely a function-level label.

Passing does not admit an optimization. It admits one separately frozen,
diagnostic-only R113 capture that preserves exact dynamically generated Wasm
bytes and joins documented source offsets to decoded operators. Failure closes
the jitdump source-position route; it does not permit another perf run, debug
flag, engine, workload, selected load, or threshold variant.

No result from R112 changes exact product `d9f686a9...`, reopens R111
partition variants or earlier closures, or earns scorecard credit.

## Result

The authoritative reports are `source-attribution-b.json` and
`source-attribution-c.json` under
`target/bench/r112-jit-debug-source/`. They are byte-identical with SHA-256
`0d84a1619199b9d207e0e8661e8abf3c25cce91cf05094e3b19196a6c8554572`.
The preserved `source-attribution-a.json` is a superseded parser draft: it
mistook V8's zero-valued 8-byte record padding for semantic payload. No sample
or record was recollected; the final parser explicitly validates that padding
according to the local jitdump layout.

All 5,985 records close exactly through the same `-6` and `-5` debug-length
resynchronizations as R110. All 250 debug records parse successfully to 6,007
entries. The analyzer independently reproduces R110's total, main-thread,
generated, tier, role, and native-family periods bit-exactly.

The source-attribution gate nevertheless fails decisively:

| Measure | Result | Gate |
| --- | ---: | ---: |
| valid physical debug payloads | `250/250` | all |
| debug records associated with generated Wasm loads | `0` | diagnostic |
| TurboFan guest period in a load with debug info | `0%` | `>=95%` |
| non-sentinel mapped TurboFan guest period | `0%` | `>=90%` |
| non-sentinel mapped explicit-stack period | `0%` | `>=90%` |
| distinct mapped stack positions | `0` | `>=2` |

Every debug entry belongs to JavaScript/Node code, including `rv64.js`, the
scorecard worker/library, and Node internals. None of the sampled
`JS:wasm-function[0-5]` Liftoff or TurboFan loads has a debug record. Thus the
absence is not a join ambiguity or malformed-record artifact: this V8 jitdump
simply contains no Wasm source map for the relevant native code.

R112 closes the jitdump source-position route without R113, another perf run,
an engine/debug-flag variant, or product work. Exact `d9f686a9...` remains the
product baseline and the official score is unchanged.
