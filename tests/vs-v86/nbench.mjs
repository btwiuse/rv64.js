// nbench (BYTEmark) — v86's actual arch-bytemark compute benchmark — run
// inside our full-system riscv64 Linux, JIT vs interpreter, with the wall-clock
// time source enabled (sys_set_wallclock) so nbench's self-timed index scores
// reflect REAL throughput (our default clock is instruction-counted, which
// would make the scores identical JIT-vs-interp — see the clock note in README).
//
// Setup (see README "nbench" section): build nbench for riscv64 with the
// newlib cross-gcc and bake it into a copy of the ext2 rootfs via debugfs, at
// /nbench. Point ROOT_NBENCH at that image.
//
//   ARTIFACTS=<scratchpad> ROOT_NBENCH=<rootfs-with-nbench> node tests/vs-v86/nbench.mjs
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const disk = process.env.ROOT_NBENCH || join(process.env.ARTIFACTS || "", "root-nbench.bin");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const img = (f) => readFile(f).then((b) => new Uint8Array(b));
const [bios, kernel, diskImg] = await Promise.all([
  img(join(root, "web/images/bbl64.bin")),
  img(join(root, "web/images/kernel-riscv64.bin")),
  img(disk),
]);
const enc = new TextEncoder();
const CAP_MS = +(process.env.CAP_MS || 240000);

async function run(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.sys_set_wallclock(1); // real-time clock so self-timing is meaningful
  let out = "";
  vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios, kernel, disk: diskImg.slice() });
  for (let i = 0; i < 60000 && !out.includes("~ #"); i++) vm.runSystem(2_000_000n);
  out = "";
  vm.consoleInput(enc.encode("cd / && ./nbench\n"));
  const t = performance.now();
  for (let i = 0; i < 40_000_000; i++) {
    vm.runSystem(4_000_000n);
    if (out.includes("Trademarks")) break;
    if (performance.now() - t > CAP_MS) break;
  }
  return { ms: performance.now() - t, out, complete: out.includes("Trademarks") };
}

for (const jit of [false, true]) {
  const r = await run(jit);
  const rows = r.out.split("\n").filter((l) => /:.*:.*:/.test(l) || /INDEX/.test(l));
  console.log(`\n===== ${jit ? "JIT" : "INTERP"}  (${(r.ms / 1000).toFixed(1)}s, ${r.complete ? "COMPLETE" : "PARTIAL"}) =====`);
  console.log(rows.join("\n"));
}
