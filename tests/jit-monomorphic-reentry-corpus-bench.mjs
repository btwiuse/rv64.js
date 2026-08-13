#!/usr/bin/env node

// Frozen R056 opportunity gate for monomorphizing the exact interpreter
// generated-entry predicate. Production code is not modified.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { median, summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const self = fileURLToPath(import.meta.url);
const corpusDir = join(root, "target/jit-monomorphic-reentry-corpus");
const repeatDir = join(root, "target/jit-monomorphic-reentry-corpus-repeat");
const variants = ["indirect", "inline"];
const PC_BASE = 0x0000_5555_0000_0000n;
const DISPATCH_MASK = 4095n;
const DISPATCH_STRIDE = 16;
const CORRECTNESS_ITERATIONS = [0, 1, 2, 17, 4096, 1_048_576];
const FIRST_ITERATIONS = 8192;
const WARM_ITERATIONS = 2_097_152;
const WARM_REPETITIONS = 4;
const STEADY_ITERATIONS = 16_777_216;
const STEADY_SAMPLES = 5;
const REQUIRED_SPEEDUP = 3.30;
const REQUIRED_LOWER_BOUND = 3.00;
const MAX_COLD_MS = 25;

const now = () => process.hrtime.bigint();
const elapsedMs = (started) => Number(process.hrtime.bigint() - started) / 1e6;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

function clearTags(memory) {
  new Uint8Array(memory.buffer).fill(0);
}

function installFirstHit(memory) {
  const pc = PC_BASE + 2n;
  const index = Number((pc >> 1n) & DISPATCH_MASK);
  new DataView(memory.buffer).setBigUint64(index * DISPATCH_STRIDE, pc, true);
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
  const { run, memory, table } = instance.exports;
  if (typeof run !== "function" || !(memory instanceof WebAssembly.Memory) ||
      !(table instanceof WebAssembly.Table)) {
    throw new Error(`${variant} corpus exports are incomplete`);
  }

  clearTags(memory);
  const missCorrectness = Object.fromEntries(CORRECTNESS_ITERATIONS.map((iterations) =>
    [iterations, run(iterations).toString()]));
  clearTags(memory);
  installFirstHit(memory);
  const firstHitResult = run(1).toString();
  clearTags(memory);

  started = now();
  const firstResult = run(FIRST_ITERATIONS).toString();
  const firstMs = elapsedMs(started);

  const warmMs = [];
  const warmResults = [];
  for (let repetition = 0; repetition < WARM_REPETITIONS; repetition++) {
    started = now();
    warmResults.push(run(WARM_ITERATIONS).toString());
    warmMs.push(elapsedMs(started));
  }

  const steadyMs = [];
  const steadyResults = [];
  for (let sample = 0; sample < STEADY_SAMPLES; sample++) {
    started = now();
    steadyResults.push(run(STEADY_ITERATIONS).toString());
    steadyMs.push(elapsedMs(started));
  }

  process.stdout.write(JSON.stringify({
    variant,
    engine: { node: process.version, v8: process.versions.v8 },
    moduleBytes: bytes.length,
    moduleHash: hash(bytes),
    compileMs,
    instantiateMs,
    missCorrectness,
    firstHitResult,
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
      "emit_monomorphic_reentry_corpus", "--", directory],
    { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "R056 corpus generation failed");
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

const engine = runs.indirect[0].engine;
if (variants.some((variant) => runs[variant].some((run) =>
  run.engine.node !== engine.node || run.engine.v8 !== engine.v8 ||
  run.moduleHash !== generated[variant].hash))) {
  throw new Error("engine or generated module changed within the paired run");
}

const stable = Object.fromEntries(variants.map((variant) => [variant,
  runs[variant].every((run) =>
    JSON.stringify(run.missCorrectness) === JSON.stringify(runs[variant][0].missCorrectness) &&
    run.firstHitResult === runs[variant][0].firstHitResult &&
    run.firstResult === runs[variant][0].firstResult &&
    run.warmResults.every((value) => value === run.warmResults[0]) &&
    run.steadyResults.every((value) => value === run.steadyResults[0]))]));
const exactEquivalent = stable.indirect && stable.inline &&
  runs.indirect.every((run, index) =>
    JSON.stringify(run.missCorrectness) === JSON.stringify(runs.inline[index].missCorrectness) &&
    run.firstHitResult === runs.inline[index].firstHitResult &&
    run.firstResult === runs.inline[index].firstResult &&
    run.warmResults[0] === runs.inline[index].warmResults[0] &&
    run.steadyResults[0] === runs.inline[index].steadyResults[0]);
if (!exactEquivalent) throw new Error("indirect and inline modules do not produce identical state");

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
    steadyMProbesPerSecond: summary(variantRuns.map((run) =>
      STEADY_ITERATIONS / median(run.steadyMs) / 1000)),
    rawRuns: variantRuns,
  };
}

const variantsSummary = Object.fromEntries(variants.map((variant) =>
  [variant, summarizeVariant(variant)]));
const pairedSteadySpeedup = summary(runs.indirect.map((run, index) =>
  median(run.steadyMs) / median(runs.inline[index].steadyMs)));
const pairedColdDeltaMs = summary(runs.inline.map((run, index) =>
  run.compileMs + run.instantiateMs -
    (runs.indirect[index].compileMs + runs.indirect[index].instantiateMs)));
const lowerBound = pairedSteadySpeedup.medianConfidence95?.[0] ?? 0;
const gate = {
  exactMissAndHitEquivalence: exactEquivalent,
  deterministicValidBytes: variants.every((variant) =>
    generated[variant].deterministic && generated[variant].validates),
  requiredSteadySpeedup: REQUIRED_SPEEDUP,
  observedPairedSteadySpeedup: pairedSteadySpeedup.median,
  steadySpeedupPass: pairedSteadySpeedup.median >= REQUIRED_SPEEDUP,
  requiredLowerMedianBound: REQUIRED_LOWER_BOUND,
  observedLowerMedianBound: lowerBound,
  lowerMedianBoundPass: lowerBound >= REQUIRED_LOWER_BOUND,
  maxInlineColdConstructionMs: MAX_COLD_MS,
  observedInlineColdConstructionMs: variantsSummary.inline.coldConstructionMs.median,
  inlineColdConstructionPass:
    variantsSummary.inline.coldConstructionMs.median <= MAX_COLD_MS,
  maxPairedColdDeltaMs: MAX_COLD_MS,
  observedPairedColdDeltaMs: pairedColdDeltaMs.median,
  pairedColdDeltaPass: pairedColdDeltaMs.median <= MAX_COLD_MS,
};
gate.admitProductionPrototype = gate.exactMissAndHitEquivalence &&
  gate.deterministicValidBytes && gate.steadySpeedupPass &&
  gate.lowerMedianBoundPass && gate.inlineColdConstructionPass &&
  gate.pairedColdDeltaPass;

const report = {
  schema: 1,
  experiment: "R056",
  mechanism: "monomorphic-exact-interpreter-reentry-predicate",
  productionModified: false,
  methodology: "frozen-bytes/fresh-process/alternating-paired-order/exact-dispatch-tag",
  samples,
  engine,
  timing: {
    firstIterations: FIRST_ITERATIONS,
    warmIterations: WARM_ITERATIONS,
    warmRepetitions: WARM_REPETITIONS,
    steadyIterations: STEADY_ITERATIONS,
    steadySamplesPerProcess: STEADY_SAMPLES,
  },
  generated,
  correctness: {
    iterations: CORRECTNESS_ITERATIONS,
    exactEquivalent,
    stable,
    missExpected: runs.indirect[0].missCorrectness,
    firstHitExpected: runs.indirect[0].firstHitResult,
  },
  variants: variantsSummary,
  pairedSteadySpeedup,
  pairedColdDeltaMs,
  gate,
};

const output = join(root, "target/bench/r056-monomorphic-reentry-corpus.json");
mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(`indirect ${variantsSummary.indirect.steadyMProbesPerSecond.median.toFixed(3)} Mprobe/s`);
console.log(`inline   ${variantsSummary.inline.steadyMProbesPerSecond.median.toFixed(3)} Mprobe/s`);
console.log(
  `paired speedup ${pairedSteadySpeedup.median.toFixed(3)}x ` +
  `CI=[${pairedSteadySpeedup.medianConfidence95[0].toFixed(3)}, ` +
  `${pairedSteadySpeedup.medianConfidence95[1].toFixed(3)}]`,
);
console.log(`admit production prototype: ${gate.admitProductionPrototype}`);
console.log(`saved ${output}`);
