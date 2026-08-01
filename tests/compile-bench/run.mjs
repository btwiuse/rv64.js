// Three-way compilation benchmark: NATIVE (host tcc) vs EMULATED-INTERP
// (guest tcc, JIT off) vs JIT (guest tcc, JIT on). The guest runs a real
// riscv64 tcc compiling a real C translation unit; we time `tcc -c` and
// md5sum the resulting object — the digest must match JIT-on vs JIT-off
// (proves the JIT compiles correctly) and also signals completion.
//
// Usage: node run.mjs <image.img> <guest.c> <native-ms> [reps]
//   image.img  : guest disk with /tcc and the source injected (mkimage.sh)
//   guest.c    : path inside the guest, e.g. /medium.c
//   native-ms  : host tcc time in ms (for the three-way print)
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const [imgPath, guestC, nativeMsArg, repsArg] = process.argv.slice(2);
const nativeMs = Number(nativeMsArg);
const reps = Number(repsArg || 3);
const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const img = (f) => join(root, "web/images", f);
const bios = new Uint8Array(await readFile(img("bbl64.bin")));
const kernel = new Uint8Array(await readFile(img("kernel-riscv64.bin")));
const disk = new Uint8Array(await readFile(imgPath));
const enc = new TextEncoder();

async function trial(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  let out = "";
  vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios, kernel, disk: disk.slice() });
  for (let i = 0; i < 40000 && !out.includes("~ #"); i++) vm.runSystem(5_000_000n);
  if (!out.includes("~ #")) throw new Error("boot failed");
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n"));
  for (let i = 0; i < 3000; i++) vm.runSystem(2_000_000n);
  // compile, then md5 the object (real output + correctness digest)
  out = "";
  const obj = "/out.o";
  vm.consoleInput(enc.encode(`/tcc -c ${guestC} -o ${obj} 2>/tmp/e; echo RC=$?; md5sum ${obj} 2>/dev/null\n`));
  const t = performance.now();
  let stop = false;
  for (let i = 0; i < 2_000_000 && !stop; i++) {
    vm.runSystem(5_000_000n);
    if (/RC=/.test(out) && (/[0-9a-f]{32}/.test(out) || /RC=[1-9]/.test(out))) stop = true;
    if (performance.now() - t > 600_000) break;
  }
  const ms = performance.now() - t;
  const rc = (out.match(/RC=(\d+)/) || [, "?"])[1];
  const md5 = (out.match(/([0-9a-f]{32})/) || ["<none>"])[0];
  return { ms, rc, md5 };
}

const median = (a) => [...a].sort((x, y) => x - y)[(a.length / 2) | 0];
const offs = [], ons = [];
let md5off, md5on, rc = "0";
for (let i = 0; i < reps; i++) {
  const o = await trial(false); offs.push(o.ms); md5off = o.md5; rc = o.rc;
  const n = await trial(true); ons.push(n.ms); md5on = n.md5;
}
const interp = median(offs), jit = median(ons);
const correct = md5off === md5on && /^[0-9a-f]{32}$/.test(md5off) && rc === "0";

console.log(`\ncompile ${guestC}  (object md5 ${md5off}${md5off === md5on ? " == " : " != "}${md5on})`);
console.log(`  native (host tcc):   ${nativeMs.toFixed(0)} ms`);
console.log(`  emulated (interp):   ${interp.toFixed(0)} ms   (${(interp / nativeMs).toFixed(0)}x slower than native)`);
console.log(`  emulated (JIT):      ${jit.toFixed(0)} ms   (${(interp / jit).toFixed(2)}x faster than interp; ${(jit / nativeMs).toFixed(0)}x slower than native)`);
console.log(`  correct (jit==interp, rc=0): ${correct ? "YES" : "NO"}`);
