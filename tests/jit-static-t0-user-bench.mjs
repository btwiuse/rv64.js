#!/usr/bin/env node

// Frozen R070 Phase-A user-mode gate. Every timed leg is a fresh, CPU-pinned
// Node/V8 process. Both legs load the same main Wasm and compile/instantiate
// the same auxiliary core before timing; only table invocation is toggled.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { cpus, loadavg, platform, release } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { median, summary } from "./statistics.mjs";
import { CPU_PROBE_SPEC, cpuProbe } from "./vs-v86/bench-math.mjs";
import { acquireBenchmarkLock } from "./vs-v86/bench-lock.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const self = fileURLToPath(import.meta.url);
const outputDir = join(root, "target/bench/r070-static-t0-phase-a");
const wasmPath = join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
const variants = ["control", "candidate"];
const corpora = Object.freeze({
  alu: Object.freeze({
    guest: "guests/syscompute/target/riscv64gc-unknown-linux-musl/release/syscompute",
    argv: ["syscompute", "alu"],
    family: "ALU/control with dense RV64C",
  }),
  memory: Object.freeze({
    guest: "guests/syscompute/target/riscv64gc-unknown-linux-musl/release/syscompute",
    argv: ["syscompute", "mix"],
    family: "scalar memory/ALU with dense RV64C",
  }),
  rvc: Object.freeze({
    argv: ["rvc-corpus"],
    family: "independent all-compressed hot loop",
    synthetic: "r070-rvc-v1",
  }),
  general: Object.freeze({
    guest: "guests/bench/target/riscv64gc-unknown-linux-musl/release/bench",
    argv: ["bench", "fast"],
    family: "held-out integer/FP general program",
  }),
});
const REQUIRED_SPEEDUP = 1.30;
const REQUIRED_LOWER_BOUND = 1.20;
const MAX_REGRESSION = 1.03;
const MAX_CONSTRUCTION_MS = 25;
const MAX_SAMPLE_SPREAD = 1.25;
const MAX_HOST_SPREAD = 1.25;
const DEFAULT_SAMPLES = 7;
const DEFAULT_CPU = "8";
const USER_MEMORY_MB = 16;
const USER_BUDGET = 20_000_000_000n;

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function elapsedMs(started) {
  return Number(process.hrtime.bigint() - started) / 1e6;
}

function spread(values) {
  const finite = values.filter((value) => Number.isFinite(value) && value > 0);
  return finite.length ? Math.max(...finite) / Math.min(...finite) : Infinity;
}

function deterministicRandom() {
  let state = 0x6d2b79f5;
  return (buffer) => {
    for (let index = 0; index < buffer.length; index++) {
      state ^= state << 13;
      state ^= state >>> 17;
      state ^= state << 5;
      buffer[index] = state >>> 24;
    }
  };
}

const encI = (opcode, f3, rd, rs1, immediate) =>
  opcode | (rd << 7) | (f3 << 12) | (rs1 << 15) | ((immediate & 0xfff) << 20);
const encU = (opcode, rd, immediate) => opcode | (rd << 7) | (immediate & 0xfffff000);
const cAddi = (rd, immediate) =>
  (((immediate >> 5) & 1) << 12) | (rd << 7) | ((immediate & 31) << 2) | 1;
const cLi = (rd, immediate) => cAddi(rd, immediate) | (0b010 << 13);
const cSlli = (rd, shift) =>
  (((shift >> 5) & 1) << 12) | (rd << 7) | ((shift & 31) << 2) | 2;
const cAdd = (rd, rs2) => (0b100 << 13) | (1 << 12) | (rd << 7) | (rs2 << 2) | 2;
const cAlu = (operation, rd, rs2) =>
  (0b100 << 13) | (0b11 << 10) | ((rd - 8) << 7) | (operation << 5) |
    ((rs2 - 8) << 2) | 1;
const cBnez = (rs1, immediate) => {
  const value = immediate & 0x1ff;
  return (0b111 << 13) | (((value >> 8) & 1) << 12) |
    (((value >> 3) & 3) << 10) | ((rs1 - 8) << 7) |
    (((value >> 6) & 3) << 5) | (((value >> 1) & 3) << 3) |
    (((value >> 5) & 1) << 2) | 1;
};

function syntheticRvcElf() {
  const bytes = [];
  const half = (instruction) => {
    bytes.push(instruction & 0xff, (instruction >>> 8) & 0xff);
  };
  const word = (instruction) => {
    bytes.push(
      instruction & 0xff,
      (instruction >>> 8) & 0xff,
      (instruction >>> 16) & 0xff,
      (instruction >>> 24) & 0xff,
    );
  };
  const iterations = 30_000_000;
  const upper = (iterations + 0x800) & ~0xfff;
  const lower = iterations - upper;
  word(encU(0x37, 15, upper));
  word(encI(0x13, 0, 15, 15, lower));
  for (const [register, value] of [[8, 1], [9, 3], [10, 5], [11, 7], [12, 11]]) {
    half(cLi(register, value));
  }
  const loop = bytes.length;
  half(cAdd(8, 9));
  half(cAlu(1, 8, 10)); // C.XOR
  half(cSlli(9, 7));
  half(cAddi(10, 3));
  half(cAlu(0, 11, 8)); // C.SUB
  half(cAdd(12, 11));
  half(cAddi(15, -1));
  const branch = bytes.length;
  half(cBnez(15, loop - branch));
  word(encI(0x13, 0, 17, 0, 93));
  word(encI(0x13, 0, 10, 0, 0));
  word(0x0000_0073);

  const elf = new Uint8Array(0x1000 + bytes.length);
  const header = new DataView(elf.buffer);
  elf.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0], 0);
  header.setUint16(0x10, 2, true);
  header.setUint16(0x12, 243, true);
  header.setUint32(0x14, 1, true);
  header.setBigUint64(0x18, 0x10000n, true);
  header.setBigUint64(0x20, 64n, true);
  header.setUint16(0x34, 64, true);
  header.setUint16(0x36, 56, true);
  header.setUint16(0x38, 1, true);
  header.setUint32(64, 1, true);
  header.setUint32(68, 7, true);
  header.setBigUint64(72, 0x1000n, true);
  header.setBigUint64(80, 0x10000n, true);
  header.setBigUint64(88, 0x10000n, true);
  header.setBigUint64(96, BigInt(bytes.length), true);
  header.setBigUint64(104, BigInt(bytes.length), true);
  header.setBigUint64(112, 0x1000n, true);
  elf.set(bytes, 0x1000);
  return Buffer.from(elf);
}

async function worker(variant, corpusName) {
  const corpus = corpora[corpusName];
  if (!variants.includes(variant) || !corpus) throw new Error("invalid worker selection");
  const [{ RV64Debug, Stop }, wasmBytes] = await Promise.all([
    import(join(root, "web/rv64.js")),
    import("node:fs/promises").then(({ readFile }) => readFile(wasmPath)),
  ]);
  const guestBytes = corpus.synthetic
    ? syntheticRvcElf()
    : readFileSync(join(root, corpus.guest));
  const hostBeforeMs = cpuProbe();
  const vm = await RV64Debug.create(wasmBytes);
  vm.onRandom = deterministicRandom();
  const output = [];
  const modules = [];
  vm.onWrite = (fd, bytes) => output.push(Buffer.concat([
    Buffer.from([fd & 0xff]),
    Buffer.from(bytes),
  ]));
  vm.onJitModule = (bytes, metadata) => modules.push({
    hash: sha256(bytes),
    bytes: bytes.length,
    metadata,
  });
  vm.ex.jit_set_enabled(0);
  if (!vm.loadElf(new Uint8Array(guestBytes), corpus.argv, USER_MEMORY_MB)) {
    throw new Error("user_load failed");
  }
  const index = vm.ex.jit_static_t0_prepare();
  if (index < 0) throw new Error("static T0 preparation failed");
  if (modules.length !== 1 || modules[0].metadata.kind !== "single") {
    throw new Error(`expected one static module, observed ${JSON.stringify(modules)}`);
  }
  const lifecycle = {
    copyMs: vm.jitCopyMs ?? 0,
    compileMs: vm.jitCompileMs ?? 0,
    instantiateMs: vm.jitInstantiateMs ?? 0,
    publishMs: vm.jitPublishMs ?? 0,
    totalMs: vm.jitRegTotalMs ?? 0,
  };
  vm.ex.jit_set_static_t0(variant === "candidate" ? 1 : 0);
  const started = process.hrtime.bigint();
  const stop = vm.runUser(USER_BUDGET);
  const runMs = elapsedMs(started);
  if (stop !== Stop.EXITED || vm.userExitCode() !== 0) {
    throw new Error(`guest did not exit cleanly: stop=${stop} exit=${vm.userExitCode()}`);
  }

  const memoryPtr = vm.ex.user_memory_ptr();
  const memoryLen = vm.ex.user_memory_len();
  const memory = new Uint8Array(vm.ex.memory.buffer, memoryPtr, memoryLen);
  const state = {
    outputSha256: sha256(Buffer.concat(output)),
    memorySha256: sha256(memory),
    pc: vm.ex.user_pc().toString(),
    instructions: vm.ex.user_insn_count().toString(),
    gpr: Array.from({ length: 32 }, (_, register) => vm.ex.user_reg(register).toString()),
    fpr: Array.from({ length: 32 }, (_, register) => vm.ex.user_freg(register).toString()),
    fcsr: vm.ex.user_fcsr(),
    exit: vm.userExitCode(),
    stop,
  };
  const staticT0 = {
    fastInstructions: vm.ex.jit_static_t0_stat(0).toString(),
    slowInstructions: vm.ex.jit_static_t0_stat(1).toString(),
    slowBatches: vm.ex.jit_static_t0_stat(2).toString(),
  };
  const hostAfterMs = cpuProbe();
  process.stdout.write(JSON.stringify({
    variant,
    corpus: corpusName,
    family: corpus.family,
    runMs,
    state,
    stateSha256: sha256(Buffer.from(JSON.stringify(state))),
    staticT0,
    lifecycle,
    auxiliary: modules[0],
    mainWasmSha256: sha256(wasmBytes),
    guestSha256: sha256(guestBytes),
    hostBeforeMs,
    hostAfterMs,
    loadAverage: loadavg(),
    affinity: readFileSync("/proc/self/status", "utf8")
      .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
    engine: { node: process.version, v8: process.versions.v8 },
  }));
}

if (process.argv.includes("--worker")) {
  const variant = process.argv.find((argument) => argument.startsWith("--variant="))
    ?.split("=")[1];
  const corpus = process.argv.find((argument) => argument.startsWith("--corpus="))
    ?.split("=")[1];
  await worker(variant, corpus);
  process.exit(0);
}

function argument(name, fallback) {
  const value = process.argv.find((item) => item.startsWith(`--${name}=`));
  return value ? value.slice(name.length + 3) : fallback;
}

const samples = Number(argument("samples", String(DEFAULT_SAMPLES)));
if (!Number.isInteger(samples) || samples < 1 || samples > 29 || !(samples & 1)) {
  throw new Error("--samples must be an odd integer from 1 through 29");
}
const cpu = argument("cpu", process.env.R070_CPU ?? DEFAULT_CPU);
if (!/^\d+$/.test(cpu)) throw new Error("--cpu must be one logical CPU number");
const selected = argument("corpora", Object.keys(corpora).join(",")).split(",");
if (!selected.length || selected.some((name) => !corpora[name])) {
  throw new Error(`--corpora must select from ${Object.keys(corpora).join(",")}`);
}
for (const required of [
  wasmPath,
  ...new Set(selected.filter((name) => corpora[name].guest)
    .map((name) => join(root, corpora[name].guest))),
]) {
  readFileSync(required);
}

mkdirSync(outputDir, { recursive: true });
const releaseLock = await acquireBenchmarkLock(resolve(join(root, "target/bench")));
const runs = Object.fromEntries(selected.map((corpus) => [
  corpus,
  Object.fromEntries(variants.map((variant) => [variant, []])),
]));
const workerFailures = [];
try {
  for (let pair = 0; pair < samples; pair++) {
    const corpusOrder = selected.map((_, offset) => selected[(pair + offset) % selected.length]);
    const variantOrder = pair & 1 ? [...variants].reverse() : variants;
    for (const corpus of corpusOrder) {
      for (const variant of variantOrder) {
        const result = spawnSync(
          "taskset",
          ["-c", cpu, process.execPath, self, "--worker", `--variant=${variant}`,
            `--corpus=${corpus}`],
          { cwd: root, encoding: "utf8", maxBuffer: 16 << 20, timeout: 300_000 },
        );
        if (result.status !== 0) {
          workerFailures.push({
            pair,
            corpus,
            variant,
            status: result.status,
            signal: result.signal,
            stdout: result.stdout?.slice(-4000),
            stderr: result.stderr?.slice(-4000),
          });
          continue;
        }
        const parsed = JSON.parse(result.stdout);
        parsed.pair = pair;
        runs[corpus][variant].push(parsed);
        console.log(
          `pair ${pair + 1}/${samples} ${corpus}/${variant}: ` +
            `${parsed.runMs.toFixed(2)} ms, ${parsed.state.instructions} instructions`,
        );
      }
    }
  }
} finally {
  await releaseLock();
}

const measurementProblems = [];
const gateFailures = [];
if (workerFailures.length) measurementProblems.push(`${workerFailures.length} worker failures`);
const aggregates = {};
for (const corpus of selected) {
  const control = runs[corpus].control;
  const candidate = runs[corpus].candidate;
  if (control.length !== samples || candidate.length !== samples) {
    measurementProblems.push(`${corpus}: incomplete samples`);
    continue;
  }
  const all = [...control, ...candidate];
  const inputHashes = new Set(all.map((run) => run.guestSha256));
  const mainHashes = new Set(all.map((run) => run.mainWasmSha256));
  const moduleHashes = new Set(all.map((run) => run.auxiliary.hash));
  const fingerprints = new Set(all.map((run) => run.stateSha256));
  const affinities = new Set(all.map((run) => run.affinity));
  if (inputHashes.size !== 1) measurementProblems.push(`${corpus}: guest input varied`);
  if (mainHashes.size !== 1) measurementProblems.push(`${corpus}: main Wasm varied`);
  if (moduleHashes.size !== 1) measurementProblems.push(`${corpus}: auxiliary Wasm varied`);
  if (fingerprints.size !== 1) measurementProblems.push(`${corpus}: architectural state differs`);
  if (affinities.size !== 1 || !affinities.has(cpu)) {
    measurementProblems.push(`${corpus}: worker affinity is not exactly CPU ${cpu}`);
  }
  const controlMs = control.map((run) => run.runMs);
  const candidateMs = candidate.map((run) => run.runMs);
  const pairedSpeedups = control.map((run, pair) => {
    const matched = candidate.find((candidateRun) => candidateRun.pair === run.pair);
    return matched ? run.runMs / matched.runMs : NaN;
  });
  const speedup = summary(pairedSpeedups);
  const construction = all.map((run) => run.lifecycle.compileMs + run.lifecycle.instantiateMs);
  aggregates[corpus] = {
    family: corpora[corpus].family,
    controlMs: summary(controlMs),
    candidateMs: summary(candidateMs),
    pairedSpeedup: speedup,
    constructionMs: summary(construction),
    controlSpread: spread(controlMs),
    candidateSpread: spread(candidateMs),
    inputSha256: [...inputHashes][0],
    mainWasmSha256: [...mainHashes][0],
    auxiliary: {
      sha256: [...moduleHashes][0],
      bytes: all[0].auxiliary.bytes,
    },
    correctnessSha256: [...fingerprints][0],
  };
  if (spread(controlMs) > MAX_SAMPLE_SPREAD || spread(candidateMs) > MAX_SAMPLE_SPREAD) {
    measurementProblems.push(`${corpus}: timing spread exceeds ${MAX_SAMPLE_SPREAD}`);
  }
  if (Math.max(...construction) > MAX_CONSTRUCTION_MS) {
    gateFailures.push(`${corpus}: auxiliary compile+instantiate exceeds ${MAX_CONSTRUCTION_MS} ms`);
  }
  if (speedup.median < REQUIRED_SPEEDUP) {
    gateFailures.push(`${corpus}: median speedup ${speedup.median.toFixed(3)} < ${REQUIRED_SPEEDUP}`);
  }
  if (speedup.medianConfidence95[0] < REQUIRED_LOWER_BOUND) {
    gateFailures.push(
      `${corpus}: lower bound ${speedup.medianConfidence95[0].toFixed(3)} < ` +
        REQUIRED_LOWER_BOUND,
    );
  }
  if (speedup.median < 1 / MAX_REGRESSION) {
    gateFailures.push(`${corpus}: regression exceeds ${Math.round((MAX_REGRESSION - 1) * 100)}%`);
  }
}

const hostProbeValues = selected.flatMap((corpus) => variants.flatMap((variant) =>
  runs[corpus][variant].flatMap((run) => [run.hostBeforeMs, run.hostAfterMs])));
const hostProbeSpread = spread(hostProbeValues);
if (hostProbeSpread > MAX_HOST_SPREAD) {
  measurementProblems.push(`host probe spread ${hostProbeSpread.toFixed(3)} > ${MAX_HOST_SPREAD}`);
}
if (samples < DEFAULT_SAMPLES) {
  gateFailures.push(`only ${samples} pairs; frozen gate requires ${DEFAULT_SAMPLES}`);
}

const report = {
  schema: 1,
  created: new Date().toISOString(),
  protocol: "R070 static hand-shaped Wasm T0 Phase A",
  methodology: "same-main-Wasm/fresh-process/pinned-CPU/alternating-paired-order/cold-aux-execution",
  samples,
  selectedCorpora: selected,
  cpu,
  thresholds: {
    requiredMedianSpeedup: REQUIRED_SPEEDUP,
    requiredPairedBootstrapLowerBound: REQUIRED_LOWER_BOUND,
    maximumRegression: MAX_REGRESSION,
    maximumCompileInstantiateMs: MAX_CONSTRUCTION_MS,
    maximumSampleSpread: MAX_SAMPLE_SPREAD,
    maximumHostProbeSpread: MAX_HOST_SPREAD,
  },
  host: {
    platform: platform(),
    release: release(),
    cpuModel: cpus()[Number(cpu)]?.model ?? null,
    node: process.version,
    v8: process.versions.v8,
    probe: CPU_PROBE_SPEC,
    probeSpread: hostProbeSpread,
  },
  measurementValid: measurementProblems.length === 0,
  gatePassed: measurementProblems.length === 0 && gateFailures.length === 0,
  measurementProblems,
  gateFailures,
  aggregates,
  runs,
  workerFailures,
};
const stamp = report.created.replace(/[:.]/g, "-");
const reportPath = join(outputDir, `static-t0-user-${stamp}.json`);
writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);

for (const [corpus, aggregate] of Object.entries(aggregates)) {
  console.log(
    `${corpus}: control=${aggregate.controlMs.median.toFixed(2)} ms, ` +
      `candidate=${aggregate.candidateMs.median.toFixed(2)} ms, ` +
      `paired=${aggregate.pairedSpeedup.median.toFixed(3)}x ` +
      `CI=[${aggregate.pairedSpeedup.medianConfidence95.map((value) => value.toFixed(3)).join(", ")}], ` +
      `compile+instantiate=${aggregate.constructionMs.median.toFixed(3)} ms`,
  );
}
console.log(`host probe spread=${hostProbeSpread.toFixed(3)}`);
console.log(report.gatePassed ? "R070 PHASE A: PASS" : "R070 PHASE A: FAIL");
if (measurementProblems.length) console.log(`invalid: ${measurementProblems.join("; ")}`);
if (gateFailures.length) console.log(`gate: ${gateFailures.join("; ")}`);
console.log(`saved ${reportPath}`);
if (!report.gatePassed) process.exitCode = 1;
