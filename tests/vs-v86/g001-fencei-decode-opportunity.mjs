#!/usr/bin/env node

// Frozen G001 standalone opportunity gate. The model, stream, schedule,
// ordering, hosts, statistics, and thresholds are fixed before this script's
// first execution. A failure closes G001 without a production interpreter edit.

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
const STEADY_ROUNDS = 65_536;
const STEADY_CALLS = 7;
const MAX_HOST_SPREAD = 1.25;
const MAX_SAMPLE_SPREAD = 1.25;
const MIN_SPEEDUP = 3.75;
const MIN_LOWER_BOUND = 3.50;
const MAX_TREATMENT_COMPILE_INSTANTIATE_MS = 25;
const EXPECTED = Object.freeze({
  sourceSha256: "27bfc111495af24e39a4f2c3e7233ac690a20e2456099dd2cab3a1e2453a0128",
  preparationHarnessSha256: "a127049217c86340db91f414f0a12afa37e79f457e4b053bcad2ddd9e39c12ee",
  modelSha256: "63f2fb590d20260c01d55186c53d8b38f9722f6798cdba6a40846de87f400026",
  freezeSha256: "5d80474a040c70ed907eefe9798d65df396fee70c7eb6ea64af8a005310496da",
  rawStreamSha256: "987912d44c5d5b1f25ca26f57ed298ba9d21c1f8e8bef6ae7e535b87a2315c0f",
  normalizedStreamSha256: "52ed0a9f402bc8e66a038d852f1afd65336b031d9233d15844c38cf320f5284a",
  instructionsPerRound: 252,
  blocksPerRound: 44,
  cacheSlots: 64,
  blockLimit: 32,
  normalizedRecordBytes: 32,
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
  const count = exports_.state_word_count();
  const bytes = Buffer.alloc(count * 8);
  for (let index = 0; index < count; index++) {
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
  };
}

async function worker(variant, modelPath) {
  if (!variants.includes(variant)) throw new Error(`invalid G001 variant ${variant}`);
  const bytes = readFileSync(modelPath);
  if (sha256(bytes) !== EXPECTED.modelSha256 || !WebAssembly.validate(bytes)) {
    throw new Error("G001 worker model identity or validation failed");
  }
  const hostBeforeMs = cpuProbe();
  let started = process.hrtime.bigint();
  const module = new WebAssembly.Module(bytes);
  const compileMs = elapsedMs(started);
  started = process.hrtime.bigint();
  const instance = new WebAssembly.Instance(module);
  const instantiateMs = elapsedMs(started);
  const exports_ = instance.exports;
  if (exports_.init_model() !== 0 || exports_.model_error() !== 0) {
    throw new Error("G001 worker initialization failed");
  }
  if (exports_.flat_count() !== EXPECTED.instructionsPerRound ||
      exports_.block_count() !== EXPECTED.blocksPerRound ||
      exports_.cache_slots() !== EXPECTED.cacheSlots ||
      exports_.block_limit() !== EXPECTED.blockLimit ||
      exports_.normalized_record_bytes() !== EXPECTED.normalizedRecordBytes) {
    throw new Error("G001 worker geometry mismatch");
  }

  const run = variant === "control" ? exports_.run_control : exports_.run_treatment;
  exports_.reset_cache();
  exports_.reset_state();
  const prewarmResult = run(PREWARM_ROUNDS);
  const prewarm = {
    rounds: PREWARM_ROUNDS,
    ...fingerprint(exports_, prewarmResult),
  };
  await turn();

  const timed = (rounds) => {
    exports_.reset_state();
    const sampleStarted = process.hrtime.bigint();
    const result = run(rounds);
    const ms = elapsedMs(sampleStarted);
    if (exports_.model_error() !== 0) throw new Error("G001 model error during sample");
    return {
      rounds,
      scalarInstructions: rounds * EXPECTED.instructionsPerRound,
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
  const expectedTreatmentHits = BigInt(EXPECTED.blocksPerRound) *
    BigInt(WARM_ROUNDS * WARM_CALLS + STEADY_ROUNDS * STEADY_CALLS);
  const cache = {
    hits: exports_.cache_hits().toString(),
    misses: exports_.cache_misses().toString(),
    expectedHits: variant === "treatment" ? expectedTreatmentHits.toString() : "0",
    expectedMisses: variant === "treatment" ? String(EXPECTED.blocksPerRound) : "0",
  };
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
    cache,
    phaseDeterminism,
  }));
}

if (process.argv.includes("--worker")) {
  const variant = process.argv.find((argument) => argument.startsWith("--variant="))
    ?.slice("--variant=".length);
  const modelPath = process.argv.find((argument) => argument.startsWith("--model="))
    ?.slice("--model=".length);
  if (!modelPath) throw new Error("G001 worker is missing --model");
  await worker(variant, modelPath);
  process.exit(0);
}

const modelRoot = resolve(process.env.G001_MODEL_ROOT ||
  join(root, "target/bench/interpreter-g001-model-v2"));
const modelPath = join(modelRoot, "model.wasm");
const freezePath = join(modelRoot, "freeze.json");
const sourcePath = join(here, "interpreter-g001/g001-fencei-decode-model.c");
const preparationHarnessPath = join(here, "prepare-g001-fencei-decode-model.mjs");
const output = resolve(process.env.G001_OPPORTUNITY_OUTPUT ||
  join(root, "target/bench/interpreter-g001-opportunity-v1/gate.json"));
mkdirSync(dirname(output), { recursive: true });

const modelBytes = readFileSync(modelPath);
const freezeBytes = readFileSync(freezePath);
const sourceBytes = readFileSync(sourcePath);
const preparationHarnessBytes = readFileSync(preparationHarnessPath);
const freeze = JSON.parse(freezeBytes);
const identityChecks = {
  source: sha256(sourceBytes) === EXPECTED.sourceSha256,
  preparationHarness: sha256(preparationHarnessBytes) === EXPECTED.preparationHarnessSha256,
  model: sha256(modelBytes) === EXPECTED.modelSha256,
  freeze: sha256(freezeBytes) === EXPECTED.freezeSha256,
  validModel: WebAssembly.validate(modelBytes),
  untimedFreeze: freeze.timingCollected === false && freeze.productionModified === false,
  preparationPassed: freeze.pass === true && Object.values(freeze.checks).every(Boolean),
  stream: freeze.stream.rawSha256 === EXPECTED.rawStreamSha256 &&
    freeze.stream.normalizedSha256 === EXPECTED.normalizedStreamSha256 &&
    freeze.stream.instructions === EXPECTED.instructionsPerRound &&
    freeze.stream.blocks === EXPECTED.blocksPerRound,
};
if (!Object.values(identityChecks).every(Boolean)) {
  throw new Error(`G001 frozen identity failure: ${JSON.stringify(identityChecks)}`);
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
          stdout: child.stdout?.slice(-4000),
          stderr: child.stderr?.slice(-4000),
        });
        continue;
      }
      pairResult[variant] = JSON.parse(child.stdout);
      console.log(
        `G001 pair ${pair + 1}/${PAIRS} ${variant} cpu${cpu}: ` +
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
const expectedTreatmentHits = BigInt(EXPECTED.blocksPerRound) *
  BigInt(WARM_ROUNDS * WARM_CALLS + STEADY_ROUNDS * STEADY_CALLS);

const measurementChecks = {
  completePairs: completePairs.length === PAIRS && workerFailures.length === 0,
  exactAffinity: completePairs.every((pair) =>
    pair.control.affinity === String(pair.cpu) && pair.treatment.affinity === String(pair.cpu)),
  exactModel: allRuns.every((run) => run.model.sha256 === EXPECTED.modelSha256),
  exactSchedule: allRuns.every((run) =>
    run.prewarm.rounds === PREWARM_ROUNDS &&
    run.warm.length === WARM_CALLS &&
    run.warm.every((sample) => sample.rounds === WARM_ROUNDS &&
      sample.scalarInstructions === WARM_ROUNDS * EXPECTED.instructionsPerRound) &&
    run.steady.length === STEADY_CALLS &&
    run.steady.every((sample) => sample.rounds === STEADY_ROUNDS &&
      sample.scalarInstructions === STEADY_ROUNDS * EXPECTED.instructionsPerRound)),
  deterministicWithinProcess: allRuns.every((run) =>
    run.phaseDeterminism.warm && run.phaseDeterminism.steady),
  exactCompleteState: crossVariantState,
  treatmentCacheProof: completePairs.every((pair) =>
    BigInt(pair.treatment.cache.misses) === BigInt(EXPECTED.blocksPerRound) &&
    BigInt(pair.treatment.cache.hits) === expectedTreatmentHits),
  controlCacheProof: completePairs.every((pair) =>
    pair.control.cache.misses === "0" && pair.control.cache.hits === "0"),
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
  experiment: "G001 FENCE.I-coherent decoded-interpreter standalone opportunity gate",
  productionModified: false,
  methodology: "architecture-balanced/fresh-process/CPU-pinned/alternating-pairs/ordinary-V8",
  frozen: {
    pairs: PAIRS,
    processes: PAIRS * 2,
    cpus: CPUS,
    prewarmRounds: PREWARM_ROUNDS,
    warmRounds: WARM_ROUNDS,
    warmCalls: WARM_CALLS,
    steadyRounds: STEADY_ROUNDS,
    steadyCalls: STEADY_CALLS,
    instructionsPerRound: EXPECTED.instructionsPerRound,
    maximumHostSpread: MAX_HOST_SPREAD,
    maximumSampleSpread: MAX_SAMPLE_SPREAD,
    minimumPairedSpeedup: MIN_SPEEDUP,
    minimumPairedLowerBound: MIN_LOWER_BOUND,
    maximumTreatmentCompileInstantiateMs: MAX_TREATMENT_COMPILE_INSTANTIATE_MS,
    bootstrap: {
      samples: 4096,
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
    rawStreamSha256: EXPECTED.rawStreamSha256,
    normalizedStreamSha256: EXPECTED.normalizedStreamSha256,
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
    ? "admit-exactly-frozen-g001-to-production-correctness"
    : "close-g001-before-production-edit-without-successor",
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(
  `G001 steady ${steady.median.toFixed(4)}x ` +
  `[${(steady.medianConfidence95 ?? [0, 0])
    .map((value) => value.toFixed(4)).join(", ")}]`,
);
console.log(
  `G001 host ${spread(hostProbes).toFixed(4)}x; ` +
  `treatment compile+instantiate max=${Math.max(0, ...treatmentCompileInstantiate).toFixed(3)} ms; ` +
  `pass=${pass}`,
);
console.log(`G001 opportunity report: ${output}`);
if (!pass) process.exitCode = 1;
