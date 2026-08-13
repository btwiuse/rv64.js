# R105 Integrated Scalar Tier-0 Qualified Reconfirmation

Date: 2026-08-10  
Status: rejected at frozen native protected-row gate; cleanly removed

## Question

Does the complete architecture-defined scalar Tier-0 loop retain a verified
net-positive Boot gain against the exact current R085-equivalent source build,
while passing the R094-qualified long-work WANIX guard and the untouched
corrected-cadence three-way scorecard?

R093 previously passed exhaustive correctness, same-Wasm native Boot, a clean
default-on native product check, and fresh Chrome Boot. It stopped at the old
sub-second 4 MiB shared-9P confidence gate. R094 subsequently proved that phase
was too noisy for small-gain decisions and prospectively qualified a fixed
32 MiB replacement. R104 and the project-owner review now authorize one
independent current-baseline confirmation. This explicitly supersedes only
D088's no-reopen clause for this single frozen experiment; it does not pool,
reinterpret, or promote any R093 sample.

## Immutable control and archived basis

The control is the clean executable-R085-equivalent source build:

- Wasm `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`,
  4,279,380 bytes;
- loader `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- `crates/rv64-core/src/cpu.rs`
  `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`;
- `crates/rv64-wasm/src/lib.rs`
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
  and
- authoritative R087 report
  `1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7`.

The sole reconstruction basis is archived default-off source
`target/bench/r093-integrated-scalar-t0/source-default-off.tgz`, SHA-256
`2ad926426a293ebc0416b7cb2059decff47908c072d7b5b7d1bea8352583e79b`.
The archive differs from the exact current control only by the intended
integrated scalar implementation and its proof plumbing.

## Frozen architecture

1. Execute complete ordinary RV64I/M integer, control, scalar load/store, and
   integer RV64C families in one Rust-to-Wasm driver.
2. Carry PC and a local retired count across the scalar stretch. Materialize
   them before interrupts, traps, stops, uncommon-family fallback, generated
   entry, and public return.
3. Preserve the exact interrupt polling schedule, split instruction fetch,
   exception priority, x0 behavior, memory helpers, page/MMIO exits, and slow
   authoritative `Cpu::step` fallback.
4. Select only the ISA-defined family. Add no privilege, guest PC, opcode
   frequency, workload, threshold, browser, or engine selector.
5. First build one default-off same-Wasm causal candidate. If it passes, build
   exactly one separately hashed default-on product candidate with no proof
   setter in its public ABI.
6. Do not alter family membership, decoder layout, poll cadence, helper
   boundary, slice size, or product lifecycle after timing begins.

## Shape and correctness gates

Before performance timing:

1. archived candidate source applies without changing unrelated current code;
2. exhaustive actual RV64C prefixes and broad directed/randomized 32-bit scalar
   cases match `Cpu::step` for registers, PC, memory, exception/stop, and
   retirement;
3. focused core, M/A/FP, system memory, Sv39/MPRV, WFI, lifecycle, T2,
   public/Worker, and Wasm differentials pass off and on as applicable;
4. direct Linux 6.12.7 and OpenSBI Linux reach their exact readiness markers
   with nonzero generated execution; and
5. static inspection proves the integrated Virt drivers and complete-step calls
   only on slow paths, with no unexpected import/export change.

Any semantic or shape failure stops R105. A correctness repair may restore the
frozen semantics and add a directed regression, but may not change the
performance architecture.

## Frozen reconstruction and pre-timing evidence

The default-off candidate Wasm is
`target/bench/r105-integrated-scalar-t0/artifacts/candidate-default-off.wasm`,
SHA-256 `0593567eb75dfe29dd06cf0cabf0747abfa3b217080e2dd2e8c72ca192469a2d`,
4,318,614 bytes. The frozen control copy is
`target/bench/r105-integrated-scalar-t0/artifacts/control-d9f686a9.wasm`,
with the exact control hash above and 4,279,380 bytes.

The candidate's CODE payload is 2,429,760 bytes with SHA-256
`7da96d0f91a41a8b8a04173d1e718a18acdec1ec9ae295adfc1b5517180c7d96`.
That is byte-exact to archived R093 candidate Wasm `118db9b85ba2...`; the two
whole modules differ by only two non-executable metadata bytes. The current
control CODE payload is likewise byte-exact to R085. This is therefore an
exact executable reconstruction, not a decoder or family variant.

Before timing, 35 core, 53 DBT, 76 system, and four Wasm Rust units passed.
The candidate-on user-mode M, A, and FP differentials; system memory,
Sv39/MPRV, WFI, randomized T2 atomic, and T2 lifecycle gates all passed.
Default-off and candidate-on Wasm smoke passed with the two obsolete
BBL/TinyEMU system sections explicitly disabled. Fresh Linux 6.12.7 direct
and OpenSBI boots both reached `ALPINE_READY`, ran a shell command, and proved
hundreds of millions of generated instructions with the candidate enabled.
Public and Worker API lifecycle checks also passed. The frozen source archive
is `target/bench/r105-integrated-scalar-t0/artifacts/source-default-off.tgz`,
SHA-256 `bafea07a1c8edd338e3023a2c7c1ff8a737023e360d4ee48a8f80b8606bb0d27`.

The exact identity-aware cold and native gate implementations and their
mutation self-test were frozen into that archive before the first timing
sample. The native gate rejects changed artifacts, loader, guest, cadence,
policy, output, input, work counters, order, host affinity, sample count, or
any diagnostic beyond same-Wasm scalar off/on selection.

## Cold and native gates

Archive the candidate source and Wasm once. Run seven alternating fresh-process
compile/instantiate pairs; measured cold elapsed may regress by at most 5%.
Artifact and section sizes are recorded but are not rejection thresholds.

Then run seven alternating fresh-process pairs for Boot, Compile, and Python on
CPUs 8--15, exact modern Linux/Alpine inputs, production policy, and R087's
one-slice cadence. All 42 legs must preserve identities, output, JIT proof,
host spread at most `1.10x`, and ordinary sample limits. R105 advances only if:

- Boot paired-median speedup is at least `1.01x` and its paired-bootstrap 95%
  lower bound is at least `1.00x`;
- Compile and Python paired-median speedups are each at least `0.99x`, with no
  interval establishing a regression; and
- normalized Boot MIPS agrees with elapsed time.

There is no sample extension, replacement, or historical pooling.

## Product, browser, and scorecard gates

If native causality passes, build one clean default-on artifact and repeat the
relevant strict correctness plus five alternating native Boot pairs. Require
Boot at least `1.01x` with lower bound at least `1.00x`.

Next run seven fresh alternating Chrome execution-Boot pairs with the same
target rule. Then use exactly the R094-qualified 32 MiB WANIX protocol with
seven fresh browsers and three repetitions per phase. Shell, Python, SHA-256,
and shared 9P candidate/control elapsed paired medians must each be at most
`1.01x`; every existing byte, duration, P9-transfer, output, generated-
execution, and stability proof remains mandatory. A confidence interval that
establishes regression also fails.

Only after all earlier gates pass, run the untouched 117-trial scorecard.
Promotion requires:

- 13/13 at parity or better versus legacy;
- at least 11/13 versus copy/v86;
- a reproducible Boot improvement of at least 1% versus R085/R087;
- no protected rewrite row more than 1% slower than the frozen control without
  an independently valid confirmation showing it is measurement noise; and
- no regression in `python /shared/bench.py`.

Stop at the first failed gate. Preserve immutable evidence, remove the
candidate completely, and restore the exact control. Passing every gate
promotes the unique product artifact as the new baseline; it does not claim
copy/v86 parity unless the scorecard itself proves it.

## Result and decision

The seven-pair cold report is
`target/bench/r105-integrated-scalar-t0/cold-construction.json`, SHA-256
`c0299f406bb03d6409828db1b9dc1f4d9405d6420c46c67e6438f1f355bbe6e0`.
It passes: control/candidate cold elapsed medians are 5.696/5.635 ms, or
`0.9893x` candidate/control. The candidate adds 39,234 whole-module bytes and
38,881 CODE-payload bytes, but actual cold construction did not regress. Size
therefore played no role in the decision.

The valid frozen 42-leg report is
`target/bench/r105-integrated-scalar-t0/native-ab/config-ab-2026-08-10T06-30-41-984Z.json`,
SHA-256 `1ee0190a3521ce54ee34f39ccb1b63ddd2a4915491fcef71fd1a79ae86ba964d`.
Host-probe spread is `1.0212x`; every artifact, loader, Linux 6.12.7 / Alpine
3.24.1 input, one-slice cadence, production policy, generated-execution,
work-counter, order, Compile MD5, and Python result proof passes.

- Boot improves from 2,162.12 to 2,042.96 ms: paired speedup `1.0588x`,
  interval `[1.0360,1.0834]`, and normalized-MIPS ratio `1.0589x`.
- Compile changes from 932.56 to 948.48 ms: paired speedup `0.9803x`, interval
  `[0.9551,1.0079]`.
- Python changes from 2,358.26 to 2,383.40 ms: paired speedup `0.9792x`,
  interval `[0.9600,0.9939]`.

The immutable gate is
`target/bench/r105-integrated-scalar-t0/native-gate.json`, SHA-256
`b35971733fb2241766da996e005254d68bcdcfe65ab46ab26227ac493508cced`.
Boot clears every target check, but Compile and Python miss the `0.99x`
protected medians and Python's entire interval establishes regression. R105 is
therefore rejected under D098. This is not a size rejection or an old 3%
threshold rejection: a real 5.9% Boot win is outweighed by repeatable protected
workload costs under the prospectively stated net-positive rule.

Per the stop rule there is no product artifact, Chrome, WANIX, scorecard,
selector, or retry. Candidate CPU/Wasm and proof plumbing are removed. Core
source is restored to `aec4b31434a6...`, Wasm source to `1da35e70bc9c...`,
release Wasm to exact executable-R085-equivalent `d9f686a9...`, and loader to
`2cbb264f4dac...`. The official score remains 13/13 versus legacy and 11/13
versus copy/v86; Boot and Compile remain open.
