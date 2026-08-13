# R111 Same-Module Partition Model Protocol

Date: 2026-08-10  
Status: closed at frozen Gate A; no model, timing, or product change

## Question

Can a deterministic same-module function partition reduce the optimized-native
frame/spill cost measured by R110 enough to clear the verified 1% whole-Compile
floor after paying every new state and function boundary? R111 is proof-only.
It may admit one separately frozen product implementation, but it cannot change
the emulator, earn scorecard credit, or use its own elapsed time as product
evidence.

## Closed axes

R111 does not reopen:

- R039's source-local reuse/stackification;
- R103's fixed full-GPR cross-module carried ABI;
- per-module function-table ownership or direct table imports (D054);
- materializing state at every member boundary, already several-fold slower
  than register residency in the frozen backend corpus;
- a forced Liftoff/TurboFan policy, engine-version selector, or background
  compilation change; or
- any guest PC, symbol, opcode family, binary, workload, privilege, benchmark,
  or observed-hotspot selector.

## Frozen source and evidence

- product baseline Wasm:
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`;
- core source:
  `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`;
- Wasm runtime source:
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- R110 authoritative native census:
  `4aab78cbb7a5dc2045b42abe420b42ed6ad9262a267b4e7029f137ce94ce971f`;
- R109 production CFG captures:
  - Boot `fc02c2f8479a7d752f7e642255be5be0a2be719597f188b9c442027d795bbc3b`,
  - Compile FIRST `951d822372b2cf79e9d457282c6d21170f129d81375ac2f323518e3fe7942ce9`,
  - Compile PRIME `66746dccc7d652f9e7f341eb122d0cae0b42ce220f653b4e5149d4c7e0d2b8b8`,
  - Compile STEADY `6f8438007242c43a6e88e6cb69fa9e20d1b5e4c3eefcfc68598693493019f5d6`;
- compiler-generated real-region inputs:
  - rvbench RV64 ELF
    `b6d7c3a155ac43923a0a961b6276284475e34138bd14a006305514ddf661a6f0`,
  - Alpine riscv64 musl
    `6f5112cbbcea72eccd78c6c9103470f22e24646545c971dcc7cf0d37f21262d2`;
- engine: Node `v26.5.0`, V8 `14.6.202.34-node.24`;
- host allocation: CPUs 8--15, with the same host-stability probe used by the
  native scorecard gates.

R109's CFG captures and real-region modules are reused read-only. R111 may add
diagnostic shape APIs and examples that are unreachable from the product Wasm.
It may not rebuild or replace the frozen captures after seeing a result.
The retained shape API is compiled only with the non-default
`rv64-dbt/r111-diagnostics` feature; the corpus example declares that feature
as required. A normal `rv64-wasm` release build therefore excludes the API.

## Frozen partition rule

The prospective shape is fixed before inspecting R111 census results:

1. Build the covered static-successor graph already supplied to structured
   lowering. Compute strongly connected components; an SCC is atomic and is
   never split across Wasm functions.
2. Order SCCs by their lowest existing dense member ID. Greedily append the
   next SCC to the current function unless either limit would be exceeded:
   - 32 guest members; or
   - 24 architectural state values in the union of integer/FPR live-ins and
     outputs, counting `fcsr` as one value.
3. An SCC that alone exceeds a limit remains one oversized function. There is
   no recursive split, hot/cold exception, register-popularity subset, alternate
   cap, or post-result fallback.
4. Covered edges inside a partition retain the existing structured Wasm
   branches. A statically known edge to another partition commits only that
   function's dirty architectural union and uses a direct same-module
   `return_call`; the target loads its required union. Dynamic/external edges
   take the unchanged precise commit and outer path.
5. All public member entries name one same-module dispatcher. Fuel, retirement,
   precise exits, memory capabilities, helper coherence, invalidation, and the
   shared cross-module tail trampoline remain unchanged.

The 32-member limit is one sixteenth of the existing hard 512-member bound and
also the architectural integer-register namespace size. The 24-value limit
reserves one quarter of that namespace for PC/fuel, SSA, memory-proof, and
engine temporaries. These are fixed portable resource bounds, not fitted
timing parameters.

## Gate A: static opportunity census

Implement one deterministic read-only analyzer for the rule above. It must:

- parse all 133 frozen production CFGs (15 Boot, 118 Compile), preserving every
  node and edge;
- lift the unchanged 56 real compiler-generated regions / 6,258 members used
  by the R109 corpus, record per-member integer/FPR/fcsr masks and SSA local
  counts, and reproduce the existing whole-region masks and module identities;
- run the exact SCC/greedy rule once and serialize every partition assignment,
  oversize SCC, cut edge, union, and estimated local footprint; and
- be deterministic across two fresh invocations with byte-identical output.

Estimated local footprint is fixed as follows. Parse the control eager
function's declared local count. Subtract its architectural-state union and
largest-member i32+i64 SSA counts to obtain one conservative fixed-extra count
(PC, retirement, fuel, selector, hop, and memory-proof temporaries). A
partition's estimate is that same whole-region fixed-extra count plus its own
architectural union and largest-member i32+i64 counts. The candidate maximum is
the largest partition estimate; no i32 discount or post-census native-slot
fit is allowed.

The model advances only if all prospective conditions pass:

1. at least 75% of real-corpus eager-module bytes belong to regions that split
   into two or more functions;
2. module-byte-weighted mean partition state union is at most 80% of the
   corresponding whole-function union;
3. module-byte-weighted estimated maximum local footprint falls by at least
   15%;
4. no more than 12.5% of internal static edges are cut in either the frozen
   Boot or aggregate Compile CFG corpus; and
5. oversized atomic SCCs account for no more than 20% of otherwise eligible
   real-corpus bytes.

Static edges are not dynamic speedup evidence. Failure stops R111 without a
timed model and without changing the rule.

## Gate B: fixed ordinary-V8 model

If Gate A passes, generate exactly two standalone Wasm modules from one fixed
model. Both execute a 256-member acyclic chain and identical state/data work:

- control: one function containing all 256 member bodies and the x1--x31
  architectural union;
- candidate: eight 32-member functions in the same module, each using a fixed
  24-value sliding architectural union, committing eight dirty values at its
  boundary, and direct-tail-calling the next function;
- every member performs 32 rounds of dependent i64 arithmetic, comparison,
  address generation, and bounded linear-memory load/store work;
- there is no padding, dead operation, imported helper, indirect call, table,
  JavaScript edge, or configurable member/round/register count.

Generation runs twice before timing and must be byte-identical. A mechanical
shape gate requires one versus eight defined functions, zero versus seven
`return_call` sites, identical member/round/useful-operation counts, control
wire body at least 128 KiB, candidate maximum body at most one quarter of the
control body, and valid modules. Directed and fixed-seed randomized initial
states must produce byte-identical architectural/data output for at least 4,096
complete invocations per variant.

Each timing sample is a fresh V8 process. Fifteen alternating paired samples
record compile, instantiate, first call, eight warm calls separated by event
loop turns, and seven fixed-work steady intervals. No V8 tier is forced. The
trace gate requires natural Liftoff and TurboFan compilation for the control
and all eight candidate bodies. Control spread may not exceed `1.10x`.

The candidate advances only if steady fixed-work paired speedup is at least
`1.03x`, its paired-bootstrap lower 95% bound is at least `1.00x`, and the
fixed-work throughput result agrees. The `1.03x` local floor is the rounded
requirement for a generated-execution mechanism to produce 1% whole Compile at
R088's `40.684%` generated share. Compile/instantiate/first-call costs are
reported and cannot be omitted from a later product protocol.

## Gate C: optimized-native fidelity

The model's native shape is part of admission, not an explanation added after
timing. Run exactly one additional fixed long-work perf collection per variant
at `cycles:u`, 1999 Hz, after the same natural warmup. Use the validated R110
JIT-dump reader; exclude perf elapsed time. Require at least 2,000 mapped model
samples and at least 90% TurboFan period in each variant.

The control is representative only if its TurboFan period-weighted native
frame lies in `[256,512]` bytes and explicit `%rbp`/`%rsp` memory operations
own `[15%,30%]` of sampled body cycles. The candidate must reduce both:

- period-weighted frame bytes to at most 80% of control; and
- explicit native-stack cycle share to at most 85% of control.

All sample-to-load, role, tier, and family periods must close exactly. If the
control fidelity window or either candidate reduction fails, stop without
changing rounds, locals, dirty count, member count, function count, warmup, or
sampling duration.

## Decision after the model

Passing Gates A--C admits one separately frozen product implementation of the
exact rule above. It does not promote model code or establish a product gain.
The product protocol must begin from exact `d9f686a9...`, preserve a default-off
causal switch until correctness passes, charge R107's real construction debit,
use 15 fixed native pairs and R104's verified-1% target/protected rules, and
then escalate through natural Chrome execution/construction clocks, qualified
long WANIX `/shared/bench.py`, and the untouched three-way scorecard.

Any failed gate closes this exact partition rule. No alternate 16/48/64 member
cap, 16/20/28/32-value union, SCC splitting, memory/global/carried-state ABI,
forced tier, or selected graph/workload variant follows from R111 results.

## Gate A result

The authoritative static report is
`target/bench/r111-partition-model/opportunity-c.json`, SHA-256
`31eb9bf4cb629e51aea61d42d2d78237189bd4011b04716e1585da3d52bd0156`.
Two fresh 336-module corpus generations (`real-region-c` and
`real-region-d`) have byte-identical manifests and member-shape files. Three
independent analyzer outputs (`opportunity-a`, `opportunity-b`, and
`opportunity-c`) are byte-identical. The earlier `real-region-a` and
`real-region-b` directories are preparatory five-mode captures; they are not
the authoritative six-mode result.

The frozen rule found real local pressure reduction:

| Gate-A measure | Result | Limit | Decision |
| --- | ---: | ---: | --- |
| eligible eager-module bytes split | `91.4146%` | `>=75%` | pass |
| byte-weighted partition state ratio | `62.9014%` | `<=80%` | pass |
| byte-weighted maximum-local reduction | `16.0587%` | `>=15%` | pass |
| Boot static edges cut | `52.8772%` | `<=12.5%` | fail |
| Compile static edges cut | `42.6839%` | `<=12.5%` | fail |
| eligible bytes containing an oversized SCC | `60.9966%` | `<=20%` | fail |

The production corpus contains all 15 Boot graphs (3,721 nodes / 4,779
edges) and all 118 Compile graphs (11,042 nodes / 12,862 edges). The rule cuts
2,527 Boot edges and 5,490 Compile edges. The real corpus contains the frozen
56 compiler-produced regions / 6,258 members / 18,626,745 eager bytes; 35
regions split and 10,386,244 otherwise eligible bytes contain an oversized
atomic SCC.

This is a prospective rejection, not a timing judgment. The rule reduces
estimated locals, but converts far too much internal control flow into
state/function boundaries, while large indivisible loops defeat its resource
cap for most eligible bytes. Gate A therefore stops R111 before constructing
or timing Gate B, collecting Gate C native samples, or editing product code.
No alternative cap, order, SCC split, ABI, or selected corpus follows from the
observed result. The release product remains exact `d9f686a9...` and earns no
scorecard credit from R111.
