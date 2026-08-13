#!/usr/bin/env node

// R117 proof-only model: compare one function with 31 long-lived i64 locals
// against the normalized-equivalent function with 31 private mutable globals.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { loadavg } from "node:os";
import { summary } from "../statistics.mjs";
import { CPU_PROBE_SPEC, cpuProbe } from "./bench-math.mjs";
import { acquireBenchmarkLock } from "./bench-lock.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");
const self = fileURLToPath(import.meta.url);
const variants = ["local", "global"];
const PAIRS = 15;
const CPUS = Object.freeze([8, 9, 10, 11, 12, 13, 14, 15]);
const STATE_VALUES = 31;
const MEMORY_BASE = 4096;
const FIRST_ITERATIONS = 4_096;
const WARM_ITERATIONS = 16_384;
const WARM_CALLS = 8;
const STEADY_ITERATIONS = 65_536;
const STEADY_CALLS = 7;
const MAX_HOST_SPREAD = 1.10;
const MAX_SAMPLE_SPREAD = 1.25;
const MIN_STEADY_SPEEDUP = 1.15;
const MIN_STEADY_LOWER = 1.00;
const MIN_FIRST_SPEEDUP = 0.99;
const MASK64 = (1n << 64n) - 1n;

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const elapsedMs = (started) => Number(process.hrtime.bigint() - started) / 1e6;
const spread = (values) => {
  const finite = values.filter((value) => Number.isFinite(value) && value > 0);
  return finite.length ? Math.max(...finite) / Math.min(...finite) : Infinity;
};
const turn = () => new Promise((resolveTurn) => setImmediate(resolveTurn));

function initialState() {
  let seed = 0x243f6a8885a308d3n;
  return Array.from({ length: STATE_VALUES }, (_, index) => {
    seed = (seed + 0x9e3779b97f4a7c15n) & MASK64;
    let value = seed;
    value = ((value ^ (value >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK64;
    value = ((value ^ (value >> 27n)) * 0x94d049bb133111ebn) & MASK64;
    return (value ^ (value >> 31n) ^ BigInt(index + 1)) & MASK64;
  });
}

const INITIAL_STATE = Object.freeze(initialState());

function reset(memory) {
  const view = new DataView(memory.buffer);
  for (let index = 0; index < STATE_VALUES; index++) {
    view.setBigUint64(MEMORY_BASE + index * 8, INITIAL_STATE[index], true);
  }
}

function stateBytes(memory) {
  return Buffer.from(memory.buffer, MEMORY_BASE, STATE_VALUES * 8);
}

function timedCall(memory, run, iterations) {
  reset(memory);
  const started = process.hrtime.bigint();
  const result = run(MEMORY_BASE, iterations);
  const ms = elapsedMs(started);
  const bytes = stateBytes(memory);
  const stateSha256 = sha256(bytes);
  const resultHex = `0x${BigInt.asUintN(64, result).toString(16).padStart(16, "0")}`;
  return {
    iterations,
    ms,
    result: resultHex,
    stateSha256,
    fingerprint: sha256(Buffer.concat([bytes, Buffer.from(resultHex)])),
  };
}

function affinity() {
  return readFileSync("/proc/self/status", "utf8")
    .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null;
}

async function worker(variant, modelDirectory) {
  if (!variants.includes(variant)) throw new Error(`invalid variant ${variant}`);
  const bytes = readFileSync(join(modelDirectory, `${variant}.wasm`));
  if (!WebAssembly.validate(bytes)) throw new Error(`${variant} model does not validate`);
  const hostBeforeMs = cpuProbe();
  let started = process.hrtime.bigint();
  const module = new WebAssembly.Module(bytes);
  const compileMs = elapsedMs(started);
  started = process.hrtime.bigint();
  const instance = new WebAssembly.Instance(module);
  const instantiateMs = elapsedMs(started);
  const { memory, run } = instance.exports;
  if (!(memory instanceof WebAssembly.Memory) || typeof run !== "function") {
    throw new Error(`${variant} exports are incomplete`);
  }

  const first = timedCall(memory, run, FIRST_ITERATIONS);
  await turn();
  const warm = [];
  for (let call = 0; call < WARM_CALLS; call++) {
    warm.push(timedCall(memory, run, WARM_ITERATIONS));
    await turn();
  }
  const steady = [];
  for (let call = 0; call < STEADY_CALLS; call++) {
    steady.push(timedCall(memory, run, STEADY_ITERATIONS));
    await turn();
  }
  const hostAfterMs = cpuProbe();
  const uniqueWarm = new Set(warm.map((sample) => sample.fingerprint));
  const uniqueSteady = new Set(steady.map((sample) => sample.fingerprint));
  if (uniqueWarm.size !== 1 || uniqueSteady.size !== 1) {
    throw new Error(`${variant} output is nondeterministic within one process`);
  }
  const steadyMs = steady.map((sample) => sample.ms);
  process.stdout.write(JSON.stringify({
    variant,
    runtime: { node: process.version, v8: process.versions.v8 },
    affinity: affinity(),
    loadAverage: loadavg(),
    wasm: { bytes: bytes.length, sha256: sha256(bytes) },
    hostBeforeMs,
    hostAfterMs,
    compileMs,
    instantiateMs,
    first,
    warm,
    steady,
    steadyMedianMs: summary(steadyMs).median,
    steadySpread: spread(steadyMs),
  }));
}

if (process.argv.includes("--worker")) {
  const variant = process.argv.find((item) => item.startsWith("--variant="))?.split("=")[1];
  const modelDirectory = process.argv.find((item) => item.startsWith("--model="))
    ?.slice("--model=".length);
  if (!modelDirectory) throw new Error("R117 worker is missing --model");
  await worker(variant, modelDirectory);
  process.exit(0);
}

function emitModel(directory) {
  mkdirSync(directory, { recursive: true });
  const child = spawnSync("cargo", [
    "run", "--release", "-q", "-p", "rv64-dbt", "--example",
    "r117_module_global_model", "--", directory,
  ], { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 });
  if (child.status !== 0) {
    throw new Error(child.stderr || child.stdout || "R117 model generation failed");
  }
}

function parseShape(path) {
  const lines = readFileSync(path, "utf8").trim().split(/\r?\n/);
  const header = lines[0].split("\t");
  const result = { state: {} };
  for (const line of lines.slice(1)) {
    const fields = line.split("\t");
    if (fields[0] === "local" || fields[0] === "global") {
      result[fields[0]] = Object.fromEntries(header.slice(1).map((name, index) =>
        [name, Number(fields[index + 1])]));
    } else {
      const match = /^(local|global)\.state\.(\d+)$/.exec(fields[0]);
      if (!match) throw new Error(`invalid shape row ${fields[0]}`);
      result.state[`${match[1]}.${match[2]}`] = {
        get: Number(fields[1]),
        set: Number(fields[2]),
      };
    }
  }
  return result;
}

function artifact(directory, name) {
  const bytes = readFileSync(join(directory, name));
  return { bytes: bytes.length, sha256: sha256(bytes) };
}

const outputArg = process.argv.find((item) => item.startsWith("--output="));
const output = outputArg?.slice("--output=".length) ??
  join(root, "target/bench/r117-module-global/gate.json");
const experimentDirectory = dirname(output);
const modelDirectory = join(experimentDirectory, "model");
const repeatDirectory = join(experimentDirectory, "model-repeat");
mkdirSync(experimentDirectory, { recursive: true });
rmSync(repeatDirectory, { recursive: true, force: true });
emitModel(modelDirectory);
emitModel(repeatDirectory);

const artifactNames = [
  "local.wasm",
  "global.wasm",
  "local.normalized.txt",
  "global.normalized.txt",
  "shape.tsv",
];
const artifacts = Object.fromEntries(artifactNames.map((name) => {
  const primary = artifact(modelDirectory, name);
  const repeat = artifact(repeatDirectory, name);
  return [name, { ...primary, repeatSha256: repeat.sha256, deterministic: primary.sha256 === repeat.sha256 }];
}));
const shape = parseShape(join(modelDirectory, "shape.tsv"));
const normalizedEqual = artifacts["local.normalized.txt"].sha256 ===
  artifacts["global.normalized.txt"].sha256;
const indexedStateEqual = Array.from({ length: STATE_VALUES }, (_, index) => {
  const local = shape.state[`local.${index}`];
  const global = shape.state[`global.${index}`];
  return local && global && local.get === global.get && local.set === global.set &&
    local.get > 0 && local.set > 0;
}).every(Boolean);

const releaseLock = await acquireBenchmarkLock(join(root, "target/bench"));
const pairs = [];
const workerFailures = [];
try {
  for (let pair = 0; pair < PAIRS; pair++) {
    const cpu = CPUS[pair % CPUS.length];
    const order = pair & 1 ? ["global", "local"] : ["local", "global"];
    const pairResult = { pair: pair + 1, cpu, order };
    for (const variant of order) {
      const child = spawnSync("taskset", [
        "-c", String(cpu), process.execPath, self, "--worker",
        `--variant=${variant}`, `--model=${modelDirectory}`,
      ], { cwd: root, encoding: "utf8", maxBuffer: 8 << 20, timeout: 180_000 });
      if (child.status !== 0) {
        workerFailures.push({
          pair: pair + 1,
          cpu,
          variant,
          status: child.status,
          signal: child.signal,
          stdout: child.stdout?.slice(-4000),
          stderr: child.stderr?.slice(-4000),
        });
        continue;
      }
      pairResult[variant] = JSON.parse(child.stdout);
      console.log(
        `R117 pair ${pair + 1}/${PAIRS} ${variant} cpu${cpu}: ` +
        `${pairResult[variant].steadyMedianMs.toFixed(3)} ms`,
      );
    }
    pairs.push(pairResult);
  }
} finally {
  await releaseLock();
}

const completePairs = pairs.filter((pair) => pair.local && pair.global);
const localSteady = completePairs.map((pair) => pair.local.steadyMedianMs);
const globalSteady = completePairs.map((pair) => pair.global.steadyMedianMs);
const steadySpeedups = completePairs.map((pair) =>
  pair.local.steadyMedianMs / pair.global.steadyMedianMs);
const firstSpeedups = completePairs.map((pair) => pair.local.first.ms / pair.global.first.ms);
const hostProbes = completePairs.flatMap((pair) => [
  pair.local.hostBeforeMs,
  pair.local.hostAfterMs,
  pair.global.hostBeforeMs,
  pair.global.hostAfterMs,
]);
const allRuns = completePairs.flatMap((pair) => [pair.local, pair.global]);
const fingerprint = (variant, phase) => new Set(completePairs.map((pair) => {
  const run = pair[variant];
  if (phase === "first") return run.first.fingerprint;
  return run[phase][0].fingerprint;
}));
const crossVariantFingerprints = ["first", "warm", "steady"].every((phase) => {
  const local = fingerprint("local", phase);
  const global = fingerprint("global", phase);
  return local.size === 1 && global.size === 1 && [...local][0] === [...global][0];
});
const steady = summary(steadySpeedups);
const first = summary(firstSpeedups);
const shapeChecks = {
  deterministicArtifacts: Object.values(artifacts).every((item) => item.deterministic),
  validates: WebAssembly.validate(readFileSync(join(modelDirectory, "local.wasm"))) &&
    WebAssembly.validate(readFileSync(join(modelDirectory, "global.wasm"))),
  normalizedOperatorStreamsEqual: normalizedEqual,
  indexedStateAccessEqual: indexedStateEqual,
  localShape: shape.local?.mutable_i64_globals === 0 &&
    shape.local?.local_i64 === STATE_VALUES + 2 && shape.local?.local_i32 === 1,
  globalShape: shape.global?.mutable_i64_globals === STATE_VALUES &&
    shape.global?.local_i64 === 2 && shape.global?.local_i32 === 1,
  sameFunctionAndOperatorCounts: shape.local?.functions === 1 &&
    shape.global?.functions === 1 && shape.local?.operators === shape.global?.operators,
  sameMemoryOperations: shape.local?.memory_load === shape.global?.memory_load &&
    shape.local?.memory_store === shape.global?.memory_store,
};
const measurementChecks = {
  completePairs: completePairs.length === PAIRS && workerFailures.length === 0,
  exactAffinity: allRuns.every((run, index) => {
    const pair = completePairs[Math.floor(index / 2)];
    return run.affinity === String(pair.cpu);
  }),
  exactArtifacts: completePairs.every((pair) =>
    pair.local.wasm.sha256 === artifacts["local.wasm"].sha256 &&
    pair.global.wasm.sha256 === artifacts["global.wasm"].sha256),
  exactOutputAndState: crossVariantFingerprints,
  exactSchedule: allRuns.every((run) =>
    run.first.iterations === FIRST_ITERATIONS &&
    run.warm.length === WARM_CALLS &&
    run.warm.every((sample) => sample.iterations === WARM_ITERATIONS) &&
    run.steady.length === STEADY_CALLS &&
    run.steady.every((sample) => sample.iterations === STEADY_ITERATIONS)),
  hostSpread: spread(hostProbes) <= MAX_HOST_SPREAD,
  localSideSpread: spread(localSteady) <= MAX_SAMPLE_SPREAD,
  globalSideSpread: spread(globalSteady) <= MAX_SAMPLE_SPREAD,
  withinProcessSpread: allRuns.every((run) => run.steadySpread <= MAX_SAMPLE_SPREAD),
};
const performanceChecks = {
  steadyMedian: steady.median >= MIN_STEADY_SPEEDUP,
  steadyLowerBound: steady.medianConfidence95[0] >= MIN_STEADY_LOWER,
  firstMedian: first.median >= MIN_FIRST_SPEEDUP,
};
const checks = { ...shapeChecks, ...measurementChecks, ...performanceChecks };
const pass = Object.values(checks).every(Boolean);
const report = {
  schema: 1,
  experiment: "R117 module-global architectural-state model",
  productionModified: false,
  methodology: "normalized-equivalent-module/fresh-process/CPU-pinned/alternating-pairs/ordinary-V8",
  frozen: {
    pairs: PAIRS,
    cpus: CPUS,
    stateValues: STATE_VALUES,
    memoryBase: MEMORY_BASE,
    firstIterations: FIRST_ITERATIONS,
    warmIterations: WARM_ITERATIONS,
    warmCalls: WARM_CALLS,
    steadyIterations: STEADY_ITERATIONS,
    steadyCalls: STEADY_CALLS,
    maximumHostSpread: MAX_HOST_SPREAD,
    maximumSampleSpread: MAX_SAMPLE_SPREAD,
    minimumSteadySpeedup: MIN_STEADY_SPEEDUP,
    minimumSteadyLowerBound: MIN_STEADY_LOWER,
    minimumFirstSpeedup: MIN_FIRST_SPEEDUP,
    cpuProbe: CPU_PROBE_SPEC,
  },
  runtime: allRuns[0]?.runtime ?? null,
  artifacts,
  shape,
  pairs,
  workerFailures,
  observed: {
    localSteadyMs: summary(localSteady),
    globalSteadyMs: summary(globalSteady),
    steadyPairedSpeedup: steady,
    firstPairedSpeedup: first,
    hostProbeMs: summary(hostProbes),
    hostProbeSpread: spread(hostProbes),
    localSideSpread: spread(localSteady),
    globalSideSpread: spread(globalSteady),
    compileMs: {
      local: summary(completePairs.map((pair) => pair.local.compileMs)),
      global: summary(completePairs.map((pair) => pair.global.compileMs)),
    },
    instantiateMs: {
      local: summary(completePairs.map((pair) => pair.local.instantiateMs)),
      global: summary(completePairs.map((pair) => pair.global.instantiateMs)),
    },
  },
  checks,
  pass,
  decision: pass
    ? "admit-natural-optimized-native-frame-gate"
    : "close-module-global-state-before-native-or-product-work",
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(
  `R117 steady ${steady.median.toFixed(4)}x ` +
  `[${steady.medianConfidence95.map((value) => value.toFixed(4)).join(", ")}]`,
);
console.log(
  `R117 first ${first.median.toFixed(4)}x; host ${spread(hostProbes).toFixed(4)}x; ` +
  `pass=${pass}`,
);
console.log(`R117 report: ${output}`);
if (!pass) process.exitCode = 1;
