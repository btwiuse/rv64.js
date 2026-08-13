#!/usr/bin/env node

// Frozen R058 opportunity gate for replacing the interpreter step's linear-
// memory structure return with a compact scalar outcome. Production code is
// not modified by this experiment.

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
const corpusDir = join(root, "target/jit-compact-step-outcome-corpus");
const repeatDir = join(root, "target/jit-compact-step-outcome-corpus-repeat");
const variants = ["sret", "compact"];
const CORRECTNESS_ITERATIONS = [0, 1, 2, 17, 4096, 1_048_576];
const FIRST_ITERATIONS = 8192;
const WARM_ITERATIONS = 4_194_304;
const WARM_REPETITIONS = 4;
const STEADY_ITERATIONS = 16_777_216;
const STEADY_SAMPLES = 5;
const SHAPE_ITERATIONS = 1_048_576;
const SHAPE_REPETITIONS = 8;
const REQUIRED_SPEEDUP = 1.20;
const REQUIRED_LOWER_BOUND = 1.15;
const MAX_COLD_MS = 25;
const MAX_SPREAD = 1.25;
const PADDING_NOPS = 640;
const REQUIRED_STEP_WIRE_BYTES = 501;

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

function probe(run, memory, iterations, mode) {
  const result = run(iterations, mode).toString();
  return { iterations, mode, result, memoryHash: memoryHash(memory) };
}

function sampleWorker(variant) {
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

  const correctness = {
    continue: CORRECTNESS_ITERATIONS.map((iterations) =>
      probe(run, memory, iterations, 0)),
    stop: probe(run, memory, 17, 1),
    exception: probe(run, memory, 17, 2),
  };

  started = now();
  const firstResult = run(FIRST_ITERATIONS, 0).toString();
  const firstMs = elapsedMs(started);
  const warmMs = [];
  const warmResults = [];
  for (let repetition = 0; repetition < WARM_REPETITIONS; repetition++) {
    started = now();
    warmResults.push(run(WARM_ITERATIONS, 0).toString());
    warmMs.push(elapsedMs(started));
  }
  const steadyMs = [];
  const steadyResults = [];
  for (let sample = 0; sample < STEADY_SAMPLES; sample++) {
    started = now();
    steadyResults.push(run(STEADY_ITERATIONS, 0).toString());
    steadyMs.push(elapsedMs(started));
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
    correctness,
    firstMs,
    firstResult,
    warmMs,
    warmResults,
    steadyMs,
    steadyResults,
  }));
}

function shapeWorker(variant) {
  const bytes = readFileSync(join(corpusDir, `${variant}.wasm`));
  const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));
  for (let repetition = 0; repetition < SHAPE_REPETITIONS; repetition++) {
    instance.exports.run(SHAPE_ITERATIONS, 0);
  }
}

if (process.argv.includes("--sample-worker")) {
  const variant = process.argv.find((argument) => argument.startsWith("--variant="))
    ?.split("=")[1];
  if (!variants.includes(variant)) throw new Error(`unknown sample variant ${variant}`);
  sampleWorker(variant);
  process.exit(0);
}

if (process.argv.includes("--shape-worker")) {
  const variant = process.argv.find((argument) => argument.startsWith("--variant="))
    ?.split("=")[1];
  if (!variants.includes(variant)) throw new Error(`unknown shape variant ${variant}`);
  shapeWorker(variant);
  process.exit(0);
}

function emitCorpus(directory) {
  const result = spawnSync(
    "cargo",
    ["run", "--release", "-q", "-p", "rv64-dbt", "--example",
      "emit_compact_step_outcome_corpus", "--", directory],
    { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "R058 corpus generation failed");
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
  const stepStart = disassembly.indexOf(`<${variant}_step>`);
  const driverStart = disassembly.indexOf(`<${variant}_driver>`);
  if (stepStart < 0 || driverStart < 0 || driverStart <= stepStart) {
    throw new Error(`${variant} named functions absent from disassembly`);
  }
  const step = disassembly.slice(stepStart, driverStart);
  const driver = disassembly.slice(driverStart);
  const nopCount = (step.match(/\bnop\b/g) ?? []).length;
  const directStepCalls = (driver.match(/\bcall\s+0\b/g) ?? []).length;
  return {
    disassembly,
    nopCount,
    directStepCalls,
    staticPass: nopCount === PADDING_NOPS && directStepCalls === 1,
  };
}

function optimizingShape(variant) {
  const result = spawnSync(
    process.execPath,
    ["--wasm-sync-tier-up", "--trace-wasm-inlining",
      "--trace-wasm-compilation-times", self, "--shape-worker", `--variant=${variant}`],
    { cwd: root, encoding: "utf8", maxBuffer: 16 << 20 },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || `${variant} shape worker failed`);
  }
  const trace = `${result.stdout}${result.stderr}`;
  const stepBodySizes = [...trace.matchAll(
    /#0 using (?:Liftoff|TurboFan).*?bodysize (\d+)/g,
  )].map((match) => Number(match[1]));
  const driverBodySizes = [...trace.matchAll(
    /#1 using (?:Liftoff|TurboFan).*?bodysize (\d+)/g,
  )].map((match) => Number(match[1]));
  const stepTurboFan = /#0 using TurboFan/.test(trace);
  const driverTurboFan = /#1 using TurboFan/.test(trace);
  const retainedCall = /function 1:.*considering call #0.*to function 0.*not enough inlining budget/
    .test(trace);
  const stepWireBytes = stepBodySizes.length ? Math.max(...stepBodySizes) : 0;
  return {
    trace,
    stepBodySizes,
    driverBodySizes,
    stepTurboFan,
    driverTurboFan,
    retainedCall,
    stepWireBytes,
    optimizingPass: stepTurboFan && driverTurboFan && retainedCall &&
      stepWireBytes >= REQUIRED_STEP_WIRE_BYTES,
  };
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
    steadyMInstructionsPerSecond: summary(variantRuns.map((run) =>
      STEADY_ITERATIONS / median(run.steadyMs) / 1000)),
    rawRuns: variantRuns,
  };
}

async function main() {
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
    throw new Error("generated R058 corpus is nondeterministic or invalid");
  }

  const staticShapes = Object.fromEntries(variants.map((variant) =>
    [variant, staticShape(variant)]));
  const optimizingShapes = Object.fromEntries(variants.map((variant) =>
    [variant, optimizingShape(variant)]));
  if (variants.some((variant) => !staticShapes[variant].staticPass ||
      !optimizingShapes[variant].optimizingPass)) {
    throw new Error("R058 direct-call/non-inlining shape check failed before timing");
  }

  const options = spawnSync(process.execPath, ["--v8-options"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 16 << 20,
  });
  const inliningMaximum = Number(options.stdout.match(
    /--wasm-inlining-max-size[^\n]*\n\s*type:[^\n]*default: --wasm-inlining-max-size=(\d+)/,
  )?.[1]);
  if (inliningMaximum !== 500) {
    throw new Error(`expected V8 maximum inlinee size 500, got ${inliningMaximum}`);
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

  const referenceEngine = runs.sret[0].engine;
  const referenceAffinity = runs.sret[0].affinity;
  const exactInputs = variants.every((variant) => runs[variant].every((run) =>
    run.engine.node === referenceEngine.node && run.engine.v8 === referenceEngine.v8 &&
    run.affinity === referenceAffinity && run.moduleHash === generated[variant].hash));
  const correctnessStable = variants.every((variant) => runs[variant].every((run) =>
    JSON.stringify(run.correctness) === JSON.stringify(runs[variant][0].correctness)));
  const exactEquivalent = correctnessStable && runs.sret.every((run, index) =>
    JSON.stringify(run.correctness) === JSON.stringify(runs.compact[index].correctness));
  const resultStable = variants.every((variant) => runs[variant].every((run) =>
    run.firstResult === runs[variant][0].firstResult &&
    run.warmResults.every((result) => result === run.warmResults[0]) &&
    run.steadyResults.every((result) => result === run.steadyResults[0])));
  const exactTimedResults = resultStable && runs.sret.every((run, index) =>
    run.firstResult === runs.compact[index].firstResult &&
    run.warmResults[0] === runs.compact[index].warmResults[0] &&
    run.steadyResults[0] === runs.compact[index].steadyResults[0]);
  const warmStable = variants.every((variant) => runs[variant].every((run) =>
    spread(run.warmMs) <= MAX_SPREAD));
  const hostValues = hostProbes.flatMap((probe) => [probe.before, probe.after]);
  const hostProbeSpread = spread(hostValues);

  const variantsSummary = Object.fromEntries(variants.map((variant) =>
    [variant, summarizeVariant(generated, runs, variant)]));
  const pairedSteadySpeedup = summary(runs.sret.map((run, index) =>
    median(run.steadyMs) / median(runs.compact[index].steadyMs)));
  const pairedColdDeltaMs = summary(runs.compact.map((run, index) =>
    run.compileMs + run.instantiateMs -
      (runs.sret[index].compileMs + runs.sret[index].instantiateMs)));
  const lowerBound = pairedSteadySpeedup.medianConfidence95?.[0] ?? 0;
  const compactCold = variantsSummary.compact.coldConstructionMs.median;
  const shapePass = variants.every((variant) =>
    staticShapes[variant].staticPass && optimizingShapes[variant].optimizingPass);
  const gate = {
    exactInputs,
    deterministicValidBytes: variants.every((variant) =>
      generated[variant].deterministic && generated[variant].validates),
    exactCorrectnessAndMemory: exactEquivalent,
    exactTimedResults,
    shapePass,
    maximumV8InlineeWireBytes: inliningMaximum,
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
    maxCompactColdConstructionMs: MAX_COLD_MS,
    observedCompactColdConstructionMs: compactCold,
    compactColdConstructionPass: compactCold <= MAX_COLD_MS,
    maxPairedColdDeltaMs: MAX_COLD_MS,
    observedPairedColdDeltaMs: pairedColdDeltaMs.median,
    pairedColdDeltaPass: pairedColdDeltaMs.median <= MAX_COLD_MS,
  };
  gate.admitProductionPrototype = gate.exactInputs &&
    gate.deterministicValidBytes && gate.exactCorrectnessAndMemory &&
    gate.exactTimedResults && gate.shapePass && gate.warmStable && gate.hostStable &&
    gate.steadySpeedupPass && gate.lowerMedianBoundPass &&
    gate.compactColdConstructionPass && gate.pairedColdDeltaPass;

  const report = {
    schema: 1,
    experiment: "R058",
    mechanism: "compact-interpreter-step-outcome-abi",
    productionModified: false,
    methodology:
      "frozen-engine-shape/fresh-process/alternating-paired-order/direct-call/no-inline",
    samples,
    engine: referenceEngine,
    affinity: referenceAffinity,
    timing: {
      correctnessIterations: CORRECTNESS_ITERATIONS,
      firstIterations: FIRST_ITERATIONS,
      warmIterations: WARM_ITERATIONS,
      warmRepetitions: WARM_REPETITIONS,
      steadyIterations: STEADY_ITERATIONS,
      steadySamplesPerProcess: STEADY_SAMPLES,
    },
    generated,
    staticShape: Object.fromEntries(variants.map((variant) => [variant, {
      nopCount: staticShapes[variant].nopCount,
      directStepCalls: staticShapes[variant].directStepCalls,
      pass: staticShapes[variant].staticPass,
    }])),
    optimizingShape: Object.fromEntries(variants.map((variant) => [variant, {
      stepBodySizes: optimizingShapes[variant].stepBodySizes,
      driverBodySizes: optimizingShapes[variant].driverBodySizes,
      stepTurboFan: optimizingShapes[variant].stepTurboFan,
      driverTurboFan: optimizingShapes[variant].driverTurboFan,
      retainedCall: optimizingShapes[variant].retainedCall,
      pass: optimizingShapes[variant].optimizingPass,
    }])),
    correctness: {
      exactEquivalent,
      stable: correctnessStable,
      expected: runs.sret[0].correctness,
    },
    cpuProbe: CPU_PROBE_SPEC,
    hostProbes,
    hostProbeSpread,
    variants: variantsSummary,
    pairedSteadySpeedup,
    pairedColdDeltaMs,
    gate,
  };

  const output = join(root, "target/bench/r058-compact-step-outcome-corpus.json");
  const shapeDirectory = join(root, "target/bench/r058-compact-step-outcome-shape");
  mkdirSync(dirname(output), { recursive: true });
  mkdirSync(shapeDirectory, { recursive: true });
  for (const variant of variants) {
    writeFileSync(
      join(shapeDirectory, `${variant}.objdump.txt`),
      staticShapes[variant].disassembly,
      { flag: "wx" },
    );
    writeFileSync(
      join(shapeDirectory, `${variant}.inlining.log`),
      optimizingShapes[variant].trace,
      { flag: "wx" },
    );
  }
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
  console.log(
    `sret    ${variantsSummary.sret.steadyMInstructionsPerSecond.median.toFixed(3)} Mstep/s`,
  );
  console.log(
    `compact ${variantsSummary.compact.steadyMInstructionsPerSecond.median.toFixed(3)} Mstep/s`,
  );
  console.log(
    `paired speedup ${pairedSteadySpeedup.median.toFixed(3)}x ` +
    `CI=[${pairedSteadySpeedup.medianConfidence95[0].toFixed(3)}, ` +
    `${pairedSteadySpeedup.medianConfidence95[1].toFixed(3)}]`,
  );
  console.log(`admit production prototype: ${gate.admitProductionPrototype}`);
  console.log(`saved ${output}`);
}

await main();
