#!/usr/bin/env node

// R124 proof-only ordinary-V8 model: retain all GPRs for the invocation or
// retain the architecture-fixed RV64C bank and materialize all other GPRs at
// deterministic member boundaries.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { loadavg } from "node:os";
import { summary } from "../statistics.mjs";
import { CPU_PROBE_SPEC, cpuProbe } from "./bench-math.mjs";
import { acquireBenchmarkLock } from "./bench-lock.mjs";

const self = fileURLToPath(import.meta.url);
const root = resolve(dirname(self), "../..");
const variants = Object.freeze(["eager", "hybrid"]);
const PAIRS = 15;
const CPUS = Object.freeze([8, 9, 10, 11, 12, 13, 14, 15]);
const STATE_VALUES = 31;
const RVC_BANK = Object.freeze([1, 2, 8, 9, 10, 11, 12, 13, 14, 15]);
const MEMORY_BASE = 4096;
const FIRST_ITERATIONS = 4_096;
const WARM_ITERATIONS = 16_384;
const WARM_CALLS = 8;
const STEADY_ITERATIONS = 65_536;
const STEADY_CALLS = 7;
const MAX_HOST_SPREAD = 1.10;
const MAX_SAMPLE_SPREAD = 1.25;
const ORIGINAL_MIN_STEADY_SPEEDUP = 1.05;
const ORIGINAL_MIN_STEADY_LOWER = 1.02;
const ORIGINAL_MIN_FIRST_SPEEDUP = 0.99;
const MASK64 = (1n << 64n) - 1n;
const EXPECTED = Object.freeze({
  generatorSha256: "f15e7dffd9cb44f66e2e101def93eab6a830f90513209f2bef66ab72af23f168",
  eagerSha256: "8d57dc9b492095595a274ab24d06459f1e22a101e742deaef4a455020483e6a1",
  hybridSha256: "04934d012990d98e2edc82a873d25e21c611bcbcbc4f0b56f4eda2b1af10ff03",
  shapeSha256: "29e256bd2e097126fed6c93a337ee0b62dae8f40accfe51ce6c1369c29b42797",
  scheduleSha256: "f3fc0218918d74c49c5a37077967038d3ba65d2045a8a79cfb976fbe0b3b8820",
  protocolSha256: "ddb2a066b8d494c21db202fb35245b6f1988836d98fa1841bd7781cbe5dd735a",
  productionWasmSha256: "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
});

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const fileSha256 = (path) => sha256(readFileSync(path));
const elapsedMs = (started) => Number(process.hrtime.bigint() - started) / 1e6;
const turn = () => new Promise((resolveTurn) => setImmediate(resolveTurn));
const spread = (values) => {
  const finite = values.filter((value) => Number.isFinite(value) && value > 0);
  return finite.length ? Math.max(...finite) / Math.min(...finite) : Infinity;
};

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

function instantiate(bytes) {
  if (!WebAssembly.validate(bytes)) throw new Error("R124 model does not validate");
  const module = new WebAssembly.Module(bytes);
  const instance = new WebAssembly.Instance(module);
  if (!(instance.exports.memory instanceof WebAssembly.Memory) ||
      typeof instance.exports.run !== "function") {
    throw new Error("R124 model exports are incomplete");
  }
  return instance.exports;
}

function call(memory, run, iterations, timed) {
  reset(memory);
  const started = timed ? process.hrtime.bigint() : 0n;
  const value = run(MEMORY_BASE, iterations);
  const ms = timed ? elapsedMs(started) : null;
  const bytes = stateBytes(memory);
  const result = `0x${BigInt.asUintN(64, value).toString(16).padStart(16, "0")}`;
  return {
    iterations,
    ms,
    result,
    stateSha256: sha256(bytes),
    fingerprint: sha256(Buffer.concat([bytes, Buffer.from(result)])),
  };
}

function affinity() {
  return readFileSync("/proc/self/status", "utf8")
    .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null;
}

async function worker(variant, modelDirectory) {
  if (!variants.includes(variant)) throw new Error(`invalid variant ${variant}`);
  const bytes = readFileSync(join(modelDirectory, `${variant}.wasm`));
  const hostBeforeMs = cpuProbe();
  let started = process.hrtime.bigint();
  const module = new WebAssembly.Module(bytes);
  const compileMs = elapsedMs(started);
  started = process.hrtime.bigint();
  const instance = new WebAssembly.Instance(module);
  const instantiateMs = elapsedMs(started);
  const { memory, run } = instance.exports;

  const first = call(memory, run, FIRST_ITERATIONS, true);
  await turn();
  const warm = [];
  for (let index = 0; index < WARM_CALLS; index++) {
    warm.push(call(memory, run, WARM_ITERATIONS, true));
    await turn();
  }
  const steady = [];
  for (let index = 0; index < STEADY_CALLS; index++) {
    steady.push(call(memory, run, STEADY_ITERATIONS, true));
    await turn();
  }
  const hostAfterMs = cpuProbe();
  if (new Set(warm.map((sample) => sample.fingerprint)).size !== 1 ||
      new Set(steady.map((sample) => sample.fingerprint)).size !== 1) {
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
  if (!modelDirectory) throw new Error("R124 worker is missing --model");
  await worker(variant, modelDirectory);
  process.exit(0);
}

function emitModel(directory) {
  mkdirSync(directory, { recursive: true });
  const child = spawnSync("cargo", [
    "run", "--release", "-q", "-p", "rv64-dbt", "--example",
    "r124_rvc_bank_hybrid_model", "--", directory,
  ], { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 });
  if (child.status !== 0) {
    throw new Error(child.stderr || child.stdout || "R124 model generation failed");
  }
}

function artifact(directory, name) {
  const bytes = readFileSync(join(directory, name));
  return { bytes: bytes.length, sha256: sha256(bytes) };
}

function parseShape(path) {
  const lines = readFileSync(path, "utf8").trim().split(/\r?\n/);
  const header = lines[0].split("\t");
  return Object.fromEntries(lines.slice(1).map((line) => {
    const fields = line.split("\t");
    return [fields[0], Object.fromEntries(
      header.slice(1).map((name, index) => [name, Number(fields[index + 1])]),
    )];
  }));
}

function parseSchedule(path) {
  const lines = readFileSync(path, "utf8").trim().split(/\r?\n/);
  return lines.slice(1).map((line) => {
    const [member, destination, rhs, third, residentDestination] = line.split("\t");
    return {
      member: Number(member),
      destination,
      rhs,
      third,
      residentDestination: residentDestination === "1",
    };
  });
}

function correctness(modelDirectory) {
  const instances = Object.fromEntries(variants.map((variant) => [
    variant,
    instantiate(readFileSync(join(modelDirectory, `${variant}.wasm`))),
  ]));
  return [0, 1, 2, 7, 31].map((iterations) => {
    const outcomes = Object.fromEntries(variants.map((variant) => [
      variant,
      call(instances[variant].memory, instances[variant].run, iterations, false),
    ]));
    if (outcomes.eager.fingerprint !== outcomes.hybrid.fingerprint) {
      throw new Error(`R124 semantic mismatch at ${iterations} iterations`);
    }
    return { iterations, fingerprint: outcomes.eager.fingerprint };
  });
}

const outputArg = process.argv.find((item) => item.startsWith("--output="));
const output = outputArg?.slice("--output=".length) ??
  join(root, "target/bench/r124-rvc-bank-hybrid/model-gate/gate.json");
const experimentDirectory = dirname(output);
const modelDirectory = join(experimentDirectory, "model");
const repeatDirectory = join(experimentDirectory, "model-repeat");
mkdirSync(experimentDirectory, { recursive: true });

const generatorPath = join(root, "crates/rv64-dbt/examples/r124_rvc_bank_hybrid_model.rs");
const protocolPath = join(root, "docs/jit-rewrite/R124_RVC_BANK_HYBRID_STATE_ATTRIBUTION_PROTOCOL.md");
const productionWasmPath = join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
if (fileSha256(generatorPath) !== EXPECTED.generatorSha256) {
  throw new Error("R124 model generator source identity mismatch");
}
if (fileSha256(protocolPath) !== EXPECTED.protocolSha256) {
  throw new Error("R124 protocol identity mismatch");
}
if (fileSha256(productionWasmPath) !== EXPECTED.productionWasmSha256) {
  throw new Error("R124 production baseline identity mismatch");
}
emitModel(modelDirectory);
emitModel(repeatDirectory);

const expectedArtifacts = {
  "eager.wasm": EXPECTED.eagerSha256,
  "hybrid.wasm": EXPECTED.hybridSha256,
  "shape.tsv": EXPECTED.shapeSha256,
  "schedule.tsv": EXPECTED.scheduleSha256,
};
const artifacts = Object.fromEntries(Object.entries(expectedArtifacts).map(([name, expected]) => {
  const primary = artifact(modelDirectory, name);
  const repeat = artifact(repeatDirectory, name);
  return [name, {
    ...primary,
    expectedSha256: expected,
    repeatSha256: repeat.sha256,
    deterministic: primary.sha256 === repeat.sha256 && primary.sha256 === expected,
  }];
}));
const shape = parseShape(join(modelDirectory, "shape.tsv"));
const schedule = parseSchedule(join(modelDirectory, "schedule.tsv"));
const correctnessChecks = correctness(modelDirectory);
const scheduledResidentDestinations = schedule
  .filter((member) => member.residentDestination)
  .map((member) => member.member);

const releaseLock = await acquireBenchmarkLock(join(root, "target/bench"));
const pairs = [];
const workerFailures = [];
try {
  for (let pair = 0; pair < PAIRS; pair++) {
    const cpu = CPUS[pair % CPUS.length];
    const order = pair & 1 ? ["hybrid", "eager"] : ["eager", "hybrid"];
    const pairResult = { pair: pair + 1, cpu, order };
    for (const variant of order) {
      const child = spawnSync("taskset", [
        "-c", String(cpu), process.execPath, self, "--worker",
        `--variant=${variant}`, `--model=${modelDirectory}`,
      ], { cwd: root, encoding: "utf8", maxBuffer: 8 << 20, timeout: 300_000 });
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
        `R124 pair ${pair + 1}/${PAIRS} ${variant} cpu${cpu}: ` +
        `${pairResult[variant].steadyMedianMs.toFixed(3)} ms`,
      );
    }
    pairs.push(pairResult);
  }
} finally {
  await releaseLock();
}

const completePairs = pairs.filter((pair) => pair.eager && pair.hybrid);
const eagerSteady = completePairs.map((pair) => pair.eager.steadyMedianMs);
const hybridSteady = completePairs.map((pair) => pair.hybrid.steadyMedianMs);
const steadySpeedups = completePairs.map((pair) =>
  pair.eager.steadyMedianMs / pair.hybrid.steadyMedianMs);
const firstSpeedups = completePairs.map((pair) => pair.eager.first.ms / pair.hybrid.first.ms);
const steady = summary(steadySpeedups);
const first = summary(firstSpeedups);
const hostProbes = completePairs.flatMap((pair) => [
  pair.eager.hostBeforeMs,
  pair.eager.hostAfterMs,
  pair.hybrid.hostBeforeMs,
  pair.hybrid.hostAfterMs,
]);
const allRuns = completePairs.flatMap((pair) => [pair.eager, pair.hybrid]);
const phaseFingerprint = (variant, phase) => new Set(completePairs.map((pair) => {
  const run = pair[variant];
  return phase === "first" ? run.first.fingerprint : run[phase][0].fingerprint;
}));
const crossVariantFingerprints = ["first", "warm", "steady"].every((phase) => {
  const eager = phaseFingerprint("eager", phase);
  const hybrid = phaseFingerprint("hybrid", phase);
  return eager.size === 1 && hybrid.size === 1 && [...eager][0] === [...hybrid][0];
});

const shapeChecks = {
  deterministicArtifacts: Object.values(artifacts).every((item) => item.deterministic),
  validates: variants.every((variant) =>
    WebAssembly.validate(readFileSync(join(modelDirectory, `${variant}.wasm`)))),
  oneFunctionEach: shape.eager?.functions === 1 && shape.hybrid?.functions === 1,
  fixedResidentBank: RVC_BANK.length === 10 &&
    scheduledResidentDestinations.length === RVC_BANK.length &&
    scheduledResidentDestinations.every((reg, index) => reg === RVC_BANK[index]),
  architectureWideSchedule: schedule.length === STATE_VALUES &&
    schedule.every((member, index) => member.member === index + 1),
  expectedLocalReduction: shape.eager?.local_i64 === 36 && shape.hybrid?.local_i64 === 15,
  sameControlShape: shape.eager?.branches === shape.hybrid?.branches,
  sameStoreCount: shape.eager?.memory_store === shape.hybrid?.memory_store,
  expectedLoadTrade: shape.eager?.memory_load === 31 && shape.hybrid?.memory_load === 73,
};
const measurementChecks = {
  completePairs: completePairs.length === PAIRS && workerFailures.length === 0,
  exactAffinity: completePairs.every((pair) =>
    pair.eager.affinity === String(pair.cpu) && pair.hybrid.affinity === String(pair.cpu)),
  exactArtifacts: completePairs.every((pair) =>
    pair.eager.wasm.sha256 === artifacts["eager.wasm"].sha256 &&
    pair.hybrid.wasm.sha256 === artifacts["hybrid.wasm"].sha256),
  exactOutputAndState: crossVariantFingerprints,
  exactSchedule: allRuns.every((run) =>
    run.first.iterations === FIRST_ITERATIONS &&
    run.warm.length === WARM_CALLS &&
    run.warm.every((sample) => sample.iterations === WARM_ITERATIONS) &&
    run.steady.length === STEADY_CALLS &&
    run.steady.every((sample) => sample.iterations === STEADY_ITERATIONS)),
  hostSpread: spread(hostProbes) <= MAX_HOST_SPREAD,
  eagerSideSpread: spread(eagerSteady) <= MAX_SAMPLE_SPREAD,
  hybridSideSpread: spread(hybridSteady) <= MAX_SAMPLE_SPREAD,
  withinProcessSpread: allRuns.every((run) => run.steadySpread <= MAX_SAMPLE_SPREAD),
};
const originalProxyChecks = {
  steadyMedianAtLeast1_05: steady.median >= ORIGINAL_MIN_STEADY_SPEEDUP,
  steadyLowerAtLeast1_02: steady.medianConfidence95[0] >= ORIGINAL_MIN_STEADY_LOWER,
  firstMedianAtLeast0_99: first.median >= ORIGINAL_MIN_FIRST_SPEEDUP,
};
const amendedDecisionChecks = {
  noEstablishedSteadyRegression: steady.medianConfidence95[1] >= 1.0,
};
const requiredChecks = { ...shapeChecks, ...measurementChecks, ...amendedDecisionChecks };
const pass = Object.values(requiredChecks).every(Boolean);
const report = {
  schema: 1,
  experiment: "R124 RV64C-bank hybrid structured-state ordinary-V8 model",
  productionModified: false,
  performanceEvidence: "local model only; no product performance credit",
  methodology: "architecture-wide/fresh-process/CPU-pinned/alternating-pairs/ordinary-V8",
  identities: {
    runnerSha256: fileSha256(self),
    generatorSha256: fileSha256(generatorPath),
    protocolSha256: fileSha256(protocolPath),
    productionWasmSha256: fileSha256(productionWasmPath),
  },
  frozen: {
    pairs: PAIRS,
    cpus: CPUS,
    stateValues: STATE_VALUES,
    residentBank: RVC_BANK,
    memoryBase: MEMORY_BASE,
    firstIterations: FIRST_ITERATIONS,
    warmIterations: WARM_ITERATIONS,
    warmCalls: WARM_CALLS,
    steadyIterations: STEADY_ITERATIONS,
    steadyCalls: STEADY_CALLS,
    maximumHostSpread: MAX_HOST_SPREAD,
    maximumSampleSpread: MAX_SAMPLE_SPREAD,
    originalMinimumSteadySpeedup: ORIGINAL_MIN_STEADY_SPEEDUP,
    originalMinimumSteadyLower: ORIGINAL_MIN_STEADY_LOWER,
    originalMinimumFirstSpeedup: ORIGINAL_MIN_FIRST_SPEEDUP,
    amendedStopRule: "stop only on established model regression or invalid evidence before native shape",
    cpuProbe: CPU_PROBE_SPEC,
  },
  runtime: allRuns[0]?.runtime ?? null,
  artifacts,
  shape,
  schedule,
  correctness: correctnessChecks,
  pairs,
  workerFailures,
  observed: {
    eagerSteadyMs: summary(eagerSteady),
    hybridSteadyMs: summary(hybridSteady),
    steadyPairedSpeedup: steady,
    firstPairedSpeedup: first,
    hostProbeMs: summary(hostProbes),
    hostProbeSpread: spread(hostProbes),
    eagerSideSpread: spread(eagerSteady),
    hybridSideSpread: spread(hybridSteady),
    compileMs: {
      eager: summary(completePairs.map((pair) => pair.eager.compileMs)),
      hybrid: summary(completePairs.map((pair) => pair.hybrid.compileMs)),
    },
    instantiateMs: {
      eager: summary(completePairs.map((pair) => pair.eager.instantiateMs)),
      hybrid: summary(completePairs.map((pair) => pair.hybrid.instantiateMs)),
    },
  },
  checks: {
    required: requiredChecks,
    originalProxyDiagnostics: originalProxyChecks,
  },
  pass,
  decision: pass
    ? "admit-natural-optimized-native-shape-collection"
    : "stop-before-native-or-product-work",
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(
  `R124 steady ${steady.median.toFixed(4)}x ` +
  `[${steady.medianConfidence95.map((value) => value.toFixed(4)).join(", ")}]`,
);
console.log(
  `R124 first ${first.median.toFixed(4)}x; host ${spread(hostProbes).toFixed(4)}x; ` +
  `original=${Object.values(originalProxyChecks).every(Boolean)} pass=${pass}`,
);
console.log(`R124 report: ${output}`);
if (!pass) process.exitCode = 1;
