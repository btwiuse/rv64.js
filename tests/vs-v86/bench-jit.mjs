// rv64.js wasm-JIT quick check, tracked against fixed v86 reference numbers.
// Best-of-3, rv64 side only. For a full fresh both-sides comparison (runs v86
// live too) use compare.mjs — that's the canonical harness; this is the fast
// rv64-only spot check.
// Usage: SC=<dir-with-alu.rv64,rvbench.rv64> node tests/vs-v86/bench-jit.mjs
// Reports per workload: JIT ms, Minsn/s, JIT coverage, and ratio vs v86's
// same-machine JIT (measured by compare.mjs 2026-07-23: ALU 2816ms, mixed
// 1421ms). Ratio <1 = rv64 faster than v86.
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { RV64, Stop } = await import(join(root, "web/rv64.js"));
const wasm = new Uint8Array(await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm")));
const SC = process.env.SC;
const WL = [
  { name: "alu",   elf: SC + "/xbench/alu.rv64",     v86: 2816 },
  { name: "mixed", elf: SC + "/xbench/rvbench.rv64", v86: 1421 },
];
const REPS = 3;
async function run1(elf, jitOn) {
  const vm = await RV64.create(wasm);
  vm.onWrite = () => {};
  vm.ex.jit_set_enabled(jitOn ? 1 : 0);
  vm.loadElf(new Uint8Array(await readFile(elf)), ["b"]);
  const t0 = performance.now();
  let s, g = 0; do { s = vm.runUser(50_000_000_000n); } while (s !== Stop.EXITED && ++g < 40);
  const ms = performance.now() - t0, insns = Number(vm.userInsnCount());
  return { ms, insns, exit: vm.userExitCode(),
           cov: jitOn ? Number(vm.ex.jit_stat(0)) / insns * 100 : 0 };
}
for (const w of WL) {
  const runs = [];
  for (let r = 0; r < REPS; r++) runs.push(await run1(w.elf, true));
  const b = runs.reduce((a, c) => c.ms < a.ms ? c : a);
  const interp = await run1(w.elf, false);
  console.log(`${w.name.padEnd(6)}  JIT ${b.ms.toFixed(0).padStart(7)}ms  ${(b.insns/b.ms/1000).toFixed(0).padStart(5)} Minsn/s  cov=${b.cov.toFixed(0).padStart(3)}%  | interp ${interp.ms.toFixed(0)}ms (jit ${(interp.ms/b.ms).toFixed(2)}x) | v86 ${w.v86}ms  => ${(b.ms/w.v86).toFixed(1)}x v86  exit=${b.exit}`);
}
