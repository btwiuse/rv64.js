#!/usr/bin/env node

// Frozen G002 standalone opportunity gate. The complete-local GPR model,
// balanced schedule, process order, work, statistics, and thresholds were
// fixed before this script's first execution.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { loadavg } from "node:os";
import { summary } from "../statistics.mjs";
import { CPU_PROBE_SPEC, cpuProbe } from "./bench-math.mjs";
import { acquireBenchmarkLock } from "./bench-lock.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");
const self = fileURLToPath(import.meta.url);
const variants = Object.freeze(["control", "treatment"]);
const PAIRS = 7;
const CPUS = Object.freeze([8, 9, 10, 11, 12, 13, 14, 15]);
const PREWARM_ROUNDS = 1;
const WARM_ROUNDS = 4_096;
const WARM_CALLS = 3;
const STEADY_ROUNDS = 16_384;
const STEADY_CALLS = 7;
const MAX_HOST_SPREAD = 1.25;
const MAX_SAMPLE_SPREAD = 1.25;
const MIN_SPEEDUP = 3.75;
const MIN_LOWER_BOUND = 3.50;
const MAX_TREATMENT_COMPILE_INSTANTIATE_MS = 25;
const EXPECTED = Object.freeze({
  sourceSha256: "0d477252985d1a5681f2fa358d535c71a758efd9b6d27be43be70b28681f7f13",
  preparationHarnessSha256: "b0f9bc11d708716770faee0666113b04f3687b73b4558050b40097e4a9891372",
  modelSha256: "d3dd92bad1792340d8bf618a4c30595af87f49b76ab81de9343f32c59e764a56",
  freezeSha256: "4d4ff1105e69944c9fd4c37b598bf308d92f1ecea2fb50fe1b73c72b858d1ab5",
  recordsSha256: "480732a812a3271e5059217cffc949dcbaaef97f3d63e8391bb64210d30be1a5",
  scheduleSha256: "d3e7f193990b28097a93c9623946ab1a1b591a7aae3ad711172461144bd424a3",
  shapeSha256: "52a6c64bf1e06963cbfc28a227a9f7f925d081c78a1e61a699e332a4a832e069",
  normalizedScheduleSha256: "145986294e0a7eee2d6d0b43874d9a46d1c6bfe9a1791a59880df0dee24f1b28",
  recordsPerRound: 1_024,
  stateWords: 32,
});

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const elapsedMs = (started) => Number(process.hrtime.bigint() - started) / 1e6;
const spread = (values) => {
  const finite = values.filter((value) => Number.isFinite(value) && value > 0);
  return finite.length ? Math.max(...finite) / Math.min(...finite) : Infinity;
};
const turn = () => new Promise((resolveTurn) => setImmediate(resolveTurn));

function affinity() {
  return readFileSync("/proc/self/status", "utf8")
    .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null;
}

function stateBytes(exports_) {
  const words = Number(exports_.state_word_count());
  const bytes = Buffer.alloc(words * 8);
  for (let index = 0; index < words; index++) {
    bytes.writeBigUInt64LE(BigInt.asUintN(64, exports_.state_word(index)), index * 8);
  }
  return bytes;
}

function fingerprint(exports_, result) {
  const bytes = stateBytes(exports_);
  const resultHex = BigInt.asUintN(64, result).toString(16).padStart(16, "0");
  return {
    result: `0x${resultHex}`,
    stateSha256: sha256(bytes),
    fingerprint: sha256(Buffer.concat([bytes, Buffer.from(resultHex)])),
    x0: `0x${bytes.readBigUInt64LE(0).toString(16)}`,
  };
}

async function worker(variant, modelPath) {
  if (!variants.includes(variant)) throw new Error(`invalid G002 variant ${variant}`);
  const bytes = readFileSync(modelPath);
  if (sha256(bytes) !== EXPECTED.modelSha256 || !WebAssembly.validate(bytes)) {
    throw new Error("G002 worker model identity or validation failed");
  }
  const hostBeforeMs = cpuProbe();
  let started = process.hrtime.bigint();
  const module = new WebAssembly.Module(bytes);
  const compileMs = elapsedMs(started);
  started = process.hrtime.bigint();
  const instance = new WebAssembly.Instance(module);
  const instantiateMs = elapsedMs(started);
  const exports_ = instance.exports;
  if (Number(exports_.record_count()) !== EXPECTED.recordsPerRound ||
      Number(exports_.state_word_count()) !== EXPECTED.stateWords) {
    throw new Error("G002 worker model shape mismatch");
  }

  const run = variant === "control" ? exports_.run_control : exports_.run_treatment;
  exports_.reset_state();
  const prewarmResult = run(PREWARM_ROUNDS, 0, EXPECTED.recordsPerRound);
  const prewarm = {
    rounds: PREWARM_ROUNDS,
    modeledInstructions: PREWARM_ROUNDS * EXPECTED.recordsPerRound,
    ...fingerprint(exports_, prewarmResult),
  };
  await turn();

  const timed = (rounds) => {
    exports_.reset_state();
    const sampleStarted = process.hrtime.bigint();
    const result = run(rounds, 0, EXPECTED.recordsPerRound);
    const ms = elapsedMs(sampleStarted);
    return {
      rounds,
      modeledInstructions: rounds * EXPECTED.recordsPerRound,
      ms,
      ...fingerprint(exports_, result),
    };
  };

  const warm = [];
  for (let call = 0; call < WARM_CALLS; call++) {
    warm.push(timed(WARM_ROUNDS));
    await turn();
  }
  const steady = [];
  for (let call = 0; call < STEADY_CALLS; call++) {
    steady.push(timed(STEADY_ROUNDS));
    await turn();
  }
  const hostAfterMs = cpuProbe();
  const steadyMs = steady.map((sample) => sample.ms);
  const phaseDeterminism = {
    warm: new Set(warm.map((sample) => sample.fingerprint)).size === 1,
    steady: new Set(steady.map((sample) => sample.fingerprint)).size === 1,
  };
  process.stdout.write(JSON.stringify({
    variant,
    runtime: { node: process.version, v8: process.versions.v8 },
    affinity: affinity(),
    loadAverage: loadavg(),
    model: { bytes: bytes.length, sha256: sha256(bytes) },
    compileMs,
    instantiateMs,
    hostBeforeMs,
    hostAfterMs,
    prewarm,
    warm,
    steady,
    steadyMedianMs: summary(steadyMs).median,
    steadySpread: spread(steadyMs),
    phaseDeterminism,
  }));
}

if (process.argv.includes("--worker")) {
  const variant = process.argv.find((argument) => argument.startsWith("--variant="))
    ?.slice("--variant=".length);
  const modelPath = process.argv.find((argument) => argument.startsWith("--model="))
    ?.slice("--model=".length);
  if (!modelPath) throw new Error("G002 worker is missing --model");
  await worker(variant, modelPath);
  process.exit(0);
}

const modelRoot = resolve(process.env.G002_MODEL_ROOT ||
  join(root, "target/bench/interpreter-g002-model-v1"));
const modelPath = join(modelRoot, "model.wasm");
const freezePath = join(modelRoot, "freeze.json");
const recordsPath = join(modelRoot, "records.bin");
const schedulePath = join(modelRoot, "schedule.tsv");
const shapePath = join(modelRoot, "shape.tsv");
const sourcePath = join(root, "crates/rv64-dbt/examples/g002_local_gpr_model.rs");
const preparationHarnessPath = join(here, "prepare-g002-local-gpr-model.mjs");
const output = resolve(process.env.G002_OPPORTUNITY_OUTPUT ||
  join(root, "target/bench/interpreter-g002-opportunity-v1/gate.json"));
mkdirSync(dirname(output), { recursive: true });

const modelBytes = readFileSync(modelPath);
const freezeBytes = readFileSync(freezePath);
const recordsBytes = readFileSync(recordsPath);
const scheduleBytes = readFileSync(schedulePath);
const shapeBytes = readFileSync(shapePath);
const sourceBytes = readFileSync(sourcePath);
const preparationHarnessBytes = readFileSync(preparationHarnessPath);
const freeze = JSON.parse(freezeBytes);
const identityChecks = {
  source: sha256(sourceBytes) === EXPECTED.sourceSha256,
  preparationHarness: sha256(preparationHarnessBytes) === EXPECTED.preparationHarnessSha256,
  model: sha256(modelBytes) === EXPECTED.modelSha256,
  freeze: sha256(freezeBytes) === EXPECTED.freezeSha256,
  records: sha256(recordsBytes) === EXPECTED.recordsSha256,
  schedule: sha256(scheduleBytes) === EXPECTED.scheduleSha256,
  shape: sha256(shapeBytes) === EXPECTED.shapeSha256,
  validModel: WebAssembly.validate(modelBytes),
  untimedFreeze: freeze.timingCollected === false && freeze.productionModified === false,
  preparationPassed: freeze.pass === true && Object.values(freeze.checks).every(Boolean),
  exactFrozenSchedule: freeze.schedule.records === EXPECTED.recordsPerRound &&
    freeze.schedule.normalizedSha256 === EXPECTED.normalizedScheduleSha256,
};
if (!Object.values(identityChecks).every(Boolean)) {
  throw new Error(`G002 frozen identity failure: ${JSON.stringify(identityChecks)}`);
}

const releaseLock = await acquireBenchmarkLock(join(root, "target/bench"));
const pairs = [];
const workerFailures = [];
try {
  for (let pair = 0; pair < PAIRS; pair++) {
    const cpu = CPUS[pair % CPUS.length];
    const order = pair & 1 ? ["treatment", "control"] : ["control", "treatment"];
    const pairResult = { pair: pair + 1, cpu, order };
    for (const variant of order) {
      const child = spawnSync("taskset", [
        "-c", String(cpu), process.execPath, self, "--worker",
        `--variant=${variant}`, `--model=${modelPath}`,
      ], {
        cwd: root,
        encoding: "utf8",
        maxBuffer: 16 << 20,
        timeout: 180_000,
      });
      if (child.status !== 0) {
        workerFailures.push({
          pair: pair + 1,
          cpu,
          variant,
          status: child.status,
          signal: child.signal,
          stdout: child.stdout?.slice(-4_000),
          stderr: child.stderr?.slice(-4_000),
        });
        continue;
      }
      pairResult[variant] = JSON.parse(child.stdout);
      console.log(
        `G002 pair ${pair + 1}/${PAIRS} ${variant} cpu${cpu}: ` +
        `${pairResult[variant].steadyMedianMs.toFixed(3)} ms`,
      );
    }
    pairs.push(pairResult);
  }
} finally {
  await releaseLock();
}

const completePairs = pairs.filter((pair) => pair.control && pair.treatment);
const allRuns = completePairs.flatMap((pair) => [pair.control, pair.treatment]);
const controlSteady = completePairs.map((pair) => pair.control.steadyMedianMs);
const treatmentSteady = completePairs.map((pair) => pair.treatment.steadyMedianMs);
const pairedSpeedups = completePairs.map((pair) =>
  pair.control.steadyMedianMs / pair.treatment.steadyMedianMs);
const hostProbes = allRuns.flatMap((run) => [run.hostBeforeMs, run.hostAfterMs]);
const steady = summary(pairedSpeedups);
const steadyLower = steady.medianConfidence95?.[0] ?? -Infinity;
const crossVariantState = ["prewarm", "warm", "steady"].every((phase) => {
  const fingerprints = new Set();
  for (const pair of completePairs) {
    for (const variant of variants) {
      const run = pair[variant];
      const sample = phase === "prewarm" ? run.prewarm : run[phase][0];
      fingerprints.add(sample.fingerprint);
    }
  }
  return fingerprints.size === 1;
});
const treatmentCompileInstantiate = completePairs.map((pair) =>
  pair.treatment.compileMs + pair.treatment.instantiateMs);

const measurementChecks = {
  completePairs: completePairs.length === PAIRS && workerFailures.length === 0,
  exactAffinity: completePairs.every((pair) =>
    pair.control.affinity === String(pair.cpu) && pair.treatment.affinity === String(pair.cpu)),
  exactModel: allRuns.every((run) => run.model.sha256 === EXPECTED.modelSha256),
  exactSchedule: allRuns.every((run) =>
    run.prewarm.rounds === PREWARM_ROUNDS &&
    run.prewarm.modeledInstructions === PREWARM_ROUNDS * EXPECTED.recordsPerRound &&
    run.warm.length === WARM_CALLS &&
    run.warm.every((sample) => sample.rounds === WARM_ROUNDS &&
      sample.modeledInstructions === WARM_ROUNDS * EXPECTED.recordsPerRound) &&
    run.steady.length === STEADY_CALLS &&
    run.steady.every((sample) => sample.rounds === STEADY_ROUNDS &&
      sample.modeledInstructions === STEADY_ROUNDS * EXPECTED.recordsPerRound)),
  deterministicWithinProcess: allRuns.every((run) =>
    run.phaseDeterminism.warm && run.phaseDeterminism.steady),
  exactCompleteState: crossVariantState,
  architecturalX0: allRuns.every((run) =>
    run.prewarm.x0 === "0x0" && run.warm.every((sample) => sample.x0 === "0x0") &&
    run.steady.every((sample) => sample.x0 === "0x0")),
  hostSpread: spread(hostProbes) <= MAX_HOST_SPREAD,
  controlSideSpread: spread(controlSteady) <= MAX_SAMPLE_SPREAD,
  treatmentSideSpread: spread(treatmentSteady) <= MAX_SAMPLE_SPREAD,
  withinProcessSpread: allRuns.every((run) => run.steadySpread <= MAX_SAMPLE_SPREAD),
};
const performanceChecks = {
  pairedMedian: steady.median >= MIN_SPEEDUP,
  pairedLowerBound: steadyLower > MIN_LOWER_BOUND,
  treatmentCompileInstantiate: treatmentCompileInstantiate.every((value) =>
    value < MAX_TREATMENT_COMPILE_INSTANTIATE_MS),
};
const checks = { ...identityChecks, ...measurementChecks, ...performanceChecks };
const pass = Object.values(checks).every(Boolean);
const selfBytes = readFileSync(self);
const report = {
  schema: 1,
  experiment: "G002 complete local-GPR standalone opportunity gate",
  productionModified: false,
  methodology: "architecture-balanced/favorable-state-only/fresh-process/CPU-pinned/alternating-pairs/ordinary-V8",
  frozen: {
    pairs: PAIRS,
    processes: PAIRS * 2,
    cpus: CPUS,
    prewarmRounds: PREWARM_ROUNDS,
    warmRounds: WARM_ROUNDS,
    warmCalls: WARM_CALLS,
    steadyRounds: STEADY_ROUNDS,
    steadyCalls: STEADY_CALLS,
    recordsPerRound: EXPECTED.recordsPerRound,
    modeledInstructionsPerSteadyCall: STEADY_ROUNDS * EXPECTED.recordsPerRound,
    maximumHostSpread: MAX_HOST_SPREAD,
    maximumSampleSpread: MAX_SAMPLE_SPREAD,
    minimumPairedSpeedup: MIN_SPEEDUP,
    minimumPairedLowerBound: MIN_LOWER_BOUND,
    maximumTreatmentCompileInstantiateMs: MAX_TREATMENT_COMPILE_INSTANTIATE_MS,
    bootstrap: {
      samples: 4_096,
      seedExpression: "0x9e3779b9 xor finite-sample-count",
      statistic: "median",
      interval: "percentile-95",
    },
    cpuProbe: CPU_PROBE_SPEC,
  },
  identities: {
    timingHarness: { path: self, bytes: selfBytes.length, sha256: sha256(selfBytes) },
    source: { path: sourcePath, bytes: sourceBytes.length, sha256: sha256(sourceBytes) },
    preparationHarness: {
      path: preparationHarnessPath,
      bytes: preparationHarnessBytes.length,
      sha256: sha256(preparationHarnessBytes),
    },
    model: { path: modelPath, bytes: modelBytes.length, sha256: sha256(modelBytes) },
    freeze: { path: freezePath, bytes: freezeBytes.length, sha256: sha256(freezeBytes) },
    records: { path: recordsPath, bytes: recordsBytes.length, sha256: sha256(recordsBytes) },
    schedule: { path: schedulePath, bytes: scheduleBytes.length, sha256: sha256(scheduleBytes) },
    shape: { path: shapePath, bytes: shapeBytes.length, sha256: sha256(shapeBytes) },
    normalizedScheduleSha256: EXPECTED.normalizedScheduleSha256,
  },
  runtime: allRuns[0]?.runtime ?? null,
  pairs,
  workerFailures,
  observed: {
    controlSteadyMs: summary(controlSteady),
    treatmentSteadyMs: summary(treatmentSteady),
    pairedSpeedup: steady,
    hostProbeMs: summary(hostProbes),
    hostProbeSpread: spread(hostProbes),
    controlSideSpread: spread(controlSteady),
    treatmentSideSpread: spread(treatmentSteady),
    treatmentCompileInstantiateMs: summary(treatmentCompileInstantiate),
    compileMs: {
      control: summary(completePairs.map((pair) => pair.control.compileMs)),
      treatment: summary(completePairs.map((pair) => pair.treatment.compileMs)),
    },
    instantiateMs: {
      control: summary(completePairs.map((pair) => pair.control.instantiateMs)),
      treatment: summary(completePairs.map((pair) => pair.treatment.instantiateMs)),
    },
  },
  checks,
  pass,
  decision: pass
    ? "admit-exactly-frozen-g002-to-production-feasibility"
    : "close-g002-before-production-edit-without-successor",
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(
  `G002 steady ${steady.median.toFixed(4)}x ` +
  `[${(steady.medianConfidence95 ?? [0, 0])
    .map((value) => value.toFixed(4)).join(", ")}]`,
);
console.log(
  `G002 host ${spread(hostProbes).toFixed(4)}x; ` +
  `treatment compile+instantiate max=${Math.max(0, ...treatmentCompileInstantiate).toFixed(3)} ms; ` +
  `pass=${pass}`,
);
console.log(`G002 opportunity report: ${output}`);
if (!pass) process.exitCode = 1;
