// Fast, load-robust JIT delta harness. Interleaves JIT/interp so both sample
// the same load window; reports best-of-N of each + the (load-robust) speedup
// ratio, and cross-checks JIT output == interp output.
// Usage: SC=<dir> node tests/vs-v86/bench-jit-fast.mjs
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { RV64, Stop } = await import(join(root, "web/rv64.js"));
const wasm = new Uint8Array(await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm")));
const SC = process.env.SC;
const WL = [{ name: "alu", elf: SC + "/xbench/alu_s.rv64" },
           { name: "mixed", elf: SC + "/xbench/rvbench_s.rv64" }];
const REPS = 3;
async function run1(bytes, jitOn) {
  const vm = await RV64.create(wasm); let out = "";
  vm.onWrite = (fd, b) => { out += new TextDecoder().decode(b); };
  vm.ex.jit_set_enabled(jitOn ? 1 : 0);
  vm.loadElf(bytes, ["b"]);
  const t0 = performance.now();
  let s, g = 0; do { s = vm.runUser(50_000_000_000n); } while (s !== Stop.EXITED && ++g < 40);
  return { ms: performance.now() - t0, insns: Number(vm.userInsnCount()),
           cov: jitOn ? Number(vm.ex.jit_stat(0)) / Number(vm.userInsnCount()) * 100 : 0,
           out: out.match(/checksum=0x[0-9a-f]+|isum=\d+/)?.[0] ?? "?" };
}
for (const w of WL) {
  const bytes = new Uint8Array(await readFile(w.elf));
  const jit = [], interp = [];
  let cref = null, cjit = null, cov = 0;
  for (let r = 0; r < REPS; r++) {
    const j = await run1(bytes, true);  jit.push(j.ms); cjit = j.out; cov = j.cov;
    const i = await run1(bytes, false); interp.push(i.ms); cref = i.out;
  }
  const bj = Math.min(...jit), bi = Math.min(...interp);
  const ok = cjit === cref ? "OK" : `MISMATCH(${cjit} vs ${cref})`;
  console.log(`${w.name.padEnd(6)} JIT ${bj.toFixed(0).padStart(6)}ms  interp ${bi.toFixed(0).padStart(6)}ms  speedup ${(bi/bj).toFixed(2)}x  cov=${cov.toFixed(0)}%  [${ok}]`);
}
