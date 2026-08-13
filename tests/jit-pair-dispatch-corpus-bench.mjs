#!/usr/bin/env node

// Frozen R053 engine-shape gate for an architecture-complete two-instruction
// handler tier. The production emulator is not modified. Each measurement is
// made in a fresh Node/V8 process, with alternating paired order, after exact
// single-versus-pair state equivalence has been checked.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { median, summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const self = fileURLToPath(import.meta.url);
const corpusDir = join(root, "target/jit-pair-dispatch-corpus");
const repeatDir = join(root, "target/jit-pair-dispatch-corpus-repeat");
const variants = ["single", "pair"];
const CORRECTNESS_STEPS = [0, 2, 128, 8192, 1_048_576];
const FIRST_STEPS = 8192;
const WARM_STEPS = 1_048_576;
const WARM_REPETITIONS = 8;
const STEADY_STEPS = 1_048_576;
const STEADY_REPETITIONS = 32;
const STEADY_SAMPLES = 7;
const STEADY_OPERATIONS = STEADY_STEPS * STEADY_REPETITIONS;
const REQUIRED_STEADY_SPEEDUP = 1.25;
const MAX_PAIR_COLD_CONSTRUCTION_MS = 100;
const MAX_PAIR_COLD_DELTA_MS = 100;

const now = () => process.hrtime.bigint();
const elapsedMs = (started) => Number(process.hrtime.bigint() - started) / 1e6;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

function worker(variant) {
  const bytes = readFileSync(join(corpusDir, `${variant}.wasm`));
  if (!WebAssembly.validate(bytes)) throw new Error(`${variant} module does not validate`);

  let started = now();
  const module = new WebAssembly.Module(bytes);
  const compileMs = elapsedMs(started);
  started = now();
  const instance = new WebAssembly.Instance(module);
  const instantiateMs = elapsedMs(started);
  if (typeof instance.exports.run !== "function" ||
      !(instance.exports.memory instanceof WebAssembly.Memory)) {
    throw new Error(`${variant} corpus exports are incomplete`);
  }
  const run = instance.exports.run;
  const selectorHash = hash(new Uint8Array(instance.exports.memory.buffer, 0, 8192));

  started = now();
  const firstChecksum = run(FIRST_STEPS);
  const firstMs = elapsedMs(started);
  const correctness = Object.fromEntries(CORRECTNESS_STEPS.map((steps) =>
    [steps, run(steps).toString()]));

  const warmMs = [];
  const warmChecksums = [];
  for (let repetition = 0; repetition < WARM_REPETITIONS; repetition++) {
    started = now();
    warmChecksums.push(run(WARM_STEPS).toString());
    warmMs.push(elapsedMs(started));
  }

  const steadyMs = [];
  const steadyChecksums = [];
  for (let sample = 0; sample < STEADY_SAMPLES; sample++) {
    let checksum = 0n;
    started = now();
    for (let repetition = 0; repetition < STEADY_REPETITIONS; repetition++) {
      checksum ^= run(STEADY_STEPS);
    }
    steadyMs.push(elapsedMs(started));
    steadyChecksums.push(checksum.toString());
  }

  process.stdout.write(JSON.stringify({
    variant,
    engine: { node: process.version, v8: process.versions.v8 },
    moduleBytes: bytes.length,
    moduleHash: hash(bytes),
    selectorHash,
    compileMs,
    instantiateMs,
    firstMs,
    firstChecksum: firstChecksum.toString(),
    correctness,
    warmMs,
    warmChecksums,
    steadyMs,
    steadyChecksums,
  }));
}

if (process.argv.includes("--worker")) {
  const variant = process.argv.find((argument) => argument.startsWith("--variant="))
    ?.split("=")[1];
  if (!variants.includes(variant)) throw new Error(`unknown worker variant ${variant}`);
  worker(variant);
  process.exit(0);
}

const sampleArgument = process.argv.find((argument) => argument.startsWith("--samples="));
const samples = sampleArgument ? Number(sampleArgument.split("=")[1]) : 7;
if (!Number.isInteger(samples) || samples < 3 || samples > 30 || !(samples & 1)) {
  throw new Error("--samples must be an odd integer from 3 through 29");
}

function emitCorpus(directory) {
  const result = spawnSync(
    "cargo",
    ["run", "--release", "-q", "-p", "rv64-dbt", "--example",
      "emit_pair_dispatch_corpus", "--", directory],
    { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "pair corpus generation failed");
  }
}

emitCorpus(corpusDir);
emitCorpus(repeatDir);
const generated = Object.fromEntries(variants.map((variant) => {
  const first = readFileSync(join(corpusDir, `${variant}.wasm`));
  const repeat = readFileSync(join(repeatDir, `${variant}.wasm`));
  return [variant, {
    bytes: first.length,
    hash: hash(first),
    repeatHash: hash(repeat),
    deterministic: first.equals(repeat),
    validates: WebAssembly.validate(first),
  }];
}));
if (variants.some((variant) => !generated[variant].deterministic ||
    !generated[variant].validates)) {
  throw new Error("generated corpus is nondeterministic or invalid");
}

const traceInputs = {
  boot: join(root, "target/bench/r047-superinstruction-exact-replay.json"),
  compile: join(root, "target/bench/r047-superinstruction-transfer-compile.json"),
  python: join(root, "target/bench/r047-superinstruction-transfer-python.json"),
};
const opportunityEvidence = Object.fromEntries(Object.entries(traceInputs).map(([name, path]) => {
  const bytes = readFileSync(path);
  const report = JSON.parse(bytes);
  const widthTwo = report.fixedWidthBounds?.find((row) => row.width === 2);
  if (report.meta?.operationKinds !== "62" || !widthTwo ||
      Number(widthTwo.fractionOfAllDispatches) < 0.4) {
    throw new Error(`${name} does not retain the preregistered pair opportunity`);
  }
  return [name, {
    report: path.slice(root.length + 1),
    reportHash: hash(bytes),
    totalInstructions: report.meta.totalInstructions,
    uniqueObservedPairs: report.uniquePairPatterns,
    fixedWidthTwoDispatchSavings: widthTwo.dispatchSavings,
    fractionOfAllDispatchesRemoved: widthTwo.fractionOfAllDispatches,
  }];
}));

const runs = Object.fromEntries(variants.map((variant) => [variant, []]));
for (let sample = 0; sample < samples; sample++) {
  const order = sample % 2 === 0 ? variants : [...variants].reverse();
  for (const variant of order) {
    const child = spawnSync(
      process.execPath,
      [self, "--worker", `--variant=${variant}`],
      { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 },
    );
    if (child.status !== 0) {
      throw new Error(child.stderr || child.stdout || `${variant} worker failed`);
    }
    runs[variant].push(JSON.parse(child.stdout));
  }
}

const engine = runs.single[0].engine;
if (variants.some((variant) => runs[variant].some((run) =>
  run.engine.node !== engine.node || run.engine.v8 !== engine.v8 ||
  run.moduleHash !== generated[variant].hash))) {
  throw new Error("engine or generated module changed within the paired run");
}

const variantStable = Object.fromEntries(variants.map((variant) => [variant,
  runs[variant].every((run) =>
    JSON.stringify(run.correctness) === JSON.stringify(runs[variant][0].correctness) &&
    run.firstChecksum === runs[variant][0].firstChecksum &&
    run.warmChecksums.every((value) => value === run.warmChecksums[0]) &&
    run.steadyChecksums.every((value) => value === run.steadyChecksums[0]))]));
const exactEquivalent = variantStable.single && variantStable.pair &&
  runs.single.every((run, index) =>
    JSON.stringify(run.correctness) === JSON.stringify(runs.pair[index].correctness) &&
    run.firstChecksum === runs.pair[index].firstChecksum &&
    run.warmChecksums[0] === runs.pair[index].warmChecksums[0] &&
    run.steadyChecksums[0] === runs.pair[index].steadyChecksums[0]);
if (!exactEquivalent) throw new Error("single and pair modules do not produce identical state");

function summarizeVariant(variant) {
  const variantRuns = runs[variant];
  return {
    moduleBytes: generated[variant].bytes,
    moduleHash: generated[variant].hash,
    selectorHash: variantRuns[0].selectorHash,
    compileMs: summary(variantRuns.map((run) => run.compileMs)),
    instantiateMs: summary(variantRuns.map((run) => run.instantiateMs)),
    coldConstructionMs: summary(variantRuns.map((run) => run.compileMs + run.instantiateMs)),
    firstMs: summary(variantRuns.map((run) => run.firstMs)),
    warmMs: summary(variantRuns.map((run) => median(run.warmMs))),
    steadyMs: summary(variantRuns.map((run) => median(run.steadyMs))),
    steadyMOperationsPerSecond: summary(variantRuns.map((run) =>
      STEADY_OPERATIONS / median(run.steadyMs) / 1000)),
    rawRuns: variantRuns,
  };
}

const pairedSteadySpeedup = summary(runs.single.map((run, index) =>
  median(run.steadyMs) / median(runs.pair[index].steadyMs)));
const pairedColdDeltaMs = summary(runs.pair.map((run, index) =>
  run.compileMs + run.instantiateMs -
    (runs.single[index].compileMs + runs.single[index].instantiateMs)));
const variantsSummary = Object.fromEntries(variants.map((variant) =>
  [variant, summarizeVariant(variant)]));
const gate = {
  exactStateEquivalence: exactEquivalent,
  deterministicValidBytes: variants.every((variant) =>
    generated[variant].deterministic && generated[variant].validates),
  requiredSteadySpeedup: REQUIRED_STEADY_SPEEDUP,
  observedPairedSteadySpeedup: pairedSteadySpeedup.median,
  steadySpeedupPass: pairedSteadySpeedup.median >= REQUIRED_STEADY_SPEEDUP,
  maxPairColdConstructionMs: MAX_PAIR_COLD_CONSTRUCTION_MS,
  observedPairColdConstructionMs: variantsSummary.pair.coldConstructionMs.median,
  pairColdConstructionPass:
    variantsSummary.pair.coldConstructionMs.median <= MAX_PAIR_COLD_CONSTRUCTION_MS,
  maxPairedColdDeltaMs: MAX_PAIR_COLD_DELTA_MS,
  observedPairedColdDeltaMs: pairedColdDeltaMs.median,
  pairedColdDeltaPass: pairedColdDeltaMs.median <= MAX_PAIR_COLD_DELTA_MS,
};
gate.admitProductionPrototype = gate.exactStateEquivalence &&
  gate.deterministicValidBytes && gate.steadySpeedupPass &&
  gate.pairColdConstructionPass && gate.pairedColdDeltaPass;

const report = {
  schema: 1,
  experiment: "R053",
  mechanism: "architecture-complete-exhaustive-two-operation-handler-tier",
  productionModified: false,
  methodology: "frozen-bytes/fresh-process/alternating-paired-order/balanced-all-pairs",
  samples,
  engine,
  operationKinds: 62,
  exhaustivePairHandlers: 62 ** 2,
  selectorCyclePairs: 4096,
  timing: {
    firstOperations: FIRST_STEPS,
    warmOperations: WARM_STEPS,
    warmRepetitions: WARM_REPETITIONS,
    steadyOperationsPerInvocation: STEADY_STEPS,
    steadyRepetitions: STEADY_REPETITIONS,
    steadySamplesPerProcess: STEADY_SAMPLES,
    steadyOperationsPerSample: STEADY_OPERATIONS,
  },
  opportunityEvidence,
  generated,
  correctness: {
    steps: CORRECTNESS_STEPS,
    exactEquivalent,
    variantStable,
    expectedChecksums: runs.single[0].correctness,
  },
  variants: variantsSummary,
  pairedSteadySpeedup,
  pairedColdDeltaMs,
  gate,
};

const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
const output = outputArgument
  ? outputArgument.split("=")[1]
  : join(root, "target/bench/r053-pair-dispatch-corpus.json");
mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);

if (process.argv.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const f = (value) => value.toFixed(3);
  console.log(`engine: Node ${engine.node}, V8 ${engine.v8}`);
  console.log(`samples: ${samples}; ${report.methodology}`);
  for (const variant of variants) {
    const row = variantsSummary[variant];
    console.log(
      `${variant}: ${row.moduleBytes} bytes; cold ${f(row.coldConstructionMs.median)} ms; ` +
      `steady ${f(row.steadyMOperationsPerSecond.median)} Mop/s`,
    );
  }
  console.log(
    `paired pair/single speedup ${f(pairedSteadySpeedup.median)}x ` +
      `[${pairedSteadySpeedup.medianConfidence95.map(f).join(", ")}]; ` +
      `cold delta ${f(pairedColdDeltaMs.median)} ms; ` +
      `admit=${gate.admitProductionPrototype}`,
  );
  console.log(`report: ${output}`);
}
