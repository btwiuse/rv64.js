# Performance baseline (pre perf-trilogy)

Recorded 2026-07-22, commit after validation roadmap completion.
Methodology: `nix develop -c node tests/bench.mjs` — best of 3 runs, fresh
wasm instance per run (cold V8 per rep; consistent across measurements).
Host: WSL2, Node 20 (nix flake).

| workload      |     ms | Minsn/s | jit% | dispatches | blocks |
|---------------|--------|---------|------|------------|--------|
| user-int+fp   | 1676.7 |   137.2 | 41.2 |   10711256 |     11 |
| boot          |    662 |    69.1 |  0.0 |       1053 |      0 |
| sys-shell     |   79.8 |    62.6 |  0.0 |       1057 |      4 |

Column notes: `jit%` = share of guest instructions retired inside JIT
blocks; `blocks` = live compiled blocks at end of run.

## Baseline diagnosis (what the trilogy must fix)

1. **System-mode JIT coverage is ~0%.** Two compounding causes:
   - Memory ops end system blocks (ALU/branch-only), so kernel/userland
     blocks are tiny and hot loops with loads/stores never tier up.
     → roadmap item 1: inline-TLB memory ops.
   - Invalidation is clear-everything: any store to any compiled page
     wipes the whole cache. Boot-time kernel code patching and program
     loading into recycled pages cause constant wipes (dispatches happen,
     but the cache ends empty).
     → fix alongside item 1: per-page invalidation.
2. **User-mode JIT coverage is only 41%** on int+fp: the FP phase runs
   fully interpreted because FP instructions end blocks.
   → roadmap item 3: FP ops inside blocks.
3. **Dispatch overhead**: 10.7M dispatches for the user workload — one
   HashMap lookup + call_indirect per ~9 retired instructions.
   → roadmap item 2: in-wasm chaining/dispatch.
