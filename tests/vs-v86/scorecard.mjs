// FULL PERFORMANCE SCORECARD — rv64.js vs copy/v86, one command.
//
// SYSTEM EMULATION ONLY: both emulators boot a FULL Linux and run every
// benchmark inside the guest. v86 has no user mode; comparing our user mode
// against v86's system mode was a past mistake — never do it again. The bar
// (user directive): rv64 must WIN or MATCH v86 on EVERY row, including every
// individual nbench kernel. Prints ONE table with a per-row verdict and a
// pass count, then writes a timestamped scorecard-<ts>.md + .json so
// before/after perf work is directly comparable.
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
import { spawn, execSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
if (!ARTIFACTS) { console.error("set ARTIFACTS=<artifacts dir> (see setup.sh)"); process.exit(2); }
const V86DIR = process.env.V86DIR || join(ARTIFACTS, "v86");
const FULL = !!+process.env.FULL;
const WANT_NBENCH = !!+process.env.NBENCH;
const WANT_V86 = !+process.env.SKIP_V86;
const SB = !!+process.env.SB; // run rv64 with page superblocks (single config for ALL rows)
// REPS>1 repeats the wall-clock main rows and reports the MEDIAN (ISSUES.md
// P1: single fixed-order samples are not trustworthy on a noisy shared host).
// nbench self-times in-guest over MINIMUM_SECONDS and needs no repetition.
const REPS = Math.max(1, +(process.env.REPS || 1));
const median = (a) => { const v = a.filter((x) => x != null).sort((x, y) => x - y); return v.length ? v[(v.length / 2) | 0] : null; };

const { RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const bbl = new Uint8Array(await readFile(join(root, "web/images/bbl64.bin")));
const kern = new Uint8Array(await readFile(join(root, "web/images/kernel-riscv64.bin")));
const enc = new TextEncoder();
const step = (vm, n) => { for (let i = 0; i < n; i++) vm.runSystem(2_000_000n); };
// Yield to the JS event loop periodically inside run loops: async-compiled
// superblock modules resolve on the microtask queue, which a synchronous
// loop starves (v86's runners are event-driven and yield constantly — this
// also makes the two sides' host behavior symmetric).
const tick = () => new Promise((r) => setImmediate(r));
const has = async (p) => { try { await access(p); return true; } catch { return false; } };

// ---------- rv64: boot buildroot, inject a freestanding binary, time it ----------
// TIMING PROTOCOL (symmetric with the v86 runners; ISSUES.md P0): markers are
// timestamped in the onWrite STREAM callback, which fires during execution at
// interrupt-quantum granularity (~1M guest insns) — never by polling buffered
// output after a runSystem slice completes.
function watchMarkers(vm, st) {
  vm.onWrite = (fd, b) => {
    st.out += new TextDecoder().decode(b);
    if (st.start && st.ts === null && st.out.includes(st.start)) st.ts = performance.now();
    if (st.done && st.td === null && st.out.includes(st.done)) st.td = performance.now();
  };
}
async function rvComputeBoot(jit, binPath) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0); if (SB && jit) vm.ex.sys_set_superblock(1);
  const st = { vm, out: "", start: null, done: null, ts: null, td: null };
  watchMarkers(vm, st);
  vm.bootLinux({ bios: bbl, kernel: kern, disk: (new Uint8Array(await readFile(join(root, "web/images/root-riscv64.bin")))).slice() });
  for (let i = 0; i < 40000 && !st.out.includes("~ #"); i++) { vm.runSystem(5_000_000n); if ((i & 15) === 0) await tick(); }
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n")); step(vm, 3000);
  const b64 = Buffer.from(await readFile(binPath)).toString("base64");
  vm.consoleInput(enc.encode(": > /tmp/b\n")); step(vm, 1500);
  for (let o = 0; o < b64.length; o += 512) { vm.consoleInput(enc.encode(`printf %s '${b64.slice(o, o + 512)}' >> /tmp/b\n`)); step(vm, 3000); }
  vm.consoleInput(enc.encode("base64 -d /tmp/b > /tmp/c && chmod 755 /tmp/c\n")); step(vm, 12000);
  return st;
}
async function rvRunBench(st) {
  st.out = ""; st.ts = null; st.td = null; st.start = "BENCH_START"; st.done = "BENCH_DONE";
  st.vm.consoleInput(enc.encode("/tmp/c\n"));
  const t0 = performance.now();
  for (let i = 0; i < 2_000_000 && st.td === null; i++) {
    st.vm.runSystem(5_000_000n);
    if ((i & 15) === 0) await tick();
    if (performance.now() - t0 > 200000) break;
  }
  if (st.ts === null || st.td === null) return null;
  const chk = (st.out.match(/checksum=(0x[0-9a-f]+)/) || [, null])[1];
  return { ms: st.td - st.ts, chk };
}
async function rvBootTime(jit) {
  const vm = await RV64.create(wasm); vm.ex.jit_set_enabled(jit ? 1 : 0); if (SB && jit) vm.ex.sys_set_superblock(1);
  let out = ""; vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  const t = performance.now();
  vm.bootLinux({ bios: bbl, kernel: kern, disk: (new Uint8Array(await readFile(join(root, "web/images/root-riscv64.bin")))).slice() });
  for (let i = 0; i < 200000 && !out.includes("~ #"); i++) { vm.runSystem(2_000_000n); if ((i & 15) === 0) await tick(); }
  return out.includes("~ #") ? performance.now() - t : null;
}
async function rvPython(jit) {
  const disk = new Uint8Array(await readFile(join(ARTIFACTS, "deb-riscv64.ext4")));
  const vm = await RV64.create(wasm); vm.ex.jit_set_enabled(jit ? 1 : 0); if (SB && jit) vm.ex.sys_set_superblock(1); vm.ex.sys_set_wallclock(1);
  const st = { vm, out: "", start: null, done: null, ts: null, td: null };
  watchMarkers(vm, st);
  vm.bootLinux({ bios: bbl, kernel: kern, disk: disk.slice(), cmdline: "console=hvc0 root=/dev/vda rw init=/binit.sh", ramMB: 512 });
  for (let i = 0; i < 400000 && !st.out.includes("BENCH_READY"); i++) { vm.runSystem(3_000_000n); if ((i & 15) === 0) await tick(); }
  st.out = ""; st.start = "FIB_START"; st.done = "FIB_DONE";
  vm.consoleInput(enc.encode("/usr/bin/python3 /fib.py\n"));
  const t = performance.now();
  for (let i = 0; i < 6_000_000 && st.td === null; i++) {
    vm.runSystem(4_000_000n);
    if ((i & 15) === 0) await tick();
    if (performance.now() - t > 300000) break;
  }
  return st.ts !== null && st.td !== null ? st.td - st.ts : null;
}
async function rvNbench(jit) {
  const disk = new Uint8Array(await readFile(join(ARTIFACTS, "root-nbench.bin")));
  const vm = await RV64.create(wasm); vm.ex.jit_set_enabled(jit ? 1 : 0); if (SB && jit) vm.ex.sys_set_superblock(1); vm.ex.sys_set_wallclock(1);
  let out = ""; vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios: bbl, kernel: kern, disk: disk.slice() });
  for (let i = 0; i < 60000 && !out.includes("~ #"); i++) { vm.runSystem(2_000_000n); if ((i & 15) === 0) await tick(); }
  out = ""; vm.consoleInput(enc.encode("cd / && ./nbench\n"));
  const t = performance.now();
  for (let i = 0; i < 40_000_000; i++) { vm.runSystem(4_000_000n); if ((i & 15) === 0) await tick(); if (out.includes("Trademarks")) break; if (performance.now() - t > 340000) break; }
  const rows = {};
  for (const m of out.matchAll(/^([A-Z][A-Z ]+?)\s+:\s+([\d.e+]+)\s+:/gm)) rows[m[1].trim()] = +m[2];
  return rows;
}
// compile benchmark: boot the buildroot image with our riscv64 tcc + w.c, time
// RUN_START -> RUN_DONE around `tcc -c /w.c` only (md5sum runs AFTER the timed
// window on BOTH sides and is captured for correctness). Same source + same
// tcc commit as the v86 side.
async function rvCompile(jit) {
  const disk = new Uint8Array(await readFile(join(ARTIFACTS, "cc-bench.img")));
  const vm = await RV64.create(wasm); vm.ex.jit_set_enabled(jit ? 1 : 0); if (SB && jit) vm.ex.sys_set_superblock(1);
  const st = { vm, out: "", start: null, done: null, ts: null, td: null };
  watchMarkers(vm, st);
  vm.bootLinux({ bios: bbl, kernel: kern, disk: disk.slice() });
  for (let i = 0; i < 40000 && !st.out.includes("~ #"); i++) { vm.runSystem(5_000_000n); if ((i & 15) === 0) await tick(); }
  if (!st.out.includes("~ #")) return null;
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n")); step(vm, 3000);
  st.out = ""; st.start = "RUN_START"; st.done = "RUN_DONE";
  vm.consoleInput(enc.encode("echo RUN_START; /tcc -c /w.c -o /w.o 2>/tmp/e; echo RUN_DONE; md5sum /w.o\n"));
  const t = performance.now();
  for (let i = 0; i < 2_000_000; i++) {
    vm.runSystem(5_000_000n);
    if ((i & 15) === 0) await tick();
    if (st.td !== null && /[0-9a-f]{32}/.test(st.out)) break;
    if (performance.now() - t > 300000) break;
  }
  const md5 = (st.out.match(/([0-9a-f]{32})/) || [null])[0];
  if (st.ts === null || st.td === null || !md5) return null;
  return { ms: st.td - st.ts, md5 };
}

// ---------- v86: spawn its runners in the checkout, parse RESULT ----------
function v86Spawn(script, env) {
  return new Promise((resolve) => {
    const p = spawn("node", ["--max-old-space-size=4096", script], { cwd: V86DIR, env: { ...process.env, ARTIFACTS, ...env } });
    let buf = ""; p.stdout.on("data", (d) => (buf += d));
    p.on("close", () => {
      const m = buf.match(/RESULT ms=(\d+)/);
      const md5 = (buf.match(/md5=([0-9a-f]{32})/) || [, null])[1];
      const chk = (buf.match(/chk=checksum=(0x[0-9a-f]+)/) || [, null])[1];
      resolve(m ? { ms: +m[1], md5, chk } : null);
    });
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

// v86 side runs FIRST: the rv64 rows compile thousands of wasm modules whose
// background tier-up would otherwise pollute the v86 subprocesses (measured
// 4x on the compile row). Order symmetric-quiet for both sides.
// v86 side
if (haveV86) {
  for (const jit of FULL ? [false, true] : [true]) {
    log(`[v86 jit=${+jit}] alu`);
    { const r = await v86Compute("alu.i386", jit); const row = (R.ALU ??= {}); row[jit ? "v8j" : "v8i"] = r?.ms ?? null; row[jit ? "v8j_chk" : "v8i_chk"] = r?.chk ?? null; }
    log(" mixed");
    { const r = await v86Compute("rvbench_fs.i386", jit); const row = (R.Mixed ??= {}); row[jit ? "v8j" : "v8i"] = r?.ms ?? null; row[jit ? "v8j_chk" : "v8i_chk"] = r?.chk ?? null; }
    log(" boot"); (R.Boot ??= {})[jit ? "v8j" : "v8i"] = (await v86Boot(jit))?.ms ?? null;
    if (await has(join(ARTIFACTS, "vmlinuz-i386"))) { log(" python"); (R["python fib(30)"] ??= {})[jit ? "v8j" : "v8i"] = (await v86Python(jit))?.ms ?? null; }
    if (await has(join(ARTIFACTS, "deb-i386-bench.cpio.gz"))) {
      log(" compile");
      const r = await v86Compile(jit);
      const row = (R["compile (tcc -c)"] ??= {});
      row[jit ? "v8j" : "v8i"] = r?.ms ?? null;
      row[jit ? "v8j_md5" : "v8i_md5"] = r?.md5 ?? null;
    }
    log("\n");
  }
}

// compute (alu, mixed): one boot per JIT setting, run both binaries
for (const jit of FULL ? [false, true] : [true]) {
  log(`[rv64 compute jit=${+jit}] boot…`);
  const st = await rvComputeBoot(jit, join(ARTIFACTS, "xbench", "alu.rv64"));
  { const r = await rvRunBench(st); const row = (R.ALU ??= {}); row[jit ? "rvj" : "rvi"] = r?.ms ?? null; row[jit ? "rvj_chk" : "rvi_chk"] = r?.chk ?? null; } log(" alu");
  // reuse boot: swap /tmp/c to the mixed binary
  const b64 = Buffer.from(await readFile(join(ARTIFACTS, "xbench", "rvbench_fs.rv64"))).toString("base64");
  st.vm.consoleInput(enc.encode(": > /tmp/b\n")); step(st.vm, 1500);
  for (let o = 0; o < b64.length; o += 512) { st.vm.consoleInput(enc.encode(`printf %s '${b64.slice(o, o + 512)}' >> /tmp/b\n`)); step(st.vm, 3000); }
  st.vm.consoleInput(enc.encode("base64 -d /tmp/b > /tmp/c && chmod 755 /tmp/c\n")); step(st.vm, 12000);
  { const r = await rvRunBench(st); const row = (R.Mixed ??= {}); row[jit ? "rvj" : "rvi"] = r?.ms ?? null; row[jit ? "rvj_chk" : "rvi_chk"] = r?.chk ?? null; } log(" mixed\n");
}
// boot time
for (const jit of FULL ? [false, true] : [true]) {
  log(`[rv64 boot jit=${+jit}]…`);
  const ms = [];
  for (let r = 0; r < REPS; r++) ms.push(await rvBootTime(jit));
  (R.Boot ??= {})[jit ? "rvj" : "rvi"] = median(ms);
  log(" ok\n");
}
// python (needs the debian image)
if (await has(join(ARTIFACTS, "deb-riscv64.ext4"))) for (const jit of FULL ? [false, true] : [true]) {
  log(`[rv64 python jit=${+jit}]…`);
  const ms = [];
  for (let r = 0; r < REPS; r++) ms.push(await rvPython(jit));
  (R["python fib(30)"] ??= {})[jit ? "rvj" : "rvi"] = median(ms);
  log(" ok\n");
}
// compile (needs the cc-bench image; same w.c + tcc commit as v86)
if (await has(join(ARTIFACTS, "cc-bench.img"))) for (const jit of FULL ? [false, true] : [true]) {
  log(`[rv64 compile jit=${+jit}]…`);
  const r = await rvCompile(jit);
  const row = (R["compile (tcc -c)"] ??= {});
  row[jit ? "rvj" : "rvi"] = r?.ms ?? null;
  row[jit ? "rvj_md5" : "rvi_md5"] = r?.md5 ?? null;
  log(" ok\n");
}

// nbench (BYTEmark) — rv64 JIT vs v86 JIT, both self-timed; gated (slow, ~8 min)
const NB_KERNELS = ["NUMERIC SORT", "STRING SORT", "BITFIELD", "FP EMULATION",
                    "FOURIER", "ASSIGNMENT", "IDEA", "HUFFMAN"];
// The guest hvc console can drop report lines under output bursts; a rerun
// is safe (nbench self-times in-guest) and the manifest still flags a
// persistent failure.
async function rvNbenchComplete(jit) {
  for (let attempt = 0; attempt < 2; attempt++) {
    const rows = await rvNbench(jit);
    if (NB_KERNELS.every((k) => rows[k] != null)) return rows;
    log(` [retry: missing ${NB_KERNELS.filter((k) => rows[k] == null).join("/")}]`);
  }
  return rvNbench(jit);
}
let nb = null;
if (WANT_NBENCH && (await has(join(ARTIFACTS, "root-nbench.bin")))) {
  log("[rv64 nbench jit]…"); const nj = await rvNbenchComplete(true);
  let ni = null; if (FULL) { log(" interp…"); ni = await rvNbench(false); }
  let v8 = null;
  if (haveV86 && (await has(join(ARTIFACTS, "deb-i386-bench.cpio.gz")))) { log(" v86…"); v8 = await v86Nbench(); }
  log(" ok\n");
  nb = { jit: nj, int: ni, v8 };
}

// ---------- render ----------
// The bar: rv64 must WIN or MATCH v86 on EVERY row (main benchmarks AND every
// individual nbench kernel — no hiding losses inside a "mixed" summary).
// Speed ratio is uniform across units: >1 = rv64 faster. MATCH allows 5%
// (this host has documented double-digit run-to-run noise; verify borderline
// rows with interleaved median-of-N, never a single run).
const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const ms = (x) => (x == null ? "—" : `${Math.round(x)}ms`);
const speedup = (r) => (r?.rvi && r?.rvj ? (r.rvi / r.rvj).toFixed(1) + "×" : "—");
const order = ["ALU", "Mixed", "Boot", "python fib(30)", "compile (tcc -c)"];

// unified verdict rows: {name, rv, v8, unit, speed} with speed = rv64/v86 (>1 = rv64 faster)
const rows = [];
for (const k of order) {
  const r = R[k]; if (!r) continue;
  rows.push({ name: k, rv: ms(r.rvj), v8: ms(r.v8j), speedup: speedup(r),
              speed: r.rvj && r.v8j ? r.v8j / r.rvj : null });
}
if (nb) for (const k of Object.keys(nb.jit)) {
  const rj = nb.jit[k], vj = nb.v8?.[k];
  rows.push({ name: `nbench ${k}`, rv: rj ?? "—", v8: vj ?? "—", speedup: "—",
              speed: rj && vj ? rj / vj : null }); // iterations/sec: higher = faster
}
const verdict = (s) => s == null ? "—" : s >= 1.05 ? `**WIN** ${s.toFixed(2)}×` : s >= 0.95 ? `**MATCH** ${s.toFixed(2)}×` : `LOSS ${(1 / s).toFixed(2)}× behind`;
const passing = rows.filter((r) => r.speed != null && r.speed >= 0.95);
const scored = rows.filter((r) => r.speed != null);
const failing = scored.filter((r) => r.speed < 0.95);

let md = `# rv64.js vs v86 — SYSTEM-EMULATION scorecard

**SYSTEM EMULATION ONLY.** Both emulators boot a **full Linux** and run every
benchmark **inside the guest** (kernel + userland, JIT vs JIT, host wall-clock
or in-guest self-timing). v86 has no user mode — a user-mode comparison is
meaningless and was a past mistake; nothing user-mode appears in this table.

_${ts}. Speed ratio is rv64/v86 (>1 = rv64 faster). The bar: WIN or MATCH on
EVERY row. MATCH = within 5% (host noise; confirm borderline rows with
interleaved median-of-N runs)._

| # | Benchmark | rv64 JIT | v86 JIT | verdict (speed vs v86) |
|--:|---|--:|--:|---|
`;
rows.forEach((r, i) => { md += `| ${i + 1} | ${r.name} | ${r.rv} | ${r.v8} | ${verdict(r.speed)} |\n`; });
md += `\n**Overall: ${passing.length}/${scored.length} rows at win-or-match.**`;
md += failing.length ? ` Failing: ${failing.map((r) => `${r.name} (${(1 / r.speed).toFixed(1)}× behind)`).join(", ")}.\n` : ` ALL ROWS PASS.\n`;
if (!nb) md += `\n_nbench kernels not run (set NBENCH=1) — the bar includes them; a scorecard without them is INCOMPLETE._\n`;
if (FULL) {
  md += `\n<details><summary>interpreter columns (FULL=1)</summary>\n\n| Benchmark | rv64 interp | v86 interp | rv64 JIT/interp |\n|---|--:|--:|--:|\n`;
  for (const k of order) { const r = R[k]; if (!r) continue; md += `| ${k} | ${ms(r.rvi)} | ${ms(r.v8i)} | ${speedup(r)} |\n`; }
  md += `\n</details>\n`;
}
// ---------- enforcement (ISSUES.md P1: manifest, correctness, exit code) ----------
// Every required row must have produced numbers on both sides; ALU checksums
// must be bit-identical cross-ISA; Mixed low-32 must match; compile must
// yield an object md5 on both sides. Any violation = nonzero exit — the
// scorecard cannot silently shrink its scope or report a win on wrong output.
const problems = [];
const need = (cond, what) => { if (!cond) problems.push(what); };
for (const k of order) {
  const r = R[k];
  need(r && r.rvj != null, `${k}: rv64 row missing/failed`);
  if (haveV86) need(r && r.v8j != null, `${k}: v86 row missing/failed`);
}
if (R.ALU?.rvj_chk || R.ALU?.v8j_chk)
  need(R.ALU?.rvj_chk && R.ALU?.rvj_chk === R.ALU?.v8j_chk,
       `ALU checksum mismatch (rv=${R.ALU?.rvj_chk} v86=${R.ALU?.v8j_chk})`);
if (R.Mixed?.rvj_chk && R.Mixed?.v8j_chk)
  need(R.Mixed.rvj_chk === R.Mixed.v8j_chk,
       `Mixed checksum mismatch (rv=${R.Mixed.rvj_chk} v86=${R.Mixed.v8j_chk})`);
{
  const r = R["compile (tcc -c)"];
  if (r) {
    need(!r.rvj || r.rvj_md5, "compile: rv64 object md5 missing");
    if (haveV86) need(!r.v8j || r.v8j_md5, "compile: v86 object md5 missing");
  }
}
if (WANT_NBENCH) {
  const KERNELS = ["NUMERIC SORT", "STRING SORT", "BITFIELD", "FP EMULATION",
                   "FOURIER", "ASSIGNMENT", "IDEA", "HUFFMAN"];
  for (const k of KERNELS) {
    need(nb?.jit?.[k] != null, `nbench ${k}: rv64 kernel missing`);
    if (haveV86) need(nb?.v8?.[k] != null, `nbench ${k}: v86 kernel missing`);
  }
}
if (problems.length) {
  md += `\n**SCORECARD INVALID — ${problems.length} problem(s):** ${problems.join("; ")}\n`;
  process.exitCode = 1;
}

console.log("\n" + md);
await writeFile(join(ARTIFACTS, `scorecard-${ts}.md`), md);
const provenance = {
  git: (() => { try { return execSync("git -C " + root + " rev-parse HEAD").toString().trim(); } catch { return "unknown"; } })(),
  git_dirty: (() => { try { return execSync("git -C " + root + " status --porcelain").toString().trim().length > 0; } catch { return null; } })(),
  wasm_sha256: createHash("sha256").update(wasm).digest("hex"),
  node: process.version,
  reps: REPS, sb: SB, nbench: WANT_NBENCH, full: FULL,
};
await writeFile(join(ARTIFACTS, `scorecard-${ts}.json`), JSON.stringify({ ts, system_emulation: true, provenance, results: R, nbench: nb, pass: `${passing.length}/${scored.length}`, problems }, null, 2));
console.log(`saved ${join(ARTIFACTS, `scorecard-${ts}.md`)} (+ .json)`);
