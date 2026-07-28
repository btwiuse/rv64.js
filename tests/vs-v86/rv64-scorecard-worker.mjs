// Isolated rv64 system-mode benchmark worker.
//
// One process runs exactly one fresh-boot trial, prints one RESULT_JSON line,
// and exits. Isolation matters: async WebAssembly tier-up from a completed
// rv64 trial must not contaminate a later v86 trial in the scorecard parent.
//
//   ARTIFACTS=target/bench node rv64-scorecard-worker.mjs alu
//   ARTIFACTS=target/bench PROFILE=1 node rv64-scorecard-worker.mjs compile
//   ARTIFACTS=target/bench node rv64-scorecard-worker.mjs numeric
//
// Rows: alu, mixed, boot, python, compile, nbench, numeric, string, bitfield,
// fpemul, fourier, assignment, idea, huffman.
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const artifactsArg = process.env.ARTIFACTS || process.env.SC;
if (!artifactsArg) {
  console.error("set ARTIFACTS=<artifacts dir>");
  process.exit(2);
}
const ARTIFACTS = resolve(artifactsArg);

const ROWS = {
  alu: { kind: "compute", binary: "alu.rv64" },
  mixed: { kind: "compute", binary: "rvbench_fs.rv64" },
  boot: { kind: "boot" },
  python: { kind: "python" },
  compile: { kind: "compile" },
  nbench: { kind: "nbench" },
  numeric: { kind: "nbench", name: "NUMERIC SORT", flag: "DONUMSORT" },
  string: { kind: "nbench", name: "STRING SORT", flag: "DOSTRINGSORT" },
  bitfield: { kind: "nbench", name: "BITFIELD", flag: "DOBITFIELD" },
  fpemul: { kind: "nbench", name: "FP EMULATION", flag: "DOEMFLOAT" },
  fourier: { kind: "nbench", name: "FOURIER", flag: "DOFOUR" },
  assignment: { kind: "nbench", name: "ASSIGNMENT", flag: "DOASSIGN" },
  idea: { kind: "nbench", name: "IDEA", flag: "DOIDEA" },
  huffman: { kind: "nbench", name: "HUFFMAN", flag: "DOHUFF" },
};
const rowKey = (process.argv[2] || "").toLowerCase();
const row = ROWS[rowKey];
if (!row) {
  console.error(`row must be one of: ${Object.keys(ROWS).join(" ")}`);
  process.exit(2);
}

const { RV64 } = await import(join(root, "web/rv64.js"));
const wasmPath =
  (process.env.WASM ? resolve(process.env.WASM) : null) ||
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
const wasm = await readFile(wasmPath);
const bios = new Uint8Array(await readFile(join(root, "web/images/bbl64.bin")));
const kernel = new Uint8Array(
  await readFile(join(root, "web/images/kernel-riscv64.bin")),
);
const enc = new TextEncoder();
function envFlag(name, fallback = false) {
  const raw = process.env[name];
  if (raw === undefined) return fallback;
  if (raw !== "0" && raw !== "1") {
    throw new Error(`${name} must be 0 or 1`);
  }
  return raw === "1";
}
const jit = !envFlag("DISABLE_JIT");
const profile = envFlag("PROFILE");
const superblocks = envFlag("SB", true);
function envInteger(name, fallback, minimum, maximum) {
  const value =
    process.env[name] === undefined ? fallback : Number(process.env[name]);
  if (
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw new Error(
      `${name} must be an integer from ${minimum} through ${maximum}`,
    );
  }
  return value;
}
const profileSampleShift = envInteger("PROFILE_SHIFT", 8, 0, 20);
const nbenchTimeoutMs = envInteger(
  "NBENCH_TIMEOUT_MS",
  360_000,
  60_000,
  3_600_000,
);
const NBENCH_KERNELS = [
  "NUMERIC SORT",
  "STRING SORT",
  "BITFIELD",
  "FP EMULATION",
  "FOURIER",
  "ASSIGNMENT",
  "IDEA",
  "HUFFMAN",
];
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const commonInputSha256 = {
  bios: sha256(bios),
  kernel: sha256(kernel),
};
const tick = () => new Promise((resolve) => setImmediate(resolve));
const step = (vm, n) => {
  for (let i = 0; i < n; i++) vm.runSystem(2_000_000n);
};

function configure(vm) {
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  const knobs = [
    ["TLBFILL", "jit_set_tlb_fill"],
    ["TRACELVL", "jit_set_trace_level"],
    ["TRACEWIN", "jit_set_trace_window"],
    ["KEEPMIN", "jit_set_trace_keep_min"],
    ["DEMOTE", "jit_set_demote"],
    ["BATCH", "jit_set_batch"],
    ["ICTRIG", "jit_set_ic_trigger"],
    ["DEFTRACK", "jit_set_defined"],
    ["ROTNEST", "jit_set_rotated_nests"],
    ["BCAP", "jit_set_batch_cap"],
    ["BPAGE", "jit_set_batch_page"],
    ["SBSPACE", "jit_set_sb_spacing"],
  ];
  for (const [env, fn] of knobs) {
    if (process.env[env] !== undefined) {
      vm.ex[fn]?.(envInteger(env, 0, 0, 0xffff_ffff));
    }
  }
  if (jit && superblocks) {
    vm.ex.sys_set_superblock(1);
  }
}

const STAT_IDS = [
  0, 1, 2, 3, 4, 5, 8, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
  22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39,
  40, 41, 42, 43, 44, 45,
];
const STAT_NAMES = {
  0: "jit_retired",
  1: "dispatches",
  2: "user_blocks",
  3: "system_blocks",
  4: "slice_calls",
  5: "slice_insns",
  8: "copy_chunks",
  10: "sb_trigger",
  11: "sb_translate_fail",
  12: "sb_issued",
  13: "sb_landed",
  14: "sb_stale",
  15: "zero_retire",
  16: "sb_individual",
  17: "pending_sb",
  18: "zero_nx",
  19: "zero_frm",
  20: "zero_fs",
  21: "drop_self",
  22: "drop_region",
  23: "dirty_events",
  24: "dirty_dropped",
  25: "sb_entries",
  26: "sb_replaced",
  27: "trace_sb_install",
  28: "trace_individual",
  29: "trace_seed",
  30: "trace_entry",
  31: "tlb_fills",
  32: "sb_extend_issued",
  33: "sb_exit_sampled",
  34: "sb_build_ms",
  35: "sb_exit_nomap",
  36: "sb_exit_inregion",
  37: "sb_extend_defer_cool",
  38: "sb_extend_no_target",
  39: "sb_extend_pushed",
  40: "sb_extend_drain_visits",
  41: "sb_extend_drain_nomatch",
  42: "sb_demoted",
  43: "batches",
  44: "batch_members",
  45: "ic_extends",
};
const GAUGES = new Set([2, 3, 17]);
const readStats = (vm) =>
  Object.fromEntries(
    STAT_IDS.map((id) => [STAT_NAMES[id], Number(vm.ex.jit_stat(id))]),
  );
const statDelta = (before, after) => {
  const out = {};
  for (const id of STAT_IDS) {
    const name = STAT_NAMES[id];
    out[name] = GAUGES.has(id) ? after[name] : after[name] - before[name];
  }
  out.insns_per_dispatch =
    out.dispatches > 0 ? out.jit_retired / out.dispatches : null;
  return out;
};

function readProfile(vm, sampleShift) {
  if (!profile) return null;
  const dispatch = [];
  for (let i = 0; i < 8192; i++) {
    const count = Number(vm.ex.dprof_get(1, i));
    if (!count) continue;
    const pc = vm.ex.dprof_get(0, i);
    const retired = Number(vm.ex.dprof_get(2, i));
    dispatch.push({
      pc: `0x${pc.toString(16)}`,
      dispatches: count,
      retired,
      insns_per_dispatch: retired / count,
    });
  }
  dispatch.sort((a, b) => b.dispatches - a.dispatches);

  const fallback = [];
  for (let i = 0; i < 1024; i++) {
    const count = Number(vm.ex.ihist_get(1, i));
    if (!count) continue;
    fallback.push({
      key: `0x${vm.ex.ihist_get(0, i).toString(16)}`,
      stretches: count,
      interpreted_insns: Number(vm.ex.ihist_get(2, i)),
    });
  }
  fallback.sort((a, b) => b.interpreted_insns - a.interpreted_insns);

  const fingerprint = createHash("sha256")
    .update(
      dispatch
        .map((entry) => `${entry.pc}:${entry.dispatches}:${entry.retired}`)
        .sort()
        .join("\n"),
    )
    .digest("hex");
  const coverageFingerprint = createHash("sha256")
    .update(dispatch.map((entry) => entry.pc).sort().join("\n"))
    .digest("hex");
  return {
    requested_sample_shift: profileSampleShift,
    sample_shift: sampleShift,
    sample_rate: `1/${2 ** sampleShift}`,
    dispatch_profile_sha256: fingerprint,
    dispatch_coverage_sha256: coverageFingerprint,
    dispatch_sites: dispatch.length,
    top_dispatch: dispatch.slice(0, 20),
    top_fallback: fallback.slice(0, 20),
  };
}

function beginMeasurement(vm) {
  let sampleShift = 0;
  if (profile) {
    if (typeof vm.ex.dprof_set_sample_shift === "function") {
      vm.ex.dprof_set_sample_shift(profileSampleShift);
      sampleShift = profileSampleShift;
    }
    vm.ex.dprof_set(1);
  }
  return { stats: readStats(vm), sampleShift };
}

function finishMeasurement(vm, before) {
  return {
    jit: statDelta(before.stats, readStats(vm)),
    profile: readProfile(vm, before.sampleShift),
  };
}

function watchMarkers(vm, state) {
  vm.onWrite = (_fd, bytes) => {
    state.out += new TextDecoder().decode(bytes);
    if (
      state.start &&
      state.started === null &&
      state.out.includes(state.start)
    ) {
      state.started = performance.now();
    }
    if (
      state.done &&
      state.finished === null &&
      state.out.includes(state.done)
    ) {
      state.finished = performance.now();
    }
  };
}

async function bootBuildroot(vm, disk) {
  let out = "";
  vm.onWrite = (_fd, bytes) => {
    out += new TextDecoder().decode(bytes);
  };
  vm.bootLinux({ bios, kernel, disk: disk.slice() });
  for (let i = 0; i < 60_000 && !out.includes("~ #"); i++) {
    vm.runSystem(2_000_000n);
    if ((i & 15) === 0) await tick();
  }
  if (!out.includes("~ #")) throw new Error("buildroot boot failed");
  return out;
}

async function runCompute() {
  const vm = await RV64.create(wasm);
  configure(vm);
  const disk = new Uint8Array(
    await readFile(join(root, "web/images/root-riscv64.bin")),
  );
  const state = {
    out: "",
    start: "BENCH_START",
    done: "BENCH_DONE",
    started: null,
    finished: null,
  };
  watchMarkers(vm, state);
  vm.bootLinux({ bios, kernel, disk: disk.slice() });
  for (let i = 0; i < 40_000 && !state.out.includes("~ #"); i++) {
    vm.runSystem(5_000_000n);
    if ((i & 15) === 0) await tick();
  }
  if (!state.out.includes("~ #")) throw new Error("compute boot failed");
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n"));
  step(vm, 3_000);
  const binary = await readFile(join(ARTIFACTS, "xbench", row.binary));
  const b64 = Buffer.from(binary).toString("base64");
  vm.consoleInput(enc.encode(": > /tmp/b\n"));
  step(vm, 1_500);
  for (let offset = 0; offset < b64.length; offset += 512) {
    vm.consoleInput(
      enc.encode(`printf %s '${b64.slice(offset, offset + 512)}' >> /tmp/b\n`),
    );
    step(vm, 3_000);
  }
  vm.consoleInput(
    enc.encode("base64 -d /tmp/b > /tmp/c && chmod 755 /tmp/c\n"),
  );
  step(vm, 12_000);

  state.out = "";
  const before = beginMeasurement(vm);
  vm.consoleInput(enc.encode("/tmp/c\n"));
  const guardStart = performance.now();
  for (let i = 0; i < 2_000_000 && state.finished === null; i++) {
    vm.runSystem(5_000_000n);
    if ((i & 15) === 0) await tick();
    if (performance.now() - guardStart > 200_000) break;
  }
  if (state.started === null || state.finished === null) {
    throw new Error(`${rowKey} markers missing`);
  }
  return {
    row: rowKey,
    metric: "ms",
    value: state.finished - state.started,
    checksum:
      state.out.match(/checksum=(0x[0-9a-f]+)/)?.[1] ?? null,
    input_sha256: {
      ...commonInputSha256,
      disk: sha256(disk),
      benchmark: sha256(binary),
    },
    ...finishMeasurement(vm, before),
  };
}

async function runBoot() {
  const vm = await RV64.create(wasm);
  configure(vm);
  const disk = new Uint8Array(
    await readFile(join(root, "web/images/root-riscv64.bin")),
  );
  let out = "";
  vm.onWrite = (_fd, bytes) => {
    out += new TextDecoder().decode(bytes);
  };
  const before = beginMeasurement(vm);
  const started = performance.now();
  vm.bootLinux({ bios, kernel, disk: disk.slice() });
  for (let i = 0; i < 200_000 && !out.includes("~ #"); i++) {
    vm.runSystem(2_000_000n);
    if ((i & 15) === 0) await tick();
  }
  if (!out.includes("~ #")) throw new Error("boot marker missing");
  return {
    row: rowKey,
    metric: "ms",
    value: performance.now() - started,
    input_sha256: {
      ...commonInputSha256,
      disk: sha256(disk),
    },
    ...finishMeasurement(vm, before),
  };
}

async function runPython() {
  const vm = await RV64.create(wasm);
  configure(vm);
  vm.ex.sys_set_wallclock(1);
  const disk = new Uint8Array(
    await readFile(join(ARTIFACTS, "deb-riscv64.ext4")),
  );
  const state = {
    out: "",
    start: "FIB_START",
    done: "FIB_DONE",
    started: null,
    finished: null,
  };
  watchMarkers(vm, state);
  vm.bootLinux({
    bios,
    kernel,
    disk: disk.slice(),
    cmdline: "console=hvc0 root=/dev/vda rw init=/binit.sh",
    ramMB: 512,
  });
  for (let i = 0; i < 400_000 && !state.out.includes("BENCH_READY"); i++) {
    vm.runSystem(3_000_000n);
    if ((i & 15) === 0) await tick();
  }
  if (!state.out.includes("BENCH_READY")) throw new Error("python boot failed");
  state.out = "";
  const before = beginMeasurement(vm);
  vm.consoleInput(enc.encode("/usr/bin/python3 /fib.py\n"));
  const guardStart = performance.now();
  for (let i = 0; i < 6_000_000 && state.finished === null; i++) {
    vm.runSystem(4_000_000n);
    if ((i & 15) === 0) await tick();
    if (performance.now() - guardStart > 300_000) break;
  }
  if (state.started === null || state.finished === null) {
    throw new Error("python markers missing");
  }
  return {
    row: rowKey,
    metric: "ms",
    value: state.finished - state.started,
    checksum: state.out.match(/fib\(30\)=\s*(\d+)/)?.[1] ?? null,
    input_sha256: {
      ...commonInputSha256,
      disk: sha256(disk),
    },
    ...finishMeasurement(vm, before),
  };
}

async function runCompile() {
  const vm = await RV64.create(wasm);
  configure(vm);
  const disk = new Uint8Array(await readFile(join(ARTIFACTS, "cc-bench.img")));
  let bootOut = "";
  vm.onWrite = (_fd, bytes) => {
    bootOut += new TextDecoder().decode(bytes);
  };
  vm.bootLinux({ bios, kernel, disk: disk.slice() });
  for (let i = 0; i < 40_000 && !bootOut.includes("~ #"); i++) {
    vm.runSystem(5_000_000n);
    if ((i & 15) === 0) await tick();
  }
  if (!bootOut.includes("~ #")) throw new Error("compile boot failed");
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n"));
  step(vm, 3_000);

  const state = {
    out: "",
    start: "RUN_START",
    done: "RUN_DONE",
    started: null,
    finished: null,
  };
  watchMarkers(vm, state);
  const before = beginMeasurement(vm);
  vm.consoleInput(
    enc.encode(
      "echo RUN_START; /tcc -c /w.c -o /w.o 2>/tmp/e; echo RUN_DONE; md5sum /w.o\n",
    ),
  );
  const guardStart = performance.now();
  for (let i = 0; i < 2_000_000; i++) {
    vm.runSystem(5_000_000n);
    if ((i & 15) === 0) await tick();
    if (state.finished !== null && /[0-9a-f]{32}/.test(state.out)) break;
    if (performance.now() - guardStart > 300_000) break;
  }
  const md5 = state.out.match(/([0-9a-f]{32})/)?.[1] ?? null;
  if (state.started === null || state.finished === null || !md5) {
    throw new Error("compile result missing");
  }
  return {
    row: rowKey,
    metric: "ms",
    value: state.finished - state.started,
    md5,
    input_sha256: {
      ...commonInputSha256,
      disk: sha256(disk),
    },
    ...finishMeasurement(vm, before),
  };
}

function parseNbench(out) {
  const values = {};
  let last = null;
  let unstable = 0;
  for (const line of out.split("\n")) {
    const named = line.match(/^([A-Z][A-Z ]+?)\s+:\s*([\d.e+]*)/);
    if (named) {
      last = named[1].trim();
      if (named[2]) {
        values[last] = +named[2];
        last = null;
      }
      continue;
    }
    const cont = line.match(/^\s+:\s+([\d.e+]+)\s+:/);
    if (cont && last) {
      values[last] = +cont[1];
      last = null;
    }
    if (
      /NOT 95 % statistically certain|variation among the individual results/.test(
        line,
      )
    ) {
      unstable++;
    }
  }
  return { values, unstable };
}

async function runNbench() {
  if (profile && !row.flag) {
    throw new Error("PROFILE=1 requires a single nbench kernel, not row=nbench");
  }
  if (profile && process.env.ALLOW_SLOW_PROFILE !== "1") {
    throw new Error(
      "per-PC nbench profiling is a multi-minute diagnostic; use exact jit counters without PROFILE, or set ALLOW_SLOW_PROFILE=1 explicitly",
    );
  }
  const vm = await RV64.create(wasm);
  configure(vm);
  vm.ex.sys_set_wallclock(1);
  const disk = new Uint8Array(
    await readFile(join(ARTIFACTS, "root-nbench.bin")),
  );
  await bootBuildroot(vm, disk);
  if (row.flag) {
    vm.consoleInput(enc.encode("echo CUSTOMRUN=T > /C\n"));
    step(vm, 400);
    if (profile) {
      // Profiling adds a branch to every dispatch and can turn calibration
      // into a multi-minute feedback loop. These command-file settings make
      // an explicitly diagnostic, non-publishable run; timing from PROFILE=1
      // is never accepted by scorecard.mjs or ab.mjs.
      vm.consoleInput(enc.encode("echo GLOBALMINTICKS=1 >> /C\n"));
      step(vm, 400);
      vm.consoleInput(enc.encode("echo MINSECONDS=1 >> /C\n"));
      step(vm, 400);
    }
    vm.consoleInput(enc.encode(`echo ${row.flag}=T >> /C\n`));
    step(vm, 400);
  }

  let out = "";
  vm.onWrite = (_fd, bytes) => {
    out += new TextDecoder().decode(bytes);
  };
  const before = beginMeasurement(vm);
  vm.consoleInput(
    enc.encode(row.flag ? "cd / && ./nbench -c/C\n" : "cd / && ./nbench\n"),
  );
  const guardStart = performance.now();
  let complete = false;
  for (let i = 0; i < 40_000_000; i++) {
    vm.runSystem(4_000_000n);
    if ((i & 3) === 0) await tick();
    if ((i & 15) === 0) {
      const values = parseNbench(out).values;
      const required = row.name ? [row.name] : NBENCH_KERNELS;
      if (required.every((name) => values[name] != null)) {
        complete = true;
        break;
      }
    }
    if (performance.now() - guardStart > nbenchTimeoutMs) break;
  }
  if (!complete) {
    throw new Error(
      `nbench did not produce every required row within ${nbenchTimeoutMs}ms; output tail:\n${out.slice(-4000)}`,
    );
  }
  const parsed = parseNbench(out);
  if (row.name && parsed.values[row.name] == null) {
    throw new Error(`${row.name} missing from nbench output`);
  }
  return {
    row: rowKey,
    metric: "iterations_per_second",
    value: row.name ? parsed.values[row.name] : parsed.values,
    unstable: parsed.unstable,
    diagnostic_shortened: profile && !!row.flag,
    timeout_ms: nbenchTimeoutMs,
    input_sha256: {
      ...commonInputSha256,
      disk: sha256(disk),
    },
    ...finishMeasurement(vm, before),
  };
}

let result;
if (row.kind === "compute") result = await runCompute();
else if (row.kind === "boot") result = await runBoot();
else if (row.kind === "python") result = await runPython();
else if (row.kind === "compile") result = await runCompile();
else result = await runNbench();

console.log(
  `RESULT_JSON ${JSON.stringify({
    ...result,
    jit_enabled: jit,
    config: {
      sb: superblocks,
      tlbfill: +(process.env.TLBFILL ?? 0),
      tracelvl: process.env.TRACELVL ?? null,
      tracewin: process.env.TRACEWIN ?? null,
      keepmin: process.env.KEEPMIN ?? null,
      demote: process.env.DEMOTE ?? null,
      ictrig: process.env.ICTRIG ?? null,
      batch: process.env.BATCH ?? null,
      deftrack: process.env.DEFTRACK ?? null,
      rotnest: process.env.ROTNEST ?? null,
      bcap: process.env.BCAP ?? null,
      bpage: process.env.BPAGE ?? null,
      sbspace: process.env.SBSPACE ?? null,
      nbench_timeout_ms: nbenchTimeoutMs,
    },
    wasm_sha256: sha256(wasm),
  })}`,
);
