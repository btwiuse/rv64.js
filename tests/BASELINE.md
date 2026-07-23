# Performance baseline (pre perf-trilogy)

Recorded 2026-07-22, commit after validation roadmap completion.

**All measurements run under WebAssembly (V8, via Node) — the browser
target.** The JIT exists only in the wasm build (it emits wasm modules the
JS host instantiates into the module's function table); the native build
has no JIT. Optimization goal is wasm/V8 throughput, not native.

Methodology: `nix develop -c node tests/bench.mjs` loads
`target/wasm32-unknown-unknown/release/rv64_wasm.wasm` and runs it in V8 —
best of 3 runs, fresh wasm instance per run. Host: WSL2, Node 20 (nix
flake).

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


## Update — inline-TLB system memory ops (roadmap item 1)

Implemented: full-system JIT blocks now do guest loads/stores via an inline
TLB probe (hit → direct RAM access; miss/MMIO/page-cross/self-modifying →
bail to interpreter). Plus per-page invalidation (drop only blocks on a
written code page, not the whole cache).

Correctness: 4 MB md5sum inside booted Linux hashes bit-identically to the
host (b5cfa9d6…) — a strong load/store check; full suite (arch-tests
193/193, lockstep 109/109) green.

Throughput: **flat at slice=4096** (no regression), and a slice sweep
(warm interpreter fallback 64→1024) showed every value that raises JIT
coverage *lowers* throughput:

| warm slice | boot Minsn/s | sys-md5 Minsn/s | boot jit% |
|-----------:|-------------:|----------------:|----------:|
| 4096 (base)|         69.2 |            70.5 |       1.2 |
| 1024       |         62.3 |            66.0 |       3.3 |
|  512       |         59.3 |            64.3 |       6.6 |
|   64       |         39.5 |            45.6 |      44.7 |

**Finding: dispatch cost is the bottleneck, not coverage.** Each block
dispatch pays a HashMap lookup + a pa-verify TLB walk + a call_indirect
into a JS-registered function + the retired-cell read — more per
instruction than the tight Rust interpreter costs to just run it. So item
1 is necessary infrastructure (longer blocks are now possible) but pays
off only once dispatch is cheap. **Roadmap item 2 (in-wasm block chaining
/ cheaper dispatch) is the real unlock** and is next; the slice is held at
4096 until then.
