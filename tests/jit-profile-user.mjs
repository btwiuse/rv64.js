// Deterministic user-mode JIT attribution. Unlike tests/bench.mjs this runs
// once, enables exact (unsampled) dispatch profiling, and reports the PCs and
// unsupported instruction classes that account for host dispatch/fallback.
//
// Run: node tests/jit-profile-user.mjs [fast|soft]
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const mode = process.argv[2] ?? "fast";
const [{ RV64Debug: RV64 }, wasmBytes, elfBytes] = await Promise.all([
  import(join(root, "web/rv64.js")),
  readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm")),
  readFile(
    join(root, "guests/bench/target/riscv64gc-unknown-linux-musl/release/bench"),
  ),
]);

const vm = await RV64.create(wasmBytes);
vm.onWrite = () => {};
vm.loadElf(new Uint8Array(elfBytes), ["bench", mode]);
vm.ex.dprof_set_sample_shift(0);
vm.ex.dprof_set(1);
const t0 = performance.now();
const stop = vm.runUser(2_000_000_000n);
const elapsedMs = performance.now() - t0;

const dispatch = [];
const edges = [];
for (let i = 0; i < 8192; i++) {
  const calls = Number(vm.ex.dprof_get(1, i));
  if (calls !== 0) {
    const retired = Number(vm.ex.dprof_get(2, i));
    dispatch.push({
      pc: `0x${vm.ex.dprof_get(0, i).toString(16)}`,
      calls,
      retired,
      insnsPerCall: retired / calls,
    });
  }
  const transitions = Number(vm.ex.eprof_get(2, i));
  if (transitions !== 0) {
    const retired = Number(vm.ex.eprof_get(3, i));
    edges.push({
      source: `0x${vm.ex.eprof_get(0, i).toString(16)}`,
      target: `0x${vm.ex.eprof_get(1, i).toString(16)}`,
      transitions,
      retired,
      insnsPerTransition: retired / transitions,
    });
  }
}
dispatch.sort((a, b) => b.calls - a.calls);
edges.sort((a, b) => b.transitions - a.transitions);

const fallback = [];
for (let i = 0; i < 1024; i++) {
  const stretches = Number(vm.ex.ihist_get(1, i));
  if (stretches !== 0) {
    fallback.push({
      key: `0x${vm.ex.ihist_get(0, i).toString(16)}`,
      stretches,
      interpretedInsns: Number(vm.ex.ihist_get(2, i)),
    });
  }
}
fallback.sort((a, b) => b.interpretedInsns - a.interpretedInsns);

console.log(
  JSON.stringify(
    {
      mode,
      stop,
      elapsedMs,
      guestInsns: vm.userInsnCount().toString(),
      jitInsns: vm.ex.jit_stat(0).toString(),
      dispatches: vm.ex.jit_stat(1).toString(),
      compiledBlocks: vm.ex.jit_stat(2).toString(),
      topDispatch: dispatch.slice(0, 20),
      topEdges: edges.slice(0, 20),
      fallback: fallback.slice(0, 20),
    },
    null,
    2,
  ),
);
