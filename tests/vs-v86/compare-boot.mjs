// rv64.js vs copy/v86 — Linux BOOT-TIME comparison (v86's linux-boot
// benchmark). Both boot buildroot to a shell prompt; wall-clock ms, JIT on/off.
//
//   ARTIFACTS=<scratchpad> nix develop -c node tests/vs-v86/compare-boot.mjs
import { readFile, copyFile, access } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
const V86DIR = process.env.V86DIR || (ARTIFACTS && join(ARTIFACTS, "v86"));
const REPS = +(process.env.REPS || 3);

const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const imgs = await Promise.all(
  ["bbl64.bin", "kernel-riscv64.bin", "root-riscv64.bin"].map(async (f) =>
    new Uint8Array(await readFile(join(root, "web/images", f)))),
);

// Our boot: time bootLinux() -> first shell prompt.
async function rvBoot(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  let out = "";
  vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  const t0 = performance.now();
  vm.bootLinux({ bios: imgs[0], kernel: imgs[1], disk: imgs[2].slice() });
  for (let i = 0; i < 200000 && !out.includes("~ #"); i++) vm.runSystem(2_000_000n);
  return out.includes("~ #") ? performance.now() - t0 : null;
}

function v86Boot(jit) {
  return new Promise((resolve) => {
    const env = { ...process.env, DISABLE_JIT: jit ? "0" : "1" };
    const p = spawn("node", ["v86-boottime.mjs"], { cwd: V86DIR, env });
    let buf = "";
    p.stdout.on("data", (d) => (buf += d));
    p.on("close", () => {
      const m = buf.match(/RESULT ms=(\d+)/);
      resolve(m ? +m[1] : null);
    });
  });
}

const best = (a) => Math.min(...a.filter((x) => x != null));

let haveV86 = true;
try {
  await access(join(V86DIR, "src/main.js"));
  await copyFile(join(root, "tests/vs-v86/v86-boottime.mjs"), join(V86DIR, "v86-boottime.mjs"));
} catch { haveV86 = false; }

const r = {};
for (const jit of [false, true]) {
  process.stderr.write(`[rv64 boot] jit=${jit ? 1 : 0}…`);
  const a = []; for (let i = 0; i < REPS; i++) a.push(await rvBoot(jit));
  r[`rv:${jit ? "j" : "i"}`] = best(a);
  process.stderr.write(" done\n");
  if (haveV86) {
    process.stderr.write(`[v86 boot] jit=${jit ? 1 : 0}…`);
    const b = []; for (let i = 0; i < Math.min(2, REPS); i++) b.push(await v86Boot(jit));
    r[`v8:${jit ? "j" : "i"}`] = best(b);
    process.stderr.write(" done\n");
  }
}

const f = (x) => (x == null || !isFinite(x) ? "     —" : `${x.toFixed(0).padStart(6)}ms`);
console.log(`\n# rv64.js vs copy/v86 — LINUX BOOT TIME (buildroot, boot -> shell prompt)`);
console.log(`# best-of-${REPS}${haveV86 ? "" : "  —  v86 NOT FOUND (rv64 only)"}\n`);
console.log("           rv64 interp   rv64 JIT     v86 interp    v86 JIT");
console.log(`boot->sh   ${f(r["rv:i"]).padStart(11)}   ${f(r["rv:j"]).padStart(8)}    ${f(r["v8:i"]).padStart(10)}   ${f(r["v8:j"]).padStart(8)}`);
if (haveV86 && r["rv:j"] && r["v8:j"])
  console.log(`\nrv64 JIT vs v86 JIT: ${(r["rv:j"] / r["v8:j"]).toFixed(2)}x  (${r["rv:j"] < r["v8:j"] ? "rv64 faster" : "v86 faster"})`);
