#!/usr/bin/env node

// Frozen R055 opportunity gate for an execute-context-tagged interpreter
// instruction-fetch pointer capability. Production code is not modified.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { median, summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const self = fileURLToPath(import.meta.url);
const corpusDir = join(root, "target/jit-interpreter-fused-fetch-corpus");
const repeatDir = join(root, "target/jit-interpreter-fused-fetch-corpus-repeat");
const variants = ["control", "fused"];
const DATA_LINEAR = 0x22000;
const DATA_BYTES = 0x2000;
const HALFWORD_FETCHES_PER_ITERATION = 12;
const CORRECTNESS_ITERATIONS = [0, 1, 2, 17, 1024, 1_048_576];
const FIRST_ITERATIONS = 8192;
const WARM_ITERATIONS = 1_048_576;
const WARM_REPETITIONS = 8;
const STEADY_ITERATIONS = 4_194_304;
const STEADY_SAMPLES = 7;
const REQUIRED_SPEEDUP = 1.50;
const REQUIRED_LOWER_BOUND = 1.35;
const MAX_COLD_MS = 25;

const now = () => process.hrtime.bigint();
const elapsedMs = (started) => Number(process.hrtime.bigint() - started) / 1e6;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

function memoryHash(memory) {
  return hash(new Uint8Array(memory.buffer, DATA_LINEAR, DATA_BYTES));
}

function worker(variant) {
  const bytes = readFileSync(join(corpusDir, `${variant}.wasm`));
  if (!WebAssembly.validate(bytes)) throw new Error(`${variant} module does not validate`);
  let started = now();
  const module = new WebAssembly.Module(bytes);
  const compileMs = elapsedMs(started);
  started = now();
  const instance = new WebAssembly.Instance(module);
  const instantiateMs = elapsedMs(started);
  const { run, edge, memory } = instance.exports;
  if (typeof run !== "function" || typeof edge !== "function" ||
      !(memory instanceof WebAssembly.Memory)) {
    throw new Error(`${variant} corpus exports are incomplete`);
  }

  const initialMemoryHash = memoryHash(memory);
  const edgeResults = Array.from({ length: 5 }, () => ({
    state: edge(0).toString(),
    memoryHash: memoryHash(memory),
  }));
  const correctness = Object.fromEntries(CORRECTNESS_ITERATIONS.map((iterations) =>
    [iterations, { state: run(iterations).toString(), memoryHash: memoryHash(memory) }]));

  started = now();
  const firstState = run(FIRST_ITERATIONS);
  const firstMs = elapsedMs(started);
  const firstResult = { state: firstState.toString(), memoryHash: memoryHash(memory) };

  const warmMs = [];
  const warmResults = [];
  for (let repetition = 0; repetition < WARM_REPETITIONS; repetition++) {
    started = now();
    const state = run(WARM_ITERATIONS);
    warmMs.push(elapsedMs(started));
    warmResults.push({ state: state.toString(), memoryHash: memoryHash(memory) });
  }

  const steadyMs = [];
  const steadyResults = [];
  for (let sample = 0; sample < STEADY_SAMPLES; sample++) {
    started = now();
    const state = run(STEADY_ITERATIONS);
    steadyMs.push(elapsedMs(started));
    steadyResults.push({ state: state.toString(), memoryHash: memoryHash(memory) });
  }

  process.stdout.write(JSON.stringify({
    variant,
    engine: { node: process.version, v8: process.versions.v8 },
    moduleBytes: bytes.length,
    moduleHash: hash(bytes),
    initialMemoryHash,
    compileMs,
    instantiateMs,
    edgeResults,
    correctness,
    firstMs,
    firstResult,
    warmMs,
    warmResults,
    steadyMs,
    steadyResults,
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
if (!Number.isInteger(samples) || samples < 3 || samples > 29 || !(samples & 1)) {
  throw new Error("--samples must be an odd integer from 3 through 29");
}

function emitCorpus(directory) {
  const result = spawnSync(
    "cargo",
    ["run", "--release", "-q", "-p", "rv64-dbt", "--example",
      "emit_interpreter_fused_fetch_corpus", "--", directory],
    { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "fused-fetch corpus generation failed");
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

const engine = runs.control[0].engine;
if (variants.some((variant) => runs[variant].some((run) =>
  run.engine.node !== engine.node || run.engine.v8 !== engine.v8 ||
  run.moduleHash !== generated[variant].hash))) {
  throw new Error("engine or generated module changed within the paired run");
}

const stable = Object.fromEntries(variants.map((variant) => [variant,
  runs[variant].every((run) =>
    JSON.stringify(run.edgeResults) === JSON.stringify(runs[variant][0].edgeResults) &&
    JSON.stringify(run.correctness) === JSON.stringify(runs[variant][0].correctness) &&
    JSON.stringify(run.firstResult) === JSON.stringify(runs[variant][0].firstResult) &&
    run.warmResults.every((result) =>
      JSON.stringify(result) === JSON.stringify(run.warmResults[0])) &&
    run.steadyResults.every((result) =>
      JSON.stringify(result) === JSON.stringify(run.steadyResults[0])))]));
const exactEquivalent = stable.control && stable.fused &&
  runs.control.every((run, index) =>
    JSON.stringify(run.edgeResults) === JSON.stringify(runs.fused[index].edgeResults) &&
    JSON.stringify(run.correctness) === JSON.stringify(runs.fused[index].correctness) &&
    JSON.stringify(run.firstResult) === JSON.stringify(runs.fused[index].firstResult) &&
    JSON.stringify(run.warmResults[0]) === JSON.stringify(runs.fused[index].warmResults[0]) &&
    JSON.stringify(run.steadyResults[0]) === JSON.stringify(runs.fused[index].steadyResults[0]));
if (!exactEquivalent) throw new Error("control and fused modules do not produce identical state");

function summarizeVariant(variant) {
  const variantRuns = runs[variant];
  return {
    moduleBytes: generated[variant].bytes,
    moduleHash: generated[variant].hash,
    compileMs: summary(variantRuns.map((run) => run.compileMs)),
    instantiateMs: summary(variantRuns.map((run) => run.instantiateMs)),
    coldConstructionMs: summary(variantRuns.map((run) => run.compileMs + run.instantiateMs)),
    firstMs: summary(variantRuns.map((run) => run.firstMs)),
    warmMs: summary(variantRuns.map((run) => median(run.warmMs))),
    steadyMs: summary(variantRuns.map((run) => median(run.steadyMs))),
    steadyMHalfwordFetchesPerSecond: summary(variantRuns.map((run) =>
      STEADY_ITERATIONS * HALFWORD_FETCHES_PER_ITERATION / median(run.steadyMs) / 1000)),
    rawRuns: variantRuns,
  };
}

const variantsSummary = Object.fromEntries(variants.map((variant) =>
  [variant, summarizeVariant(variant)]));
const pairedSteadySpeedup = summary(runs.control.map((run, index) =>
  median(run.steadyMs) / median(runs.fused[index].steadyMs)));
const pairedColdDeltaMs = summary(runs.fused.map((run, index) =>
  run.compileMs + run.instantiateMs -
    (runs.control[index].compileMs + runs.control[index].instantiateMs)));
const lowerBound = pairedSteadySpeedup.medianConfidence95?.[0] ?? 0;
const gate = {
  exactStateAndMemoryEquivalence: exactEquivalent,
  deterministicValidBytes: variants.every((variant) =>
    generated[variant].deterministic && generated[variant].validates),
  requiredSteadySpeedup: REQUIRED_SPEEDUP,
  observedPairedSteadySpeedup: pairedSteadySpeedup.median,
  steadySpeedupPass: pairedSteadySpeedup.median >= REQUIRED_SPEEDUP,
  requiredLowerMedianBound: REQUIRED_LOWER_BOUND,
  observedLowerMedianBound: lowerBound,
  lowerMedianBoundPass: lowerBound >= REQUIRED_LOWER_BOUND,
  maxFusedColdConstructionMs: MAX_COLD_MS,
  observedFusedColdConstructionMs: variantsSummary.fused.coldConstructionMs.median,
  fusedColdConstructionPass: variantsSummary.fused.coldConstructionMs.median <= MAX_COLD_MS,
  maxPairedColdDeltaMs: MAX_COLD_MS,
  observedPairedColdDeltaMs: pairedColdDeltaMs.median,
  pairedColdDeltaPass: pairedColdDeltaMs.median <= MAX_COLD_MS,
};
gate.admitProductionPrototype = gate.exactStateAndMemoryEquivalence &&
  gate.deterministicValidBytes && gate.steadySpeedupPass &&
  gate.lowerMedianBoundPass && gate.fusedColdConstructionPass &&
  gate.pairedColdDeltaPass;

const report = {
  schema: 1,
  experiment: "R055",
  mechanism: "interpreter-exact-fused-fetch-pointer-capability",
  productionModified: false,
  methodology: "frozen-bytes/fresh-process/alternating-paired-order/mixed-rv64-fetch",
  samples,
  engine,
  timing: {
    hotInstructionsPerIteration: 8,
    halfwordFetchesPerIteration: HALFWORD_FETCHES_PER_ITERATION,
    firstIterations: FIRST_ITERATIONS,
    warmIterations: WARM_ITERATIONS,
    warmRepetitions: WARM_REPETITIONS,
    steadyIterations: STEADY_ITERATIONS,
    steadySamplesPerProcess: STEADY_SAMPLES,
  },
  generated,
  correctness: {
    iterations: CORRECTNESS_ITERATIONS,
    edgeExercises: ["32-bit-fetch-at-page-offset-0xffe", "next-page", "return-page"],
    exactEquivalent,
    stable,
    expected: runs.control[0].correctness,
    edgeExpected: runs.control[0].edgeResults[0],
  },
  variants: variantsSummary,
  pairedSteadySpeedup,
  pairedColdDeltaMs,
  gate,
};

const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
const output = outputArgument
  ? outputArgument.split("=")[1]
  : join(root, "target/bench/r055-interpreter-fused-fetch-corpus.json");
mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });

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
      `steady ${f(row.steadyMHalfwordFetchesPerSecond.median)} Mfetch/s`,
    );
  }
  console.log(
    `paired fused/control speedup ${f(pairedSteadySpeedup.median)}x ` +
      `[${pairedSteadySpeedup.medianConfidence95.map(f).join(", ")}]; ` +
      `cold delta ${f(pairedColdDeltaMs.median)} ms; ` +
      `admit=${gate.admitProductionPrototype}`,
  );
  console.log(`report: ${output}`);
}
