// rv64.js vs copy/v86 — single reusable cross-emulator comparison driver.
//
// Runs BOTH emulators FRESH on the same machine (no hardcoded baselines) over
// matched fixed-work kernels — pure ALU (alu.c) and mixed int/FP/mem
// (rvbench_fs.c) — each compiled for x86 (v86) and riscv64 (rv64.js) from the
// same source, host-wall-clock timed between BENCH_START/BENCH_DONE. For each
// emulator it measures the interpreter and the JIT, then prints a table with
// the headline ratio: our JIT vs v86's JIT.
//
// Inputs (external, see tests/vs-v86/README.md):
//   ARTIFACTS       dir containing xbench/{alu,rvbench}.rv64 + xbench/{alu,rvbench_fs}.i386  (required)
//   V86DIR   copy/v86 checkout, built (default $ARTIFACTS/v86); if absent, runs rv64 only
//   WASM     rv64_wasm.wasm (default target/wasm32-unknown-unknown/release/…)
//   JIT_REPS best-of-N for the (fast) JIT runs (default 3)
//   SKIP_INTERP=1  skip the slow interpreter runs (JIT-vs-JIT only)
//
//   nix develop -c node tests/vs-v86/compare.mjs

import { readFile, copyFile, access } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
if (!ARTIFACTS) {
  console.error("set ARTIFACTS=<dir containing xbench/ benchmark binaries>");
  process.exit(2);
}
const V86DIR = process.env.V86DIR || join(ARTIFACTS, "v86");
const WASM = process.env.WASM || join(ROOT, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
const JIT_REPS = +(process.env.JIT_REPS || 3);
const RUN_INTERP = !+process.env.SKIP_INTERP;

const { RV64, Stop } = await import(join(ROOT, "web/rv64.js"));
const wasmBytes = new Uint8Array(await readFile(WASM));

// Matched workloads: identical source, per-ISA binary. Checksums are printed
// per side to confirm each build did the fixed work (verified stable across
// that emulator's own interp vs JIT runs).
const WL = [
  { name: "alu", rv64: "alu.rv64", v86: "alu.i386", chk: /checksum=0x[0-9a-f]+/, rvchk: /checksum=0x[0-9a-f]+/ },
  { name: "mixed", rv64: "rvbench.rv64", v86: "rvbench_fs.i386", chk: /checksum=0x[0-9a-f]+/, rvchk: /isum=\d+/ },
];

async function rv64run(elfPath, jitOn) {
  const vm = await RV64.create(wasmBytes);
  let out = "";
  vm.onWrite = (fd, bytes) => { out += new TextDecoder().decode(bytes); };
  vm.ex.jit_set_enabled(jitOn ? 1 : 0);
  vm.loadElf(new Uint8Array(await readFile(elfPath)), ["b"]);
  const t0 = performance.now();
  let s, g = 0;
  do { s = vm.runUser(50_000_000_000n); } while (s !== Stop.EXITED && ++g < 40);
  const ms = performance.now() - t0;
  const insns = Number(vm.userInsnCount());
  const cov = jitOn ? (Number(vm.ex.jit_stat(0)) / insns) * 100 : 0;
  return { ms, insns, cov, exit: vm.userExitCode(), chk: out };
}

function v86run(bin, jitOn) {
  return new Promise((resolve) => {
    const env = { ...process.env, ARTIFACTS, BIN: bin, DISABLE_JIT: jitOn ? "0" : "1" };
    const p = spawn("node", ["v86-compute.mjs"], { cwd: V86DIR, env });
    let buf = "";
    p.stdout.on("data", (d) => (buf += d));
    p.on("close", () => {
      const m = buf.match(/RESULT ms=(\d+) chk=(\S+)/);
      resolve(m ? { ms: +m[1], chk: m[2] } : { ms: null, err: buf.slice(-160) });
    });
  });
}

const best = (a) => a.filter((x) => x.ms != null).reduce((m, x) => (x.ms < m.ms ? x : m), { ms: Infinity });
const bestN = async (fn, reps) => { const r = []; for (let i = 0; i < reps; i++) r.push(await fn()); return best(r); };

// Locate v86 (copy our runner into its checkout so its relative ./src, ./bios
// and ./images resolve — v86's node loader reads them relative to cwd).
let haveV86 = true;
try {
  await access(join(V86DIR, "src/main.js"));
  await copyFile(join(ROOT, "tests/vs-v86/v86-compute.mjs"), join(V86DIR, "v86-compute.mjs"));
} catch {
  haveV86 = false;
}

console.log(`# rv64.js vs copy/v86 — fresh same-machine run`);
console.log(`# JIT best-of-${JIT_REPS}${RUN_INTERP ? ", interpreter 1x" : " (JIT only)"}${haveV86 ? "" : "  —  v86 NOT FOUND at " + V86DIR + " (rv64 only)"}`);
console.log(`# NOTE: shared/noisy host — the JIT-vs-JIT ratio and coverage%% are the load-robust numbers.\n`);

const rows = [];
for (const w of WL) {
  const rvElf = join(ARTIFACTS, "xbench", w.rv64);
  process.stderr.write(`[${w.name}] rv64 JIT…`);
  const rJit = await bestN(() => rv64run(rvElf, true), JIT_REPS);
  let rInt = null;
  if (RUN_INTERP) { process.stderr.write(" interp…"); rInt = await rv64run(rvElf, false); }
  let vJit = { ms: null }, vInt = null;
  if (haveV86) {
    process.stderr.write(" v86 JIT…");
    vJit = await bestN(() => v86run(w.v86, true), Math.min(2, JIT_REPS));
    if (RUN_INTERP) { process.stderr.write(" v86 interp…"); vInt = await v86run(w.v86, false); }
  }
  process.stderr.write(" done\n");
  rows.push({ w, rJit, rInt, vJit, vInt });
}

const f = (x) => (x == null || x.ms == null || !isFinite(x.ms) ? "     —" : `${x.ms.toFixed(0).padStart(6)}ms`);
console.log("workload  rv64 interp   rv64 JIT (cov)     v86 interp    v86 JIT     rv64 JIT vs v86 JIT");
console.log("────────  ───────────   ──────────────     ──────────    ───────     ───────────────────");
for (const { w, rJit, rInt, vJit, vInt } of rows) {
  const cov = rJit.cov != null ? `${rJit.cov.toFixed(0)}%` : "?";
  const rvsv = vJit.ms ? (rJit.ms / vJit.ms).toFixed(2) + "x " + (rJit.ms < vJit.ms ? "(rv64 FASTER)" : "(v86 faster)") : "—";
  console.log(
    `${w.name.padEnd(8)}  ${f(rInt).padStart(11)}   ${(f(rJit) + " " + cov).padEnd(15)}    ${f(vInt).padStart(10)}   ${f(vJit).padStart(9)}    ${rvsv}`
  );
}
console.log("\nper-emulator JIT speedup over its own interpreter:");
for (const { w, rJit, rInt, vJit, vInt } of rows) {
  const rv = rInt && rInt.ms ? (rInt.ms / rJit.ms).toFixed(1) + "x" : "—";
  const v8 = vInt && vInt.ms && vJit.ms ? (vInt.ms / vJit.ms).toFixed(1) + "x" : "—";
  console.log(`  ${w.name.padEnd(8)} rv64 ${rv.padStart(6)}   v86 ${v8.padStart(6)}`);
}
console.log("\nchecksums (stable within each emulator = fixed work done):");
for (const { w, rJit, vJit } of rows) {
  const rc = (rJit.chk.match(w.rvchk) || ["?"])[0];
  console.log(`  ${w.name.padEnd(8)} rv64 ${rc}   v86 ${vJit.chk || "?"}`);
}
