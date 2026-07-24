// FULL PERFORMANCE SCORECARD — rv64.js vs copy/v86, one command.
//
// Runs the whole benchmark suite (system-mode, the only fair basis vs v86 —
// v86 has no user mode) and prints ONE table, then writes a timestamped
// scorecard-<ts>.md + .json so before/after perf work is directly comparable.
//
//   ARTIFACTS=<artifacts> nix develop -c node tests/vs-v86/scorecard.mjs
//   FULL=1     include interpreter columns + JIT-over-interp (slow)
//   NBENCH=1   include the BYTEmark suite, rv64 vs v86 (~8 min)
//   SKIP_V86=1 rv64 only
//
// Rows: ALU / Mixed / Boot / python fib(30) / compile (tcc -c), all rv64-JIT vs
// v86-JIT; plus the NBENCH=1 BYTEmark table. compile + nbench run the SAME source
// on both sides (w.c through tcc@d9d02c5; nbench-byte-2.2.3), one build per ISA.
// Artifacts (build once with setup.sh; DEBIAN=1 for python + v86 compile/nbench):
// $ARTIFACTS/xbench/*, root-nbench.bin, cc-bench.img, deb-riscv64.ext4,
// vmlinuz-i386, deb-i386.cpio.gz, deb-i386-bench.cpio.gz, and a built copy/v86
// checkout at $ARTIFACTS/v86.
import { readFile, writeFile, copyFile, access } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
if (!ARTIFACTS) { console.error("set ARTIFACTS=<artifacts dir> (see setup.sh)"); process.exit(2); }
const V86DIR = process.env.V86DIR || join(ARTIFACTS, "v86");
const FULL = !!+process.env.FULL;
const WANT_NBENCH = !!+process.env.NBENCH;
const WANT_V86 = !+process.env.SKIP_V86;

const { RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const bbl = new Uint8Array(await readFile(join(root, "web/images/bbl64.bin")));
const kern = new Uint8Array(await readFile(join(root, "web/images/kernel-riscv64.bin")));
const enc = new TextEncoder();
const step = (vm, n) => { for (let i = 0; i < n; i++) vm.runSystem(2_000_000n); };
const has = async (p) => { try { await access(p); return true; } catch { return false; } };

// ---------- rv64: boot buildroot, inject a freestanding binary, time it ----------
async function rvComputeBoot(jit, binPath) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  let out = ""; vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios: bbl, kernel: kern, disk: (new Uint8Array(await readFile(join(root, "web/images/root-riscv64.bin")))).slice() });
  for (let i = 0; i < 40000 && !out.includes("~ #"); i++) vm.runSystem(5_000_000n);
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n")); step(vm, 3000);
  const b64 = Buffer.from(await readFile(binPath)).toString("base64");
  vm.consoleInput(enc.encode(": > /tmp/b\n")); step(vm, 1500);
  for (let o = 0; o < b64.length; o += 512) { vm.consoleInput(enc.encode(`printf %s '${b64.slice(o, o + 512)}' >> /tmp/b\n`)); step(vm, 3000); }
  vm.consoleInput(enc.encode("base64 -d /tmp/b > /tmp/c && chmod 755 /tmp/c\n")); step(vm, 12000);
  const get = { vm, get out() { return out; }, set out(v) { out = v; } };
  return get;
}
function rvRunBench(st) {
  st.out = ""; st.vm.consoleInput(enc.encode("/tmp/c\n"));
  const t0 = performance.now(); let ts = null, td = null;
  for (let i = 0; i < 2_000_000; i++) {
    st.vm.runSystem(5_000_000n);
    if (ts === null && st.out.includes("BENCH_START")) ts = performance.now();
    if (st.out.includes("BENCH_DONE")) { td = performance.now(); break; }
    if (performance.now() - t0 > 200000) break;
  }
  return ts && td ? td - ts : null;
}
async function rvBootTime(jit) {
  const vm = await RV64.create(wasm); vm.ex.jit_set_enabled(jit ? 1 : 0);
  let out = ""; vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  const t = performance.now();
  vm.bootLinux({ bios: bbl, kernel: kern, disk: (new Uint8Array(await readFile(join(root, "web/images/root-riscv64.bin")))).slice() });
  for (let i = 0; i < 200000 && !out.includes("~ #"); i++) vm.runSystem(2_000_000n);
  return out.includes("~ #") ? performance.now() - t : null;
}
async function rvPython(jit) {
  const disk = new Uint8Array(await readFile(join(ARTIFACTS, "deb-riscv64.ext4")));
  const vm = await RV64.create(wasm); vm.ex.jit_set_enabled(jit ? 1 : 0); vm.ex.sys_set_wallclock(1);
  let out = ""; vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios: bbl, kernel: kern, disk: disk.slice(), cmdline: "console=hvc0 root=/dev/vda rw init=/binit.sh", ramMB: 512 });
  for (let i = 0; i < 400000 && !out.includes("BENCH_READY"); i++) vm.runSystem(3_000_000n);
  out = ""; vm.consoleInput(enc.encode("/usr/bin/python3 /fib.py\n"));
  const t = performance.now(); let ts = null, td = null;
  for (let i = 0; i < 6_000_000; i++) {
    vm.runSystem(4_000_000n);
    if (ts === null && out.includes("FIB_START")) ts = performance.now();
    if (out.includes("FIB_DONE")) { td = performance.now(); break; }
    if (performance.now() - t > 300000) break;
  }
  return ts && td ? td - ts : null;
}
async function rvNbench(jit) {
  const disk = new Uint8Array(await readFile(join(ARTIFACTS, "root-nbench.bin")));
  const vm = await RV64.create(wasm); vm.ex.jit_set_enabled(jit ? 1 : 0); vm.ex.sys_set_wallclock(1);
  let out = ""; vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios: bbl, kernel: kern, disk: disk.slice() });
  for (let i = 0; i < 60000 && !out.includes("~ #"); i++) vm.runSystem(2_000_000n);
  out = ""; vm.consoleInput(enc.encode("cd / && ./nbench\n"));
  const t = performance.now();
  for (let i = 0; i < 40_000_000; i++) { vm.runSystem(4_000_000n); if (out.includes("Trademarks")) break; if (performance.now() - t > 240000) break; }
  const rows = {};
  for (const m of out.matchAll(/^([A-Z][A-Z ]+?)\s+:\s+([\d.e+]+)\s+:/gm)) rows[m[1].trim()] = +m[2];
  return rows;
}
// compile benchmark: boot the buildroot image with our riscv64 tcc + w.c, time
// `tcc -c /w.c` (the SAME source + tcc commit as the v86 side). Wall-clock ms.
async function rvCompile(jit) {
  const disk = new Uint8Array(await readFile(join(ARTIFACTS, "cc-bench.img")));
  const vm = await RV64.create(wasm); vm.ex.jit_set_enabled(jit ? 1 : 0);
  let out = ""; vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios: bbl, kernel: kern, disk: disk.slice() });
  for (let i = 0; i < 40000 && !out.includes("~ #"); i++) vm.runSystem(5_000_000n);
  if (!out.includes("~ #")) return null;
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n")); step(vm, 3000);
  out = ""; vm.consoleInput(enc.encode("/tcc -c /w.c -o /w.o 2>/tmp/e; echo RC=$?; md5sum /w.o\n"));
  const t = performance.now(); let done = false;
  for (let i = 0; i < 2_000_000 && !done; i++) {
    vm.runSystem(5_000_000n);
    if (/RC=/.test(out) && (/[0-9a-f]{32}/.test(out) || /RC=[1-9]/.test(out))) done = true;
    if (performance.now() - t > 300000) break;
  }
  return /RC=0/.test(out) ? performance.now() - t : null;
}

// ---------- v86: spawn its runners in the checkout, parse RESULT ----------
function v86Spawn(script, env) {
  return new Promise((resolve) => {
    const p = spawn("node", ["--max-old-space-size=4096", script], { cwd: V86DIR, env: { ...process.env, ARTIFACTS, ...env } });
    let buf = ""; p.stdout.on("data", (d) => (buf += d));
    p.on("close", () => { const m = buf.match(/RESULT ms=(\d+)/); resolve(m ? +m[1] : null); });
  });
}
const v86Compute = (bin, jit) => v86Spawn("v86-compute.mjs", { BIN: bin, DISABLE_JIT: jit ? "0" : "1" });
const v86Boot = (jit) => v86Spawn("v86-boottime.mjs", { DISABLE_JIT: jit ? "0" : "1" });
const v86Python = (jit) => v86Spawn("deb-v86.mjs", { DISABLE_JIT: jit ? "0" : "1" });
const v86Compile = (jit) => v86Spawn("v86-compile.mjs", { DISABLE_JIT: jit ? "0" : "1" });
// v86 nbench: parse its self-timed per-kernel iterations/sec from the raw output.
function v86Nbench() {
  return new Promise((resolve) => {
    const p = spawn("node", ["--max-old-space-size=4096", "v86-nbench.mjs"], { cwd: V86DIR, env: { ...process.env, ARTIFACTS } });
    let buf = ""; p.stdout.on("data", (d) => (buf += d));
    p.on("close", () => {
      const rows = {};
      for (const m of buf.matchAll(/^([A-Z][A-Z ]+?)\s+:\s+([\d.e+]+)\s+:/gm)) rows[m[1].trim()] = +m[2];
      resolve(rows);
    });
  });
}

// copy v86 runners into the checkout (relative ./src, ./bios resolve there)
let haveV86 = WANT_V86;
if (haveV86 && (await has(join(V86DIR, "src/main.js")))) {
  for (const f of ["v86-compute.mjs", "v86-boottime.mjs", "deb-v86.mjs", "v86-compile.mjs", "v86-nbench.mjs"])
    if (await has(join(root, "tests/vs-v86", f))) await copyFile(join(root, "tests/vs-v86", f), join(V86DIR, f));
} else haveV86 = false;

// ---------- run ----------
const R = {}; // name -> {rvj, rvi, v8j, v8i}
const log = (m) => process.stderr.write(m);

// compute (alu, mixed): one boot per JIT setting, run both binaries
for (const jit of FULL ? [false, true] : [true]) {
  log(`[rv64 compute jit=${+jit}] boot…`);
  const st = await rvComputeBoot(jit, join(ARTIFACTS, "xbench", "alu.rv64"));
  (R.ALU ??= {})[jit ? "rvj" : "rvi"] = rvRunBench(st); log(" alu");
  // reuse boot: swap /tmp/c to the mixed binary
  const b64 = Buffer.from(await readFile(join(ARTIFACTS, "xbench", "rvbench_fs.rv64"))).toString("base64");
  st.vm.consoleInput(enc.encode(": > /tmp/b\n")); step(st.vm, 1500);
  for (let o = 0; o < b64.length; o += 512) { st.vm.consoleInput(enc.encode(`printf %s '${b64.slice(o, o + 512)}' >> /tmp/b\n`)); step(st.vm, 3000); }
  st.vm.consoleInput(enc.encode("base64 -d /tmp/b > /tmp/c && chmod 755 /tmp/c\n")); step(st.vm, 12000);
  (R.Mixed ??= {})[jit ? "rvj" : "rvi"] = rvRunBench(st); log(" mixed\n");
}
// boot time
for (const jit of FULL ? [false, true] : [true]) { log(`[rv64 boot jit=${+jit}]…`); (R.Boot ??= {})[jit ? "rvj" : "rvi"] = await rvBootTime(jit); log(" ok\n"); }
// python (needs the debian image)
if (await has(join(ARTIFACTS, "deb-riscv64.ext4"))) for (const jit of FULL ? [false, true] : [true]) { log(`[rv64 python jit=${+jit}]…`); (R["python fib(30)"] ??= {})[jit ? "rvj" : "rvi"] = await rvPython(jit); log(" ok\n"); }
// compile (needs the cc-bench image; same w.c + tcc commit as v86)
if (await has(join(ARTIFACTS, "cc-bench.img"))) for (const jit of FULL ? [false, true] : [true]) { log(`[rv64 compile jit=${+jit}]…`); (R["compile (tcc -c)"] ??= {})[jit ? "rvj" : "rvi"] = await rvCompile(jit); log(" ok\n"); }

// v86 side
if (haveV86) {
  for (const jit of FULL ? [false, true] : [true]) {
    log(`[v86 jit=${+jit}] alu`); (R.ALU ??= {})[jit ? "v8j" : "v8i"] = await v86Compute("alu.i386", jit);
    log(" mixed"); (R.Mixed ??= {})[jit ? "v8j" : "v8i"] = await v86Compute("rvbench_fs.i386", jit);
    log(" boot"); (R.Boot ??= {})[jit ? "v8j" : "v8i"] = await v86Boot(jit);
    if (await has(join(ARTIFACTS, "vmlinuz-i386"))) { log(" python"); (R["python fib(30)"] ??= {})[jit ? "v8j" : "v8i"] = await v86Python(jit); }
    if (await has(join(ARTIFACTS, "deb-i386-bench.cpio.gz"))) { log(" compile"); (R["compile (tcc -c)"] ??= {})[jit ? "v8j" : "v8i"] = await v86Compile(jit); }
    log("\n");
  }
}
// nbench (BYTEmark) — rv64 JIT vs v86 JIT, both self-timed; gated (slow, ~8 min)
let nb = null;
if (WANT_NBENCH && (await has(join(ARTIFACTS, "root-nbench.bin")))) {
  log("[rv64 nbench jit]…"); const nj = await rvNbench(true);
  let ni = null; if (FULL) { log(" interp…"); ni = await rvNbench(false); }
  let v8 = null;
  if (haveV86 && (await has(join(ARTIFACTS, "deb-i386-bench.cpio.gz")))) { log(" v86…"); v8 = await v86Nbench(); }
  log(" ok\n");
  nb = { jit: nj, int: ni, v8 };
}

// ---------- render ----------
const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const ms = (x) => (x == null ? "—" : `${Math.round(x)}ms`);
const ratio = (r) => (r?.rvj && r?.v8j ? (r.rvj / r.v8j).toFixed(2) + "× " + (r.rvj < r.v8j ? "rv64" : "v86") : "—");
const speedup = (r) => (r?.rvi && r?.rvj ? (r.rvi / r.rvj).toFixed(1) + "×" : "—");
const order = ["ALU", "Mixed", "Boot", "python fib(30)", "compile (tcc -c)"];
let md = `# rv64.js vs v86 — performance scorecard\n\n_${ts}, system-mode, host wall-clock. Ratio = rv64 JIT vs v86 JIT (lower rv64 = rv64 faster)._\n\n`;
md += `| Benchmark | rv64 interp | rv64 JIT | v86 interp | v86 JIT | rv64 JIT/interp | rv64 vs v86 JIT |\n|---|--:|--:|--:|--:|--:|--:|\n`;
for (const k of order) { const r = R[k]; if (!r) continue; md += `| ${k} | ${ms(r.rvi)} | ${ms(r.rvj)} | ${ms(r.v8i)} | ${ms(r.v8j)} | ${speedup(r)} | ${ratio(r)} |\n`; }
if (nb) {
  // BYTEmark self-times iterations/sec (higher = faster); compare rv64 vs v86.
  const nr = (a, b) => (a && b ? (a / b).toFixed(2) + "× " + (a > b ? "rv64" : "v86") : "—");
  md += `\n**nbench (BYTEmark, iterations/sec, higher=better; ratio = rv64 JIT vs v86 JIT):**\n\n`;
  md += `| Kernel | rv64 JIT | v86 JIT | rv64 vs v86 |${nb.int ? " rv64 interp |" : ""}\n|---|--:|--:|--:|${nb.int ? "--:|" : ""}\n`;
  for (const k of Object.keys(nb.jit)) {
    const rj = nb.jit[k], vj = nb.v8?.[k];
    md += `| ${k} | ${rj ?? "—"} | ${vj ?? "—"} | ${nr(rj, vj)} |${nb.int ? ` ${nb.int[k] ?? "—"} |` : ""}\n`;
  }
}
console.log("\n" + md);
await writeFile(join(ARTIFACTS, `scorecard-${ts}.md`), md);
await writeFile(join(ARTIFACTS, `scorecard-${ts}.json`), JSON.stringify({ ts, results: R, nbench: nb }, null, 2));
console.log(`saved ${join(ARTIFACTS, `scorecard-${ts}.md`)} (+ .json)`);
