# JIT Rewrite Findings

This log records observed facts before conclusions. Historical JIT tuning
documents and the deleted compiler source are intentionally excluded as design
inputs.

## 2026-08-06: trace-driven async page admission

- Exact JIT-off calibration on three fresh Node/V8 processes measured direct
  Linux boot at 832.5 ms, ALU1 at 148.8/140.5 ms, ALU5 at 574.3/570.2 ms, and
  mix20 at 149.2/148.1 ms for first/repeat invocation medians.
- Direct/OpenSBI boot traces retired 42.14M/48.60M instructions across 544/571
  physical pages. Long ALU and mix traces retired 551.36M and 159.34M. Compute
  heat is extremely concentrated; the hottest page accounts for roughly
  98–99%.
- `(virtual page, physical page)` is the safe sharing identity. Compared with
  SATP-keyed contexts it recovers about nine percentage points of ideal boot
  coverage near 200K, while physical-only sharing cannot distinguish aliases.
- A q32 threshold sweep initially made short work appear slower than the exact
  interpreter. The sampled machine had lost the WFI stop reason after retiring
  some work, so the JIT driver ran through idle wakeups to the full public
  budget. Returning `(retired, yielded)` restored exact host scheduling and
  made a no-compile q1024 control match the direct bypass within noise.
- At q1024, a 1,048,576-instruction threshold is the measured knee. It emits a
  median 26 modules and boots in 867.1 ms, versus 112 modules/930.5 ms at 262K
  and 12 modules/831.2 ms at 2M. The selected threshold retains materially more
  first-use coverage than 2M without the low-reuse boot compilation of 262K.
- Fresh Chrome 150 processes/profiles measured 843.2 ms exact-interpreter boot
  versus 891.9 ms policy boot. First/repeat gains were 2.35x/3.46x for ALU1,
  8.54x/13.80x for ALU5, and 1.29x/2.39x for mix20. Every generated module was
  async-region; default runs had no queue drop or compilation failure.
- The selected OpenSBI run passed all five checksums with 32 async modules,
  reached 99.5% generated coverage on the first long ALU run, and performed no
  synchronous per-PC build.
- The stable `RV64` loader now enables the policy. The low-level debug loader
  keeps selection explicit so existing differential tests can choose their
  compiler path. The WANIX comparison archive/page was rebuilt with JIT on.

## 2026-08-05: repository boundary

- The compiler/emitter crate was exactly `crates/rv64-jit`, containing three
  tracked files. It was deleted without reading those files.
- The cloned baseline commit is
  `4b0896decdff7538f9c1d2b44dc19a1d3d14f7c2`, so deletion is recoverable without
  retaining old implementation files in the active tree.
- Compiler orchestration is not wholly isolated in that crate. `rv64-wasm`
  depends on it and contains dynamic-module publication, dispatch, statistics,
  and run-loop integration.
- `rv64-core` and `rv64-system` contain JIT-oriented cache/TLB/invalidation
  hooks. These are candidates for neutral execution-engine contracts, not
  automatically accepted parts of the new design.
- JavaScript contains the host side of generated-module instantiation. Its
  external behavior must be separated from any legacy compiler policy.
- Existing interpreter-vs-JIT and system workload tests are valuable behavioral
  and benchmark assets. Assertions that encode a specific old JIT layout or
  private API will need replacement with engine-neutral contracts.

## 2026-08-05: first independent T1

- The non-compiler crates and optimized Wasm build remained healthy after the
  clean-room deletion. The repository has 104 applicable native tests.
- A generated function can use the surviving publication adapter with only an
  `env.memory` import and a `(i32) -> ()` exported `run` function.
- Release LTO removes unused optional host imports. Raw ABI validation must
  distinguish required imports from optional publication capabilities rather
  than require a policy-specific exact list.
- The first pure-integer T1 compiled enough of the existing user benchmark to
  retire 74% of guest instructions in generated code, using 11 installed
  blocks. One sample measured 1096.8 ms for 230,000,332 instructions
  (209.7 Minsn/s).
- That run made 18,980,824 compiled-block dispatches. This is direct evidence
  that multi-block/loop residency is the next structural optimization; adding
  isolated instruction lowerings alone cannot remove the dominant edge count.
- Materializing every architectural `ReadX` before state stores is required
  even when it has only one SSA use. `JALR x1, imm(x1)` otherwise reloads x1
  after committing the link value and computes the wrong target.
- The minimal T1 matched the interpreter over 100 randomized full-state
  programs and exact retirement over a 230-million-instruction guest run.

## 2026-08-05: loop residency

- Recognizing a conditional backedge to the region entry and executing it as a
  fuel-metered Wasm `loop` reduced the user benchmark's compiled dispatch count
  from 18,980,824 to 48,466.
- The same change improved one Node/V8 sample from 1096.8 ms (209.7 Minsn/s) to
  852.5 ms (269.8 Minsn/s). This confirms host-edge frequency was structural,
  although cross-engine paired samples are still required before tuning policy.
- The first loop iteration must be allowed even when its static length exceeds
  remaining fuel, matching the bounded-basic-block overshoot contract. Later
  iterations run only when one more iteration fits.
- Effectful loops need more state than pure loops. A precise fault at the first
  memory instruction of iteration N must expose register results from iteration
  N-1 even when the static body has not rewritten those registers yet. Dedicated
  dirty-register carry locals solve this without committing state every lap.

## 2026-08-05: flat memory and RV64M

- WebAssembly out-of-bounds traps cannot represent guest access faults because
  they abort the generated call before architectural state is materialized.
  Explicit `address <= length - width` checks permit an exact pre-instruction
  interpreter side exit.
- Loads whose values are dead must remain IR roots because the access can fault.
  Stores carry an explicit SSA-stream position; removing pure values remaps that
  position while retaining store/load order.
- A first-instruction memory side exit retires zero instructions. The user-mode
  chain loop must break immediately in that case or it repeatedly invokes the
  same compiled block before falling back.
- Dedicated memory differential tests cover signed/unsigned 1-, 2-, and 4-byte
  loads, 8-byte loads, all four stores, and a post-tier-up bounds fault. Full
  state and retirement match the interpreter.
- RISC-V division is total, while WebAssembly division is partial. Result-typed
  structured `if` guards are required; Wasm `select` is eager and would still
  evaluate a trapping divide. The guards implement zero-divisor results and
  MIN/-1 overflow for both 64-bit and word operations.
- Wasm has no 64x64 high-half multiply. A four-limb 32-bit expansion plus signed
  correction terms implements MULH/MULHSU/MULHU without a helper transition.
  Constant wide-arithmetic tests and a hot-loop full-state differential pass.
- After memory, loops, and full RV64M, one Node/V8 sample measured 828.8 ms
  (277.5 Minsn/s) for the same 230,000,332-instruction user workload.

## 2026-08-05: guarded traces and exact FP

- Following decodable forward JAL targets and conditional fallthrough with an
  exact taken-edge guard removes direct control redispatch while retaining a
  bounded T1. A dedicated diamond/call/memory-loop differential retired 263,683
  compiled instructions in 15,147 dispatches with full-state parity.
- FP registers are represented as raw `i64` bits and `fcsr` as `i32` SSA.
  NaN-boxed loads/stores, moves, sign injection, and FP CSR read/modify/write
  first passed bit-exact differentials without host floating point.
- A single exact helper ABI now covers scalar F/D add/sub/mul/div, sqrt,
  min/max, comparisons, classification, all scalar conversions, and the four
  fused multiply-add sign families. It is an imported Wasm function from the
  already-instantiated main module, not a JavaScript callback.
- Effectful loops must carry dirty FP registers and `fcsr` in addition to GPRs.
  Omitting those carries preserved module validity but lost state across later
  iterations and precise exits.
- Per-PC profiling identified `FLT.D` as the reason an FP loop made 191,067
  compiled dispatches: two tiny blocks executed around a 512-instruction
  interpreter stretch on every iteration. Exact comparison lowering reduced
  the complete benchmark to four compiled dispatches and raised coverage from
  78.5% to 99.7%.
- A dedicated FP differential executes every helper-backed scalar F/D family
  hot under RNE, RTZ, RDN, RUP, and RMM. Normal values, signaling/quiet NaNs,
  infinities, signed zero, minimum subnormals, and maximum finite values match
  the interpreter in all GPRs, FP bits, `fcsr`, PC, trap/exit, and retirement.

## 2026-08-05: proven native-FP fast path

- Wasm native FP cannot expose IEEE flags or non-RNE rounding, so it is not a
  general RISC-V lowering. It is exact when RNE is active, NX is already sticky,
  operands are finite, and operand/result classes rule out NV, DZ, OF, and UF.
- Adding that proof inside the Wasm-to-Wasm helper retained all directed and
  randomized differentials. The 230,000,332-instruction mixed benchmark then
  improved from roughly 495 ms (464 Minsn/s) to 159-172 ms
  (1,338-1,446 Minsn/s), still at 99.7% coverage and four dispatches.
- The interpreter-only sample was 3,343.9 ms (68.8 Minsn/s), making this one
  local sample about 21x faster. It is not yet a cross-engine confidence result.

## 2026-08-05: lifecycle and second-tier behavior

- Generated module order and SHA-256 hashes were identical across fresh Node
  processes. Current workload runs emit four modules.
- A three-process post-FP sample measured cold total median/p95
  190.250/190.284 ms, guest translation 3.834/4.709 ms, V8 compile
  0.242/0.262 ms, instantiate 0.100/0.103 ms, and publication
  0.133/0.151 ms. Frozen-byte V8 compile was 0.180/0.202 ms.
- The same frozen modules validated and AOT-compiled in Wasmer 5.0.3. CLI
  Singlepass and Cranelift medians were 53.073 and 55.278 ms respectively, but
  each module launches a separate CLI process, so these are toolchain lifecycle
  numbers rather than in-process compiler latency.
- V8 documents no Wasm on-stack replacement. A runtime loop-quantum experiment
  forced re-entry every 16K through 4M guest instructions. A 64K quantum raised
  dispatches from 4 to 3,503 and did not produce a repeatable production-mode
  win on this workload. TurboFan-only diagnostics were only a few percent
  faster than Liftoff-only, so uncapped loop residency remains the default.
- The per-PC profiler is now available for user mode. It attributes compiled
  calls/retirement, exit edges, and the first unsupported encoding responsible
  for each interpreted stretch; aggregate coverage alone had hidden the FP
  control-flow failure.

## 2026-08-05: atomics and full-system execution

- User and system LR/SC need different typed state capabilities. Treating the
  reservation pointer as an untyped integer can validate as Wasm while calling
  a helper with the wrong host-state representation.
- Directed hot tests cover every AMO operation at word and doubleword widths,
  LR/SC success and failure, reservation clearing, and a bounds fault after
  tier-up. The generated and interpreted full architectural states match.
- A full-system fast translation row consists of a virtual tag and a signed
  linear-memory offset. On a miss, a Wasm-to-Wasm helper may refill the row, but
  generated code must re-read and revalidate both fields. This makes failure,
  MMIO, permission denial, and any future row encoding independent of a magic
  helper return value.
- Page-crossing memory operations, MMIO, and stores to pages containing compiled
  code remain precise T0 exits. Synthetic bare-system tests exercised each path;
  the code-page store invalidated the expected generation before execution
  continued.
- A three-level Sv39 differential under MPRV/MPP=S started with A/D bits clear.
  One generated refill set the leaf PTE to `0x200008c7`, and the hot load/store
  loop matched interpreted registers and memory.
- Full-system FP requires an ordered architectural effect separate from the FP
  computation. The generated pre-instruction check exits when `mstatus.FS` is
  Off and marks Dirty at the same point as the interpreter. Stores check FS but
  do not dirty it; loads dirty before a potentially faulting memory access.
- The post-change regression matrix passes 140 Rust tests and all user/system
  Node differentials. Representative system runs retired 262,160 compiled
  instructions for LR/SC+AMO, 262,176 for Sv39 memory, and 262,104 for FP.

## 2026-08-05: first operational T2

- A portable core-Wasm multi-entry module can export one shared dispatcher for
  every installed entry. The dispatcher reads architectural PC, selects a body
  through a balanced decision tree, rejects zero-retirement re-entry, enforces
  cumulative fuel, and returns only when PC leaves the member set. It needs no
  JavaScript hot edge, function-table import, or Wasm tail-call proposal.
- Static leader discovery from hot seeds finds conditional targets/fallthrough,
  direct jump targets, call return addresses, and backedges. Sparse region
  lifting limits each entry to its virtually contiguous code run, preventing a
  concatenated noncontiguous page buffer from inventing fallthrough bytes.
- The first live region test landed but retired only two region instructions.
  The cause was outside code generation: a documented “0 means claim all”
  setting tested `trace_length >= 0` and therefore retained every integer
  trace. Correcting the zero-special case raised the same test to 107,847
  region-retired instructions.
- A monomorphic indirect edge is best represented inside IR, not as a host
  dispatch hint. `GuardTarget` compares the computed target with the observed
  target after the JALR's architectural writes; match continues in SSA, while
  mismatch commits the computed PC. A target-changing differential executed
  two extensions and reached identical x/PC/memory completion state.
- Observed indirect cycles that return to their region entry become
  unconditional fuel-bounded Wasm loops. Their loop guard compares local work
  plus the invocation's already-cumulative retired count against fuel, so an
  in-module predecessor cannot silently grant a fresh budget.
- Batch and async-region registration previously escaped lifecycle accounting.
  They now contribute copied bytes and copy/compile/instantiate/publication
  time and provide exact bytes to frozen replay after publication.
- A three-process Node 26 / V8 14.6 synthetic lifecycle sample produced
  deterministic module order in all modes. Batch emitted six small modules
  plus one 6,217-byte six-member module; median translation/Wasm-compile/frozen
  compile time was 4.74/0.35/0.33 ms. Region emitted twelve small modules plus
  one 1,253-byte async region; corresponding medians were 2.79/2.20/0.39 ms.
  IC emitted twelve small/fused modules; medians were 4.07/0.92/0.33 ms. These
  diagnose lifecycle shape on a short synthetic workload, not a default-policy
  performance verdict.

## 2026-08-05: register-resident multi-entry backend

- Independent member functions that store state before every dispatch leave a
  large optimization boundary for the engine's own Wasm compiler. Emitting one
  shared defined function allows all member exports to reference the same body,
  loads the union of live architectural state once, and carries it through the
  balanced internal dispatcher and nested self-backedge loops.
- Precise exits cannot simply commit the current member's outputs. A member may
  read state written by an earlier member without rewriting it itself, so each
  exit merges the current SSA snapshot with cached prior-member locals. Fuel
  and retirement use the same cumulative local-state rule.
- The six-member synthetic batch shrank from 6,217 to 4,804 bytes after shared
  state and cumulative retirement were made local. Its async-region form is
  approximately 1,162 bytes. The materialized implementation remains available
  behind an emitter mode so the comparison does not depend on old binaries.
- The current deterministic seven-sample Node 26 / V8 14.6 frozen-corpus run
  measured 7,388 versus 1,666 Minsn/s for cached versus materialized integer
  state, and 3,775 versus 1,470 Minsn/s when the cycle also performs translated
  memory. Paired medians were 4.346x and 2.692x. These are synthetic steady-state rates;
  the corpus also retains raw compile, instantiate, first-call, and warm-up
  samples so they cannot be presented as end-to-end guest speedups.

## 2026-08-05: bounded translation proof cache

- A register-resident invocation can safely reuse one proven load and one
  proven store guest-page translation until it exits: privileged context and
  mapping changes are interpreter exits, and a refill is accepted only after
  generated code re-probes the authoritative tag/offset row. An impossible
  page tag initializes each local, avoiding a separate validity branch.
- More eliminated row probes do not imply a faster second-stage JIT result. In
  the current Node corpus, caching measured 3,775 versus 3,057 Minsn/s (paired
  median 1.243x). In three fresh-profile browser samples, Chrome 150 measured
  4,050 versus 4,249 Minsn/s (0.952x), while Firefox 120 measured 405 versus
  262 Minsn/s (1.579x). An earlier Chrome corpus measured a larger 0.703x
  regression. The additional locals/branches help one backend and have not
  produced a repeatable Chrome benefit.
- Therefore register residency is the portable default, while invocation-local
  translation caching is disabled by default and exposed through
  `jit_set_region_tlb_cache` for engine/workload experiments. This is a direct
  example of why the emulator must not tune only against its current Wasm JIT.

## 2026-08-05: frozen cross-browser backend corpus

- `emit_backend_corpus` emits exact cached/materialized integer cycles plus
  cached/materialized full-system-memory variants. The harness verifies module
  hashes, byte sizes, retirement, PC, and architectural effects before using a
  timing sample.
- Each browser sample starts a fresh process and profile. Variants use fresh
  documents in alternating order; the local server disables HTTP caching, and
  DevTools polling occurs outside all timed intervals. The raw diagnostic report
  is `target/jit-backend-browser-report.json` and is reproducible with
  `node tests/jit-backend-browser-bench.mjs --samples=3 --output=target/jit-backend-browser-report.json`.
- Three-sample Chrome 150 medians were 6,171 versus 1,552 Minsn/s for cached
  versus materialized integer state and 4,050 versus 1,440 Minsn/s with memory;
  paired speedups were 3.690x and 2.813x. Firefox 120 medians were 6,480 versus
  291 and 405 versus 199 Minsn/s; paired medians were 22.25x and 2.031x. Firefox's
  coarse timer and the small sample count make these directional diagnostics,
  not confidence claims, but both engines strongly select register residency.

## 2026-08-05: bounded two-way indirect targets

- Successful fused traces may return from a later edge, so architectural PC at
  function return is not reliable feedback for the original indirect site.
  Generated target guards now write an explicit target and then an owner commit
  cell only on mismatch. The runtime clears the owner before every invocation.
- Two constant-space Misra-Gries candidates prevent a rare third target from
  immediately evicting either frequent target. An unspecialized site becomes
  monomorphic only when one mature target is at least 8x dominant; two balanced
  mature targets go directly to a two-way region. Recompilation is bounded to
  two extensions, and any other target exits through the exact computed PC.
- The two-way region lifts the indirect source without speculation and lifts
  each target as an independent entry in the register-resident function. Its
  internal PC dispatcher selects either target without JavaScript or a Wasm
  table call.
- A PIC upgrade now captures only the distinct source/target pages into a
  sparse immutable snapshot. This covers arbitrary cross-page targets without
  restoring the expensive 256-KiB capture window for ordinary T1 blocks. If a
  target is temporarily unavailable, the valid source still compiles instead
  of being blacklisted.
- An end-to-end alternating-target differential matched interpreter PC, GPRs,
  memory, retirement, and power-off state after 262,144 transfers. Each arm ran
  131,072 times; the runtime recorded one PIC upgrade, retired 77,341 guest
  instructions in generated code in the sampled run, and made 199 host
  dispatches.
- Moving the second target to another page produces the same one-upgrade,
  77,341-instruction, 199-dispatch result. A second test modifies that target
  page only after the PIC is resident: it records one dirty-page event, drops
  four dependent entries, recompiles from the changed bytes, respects the
  cumulative 100,000-instruction slice bound, and matches the interpreter's
  changed instruction result exactly.

## 2026-08-05: bounded SSA temporary pressure

- Multi-entry bodies are mutually exclusive, and every body transfers its
  architectural outputs to the shared state locals before another body runs.
  Their non-architectural SSA temporaries can therefore use common i32/i64
  pools. The generated function's temporary count is now bounded by the
  largest member rather than the sum across all members.
- A structural unit test reads the generated Wasm local declarations and
  checks that exact bound. The current six-member live batch dropped 216 bytes
  (5,682 to 5,466 bytes) because local declaration counts crossed encoding
  boundaries; more importantly, the embedding baseline/optimizing compilers
  see fewer virtual locals as region population grows.

## 2026-08-05: mixed-state multi-entry execution

- Wasm validation does not prove that helper-visible memory state and cached
  architectural locals remain coherent. A standalone generated two-entry cycle
  now executes exact `fadd.d` through the real Wasm-to-Wasm helper while GPRs,
  raw FPRs, `fcsr`, PC, and retirement remain register-resident.
- Ten cycles produce raw double bits for exactly 6.0 from 1.0 plus ten 0.5
  additions, leave `fcsr` clear, increment both integer counters to ten, and
  commit exactly 50 retired instructions. This runs through the same host
  publication/import path as live JIT modules.

## 2026-08-05: precise SC side exits and bounded Linux gates

- The original FP/AMO artifact harnesses issued thousands of unconditional
  multi-million-instruction slices after boot and after every transfer chunk.
  One run spent more than eleven minutes without identifying whether boot,
  transfer, decode, or the guest program had stalled. Quoted guest markers and
  wall-clock bounds now terminate each phase as soon as its observable work is
  complete and report PC, retirement, and console tail on failure.
- With the pinned TinyEMU images, T0 reached first kernel output after
  40,000,000 instructions in 1.21 seconds. T1 instead retired more than a
  billion instructions with no output while alternating at
  `0xffffffe000233a5c/64`. Disabling JIT in that live machine immediately let
  Linux continue, isolating generated execution rather than image staging.
- The stuck instructions are Linux's acquire/release `lr.d`/`sc.d` loop. LR
  repeatedly read an unlocked zero while SC returned failure. The synthetic
  system-A test had enabled Wasm-to-Wasm TLB refill, masking the default-policy
  path.
- Generated SC previously consumed the canonical reservation before checking
  its store translation. On a no-refill store-TLB miss, the precise side exit
  performed no store, but T0 retried with an already-cleared reservation and
  therefore reported SC failure. In the focused finite loop this lost exactly
  one update; Linux's retry loop could reproduce the miss indefinitely because
  a failed T0 SC never performs the store translation that would populate the
  row.
- SC is now two-phase: a non-destructive ownership probe controls the checked
  store, and a completed generated attempt clears the reservation afterward.
  A store-address side exit returns before the clear, matching interpreter
  re-execution. IR validation rejects conditional stores not paired with their
  SC probe. The focused full-system test passes with zero refill-helper calls
  in default mode and with one expected call in refill mode.
- A separate six-entry indirect cycle forces the reservation body into the
  register-resident T2 backend. It formed four batches containing 17 members;
  two emitted batch modules import the typed system-reservation helper. The
  no-refill run accumulated zero failed SCs, left the lock cell zero, matched
  all T0 completion state, respected the 100,100-instruction slice tolerance,
  and retired 117,647 instructions in generated code.
- The marker-driven FP context-switch gate now completes in about 17 seconds
  and both concurrent processes produce `0x29c0709f16c84da4`. The three-mode
  AMO artifact completes in about 5 seconds and interpreter, T1, and T2 all
  produce `0xec4798f5be2193b5`. The aggregate Wasm smoke also boots Linux and
  passes 9p, virtio-net, proxy, and relay checks.
- External conformance was run in the provisioned Nix shell: all 134 official
  `riscv-tests` passed; Spike lockstep passed 109/109 applicable binaries with
  24,103 register writebacks and one spec-legal skip; all 193 architecture-test
  signatures matched. The post-fix native matrix contains 145 Rust tests, and
  the generated-module smoke validates 338 modules.

## 2026-08-05: clean-room comparison with the removed JIT

- The removed implementation was rebuilt from an isolated `git archive` of
  baseline commit `4b0896decdff7538f9c1d2b44dc19a1d3d14f7c2`; its source was not
  opened. Both versions used Node 26.5.0 on the same host, the same pinned guest
  images/artifacts, and their equivalent best-of-N repository harnesses.
- On `tests/bench.mjs` (best of three), the removed/current results were:
  user integer+FP 63.0/160.6 ms (3,648/1,432 Minsn/s), Linux boot
  1,041.2/1,269.1 ms, and system MD5 837.2/951.8 ms. The large user-mode gap is
  credible; the smaller boot and MD5 gaps still need alternating-process samples
  before treating them as precise. The MD5 generated-retirement percentage is
  known to exceed 100%, so it is not used as evidence.
- A same-artifact, JIT-only fixed-work comparison (best of five) was more
  diagnostic. The integer ALU kernel was effectively tied at 1,810/1,794 ms,
  while the mixed kernel was 1,689/2,778 ms (removed/current), making the new
  path about 1.64x slower despite 100% current coverage and matching checksums.
  The deficit is therefore not uniform and is not explained solely by dispatch
  coverage or cold compilation; mixed-operation lowering and generated-code
  shape are the immediate profiling targets.
- The new compiler is roughly 10x faster on the user workload than the much
  older 1,676.7 ms historical baseline recorded in `tests/BASELINE.md`, but it
  has not yet recovered the performance of the implementation deleted at the
  clean-room baseline commit.

## 2026-08-05: latest upstream and modern boot paths

- The rewrite was moved from its original clean-room baseline onto upstream
  `96aa93896e7bb6fa561d1f977c9bf23cd909a100`. The upstream image/runtime work
  makes Alpine and the modern Virt machine the supported browser path; TinyEMU,
  BBL, and the old kernel are no longer the delivery target.
- The JIT runtime now selects the correct full-system machine layout for both
  the legacy and Virt implementations. The modern path uses the Virt CPU, RAM,
  translation rows, page marking, dirty-page checks, async publication, and
  dispatch state directly rather than adapting through the old machine.
- Direct boot enters Linux at `RAM_BASE + 2 MiB` in S-mode with the emulator's
  SBI implementation. Firmware boot starts OpenSBI `fw_dynamic` in M-mode with
  the generated DTB and dynamic-info address, then transfers to the same kernel.
- In the strict Node 20 run, direct boot retired 412,002,008 guest instructions,
  338,816,766 in generated code, and landed 18 T2 regions. OpenSBI boot retired
  426,002,120 instructions, 383,077,088 generated, and landed 13 regions. Both
  reached the Alpine shell and completed the same bounded shell workload; the
  direct path reported zero unsupported SBI extensions.
- The independent OpenSBI/Linux Virt smoke gate reached `SMOKE_START`, then
  validated `RDTIME`, RTC, UART draining, forks, and shutdown before
  `SMOKE_OK`.

## 2026-08-05: randomized T2 reservation and atomic hardening

- A deterministic generator now combines LR/SC width W/D, interfering and
  non-interfering stores, refill/no-refill system-memory policy, region/batch
  T2 formation, eager/lazy state, and two precise-fault sites.
- All 12 seeds match interpreter state, memory, PC, retirement, reservation
  behavior, and completion. The aggregate run retired 49,579,271 guest
  instructions in generated code and structurally found reservation imports in
  eight T2 modules.
- This extends the earlier fixed LR/SC regression from one known instruction
  shape to generated combinations whose tier transition occurs at different
  points. No seed depends on a Wasm trap or an assumed filled store TLB.

## 2026-08-05: state and dispatch backend selection

- The frozen synthetic corpus now has eager register state, lazy valid-bit
  state, direct structured dispatch, materialized members, same-module tail
  calls, and corresponding full-system-memory variants. Each of seven trials
  starts a fresh Node process or browser process/profile and alternates paired
  order.
- On the integer-state kernel, eager versus materialized paired median speedup
  was 4.432x in Node 26/V8 14.6, 4.559x in Chrome 150, and 3.250x in Firefox
  153. Lazy/eager was 0.436x, 0.596x, and 0.364x respectively. Direct
  structured/balanced was 0.333x, 0.304x, and 0.364x on that deliberately
  dispatcher-heavy shape. These are backend diagnostics, not whole-emulator
  throughput claims.
- Same-module tail calls were essentially neutral against materialized state in
  Node (1.010x) and Chrome (1.013x), but Firefox's integer result was 0.277x.
  Firefox supports the proposal; the regression is generated-code quality, not
  a feature-detection failure.
- The one-load/one-store translation proof cache remains engine-dependent:
  cached/no-cache paired medians were 1.289x in Node, 0.836x in Chrome, and
  1.800x in Firefox. This directly rejects a universal default based on one
  embedding engine.

## 2026-08-05: compiler-generated real-region corpus

- A dependency-free ELF64 RISC-V parser captures entry, `main`, and large
  symbol-delimited functions from `rvbench.rv64` and Alpine musl's dynamic
  loader. It evaluates one-, two-, and three-page regions at 32 leaders, then
  three-page regions at 64, 128, 256, and 512 leaders.
- The matched corpus has 56 regions. Five backend forms produce 280 modules;
  the manifest records source path/hash, selected function/geometry, backend,
  Wasm size, module hash, and relevant structural metadata.
- Across the 14 source/geometry cohorts in the regenerated eight-process Node
  report, lazy/eager size ranged from 1.954x to 2.175x and compile-time median
  from 1.896x to 2.892x. Direct/eager size stayed between 1.008x and 1.024x.
  Tail-call/materialized size stayed between 1.012x and 1.042x.
- Eager register residency costs 1.981x to 3.005x the materialized bytes in
  this corpus. Its compile premium grows on large `rvbench` regions, reaching a
  7.396x median at the 256-leader geometry in this sample. That is a real cold
  cost to trade against the 3.25x-4.56x hot execution win; it is why region
  population is bounded rather than maximizing coverage.
- Current Chrome and Firefox runs compile the exact same hashes from fresh
  profiles and show the same qualitative frontend result: lazy state expands
  modules, direct dispatch is near eager, and very large register unions have a
  disproportionate baseline-compiler cost.

## 2026-08-05: live modern-Linux region policy

- A fresh-Node/fresh-VM alternating sweep performs a direct Linux 6.12 boot and
  a deterministic 516-million-instruction shell workload for each cap. It
  records raw boot/work time, generated retirement, dispatches, translation,
  copied/module bytes, async compile/instantiate/publication, and every region
  hash/size.
- With three pages, 32 leaders achieved a median 87.208 Minsn/s at 92.57%
  generated coverage, 64.0 ms translation, 4.32 MiB emitted bytes, and a
  207,257-byte maximum region. At 512 leaders the result was 75.225 Minsn/s at
  92.87% coverage, 280.9 ms translation, 26.68 MiB emitted bytes, and a
  1,397,912-byte maximum region.
- The extra 480 leaders therefore changed coverage by only 0.30 percentage
  points while making the workload about 16% slower, translation about 4.4x
  larger, and emitted bytes about 6.2x larger. The leader default is 32.
- At 32 leaders, one-, two-, and three-page medians were 87.206, 87.805, and
  87.208 Minsn/s. With three samples their uncertainty overlaps; three pages is
  retained for cross-page reach rather than claimed as a speed win.

## 2026-08-05: current engines and standalone execution

- The delivery measurement set records Node 26.5.0/V8 14.6, Chrome 150,
  Firefox 153.0.3, and Wasmtime 47.0.3. Firefox 141 removed its old CDP
  endpoint, so the runner now uses WebDriver BiDi with a fresh temporary profile
  and WebSocket session while retaining CDP for Chromium.
- Wasmtime executes all 11 frozen synthetic modules through preloaded `env` and
  `jit` modules and a generated driver. Exact visible state is checked rather
  than treating validation as execution. It also AOT-compiles all 56 eager real
  regions successfully.
- Reports retain raw samples, conventional medians, min/p95/max, and a
  deterministic 4,096-resample bootstrap 95% interval for the median. The
  even-sample Node real-region report was regenerated after correcting median
  calculation to average its two middle observations.

## 2026-08-05: Phase 6 delivery matrix

- The strict Nix-shell matrix completed with `ALL STAGES PASSED` on the current
  upstream tree. It includes 145 Rust tests, three QEMU differentials, 134/134
  official ISA tests, 109/109 applicable Spike locksteps with 24,103 register
  writebacks, and 193/193 architecture signatures.
- The Wasm stage validates 338 generated modules, matches 60 randomized
  full-state programs, and passes all directed scalar integer, M, A, F/D,
  memory, Sv39, MMIO, precise-exit, SMC, and T2 tests. The previously
  timing-sensitive async T2 test also passed five consecutive Node 20 runs
  after honoring `sys_pending_builds()`.
- Standalone Wasmtime, both modern Linux boot paths, FP context switching, the
  interpreter/T1/T2 AMO artifact, and the modern Virt smoke workload are part
  of the same strict command. No tool-dependent stage was skipped.

## 2026-08-06: same-commit black-box comparison

- The previous build was archived and compiled from upstream
  `96aa93896e7bb6fa561d1f977c9bf23cd909a100`, the same commit as the rewrite.
  Its compiler source remained unopened. The accepted run used nine alternating
  pairs, a fresh Node 26.5/V8 14.6 process for every leg, exact input and binary
  hashes, a 1.25x CPU-probe rejection threshold, and bootstrap intervals. Its
  probe spread was 1.197x and every functional check passed.
- Paired old/new time ratios were 0.499x for the 8-billion-instruction ALU
  kernel, 0.696x for the 1.64-billion-instruction mixed user kernel, and 0.936x
  for fixed legacy-Linux MD5 work. The rewrite is therefore about 2.00x, 1.44x,
  and 1.07x slower respectively where both implementations generate code.
- The previous JIT records no entries, modules, or generated retirement on the
  modern Virt machine. The rewrite's fixed shell work is 1.965x faster under
  direct SBI at 92.75% generated coverage and 1.977x faster under OpenSBI at
  98.33%. These are product-capability results, not old-JIT/new-JIT code-quality
  results.
- Modern direct cold-to-prompt is tied. OpenSBI rewrite cold-to-prompt is
  0.587x the previous speed, or about 1.70x slower. A representative rewrite
  OpenSBI run registered 5,394 modules and 11.69 MiB versus 1,889 modules and
  3.93 MiB for direct boot, identifying eager firmware/early-boot translation
  as the first cold-start policy target.
- The hot ALU module exposes one-local-per-SSA lowering: 35 locals and 86 local
  accesses in the rewrite versus 15 and 45 previously. Liftoff-only reproduces
  the 1.96x deficit; TurboFan-from-start is tied. Normal V8 compiles optimized
  code after entry but cannot replace the active long Wasm call. Re-entering
  every one million guest instructions makes the normal-tier result tied at
  1.010x, using 8,001 calls per side.
- ALU cold-tier latency has a repeatable tail. The ordinary nine-pair run had
  one 13.01-second rewrite sample among eight 3.92-4.18-second samples. A
  separate CPUs-8-15 affinity run had one 11.16-second rewrite sample and a
  0.443x paired median; its previous samples all stayed within 1.964-1.967
  seconds. Normal bracketing probes and repeated direction under affinity mean
  the outliers cannot simply be removed as scheduler noise.
- Mixed code has 99.91% rewrite coverage versus 98.08% previously, but makes
  105,986,488 generated dispatches versus 59,804,397. Its modules are much
  smaller (15,803 versus 126,638 bytes), so frontend size and coverage do not
  explain the gap. Liftoff-only is 0.503x and TurboFan-from-start is 0.605x;
  local/stack quality and same-module edge fusion are independent needs.
- Supporting three-pair runs show the result is engine-sensitive. Deno/V8
  reports 0.503x ALU and 0.652x mixed ratios; Bun/JavaScriptCore reports 1.042x
  ALU and 0.526x mixed. Exact runtime identity and cold/re-entry/stabilized-tier
  regimes are required for future claims.
- The full report, intervals, artifact hashes, caveats, and reproduction
  commands are in [COMPARISON.md](COMPARISON.md).

## 2026-08-06: WANIX three-way integration and WFI host yield

- A new WANIX page packages copy/v86, the immutable rv64.js `v0.1.0`
  release, and the workspace rewrite under three distinct VM types. The legacy
  loader and core are verified against pinned SHA-256 values before packaging;
  both RV64 archives use the same current adapter and guest filesystem.
- Browser boot exposed an event-loop starvation bug rather than a 9P protocol
  bug. After WFI, an interpreter call can retire zero instructions. Treating
  that as one unit of synthetic budget consumption kept a two-million-unit
  `virt_run` call spinning while the external 9P reply was queued in
  JavaScript, so userspace root loading appeared frozen.
- Both cold and warm interpreter fallback paths now return to the host on zero
  architectural progress. `jit-system-wfi-yield.mjs` exercises the empty-cache
  and live-JIT cases and asserts that interpreter-call count stays bounded
  independently of the caller's two-million-instruction budget.
- Yielding exposed the second half of the wake-up contract: the host could
  deposit an external 9P reply without publishing the used-ring entry until a
  later interpreter poll. A hart already in WFI then had no virtio interrupt
  to wake it; an unrelated UART keypress appeared to unstick boot. External 9P
  reply delivery now processes the queue immediately and raises its own IRQ.
- In a clean Chrome profile, copy/v86 and the pinned legacy RV64 reached shell
  prompts. After both wake-up fixes, the rewrite also reaches its prompt with
  no injected input and executes a command through the browser terminal. Init
  no longer recreates every BusyBox applet link when Alpine already packaged
  them, avoiding hundreds of serialized 9P mutations, and comparison mode
  skips the optional fetch-proxy mount unused by the benchmark.

## 2026-08-06: matched i686 and RV64 WANIX roots

- The initial three-way page was not an equivalent guest comparison. Its v86
  pane consumed WANIX's stock `wanix-linux.tgz`: Alpine 3.22.5/x86 with Linux
  6.16.3, the stock init, and no Python. The RV64 panes consumed the custom
  Alpine 3.22.5/riscv64 archive with Linux 6.12.7, the optimized init, and
  Python.
- The guest builder now selects either Alpine `x86`/Go `386`/i686 or RISC-V 64
  from one recipe. Both outputs have the same seven-entry apk world, including
  Python, identical init bytes, Linux 6.12.7 from the same pinned nixpkgs input,
  and helpers built from WANIX commit
  `6594fe3763eb8712e81914f78b79243bb403f5cc` with the same runtime patches.
- Kernel hardware differences remain intentional: copy/v86 needs an i686
  `bzImage`, virtio-pci, 9P root, and `hvc0`; rv64.js needs a RISC-V `Image`,
  virtio-mmio, 9P root, and `ttyS0`. The shared init derives its interactive
  device from the kernel's `console=` argument, leaving the other device for
  hostexport.
- `tests/wanix-v86-matched-smoke.mjs` starts only the reference pane in a fresh
  Chrome process/profile, reaches the shell, verifies `i686`, Alpine 3.22.5,
  and Python 3.12.13, then completes pure-Python, SHA-256, and shared-9P phases.
  Its one successful validation run is not a controlled performance claim.

## Remaining optimization candidates

These are future experiments, not incomplete Phase 5/6 gates:

- baseline-friendly loop stackification and liveness-based local reuse;
- selective re-entry for long single-entry loops so optimized Wasm can take
  over without imposing a global dispatch tax;
- direct same-module edge continuation to reduce mixed-workload dispatches;
- delayed or higher-threshold firmware-era compilation during OpenSBI boot;
- profile-guided hot/cold architectural state partitioning and live-exit-aware
  region selection beyond static page/leader caps;
- Memory64 versus memory32 translation/range-check cost and browser limits;
- Safari/JavaScriptCore and in-process Wasmtime Winch/Cranelift comparison;
- shared-memory multi-hart atomics and a separately advertised RVA23/RVV
  compiler contract.

## 2026-08-06: disabled-JIT baseline must bypass the dispatcher

- Raising the tier threshold to `u32::MAX` prevented compilation but did not
  reproduce the legacy modern-Virt interpreter path. Every public two-million
  instruction call still entered the rewrite dispatcher, performed cache and
  hotness bookkeeping, and ran the interpreter in up to 4,096-instruction
  chunks.
- The RV64 instruction interpreter itself was unchanged; the extra driver was
  the confirmed source of the misleading interpreter-only slowdown.
- `jit_set_enabled(0)` now selects the direct machine runner before any JIT
  synchronization or state lookup. The modern path therefore matches the
  legacy shape: realtime update, one `VirtMachine::run_slice(max_insns)`, then
  host-I/O drain.
- A regression treats dispatcher counters as a structural proof: disabled
  modern Virt execution must leave JIT retirement, dispatch, cache, fallback
  slice, translation, and module-registration counts at zero.
- Three alternating fresh-process pairs on Node 26.5/V8 14.6 ran the packaged
  legacy and rewrite cores for 100 million modern-Virt loop instructions each.
  Rewrite/legacy throughput was 0.995x, 1.015x, and 1.005x (1.005x median),
  with every checked JIT activity counter zero on both sides. This narrow
  diagnostic is not a Linux benchmark; it verifies that the prior large
  interpreter-only gap is gone at the driver level. Reproduce it with
  `node tests/interpreter-old-new-baseline.mjs` after building the comparison
  archives.

## 2026-08-06: matched CPython workload is CPU-bound and exposes unfinished optimization work

- A fresh Chrome 150 profile ran `/shared/bench.py` on roots matched for Alpine
  3.22.5, Linux 6.12.7, Python 3.12.13, package world, init, and WANIX helper
  sources. The copy/v86 i686 versus rewrite-RV64 times were 2.016 versus
  10.695 seconds for pure Python, 4.581 versus 34.318 seconds for SHA-256, and
  1.049 versus 1.711 seconds for shared 9P I/O.
- The corresponding rewrite/v86 ratios are 5.30x, 7.49x, and 1.63x. This is a
  one-sample diagnostic rather than a controlled performance claim, but it
  rules out shared-root I/O as the dominant cause of the observed gap.
- The active production policy waits for 1,048,576 sampled instructions on a
  safe `(VA page, PA page)` identity, permits one async Wasm build in flight,
  and issues one page with at most 32 observed leaders. That policy performed
  well on the compact calibration loops but has not been tuned for CPython.
- CPython's computed-goto bytecode dispatch presents many indirect targets.
  The current RV64 lifter terminates a region at `JALR`, and production page
  issuance does not use the compiler's available multi-page reachability.
  Those choices increase fallback and generated dispatcher crossings.
- Generated regions containing guest-memory effects deliberately use one Wasm
  local per SSA value to preserve precise side exits. Liveness-based local
  reuse and broader stackification remain performance work for this workload.
- SHA-256 is also affected by architecture-specific guest code and crypto
  libraries, so it must not be treated alone as backend quality. The 5.30x
  pure-Python result remains direct evidence that the rewrite is not yet
  performance-complete.

## 2026-08-06: CFG Stackifier is implemented separately from value stackification

- The cached multi-entry backend now has a distinct structured-CFG mode. It
  forms SCCs, lowers reducible cycles to nested Wasm `loop`/`block` scopes,
  uses direct depth-indexed branches for known internal edges, duplicates only
  small multi-entry SCCs under an explicit code-size budget, and localizes a
  selector dispatcher to larger irreducible SCC headers. Unknown indirect RV64
  targets remain precise exits.
- Directed execution tests cover reducible loops, multiple callable entries,
  irreducible dispatcher fallback, bounded duplication, cross-module tail
  transfer, fuel accounting, and exact architectural state. The ordinary T2
  differentials also exercise structured mode over randomized and system-memory
  cases.
- This is the Stackifier family described by Yuri Iozzelli in Leaning
  Technologies' [structured-control-flow article](https://medium.com/leaningtech/solving-the-structured-control-flow-problem-once-and-for-all-5123117b1ee2)
  for Cheerp and LLVM-to-Wasm, not a claim of source identity with Cheerp's
  dominance-constrained topological ordering and if/else-nesting heuristics.
  The rewrite's earlier “small stackifier” still refers only to pure SSA value
  stackification; the two optimizations remain separately selectable.
- The current emitter conservatively passes every captured region member as an
  externally callable entry to `stackify`. This preserves exact arbitrary-PC
  re-entry, but the synthetic external predecessor can make an otherwise
  single-entry cyclic SCC appear multi-entry and select bounded duplication or
  a local dispatcher. A follow-on should first count those classifications and
  their hot execution, then test a smaller observed-entry set (with precise
  fallback for uncommon entries) against arbitrary-entry differentials, Wasm
  size/publication time, boundary density, and end-to-end workload time.
- Browser measurements support the article's scale estimate: structured
  control flow is useful backend hygiene, but the multi-fold WANIX gap closed
  only after fixing driver bypass, WFI scheduling, generated coverage, and
  workload-adaptive region admission. Region composition and asynchronous
  Wasm tiering dominated the final result.

## 2026-08-06: CPython and SHA require adaptive region geometry

- Raising the region leader cap was the largest single coverage correction.
  With generated TLB fill disabled, one browser diagnostic moved from
  3-page/256-leader Python 2.531 seconds to 3-page/512-leader 2.356 seconds;
  2-page/512-leader measured 2.328 seconds. A one-page/512-leader region reached
  1.993 seconds for Python but regressed SHA to 5.373 seconds. No single fixed
  page cap served both workloads.
- An entry-count-only multi-page gate did not separate the workloads. A cap of
  eight protected Python but blocked useful SHA pages; caps 16 and 32 admitted
  both. Excluding cross-page direct calls also failed: with the then-current
  3-page/128-leader diagnostic it measured Python 3.393 and SHA 4.155 seconds.
- Eager measured extension was harmful. Demotion reduced Python generated
  coverage to about 6% and raised it to 13.6 seconds. Keeping extensions while
  disabling demotion preserved Python but moved SHA to about 6.1 seconds;
  bounded adjacent-only and minimum-stay variants did not recover it. Measured
  extension and demotion therefore remain off by default.
- Sampled non-sequential/control-entry density did separate the workloads.
  The selected gate permits a second reachable page only when each page has at
  most 100 control entries per thousand observations. With page cap 2 and
  leader cap 512, CPython normally records two or three eligible pages and
  blocks roughly 19 or 20, issuing no multi-page region. SHA records roughly
  eight to ten eligible pages and issues eight or nine multi-page regions.
- The cold interpreter path initially failed to record control entries because
  it used the unsampled runner before precise stopping was requested. Selecting
  the sampled runner when control-entry classification is enabled fixed the
  cold-page evidence. A synthetic regression now proves that a zero-per-thousand
  gate blocks a page whose first cold sample contains a control entry.

## 2026-08-06: frame-free chaining needs one table-owning trampoline

- Structured CFG removes most in-region dispatch, but a hot successor in a
  different generated module still needs a fast transfer. Recursive
  `chain_next` calls grow the Wasm stack and require a defensive depth cap.
  Direct `return_call_indirect` initially made every generated module import
  the shared function table; measurements exposed V8 publication work growing
  with the number of table-importing instances.
- The stable design creates one tiny host-side Wasm module. It alone imports
  the function table and exports `(state, table_index) -> ()` implemented by
  `return_call_indirect`. Generated modules import that ordinary
  `env.tail_chain` function and tail-call it. They no longer import the table,
  while the complete transfer remains frame-free and does not return through
  JavaScript.
- A feature probe enables this path only on engines that implement Wasm tail
  calls. Stable `RV64.create` enables structured-region tail chaining after the
  probe; `configureJit({regionTailChain:false})` remains an A/B control.
  `jit_page_policy_stat(40)` and `jitStats()` expose the selected state, while
  `chainHops` proves transfers occurred.
- The multi-entry differential verifies the generated import kind, absence of
  a table import, cross-module execution, exact fuel, side exits, invalidation,
  and lifecycle behavior. Chrome and Edge measurements both report the feature
  enabled and nonzero transfers in every phase.

## 2026-08-06: final policy outcomes must not pay repeated sampling cost

- Interpreter-instruction profiling of the remaining shared-9P gap found
  hundreds of thousands of short fallbacks at Linux privileged CSR paths. The
  dominant instruction words included `0x10002073`, `0x10007073`,
  `0x10006073`, and `0x10200073`. Those PCs deliberately leave generated code,
  but the control-entry sampler kept revisiting them even after compilation
  policy had reached a final outcome.
- For an exact `(VA page, PA page, PC)` already present in either
  `policy_attempted` or `policy_installed`, `page_policy_observe` cannot change
  heat, candidates, or emitted code. Such fallbacks now use the exact
  unsampled interpreter while retaining the same stop-at-compiled predicate.
  New mappings and new PCs still take the sampled path. This removes redundant
  policy work without changing guest execution or compilation decisions.
- A directed eight-byte S-mode loop (`csrrs sstatus`; backward `jal`) proves
  one initial failed policy attempt, then retires another 200,000 interpreted
  instructions with unchanged policy, sampled-retirement, and control-entry
  counters and the exact expected PC.
- The standalone Wasmtime audit also caught a test-fixture error: its fused-TLB
  row used VPN `2` instead of the canonical complete page-base tag `0x2000`.
  Correcting the fixture makes all 11 frozen backend variants reach exactly
  216,000 retired instructions and their expected state checks.

## 2026-08-06: final defaults beat copy/v86 compute and pass shared-9P parity

- The selected defaults are threshold 131,072, quantum 1,024, two async builds,
  page cap 2, leader cap 512, control-entry sampling enabled, and a 100-per-
  thousand multi-page control gate. Structured CFG/direct dispatch, eager
  state, and the feature-tested tail trampoline are enabled. Rebuild, measured
  extension/demotion, generated TLB fill/hash, and recursive chaining remain
  disabled.
- Chrome RV64/v86 raw seconds were Python
  `[1.760,1.761,1.829,1.743,1.765]` / `[1.866,2.262,1.943,1.978,2.121]`, SHA
  `[3.903,3.898,3.818,3.940,4.204]` / `[4.762,4.843,4.820,4.649,4.680]`, and
  shared 9P `[1.092,1.107,1.089,1.095,1.109]` /
  `[1.092,1.085,1.057,1.048,1.099]`. Paired geometric means and exact 95%
  intervals were 0.873 `[0.820,0.930]`, 0.832 `[0.803,0.867]`, and 1.021
  `[1.008,1.035]`, respectively.
- Edge RV64/v86 raw seconds were Python
  `[1.744,1.777,1.754,1.764,1.847]` / `[2.001,1.981,1.898,2.035,1.970]`, SHA
  `[3.954,3.968,3.862,4.105,3.842]` / `[4.560,4.548,4.666,4.601,4.643]`, and
  shared 9P `[1.193,1.246,1.163,1.200,1.176]` /
  `[1.142,1.148,1.163,1.120,1.140]`. Paired geometric means and intervals were
  0.899 `[0.875,0.924]`, 0.857 `[0.835,0.879]`, and 1.046
  `[1.020,1.072]`.
- All six interval upper bounds are below the predeclared 1.10 limit. The
  rewrite is therefore faster on pure Python and SHA in both Chromium engines,
  while the I/O phase is within 2.1% in Chrome and 4.6% in Edge. No sample was
  discarded or replaced by best-of-N.
- The runner pre-registers three to seven fixed pairs, alternates which VM runs
  first, holds a global lock across the complete experiment, pins browser and
  harness to CPUs 8–15, and gives every leg a new process, profile, and guest.
  The analyzer rejects chronology/overlap violations, a changed browser or
  artifact, incorrect ISA/Alpine/Python/checksums, an override, unexpected
  runtime defaults, inactive tail chaining, or generated coverage below 90%.
- The immutable page hash is
  `34a0d0a70730549d68582432f0a207535db3fa81bb468d17585f1d30cb834588`.
  Its RV64 JIT archive is
  `2598532832a2b9ad27ca99889a70cc8ab42796296990b91dd2b60e70c793be86`
  and contains Wasm
  `7136691c8ba0ab5ee3f4c9b3e74cd9f09af73bf43bb19b71cad97cff66a241ac`.
  The matched RV64/i686 root hashes remain
  `274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb`
  and
  `09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320`.
- Raw protocols, logs, and reports are retained under
  `target/jit-policy-traces/wanix-parity-known-fallback-pinned-8-15-{chrome,edge}150-20260807/`.
  The first immutable tail-trampoline control is also retained under
  `wanix-parity-tail-trampoline-pinned-8-15-chrome150-20260807/`: its shared-9P
  geometric mean was 1.090 and its interval `[1.076,1.110]` narrowly failed the
  gate. An unpinned noisy run and an Edge sequence whose archive changed
  mid-experiment were explicitly invalidated rather than folded into the
  accepted result.
- The final strict Nix release command reports `ALL STAGES PASSED`: 164
  workspace Rust tests, three QEMU differentials, 134/134 official ISA tests,
  109/109 Spike locksteps, 193/193 architecture signatures, all Node/Wasm
  differentials and lifecycle gates, standalone Wasmtime 47, direct/OpenSBI
  modern Linux, FP/AMO guest checks, and the OpenSBI/Linux Virt smoke. No
  tool-dependent stage was skipped.

## 2026-08-07: historical rv64.js scorecard exposes the compatibility-policy gap

- The complete authoritative rv64.js scorecard ran with `AUTHORITATIVE=1`,
  `NBENCH=1`, `SB=1`, `REPS=3`, and `NBREPS=3`, pinned to CPUs 8-15. Every leg
  used a fresh process and paired order alternated. The report is
  `/home/darren/src/arm64.js/target/bench/scorecard-2026-08-07T16-13-33.json`;
  it is valid, has no checksum/input/provenance problem, and recorded a 1.08x
  host-probe spread. The active Wasm hash was `7136691c8ba0...`.
- The rewrite scored 4/13 against copy/v86. It won ALU (1.43x), BITFIELD
  (2.37x), and IDEA (2.74x), and matched FP EMULATION (1.03x). It lost Mixed
  (2.75x behind), matched Boot (1.70x), Python `fib(30)` (2.10x), Compile
  (3.30x), Numeric Sort (8.42x), String Sort (14.44x), Fourier (1.77x),
  Assignment (1.41x), and Huffman (1.78x).
- This is a real result for the requested historical configuration rather than
  an artifact mismatch. The v86 revision, v86 Wasm, Node version, and all 14
  scorecard workload hashes exactly match the two valid 11/13 legacy-JIT
  reports from August 1. The unchanged v86 ALU/Mixed/Python/Compile medians
  remain close. Boot is intentionally not directly comparable: K007 replaced
  the old unrelated boot images with matched Linux 6.12.7/Alpine artifacts.
- The rv64 BYTEmark samples also expose lifecycle instability. Numeric Sort was
  `391.62`, `46.471`, and `46.152`; IDEA was `5663.8`, `5458.4`, and `1552.3`.
  One of three rv64 processes reported BYTEmark's internal instability warning,
  which is below the harness's majority-invalidation rule, so the report is
  formally valid but the extreme cross-process modes are themselves a defect
  to investigate.
- The historical legacy-system worker imports `RV64Debug`, enables `SB=1`, and
  leaves `jit_set_page_policy` disabled; matched Boot separately uses
  `RV64Debug` defaults. Consequently neither path measures the stable
  `RV64.create` configuration, which enables the bounded async page-heat policy
  and, when feature-tested, the frame-free tail-call trampoline. Do not present
  4/13 as the production browser-policy score. Preserve this unchanged run as
  the legacy-protocol baseline, then add an explicit, provenance-recorded
  production-policy mode and run focused Mixed/Numeric/String gates before
  repeating all 13 rows.

## 2026-08-08: clean production policy closes at 11/13, with only Boot and Compile open

- The current clean artifact is Wasm
  `d93345139c5a74ed9367fca1cd2c8b2e1491c5c86bb7179e77b0c329f218c96a`.
  All temporary post-R021 candidate and opportunity instrumentation was removed
  before measurement; the full correctness matrix and modern direct/OpenSBI
  boots pass.
- The fixed five-pair Chrome 150/V8 15.0 browser experiment used fresh browser
  processes/profiles, alternating order, CPUs 8-15, immutable archive
  `8d4aadfd0a4e52ca...`, one repetition per guest, and no replacement. Exact
  paired geometric-mean rewrite/v86 time ratios and 95% intervals were Python
  0.883 `[0.869,0.903]`, SHA-256 0.603 `[0.591,0.613]`, and shared 9P 0.697
  `[0.635,0.747]`. The raw protocol/logs and analysis are under
  `target/jit-policy-traces/wanix-r043-clean-d9334513-chrome-20260808/`.
- The earlier v86 9P timeout did not reproduce across eight fresh diagnostic
  guests, ten repeated shared-file cycles, or the formal five pairs. External
  tracing observed a maximum queue depth of one and zero tag collisions. This
  makes saturation/tag reuse false leads, but it does not establish a root
  cause; preserve the earlier run as invalid and retain pending request
  type/age diagnostics for any recurrence.
- The final authoritative report is
  `target/bench/r043-final-three-way/scorecard-v2-2026-08-08T15-55-38-777Z.json`
  (report SHA-256 `1d26ccfa5983f...`). It covers rewrite, isolated modern-Virt
  legacy, and pinned copy/v86; all 13 rows; three alternating fresh-process
  repetitions; production policy; v86 generated-dispatch proof; exact modern
  Linux 6.12.7/Alpine 3.24.1 inputs; CPUs 8-15; 236 host probes with 1.068x
  spread; and no validity problem.
- Rewrite versus copy/v86 medians are: ALU 1,772.2/3,231.4 ms, Mixed
  1,572.5/2,235.5, Boot 2,608.9/1,559.4, Python 3,076.0/3,421.4, Compile
  1,113.6/732.9, Numeric 284.4/308.5, String 232.7/235.4, Bitfield
  176.3/211.4, FP Emulation 266.8/871.1, Fourier 537.0/749.1, Assignment
  486.3/530.6, IDEA 359.5/537.6, and Huffman 282.8/1,702.3 ms. At the fixed
  10% rule this is ten wins, one match (String), and two losses (Boot and
  Compile): 11/13.
- Rewrite wins all 13 rows against the isolated legacy comparator. The smallest
  margin is ALU at 1.06x; Mixed is 10.77x, Python 22.16x, Compile 23.18x, and
  String Sort 187.64x. This resolves whether the rewrite is merely trading one
  legacy weakness for another: on the modern guest contract it is uniformly
  better.
- The remaining gaps are too large for another sub-10% micro-tuning series.
  Boot needs about 40% elapsed-time reduction to reach v86 and is predominantly
  privileged T0/runtime Wasm. Compile needs about 34% elapsed-time reduction;
  the RV64 workload executes 25.6% more guest instructions than i386 and the
  rewrite also remains slower per guest instruction. These are distinct
  lifecycle problems and require a new leverage analysis before selecting a
  structural mechanism.

## 2026-08-08: exact engine attribution and rejected decoded-page baseline

- Exact current-artifact profiles are retained in
  `target/bench/r045-final-engine-profile` and
  `target/bench/engine-profile-r045/phase`. Rewrite Boot sampled 2,740.66 ms:
  94.65% runtime Wasm and 3.88% generated Wasm, including 1,268.43 ms in
  `Cpu::step`. v86 Boot sampled 1,651.00 ms at 69.97% runtime and 21.53%
  generated Wasm. Rewrite Compile STEADY sampled 1,680.77 ms at 52.54%
  runtime and 46.37% generated Wasm; v86 sampled 838.15 ms at 22.70% and
  73.21%. This confirms two different lifecycle bottlenecks rather than one
  globally slow dispatcher.
- Rewrite Boot retired 180.75M guest instructions, only 37.26% generated, in
  ten generated modules totaling about 4.05 MiB. Compile STEADY retired
  325.76M, 92.32% generated, in 12 modules totaling about 4.65 MiB with
  550.9 ms host compile time. v86 Compile retired about 248.98M and needed one
  roughly 164 KiB generated module. Both guest work and emission shape matter.
- Proof-only `target/bench/r045-page-opportunity.json` traced the exact Boot
  inputs with JIT disabled: 438 physical code pages, 37.89% of instructions on
  the hottest page, 58.91% on the top ten, and 97.59% on the 94 pages executed
  at least 131,072 times. That was enough opportunity to test a physical-page,
  generation-guarded decoded executor; the selector contained no guest PC,
  symbol, benchmark, or browser identity.
- The first executor handled 110.56M instructions but made 25.35M page/block
  entries and regressed Boot 2,669.2 to 2,919.4 ms. Same-page control chaining
  reduced entries to 7.07M and regressed 2,651.6 to 2,742.2 ms. Replacing the
  per-instruction callback with direct opcode dispatch produced 2,664.6 ms,
  bracketed by adjacent controls at 2,651.6 and 2,709.2 ms. This is a tie, not
  an improvement claim, and is far below the fixed 10% replication gate.
- The final profile localized the failure: the packed decoded loop occupied
  essentially the same self time as the reference `Cpu::step`. Decode caching
  changes representation but retains an expensive Wasm-level operation
  dispatch and architectural-state update per guest instruction. All runtime,
  counter, and harness code was removed. The rebuilt release exactly matches
  `d93345139c5a74ed9367fca1cd2c8b2e1491c5c86bb7179e77b0c329f218c96a`.

## 2026-08-08: compact early Wasm passes coverage modeling but fails at runtime

- The corrected proof-only snapshot report is
  `target/bench/r046-early-wasm-opportunity-snapshotted.json`. It traces
  182,948,621 modern-Boot instructions on 438 physical pages. At a 200,000
  instruction crossing, 85 pages retain 159.20M post-threshold instructions.
  Eighty-four stable one-page, 64-leader register-structured modules average
  72.5 KiB, cover 99.07% of entry events, and give an optimistic coverage
  ceiling equal to 79.14% of production's interpreted Boot work. Isolated
  compilation of the twelve selected modules had 0.376 ms median and 1.753 ms
  p90 latency. These figures passed the preregistered opportunity gate but did
  not model when promises settle or how background V8 work competes with the
  running main Wasm instance.
- Exact experimental Wasm `77f2f9a2a42a...` supplied both control and candidate
  legs. The only difference was a default-off, architecture-general privileged
  policy selecting the 200,000/one-page/64-leader geometry. Order-reversed
  reports are under `target/bench/r046-compact-{control,candidate}-{a,b}`.
  Control Boot was 2,581.8 and 2,583.2 ms; candidate Boot was 3,763.2 and
  3,762.2 ms. Every modern guest/readiness marker passed and host-probe spread
  was at most 1.0143x, so the 45.7% regression is causal rather than throttling
  or a guest failure.
- Control Boot built 10 modules, emitted 3.91-3.98 MiB, accumulated 412-418 ms
  host compile latency, and retired 69.4M generated instructions. The compact
  candidate built 51-52 modules, emitted only 2.76-3.38 MiB, accumulated
  6,774-6,938 ms latency, and retired 82.4-83.5M generated instructions. Thus
  smaller aggregate bytes did not mean cheaper live compilation: only 13-14M
  additional instructions became generated before readiness, far below the
  offline upper bound, while many concurrent modules consumed engine work.
- Compile STEADY tied: the two-run control/candidate medians were 1,085.0 and
  1,094.5 ms with the exact expected object MD5. There is no compensating win.
  Do not tune heat, leader count, page count, or concurrency around this result;
  R022, R035, and R005 already close those neighboring policies. The new
  architectural constraint is stronger: a useful Boot baseline must amortize
  more than one guest instruction per already-compiled main-module dispatch or
  otherwise avoid runtime Wasm compilation entirely.
- All R046 runtime code, trace snapshots, counters, exports, and harness
  switches were removed. Formatting, 160 relevant Rust tests, and scorecard
  syntax checks pass. The release module again exactly hashes to
  `d93345139c5a74ed9367fca1cd2c8b2e1491c5c86bb7179e77b0c329f218c96a`.

## 2026-08-08: exact triples clear Boot alone but fail architecture transfer

- R047 first counted 62 normalized scalar RV64 operations inside page-local,
  context-stable dynamic basic blocks with JIT disabled. Modern Boot was
  99.40% eligible. A fixed-width ideal suggested that three-instruction groups
  could remove 58.63% of all dispatches, but that bound and the raw top-pattern
  counts allowed overlapping occurrences and were not implementation evidence.
- A second fresh Boot replay installed the first trace's immutable top-256
  triples and selected them greedily without overlap. Every eligible operation
  was accounted by `resulting dispatches + savings = original dispatches`.
  Libraries through 128 failed the 40% gate; 256 handlers barely passed at
  75,034,540 of 183,873,072 dispatches removed, or 40.81%.
- The same frozen Boot-derived library then failed the required transfer test:
  31.20% dispatch removal over the exact Compile workload and 24.78% over
  Python `fib(30)`. Compile retained object MD5
  `24eedf7e06beffd4d3ba1945585588db`; Python returned `832040`. Reports are
  `target/bench/r047-superinstruction-{opportunity,exact-replay}.json` and
  `target/bench/r047-superinstruction-transfer-{compile,python}.json`.
- Selecting new triples after seeing each failing benchmark would be corpus
  specialization, not evidence that one architecture-level precompiled
  library solves the emulator. The exact-pattern design is therefore closed
  before implementation. All runtime tracing/replay code was removed, 30 core
  and one Wasm unit test pass, and release Wasm is again byte-identical to
  accepted `d93345139c5a...`.
- R045-R047 now close decoded one-instruction handlers, cold per-page runtime
  Wasm, and exact precompiled opcode triples for Boot. The next structural
  attribution moves to the nested Wasm engine itself: establish whether large
  generated Compile functions tier from the baseline compiler, how long that
  takes, and whether function/module geometry—not another guest selector—is
  keeping useful code in a weak engine tier.

## 2026-08-08: late tiering is real, but removing oversized overlap is a wall-time tie

- Diagnostic-only V8 14.6 traces use explicit lifecycle markers and exact
  Compile output; their compilation sums are parallel engine work, not wall
  time. Rewrite produced 84 observed generated functions totaling 33,025,665
  body bytes, with median 243,229, p90 644,518, and maximum 3,223,396. v86
  produced 100 totaling 10,499,101 bytes, with median 113,437, p90 169,810,
  and maximum 223,429. Reports and raw traces are under
  `target/bench/r048-engine-tier`.
- Rewrite emitted one approximately 3.22 MiB function in each measured Compile
  phase. FIRST's 3,221,529-byte body entered Liftoff immediately and finished
  its 1.544 s TurboFan job during PRIME. PRIME's 3,223,396-byte body finished a
  1.458 s job during STEADY. STEADY's 3,221,529-byte body finished its 1.528 s
  job after the result. This proves late tiering and background overlap, but
  does not by itself prove synchronous wall-time leverage.
- Module capture and the existing exact page-template diagnostic identify all
  three as a single 512-entry physical code page mapped at different ASLR
  virtual addresses. The ordinary modules are 3,223,473 bytes in FIRST and
  3,221,606 in PRIME/STEADY; pairwise bytes differ by only about 0.79%.
- The structural cause is independently lifted straight-line suffixes: ending
  a member before the next known callable entry reduced the 512-entry module
  to about 79.8 KiB, or 97.5%, while retaining every entry. The first form also
  broke the existing whole-loop bulk-copy recognizer, so the tested form kept
  loop, dense-copy/store, and bulk-copy members intact. It preserved exact
  Compile MD5 and 12,122-12,125 bulk-copy calls.
- The general all-region form failed a valid two-pair same-Wasm A/B: Compile
  STEADY was 1,133.9 to 1,182.1 ms (0.959x), while Boot tied at 2,617.2 to
  2,580.3 ms (1.014x). A final fixed geometry rule applied splitting only when
  the ordinary module exceeded 1 MiB and the alternative removed at least 75%
  of bytes. It cut STEADY emitted volume by 57-62% but measured 1,073.2 to
  1,075.6 ms (0.998x); Boot measured 2,613.8 to 2,663.8 ms (0.981x). Exact
  fingerprints passed and host-probe spread was 1.019x. Reports are under
  `target/bench/r048-{split-entry,large-overlap}-ab`.
- Therefore oversized nested Wasm and late TurboFan jobs are observable but
  not the remaining scored limiter. Do not sweep module sizes, overlap ratios,
  or boundary layouts around a result whose near-impossible-elimination form
  is a tie. Every candidate path is removed; 53 DBT, 30 core, and one Wasm unit
  tests pass, and release Wasm is again byte-identical to accepted
  `d93345139c5a...`. The diagnostic trace/capture support and generic
  `scorecard-v2-config-ab.mjs` remain because they change no production path.

## 2026-08-08: affine stack behavior is nearly universal, but carrying two page proofs regresses

- The retained QEMU plugin diagnostic now classifies every dynamic `x2` write
  and relates every `x2`-based effective address to the live stack page. In the
  exact Compile workload, 6,302,496 of 6,302,585 writes (99.9986%) were
  affine-immediate updates; only 88 were affine-register updates and one was
  another form. Of the affine-immediate changes, 5,851,924 (92.85%) stayed on
  the same 4 KiB page and 450,572 crossed a page.
- The same run observed 82,473,135 stack-root memory operations, or 13.09 per
  dynamic `x2` write. Exactly 76,632,457 (92.92%) used the current stack page
  and 5,840,678 (7.08%) used the following page. None used the preceding page
  or any other page, and a temporal two-page model missed only 9 events. This
  is much stronger evidence than the earlier first-value and recent-page
  locality counts and was sufficient to admit one carried-proof design.
- The implementation retained independently faultable load and store offsets
  for the current page and its successor across structured members, filled
  them on demand through the exact fused-TLB/refill path, and rotated or
  invalidated them when generated code changed `x2`. The selector used only
  RV64 register/dataflow and page geometry. Exact differentials and 54 DBT
  units passed, and the modern Compile object remained
  `24eedf7e06beffd4d3ba1945585588db`.
- The valid two-repetition alternating same-Wasm A/B is
  `target/bench/r049-stack-carry-ab/config-ab-2026-08-08T18-50-46-285Z.json`.
  With 1.012x host-probe spread, Compile STEADY changed from 1,052.18 to
  1,484.94 ms: 0.709x throughput, or a 41.1% regression. The invariant is real,
  but selecting a slot, checking validity, carrying extra locals, and updating
  them costs substantially more than V8's optimized ordinary fused-TLB path.
- No threshold or page-count tuning is justified around a regression this
  large. The rejected Wasm is archived as
  `target/bench/wasm-candidates/r049-stack-translation-carry-rejected.wasm`;
  all candidate DBT/runtime/harness code is removed. The expanded QEMU
  diagnostic remains, 53 DBT, 30 core, and one Wasm units pass, and production
  Wasm again matches accepted `d93345139c5a...` byte-for-byte.

## 2026-08-08: stable Compile cost is dominated by residual interpretation, not translation

- `tests/vs-v86/analyze-engine-profile.mjs` now reconstructs the immediate
  subtree below `run_system_jit`. Unlike leaf-only categorization, this keeps a
  runtime helper invoked by generated code with that generated module. The raw
  R045 PRIME and STEADY profiles remain unchanged and reproducible.
- In STEADY, 1,608.50 of 1,680.77 sampled ms (95.70%) was below the scheduler.
  Generated module subtrees used 737.18 ms (43.86% of all samples), the
  policy-sampled interpreter 323.00 (19.22%), the final-outcome interpreter
  251.76 (14.98%), scheduler self plus direct cache hashes 199.33 (11.86%), and
  translation/issue 91.76 (5.46%). PRIME reproduced the hierarchy at 40.99%,
  22.41%, 13.96%, 12.46%, and 6.05% respectively.
- Exact R045 counters normalize these samples. STEADY retired 300,882,267
  generated and 24,901,348 interpreted instructions: 2.45 sampled ns per
  generated instruction versus 23.08 per interpreted instruction, a 9.42x
  ratio. PRIME was 2.46 versus 24.61 ns, or 10.00x. Scheduler self/hash cost
  was 361-382 sampled ns per dispatch. Translation/issue was 6.39-7.65 ms per
  attempt, but occupied only 5-6% of total samples.
- Therefore the earlier 52.54% runtime-Wasm label did not mean one slow runtime
  function. The remaining 7.64% interpreter retirement accounts for 34.20% of
  STEADY samples and is the only single component with comfortable whole-row
  leverage. Scheduler/cache work is a second 11.86% ceiling. Translation alone
  cannot pass the 10% gate, consistent with earlier registration and module
  geometry closures.
- A fresh proof-only profile on exact accepted Wasm `d93345139c5a...` is
  `target/bench/r050-runtime-attribution/scorecard-v2-2026-08-08T19-02-49-231Z.json`.
  It preserves the modern inputs and Compile MD5. The largest starting opcode
  accounts for 15.33% of residual interpreted instructions and the top 30 for
  49.42%, but opcode alone does not distinguish a generated side exit, missing
  page-policy entry, final rejected entry, or cold successor. Exact PC and
  policy-state attribution was used only as a diagnostic cross-check before
  selecting an implementation.

## 2026-08-08: exact fallback-site attribution repeats a closed result

- The transient R051 table partitioned residual fallback stretches by exact
  starting PC, instruction, generated-entry state, and page-policy state. The
  proof-only report is
  `target/bench/r051-fallback-sites/scorecard-v2-2026-08-08T19-11-00-074Z.json`;
  it retained the exact modern inputs and Compile output MD5.
- The dominant STEADY site was ASLR-relative PC `0x5555610ca990`, instruction
  `0x0047171b`: 12,465 stretches and 3,827,535 interpreted instructions. Its
  page was already compiled and superblocked, the entry had been requested and
  attempted, but it was not installed. The address is evidence only and never
  entered an execution selector.
- This is not a new cause. R032 had already found the same general entry state
  at essentially the same scale: 3.807M instructions over 12,395 calls, beyond
  the 512-leader cap. R033's general interpreted-work ranking reduced
  attempted-not-installed work from 3.896M to 0.114M and total interpretation
  from 24.847M to 21.229M. R034 then measured Compile 1,103.10 to 1,141.56 ms
  and Boot 2,673.09 to 2,711.78 ms, rejecting the mechanism on wall time.
- R051 therefore stopped before another candidate or timing sweep. Every
  transient fallback table, counter, export, and worker switch was removed;
  production Wasm again matches `d93345139c5a...` byte-for-byte. The correction
  is to audit R050's component map against R001-R049 closures before proposing
  more implementation, not to repeat entry-ranking or threshold experiments.

## 2026-08-08: a one-load exact Sv39 proof is slower than the split rows

- R052 followed the closure audit's remaining generated-memory question with a
  narrower representation than R038: load and store kept independent direct-map
  rows, while each row packed the complete canonical Sv39 VPN/context proof and
  exact memory32 translation offset into one `i64`. It removed one table load
  without sharing permissions or creating the proof collisions that invalidated
  R038's execution shape.
- Correctness gates caught two invalid assumptions before performance was
  considered. Allocating another 64 KiB bank overflowed Wasm machine
  construction, so the candidate overlaid the existing tag row. Guest RAM's
  Rust allocation is not page aligned, so storing an aligned linear page was
  wrong; the final form stored the exact wrapping memory32 offset. Bare/alias
  and Sv39+MPRV differentials then matched 262,160 and 262,176 retired
  instructions exactly, with one expected refill in the translated case.
- `target/bench/r052-packed-sv39-corpus.json` freezes the exact generated bytes
  and runs seven alternating paired fresh Node 26.5/V8 14.6 processes. The
  ordinary split probe measured 1,414.96 Minsn/s without the page cache and
  4,923.56 Minsn/s with it. Packed measured 797.82 and 4,691.58 Minsn/s. The
  paired speedups were 0.561x (95% median interval 0.551-0.584) and 0.885x
  (0.853-1.096), respectively.
- V8 native-code inspection explains the negative result. Exactness requires
  reconstructing and validating the canonical VPN plus permission context at
  every access; those integer operations and branches cost more than the
  removed scalar offset load. Fewer Wasm memory loads is not a useful proxy
  when the compressed proof must be rebuilt dynamically.
- The candidate therefore failed before a guest scorecard was warranted. It
  was removed without bit-layout, threshold, or workload tuning. The rebuilt
  production CODE section is byte-identical to accepted `d93345139c5a...` and
  the complete packed path is absent; recompilation changed only non-executable
  Rust/LLVM name metadata. R037, R038, and R052 jointly close interleaved SIMD,
  shared scalar, and separate scalar packing of the current proof.

The remaining Compile lane cannot be another representation of the same
per-access translation check. A future generated-memory proposal must remove
that proof architecturally (for example through an exact sparse/mirrored
translation mechanism) and first demonstrate at least 10% whole-row leverage
on frozen, address-independent work. Otherwise the honest result is a plateau,
not another implementation sweep.

## 2026-08-08: exhaustive pair handlers lose to single-operation dispatch

- R053 followed the independent fixed-width bounds left by R047 without using
  its failed selected-triple library. Exact Boot, Compile, and Python traces
  show that pairing could remove 42.79%, 43.77%, and 46.30% of all dispatches,
  respectively. This admitted an engine-shape screen, not production code.
- The deterministic emitter constructs all 3,844 ordered pairs of the 62
  normalized operation kinds. Its balanced selector visits every pair, so no
  workload population, PC, binary, symbol, or engine is encoded. The single
  and pair modules regenerate byte-identically, validate, and match state at
  five operation counts.
- `target/bench/r053-pair-dispatch-corpus.json` contains seven alternating
  paired fresh Node 26.5/V8 14.6 processes. Single dispatch reached 116.367
  million operations/s; exhaustive pairs reached 102.191 million. The paired
  ratio is 0.879x with a 0.878-0.882 bootstrap median interval, a 12.1%
  regression. Pair cold compile plus instantiate was only 1.068 ms, 0.847 ms
  above control.
- Halving calls is therefore not sufficient when it expands the dynamic
  indirect target space from 62 to 3,844 functions. The production prototype
  was not admitted. Pair-library size, popularity, and workload-weighted
  replay are intentionally not explored because they return to R047's failed
  selection family. R047 and R053 jointly close precompiled exact pair/triple
  handler libraries.

## 2026-08-08: the interpreter can safely consume its fused memory capability

- R054 began from whole-row attribution, not a benchmark address or opcode
  population. Accepted Boot profiles placed 16.647% of all CPU samples in
  scalar `Cpu::ld`/`Cpu::st`. To create at least 10% whole-row opportunity,
  that local path needed at least 2.51x throughput.
- The ordinary interpreter memory path already publishes an exact fused
  JIT-TLB row after resolving virtual translation, permissions, context, and
  physical RAM backing. The row's live native pointer remains a valid
  capability until the same invalidation events clear it. On an exact hit,
  consuming that capability in T0 removes the second standard-TLB lookup,
  physical-bus RAM classification, and redundant row publication; a miss uses
  the unchanged authoritative path.
- The frozen architecture-wide corpus report
  `target/bench/r054-interpreter-fused-memory-corpus.json` contains seven
  alternating fresh Node 26.5/V8 14.6 pairs. Exact state and memory match at
  all tested widths and alignments. Fused/control throughput is 3.030x with a
  paired bootstrap interval of `[3.024,3.041]`, clearing the preregistered
  2.51x local gate; cold construction differs by only -0.017 ms.
- Five alternating fresh-process same-Wasm Linux pairs in
  `target/bench/r054-interpreter-fused-memory-same-wasm-ab/` measured Boot
  1.161x `[1.113,1.181]` and Compile 1.022x `[0.968,1.058]`. Exact accepted
  versus final-default artifact A/B in `target/bench/r054-final-artifact-ab/`
  measured Boot 1.151x `[1.130,1.168]` and Compile 1.070x
  `[0.993,1.079]`. Both comparisons used exact modern guest/output hashes and
  stable host probes.
- The first complete 117-leg three-way run was retained as invalid because one
  legacy String Sort sample made that engine/row spread 1.28x, beyond the fixed
  1.25x gate. No leg was replaced. The entire matrix was rerun untouched;
  `target/bench/r054-final-three-way-rerun/scorecard-v2-2026-08-08T23-01-30-777Z.json`
  is valid, authoritative, and reports no problems. Relative to the prior
  accepted baseline, Boot falls 2,608.9 to 2,260.5 ms (13.35%) and Compile
  1,113.6 to 1,060.9 ms (4.73%); no other rewrite row regresses 10%.
- Five alternating fresh Chrome 150/V8 15.0 browser pairs also pass the
  `/shared/bench.py` guard. The report
  `target/jit-policy-traces/wanix-r054-416033-chrome-20260808/analysis.json`
  gives paired geometric rewrite/v86 elapsed-time ratios 0.841
  `[0.778,0.885]` for Python, 0.619 `[0.608,0.631]` for SHA-256, and 0.608
  `[0.499,0.742]` for shared 9P.
- Correctness covers 1/2/4/8-byte and unaligned scalar accesses, privilege-tag
  mismatch, bare/Sv39/MPRV operation, page-cross and MMIO fallback, and store
  invalidation of generated-code pages. The full workspace, randomized
  full-state/atomic/T2, generated-module validation, and direct/OpenSBI modern
  Linux gates pass.

R054 is therefore a causal architecture-level improvement rather than a
scorecard-specific selector. Artifact
`4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d`
is the new accepted baseline. It advances but does not complete parity: Boot
and Compile remain about 1.48x slower than copy/v86, so their residual must be
profiled again after this material T0 cost change.

## 2026-08-08: isolated fused instruction fetch does not transfer to Linux

- R055 refreshed the exact post-R054 profiles before editing production. Boot
  remained 93.59% runtime Wasm and 78.88% complete policy-interpreter
  subtrees; `Cpu::step` self time was 49.89% of the row. Compile showed nearly
  equal absolute generated-module time for rewrite and v86, leaving most of
  their difference in rewrite runtime/interpreter/scheduler work.
- An independent R041 trace counted 159.25M interpreter halfword fetches in
  Boot versus 40.14M scalar data operations, so an architecture-wide direct
  fetch capability had a plausible whole-row ceiling. The frozen mixed-RVC
  corpus validated that local shape: exact deterministic modules measured
  1.989x fused/control throughput with interval `[1.984,1.993]` and negligible
  cold construction cost.
- The complete default-off production prototype passed directed page-boundary,
  permission-context, fence, memory, Sv39/MPRV, Wasm, and both modern Linux
  boot gates. It then failed the preregistered same-Wasm Linux screen. Five
  alternating fresh pairs measured Boot at only 0.962x paired speedup
  `[0.935,1.152]`; Compile was a tie at 1.008x `[0.982,1.053]`. Inputs,
  Compile MD5, host probes, and harness validity all passed.
- The gap between corpus and product is itself the result: removing a modeled
  fetch proof is insufficient when its extra hot branch, real context churn,
  capability refill/invalidation, and whole-function code layout are included.
  The production gate supersedes the local throughput model; no cache-size,
  refill, address, opcode, or workload sweep is justified.
- Every production and harness switch was removed. The release rebuild is
  exactly the accepted R054 Wasm SHA `4160333352b18b...` and 32 core tests
  pass. Only the emitter, corpus, protocol, and raw invalidating A/B remain.

R020 already showed that caching only virtual-to-physical fetch translation did
not improve Boot; R055 now rejects the stronger direct live-pointer form. Do
not reopen one-page instruction-fetch caching in this scalar `Cpu::step`
shape. The parity goal remains open, but this candidate was bounded, tested,
and closed rather than tuned until favorable.

## 2026-08-08: exact callback monomorphization is real but too small

- Accepted disassembly confirms that `Cpu::run_until` invokes the exact
  generated-entry predicate through Wasm `call_indirect` after every
  interpreted instruction. Its 298.626 ms of self time is 13.101% of the fresh
  post-R054 Boot profile, so a 3.27x local improvement is required for a 10%
  whole-row gain.
- R056 isolated only that boundary. Both tiny frozen modules retain the same
  direct step call, exported/mutable dispatch tags and table, exact tag check,
  loop, and state result. Miss results at six iteration counts and an externally
  installed exact hit match; regenerated bytes are deterministic and valid.
- Seven alternating fresh Node/V8 pairs measured 753.976 Mprobe/s through
  `call_indirect` and 1,126.763 Mprobe/s inline. The paired speedup is 1.494x
  with interval `[1.492,1.500]`; cold construction is negligible.
- The mechanism is causal but cannot meet the whole-row gate. Even assigning
  the full `run_until` self category to it projects only a 1.045x Boot speedup.
  Production therefore remained byte-identical to accepted R054; no source
  prototype or product timing was run.

This is a useful closure rather than a candidate. Do not combine its 4.5%
optimistic projection with unrelated unimplemented savings to evade the fixed
advancement gate. A viable successor must remove a broader execution category.

## 2026-08-08: compiler Workers do not isolate foreground Wasm

- R057 first fixed a diagnostic-only scorecard defect: workload rows installed
  the module-capture callback, but the separate Boot path ignored it and still
  labeled the run measurement-eligible. Boot capture now uses the same exact
  downstream hook and makes the run proof-only. Production execution is
  unchanged.
- Accepted R054 then captured the exact current streams. Timed Boot tickets
  1-10 comprise ten unique modules and 3,974,380 bytes. Compile STEADY
  comprises fifteen modules and 5,745,513 bytes, including one 2,946,434-byte
  module. Their manifests and every module hash are frozen in the protocol.
- The preregistered corpus compared the current two concurrent
  `WebAssembly.compile` promises with two compiler Workers. Each Worker
  synchronously constructed the same module from a transferred buffer and
  structured-cloned the completed `WebAssembly.Module` back. A warmed,
  deterministic Wasm kernel supplied identical fixed foreground work. Seven
  paired fresh Node 26.5/V8 14.6 processes used CPUs 8-15; all module
  descriptors, counts, bytes, and foreground checksums matched, and host-probe
  spread was 1.071x.
- Boot foreground call time was 0.998x control/Worker
  `[0.997,1.008]`; foreground wall time was 1.002x `[1.001,1.013]`.
  The Worker route therefore supplied no isolation benefit. Worse, the full
  ten-module stream became ready at only 0.489x control speed
  `[0.465,0.632]`. Worker-local builds summed roughly 14.5 ms versus 25.7 ms
  for current-realm promises, but end-to-end Worker latency rose to about
  50.1 ms after transfer, message scheduling, and module cloning.
- The large Compile stream was the one favorable subcase: ready-time ratio
  1.073x `[1.071,1.119]`, foreground call/wall 1.005x/1.008x. It still did not
  move foreground execution, and selecting the Worker only for large modules
  would be an unpreregistered size policy adjacent to R048's rejected
  oversized-module tuning.
- Four of fourteen warmup sequences exceeded the frozen 1.25x tier-stability
  gate. Those legs remain recorded. Even ignoring that independent failure,
  Boot misses the 1.10x foreground gate and breaches the publication-latency
  guard by about twofold, so no repeat or product prototype is justified.

This closes a separate compiler Worker as a latency/performance mechanism for
the accepted policy on this engine. It may still be an embedder/UI isolation
choice, but must not be claimed as emulator throughput. The useful new
observation is that frozen Liftoff work itself completes in tens of
milliseconds when the event loop yields frequently; the much larger live
`jitCompileMs` sums include promise availability across long guest slices, not
hundreds of milliseconds of unavoidable compiler CPU. Improving publication
cadence therefore requires changing safe scheduler/yield structure together
with a broader execution benefit, not relocating the same callbacks.

## 2026-08-08: compact scalar step outcomes lose across a non-inlined Wasm call

- Accepted `Cpu::step` returns a 24-byte
  `Result<Option<StopReason>, Exception>` through linear memory. The normal
  callee path writes `Ok` and `None`; `run_until` reloads both immediately.
  Combining all `Cpu::step` and `run_until` self time gives a deliberately
  generous 62.9875% Boot ceiling and requires 1.1687x local speed for a 1.10x
  whole-row opportunity. R058 froze a 1.20x/1.15-lower-bound admission gate.
- Deterministic sret and compact modules execute identical state work and
  preserve a separate direct call. Six normal counts, stop, exception payload,
  complete memory, and all timed results match. Static shape contains 640
  identical nops and one driver call; V8's optimizing trace reports 777/751
  wire-byte step bodies and refuses inlining for lack of budget.
- Seven alternating fresh Node/V8 pairs measured 370.972 Mstep/s for sret and
  177.587 Mstep/s for compact. Paired compact/sret was 0.477x
  `[0.469,0.650]`, despite stable host probes at 1.015x spread and negligible
  construction cost. Warm stability independently failed because one compact
  process ran much faster; it remains in the report and does not move the
  median above parity.
- Follow-up diagnostics were explanatory only. Synchronous tier-up, explicit
  Liftoff, and forced top tier retained the reversal. Ten fresh inlining-traced
  compact processes all preserved the call and clustered at 94.4-96.3 ms.
  Native code is smaller for compact, so instruction count alone does not
  explain V8's call-ABI behavior. A scalar return introduces a true outcome
  dependency across the call while sret communicates through store/load
  forwarding; that is a plausible interpretation, not a portable engine
  contract.

The frozen gate supersedes the appealing source-level store/load count. Do not
refactor the giant decoder, introduce exception sidecars, force its inlining,
or change warmup to select the lone favorable engine state. Production was
never modified. R023 and R058 now close both giant decoder inlining and a
compact non-inlined return ABI as standalone ways to remove the step/run-loop
boundary.

## 2026-08-08: flat RV64C dispatch is locally strong but fails its stability gate

- Accepted bytes retain one quadrant and three funct3 `br_table`s for the
  complete compressed decoder. Post-R054 `Cpu::step` self attribution and the
  independent exact 61.849% compressed share give a generous 30.854% whole-
  Boot ceiling. That category requires 1.418x local speed for a 1.10x row gain;
  R059 froze a 1.45x/1.40-lower-bound gate.
- Architecture-balanced nested and flat modules cover all 24 combined
  quadrant/funct3 families with identical minimal handler work. Immutable and
  externally mutated selectors, complete memory, and all checksums match.
  Static bytes contain exactly four versus one jump tables and no helper call;
  both functions reach TurboFan after the same fixed-yield prewarm.
- Seven alternating fresh Node/V8 pairs measured 603.125 versus 960.376
  Mdispatch/s. Paired flat/nested throughput was 1.592x `[1.592,1.617]`, with
  1.012x host spread and negligible cold delta. The local mechanism therefore
  is real, not a source-level instruction-count guess.
- Admission still fails. One flat process's measured warm calls were 5.678,
  4.369, 4.368, and 4.366 ms, a 1.301x spread beyond the frozen 1.25x gate.
  Its later steady samples were stable and all paired ratios favored flat, but
  the observation is neither removed nor replaced.
- Applying the microkernel's full 1.592x ratio to the entire 30.854% upper
  bound projects only 1.130x Boot. Real handlers retain nearly all decoder
  work, so the product ceiling is narrower than that deliberately favorable
  calculation. A marginal product attempt does not justify overriding an
  independent failed gate.

Retain the positive local evidence and negative admission decision together.
Do not tune yields, use Boot-weighted family frequencies, or implement only
popular combinations. A future broad interpreter mechanism must remove more
than one nested dispatch operation and establish stable leverage without
reopening R025-R027/R045 decoded-handler forms.

## 2026-08-08: exact v86 runtime comparison narrows the remaining mechanism

- The scorecard's comparator source is the checkout at
  `/home/darren/src/arm64.js/target/bench/v86`, exact commit
  `2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f`. The source validator records
  `ca8afd71c1444a56c20b1ab63939569329fd5369a1a75760b5dce53fc3ba00f8`;
  the executed `build/v86.wasm` is
  `4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1`.
  The checkout contains local runner/manifest diagnostics, so the source hash
  and Wasm hash—not worktree cleanliness—are the comparator identity.
- Scorecard preflight proves generated v86 functions are installed and
  execute. v86 compilation is asynchronous: Rust emits one structured module,
  JavaScript calls `WebAssembly.instantiate`, and the resolved function is
  placed in a shared table. The non-generated path stays inside the main Wasm
  instance, directly looks up the translated code page, and interprets a
  straight-line/same-page stretch before charging hotness once. It does not
  synchronously compile every first-seen PC.
- The rewrite also interprets stretches rather than returning to JavaScript
  after every instruction, but each instruction crosses the generic
  `Cpu::step` decoder/result boundary. In accepted R054 Compile STEADY, the
  absolute generated subtrees are already close (about 740 sampled ms rewrite
  versus 675 ms v86); the large difference is runtime/fallback work (about 782
  versus 195 ms). This rejects the idea that generated-Wasm quality alone is
  the current parity blocker.
- The independent R041 opcode census gives a broad but only theoretical T0
  opportunity. Excluding floating-point load/store, atomics, fences, and
  system opcodes, ordinary scalar integer/control/compressed families account
  for 114,121,267 of 115,272,067 classified Boot instructions (99.002%) and
  81,803,312 of 81,975,865 Compile instructions (99.790%). These percentages
  establish address-independent coverage; they do not establish speed or run
  length.

No production candidate follows from coverage alone. A distinct admissible
mechanism would have to keep execution inside one compact precompiled loop and
perform several common RV64 operations directly before falling back to the
complete decoder. Merely flattening dispatch, forcing the current decoder into
its caller, caching decoded handlers, or generating one function per sequence
would reopen R023/R025-R027/R045/R053/R059 and is forbidden. The next gate is
a frozen architecture-balanced engine-shape corpus with a whole-row projection
of at least 10%; until it passes, R054 remains production and this is a bounded
investigation rather than an optimization claim.

## 2026-08-08: whole-runtime post-link optimization does not improve the failing rows

- R060 froze exact accepted R054 as input and the four standard Binaryen 125
  speed levels (`-O1` through `-O4`) as the complete search space. Every output
  regenerated deterministically, validated, preserved the 13-import/170-export
  ABI, and exactly matched 230,000,332 JIT-off/JIT-on user instructions plus
  output SHA `ea1a7f7c2489...`.
- `-O3` cut the runtime from 4,272,517 to 2,134,850 bytes but its valid
  three-pair product result was Boot 0.940x `[0.937,0.956]` and Compile STEADY
  0.976x `[0.975,1.024]`. Smaller runtime Wasm is not faster runtime Wasm on
  these rows.
- The remaining frozen levels did not reveal a transferable optimum. Paired
  Boot/Compile ratios were `-O1` 1.007x/0.983x, `-O2` 0.976x/0.974x, and `-O4`
  0.975x/1.049x. Exact reports are under
  `target/bench/r060-wasm-opt-{o1,o2,o4}-ab` and
  `target/bench/r060-wasm-opt-ab`; all host spreads were at most 1.019x.

No level reaches the fixed 10% gate without losing another row. R060 is closed
without searching Binaryen pass combinations, and the build pipeline remains
unchanged.

## 2026-08-08: matching v86's SIMD target feature regresses Compile

- The exact v86 build enables `+simd128`, while accepted rewrite R054 does not.
  R061 therefore rebuilt unchanged source with only that target feature plus
  the existing table linker flags. Candidate `5cbdfaa3de7f...` contains 18,565
  actual SIMD operations, preserves the complete ABI, and matches exact
  JIT-off/JIT-on user retirement and output.
- Three alternating fresh-process pairs are retained at
  `target/bench/r061-simd-ab/config-ab-2026-08-09T01-34-53-141Z.json`.
  Boot tied at 0.985x `[0.968,1.041]`; Compile regressed to 0.937x
  `[0.895,1.038]`. Host spread was 1.021x and all guest fingerprints passed.

The comparator's build feature is not itself the explanation for its runtime
advantage. Do not tune individual vectorized functions or combine this flag
with scorecard-selected Binaryen passes.

## 2026-08-08: loop-carried code-page backing is slower than exact per-step fetch

- R062 implemented the structural difference observed in v86: resolve one
  execute-proved physical/direct code-page backing and carry it in local state
  across `Cpu::run`/`run_until`, discarding it on page, mapping generation,
  privilege context, interrupt, or exception changes. It had no cache size,
  address, opcode, workload, or refill selector and was distinct from R020/R055's
  persistent per-step capability probe.
- The default-off prototype passed 32 core tests, an explicit compressed plus
  `0xffe` 32-bit page-split/refill differential, the complete Wasm smoke suite,
  and both modern direct-SBI and OpenSBI generated-execution boots.
- The immutable same-Wasm gross gate is
  `target/bench/r062-carried-fetch-gross-ab/config-ab-2026-08-09T01-47-00-747Z.json`.
  Two alternating pairs measured Boot at 0.886x `[0.876,0.895]` and Compile at
  0.981x `[0.977,0.984]`, with exact outputs and 1.014x host spread. The 12.9%
  Boot regression is decisive.

Every core, runtime, export, scorecard, and modern-Linux switch was removed.
The rebuilt production module is byte-identical to accepted R054 SHA
`4160333352b18b...`, and all 32 core tests pass. Together with R020/R055, this
now closes translation caching, direct persistent capability caching, and
loop-local physical/direct backing in their tested scalar-interpreter forms.

## 2026-08-09: event-loop cadence exposes publication delay but does not fix Boot

- The exact comparator audit found a scheduler mismatch in the scorecard.
  copy/v86 calls one `cpu.main_loop()` and schedules the next through
  `setImmediate`; the public rewrite runtime likewise schedules one
  2,000,000-instruction slice per host turn. The scorecard's manual rewrite
  driver yielded only on pump iterations 1, 5, 9, and so on, allowing an async
  module completion to wait through roughly eight million extra guest
  instructions.
- R063 added only a common diagnostic `yield after every pump` switch. It
  changed no Wasm or guest bytes and applied to every engine. The valid
  three-pair report is
  `target/bench/r063-yield-every-pump-screen-valid/config-ab-2026-08-09T02-01-24-637Z.json`.
  Boot tied at 1.008x `[0.946,1.016]`, Compile improved 1.060x
  `[1.058,1.076]`, and Python improved 1.246x `[1.156,1.254]`; host spread was
  1.019x and fingerprints matched.
- This is causal publication evidence: Python interpreter retirement fell
  from about 56.5-57.5M to 29.8-33.7M and landed regions rose from 26 to 62-65.
  It is not credited as a production JIT win. The preregistered gate required
  1.10x on Boot or Compile, which R063 missed; the accepted scorecard remains
  unchanged.

## 2026-08-09: returning only while a module is pending has insufficient leverage

- R064 narrowed R063 into a default-off product mechanism: after issuing an
  async page module, `run_system_jit` returned at the next quantum boundary
  while `PENDING_SB` was non-empty. Both legs used the same candidate Wasm and
  R063 cadence; only this switch differed.
- The valid three-pair report is
  `target/bench/r064-pending-publication-screen/config-ab-2026-08-09T02-08-11-937Z.json`.
  Boot regressed to 0.976x `[0.969,0.987]`; Compile improved 1.049x
  `[1.004,1.058]`; Python improved 1.054x `[1.037,1.149]`; host spread was
  1.038x and all outputs matched.
- The mechanism did exactly what it claimed: recorded async availability fell
  from 174-180 ms to about 8 ms in Compile and 2.36-2.47 s to 0.43-0.50 s in
  Python. The much smaller wall gains prove that publication latency is not a
  remaining 10% Boot/Compile lever in this runtime shape.

R064 is rejected and completely removed. The candidate Wasm
`dfc9026b165c...` remains archived for evidence; the rebuilt production Wasm
is again exact R054 `4160333352b18b...` at 4,272,517 bytes. Do not revisit
event-loop frequency or pending-build early returns absent a new scheduler
architecture with independently measured whole-row leverage.

## 2026-08-09: raw parity contains two different gaps

- R065 adds no scoring or product change. It decomposes the accepted Compile
  result using architectural counters already present in the scorecard. Rewrite
  executes 325.715M instructions at 307.007 MIPS; v86 executes 248.939M at
  346.484 MIPS. The raw 1.4767x time gap is 1.3084x more RV64 guest work times
  a 1.1286x emulator-normalized cost. Raw parity would require rewrite to reach
  453.344 MIPS—30.8% faster than v86—not merely match its per-instruction rate.
- A balanced three-pair Boot diagnostic added the previously missing read-only
  v86 instruction counter outside timing. Rewrite/v86 retired 180.816M/183.982M
  median instructions, essentially equal work volume, but reached only
  78.964/120.827 MIPS. Boot's 1.5036x raw gap is therefore a 1.5301x
  time-per-instruction deficit rather than RV64 compiler inflation.
- Reports and complete math are frozen in
  `docs/jit-rewrite/R065_RAW_GAP_DECOMPOSITION.md`. Raw wall time remains the
  user-visible verdict; normalized rates are an anti-reward-hacking diagnostic,
  not a replacement score.

The consequence is strategic. Compile needs a roughly 13% emulator-throughput
gain just to reach normalized parity and a 47.7% gain for raw parity because
the guest performs 30.8% more work. Boot instead needs a broad T0/runtime
redesign; policy, publication, compile latency, and isolated generated-code
micro-optimizations cannot explain its approximately 53% per-instruction gap.

## 2026-08-09: integrating the complete scalar T0 loop helps Boot, but not enough

- R066 tested the broad execution-loop redesign admitted by R065. The complete
  ordinary RV64I/M integer, control, scalar-memory, and integer-RVC families
  executed inside one Tier-0 driver while PC and retirement stayed in Wasm
  locals. State was flushed before interrupts, exceptions, slow families,
  stops, and host returns. F/D, A, FENCE, SYSTEM/CSR, and compressed FP retained
  the complete decoder. No PC, binary, workload, or opcode-frequency selector
  existed.
- The same-Wasm candidate `d2ac8852eaf1...` passed exhaustive actual-RVC-prefix
  and broad randomized/directed 32-bit decoder differentials, the complete
  Wasm smoke suite both off and on, and the full-system memory, Sv39/MPRV,
  atomic, FP, WFI, T2, modern direct/OpenSBI Linux, context-switch, and AMO
  gates when forced on.
- The valid three-pair report is
  `target/bench/r066-integrated-scalar-t0-screen/config-ab-2026-08-09T02-43-43-436Z.json`.
  Paired results were Boot 1.074x `[1.060,1.129]`, Compile 0.974x
  `[0.959,0.996]`, and Python 0.987x `[0.947,0.999]`, with a 1.022x host spread
  and exact inputs/outputs.

This is measurable architectural progress, but it fails the frozen 1.10x Boot
gate and slightly regresses both guards. It also bounds the value of merely
removing the per-instruction PC/minstret materialization and `Cpu::step` call
for 99% of Boot: that broad change recovers about 7%, not the roughly 35%
still needed for v86 parity. R066 is fully removed without sub-family tuning;
production is byte-exact R054 `4160333352b18b...`. A successor needs a more
fundamental execution representation or generated privileged tier, not
another scalar decoder layout or local driver variation.

## 2026-08-09: R066 was fully integrated; generated coverage is the remaining broad lever

- R067 statically inspected archived R066 rather than inferring its shape from
  source intent. The VirtBus `Cpu::run`/`run_until` bodies are 6,138/6,331
  bytes versus R054's 272/381, carry about 28/30 locals, contain 22 branch
  tables, and have no separate scalar-T0 helper. The three remaining static
  `Cpu::step` calls are confined to the disabled control and slow-family paths.
  R066 therefore validly bounds integrated scalar interpretation.
- Phase-isolated profiles are frozen under
  `target/bench/r067-post-r066-residual/`. R066 treatment spends 93.30% of
  sampled time in runtime Wasm and only 4.82% in generated Wasm. Its integrated
  `run`/`run_until` bodies own 62.19% of all samples; `ld` plus `st` own only
  8.78%. The pinned copy/v86 control spends 69.91% in runtime Wasm and 21.59%
  in generated Wasm.
- Exact treatment counters leave 111.363M Boot instructions interpreted and
  69.419M generated. Using the independently measured approximate 62/661 MIPS
  rates, transferring 40M instructions has an optimistic gross value around
  585 ms, enough to clear the fixed 10% opportunity floor. That projection is
  an admission bound, not a performance claim.

Further scalar-loop, helper-boundary, fetch-cache, and load/store-only tuning is
closed. The only admitted next diagnostic is an offline cold privileged
generated tier that batches unrelated hot pages into far fewer modules. It must
show architecture-general post-threshold coverage, bounded functions, and
actual host compile geometry before any runtime change. This is distinct from
R046's failed 51-52 one-page compile storm; failure to amortize that work closes
the mechanism without runtime tuning.

## 2026-08-09: eight-page memory-state packaging clears the cold-tier opportunity gate

- R068 traced the exact modern direct Boot with generated execution fully
  bypassed and emitted candidate modules only after readiness. The corrected
  report is `target/bench/r068-batched-privileged-t0/opportunity.json`
  (`7decd1563eb1...`): 183.000M observed instructions, no trace overflow or
  out-of-RAM events, one conflicted physical page excluded, and 82 stable
  privileged pages selected solely by the frozen 200,000 threshold.
- The primary creates 11 eight-page memory-state modules. They cover 99.047%
  of exact observed entry events and 73.622M incremental instructions after
  batch formation and compile delay, capped at R054's existing 4,194,304
  privileged threshold. Total bytes are 5.084 MB; the largest module/function
  is 615,820/15,825 bytes. All modules validate and compile in fresh V8
  processes, with 9.900 ms summed and 1.125 ms maximum compile time after the
  guest stopped.
- Controls establish the packaging boundary rather than supplying post-result
  alternatives. Four-page batches require 21 jobs. Sixteen-page batches make a
  1,222,786-byte module. Register-structured eight-page batches make an
  834,421-byte single function and 7.231 MB total, recreating R048's risk.
- The first completed readiness calculation was invalid: it credited execution
  after the normal production tier would already compile a page and claimed an
  impossible 139.600M new instructions against 111.363M remaining interpreted.
  It was rejected before decision. The sole accepted calculation caps every
  page at the existing production threshold and therefore only reduces the
  opportunity claim.

All frozen R068 gates pass. Admit exactly one default-off runtime candidate:
privileged-only FIFO formation at 200,000 instructions, eight pages per module,
64 leaders per page, memory-state emission, existing in-flight concurrency,
and all current mapping/dirty-page proofs. This does not promote the mechanism;
the paired runtime product gate remains authoritative.

## 2026-08-09: live eight-page batching fails before the product timing gate

- R069 implemented exactly the admitted FIFO 200,000-instruction, eight-page,
  64-leader-per-page, memory-state geometry behind a same-Wasm default-off
  switch. A synthetic gate proved issue, landing, all-page mapping validation,
  and generated retirement; core/DBT/Wasm/page-policy gates passed.
- Live modern Linux overturned the offline model. The switch-off control booted
  in 2.30 seconds; the first treatment boot took 18.04 seconds, issued 13
  batches/104 pages, accumulated 3.68 seconds of host compilation, and retired
  only 0.297M instructions after verified batch entry.
- The frozen report is
  `target/bench/r069-runtime-privileged-batch/ab/config-ab-2026-08-09T03-53-10-212Z.json`.
  It is intentionally invalid: eight of nine treatment legs failed the
  unchanged 30-second settle requirement, while every control leg completed
  and host spread was 1.031x. The sole complete pair measured 17.260 versus
  2.279 seconds (0.132x); its treatment spent 4.286 seconds compiling 8.821 MB
  across 18 total modules but verified only 0.258M batch-retired instructions.

Offline post-trace entry coverage plus isolated `WebAssembly.compile` latency
is not a valid availability model for runtime-generated Wasm. It can use
future entry knowledge and omits compiler/guest contention, publication, and
settle behavior. Together R046 and R069 close early privileged runtime Wasm
generation in the current architecture, from one-page compile storms through
eight-page batching. R069 was removed without tuning; production is exact R054
`4160333352b18b...`.

## 2026-08-09: residual-only static T0 is real but smaller than a retainable independent gain

- R070's hand-shaped decoder is locally broad and fast, but the system
  integration invokes it only after an exact page-policy entry has reached a
  final attempted/installed outcome. In five fresh Boot treatments it retired
  only 4.878M-7.873M fast instructions; the sampled page-policy path still
  owns most of the 105M+ residual instructions.
- R071 froze a reusable cumulative-gain policy before collecting new evidence:
  five pairs, a 3% primary-row median with a non-regressing lower bound, 3%
  native/browser guard medians, complete correctness, and a tighter 5% full
  scorecard guard. This is a prospective policy change, not a relabeling of
  earlier 10% decisions.
- The valid report
  `target/bench/r071-static-t0-independent-confirmation/config-ab-2026-08-09T05-43-19-283Z.json`
  measures Boot 1.024x `[1.000,1.050]`, Compile 0.998x `[0.962,1.020]`, and
  Python 1.032x `[0.983,1.043]`, with exact inputs/outputs, zero static-T0
  errors, and 1.062x host spread.
- Boot's unpaired side medians imply 1.035x, but the frozen primary statistic
  is the order-balanced paired median. Substituting the favorable statistic or
  pooling R070's earlier three pairs after seeing R071 would be reward hacking.
  The machine verifier rejects Gate A, so no browser or full scorecard is run.

The next hypothesis must move page-policy sampling itself into the static
execution shape while producing the exact same `(pc, pa, satp, mode, retired,
control_entry)` observation stream. Merely enabling R070 for more PCs without
those observations would change compilation policy and invalidate both the
causal comparison and generated-code correctness evidence.

## 2026-08-09: sampled execution transfers strongly to Boot, but short shared 9P is a weak hard gate

- R072 reproduced the exact page-policy observation stream while running
  ordinary instructions in the static decoder. Direct/OpenSBI boots execute
  about 55M-60M sampled instructions in directed validation; scorecard Boot
  executes about 106M. No workload PC, page, opcode subset, or threshold is
  selected.
- Five fresh native pairs show a decisive 1.209x Boot speedup
  `[1.190,1.239]`, plus 1.021x Compile and 1.049x Python. This refutes the idea
  that mature-stage gains must all be below 10%; it does not justify demanding
  20% from every future mechanism.
- The first browser candidate/control set is permanently invalid. Its added
  1.25x within-side cap rejected shared 9P, but accepted pre-R072 R054 and R043
  shared samples already spread 1.612x and 1.284x. A fresh schema-2 sample was
  therefore frozen before collection and did not pool the first set.
- That independent sample passes Python and SHA-256 at 1.029x and 0.990x
  candidate/control, but shared 9P is 1.041x against a 1.03 gate. Its five
  ratios range from 0.829x to 1.268x and its exact median interval spans both
  large improvement and large regression. A subsecond-to-one-second external
  9P phase plus a five-value point-median threshold is too noisy to attribute
  a 4% causal change reliably.

The methodology weakness is a finding, not a post-hoc promotion argument.
R072 fails its frozen rule and stays default-off. Any future project-wide
browser-I/O rule must be selected before a candidate result, lengthen the fixed
workload or require evidence relative to an uncertainty bound, and first prove
repeatability on controls. It cannot relabel R072.

## 2026-08-09: sampled-only acceleration is compute-neutral but has a repeatable I/O lifecycle cost

- R073 removes R070 residual activation and retains almost all of R072's Boot
  leverage: five native pairs improve Boot 1.157x `[1.137,1.187]` while
  Compile and Python remain within their guards. This confirms exact sampled
  execution itself, rather than residual selection, supplies the useful cold
  gain.
- The strengthened browser design resolves R072's decision uncertainty with
  seven fresh pairs and a median of three phase-synchronized observations per
  leg. Python is 0.994x `[0.979,0.997]` and SHA-256 0.999x
  `[0.984,1.003]`: static sampling does not impose a general warmed-compute
  penalty under Chrome/V8.
- Shared 9P is 1.058x `[0.932,1.114]`. The interval is still much wider than
  compute, but now misses both prospectively frozen 1.03 median and 1.10 upper
  limits. Six of seven per-leg ratios exceed 0.93 and five exceed 1.04; this is
  not rescued by selecting the favorable first pair.
- The raw within-leg shared-I/O variation remains large even after triplicate
  medians (control spread 1.410x, candidate 1.302x). Longer workload design
  would improve attribution in a future benchmark revision, but cannot change
  R073's valid outcome.

R073 proves a useful boundary for a successor: preserve the architecture-wide
sampled static decoder for dense cold execution, but avoid paying its
observation/transition cost during sparse device-driven stretches using only
guest-independent execution-state evidence. A shell, benchmark, PC, opcode,
device request, or phase selector would be reward hacking and is inadmissible.

## 2026-08-09: short-entry backoff works mechanically but does not break the page-JIT lifecycle feedback

- R074 used one opaque mapped-entry rule selected before implementation: the
  first sampled static stretch retiring fewer than the existing 64-instruction
  floor marks that `(virtual page, physical page, entry PC)`, and later samples
  use the accepted interpreter. Dirty-page and reset lifecycle, q1/q32/q1024,
  generated-entry, WFI, direct/OpenSBI Linux, 134 ISA, 109 Spike, and 193
  signature gates all pass. The first directed test caught and permanently
  regressed a tuple-order bug before timing.
- Five fresh native pairs pass the frozen product screen: Boot 1.154x
  `[1.145,1.177]`, Compile 0.979x `[0.948,1.054]`, and Python 0.999x
  `[0.981,1.061]`, with exact outputs/lifecycle and zero errors.
- All 14 formal Chrome legs complete in their fixed alternating order. The
  valid immutable report
  `target/jit-policy-traces/wanix-r074-28ceaf7b-chrome150-20260809-config-ab/analysis.json`
  (`d3cf3a102966...`) measures Python 1.010x `[0.995,1.021]`, SHA-256 1.002x
  `[0.986,1.010]`, and shared 9P 1.068x `[0.959,1.122]` candidate/control.
  Shared 9P fails both the 1.03 median and 1.10 upper limits.
- The mechanism is not inert. Relative to R073 shared-side medians it cuts
  sampled-static retirement 15.740M to 8.213M and sample calls 439,866 to
  29,656, with 391,360 later interpreter bypasses. Nevertheless the candidate
  still executes 49.937M versus 46.210M control instructions and issues 10
  versus 6 page modules. Host 9P service changes only 0.6%.

Per-entry decoder backoff therefore does not address the causal product
boundary. Faster sampled execution changes asynchronous page-module admission
and readiness enough to create additional guest/JIT work during short I/O.
Reject R074 without a key/64 threshold sweep. Before another product candidate,
separate raw static-decoder cost from module-admission feedback using a fixed
non-scoring diagnostic; any successor must make the lifecycle work-based and
architecture-general rather than select a device, phase, PC, or workload.

## 2026-08-09: preboot state eliminates the I/O penalty but Chrome does not accelerate Boot

- R075 changes only activation lifecycle. Control and candidate prepare the
  identical static decoder before `vm.start`; candidate uses R074's exact
  sampled/backoff rule from the first firmware instruction. The immutable
  pages differ by one adapter flag and share archive `e0c1971d1ecd...` with
  main Wasm `28ceaf7bcf63...`.
- Gate A passes the complete strict matrix plus adapter, public/Worker, and
  explicit candidate direct/OpenSBI tests. At the WANIX shell, candidate has
  roughly 64M sampled-static retirement and 93k median short-entry bypasses;
  control has zero static activity. Both report module index 822 and zero
  errors.
- The valid seven-pair, three-repetition Chrome report has protocol SHA
  `b50a9bf10b5d...` and analysis SHA `53d7233fb9dd...`. Shell speedup is 0.997x
  `[0.993,1.002]`, with control/candidate medians 31.190/31.301 seconds. The
  narrow interval excludes a material benefit.
- Shell-side guest work is nearly identical: 85.546M/85.392M instructions,
  23/22 page modules, and 72.2/57.2 ms host module compile time. Candidate host
  9P service is lower, 842 versus 1,173 ms. Neither extra guest work, module
  storms, browser compilation, nor host 9P explains away a hidden Boot win.
- All warmed guards pass. Candidate/control ratios are Python 1.014x
  `[0.990,1.034]`, SHA-256 0.996x `[0.990,0.998]`, and shared 9P 0.871x
  `[0.799,1.059]`. The earlier R074 shared-I/O loss was a cold-state/lifecycle
  interaction and is not intrinsic to the static decoder.

The remaining discrepancy is now specifically Node/native versus Chrome/V8
execution of the sampled static decoder and accepted Rust interpreter during
Boot, not the preboot page-JIT or 9P lifecycle. R075 is rejected because the
product browser receives no Boot benefit. Use boot-scoped engine profiles as
attribution only before admitting a new representation; do not tune against
WANIX PCs, firmware symbols, guest files, or phase identity.

## 2026-08-09: timer dilution explained R075, but isolated speedup did not transfer to production scorecard

- R076 moved the Chrome timer from WANIX launch-to-shell to the exact
  scorecard pump-to-ready boundary. Seven fresh paired processes measured a
  1.175x execution gain `[1.167,1.189]` and 1.174x normalized-MIPS gain with
  effectively equal guest work. R077's separately rebuilt production-default
  integration reproduced it at 1.163x `[1.136,1.191]`.
- This resolves the apparent R075 contradiction: sampled static T0 does speed
  the emulator's Chrome execution path, while WANIX host startup dilutes that
  gain. It also shows why a 5% cumulative rule is useful—both confirmations
  comfortably pass without requiring every mature change to reach 20%.
- The R076 candidate-v86 product guard is stronger than nonregression. Fresh
  paired RV64/v86 ratios are 0.891 Python, 0.632 SHA-256, and 0.669 shared 9P,
  so `/shared/bench.py` is not the obstacle to promotion.
- R077 nevertheless fails at the final system-level boundary. Its complete
  authoritative run is invalid only because old-legacy HUFFMAN has 1.425x
  spread; rewrite HUFFMAN is stable at 1.008x and the largest host probe is
  1.049x. Keeping the invalid run is necessary, but the unrelated legacy
  variance is not the decisive negative performance evidence.
- The decisive independent failure is raw Boot: three rewrite samples
  2,259.153/2,329.830/2,293.093 ms produce a 2,293.093 ms median, 1.44% slower
  than R054's 2,260.485 ms and far below the frozen 5% improvement. The same
  descriptive report still has 11/13 v86 matches and 13/13 legacy matches.

The boundary mismatch is now an experimental fact, not a reason to pick the
favorable Chrome result. R076/R077 prove the static decoder's local execution
value, but also show that its isolated A/B speedup does not survive the exact
Node/V8 authoritative default-on scorecard as a raw Boot improvement. Future
work should attribute the remaining Boot cost with the accepted default-off
runtime and use the 5%/confidence track for broad changes. Repeating the same
sampled decoder under a new timer, threshold, lifecycle, or browser wrapper is
closed unless an independently measured engine/runtime cause changes.

The rollback itself is verified rather than assumed. The final WANIX archive
uses unchanged main Wasm `28ceaf7bcf63...` with loader `f1d56b133c39...` and
leaves sampled/static T0 off absent explicit configuration. At shell and
through correct Python it reports module index -1 and exact zero static
activity while generated code retires 566.4M instructions. Thus rejection did
not disable the ordinary async page JIT or regress the runnable product.

## 2026-08-09: disabled experimental machinery was not performance-neutral

- R078 proves the post-R077 default-off artifact was not an acceptable
  control. Exact R054 is 1.178x faster on Boot `[1.116,1.190]`, 1.037x on
  Compile, and 1.031x on Python even though both sides report zero static-T0
  activity. A false runtime flag does not make code-layout changes, extra
  state refreshes, and hot-loop branches free to V8.
- R079 removed the obvious large emitter/runtime but remained 18.3% slower
  than R054 on Boot. Binary size and symbol-count similarity were insufficient
  causal evidence. The exact session history exposed smaller surviving
  R070/R072 changes in fetch synchronization and page-policy observation that
  source-name searches had missed.
- Removing that complete residue in R080 restores the performance envelope:
  five-pair R080/R054 speedups are Boot 1.033x, Compile 1.013x, and Python
  1.011x, with all confidence/identity/correctness gates satisfied. Its code
  differs from R054 by only the independently required 38-byte WFI-yield fix.
- The valid untouched scorecard remains 11/13 versus v86 and 13/13 versus
  legacy. The open deficits are sharply localized: Boot is 1.497x slower and
  Compile 1.474x slower. Python is already 1.099x faster than v86.
- A fresh browser product guard confirms the latter is not a harness illusion:
  RV64/v86 ratios are 0.875 `[0.863,0.888]` for Python, 0.608
  `[0.592,0.630]` for SHA-256, and 0.655 `[0.551,0.747]` for shared 9P, with
  exact artifacts and required generated retirement.

These results reinforce the cumulative-gain policy. Requiring 20% from each
mature optimization would discard useful general improvements; eight
independent 5% throughput gains compound to about 1.48x, approximately the
remaining raw gap. Three-to-five-percent changes must still clear fresh paired
confidence, correctness, non-target, browser, and full-scorecard gates. The
smaller threshold changes statistical rigor, not the architecture-general
rule: a PC, symbol, workload, output, browser, or compiler-binary selector is
still reward hacking and remains forbidden.

## 2026-08-09: clean-baseline profiling rules out a V8 tier-up failure

- R081 profiles exact R080 with fixed 250-microsecond sampling. Inspector
  timings are proof-only, but component attribution is decisive: runtime Wasm
  is 93.41% of Boot and `Cpu::step` is 50.28% of the entire phase. Generated
  Wasm is only 4.87%, matching exact 38.43% generated retirement.
- The full-system interpreter body is not stuck in Liftoff. V8 produces a
  17,280-byte TurboFan body for Wasm function 1433 after its 26,880-byte
  Liftoff body. Optimizing thresholds or forcing tier-up would therefore target
  a mechanism already working.
- Compile STEADY still gives 46.44% of samples to generated Wasm, but
  24.76M interpreted instructions plus policy/final/scheduler boundaries own
  most of the other half. At 92.40% generated retirement, residual execution
  is roughly an order of magnitude more expensive per guest instruction.
- R078 and R081 together separate the old static-tier result into two causal
  facts: auxiliary scalar execution can accelerate cold work, and linking its
  emitter/integration into the main Wasm can independently slow all execution.
  A successor may isolate the compiler/emitter in a separate artifact and
  retain only a minimal runtime ABI. It may not restore the archived code as a
  production path or revive its entry/threshold tuning.

## 2026-08-09: externalizing the emitter did not clear dormant non-inferiority

- R082 physically separates the 217,556-byte compiler and emits a valid
  11.3-KiB scalar module, leaving only 9,204 bytes of integration in the main
  runtime. This solves the old emitter-bloat mechanism and passes focused
  semantics, lifecycle, generated-handoff, and modern-Linux proofs.
- The frozen dormant-capable A/B has neutral point estimates, so it does not
  prove a deterministic slowdown. It does fail the prospectively required
  uncertainty bound: Boot's paired lower confidence limit is 0.967 versus the
  0.970 floor. A narrow miss is still a miss; rerunning until it passes would
  invalidate the experiment.
- This result closes the current external scalar-tier integration, not all
  small improvements. It reinforces why hot-path additions need artifact-level
  non-inferiority even when disabled and why general 3-5% gains remain useful
  only with fresh paired confidence and guard rows.
- Removal restores the exact R080 Wasm and loader hashes. The next independent
  candidate should reduce work already inside the dominant full-system
  `Cpu::step` path without adding a dormant subsystem, guest selector, or
  benchmark-specific policy.

## 2026-08-09: removing system-state tests worsens optimized interpreter shape

- R083 is a direct test of R081's remaining small-candidate hypothesis. It
  removes repeated `Option<SysCsrs>` tests from const-specialized
  fetch/load/store paths and produces a main Wasm 22,957 bytes smaller than
  R080 with unchanged section counts. Correctness and cold construction pass.
- Whole Linux behavior reverses the source-level expectation. Candidate Boot
  is 11.2% slower by paired median, with the complete interval below parity;
  Compile is 3.7% slower and Python 1.5% slower by paired medians. The result
  is stable enough that noise cannot explain the Boot loss.
- A smaller Wasm/code section and fewer explicit source branches are not
  optimized-native throughput proxies. Const specialization changes complete
  function identity, layout, tiering, and call shape; the product gate owns the
  decision even though no single native subcause is assigned.
- This closes full-system/user-only const specialization and adjacent
  helper-boundary variants in the current interpreter. It does not lower the
  3% cumulative rule. It means the next candidate needs a distinct broad
  dynamic operation, not another representation of the same system-state
  check.

## 2026-08-09: closure-aware integer hashing is a retained cumulative gain

- Immediate-child CPU-profile accounting understated Rust hashing because
  policy helpers sit between `Cpu::step` and the default hasher. Complete
  closure accounting assigns 6.586% of R080 Boot and 4.402% of Compile STEADY
  to hash/probe self time, enough for a general 3-5% candidate.
- A crypto-seeded mx3-style integer builder is 5.508x faster for raw hashing
  and 3.021x faster for the representative JIT state-map mix in the same Wasm
  environment. Randomized map-local seeds preserve collision unpredictability;
  lookups and insertions make no host calls.
- Whole-product evidence agrees across engines and boundaries. Native Compile
  improves 1.051x; direct Chrome Boot improves 1.023x with every pair
  favorable; WANIX Python improves 1.028x; the untouched scorecard improves
  Boot 1.041x, Compile 1.042x, and Python 1.017x versus exact R080.
- This candidate has no PC, symbol, workload, compiler-output, browser, or
  checksum selector and changes no generated code or tier policy. It is a
  concrete demonstration that a correctly gated 4-5% cumulative gain should
  be retained even when it does not immediately change the 11/13 parity count.
- R085 is promoted as Wasm `efd7830307ef...` and archive `0b953be67610...`.
  Boot and Compile remain the only v86 losses at 1.408x and 1.378x elapsed;
  the next profile must use exact R085 and treat default hashing as closed.

## 2026-08-09: public scheduler cadence materially changes the baseline

- The historical scorecard gave rewrite four 2M-instruction pumps per event-
  loop turn while the public RV64 scheduler and event-driven v86 yield after
  each slice. R087 makes one slice the ordinary scored default and records the
  cadence in every result; old behavior remains diagnostic-only.
- A fresh same-R085 causal check improves Compile 1.064x and Python 1.296x;
  Boot is 1.012x with an interval spanning parity. The complete corrected
  117-trial scorecard is valid and remains 13/13 legacy, 11/13 v86.
- The corrected gaps are smaller but still structural: Boot needs 28.16% less
  rewrite elapsed time and Compile needs 23.64%. Python is now 1.473x faster
  than v86. Harness gains are never credited as JIT product gains.

## 2026-08-09: exact re-entry self time does not convert to Boot wall time

- R088's corrected-cadence profile assigns 12.719% of complete Boot samples to
  exact generated-entry re-entry, clearing the prospective 9% gate. Combined
  with R056's 1.494x local indirect/direct lookup result, the honest Amdahl
  projection was a useful 1.0439x Boot candidate under the cumulative rule.
- R089 made only the callback types generic through the existing Rust/Wasm
  path. Full correctness passes, and disassembly proves `Cpu::run_until`
  changes from two `call_indirect` operations to zero while preserving direct
  calls and spelling both exact index/tag checks inline.
- Whole-product behavior contradicts the local projection. Five fresh pairs
  measure Boot 0.972x `[0.958,0.996]`, a clear regression, even while Compile
  improves 1.026x and Python is a paired tie. Artifact compile time is neutral.
- Sampled self-time attribution identifies where V8 charges samples; it does
  not prove that replacing a frame removes that fraction from optimized wall
  time. Here monomorphization changes larger function identity/layout enough
  to outweigh the local indirect-call saving on cold Boot. Close exact callback
  monomorphization and adjacent generic/inlining variants under this runtime.

## 2026-08-09: production feedback bookkeeping is dynamically dormant

- The scheduler source visibly contains two hash-table probes after an
  ordinary generated-block return, but source presence was misleading under
  production page policy. R090 counted the decision directly before changing
  its representation.
- Boot FIRST executed 575,382 generated outer dispatches; Compile
  FIRST/PRIME/STEADY executed 733,451/572,534/612,605. Every phase recorded
  zero non-region returns, ordinary feedback checks, explicit indirect misses,
  one-body returns, embedded-target skips, and successor observations.
- The existing region tag therefore bypasses this entire branch for both open
  scorecard rows. Dispatch-line feedback metadata has exactly zero dynamic
  opportunity and was rejected before a product implementation or timing run.
- Compile's 16.898% scheduler self belongs to the region call/return loop and
  surrounding boundary work. A next scheduler experiment must isolate that
  executed shape; optimizing dormant maps would be benchmark theater despite
  looking compelling in source.

## 2026-08-09: making the hot scheduler loop TurboFan-eligible is a product tie

- R091 changed only code layout: the exact generated chain moved from the
  33,230-byte Virt scheduler into one non-inlined helper, with every dispatch,
  mapping, feedback, profiling, fuel, retirement, and zero-progress operation
  retained in order. Both generated indirect calls moved and the caller invokes
  the helper exactly once; imports and exports are unchanged.
- The structural hypothesis activated unusually cleanly. The caller shrank
  18.98% to 26,922 executable Wasm bytes. V8 emitted a 12,860-byte Liftoff body
  for the helper and then a distinct 11,028-byte TurboFan body during an exact
  modern production Compile run. Main-module cold compile remained neutral.
- Complete correctness and three additional fresh modern Boots pass. The valid
  five-pair product A/B nevertheless measures Boot 0.975x `[0.960,1.020]` and
  Compile 1.009x `[0.950,1.070]`; Python remains guarded at 1.010x. Neither
  target reaches the deliberately modest 1.03x cumulative-gain rule.
- A missing optimizing tier for the giant scheduler was real but not a material
  cause of the wall-time gap. Sampled scheduler self includes required boundary
  work and time around nested generated calls; moving that code into optimized
  native form does not remove the nested work. Close scheduler outlining and
  adjacent helper/inlining variants rather than tuning a visually successful
  engine trace. Exact R085 is restored.

## 2026-08-09: a large target-row gain can still be a product regression

- R092's fixed whole-member range proof is not a marginal micro-result: native
  Compile improved 13.2% with a lower confidence bound above 3%, and browser
  Python improved 2.5%. This confirms that direct range accesses can remove
  meaningful generated-memory cost.
- The same immutable binary slowed WANIX shared 9P to 0.810x R085 with interval
  `[0.741,0.894]`; six of seven paired medians were unfavorable. Shell and
  SHA-256 stayed near parity, and every correctness marker passed.
- Therefore a target benchmark alone is not the objective. The product gate
  correctly rejected an optimization that would have looked excellent if the
  work stopped at native Compile. This is evidence of progress in both the JIT
  and the measurement discipline, not a justification to loosen the guard.
- R092 may not be repaired by selecting privilege mode, workload, PC, access
  floor, or clone shape after seeing this outcome. A future memory design must
  be independently motivated and preregistered. Exact R085 is restored and the
  parity score remains 11/13.

## 2026-08-09: small general gains are real, but confidence gates still bind

- R093 independently reconfirmed the old complete scalar Tier-0 loop under the
  current cumulative policy. Its same-Wasm Boot gain is 1.045x with lower bound
  1.029; the default-on product measures 1.074x native and 1.089x in fresh
  Chrome. Compile and Python stayed essentially flat. This is direct evidence
  against using 20% as a minimum per optimization: a selector-free 4--9% gain
  survives multiple engines and should advance through product gates.
- The mechanism is not benchmark-shaped. It covers the ISA-defined ordinary
  RV64I/M integer, control, scalar-memory, and integer-RVC families, with the
  authoritative decoder handling all slow families. Exhaustive instruction
  differentials and full modern-system gates pass.
- WANIX shell, Python, and SHA-256 all tie R085. Shared 9P's point estimate is
  favorable at 1.040x, but its seven paired medians range from 0.674x to 1.636x
  and produce `[0.730,1.580]`. A favorable median does not satisfy a frozen
  confidence rule.
- Do not call this a performance failure or a promotion. It is a real CPU-side
  gain rejected by the prospectively chosen integration-confidence contract.
  Rerunning only the noisy row after seeing the interval would be optional
  stopping and is forbidden. Exact R085 is restored; the official scorecard
  remains 11/13.

## 2026-08-09: fixed work, not more retries, stabilizes the shared-9P guard

- R094 compared exact R085 with itself in fourteen fresh browsers. Increasing
  only the versioned shared-9P work from 4 MiB to 32 MiB raised each synchronized
  sample from R093's 0.388--0.651 seconds to 23.765--25.757 seconds.
- The null point estimate is 1.0004x and its exact paired-bootstrap interval is
  `[0.9984,1.0165]`, versus R093's unusable `[0.730,1.580]`. Maximum local
  three-sample spread is 1.068x. Every one of 42 samples has exact 32 MiB P9
  writes, at least 32 MiB reads, 4 KiB maximum transfers, generated execution,
  and exact retirement accounting.
- This is prospective harness evidence, not a post-hoc rerun of R093. It
  qualifies a versioned guard for the next independent candidate while leaving
  the public workload and prior decisions untouched.
- The result separates two questions cleanly: small general CPU gains can be
  worth keeping, while integration non-regression still needs adequate power.
  A 3% promotion floor is defensible only when the guard's noise is much less
  than 3%; the qualified R094 interval now meets that standard.

## 2026-08-10: dynamic frequency alone does not establish tail-transfer cost

- Compile executes roughly eight million and Python roughly 37 million
  cross-module tail transfers, each with a diagnostic load/add/store. That made
  removing the counter a plausible general candidate, but frequency was only
  opportunity evidence.
- R096 used one main artifact and an emission-time mode, so neither leg paid a
  per-transfer branch. An exact structural test removed the six counter
  operators from the accounted module and proved every remaining operator
  identical. Directed architectural results were also identical.
- Seven fresh pairs show no target conversion: Compile is 0.991x with interval
  `[0.959,1.028]`. Boot is a small 1.005x `[0.999,1.009]` and Python is 0.994x
  `[0.973,1.031]`. All outputs and policy proofs pass.
- Close per-hop diagnostic accounting as a standalone optimization. Do not
  replace it with sampling or aggregation merely because the counter is visibly
  hot. The next Compile mechanism must remove or combine architectural transfer
  proof/work, not just its observer.

## 2026-08-10: tail-proof load reuse is a Liftoff-only optimization

- Holding the first dispatch generation and index loads in dead i32 locals
  changes four metadata loads to two and preserves every proof. Exact modeled
  success, miss, sentinel, blacklist, tagged-index, and cross-instance memory
  mutation cases match.
- Liftoff does not remove the redundancy: cached metadata is 1.649x faster with
  lower bound 1.525 and saves about 2.864 ns per modeled hop.
- Ordinary tiered V8 is 0.998x `[0.992,1.005]`; the saved cost disappears after
  optimization. The generated Compile modules that matter naturally tier, so a
  baseline-only micro-win is not portable whole-product evidence.
- Do not force Liftoff, key on engine identity, or keep functions cold to claim
  the local win. Close metadata-local reuse and look for work that remains
  necessary after TurboFan rather than source-level duplicate operations.

## 2026-08-10: removing an interpreter store yields only a small, unsafe product effect

- R098 preserved the existing interrupt sample points exactly while replacing
  a per-instruction countdown decrement/store with a modular absolute
  retired-count deadline.  Trap, xRET, CSR, wraparound, and pending-interrupt
  tests pass; this did not win by polling less often.
- Concrete disassembly across six `run` bodies and the exact re-entry,
  observed, and traced drivers proves one hot interrupt-cell store disappeared
  from every loop.  Complete release correctness, modern Linux, and native
  virt-smoke pass.  The final candidate is 48 bytes smaller, but size was only
  recorded—not used as an acceptance rule.
- Five fresh pairs convert the operator saving into a small Boot point gain of
  1.020x, with interval `[0.988,1.032]`.  Compile is 1.008x
  `[0.974,1.089]`, while Python's point estimate is a 0.948x regression with
  interval `[0.930,1.053]`.
- The countdown store is therefore real but not a safe standalone explanation
  for the remaining gap.  Reject at the native product gate rather than
  sweeping interrupt interval, counter representation, comparison spelling,
  or privilege modes after seeing the result.

## 2026-08-10: production region-policy bodies are dormant

- R099 records zero sampled exits, extension decisions, queue/drain visits,
  demotions, compatibility batches, and indirect-cache extensions across every
  Boot and Compile phase under production page policy.
- This is not evidence that the outer scheduler is cold: Boot still performs
  581,658 dispatches and Compile performs 533k--696k. It distinguishes hot
  dispatch from dormant optional policy machinery.
- Removing or tuning the dormant bodies cannot provide operation-count leverage
  toward the remaining 24--28% reductions. Keep the counters as a regression
  alarm and move attribution to dynamically active work.

## 2026-08-10: one vector TLB-entry load is a real but sub-threshold gain

- R100's exact paired shape proof replaces the scalar tag and offset reads with
  one vector read and consumes the offset lane only after the unchanged tag
  proof. The complete system-memory semantics and modern boots pass.
- Cold construction ties at 1.003x candidate/control. The candidate is 608
  bytes smaller, reinforcing that artifact size is neither a benefit nor a
  rejection proxy without measured effects.
- Five fresh pairs produce a 1.017x Compile point gain and the same 1.017x
  normalized-MIPS gain, but the interval `[0.966,1.083]` crosses regression and
  the median misses the frozen 1.03 cumulative floor. Boot and Python remain
  inside their guards.
- The earlier one-screen 6.4% R037 result did not replicate as a stable product
  gain. Close interleaved SIMD representation and do not search encodings after
  observing the result. Generated-memory optimization remains open only for a
  new mechanism with independent operation-level evidence.

## 2026-08-10: emitted operator frequency is not optimized-engine cost

- R101 counted 14,873,571 conservatively coalescible structured-member entries
  in Compile STEADY, or 37.210% of all member entries and 59,494,284 removable
  `local.get/local.get/i64.ge_u/br_if` operators. The selection was based only
  on CFG shape and static retirement bounds; it was not workload- or PC-shaped.
- R102 removed those checks, passed a 260-case fuel-bound proof and the full
  correctness matrix, and preserved the existing 127-instruction overshoot
  bound. The mechanism therefore activated exactly as intended.
- Seven native pairs nevertheless tie: Compile paired speedup and normalized
  MIPS are both 0.997x, with elapsed interval `[0.978,1.083]`. Boot and Python
  pass their guards, so there is no hidden product regression to explain the
  rejection; the target benefit itself is absent.
- The candidate grew by 14,623 bytes yet passed direct cold construction at
  1.018x candidate/control. This is concrete evidence that a fixed byte cutoff
  would have rejected on the wrong variable. Actual construction and workload
  timing remain the gates.
- Dynamic source/operator count is therefore only opportunity evidence. A hot
  host optimizer can make simple comparisons and branches effectively free or
  subordinate to unavoidable control transfer. Future generated-code work
  needs optimized-tier attribution or a local test that preserves the host
  engine tier, not only an emitted-operator census.

## 2026-08-10: carrying all integer state does not beat optimized memory state

- R103 proves the boundary volume exactly. Compile STEADY has 9,092,297
  generated invocations, of which 8,558,835 (94.13%) arrive through a
  cross-module tail call. It executes 185,783,621 GPR entry loads and
  178,499,259 GPR exit stores. After giving every outer call the maximum
  possible 62 operations, at least 331,208,236 remain chain-attributable.
- A new model uses two independently instantiated generated modules and the
  same one-table-owning `return_call_indirect` topology as production. Its
  treatment carries the architecture-defined x1--x31 plus PC, retirement, and
  fuel; there is no popularity subset or workload selector. Exact output and
  both Liftoff/TurboFan activation pass.
- Materialized and carried medians are 19.3977 and 19.4303 ns/hop, respectively,
  for 0.9989x paired speedup `[0.9861,1.0533]`. The control side also exceeds
  the frozen spread limit, so the result cannot admit product work even if its
  neutral point estimate were favorable.
- This reinforces R102's lesson at a more realistic boundary: hundreds of
  millions of memory operators are not automatically removable native cost.
  TurboFan can make linear-memory state highly efficient, while a 34-value
  typed tail-call ABI pays register-move/spill costs of its own. Close fixed
  full-GPR carrying rather than selecting fewer registers after seeing the
  outcome.

## 2026-08-10: a 1% floor still requires whole-product non-regression

- R104 replaces the arbitrary 3% economic cutoff with a verified 1% rule. A
  point estimate is not enough: the paired lower bound, fixed-work normalized
  throughput, protected rows, and product integration gates still bind.
- R105 is an exact executable reconstruction of the architecture-defined
  scalar Tier-0, not a tuned variant. Its 39,234 additional module bytes pass
  actual cold construction at `0.9893x` candidate/control. This directly
  falsifies source/Wasm size as a reason to reject the mechanism.
- The fresh same-Wasm result reconfirms a large real Boot benefit: `1.0588x`
  `[1.0360,1.0834]`, with normalized MIPS `1.0589x`. However, Compile falls to
  `0.9803x` and Python to `0.9792x`; Python's interval `[0.9600,0.9939]`
  excludes parity.
- This is why “accept every measured gain above 1%” must mean net positive, not
  “one selected row improved.” R105 clears the target-row floor but fails the
  prospectively declared protected workload contract. It is rejected for
  measured product tradeoffs, not code size or the superseded 3% threshold.
- No post-result privilege, workload, opcode-family, or sample retry follows.
  Future cumulative candidates remain eligible at 1%, but need evidence that
  the gain is not purchased by moving comparable cost into Compile or Python.

## 2026-08-10: small gains need powered gates at every stage

- R106's exact scalar/publication composition passed source, shape, semantic,
  lifecycle, public/Worker, and modern-Linux gates. Its 38,127-byte module
  growth was recorded but did not decide the result.
- Seven fresh construction pairs measured `1.0511625x` candidate/control versus
  a frozen `1.0500000x` maximum. The paired interval was wide
  (`[1.0102710,1.1289065]`), showing that a knife-edge point limit can classify
  a noisy startup cost without resolving its true size.
- R106 still must be rejected under its prospective protocol; relaxing or
  rerunning after a 0.116-point miss would be result-driven selection. No
  native runtime comparison was collected, so this is not a rejected 1% gain.
- Future cold gates should predeclare a powered non-inferiority interval or an
  amortized end-to-end budget. The verified-1% runtime rule remains: retain a
  general gain once its median is at least 1.01, its lower bound excludes
  regression, normalized work agrees, and protected workloads pass.

## 2026-08-10: cold construction should be a millisecond debit, not a ratio veto

- The scorecard deliberately starts after main-runtime creation, while dynamic
  generated-Wasm compilation occurs during the measured guest pumps. Only the
  former needs separate accounting; charging the latter again would penalize
  the same cost twice.
- R107 measures the real `RV64Debug.create` path rather than using module bytes
  or a synthetic `WebAssembly.compile` proxy. Fifteen same-artifact fresh pairs
  show 20.629/20.518 ms medians and a paired delta interval
  `[-2.273,0.155]` ms.
- The prospective debit is the positive upper confidence bound of that paired
  delta. It is added once to every candidate row before target, confidence,
  normalized-work, and protected-row decisions. Cold improvements receive no
  runtime credit.
- This preserves the user's intended rule: accept a verified net gain above
  1%, even when it is small, but subtract real excluded cost first. It also
  prevents a 0.6 ms event from being rejected merely because it is 5% of a
  tiny denominator.
- Browser survivors retain an independent construction-to-ready clock because
  Node cannot predict a browser Wasm engine's compile and tiering behavior.
  Code size remains diagnostic; actual construction and execution decide.

## 2026-08-10: a large local compiler win can still be a marginal product win

- R109's opportunity was real and general. Production uses dense member IDs,
  every captured edge was retained, and the fixed bit-matrix model was
  5.264x faster on first-call Boot and 6.628x faster when tiered. Compile also
  exceeded 5x. This was not a synthetic-only or workload-selected result.
- Exactness was unusually strong: 14,931 production/exhaustive/random graphs
  produced byte-identical structure serializations, and 280 modules generated
  from real RV64 ELF inputs were byte-identical across five state modes. The
  full emulator/JIT semantic matrix and direct/OpenSBI Linux boots passed.
- The whole product converted that local win into only a 1.01522x
  debit-adjusted Boot median, with interval `[0.99726,1.02755]`. Boot normalized
  MIPS agreed at 1.01507x, so the point effect is coherent but not resolved at
  the frozen confidence boundary.
- Compile's adjusted median was 0.98926x, narrowly below its 0.99 protection
  floor, while Python passed at 1.00453x. A 48,578-byte smaller Wasm neither
  rescued nor rejected the change; direct construction contributed a 1.042 ms
  conservative debit.
- This answers the small-gain policy question concretely. The 1% track did not
  discard the candidate for being small: it admitted full product timing and
  accepted the point estimate. The rejection comes from unresolved confidence
  and a prospectively declared protected-row miss. Do not retry, add samples,
  choose a hybrid width, or bundle the R108 sink after observing this result.

## 2026-08-10: optimized native frames are a measured Compile opportunity

- R110 samples hardware cycles in the exact modern Compile worker while V8
  writes native JIT code. The run is diagnostic only: perf and JIT logging
  perturb phase timing, so none of its milliseconds are performance evidence.
- Node/V8's dump contains two malformed debug-record sizes. A validated reader
  resynchronizes by exactly -6 and -5 bytes, parses all 5,985 records, and maps
  4,506 samples to the low-index JIT path. The same samples are never rerun;
  two preserved drafts are superseded only because operand classification and
  the shared-trampoline role were corrected.
- TurboFan owns 91.76% of mapped JIT-path cycles. Excluding the 128-byte shared
  tail trampoline, explicit `%rbp`/`%rsp` loads and stores own 22.44% of guest
  body cycles. Prologue/epilogue operations are separate, so this does not
  inflate the result with every unavoidable call-frame operation.
- Sampled TurboFan guest bodies have a 323.84-byte period-weighted frame. The
  weighted frame/stack-share correlation is 0.384; frames above 512 bytes spend
  31.02% of their sampled cycles on explicit stack traffic versus 18.45% for
  129--256-byte frames. Large generated functions are therefore creating real
  optimized-native register-pressure cost, not merely verbose Wasm.
- R088 independently places generated execution at 40.684% of Compile STEADY.
  Multiplying it by R110's guest-stack fraction of the complete JIT path gives
  an 8.87% whole-row upper bound. Removing roughly one ninth of that cost could
  in principle clear the verified 1% floor, but boundaries and unavoidable
  spills make the attainable result unknown.
- This does not reopen R039's local-reuse lowering, R103's full-state carried
  ABI, or per-module table imports. Admit only a same-module, CFG/liveness-based
  partition model that proves smaller optimized frames and net ordinary-V8
  execution. Do not select PCs, binaries, symbols, workloads, or forced tiers.

## 2026-08-10: naive same-module partitioning moves more cost than it removes

- R111 froze one architecture-general SCC/32-member/24-state rule before
  inspecting its static census. It covered all 15 Boot and 118 Compile CFGs
  and the existing 56 real compiler-produced regions / 6,258 members.
- The pressure-reduction half of the hypothesis passed. Regions representing
  91.41% of eager bytes split; their weighted state union was 62.90% of the
  whole-function value; and estimated maximum locals fell 16.06%.
- The boundary half failed by a wide margin. The rule cuts 2,527 of 4,779 Boot
  edges (52.88%) and 5,490 of 12,862 Compile edges (42.68%), versus the frozen
  12.5% ceiling. Oversized atomic SCCs occur in regions accounting for 61.00%
  of eligible bytes, versus the 20% ceiling.
- This is useful negative evidence: reducing a Wasm function's live-state
  footprint is not sufficient when the chosen split makes nearly half the CFG
  pay explicit state and function boundaries. Large loop SCCs also preserve
  exactly the bodies most likely to create register pressure.
- Gate A therefore stops before any synthetic model timing, native sampling,
  or product edit. Trying 16/48/64-member caps, another state limit, a graph
  order, SCC splitting, or a selected workload after these results would be
  parameter search, not the frozen experiment.
- Byte-identical duplicate corpora and analyzer reports preserve the result at
  `target/bench/r111-partition-model/`. The exact product remains
  `d9f686a9...`; R111 earns neither a performance rejection against the product
  nor scorecard credit because no product candidate was timed.

## 2026-08-10: Node's jitdump does not source-map generated Wasm

- R112 reuses the exact R110 perf data and jitdump. Its independent join closes
  bit-exactly against R110's total, main-thread, generated, tier, role, and
  native-family periods; no new samples or timings enter the result.
- All 250 `JIT_CODE_DEBUG_INFO` records are structurally valid after explicitly
  validating zero-valued 8-byte record padding. They contain 6,007 entries and
  associate unambiguously with ordinary JavaScript code loads.
- Their filenames cover `rv64.js`, the scorecard worker/library, and Node
  internals. Zero debug records associate with any sampled
  `wasm-function[0-5]` Liftoff or TurboFan load. Consequently, 0% of the
  8.736-billion-period TurboFan guest body and 0% of its 1.959-billion-period
  explicit stack traffic has a source position.
- This rules out a cheap post-processing route from existing native samples to
  Wasm operators. The missing mapping is an engine-output fact, not evidence
  that the spill opportunity disappeared.
- Do not recollect with a selected engine, debug flag, or workload to make the
  source gate pass. Any next attribution must use only native structure already
  present (for example stack-slot/control context) or begin a separately
  justified architecture-general experiment; it cannot call debug lines Wasm
  offsets by inference.

## 2026-08-10: optimized spills are distributed body pressure, not calls

- R113 reclassifies all 897 TurboFan guest stack samples while reproducing
  every R110 period partition. Three executions produce byte-identical JSON.
- No stack sample is within the frozen same-basic-block eight-instruction call
  neighborhood. Entry-prefix work owns 14.29%; control-neighborhood work owns
  32.66%; and the general body owns 53.05%. Removing helper-call boundaries
  therefore cannot explain the observed spill opportunity.
- Pure register reloads are the largest form: 51.70% of stack period and a
  4.196% optimistic whole-Compile exposure. They occur in 70 code loads; the
  top one/top five own only 9.56%/27.85%, so the symptom is general rather than
  a selected module.
- Immediate stack comparisons adjacent to branches own another 21.83% of stack
  period, while entry-prefix register spills own 12.28%. This is consistent
  with long-lived state and structured selector pressure across large bodies.
- Register reloads miss R113's prospectively frozen 500-sample model gate at
  464. The threshold is not relaxed after seeing the count, and no other form
  passes. This closes a form-specific model, not an actual optimization: the
  4.196% number is removable-cost ceiling, not measured gain.
- R039 already showed that source local reuse can shrink Wasm while worsening
  optimized execution, R103 showed that a large typed ABI moves spill cost,
  and R111 showed that function partitioning creates too many boundaries. The
  remaining design question is whether within-function structured state SSA
  can shorten live ranges without any of those three costs; it requires an
  independent model, not reinterpretation of R113.

## 2026-08-10: the clearest old small-gain casualty fails fresh reconfirmation

- The historical audit did find threshold casualties. R071 would have advanced
  past native timing under today's verified-1% rule, while R014 was rejected in
  part because 1.1% FIRST and 3.0% STEADY improvements missed the then-fixed
  10% gate. Neither old result is sufficient for promotion: R071's surrounding
  baseline was later shown to be contaminated, and R014 mixed two mechanisms
  with 92%/95% generated-coverage strata.
- R114 reconstructed only R014's independent lazy-PC component on exact current
  `d9f686a9...`. It passed full semantics and matched coverage. The generated
  change is real: 8,542 member safety branches changed shape, and internal PC
  writes moved to leaving paths across all 56 real regions.
- Fifteen fixed pairs do not reproduce the old gain. After the prospectively
  frozen 1.432463 ms construction debit, Compile is `0.98579x` with interval
  `[0.95202,0.99629]`; normalized work is `0.98577x`. Because even the upper
  interval bound is below 1.00, the current implementation is demonstrably
  slower rather than merely unresolved around a 1% gain.
- The conclusion is not that small changes are unworthy. A verified net gain
  above 1% is now promotable and code size is diagnostic only. The conclusion
  is that historical point estimates must be reconstructed against the current
  product and pass end-to-end confidence and protection gates before acceptance.
- No historical artifact is retroactively promoted by the audit. R114 evidence
  is authenticated by `target/bench/r114-lazy-internal-pc/SHA256SUMS`; the
  live product is restored byte-exact to `d9f686a9...`.

## 2026-08-10: R095 paid a real instance tax, but removing it is not a product win

- R115 relocates the exact four-function R095 hand-emitted Tier-0 and passive
  RVC table into its main Wasm module. Helper calls become same-instance direct
  calls; decoder semantics, orchestration, policy, guest inputs, and ordinary
  generated code are unchanged.
- Seven same-artifact pairs establish the boundary cost. Same-instance versus
  external-instance Boot is `1.03413x` with interval `[1.01796,1.04451]`;
  Compile's point result is `1.02349x`; Python is neutral. All 42 legs pass,
  host spread is 1.072156x, work matches, and each leg executes roughly 107M
  Tier-0 fast instructions with zero errors.
- That corrects the causal interpretation of R095: cross-instance calls and
  instance-state switching materially harmed it. It does not reverse its
  product decision. On the identical embedded artifact, enabled versus
  disabled Boot is `0.97987x` with interval `[0.95574,0.99572]`, establishing
  regression. Compile is `0.99715x`; Python's unresolved `1.00955x` cannot
  compensate for Boot.
- One Compile control sample exceeded the report spread limit, which alone
  forbids positive admission. The complete Boot subset is stable and its
  upper confidence endpoint remains below parity, so no rerun can turn this
  frozen candidate into a protected-row pass.
- This is precisely how the verified-1% policy should behave: accept the
  measured 3.4% boundary improvement as a finding, but do not confuse removing
  a penalty with a net product gain. Module size is diagnostic and played no
  role. Evidence is authenticated under
  `target/bench/r115-same-instance-proof/`; product remains `d9f686a9...`.

## 2026-08-10: provably free selective residency is too narrow

- R116 freezes one natural hot/cold split before inspecting its census: only a
  state value referenced by one acyclic member becomes materialized. This
  adds no more architectural loads/stores per invocation than the eager entry
  and exit operations it replaces, while adding no edge or function boundary.
- The deterministic 56-region / 6,258-member census proves the rule is broad
  enough to activate: 40 regions and 77.52% of eager-byte weight contain cold
  state. No cold value is cyclic and no region violates the operation bound.
- It nevertheless removes only 8.28% of weighted architectural state and
  2.04% of total declared locals, missing the frozen 20% and 5% opportunity
  floors. The median region has two cold values; long-lived cross-member or
  cyclic values dominate the state union.
- Stop before implementation. Making multi-member or cyclic registers cold
  would abandon the proof that materialization adds no dynamic operations and
  would require a separately justified cost model. R116 does not measure or
  reject a runtime gain; it rules out this zero-extra-work form as too narrow
  to address R110's broad optimized-native pressure.

## 2026-08-10: private Wasm globals do not provide cheaper spill storage

- R117 isolates one architecture-general representation change inside the same
  generated module and function: 31 long-lived i64 architectural values move
  from locals to private mutable globals while all other work remains exact.
- Independent decoding proves the normalized 963-operator streams identical.
  Every one of 160 state reads and 95 state writes maps by index; both sides
  perform the same 31 input loads and 31 output stores. The candidate removes
  31 state locals rather than leaving unused declarations behind.
- All 15 alternating CPU-pinned pairs pass identity, deterministic output,
  affinity, host, and timing-spread checks. Ordinary-tiered V8 measures
  candidate speedup `0.970330x` with interval `[0.967047,0.973196]`; first
  execution is `0.826972x` `[0.814919,0.842724]`.
- The result rejects the premise before native inspection: V8 globals are not a
  cheaper optimizer-visible backing store for distributed architectural state.
  Removing Wasm locals does not imply a smaller or faster native frame, and
  the global access mechanism adds an established execution cost.
- Stop without a product mode or a partial-register sweep. This is not a small
  positive change rejected by a coarse threshold; it is a stable 3% steady
  regression. Evidence is under `target/bench/r117-module-global/`; production
  remains exact `d9f686a9...`.

## 2026-08-10: flat RVC dispatch was worth retesting, but regresses current Boot

- The expanded historical audit found a methodology casualty beyond the old
  percentage cutoff. R059's flat RV64C model was rejected because one warmup
  call caught V8 tier publication even though all later steady calls were
  stable and strongly favorable. Current methodology correctly separates
  FIRST, PRIME/tier-up, and STEADY, so one clean current-baseline reconstruction
  was warranted without crediting the old timing.
- R118 changes only the complete outer RVC selector. Two release builds are
  byte-identical; all six interpreter specializations replace four initial
  tables with one, preserve call topology, and leave every non-step function
  body byte-identical. Exhaustive RVC equivalence and the complete strict
  correctness/Linux gate pass.
- The local-model premise does not survive the full product. The stable 15-pair
  Boot subset measures raw `0.982719x [0.968926,0.993667]` and construction-
  debited `0.982183x [0.968403,0.993110]`; normalized work is `0.982098x`.
  This establishes about a 1.8% Boot regression rather than a small gain.
- Python's adjusted point result is favorable at `1.022471x`, while Compile is
  `0.996783x`. One control Compile outlier breaches its spread gate, making the
  complete report invalid for positive admission. That cannot rescue R118:
  Boot itself is stable and its upper confidence endpoint remains below parity.
- Candidate Wasm grew only 1,354 bytes. That is diagnostic and played no role.
  Reject the flat selector on end-to-end behavior, archive the exact evidence,
  and restore `d9f686a9...` without a rerun or family-order/spelling variant.
- The policy conclusion remains affirmative: verified net gains at or above
  1% should accumulate. The audit found an experiment that deserved a fair
  modern retest, not an old implementation that deserves retroactive adoption.

## 2026-08-10: the expanded legacy ledger contains two more small-gain leads

- The earlier historical-audit closure was accurate for implemented rewrite
  candidates but too broad when read as covering the deleted legacy backend.
- Legacy E005b reported a one-pair 4.5% Compile improvement from disabling its
  per-trace TLB cache. Legacy E006b reported a one-pair 2.6% Compile
  improvement at tier threshold 32 instead of 64. Their recorded rejection
  reasons were the former 10% floor, not measured protected-row regressions.
- Neither result is acceptable evidence today: each is a single point sample
  without confidence or product guards, and neither mechanism maps directly
  to the rewrite's active production page policy. E005b's cache and E006b's
  tier path disappeared with `rv64-jit`.
- Treat them as historical leads, not artifacts to restore. Reopen an idea only
  if current profiling proves an equivalent cost is active, then freeze one
  current-baseline test and require the same >=1% confidence, construction,
  correctness, browser, WANIX, and scorecard gates as any new candidate.
- Small verified gains are valuable: ten independent 1% gains compound to
  about 10.5%, and twenty compound to about 22.0%. A large per-change target
  would systematically discard that path to parity.

## 2026-08-10: fused execute-TLB fetch is a real Boot near miss, not an admissible net win

- R119 used a fresh exact-baseline Boot profile to isolate the physical-bus
  path after the execute-TLB hit. That native band owns 1.8554% of all sampled
  cycles, so the implementation reused the existing tag proof and consumed a
  direct-RAM capability in its existing payload slot. No second probe, cache,
  selector, or benchmark-specific condition was added.
- The full strict correctness/Linux gate and native one-probe shape gate pass.
  Two candidate Wasm builds are byte-identical at `41b94faa...`; the 3,187-byte
  growth is recorded but plays no role in the decision.
- The valid 90-leg report has host spread 1.067733x and all identities, work,
  output, cadence, affinity, and generated-coverage guards pass. After the
  conservative 1.258935 ms construction debit, Boot is `1.012411x` with 95%
  interval `[0.997859,1.015302]` and normalized work `1.012351x`.
- That positive 1.24% point is not confidence-verified because the lower bound
  crosses parity. Compile also misses its protected median at `0.984634x`
  `[0.947863,1.016480]`; Python passes at `1.001649x`
  `[0.981848,1.029909]`.
- Reject and restore exact baseline. This is precisely the distinction the new
  policy needs: small verified gains are accepted, while a favorable small
  point with unresolved downside and a protected-row miss is not called a net
  product win. Do not extend the sample or tune an R119 variant after seeing
  the result.

## 2026-08-10: R100 deserved remeasurement, but its 1.7% Compile point was noise

- R100 was a concrete former-floor casualty: its exact interleaved fused-TLB
  candidate showed a five-pair `1.017x` Compile point but failed the old
  `1.03x` target. R120 prospectively froze those exact bytes and today's
  verified-1% rule; no old observation was pooled into the new decision.
- Authenticity is exact. Two isolated archived-source builds reproduce
  `c36da489...`; the candidate is 608 bytes smaller than exact control
  `d9f686a9...`, and size plays no decision role.
- Fifteen construction pairs impose a conservative 0.231087 ms debit. The
  valid 90-leg native report has host spread 1.065187x and all identity,
  output, work, cadence, affinity, and generated-execution guards pass.
- The old Compile point does not reproduce: adjusted Compile is `0.992069x`
  `[0.952178,1.015084]`, with normalized fixed work `0.992040x`. Python also
  misses its protected median at `0.982841x [0.954431,1.033982]`; Boot is a
  safe but immaterial `1.003286x [1.001075,1.013737]`.
- Reject without product reapplication or browser/WANIX/scorecard work. The
  historical lesson is not to demand large improvements; it is to give small
  candidates enough predeclared evidence. A verified net >=1% gain should be
  accepted, while one favorable five-pair estimate should not.

## 2026-08-10: dispatch misses are frequent but their native ceiling is below a credible net 1%

- R121 closes every preserved optimized `run_system_jit` sample over exact
  native control-flow bands. The whole scheduler owns 2.6048% of R110
  main-thread period; direct lookup owns 0.2182%, fallback hash/mapping/refill
  owns 0.7481%, and the production region-call entry owns 0.1771%.
- One exact diagnostic Compile run explains the fallback population. STEADY
  performs 924,436 outer visits: 38.29% direct hits and 61.71% fallbacks. Of
  fallbacks, 61.21% begin at an empty direct line, 37.45% are stale-generation
  lines, 1.31% are collisions, and 0.03% are unverified publications.
- Mapping failures are not the churn: 213,808 compiled-block lookups produce
  213,807 successful refills and one actual drop. Selective page SFENCEs are
  3,118 of 3,143 STEADY mapping invalidations, explaining about 68 stale line
  refreshes per event.
- Negative absence caching or parallel PA metadata could remove only part of
  the 0.7481% measured fallback band and would add line/table loads,
  publication invalidation, and collision pressure while retaining mapping
  proofs. Late one-second bins briefly exceed 1%, but have no authenticated
  phase markers and cannot establish STEADY exposure.
- Close these variants before implementation. This is an exposure decision,
  not a code-size decision or rejection of a measured small gain. Diagnostic
  counters are removed and exact `d9f686a9...` is restored.

## 2026-08-10: the interpreter semantic body is large but has no independent one-percent leaf

- R122 reuses the immutable uninstrumented R119 Boot capture. Its deterministic
  native analyzer closes all `Cpu::step` period and assigns 16.1968% of
  main-thread period to compressed semantics and 14.0594% to RV32 semantics.
  Outcome, retirement, PC-store, and return bands are separated rather than
  being credited to a hypothetical decoder rewrite.
- Five semantic blocks exceed the frozen 1.25% evidence floor. RV32 assembly
  plus dense opcode dispatch is required; two compressed blocks are
  quadrant-2 family tests covered by R118's no-selector-variant closure; one
  is accepted R054 fused-memory work; and the remaining 1.3996% quadrant-1
  block combines required dispatch/register decode with a partial immediate
  opportunity.
- The exact count census shows that only 30.1242% of quadrant-1 attempts avoid
  the common six-bit immediate. Charging them the entire mixed block gives an
  impossible 0.4216% main-thread ceiling before duplicated decode cost. Do not
  implement or time that partial source motion.
- Modern Boot retires 180,397,864 guest instructions: 71,704,074 generated and
  108,693,790 interpreted. Compressed and 32-bit retirement are
  67,576,293/41,117,497; sequential and non-sequential successors are
  91,387,416/17,306,374. Every total independently closes.
- The GPR write helper runs 66,630,535 times but discards x0 only 730,168 times
  (1.0958%). A per-retirement x0 restore adds more stores than it removes
  branches, while a scratch slot adds comparison/select/address work to every
  helper call. It supplies no credible architecture-wide candidate.
- No product performance run follows. Counter timing is explicitly ineligible,
  all temporary instrumentation is removed, evidence is authenticated under
  `target/bench/r122-interpreter-body/`, and exact `d9f686a9...` is restored.
  This closes an opportunity decomposition, not a measured small gain.

## 2026-08-10: the fused-memory production flag is real but only 0.104% of Boot

- R123 authenticates the enable-flag load, compare, and branch in all five
  optimized scalar `ld`/`st` bodies from the immutable R119 native capture.
  Two analyzer outputs are byte-identical at `d08985d0...`.
- The complete memory bodies own 7.093307% of main-thread period, but their
  complete guard blocks own only 0.104231%. The `ld1` band includes one
  unrelated spill and therefore deliberately overstates removable exposure.
- Impossible zero-cost removal projects only `1.001043x` whole Boot. The
  once-per-host-call setter cannot supply the missing approximately 0.9%; it
  is not the same dynamic population and cannot be combined with unrelated
  scheduler work to manufacture exposure.
- Stop before a product edit or timing. This is not a code-size veto or a
  rejected positive runtime result. Retain R054's diagnostic flag and exact
  `d9f686a9...`, and move to an independent current cost.

## 2026-08-10: high proxy thresholds can recreate the rejected small-gain policy

- The ledger audit confirms actual old threshold casualties: R071, R014,
  R100, and legacy E005b/E006b had favorable 1--4.5% observations that older
  3--10% rules stopped; R059 was additionally lost to a warmup/tiering gate.
- Fair modern reconstructions do not justify restoring an artifact. R114,
  R118, and R120 are slower on their current target rows; R119 has a favorable
  1.24% point but does not verify parity and regresses protected Compile.
- The lesson still changes prospective work. Ten independent 1% gains compound
  to about 10.5%, and twenty to about 22.0%; demanding a large fraction of the
  remaining parity gap from each edit systematically discards that route.
- R104's final 1% product rule is not sufficient if an earlier model demands
  5% or a structural proxy demands 10--20%. Such proxies remain attribution
  evidence, but only a hard complete-cost ceiling below 1%, invalidity, or an
  established regression may prevent a practical candidate from reaching the
  authoritative product measurement.
- This clarification was recorded after R124 A1 passed and before any dynamic
  R124 result. Its original proxy targets remain reported diagnostics, and its
  architecture-fixed register bank remains unchanged.

## 2026-08-10: the first R124 model inverted product boundary frequency

- The frozen ordinary-V8 run is reproducible and internally valid for its two
  modules: steady hybrid/eager is `0.966035x [0.962105,0.967411]`, FIRST is
  `1.018706x [1.008923,1.024598]`, and all host, identity, work, output, state,
  affinity, and schedule checks pass.
- It is not a valid model of the proposed product trade.  One generated call
  executes 65,536 complete rounds.  Eager pays 31 loads and 31 stores once;
  hybrid pays 20 resident boundary operations once plus 84 materialized
  operations per round.  At STEADY that is 5,505,044 versus 62, or an
  approximately 88,791x relative overcharge.
- The exact production census has the opposite topology: projected hybrid
  operations are `0.737698x` current in STEADY.  The model slowdown therefore
  cannot establish that the product mechanism regresses.
- Preserve the raw failed report and do not repair/retest a synthetic model
  after seeing its outcome.  D121 permits the bounded fixed-bank product to
  reach authoritative product measurement; full correctness, construction,
  confidence, and protected-row gates remain unchanged.

## 2026-08-10: the exact R124 product strongly passes the verified 1% gate

- Candidate `d017a10f...` is reproduced by two isolated builds and passes the
  complete predeclared correctness matrix, including bulk copy, precise exits,
  randomized T2, and both modern Linux boot paths.
- Fifteen alternating construction pairs impose a conservative `0.168840 ms`
  debit.  The 90-leg native A/B is valid with host spread `1.054948x`.
- Debited Compile is `1.083675x [1.037357,1.112250]`; normalized fixed work is
  `1.083602x`.  This is a confidence-verified 8.37% target gain, not a noisy
  favorable point.
- Boot improves `1.018471x [1.000973,1.035481]`.  Python improves
  `1.200538x [1.180720,1.220097]`, so the protected benchmark not only avoids
  regression but benefits materially.
- All frozen checks pass.  The 2,406-byte Wasm increase has no decision role.
  Advance the exact bytes unchanged; do not tune the bank or add another
  optimization before Chromium, WANIX, and scorecard qualification.

## 2026-08-10: R124's gain survives natural Chromium and inclusive construction

- In seven untouched fresh-Chrome pairs, execution-only modern Boot improves
  `1.018971x` with exact paired-bootstrap interval
  `[1.001431,1.037301]`.
- The second clock starts immediately before `RV64.create` and therefore
  includes main-Wasm compilation/instantiation and generated-module work. It
  improves `1.017013x [0.998068,1.029593]`; its interval does not establish a
  construction regression.
- This is an engine-portability confirmation, not pooled credit from native
  timing. The sole browser run passes independently, and every immutable
  artifact, Linux 6.12.7 / Alpine 3.24.1, output, generated-retirement, and
  cadence check closes.
- Exact `d017a10f...` therefore advances unchanged. The WANIX gate was frozen
  with the original Python body and R094's independently qualified 32 MiB
  shared-9P work before its first R124 sample.

## 2026-08-10: R124 WANIX was invalidated by the common external-9P endpoint

- The sole frozen R124 WANIX run stopped without replacement at pair 5 exact
  control. That leg reached the shell and completed correct Python and SHA-256,
  but its first 32 MiB shared-9P sample never returned. The four completed
  pairs and every diagnostic sample are ineligible for candidate credit.
- A visible console-release token arrived, the exact write and read traffic
  crossed the emulator, and guest instructions continued to retire. One
  decoded `T_UNLINKAT` request (tag 0, directory fid 30, flags 0, benchmark
  temporary filename) remained unanswered for more than 64 seconds. This
  rejects the console-input hypothesis and locates the invalidity in the
  common external-9P integration, not in candidate execution speed.
- Read-only inspection of exact WANIX source `76779e30...` shows the adapter
  feeding multiplexed requests into a stream-backed concurrent Go 9P server.
  Multiplexed delivery is established as a prerequisite for the stall; the
  exact internal WANIX lock cycle was not stack-dumped and is not claimed.
- The correction queues only the WANIX adapter endpoint FIFO single-flight.
  The generic emulator loader remains concurrent, and the actual-adapter
  qualification observed loader maximum pending 3. Directed tests cover FIFO
  ordering, tag collision, unknown replies, and synchronous post failure.
- Six fresh loader-prototype and six fresh actual-adapter 32 MiB runs all
  complete without a stall. Actual-adapter elapsed samples span
  `25.118--25.612 s` (`1.020x` max/min), with exact writes and at least 32 MiB
  reads. Two ordinary builds reproduce adapter `bba6baaf...` byte-for-byte.
- R125 is frozen from zero with common corrected adapter `bba6baaf...`, exact
  control `d9f686a9...`, and unchanged candidate `d017a10f...`. It retains the
  seven-pair, three-repetition protected-row rules. No old or diagnostic sample
  may enter its result.

## 2026-08-10: R125 is valid and broadly faster but fails its strict shell guard

- All fourteen from-zero formal legs complete without a stall or stderr. Every
  identity, output, active-JIT, generated-coverage, exact-work, 9P-byte,
  transfer-size, spread, affinity, cadence, and freshness proof passes. The
  adapter correction is therefore qualified under the full formal workload.
- Candidate `d017a10f...` improves unchanged Python by `1.079692x
  [1.070039,1.111911]`, SHA-256 by `1.021724x [1.018486,1.037770]`, and shared
  9P by `1.009359x [1.003915,1.016363]`.
- Shell is `0.996257x [0.989513,0.998082]`: only 0.374% slower by paired
  median and still above the frozen `0.99x` material floor, but its upper
  interval endpoint establishes regression. That is the sole frozen failure.
- The unweighted four-row geometric-mean point is `1.026272x`; because no
  aggregate weighting was preregistered, it is diagnostic and supplies no
  promotion credit. This result cleanly exposes the difference between
  "nothing may establish any regression" and "no protected path may regress
  by 1%".
- Preserve the valid failure and stop before the scorecard. Prospective policy
  may use the one-percent median floor as the material-regression boundary or
  define a weighted net score, but neither may reinterpret R125 after seeing
  its samples.

## 2026-08-10: the owner selects the one-percent material boundary

- R125's immutable analyzer failure remains part of the record. The owner
  explicitly resolves the separate product-policy question rather than
  relabeling that analyzer output or collecting friendlier samples.
- A protected row now vetoes a candidate for performance only below `0.99x`
  by paired median or when its interval establishes a regression larger than
  1%. A confidence-established 0.374% slowdown is reported, not promoted as a
  gain, and not allowed to erase independently verified general improvements.
- On that rule, R125 supplies valid WANIX qualification: unchanged Python is
  7.97% faster, SHA-256 2.17% faster, shared 9P 0.94% faster, and all work,
  output, JIT, transport, and correctness proofs pass.
- Exact R124 `d017a10f...` advances without retuning to the untouched modern
  117-trial scorecard. That independent report, not a WANIX rerun, decides
  product retention.

## 2026-08-10: R126 exposed two scorecard admission gaps before measurement

- R126 scheduled all 117 processes but produced zero eligible trials. The
  explicit candidate-Wasm override is intentionally diagnostic-only, so 78
  RV64 results cannot enter an authoritative aggregate.
- The exact matched x86 kernel was absent and the top-level runner did not
  preflight it. All 39 v86 workers exited before results. The exact R087 input
  remains available at hash `8854efec...`; a similarly named `507a759c...`
  image is different and must not substitute.
- Report `ac096fec...` is measurement-invalid with 135 problems. Its printed
  medians and apparent `goalMet` value receive no credit and cannot influence
  the replacement.
- A valid product scorecard must source-build exact R124 as the ordinary live
  release, use no diagnostic override, require every worker input before the
  first trial, and collect all 117 replacement trials from zero.

## 2026-08-10: R127 proved repaired admission but was externally terminated

- The exact live `d017a10f...` build passed the corrected top-level input
  preflight and the separate v86 generated-execution dispatch. The missing
  kernel and diagnostic-override defects from R126 did not recur.
- R127 completed ALU, Mixed, and Boot and entered Python without a worker
  failure. Long legacy Python legs were observed CPU-bound at roughly one core,
  consistent with work rather than a deadlock.
- The interactive execution session was then externally aborted during an
  agent continuation handoff. Its scorecard parent and active worker vanished;
  no report or formal output directory exists.
- Terminal row markers are orchestration traces, not a recoverable formal
  population. R127 provides no performance evidence and no process is reused.
  R128 will rerun all 117 processes with the unchanged experiment, but with the
  parent detached and raw logs persisted so a tool-session handoff cannot kill
  it.

## 2026-08-10: R128 detachment worked and the owner paused the experiment

- PID 525150 was adopted by PID 1 in its own session, so R128 remained alive
  independently of interactive tool calls. The manifest, both selftests, and
  real v86 generated dispatch passed.
- ALU, Mixed, and Boot reached orchestration `ok`; Python had launched rewrite
  and the first legacy leg when the owner requested a pause.
- The exact R128 process group was terminated with `SIGTERM`. No scorecard
  process remains, no formal report exists, and no partial process receives
  performance credit.
- The live uncommitted source/release remains exact candidate `d017a10f...` for
  experiment reproducibility. It has not been accepted by the modern
  scorecard, committed, or declared the product.

## 2026-08-10: final exercise conclusion

- The implementation milestone succeeded: the former compiler was replaced by
  a clean-room RV64-to-Wasm DBT with modern full-system boot, broad differential
  correctness, asynchronous generated-Wasm lifecycle management, and a much
  stronger three-way measurement harness.
- The performance objective did not finish. R087 is the last valid full modern
  scorecard at 13/13 legacy and 11/13 copy/v86; Boot and Compile remain open.
- Exact final candidate `d017a10f...` is promising but unpromoted. It passed
  R124 native and natural-Chromium gates and improved R125 public Python,
  SHA-256, and shared 9P, while measuring a materiality-tolerated 0.374% shell
  slowdown. It never produced a valid full three-way report.
- R126 has zero eligible measurements, R127 has no report after external
  termination, and R128 has no report after the owner-requested stop. No result
  can be inferred from their displayed or partial state.
- Closure verification on the terminal candidate passes all 177 Rust workspace
  tests and the scorecard/R125/R126 analyzer selftests. This proves the recorded
  code is testable; it does not fill the performance-evidence gap.
- The exercise ends with no new Git commit or merge, exact candidate live in
  the dirty worktree, exact accepted-control source/artifacts preserved, no
  running benchmark, and the parity objective explicitly unachieved.
