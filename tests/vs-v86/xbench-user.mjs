import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
const root = "/home/darren/src/arm64.js";
const { RV64, Stop } = await import(join(root, "web/rv64.js"));
const wasm = new Uint8Array(await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm")));
const elf = new Uint8Array(await readFile(process.argv[2]));

async function run(jitOn, label) {
  const vm = await RV64.create(wasm);
  let out = "";
  vm.onWrite = (b) => { out += String.fromCharCode(b); };
  vm.ex.jit_set_enabled(jitOn ? 1 : 0);
  vm.loadElf(elf, ["rvbench"]);
  const t0 = performance.now();
  let stop, guard = 0;
  do { stop = vm.runUser(50_000_000_000n); } while (stop !== Stop.EXITED && ++guard < 20);
  const ms = performance.now() - t0;
  const insns = Number(vm.userInsnCount());
  const jitpct = ((Number(vm.ex.jit_stat(0)) / insns) * 100).toFixed(1);
  const chk = (out.match(/isum=\d+/) || ["?"])[0];
  console.log(`${label.padEnd(14)} ${ms.toFixed(0).padStart(7)}ms  ${(insns/ms/1000).toFixed(0).padStart(5)} Minsn/s  jit=${jitpct}%  exit=${vm.userExitCode()}  ${chk}`);
  return ms;
}
await run(false, "wasm INTERP");
await run(true,  "wasm JIT");
