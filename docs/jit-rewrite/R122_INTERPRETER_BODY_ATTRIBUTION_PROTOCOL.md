# R122 Current-Product Interpreter-Body Attribution Protocol

Date: 2026-08-10  
Status: frozen diagnostic; no product candidate and no performance credit

## Question

Which architecture-wide operation inside the exact current `Cpu::step` body
owns enough *removable* modern-Boot cost to support a verified 1% end-to-end
gain, after excluding mechanisms already closed by R058, R105, R118, and R119?

R119's immutable uninstrumented native capture assigns 19.6469% of main-thread
period to the compressed execution body and 16.3902% to the 32-bit body and
exit. Those broad ranges combine semantic execution, register writeback, PC
materialization, the Rust result ABI, retirement, and uncommon exits. R122
must separate those operations before another product edit.

## Immutable uninstrumented inputs

- product Wasm:
  `d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d`;
- core source:
  `aec4b31434a6c3940802463954ee8117313a2dda7c5d98a4d99732aa70a0c2ce`;
- Wasm source:
  `1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339`;
- loader:
  `2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385`;
- R119 perf data:
  `target/bench/r119-existing-probe-opportunity/perf.jitted.data`, SHA-256
  `24028310d77f5f8c5be6e5cb560e20d8c051b010a5b9fe078cc41fa73a691a2c`;
- optimized `Cpu::step` DSO:
  `target/bench/r119-existing-probe-opportunity/jitted-384587-2419.so`,
  SHA-256
  `5c712b022f0808b58a7b366209a951a5e1bfdd3922ab625f6f5a5d11a62b90b6`;
  and
- modern guest contract: Linux 6.12.7, Alpine 3.24.1, riscv64, production page
  policy, and public one-slice-per-turn cadence.

Perf and V8 JIT logging perturb execution. R122 reuses the existing capture,
never reports its elapsed time, and gives it no promotion credit.

## Stage A: exact native closure

One deterministic analyzer must parse every sample and the immutable native
body. It must close all `Cpu::step` period into these frozen half-open bands,
expressed as offsets from the optimized symbol:

| Begin | End | Meaning |
|---:|---:|---|
| `0x0000` | `0x0091` | entry and execute-TLB |
| `0x0091` | `0x018b` | physical-bus fetch |
| `0x018b` | `0x0250` | instruction length and RVC dispatch |
| `0x0250` | `0x165c` | compressed semantic body |
| `0x165c` | `0x1664` | compressed architectural-PC store |
| `0x1664` | `0x1675` | compressed common `Ok(None)` result stores |
| `0x1675` | `0x1689` | compressed retirement load/add/store |
| `0x1689` | `0x1698` | compressed return sequence |
| `0x1698` | `0x3e17` | 32-bit semantic body |
| `0x3e17` | `0x3e22` | 32-bit architectural-PC store |
| `0x3e22` | `0x3e33` | 32-bit common `Ok(None)` result stores |
| `0x3e33` | `0x3e47` | 32-bit retirement load/add/store |
| `0x3e47` | `0x3e57` | 32-bit return sequence |
| `0x3e57` | symbol end | uncommon exits, traps, and embedded tables |

The analyzer also emits the top exact instruction PCs and native basic blocks
within the two semantic bodies. Native samples can skid; block names are
therefore attribution evidence, not source-level causal timing.

## Stage B: one count-only diagnostic build

Only if Stage A leaves a semantic-body family with plausible leverage, one
instrumented product build may count:

1. all 24 compressed quadrant/funct3 families and all 128 32-bit opcodes;
2. calls to the architecture-wide GPR write helper and writes discarded for
   `rd == x0`;
3. ordinary sequential versus non-sequential successful retirement; and
4. exact closure against interpreter retirement for the measured Boot phase.

Counters may perturb native code and elapsed time. Exactly one modern Boot is
allowed, elapsed fields are excluded, and no counter result may be interpreted
as a speedup. Archive the instrumented source/artifact/report, remove all hot
counters, and reproduce every immutable product identity above before any
candidate decision.

## Admission rule

R122 admits at most one architecture-general product candidate. Before its
implementation the record must name the exact common operation it removes and
show both:

- at least 1.25% of uninstrumented main-thread period in a complete native
  band or closed set of semantically identical native blocks; and
- a realistic removal fraction and added-cost accounting that projects at
  least a 1.01x whole-Boot gain after construction debit.

Do not combine unrelated leaves merely to cross the floor. Guest PC, symbol,
binary, workload, compiler output, opcode frequency, engine identity, or an
observed favorable result may not select the candidate.

The following are not independent candidates:

- compact scalar `Cpu::step` results or sidecars (R058);
- the complete integrated scalar interpreter loop or a post-result family
  subset (R105);
- flat/frequency-reordered compressed selection (R118); and
- execute-TLB fetch capabilities or another fetch cache (R119).

An operation already closed by those experiments is reported and subtracted,
not reopened. If no independent operation clears admission, R122 closes with
no product edit.

## Product sequence if admitted

Freeze the unique implementation, control/candidate artifacts, construction
protocol, target row, protected rows, and 15-pair rule before timing. Then run,
in order:

1. deterministic shape and focused/exhaustive correctness;
2. the complete strict suite and direct/OpenSBI Linux 6.12.7;
3. R107 construction debit;
4. 15 alternating native Boot/Compile/Python pairs under R104;
5. Chromium Boot when affected;
6. qualified WANIX including `python /shared/bench.py`; and
7. the untouched 117-trial legacy/rewrite/copy-v86 scorecard.

Promotion requires a target median at least `1.01x`, 95% lower bound at least
parity, agreeing normalized work, protected medians at least `0.99x` without
confidence evidence of regression, complete correctness, 13/13 versus legacy,
at least 11/13 versus copy/v86, and no `/shared/bench.py` regression. Source or
Wasm byte growth is diagnostic only.
