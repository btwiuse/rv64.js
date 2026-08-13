#!/usr/bin/env node

// Frozen R059 opportunity gate for flattening the RV64C quadrant/funct3
// dispatch. Production code is not modified by this experiment.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { median, summary } from "./statistics.mjs";
import { CPU_PROBE_SPEC, cpuProbe } from "./vs-v86/bench-math.mjs";
import { acquireBenchmarkLock } from "./vs-v86/bench-lock.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const self = fileURLToPath(import.meta.url);
const corpusDir = join(root, "target/jit-flat-rvc-dispatch-corpus");
const repeatDir = join(root, "target/jit-flat-rvc-dispatch-corpus-repeat");
const variants = ["nested", "flat"];
const CORRECTNESS_ITERATIONS = [0, 1, 2, 17, 24, 4097, 1_048_576];
const FIRST_ITERATIONS = 8192;
const WARM_ITERATIONS = 4_194_304;
const PREWARM_REPETITIONS = 8;
const FINAL_PREWARM_YIELDS = 16;
const WARM_REPETITIONS = 4;
const STEADY_ITERATIONS = 16_777_216;
const STEADY_SAMPLES = 5;
const REQUIRED_SPEEDUP = 1.45;
const REQUIRED_LOWER_BOUND = 1.40;
const MAX_COLD_MS = 25;
const MAX_SPREAD = 1.25;
const expectedBrTables = Object.freeze({ nested: 4, flat: 1 });

const immediate = () => new Promise((resolveImmediate) => setImmediate(resolveImmediate));
const now = () => process.hrtime.bigint();
const elapsedMs = (started) => Number(process.hrtime.bigint() - started) / 1e6;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

function spread(values) {
  const finite = values.filter(Number.isFinite);
  return finite.length ? Math.max(...finite) / Math.min(...finite) : Infinity;
}

function memoryHash(memory) {
  return hash(new Uint8Array(memory.buffer));
}

function probe(run, memory, iterations) {
  return {
    iterations,
    result: run(iterations).toString(),
    memoryHash: memoryHash(memory),
  };
}

async function fixedPrewarm(run) {
  const results = [];
  for (let repetition = 0; repetition < PREWARM_REPETITIONS; repetition++) {
    results.push(run(WARM_ITERATIONS).toString());
    await immediate();
  }
  for (let yieldIndex = 0; yieldIndex < FINAL_PREWARM_YIELDS; yieldIndex++) {
    await immediate();
  }
  return results;
}

async function sampleWorker(variant) {
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

  const originalMemory = Uint8Array.from(new Uint8Array(memory.buffer));
  const ordinary = CORRECTNESS_ITERATIONS.map((iterations) =>
    probe(run, memory, iterations));
  const mutable = new Uint8Array(memory.buffer);
  mutable[0] = 23;
  mutable[1] = 0;
  mutable[4095] = 7;
  const externallyMutated = [
    probe(run, memory, 1),
    probe(run, memory, 2),
    probe(run, memory, 4097),
  ];
  mutable.set(originalMemory);

  started = now();
  const firstResult = run(FIRST_ITERATIONS).toString();
  const firstMs = elapsedMs(started);
  const prewarmResults = await fixedPrewarm(run);

  const warmMs = [];
  const warmResults = [];
  for (let repetition = 0; repetition < WARM_REPETITIONS; repetition++) {
    started = now();
    warmResults.push(run(WARM_ITERATIONS).toString());
    warmMs.push(elapsedMs(started));
    await immediate();
  }
  const steadyMs = [];
  const steadyResults = [];
  for (let sample = 0; sample < STEADY_SAMPLES; sample++) {
    started = now();
    steadyResults.push(run(STEADY_ITERATIONS).toString());
    steadyMs.push(elapsedMs(started));
    await immediate();
  }

  process.stdout.write(JSON.stringify({
    variant,
    engine: { node: process.version, v8: process.versions.v8 },
    affinity: readFileSync("/proc/self/status", "utf8")
      .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
    moduleBytes: bytes.length,
    moduleHash: hash(bytes),
    compileMs,
    instantiateMs,
    correctness: { ordinary, externallyMutated },
    firstMs,
    firstResult,
    prewarmResults,
    warmMs,
    warmResults,
    steadyMs,
    steadyResults,
  }));
}

async function shapeWorker(variant) {
  const bytes = readFileSync(join(corpusDir, `${variant}.wasm`));
  const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));
  await fixedPrewarm(instance.exports.run);
  for (let repetition = 0; repetition < WARM_REPETITIONS; repetition++) {
    instance.exports.run(WARM_ITERATIONS);
    await immediate();
  }
}

function emitCorpus(directory) {
  const result = spawnSync(
    "cargo",
    ["run", "--release", "-q", "-p", "rv64-dbt", "--example",
      "emit_flat_rvc_dispatch_corpus", "--", directory],
    { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "R059 corpus generation failed");
  }
}

function staticShape(variant) {
  const result = spawnSync(
    "llvm-objdump",
    ["-d", join(corpusDir, `${variant}.wasm`)],
    { cwd: root, encoding: "utf8", maxBuffer: 16 << 20 },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || `${variant} disassembly failed`);
  }
  const disassembly = result.stdout;
  const brTables = (disassembly.match(/\bbr_table\b/g) ?? []).length;
  const calls = (disassembly.match(/\bcall(?:_indirect)?\b/g) ?? []).length;
  return {
    disassembly,
    brTables,
    calls,
    pass: brTables === expectedBrTables[variant] && calls === 0,
  };
}

function optimizingShape(variant) {
  const result = spawnSync(
    process.execPath,
    ["--trace-wasm-compilation-times", self, "--shape-worker", `--variant=${variant}`],
    { cwd: root, encoding: "utf8", maxBuffer: 16 << 20 },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || `${variant} shape worker failed`);
  }
  const trace = `${result.stdout}${result.stderr}`;
  const liftoff = /#0 using Liftoff/.test(trace);
  const turboFan = /#0 using TurboFan/.test(trace);
  const bodySizes = [...trace.matchAll(/#0 using (?:Liftoff|TurboFan).*?bodysize (\d+)/g)]
    .map((match) => Number(match[1]));
  return { trace, liftoff, turboFan, bodySizes, pass: liftoff && turboFan };
}

function summarizeVariant(generated, runs, variant) {
  const variantRuns = runs[variant];
  return {
    moduleBytes: generated[variant].bytes,
    moduleHash: generated[variant].hash,
    compileMs: summary(variantRuns.map((run) => run.compileMs)),
    instantiateMs: summary(variantRuns.map((run) => run.instantiateMs)),
    coldConstructionMs: summary(variantRuns.map((run) =>
      run.compileMs + run.instantiateMs)),
    firstMs: summary(variantRuns.map((run) => run.firstMs)),
    warmMs: summary(variantRuns.map((run) => median(run.warmMs))),
    steadyMs: summary(variantRuns.map((run) => median(run.steadyMs))),
    steadyMDispatchesPerSecond: summary(variantRuns.map((run) =>
      STEADY_ITERATIONS / median(run.steadyMs) / 1000)),
    rawRuns: variantRuns,
  };
}

async function orchestrate() {
  const sampleArgument = process.argv.find((argument) => argument.startsWith("--samples="));
  const samples = sampleArgument ? Number(sampleArgument.split("=")[1]) : 7;
  if (!Number.isInteger(samples) || samples < 3 || samples > 29 || !(samples & 1)) {
    throw new Error("--samples must be an odd integer from 3 through 29");
  }
  const artifacts = process.env.ARTIFACTS;
  if (!artifacts) throw new Error("set ARTIFACTS to acquire the benchmark lock");

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
    throw new Error("generated R059 corpus is nondeterministic or invalid");
  }

  const staticShapes = Object.fromEntries(variants.map((variant) =>
    [variant, staticShape(variant)]));
  const optimizingShapes = Object.fromEntries(variants.map((variant) =>
    [variant, optimizingShape(variant)]));
  if (variants.some((variant) => !staticShapes[variant].pass ||
      !optimizingShapes[variant].pass)) {
    throw new Error("R059 dispatch/tier shape check failed before timing");
  }

  const releaseLock = await acquireBenchmarkLock(resolve(artifacts));
  const runs = Object.fromEntries(variants.map((variant) => [variant, []]));
  const hostProbes = [];
  try {
    cpuProbe();
    for (let pair = 0; pair < samples; pair++) {
      const order = pair % 2 === 0 ? variants : [...variants].reverse();
      for (const variant of order) {
        const before = cpuProbe();
        const child = spawnSync(
          process.execPath,
          [self, "--sample-worker", `--variant=${variant}`],
          { cwd: root, encoding: "utf8", maxBuffer: 16 << 20 },
        );
        const after = cpuProbe();
        hostProbes.push({ pair: pair + 1, variant, before, after });
        if (child.status !== 0) {
          throw new Error(child.stderr || child.stdout || `${variant} sample failed`);
        }
        runs[variant].push(JSON.parse(child.stdout));
        process.stderr.write(`pair ${pair + 1}/${samples} ${variant} complete\n`);
      }
    }
  } finally {
    await releaseLock();
  }

  const referenceEngine = runs.nested[0].engine;
  const referenceAffinity = runs.nested[0].affinity;
  const exactInputs = variants.every((variant) => runs[variant].every((run) =>
    run.engine.node === referenceEngine.node && run.engine.v8 === referenceEngine.v8 &&
    run.affinity === referenceAffinity && run.moduleHash === generated[variant].hash));
  const correctnessStable = variants.every((variant) => runs[variant].every((run) =>
    JSON.stringify(run.correctness) === JSON.stringify(runs[variant][0].correctness)));
  const exactEquivalent = correctnessStable && runs.nested.every((run, index) =>
    JSON.stringify(run.correctness) === JSON.stringify(runs.flat[index].correctness));
  const resultStable = variants.every((variant) => runs[variant].every((run) =>
    run.firstResult === runs[variant][0].firstResult &&
    run.prewarmResults.every((result) => result === run.prewarmResults[0]) &&
    run.warmResults.every((result) => result === run.warmResults[0]) &&
    run.steadyResults.every((result) => result === run.steadyResults[0])));
  const exactTimedResults = resultStable && runs.nested.every((run, index) =>
    run.firstResult === runs.flat[index].firstResult &&
    run.prewarmResults[0] === runs.flat[index].prewarmResults[0] &&
    run.warmResults[0] === runs.flat[index].warmResults[0] &&
    run.steadyResults[0] === runs.flat[index].steadyResults[0]);
  const warmStable = variants.every((variant) => runs[variant].every((run) =>
    spread(run.warmMs) <= MAX_SPREAD));
  const hostValues = hostProbes.flatMap((probe) => [probe.before, probe.after]);
  const hostProbeSpread = spread(hostValues);

  const variantsSummary = Object.fromEntries(variants.map((variant) =>
    [variant, summarizeVariant(generated, runs, variant)]));
  const pairedSteadySpeedup = summary(runs.nested.map((run, index) =>
    median(run.steadyMs) / median(runs.flat[index].steadyMs)));
  const pairedColdDeltaMs = summary(runs.flat.map((run, index) =>
    run.compileMs + run.instantiateMs -
      (runs.nested[index].compileMs + runs.nested[index].instantiateMs)));
  const lowerBound = pairedSteadySpeedup.medianConfidence95?.[0] ?? 0;
  const flatCold = variantsSummary.flat.coldConstructionMs.median;
  const shapePass = variants.every((variant) =>
    staticShapes[variant].pass && optimizingShapes[variant].pass);
  const gate = {
    exactInputs,
    deterministicValidBytes: variants.every((variant) =>
      generated[variant].deterministic && generated[variant].validates),
    exactCorrectnessAndMemory: exactEquivalent,
    exactTimedResults,
    shapePass,
    warmStable,
    maximumWarmSpread: MAX_SPREAD,
    hostProbeSpread,
    maximumHostProbeSpread: MAX_SPREAD,
    hostStable: hostProbeSpread <= MAX_SPREAD,
    requiredSteadySpeedup: REQUIRED_SPEEDUP,
    observedPairedSteadySpeedup: pairedSteadySpeedup.median,
    steadySpeedupPass: pairedSteadySpeedup.median >= REQUIRED_SPEEDUP,
    requiredLowerMedianBound: REQUIRED_LOWER_BOUND,
    observedLowerMedianBound: lowerBound,
    lowerMedianBoundPass: lowerBound >= REQUIRED_LOWER_BOUND,
    maxFlatColdConstructionMs: MAX_COLD_MS,
    observedFlatColdConstructionMs: flatCold,
    flatColdConstructionPass: flatCold <= MAX_COLD_MS,
    maxPairedColdDeltaMs: MAX_COLD_MS,
    observedPairedColdDeltaMs: pairedColdDeltaMs.median,
    pairedColdDeltaPass: pairedColdDeltaMs.median <= MAX_COLD_MS,
  };
  gate.admitProductionPrototype = gate.exactInputs &&
    gate.deterministicValidBytes && gate.exactCorrectnessAndMemory &&
    gate.exactTimedResults && gate.shapePass && gate.warmStable && gate.hostStable &&
    gate.steadySpeedupPass && gate.lowerMedianBoundPass &&
    gate.flatColdConstructionPass && gate.pairedColdDeltaPass;

  const report = {
    schema: 1,
    experiment: "R059",
    mechanism: "flattened-complete-rv64c-quadrant-funct3-dispatch",
    productionModified: false,
    methodology:
      "architecture-balanced/fresh-process/alternating-paired-order/fixed-tier-yields",
    samples,
    engine: referenceEngine,
    affinity: referenceAffinity,
    timing: {
      correctnessIterations: CORRECTNESS_ITERATIONS,
      firstIterations: FIRST_ITERATIONS,
      prewarmIterations: WARM_ITERATIONS,
      prewarmRepetitions: PREWARM_REPETITIONS,
      finalPrewarmYields: FINAL_PREWARM_YIELDS,
      warmIterations: WARM_ITERATIONS,
      warmRepetitions: WARM_REPETITIONS,
      steadyIterations: STEADY_ITERATIONS,
      steadySamplesPerProcess: STEADY_SAMPLES,
    },
    generated,
    staticShape: Object.fromEntries(variants.map((variant) => [variant, {
      brTables: staticShapes[variant].brTables,
      calls: staticShapes[variant].calls,
      pass: staticShapes[variant].pass,
    }])),
    optimizingShape: Object.fromEntries(variants.map((variant) => [variant, {
      liftoff: optimizingShapes[variant].liftoff,
      turboFan: optimizingShapes[variant].turboFan,
      bodySizes: optimizingShapes[variant].bodySizes,
      pass: optimizingShapes[variant].pass,
    }])),
    correctness: {
      exactEquivalent,
      stable: correctnessStable,
      expected: runs.nested[0].correctness,
    },
    cpuProbe: CPU_PROBE_SPEC,
    hostProbes,
    hostProbeSpread,
    variants: variantsSummary,
    pairedSteadySpeedup,
    pairedColdDeltaMs,
    gate,
  };

  const output = join(root, "target/bench/r059-flat-rvc-dispatch-corpus.json");
  const shapeDirectory = join(root, "target/bench/r059-flat-rvc-dispatch-shape");
  mkdirSync(dirname(output), { recursive: true });
  mkdirSync(shapeDirectory, { recursive: true });
  for (const variant of variants) {
    writeFileSync(
      join(shapeDirectory, `${variant}.objdump.txt`),
      staticShapes[variant].disassembly,
      { flag: "wx" },
    );
    writeFileSync(
      join(shapeDirectory, `${variant}.tier.log`),
      optimizingShapes[variant].trace,
      { flag: "wx" },
    );
  }
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
  console.log(
    `nested ${variantsSummary.nested.steadyMDispatchesPerSecond.median.toFixed(3)} Mdispatch/s`,
  );
  console.log(
    `flat   ${variantsSummary.flat.steadyMDispatchesPerSecond.median.toFixed(3)} Mdispatch/s`,
  );
  console.log(
    `paired speedup ${pairedSteadySpeedup.median.toFixed(3)}x ` +
    `CI=[${pairedSteadySpeedup.medianConfidence95[0].toFixed(3)}, ` +
    `${pairedSteadySpeedup.medianConfidence95[1].toFixed(3)}]`,
  );
  console.log(`admit production prototype: ${gate.admitProductionPrototype}`);
  console.log(`saved ${output}`);
}

if (process.argv.includes("--sample-worker")) {
  const variant = process.argv.find((argument) => argument.startsWith("--variant="))
    ?.split("=")[1];
  if (!variants.includes(variant)) throw new Error(`unknown sample variant ${variant}`);
  await sampleWorker(variant);
} else if (process.argv.includes("--shape-worker")) {
  const variant = process.argv.find((argument) => argument.startsWith("--variant="))
    ?.split("=")[1];
  if (!variants.includes(variant)) throw new Error(`unknown shape variant ${variant}`);
  await shapeWorker(variant);
} else {
  await orchestrate();
}
