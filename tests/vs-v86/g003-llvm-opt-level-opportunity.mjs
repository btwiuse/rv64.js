#!/usr/bin/env node

// Frozen G003 finite standard LLVM optimization-level opportunity screen.
// Every process executes only the balanced direct-interpreter run_control path.

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
const candidates = Object.freeze(["o1", "o2", "os", "oz"]);
const variants = Object.freeze(["control", "candidate"]);
const PAIRS = 7;
const CPUS = Object.freeze([8, 9, 10, 11, 12, 13, 14, 15]);
const PREWARM_ROUNDS = 1;
const WARM_ROUNDS = 4_096;
const WARM_CALLS = 3;
const STEADY_ROUNDS = 65_536;
const STEADY_CALLS = 7;
const MAX_HOST_SPREAD = 1.25;
const MAX_SAMPLE_SPREAD = 1.25;
const MIN_SPEEDUP = 3.25;
const MIN_LOWER_BOUND = 3.00;
const MAX_CANDIDATE_COMPILE_INSTANTIATE_MS = 25;
const EXPECTED = Object.freeze({
  sourceSha256: "27bfc111495af24e39a4f2c3e7233ac690a20e2456099dd2cab3a1e2453a0128",
  preparationHarnessSha256: "8181cb607411c0a5b52980616ec426b4663ffab1f481456d5ccd4840ed180c8e",
  freezeSha256: "0ecceda6a5f519577da7ea4f09744cf5f34756663c216b299dda4491db8ed04c",
  rawStreamSha256: "987912d44c5d5b1f25ca26f57ed298ba9d21c1f8e8bef6ae7e535b87a2315c0f",
  normalizedStreamSha256: "52ed0a9f402bc8e66a038d852f1afd65336b031d9233d15844c38cf320f5284a",
  artifacts: Object.freeze({
    o1: "63f2fb590d20260c01d55186c53d8b38f9722f6798cdba6a40846de87f400026",
    o2: "63f2fb590d20260c01d55186c53d8b38f9722f6798cdba6a40846de87f400026",
    o3: "63f2fb590d20260c01d55186c53d8b38f9722f6798cdba6a40846de87f400026",
    os: "128d7db59f57ac2053689408a5e5fda7ae2b89b9d53ec1a2ebdb83469ad2f55e",
    oz: "128d7db59f57ac2053689408a5e5fda7ae2b89b9d53ec1a2ebdb83469ad2f55e",
  }),
  instructionsPerRound: 252,
  blocksPerRound: 44,
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
  const count = Number(exports_.state_word_count());
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

async function worker(level, modelPath) {
  if (![...candidates, "o3"].includes(level)) throw new Error(`invalid G003 level ${level}`);
  const bytes = readFileSync(modelPath);
  if (sha256(bytes) !== EXPECTED.artifacts[level] || !WebAssembly.validate(bytes)) {
    throw new Error("G003 worker artifact identity or validation failed");
  }
  const hostBeforeMs = cpuProbe();
  let started = process.hrtime.bigint();
  const module = new WebAssembly.Module(bytes);
  const compileMs = elapsedMs(started);
  started = process.hrtime.bigint();
  const instance = new WebAssembly.Instance(module);
  const instantiateMs = elapsedMs(started);
  const exports_ = instance.exports;
  if (exports_.init_model() !== 0 || exports_.model_error() !== 0 ||
      Number(exports_.flat_count()) !== EXPECTED.instructionsPerRound ||
      Number(exports_.block_count()) !== EXPECTED.blocksPerRound) {
    throw new Error("G003 worker model initialization or shape failed");
  }

  exports_.reset_cache();
  exports_.reset_state();
  const prewarmResult = exports_.run_control(PREWARM_ROUNDS);
  const prewarm = {
    rounds: PREWARM_ROUNDS,
    scalarInstructions: PREWARM_ROUNDS * EXPECTED.instructionsPerRound,
    ...fingerprint(exports_, prewarmResult),
  };
  await turn();

  const timed = (rounds) => {
    exports_.reset_state();
    const sampleStarted = process.hrtime.bigint();
    const result = exports_.run_control(rounds);
    const ms = elapsedMs(sampleStarted);
    if (exports_.model_error() !== 0) throw new Error("G003 model error during sample");
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
  process.stdout.write(JSON.stringify({
    level,
    runtime: { node: process.version, v8: process.versions.v8 },
    affinity: affinity(),
    loadAverage: loadavg(),
    artifact: { bytes: bytes.length, sha256: sha256(bytes) },
    compileMs,
    instantiateMs,
    hostBeforeMs,
    hostAfterMs,
    prewarm,
    warm,
    steady,
    steadyMedianMs: summary(steadyMs).median,
    steadySpread: spread(steadyMs),
    phaseDeterminism: {
      warm: new Set(warm.map((sample) => sample.fingerprint)).size === 1,
      steady: new Set(steady.map((sample) => sample.fingerprint)).size === 1,
    },
  }));
}

if (process.argv.includes("--worker")) {
  const level = process.argv.find((argument) => argument.startsWith("--level="))
    ?.slice("--level=".length);
  const modelPath = process.argv.find((argument) => argument.startsWith("--model="))
    ?.slice("--model=".length);
  if (!level || !modelPath) throw new Error("G003 worker is missing level or model");
  await worker(level, modelPath);
  process.exit(0);
}

const modelRoot = resolve(process.env.G003_MODEL_ROOT ||
  join(root, "target/bench/interpreter-g003-model-v2"));
const freezePath = join(modelRoot, "freeze.json");
const sourcePath = join(here, "interpreter-g001/g001-fencei-decode-model.c");
const preparationHarnessPath = join(here, "prepare-g003-llvm-opt-level-model.mjs");
const output = resolve(process.env.G003_OPPORTUNITY_OUTPUT ||
  join(root, "target/bench/interpreter-g003-opportunity-v1/gate.json"));
mkdirSync(dirname(output), { recursive: true });

const sourceBytes = readFileSync(sourcePath);
const preparationHarnessBytes = readFileSync(preparationHarnessPath);
const freezeBytes = readFileSync(freezePath);
const freeze = JSON.parse(freezeBytes);
const artifactBytes = Object.fromEntries(
  [...candidates, "o3"].map((level) => [
    level,
    readFileSync(join(modelRoot, level, "model.wasm")),
  ]),
);
const identityChecks = {
  source: sha256(sourceBytes) === EXPECTED.sourceSha256,
  preparationHarness: sha256(preparationHarnessBytes) === EXPECTED.preparationHarnessSha256,
  freeze: sha256(freezeBytes) === EXPECTED.freezeSha256,
  artifacts: Object.entries(artifactBytes).every(([level, bytes]) =>
    sha256(bytes) === EXPECTED.artifacts[level] && WebAssembly.validate(bytes)),
  untimedFreeze: freeze.timingCollected === false && freeze.productionModified === false,
  preparationPassed: freeze.pass === true && Object.values(freeze.checks).every(Boolean),
  exactStream: freeze.stream.instructions === EXPECTED.instructionsPerRound &&
    freeze.stream.rawSha256 === EXPECTED.rawStreamSha256 &&
    freeze.stream.normalizedSha256 === EXPECTED.normalizedStreamSha256,
  exactFiniteSet: freeze.compiler.levels.map(({ name }) => name).join(",") ===
    "o1,o2,o3,os,oz",
};
if (!Object.values(identityChecks).every(Boolean)) {
  throw new Error(`G003 frozen identity failure: ${JSON.stringify(identityChecks)}`);
}

const releaseLock = await acquireBenchmarkLock(join(root, "target/bench"));
const screens = Object.fromEntries(candidates.map((level) => [level, []]));
const issueOrder = [];
const workerFailures = [];
let screenIndex = 0;
try {
  for (let pair = 0; pair < PAIRS; pair++) {
    const rotated = candidates.map((_, index) => candidates[(index + pair) % candidates.length]);
    for (const level of rotated) {
      const cpu = CPUS[screenIndex % CPUS.length];
      screenIndex++;
      const order = pair & 1 ? ["candidate", "control"] : ["control", "candidate"];
      const pairResult = { pair: pair + 1, level, cpu, order };
      for (const variant of order) {
        const artifactLevel = variant === "control" ? "o3" : level;
        const modelPath = join(modelRoot, artifactLevel, "model.wasm");
        issueOrder.push({ pair: pair + 1, level, cpu, variant, artifactLevel });
        const child = spawnSync("taskset", [
          "-c", String(cpu), process.execPath, self, "--worker",
          `--level=${artifactLevel}`, `--model=${modelPath}`,
        ], {
          cwd: root,
          encoding: "utf8",
          maxBuffer: 16 << 20,
          timeout: 180_000,
        });
        if (child.status !== 0) {
          workerFailures.push({
            pair: pair + 1,
            level,
            cpu,
            variant,
            artifactLevel,
            status: child.status,
            signal: child.signal,
            stdout: child.stdout?.slice(-4_000),
            stderr: child.stderr?.slice(-4_000),
          });
          continue;
        }
        pairResult[variant] = JSON.parse(child.stdout);
        console.log(
          `G003 ${level} pair ${pair + 1}/${PAIRS} ${variant} cpu${cpu}: ` +
          `${pairResult[variant].steadyMedianMs.toFixed(3)} ms`,
        );
      }
      screens[level].push(pairResult);
    }
  }
} finally {
  await releaseLock();
}

const allCompleteRuns = [];
const results = {};
for (const level of candidates) {
  const completePairs = screens[level].filter((pair) => pair.control && pair.candidate);
  const allRuns = completePairs.flatMap((pair) => [pair.control, pair.candidate]);
  allCompleteRuns.push(...allRuns);
  const controlSteady = completePairs.map((pair) => pair.control.steadyMedianMs);
  const candidateSteady = completePairs.map((pair) => pair.candidate.steadyMedianMs);
  const pairedSpeedups = completePairs.map((pair) =>
    pair.control.steadyMedianMs / pair.candidate.steadyMedianMs);
  const paired = summary(pairedSpeedups);
  const lower = paired.medianConfidence95?.[0] ?? -Infinity;
  const candidateCompileInstantiate = completePairs.map((pair) =>
    pair.candidate.compileMs + pair.candidate.instantiateMs);
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
  const measurementChecks = {
    completePairs: completePairs.length === PAIRS,
    exactAffinity: completePairs.every((pair) =>
      pair.control.affinity === String(pair.cpu) && pair.candidate.affinity === String(pair.cpu)),
    exactArtifacts: completePairs.every((pair) =>
      pair.control.artifact.sha256 === EXPECTED.artifacts.o3 &&
      pair.candidate.artifact.sha256 === EXPECTED.artifacts[level]),
    exactSchedule: allRuns.every((run) =>
      run.prewarm.rounds === PREWARM_ROUNDS &&
      run.prewarm.scalarInstructions === PREWARM_ROUNDS * EXPECTED.instructionsPerRound &&
      run.warm.length === WARM_CALLS &&
      run.warm.every((sample) => sample.rounds === WARM_ROUNDS &&
        sample.scalarInstructions === WARM_ROUNDS * EXPECTED.instructionsPerRound) &&
      run.steady.length === STEADY_CALLS &&
      run.steady.every((sample) => sample.rounds === STEADY_ROUNDS &&
        sample.scalarInstructions === STEADY_ROUNDS * EXPECTED.instructionsPerRound)),
    deterministicWithinProcess: allRuns.every((run) =>
      run.phaseDeterminism.warm && run.phaseDeterminism.steady),
    exactCompleteState: crossVariantState,
    controlSideSpread: spread(controlSteady) <= MAX_SAMPLE_SPREAD,
    candidateSideSpread: spread(candidateSteady) <= MAX_SAMPLE_SPREAD,
    withinProcessSpread: allRuns.every((run) => run.steadySpread <= MAX_SAMPLE_SPREAD),
  };
  const performanceChecks = {
    pairedMedian: paired.median >= MIN_SPEEDUP,
    pairedLowerBound: lower > MIN_LOWER_BOUND,
    candidateCompileInstantiate: candidateCompileInstantiate.every((value) =>
      value < MAX_CANDIDATE_COMPILE_INSTANTIATE_MS),
  };
  results[level] = {
    pairs: screens[level],
    observed: {
      controlSteadyMs: summary(controlSteady),
      candidateSteadyMs: summary(candidateSteady),
      pairedSpeedup: paired,
      controlSideSpread: spread(controlSteady),
      candidateSideSpread: spread(candidateSteady),
      candidateCompileInstantiateMs: summary(candidateCompileInstantiate),
    },
    measurementChecks,
    performanceChecks,
    passesMeasurement: Object.values(measurementChecks).every(Boolean),
    passesPerformance: Object.values(performanceChecks).every(Boolean),
  };
  results[level].pass = results[level].passesMeasurement && results[level].passesPerformance;
}

const hostProbes = allCompleteRuns.flatMap((run) => [run.hostBeforeMs, run.hostAfterMs]);
const globalChecks = {
  ...identityChecks,
  completeIssueOrder: issueOrder.length === candidates.length * PAIRS * 2,
  noWorkerFailures: workerFailures.length === 0,
  hostSpread: spread(hostProbes) <= MAX_HOST_SPREAD,
};
const passing = candidates.filter((level) => results[level].pass);
const tieOrder = Object.freeze(["o2", "o1", "os", "oz"]);
const selected = passing.toSorted((left, right) => {
  const leftLower = results[left].observed.pairedSpeedup.medianConfidence95?.[0] ?? -Infinity;
  const rightLower = results[right].observed.pairedSpeedup.medianConfidence95?.[0] ?? -Infinity;
  if (Math.abs(leftLower - rightLower) > 0.02) return rightLower - leftLower;
  return tieOrder.indexOf(left) - tieOrder.indexOf(right);
})[0] ?? null;
const pass = Object.values(globalChecks).every(Boolean) && selected !== null;
const selfBytes = readFileSync(self);
const report = {
  schema: 1,
  experiment: "G003 finite standard LLVM optimization-level opportunity screen",
  productionModified: false,
  methodology: "architecture-balanced/direct-interpreter/fresh-process/CPU-pinned/alternating-pairs/ordinary-V8",
  frozen: {
    candidates,
    control: "o3",
    pairsPerCandidate: PAIRS,
    processes: candidates.length * PAIRS * 2,
    cpus: CPUS,
    candidateOrder: "O1,O2,Os,Oz rotated left by pair index",
    variantOrder: "even control/candidate; odd candidate/control",
    prewarmRounds: PREWARM_ROUNDS,
    warmRounds: WARM_ROUNDS,
    warmCalls: WARM_CALLS,
    steadyRounds: STEADY_ROUNDS,
    steadyCalls: STEADY_CALLS,
    instructionsPerRound: EXPECTED.instructionsPerRound,
    instructionsPerSteadyCall: STEADY_ROUNDS * EXPECTED.instructionsPerRound,
    maximumHostSpread: MAX_HOST_SPREAD,
    maximumSampleSpread: MAX_SAMPLE_SPREAD,
    minimumPairedSpeedup: MIN_SPEEDUP,
    minimumPairedLowerBound: MIN_LOWER_BOUND,
    maximumCandidateCompileInstantiateMs: MAX_CANDIDATE_COMPILE_INSTANTIATE_MS,
    tieOrder,
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
    freeze: { path: freezePath, bytes: freezeBytes.length, sha256: sha256(freezeBytes) },
    artifacts: Object.fromEntries(Object.entries(artifactBytes).map(([level, bytes]) => [
      level,
      { path: join(modelRoot, level, "model.wasm"), bytes: bytes.length, sha256: sha256(bytes) },
    ])),
    rawStreamSha256: EXPECTED.rawStreamSha256,
    normalizedStreamSha256: EXPECTED.normalizedStreamSha256,
  },
  runtime: allCompleteRuns[0]?.runtime ?? null,
  issueOrder,
  workerFailures,
  hostProbeMs: summary(hostProbes),
  hostProbeSpread: spread(hostProbes),
  globalChecks,
  results,
  passing,
  selected,
  pass,
  decision: pass
    ? `admit-${selected}-to-single-production-build`
    : "close-standard-llvm-opt-levels-before-production-without-successor",
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
for (const level of candidates) {
  const paired = results[level].observed.pairedSpeedup;
  console.log(
    `G003 ${level}: ${paired.median.toFixed(4)}x ` +
    `[${(paired.medianConfidence95 ?? [0, 0]).map((value) => value.toFixed(4)).join(", ")}] ` +
    `pass=${results[level].pass}`,
  );
}
console.log(
  `G003 host ${spread(hostProbes).toFixed(4)}x; selected=${selected}; pass=${pass}`,
);
console.log(`G003 opportunity report: ${output}`);
if (!pass) process.exitCode = 1;
