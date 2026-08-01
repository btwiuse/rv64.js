// LEGACY FLIP CHECK — prefer screen.mjs for rejection and ab.mjs for serial
// candidate-vs-control evidence. This measures ONE row on rv64 against a saved
// v86 number, so it accepts only a valid AUTHORITATIVE scorecard.
//
// The full scorecard is slow; this is quicker because:
//
//   * v86 is the CONTROL. Changing our JIT cannot change v86's number, so
//     re-measuring it every iteration buys nothing. Its spread across every
//     run today was small (ASSIGNMENT 10.73-10.88, python 3050-3283), and
//     the baseline used here is printed so it can be sanity-checked.
//   * nbench can run ONE kernel: CUSTOMRUN=T plus DO<KERNEL>=T in a command
//     file. (Quirk: nbench upper-cases the -c path, so the file must be /C.)
//     Running one kernel instead of eight is most of the saving.
//
// LIMITS — read before trusting a flip:
//   * A kernel run ALONE is not identical to the same kernel inside the full
//     table: different warm-up, JIT population, and page history. A flip here
//     is a SCREEN, and must be confirmed by the serial scorecard.
//   * Boots are run in parallel (K), which inflates absolute times; ratios
//     against a serially-measured v86 baseline are therefore conservative
//     for wall-clock rows (python/compile) and roughly fair for the
//     self-timed nbench kernels.
//
//   node tests/vs-v86/flip.mjs assignment [K]
//   node tests/vs-v86/flip.mjs python [K]
//   ROWS: alu mixed boot python compile numeric string bitfield fpemul
//         fourier assignment idea huffman
import { readFile, readdir } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = process.env.ROOT || fileURLToPath(new URL("../..", import.meta.url));
const ARTIFACTS = process.env.ARTIFACTS || join(root, "target/bench");

// row key -> [scorecard row name, nbench DO-flag or null, higher-is-better]
const ROWS = {
  alu: ["ALU", null, false],
  mixed: ["Mixed", null, false],
  boot: ["Boot", null, false],
  python: ["python fib(30)", null, false],
  compile: ["compile (tcc -c)", null, false],
  numeric: ["NUMERIC SORT", "DONUMSORT", true],
  string: ["STRING SORT", "DOSTRINGSORT", true],
  bitfield: ["BITFIELD", "DOBITFIELD", true],
  fpemul: ["FP EMULATION", "DOEMFLOAT", true],
  fourier: ["FOURIER", "DOFOUR", true],
  assignment: ["ASSIGNMENT", "DOASSIGN", true],
  idea: ["IDEA", "DOIDEA", true],
  huffman: ["HUFFMAN", "DOHUFF", true],
};

const key = (process.argv[2] || "").toLowerCase();
const K = Math.max(1, +(process.argv[3] || 4));
if (!ROWS[key]) {
  console.error(`row must be one of: ${Object.keys(ROWS).join(" ")}`);
  process.exit(2);
}
const [rowName, kernelFlag, higherBetter] = ROWS[key];

// ---- v86 baseline from the newest scorecard JSON ----
async function baseline() {
  const files = (await readdir(ARTIFACTS))
    .filter((f) => f.startsWith("scorecard-") && f.endsWith(".json"))
    .sort();
  for (const f of files.reverse()) {
    const j = JSON.parse(await readFile(join(ARTIFACTS, f), "utf8"));
    if (j.valid !== true || j.authoritative !== true) continue;
    const v = kernelFlag ? j.nbench?.v8?.[rowName] : j.results?.[rowName]?.v8j;
    if (v != null) return { v, from: f };
  }
  return null;
}

if (process.env.FLIP_CHILD) {
  const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
  const wasm = await readFile(
    process.env.WASM || join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  );
  const bbl = new Uint8Array(await readFile(join(root, "web/images/bbl64.bin")));
  const kern = new Uint8Array(await readFile(join(root, "web/images/kernel-riscv64.bin")));
  const enc = new TextEncoder();
  const tick = () => new Promise((r) => setImmediate(r));
  const step = (vm, n) => { for (let i = 0; i < n; i++) vm.runSystem(2_000_000n); };
  const mk = async (diskFile) => {
    const disk = new Uint8Array(await readFile(join(ARTIFACTS, diskFile)));
    const vm = await RV64.create(wasm);
    vm.ex.jit_set_enabled(1);
    vm.ex.jit_set_tlb_fill(+(process.env.TLBFILL ?? 0));
    for (const [env, fn] of [
      ["TRACELVL", "jit_set_trace_level"], ["TRACEWIN", "jit_set_trace_window"],
      ["KEEPMIN", "jit_set_trace_keep_min"],
      ["DEMOTE", "jit_set_demote"], ["BATCH", "jit_set_batch"],
      ["ICTRIG", "jit_set_ic_trigger"], ["DEFTRACK", "jit_set_defined"],
      ["BCAP", "jit_set_batch_cap"], ["BPAGE", "jit_set_batch_page"],
    ]) if (process.env[env] !== undefined) vm.ex[fn]?.(+process.env[env]);
    if (process.env.SB === undefined || +process.env.SB) vm.ex.sys_set_superblock(1);
    return { vm, disk };
  };

  if (kernelFlag) {
    const { vm, disk } = await mk("root-nbench.bin");
    vm.ex.sys_set_wallclock(1);
    let out = "";
    vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
    vm.bootLinux({ bios: bbl, kernel: kern, disk: disk.slice() });
    for (let i = 0; i < 60000 && !out.includes("~ #"); i++) { vm.runSystem(2_000_000n); if ((i & 15) === 0) await tick(); }
    // Single-kernel command file. nbench upper-cases the -c path, so /C.
    vm.consoleInput(enc.encode("echo CUSTOMRUN=T > /C\n")); step(vm, 400);
    vm.consoleInput(enc.encode(`echo ${kernelFlag}=T >> /C\n`)); step(vm, 400);
    out = "";
    vm.consoleInput(enc.encode("cd / && ./nbench -c/C\n"));
    const t = performance.now();
    for (let i = 0; i < 40_000_000; i++) {
      vm.runSystem(4_000_000n);
      if ((i & 3) === 0) await tick();
      if (out.includes("Trademarks")) break;
      if (performance.now() - t > 300000) break;
    }
    // Same parser as the scorecard: a kernel nbench flags as statistically
    // uncertain puts its number on a later continuation line.
    let last = null, val = null;
    for (const line of out.split("\n")) {
      const named = line.match(/^([A-Z][A-Z ]+?)\s+:\s*([\d.e+]*)/);
      if (named) { last = named[1].trim(); if (named[2] && last === rowName) val = +named[2]; if (named[2]) last = null; continue; }
      const cont = line.match(/^\s+:\s+([\d.e+]+)\s+:/);
      if (cont && last === rowName) { val = +cont[1]; last = null; }
    }
    console.log(`VALUE ${val ?? "null"}`);
  } else if (key === "python") {
    const { vm, disk } = await mk("deb-riscv64.ext4");
    vm.ex.sys_set_wallclock(1);
    const st = { out: "", ts: null, td: null };
    vm.onWrite = (fd, b) => {
      st.out += new TextDecoder().decode(b);
      if (st.ts === null && st.out.includes("FIB_START")) st.ts = performance.now();
      if (st.td === null && st.out.includes("FIB_DONE")) st.td = performance.now();
    };
    vm.bootLinux({ bios: bbl, kernel: kern, disk: disk.slice(), cmdline: "console=hvc0 root=/dev/vda rw init=/binit.sh", ramMB: 512 });
    for (let i = 0; i < 400000 && !st.out.includes("BENCH_READY"); i++) { vm.runSystem(3_000_000n); if ((i & 15) === 0) await tick(); }
    st.out = "";
    vm.consoleInput(enc.encode("/usr/bin/python3 /fib.py\n"));
    const t = performance.now();
    for (let i = 0; i < 6_000_000 && st.td === null; i++) {
      vm.runSystem(4_000_000n);
      if ((i & 15) === 0) await tick();
      if (performance.now() - t > 300000) break;
    }
    console.log(`VALUE ${st.ts !== null && st.td !== null ? (st.td - st.ts).toFixed(0) : "null"}`);
  } else if (key === "compile") {
    const { vm, disk } = await mk("cc-bench.img");
    const st = { out: "", ts: null, td: null };
    vm.onWrite = (fd, b) => {
      st.out += new TextDecoder().decode(b);
      if (st.ts === null && st.out.includes("RUN_START")) st.ts = performance.now();
      if (st.td === null && st.out.includes("RUN_DONE")) st.td = performance.now();
    };
    vm.bootLinux({ bios: bbl, kernel: kern, disk: disk.slice() });
    for (let i = 0; i < 40000 && !st.out.includes("~ #"); i++) { vm.runSystem(5_000_000n); if ((i & 15) === 0) await tick(); }
    vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n")); step(vm, 3000);
    st.out = "";
    vm.consoleInput(enc.encode("echo RUN_START; /tcc -c /w.c -o /w.o 2>/tmp/e; echo RUN_DONE; md5sum /w.o\n"));
    const t = performance.now();
    for (let i = 0; i < 2_000_000; i++) {
      vm.runSystem(5_000_000n);
      if ((i & 15) === 0) await tick();
      if (st.td !== null && /[0-9a-f]{32}/.test(st.out)) break;
      if (performance.now() - t > 300000) break;
    }
    console.log(`VALUE ${st.ts !== null && st.td !== null ? (st.td - st.ts).toFixed(0) : "null"}`);
  } else {
    console.log("VALUE null"); // alu/mixed/boot need the inject protocol
  }
  process.exit(0);
}

// ---- parent ----
const base = await baseline();
if (!base) {
  console.error(`no v86 baseline for "${rowName}" in any ${ARTIFACTS}/scorecard-*.json`);
  process.exit(2);
}
const runOne = () =>
  new Promise((resolve) => {
    const p = spawn(process.execPath, [process.argv[1], key], {
      env: { ...process.env, FLIP_CHILD: "1" },
    });
    let buf = "";
    p.stdout.on("data", (d) => (buf += d));
    p.stderr.on("data", () => {});
    p.on("close", () => {
      const m = buf.match(/VALUE ([\d.]+)/);
      resolve(m ? +m[1] : null);
    });
  });

const t0 = performance.now();
const vals = (await Promise.all(Array.from({ length: K }, runOne))).filter((v) => v != null);
const dt = ((performance.now() - t0) / 1000).toFixed(0);
if (!vals.length) {
  console.log(`FLIP ${rowName}: no samples (all runs failed)`);
  process.exit(1);
}
const sorted = [...vals].sort((a, b) => a - b);
const med = sorted[(sorted.length / 2) | 0];
// Speed ratio, uniform with the scorecard: >1 = rv64 faster.
const speed = higherBetter ? med / base.v : base.v / med;
const verdict = speed >= 1.05 ? "WIN" : speed >= 0.95 ? "MATCH" : "LOSS";
console.log(
  `FLIP ${rowName}: rv64 median=${med} (n=${vals.length}/${K}, ${dt}s)  v86=${base.v} [${base.from}]\n` +
    `  speed=${speed.toFixed(3)}x  ->  ${verdict}${verdict === "LOSS" ? ` (${(1 / speed).toFixed(2)}x behind)` : ""}\n` +
    `  SCREEN ONLY — confirm any flip with the serial scorecard.`,
);
