// rv64.js vs copy/v86 — SYSTEM-MODE comparison (the fair one: v86 has no
// user mode). Both emulators boot a full Linux and run the SAME fixed-work
// kernels inside the guest, timed host-wall-clock between the guest's
// BENCH_START / BENCH_DONE serial markers.
//
//   rv64.js : boots our riscv64 buildroot Linux (web/images/*), injects the
//             freestanding riscv64 binary over the console (base64), runs it.
//   v86     : v86-compute.mjs boots i686 buildroot Linux, injects the i386
//             binary over 9p, runs it. (BIN selects the kernel.)
//
// NOTE: our recent JIT wins (FP-in-blocks, structured/nested loops, FP-in-
// locals — roadmap 3a-3e) are USER-MODE only (gated on lay.mem / f_base).
// The system-mode JIT here is the older ALU-inline + inline-TLB one with
// per-iteration loop dispatch and NO FP compilation, so expect it to trail
// its own user-mode numbers (and likely v86, whose JIT is system-mode).
//
//   SC=<scratchpad> nix develop -c node tests/vs-v86/compare-sys.mjs

import { readFile, copyFile, access } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const SC = process.env.SC;
if (!SC) { console.error("set SC=<dir with xbench/ binaries>"); process.exit(2); }
const V86DIR = process.env.V86DIR || join(SC, "v86");
const RUN_INTERP = !+process.env.SKIP_INTERP;

const { RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const imgs = await Promise.all(
  ["bbl64.bin", "kernel-riscv64.bin", "root-riscv64.bin"].map(async (f) =>
    new Uint8Array(await readFile(join(root, "web/images", f)))),
);
const enc = new TextEncoder();

// Matched kernels: same C source per ISA, freestanding (raw syscalls), so
// each runs as a static process in either guest.
const WL = [
  { name: "alu", rv64: "alu.rv64", v86: "alu.i386" },
  { name: "mixed", rv64: "rvbench_fs.rv64", v86: "rvbench_fs.i386" },
];

function step(vm, n) { for (let i = 0; i < n; i++) vm.runSystem(2_000_000n); }

// Boot our Linux with the given JIT setting and inject every workload binary
// into /tmp/<name>. Returns the vm + a live-output accessor.
async function bootInject(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  let out = "";
  vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios: imgs[0], kernel: imgs[1], disk: imgs[2].slice() });
  for (let i = 0; i < 40000 && !out.includes("~ #"); i++) vm.runSystem(5_000_000n);
  if (!out.includes("~ #")) throw new Error("boot failed");
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n")); step(vm, 3000);
  for (const w of WL) {
    const b64 = Buffer.from(await readFile(join(SC, "xbench", w.rv64))).toString("base64");
    vm.consoleInput(enc.encode(`: > /tmp/${w.name}.b64\n`)); step(vm, 1500);
    for (let o = 0; o < b64.length; o += 512) {
      vm.consoleInput(enc.encode(`printf %s '${b64.slice(o, o + 512)}' >> /tmp/${w.name}.b64\n`));
      step(vm, 3000);
    }
    vm.consoleInput(enc.encode(`base64 -d /tmp/${w.name}.b64 > /tmp/${w.name} && chmod 755 /tmp/${w.name}\n`));
    step(vm, 12000);
  }
  return { vm, get out() { return out; }, set out(v) { out = v; } };
}

// Run one injected workload, timing the compute phase (START->DONE markers).
function runOne(st, name) {
  st.out = "";
  st.vm.consoleInput(enc.encode(`/tmp/${name}\n`));
  const t0 = performance.now();
  let tstart = null, tdone = null;
  for (let i = 0; i < 2_000_000; i++) {
    st.vm.runSystem(5_000_000n);
    if (tstart === null && st.out.includes("BENCH_START")) tstart = performance.now();
    if (st.out.includes("BENCH_DONE")) { tdone = performance.now(); break; }
    if (performance.now() - t0 > 180000) break;
  }
  const chk = (st.out.match(/checksum=0x[0-9a-f]+/) || ["?"])[0];
  return { ms: tstart && tdone ? tdone - tstart : null, chk };
}

function v86run(bin, jit) {
  return new Promise((resolve) => {
    const env = { ...process.env, SC, BIN: bin, DISABLE_JIT: jit ? "0" : "1" };
    const p = spawn("node", ["v86-compute.mjs"], { cwd: V86DIR, env });
    let buf = "";
    p.stdout.on("data", (d) => (buf += d));
    p.on("close", () => {
      const m = buf.match(/RESULT ms=(\d+) chk=(\S+)/);
      resolve(m ? { ms: +m[1], chk: m[2] } : { ms: null });
    });
  });
}

// ---- our side: one boot per JIT setting, run every workload (JIT warms once) ----
const rv = {};
for (const jit of RUN_INTERP ? [false, true] : [true]) {
  process.stderr.write(`[rv64 sys] boot+inject (jit=${jit ? 1 : 0})…`);
  const st = await bootInject(jit);
  for (const w of WL) {
    if (jit) runOne(st, w.name); // warm (compile)
    process.stderr.write(` ${w.name}`);
    rv[`${w.name}:${jit ? "jit" : "int"}`] = runOne(st, w.name);
  }
  process.stderr.write(" done\n");
  st.vm.consoleInput(enc.encode("")); // release
}

// ---- v86 side ----
let haveV86 = true;
try {
  await access(join(V86DIR, "src/main.js"));
  await copyFile(join(root, "tests/vs-v86/v86-compute.mjs"), join(V86DIR, "v86-compute.mjs"));
} catch { haveV86 = false; }
const v8 = {};
if (haveV86) {
  for (const jit of RUN_INTERP ? [false, true] : [true]) {
    for (const w of WL) {
      process.stderr.write(`[v86 sys] ${w.name} jit=${jit ? 1 : 0}…\n`);
      v8[`${w.name}:${jit ? "jit" : "int"}`] = await v86run(w.v86, jit);
    }
  }
}

// ---- report ----
const f = (x) => (x == null || x.ms == null ? "     —" : `${x.ms.toFixed(0).padStart(6)}ms`);
console.log(`\n# rv64.js vs copy/v86 — SYSTEM MODE (both boot full Linux, run the kernel in-guest)`);
console.log(`# host-wall-clock BENCH_START->BENCH_DONE${haveV86 ? "" : "  —  v86 NOT FOUND (rv64 only)"}\n`);
console.log("workload  rv64 interp   rv64 JIT      v86 interp    v86 JIT     rv64 JIT vs v86 JIT");
console.log("────────  ───────────   ─────────     ──────────    ───────     ───────────────────");
for (const w of WL) {
  const ri = rv[`${w.name}:int`], rj = rv[`${w.name}:jit`];
  const vi = v8[`${w.name}:int`], vj = v8[`${w.name}:jit`];
  const ratio = rj && rj.ms && vj && vj.ms
    ? (rj.ms / vj.ms).toFixed(2) + "x " + (rj.ms < vj.ms ? "(rv64 faster)" : "(v86 faster)")
    : "—";
  console.log(`${w.name.padEnd(8)}  ${f(ri).padStart(11)}   ${f(rj).padStart(9)}     ${f(vi).padStart(10)}   ${f(vj).padStart(9)}    ${ratio}`);
}
console.log("\nJIT speedup over own interpreter:");
for (const w of WL) {
  const ri = rv[`${w.name}:int`], rj = rv[`${w.name}:jit`], vi = v8[`${w.name}:int`], vj = v8[`${w.name}:jit`];
  const rs = ri && ri.ms && rj && rj.ms ? (ri.ms / rj.ms).toFixed(1) + "x" : "—";
  const vs = vi && vi.ms && vj && vj.ms ? (vi.ms / vj.ms).toFixed(1) + "x" : "—";
  console.log(`  ${w.name.padEnd(8)} rv64 ${rs.padStart(6)}   v86 ${vs.padStart(6)}`);
}
console.log("\nchecksums (rv64 low-32 should match v86; FP high bits differ by double-vs-SSE):");
for (const w of WL) {
  console.log(`  ${w.name.padEnd(8)} rv64 ${(rv[`${w.name}:jit`] || {}).chk || "?"}   v86 ${(v8[`${w.name}:jit`] || {}).chk || "?"}`);
}
