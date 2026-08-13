#!/usr/bin/env node

// Frozen R054 opportunity gate for direct interpreter consumption of the
// existing fused JIT-TLB capability. Production code is not modified.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { median, summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const self = fileURLToPath(import.meta.url);
const corpusDir = join(root, "target/jit-interpreter-fused-memory-corpus");
const repeatDir = join(root, "target/jit-interpreter-fused-memory-corpus-repeat");
const profilePath = join(root,
  "target/bench/engine-profile-r045/phase/rewrite-boot-first.cpuprofile");
const variants = ["control", "fused"];
const DATA_LINEAR = 0x44000;
const DATA_BYTES = 512;
const OPERATIONS_PER_ITERATION = 8;
const CORRECTNESS_ITERATIONS = [0, 1, 2, 17, 1024, 1_048_576];
const FIRST_ITERATIONS = 8192;
const WARM_ITERATIONS = 1_048_576;
const WARM_REPETITIONS = 8;
const STEADY_ITERATIONS = 4_194_304;
const STEADY_SAMPLES = 7;
const REQUIRED_SPEEDUP = 2.51;
const REQUIRED_LOWER_BOUND = 2.25;
const MAX_COLD_MS = 25;

const now = () => process.hrtime.bigint();
const elapsedMs = (started) => Number(process.hrtime.bigint() - started) / 1e6;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

function resetData(memory, initial) {
  new Uint8Array(memory.buffer, DATA_LINEAR, DATA_BYTES).set(initial);
}

function dataHash(memory) {
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
  const { run, memory } = instance.exports;
  if (typeof run !== "function" || !(memory instanceof WebAssembly.Memory)) {
    throw new Error(`${variant} corpus exports are incomplete`);
  }
  const initial = new Uint8Array(memory.buffer, DATA_LINEAR, DATA_BYTES).slice();

  resetData(memory, initial);
  started = now();
  const firstState = run(FIRST_ITERATIONS);
  const firstMs = elapsedMs(started);
  const firstMemoryHash = dataHash(memory);

  const correctness = Object.fromEntries(CORRECTNESS_ITERATIONS.map((iterations) => {
    resetData(memory, initial);
    const state = run(iterations);
    return [iterations, { state: state.toString(), memoryHash: dataHash(memory) }];
  }));

  const warmMs = [];
  const warmResults = [];
  for (let repetition = 0; repetition < WARM_REPETITIONS; repetition++) {
    resetData(memory, initial);
    started = now();
    const state = run(WARM_ITERATIONS);
    warmMs.push(elapsedMs(started));
    warmResults.push({ state: state.toString(), memoryHash: dataHash(memory) });
  }

  const steadyMs = [];
  const steadyResults = [];
  for (let sample = 0; sample < STEADY_SAMPLES; sample++) {
    resetData(memory, initial);
    started = now();
    const state = run(STEADY_ITERATIONS);
    steadyMs.push(elapsedMs(started));
    steadyResults.push({ state: state.toString(), memoryHash: dataHash(memory) });
  }

  process.stdout.write(JSON.stringify({
    variant,
    engine: { node: process.version, v8: process.versions.v8 },
    moduleBytes: bytes.length,
    moduleHash: hash(bytes),
    initialMemoryHash: hash(initial),
    compileMs,
    instantiateMs,
    firstMs,
    firstResult: { state: firstState.toString(), memoryHash: firstMemoryHash },
    correctness,
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
      "emit_interpreter_fused_memory_corpus", "--", directory],
    { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "fused-memory corpus generation failed");
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

function profileSubtrees(bytes) {
  const profile = JSON.parse(bytes);
  const nodes = new Map(profile.nodes.map((node) => [node.id, node]));
  const parent = new Map();
  for (const node of profile.nodes) {
    for (const child of node.children ?? []) parent.set(child, node.id);
  }
  let totalUs = 0;
  let loadUs = 0;
  let storeUs = 0;
  for (let sample = 0; sample < profile.samples.length; sample++) {
    const elapsedUs = profile.timeDeltas[sample] ?? 0;
    totalUs += elapsedUs;
    let id = profile.samples[sample];
    let load = false;
    let store = false;
    const seen = new Set();
    while (id && !seen.has(id)) {
      seen.add(id);
      const name = nodes.get(id)?.callFrame?.functionName ?? "";
      load ||= /Cpu2ld|Cpu::ld/.test(name);
      store ||= /Cpu2st|Cpu::st/.test(name);
      id = parent.get(id);
    }
    if (load) loadUs += elapsedUs;
    if (store) storeUs += elapsedUs;
  }
  const combinedFraction = (loadUs + storeUs) / totalUs;
  return {
    totalMs: totalUs / 1000,
    loadMs: loadUs / 1000,
    loadFraction: loadUs / totalUs,
    storeMs: storeUs / 1000,
    storeFraction: storeUs / totalUs,
    combinedMs: (loadUs + storeUs) / 1000,
    combinedFraction,
    minimumSubtreeSpeedupForTenPercentWholeRow: 1 / (1 - 0.1 / combinedFraction),
  };
}

const profileBytes = readFileSync(profilePath);
const opportunity = profileSubtrees(profileBytes);
if (opportunity.combinedFraction < 0.16 ||
    opportunity.minimumSubtreeSpeedupForTenPercentWholeRow > REQUIRED_SPEEDUP) {
  throw new Error(`accepted profile no longer supports the R054 bound: ${JSON.stringify(opportunity)}`);
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
    JSON.stringify(run.correctness) === JSON.stringify(runs[variant][0].correctness) &&
    JSON.stringify(run.firstResult) === JSON.stringify(runs[variant][0].firstResult) &&
    run.warmResults.every((result) =>
      JSON.stringify(result) === JSON.stringify(run.warmResults[0])) &&
    run.steadyResults.every((result) =>
      JSON.stringify(result) === JSON.stringify(run.steadyResults[0])))]));
const exactEquivalent = stable.control && stable.fused &&
  runs.control.every((run, index) =>
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
    steadyMOperationsPerSecond: summary(variantRuns.map((run) =>
      STEADY_ITERATIONS * OPERATIONS_PER_ITERATION / median(run.steadyMs) / 1000)),
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
  experiment: "R054",
  mechanism: "interpreter-direct-consumption-of-existing-fused-jit-tlb",
  productionModified: false,
  methodology: "frozen-bytes/fresh-process/alternating-paired-order/mixed-load-store",
  samples,
  engine,
  opportunityEvidence: {
    profile: profilePath.slice(root.length + 1),
    profileHash: hash(profileBytes),
    ...opportunity,
  },
  timing: {
    operationsPerIteration: OPERATIONS_PER_ITERATION,
    firstIterations: FIRST_ITERATIONS,
    warmIterations: WARM_ITERATIONS,
    warmRepetitions: WARM_REPETITIONS,
    steadyIterations: STEADY_ITERATIONS,
    steadySamplesPerProcess: STEADY_SAMPLES,
    steadyOperationsPerSample: STEADY_ITERATIONS * OPERATIONS_PER_ITERATION,
  },
  generated,
  correctness: {
    iterations: CORRECTNESS_ITERATIONS,
    exactEquivalent,
    stable,
    expected: runs.control[0].correctness,
  },
  variants: variantsSummary,
  pairedSteadySpeedup,
  pairedColdDeltaMs,
  gate,
};

const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
const output = outputArgument
  ? outputArgument.split("=")[1]
  : join(root, "target/bench/r054-interpreter-fused-memory-corpus.json");
mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);

if (process.argv.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const f = (value) => value.toFixed(3);
  console.log(`engine: Node ${engine.node}, V8 ${engine.v8}`);
  console.log(`samples: ${samples}; ${report.methodology}`);
  console.log(
    `Boot ld+st subtree ${f(opportunity.combinedFraction * 100)}%; ` +
      `minimum local speedup ${f(opportunity.minimumSubtreeSpeedupForTenPercentWholeRow)}x`,
  );
  for (const variant of variants) {
    const row = variantsSummary[variant];
    console.log(
      `${variant}: ${row.moduleBytes} bytes; cold ${f(row.coldConstructionMs.median)} ms; ` +
      `steady ${f(row.steadyMOperationsPerSecond.median)} Mop/s`,
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
