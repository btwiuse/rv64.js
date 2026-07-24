// arch-python (v86's python fib(30) benchmark) — APPLES-TO-APPLES: the SAME
// Debian trixie python 3.13 on both emulators, JIT vs interpreter.
//   rv64: boots the Debian riscv64 rootfs under our JIT machine (virtio-blk).
//   v86 : boots the Debian i386 rootfs as an initramfs under a stock i386
//         kernel (v86 has only IDE and its buildroot kernel lacks ATA).
//
// Setup:
//   nix develop -c tests/vs-v86/mk-debian-rootfs.sh <out>              # riscv64
//   ARCH=i386 nix develop -c tests/vs-v86/mk-debian-rootfs.sh <out>    # i386
//   nix develop -c tests/vs-v86/mk-v86-debian.sh <out>                 # v86 kernel+initramfs
//   ARTIFACTS=<out> V86DIR=<out>/v86 nix develop -c node tests/vs-v86/compare-python.mjs
import { readFile, copyFile, access } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
if (!ARTIFACTS) { console.error("set ARTIFACTS=<artifacts dir>"); process.exit(2); }
const V86DIR = process.env.V86DIR || join(ARTIFACTS, "v86");
const RUN_INTERP = !+process.env.SKIP_INTERP;

const { RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const imgs = await Promise.all(
  ["bbl64.bin", "kernel-riscv64.bin"].map(async (f) => new Uint8Array(await readFile(join(root, "web/images", f)))),
);
const rvDisk = new Uint8Array(await readFile(join(ARTIFACTS, "deb-riscv64.ext4")));
const enc = new TextEncoder();

async function rvRun(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.sys_set_wallclock(1);
  let out = "";
  vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios: imgs[0], kernel: imgs[1], disk: rvDisk.slice(), cmdline: "console=hvc0 root=/dev/vda rw init=/binit.sh", ramMB: 512 });
  for (let i = 0; i < 400000 && !out.includes("BENCH_READY"); i++) vm.runSystem(3_000_000n);
  out = "";
  vm.consoleInput(enc.encode("/usr/bin/python3 /fib.py\n"));
  const t = performance.now(); let ts = null, td = null;
  for (let i = 0; i < 6_000_000; i++) {
    vm.runSystem(4_000_000n);
    if (ts === null && out.includes("FIB_START")) ts = performance.now();
    if (out.includes("FIB_DONE")) { td = performance.now(); break; }
    if (performance.now() - t > 300000) break;
  }
  return { ms: ts && td ? td - ts : null, chk: (out.match(/fib\(30\)=\s*\d+/) || ["?"])[0].replace(/\s/g, "") };
}

function v86Run(jit) {
  return new Promise((resolve) => {
    const p = spawn("node", ["--max-old-space-size=4096", "deb-v86.mjs"],
      { cwd: V86DIR, env: { ...process.env, ARTIFACTS, DISABLE_JIT: jit ? "0" : "1" } });
    let buf = ""; p.stdout.on("data", (d) => (buf += d));
    p.on("close", () => { const m = buf.match(/RESULT ms=(\d+) chk=(\S+)/); resolve(m ? { ms: +m[1], chk: m[2] } : { ms: null }); });
  });
}

let haveV86 = true;
try { await access(join(V86DIR, "src/main.js")); await copyFile(join(root, "tests/vs-v86/deb-v86.mjs"), join(V86DIR, "deb-v86.mjs")); }
catch { haveV86 = false; }

const r = {};
for (const jit of RUN_INTERP ? [false, true] : [true]) {
  process.stderr.write(`[rv64 python] jit=${jit ? 1 : 0}…`); r[`rv:${jit ? "j" : "i"}`] = await rvRun(jit); process.stderr.write(" done\n");
  if (haveV86) { process.stderr.write(`[v86 python] jit=${jit ? 1 : 0}…`); r[`v8:${jit ? "j" : "i"}`] = await v86Run(jit); process.stderr.write(" done\n"); }
}

const f = (x) => (x == null || x.ms == null ? "     —" : `${x.ms.toFixed(0).padStart(6)}ms`);
console.log(`\n# arch-python — fib(30), same Debian trixie python 3.13, both JIT (apples-to-apples)`);
console.log(`# fib compute only (FIB_START->FIB_DONE)${haveV86 ? "" : "  —  v86 NOT FOUND (rv64 only)"}\n`);
console.log("           rv64 interp   rv64 JIT      v86 interp    v86 JIT     rv64 JIT vs v86 JIT");
const rj = r["rv:j"], vj = r["v8:j"];
const ratio = rj?.ms && vj?.ms ? (rj.ms / vj.ms).toFixed(2) + "x " + (rj.ms < vj.ms ? "(rv64 faster)" : "(v86 faster)") : "—";
console.log(`fib(30)    ${f(r["rv:i"]).padStart(11)}   ${f(r["rv:j"]).padStart(8)}     ${f(r["v8:i"]).padStart(10)}   ${f(r["v8:j"]).padStart(8)}    ${ratio}`);
console.log(`\nresults (all should be 832040): rv64 ${r["rv:j"]?.chk} | v86 ${r["v8:j"]?.chk || "?"}`);
