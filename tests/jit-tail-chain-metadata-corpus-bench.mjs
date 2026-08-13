#!/usr/bin/env node

// R097 proof-only opportunity screen. It measures the exact successful
// tail-transfer dispatch predicate with production's repeated metadata loads
// versus caching the first generation/index loads in dead i32 locals.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { median, summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const self = fileURLToPath(import.meta.url);
const outputDir = join(root, "target/bench/r097-tail-chain-metadata-opportunity/corpus");
const repeatDir = join(root, "target/bench/r097-tail-chain-metadata-opportunity/repeat");
const variants = ["reload", "cached"];
const engines = ["liftoff", "default"];
const STEADY_ITERATIONS = 8_388_608;
const STEADY_SAMPLES = 7;
const WARM_ITERATIONS = 1_048_576;
const WARM_REPETITIONS = 3;
const PC = 0x4000n;
const DISPATCH_BASE = 0x1000;
const DISPATCH_MASK = 255n;
const MAP_GEN_ADDR = 0x3000;
const BARRIER_ADDR = 0x3010;
const MAP_GEN = 7;
const SB_IDX_BIT = 1 << 30;
const COMPILE_HOPS = 8_200_000;
const COMPILE_BASELINE_MS = 948.163;

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const elapsedMs = (started) => Number(process.hrtime.bigint() - started) / 1e6;

function emitCorpus(directory) {
  const result = spawnSync("cargo", [
    "run", "--release", "-q", "-p", "rv64-dbt", "--example",
    "emit_tail_chain_metadata_corpus", "--", directory,
  ], { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 });
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "R097 corpus generation failed");
  }
}

function instantiateCorpus() {
  const corpusBytes = readFileSync(join(outputDir, "corpus.wasm"));
  const barrierBytes = readFileSync(join(outputDir, "barrier.wasm"));
  if (!WebAssembly.validate(corpusBytes) || !WebAssembly.validate(barrierBytes)) {
    throw new Error("R097 corpus does not validate");
  }
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 1 });
  let started = process.hrtime.bigint();
  const barrierModule = new WebAssembly.Module(barrierBytes);
  const corpusModule = new WebAssembly.Module(corpusBytes);
  const compileMs = elapsedMs(started);
  started = process.hrtime.bigint();
  const barrier = new WebAssembly.Instance(barrierModule, { env: { memory } }).exports.barrier;
  const instance = new WebAssembly.Instance(corpusModule, { env: { memory, barrier } });
  const instantiateMs = elapsedMs(started);
  return { memory, instance, compileMs, instantiateMs, corpusBytes, barrierBytes };
}

function lineAddress(pc = PC) {
  return DISPATCH_BASE + Number((pc >> 1n) & DISPATCH_MASK) * 16;
}

function reset(memory, { tag = PC, idx = 123, gen = MAP_GEN, liveGen = MAP_GEN } = {}) {
  const view = new DataView(memory.buffer);
  const line = lineAddress(PC);
  view.setBigUint64(line, BigInt.asUintN(64, tag), true);
  view.setInt32(line + 8, idx, true);
  view.setUint32(line + 12, gen >>> 0, true);
  view.setUint32(MAP_GEN_ADDR, liveGen >>> 0, true);
  view.setBigUint64(BARRIER_ADDR, 0n, true);
}

function callChecked(memory, run, iterations, state) {
  reset(memory, state);
  const result = run(iterations, PC);
  const barriers = new DataView(memory.buffer).getBigUint64(BARRIER_ADDR, true);
  if (barriers !== BigInt(iterations)) {
    throw new Error(`barrier count ${barriers} != ${iterations}`);
  }
  return result;
}

function worker(variant, engine) {
  const { memory, instance, compileMs, instantiateMs, corpusBytes, barrierBytes } =
    instantiateCorpus();
  const selected = instance.exports[variant];
  const other = instance.exports[variant === "reload" ? "cached" : "reload"];
  if (typeof selected !== "function" || typeof other !== "function") {
    throw new Error("R097 exports are incomplete");
  }
  const cases = [
    ["hit", {}, 123n * 17n],
    ["pc-miss", { tag: PC + 4n }, 0n],
    ["generation-miss", { gen: MAP_GEN + 1 }, 0n],
    ["unverified", { gen: 0xffff_ffff, liveGen: 0xffff_ffff }, 0n],
    ["blacklisted", { idx: -1 }, 0n],
    ["tagged-index", { idx: SB_IDX_BIT | 123 }, 123n * 17n],
  ];
  const correctness = Object.fromEntries(cases.map(([name, state, expected]) => {
    const selectedResult = callChecked(memory, selected, 17, state);
    const otherResult = callChecked(memory, other, 17, state);
    if (selectedResult !== expected || otherResult !== expected) {
      throw new Error(`${name}: ${selectedResult}/${otherResult} != ${expected}`);
    }
    return [name, selectedResult.toString()];
  }));

  const warmMs = [];
  for (let repetition = 0; repetition < WARM_REPETITIONS; repetition++) {
    reset(memory);
    const started = process.hrtime.bigint();
    const result = selected(WARM_ITERATIONS, PC);
    warmMs.push(elapsedMs(started));
    if (result !== 123n * BigInt(WARM_ITERATIONS)) throw new Error("warm checksum mismatch");
  }
  const steadyMs = [];
  const steadyResults = [];
  for (let sample = 0; sample < STEADY_SAMPLES; sample++) {
    reset(memory);
    const started = process.hrtime.bigint();
    steadyResults.push(selected(STEADY_ITERATIONS, PC).toString());
    steadyMs.push(elapsedMs(started));
  }
  process.stdout.write(JSON.stringify({
    variant,
    engine,
    runtime: { node: process.version, v8: process.versions.v8 },
    affinity: readFileSync("/proc/self/status", "utf8")
      .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
    corpusHash: hash(corpusBytes),
    barrierHash: hash(barrierBytes),
    compileMs,
    instantiateMs,
    correctness,
    warmMs,
    steadyMs,
    steadyResults,
  }));
}

if (process.argv.includes("--worker")) {
  const variant = process.argv.find((arg) => arg.startsWith("--variant="))?.split("=")[1];
  const engine = process.argv.find((arg) => arg.startsWith("--engine="))?.split("=")[1];
  if (!variants.includes(variant) || !engines.includes(engine)) {
    throw new Error("invalid R097 worker variant or engine");
  }
  worker(variant, engine);
  process.exit(0);
}

const sampleArg = process.argv.find((arg) => arg.startsWith("--samples="));
const samples = sampleArg ? Number(sampleArg.split("=")[1]) : 7;
if (!Number.isInteger(samples) || samples < 3 || samples > 29 || !(samples & 1)) {
  throw new Error("--samples must be an odd integer from 3 through 29");
}

emitCorpus(outputDir);
emitCorpus(repeatDir);
const artifacts = Object.fromEntries(["corpus.wasm", "barrier.wasm"].map((name) => {
  const first = readFileSync(join(outputDir, name));
  const repeat = readFileSync(join(repeatDir, name));
  return [name, {
    bytes: first.length,
    sha256: hash(first),
    deterministic: first.equals(repeat),
    validates: WebAssembly.validate(first),
  }];
}));
if (Object.values(artifacts).some((item) => !item.deterministic || !item.validates)) {
  throw new Error("R097 corpus is nondeterministic or invalid");
}

const runs = Object.fromEntries(engines.map((engine) => [engine,
  Object.fromEntries(variants.map((variant) => [variant, []]))]));
for (const engine of engines) {
  for (let sample = 0; sample < samples; sample++) {
    const order = sample % 2 === 0 ? variants : [...variants].reverse();
    for (const variant of order) {
      const args = [self, "--worker", `--variant=${variant}`, `--engine=${engine}`];
      if (engine === "liftoff") args.unshift("--liftoff-only");
      const child = spawnSync(process.execPath, args, {
        cwd: root,
        encoding: "utf8",
        maxBuffer: 8 << 20,
      });
      if (child.status !== 0) {
        throw new Error(child.stderr || child.stdout || `${engine}/${variant} worker failed`);
      }
      runs[engine][variant].push(JSON.parse(child.stdout));
    }
  }
}

const expectedCorrectness = JSON.stringify(runs.default.reload[0].correctness);
const expectedResult = (123n * BigInt(STEADY_ITERATIONS)).toString();
const exactEquivalent = engines.every((engine) => variants.every((variant) =>
  runs[engine][variant].every((run) =>
    JSON.stringify(run.correctness) === expectedCorrectness &&
    run.steadyResults.every((result) => result === expectedResult))));
if (!exactEquivalent) throw new Error("R097 variants are not exactly equivalent");

const engineReports = Object.fromEntries(engines.map((engine) => {
  const perVariant = Object.fromEntries(variants.map((variant) => [variant, {
    steadyMs: summary(runs[engine][variant].map((run) => median(run.steadyMs))),
    millionProofsPerSecond: summary(runs[engine][variant].map((run) =>
      STEADY_ITERATIONS / median(run.steadyMs) / 1000)),
    rawRuns: runs[engine][variant],
  }]));
  const pairedSpeedup = summary(runs[engine].reload.map((run, index) =>
    median(run.steadyMs) / median(runs[engine].cached[index].steadyMs)));
  const savedNsPerHop =
    (perVariant.reload.steadyMs.median - perVariant.cached.steadyMs.median) * 1e6 /
    STEADY_ITERATIONS;
  return [engine, {
    variants: perVariant,
    pairedSpeedup,
    savedNsPerHop,
    projectedCompileSavedMs: savedNsPerHop * COMPILE_HOPS / 1e6,
    projectedCompilePercent: savedNsPerHop * COMPILE_HOPS / 1e6 /
      COMPILE_BASELINE_MS * 100,
  }];
}));

const liftoff = engineReports.liftoff.pairedSpeedup;
const optimized = engineReports.default.pairedSpeedup;
const gate = {
  exactEquivalent,
  liftoffPositiveMedian: liftoff.median > 1,
  liftoffNonRegressingLowerBound: liftoff.medianConfidence95[0] >= 1,
  defaultPositiveMedian: optimized.median > 1,
  defaultNonInferiority: optimized.medianConfidence95[0] >= 0.98,
};
gate.admitProductPrototype = Object.values(gate).every(Boolean);

const report = {
  schema: 1,
  experiment: "R097",
  mechanism: "reuse-tail-chain-dispatch-generation-and-index-loads",
  productionModified: false,
  methodology: "same-wasm/fresh-process/alternating-pairs/cross-instance-memory-barrier",
  samples,
  corpus: {
    steadyIterations: STEADY_ITERATIONS,
    steadySamplesPerProcess: STEADY_SAMPLES,
    warmIterations: WARM_ITERATIONS,
    warmRepetitions: WARM_REPETITIONS,
    compileHopEstimate: COMPILE_HOPS,
    compileBaselineMs: COMPILE_BASELINE_MS,
  },
  artifacts,
  correctness: { exactEquivalent, cases: runs.default.reload[0].correctness },
  engines: engineReports,
  gate,
};
const outputArg = process.argv.find((arg) => arg.startsWith("--output="));
const output = outputArg?.split("=")[1] ??
  join(root, "target/bench/r097-tail-chain-metadata-opportunity/opportunity.json");
mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
for (const engine of engines) {
  const result = engineReports[engine];
  console.log(
    `${engine}: ${result.pairedSpeedup.median.toFixed(3)}x ` +
      `CI=[${result.pairedSpeedup.medianConfidence95.map((v) => v.toFixed(3)).join(", ")}] ` +
      `saved=${result.savedNsPerHop.toFixed(3)} ns/hop ` +
      `projection=${result.projectedCompilePercent.toFixed(2)}%`,
  );
}
console.log(`candidate admitted: ${gate.admitProductPrototype}`);
console.log(`report: ${output}`);
if (!gate.admitProductPrototype) process.exitCode = 1;
