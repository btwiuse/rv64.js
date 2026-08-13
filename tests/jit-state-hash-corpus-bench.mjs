#!/usr/bin/env node

// R084 proof-only opportunity gate. It compares Rust's default RandomState
// with the candidate integer-key hasher in one identical Wasm module. Product
// code is not modified. Measurements use alternating fresh Node/V8 processes.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { median, summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const self = fileURLToPath(import.meta.url);
const source = join(here, "jit-state-hash-corpus.rs");
const outputDir = join(root, "target/bench/r084-hash-opportunity");
const repeatDir = join(root, "target/bench/r084-hash-opportunity-repeat");
const wasmName = "r084_hash_corpus.wasm";
const variants = ["default", "fast"];
const variantIndex = { default: 0, fast: 1 };
const HASH_ROUNDS = 2_097_152;
const MAP_ENTRIES = 4096;
const MAP_ROUNDS = 262_144;
const WARM_REPETITIONS = 4;
const STEADY_SAMPLES = 7;
const REQUIRED_HASH_SPEEDUP = 2.50;

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const elapsedMs = (start) => Number(process.hrtime.bigint() - start) / 1e6;

function compile(directory) {
  mkdirSync(directory, { recursive: true });
  const output = join(directory, wasmName);
  const result = spawnSync("rustc", [
    source,
    "--edition=2021",
    "--crate-name=r084_hash_corpus",
    "--crate-type=cdylib",
    "--target=wasm32-unknown-unknown",
    "-Copt-level=3",
    "-Clto=fat",
    "-Ccodegen-units=1",
    "-Cpanic=abort",
    `-o${output}`,
  ], { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 });
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "R084 corpus compilation failed");
  }
  return output;
}

function worker(variant) {
  const bytes = readFileSync(join(outputDir, wasmName));
  if (!WebAssembly.validate(bytes)) throw new Error("R084 corpus does not validate");
  const module = new WebAssembly.Module(bytes);
  const instance = new WebAssembly.Instance(module);
  const { run_hash: runHash, run_maps: runMaps } = instance.exports;
  if (typeof runHash !== "function" || typeof runMaps !== "function") {
    throw new Error("R084 corpus exports are incomplete");
  }
  const index = variantIndex[variant];
  const correctness = {
    hash: [0, 1, 17, 4096].map((rounds) => runHash(index, rounds).toString()),
    maps: [[1, 0], [1, 17], [64, 4096], [1024, 16384]].map(([entries, rounds]) =>
      runMaps(index, entries, rounds).toString()),
  };

  for (let repetition = 0; repetition < WARM_REPETITIONS; repetition++) {
    runHash(index, HASH_ROUNDS >>> 2);
    runMaps(index, MAP_ENTRIES, MAP_ROUNDS >>> 2);
  }

  const hashMs = [];
  const mapMs = [];
  const hashResults = [];
  const mapResults = [];
  for (let sample = 0; sample < STEADY_SAMPLES; sample++) {
    let start = process.hrtime.bigint();
    hashResults.push(runHash(index, HASH_ROUNDS).toString());
    hashMs.push(elapsedMs(start));
    start = process.hrtime.bigint();
    mapResults.push(runMaps(index, MAP_ENTRIES, MAP_ROUNDS).toString());
    mapMs.push(elapsedMs(start));
  }
  process.stdout.write(JSON.stringify({
    variant,
    engine: { node: process.version, v8: process.versions.v8 },
    affinity: readFileSync("/proc/self/status", "utf8")
      .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
    moduleHash: hash(bytes),
    correctness,
    hashMs,
    mapMs,
    hashResults,
    mapResults,
  }));
}

if (process.argv.includes("--worker")) {
  const variant = process.argv.find((argument) => argument.startsWith("--variant="))
    ?.split("=")[1];
  if (!variants.includes(variant)) throw new Error(`unknown R084 variant ${variant}`);
  worker(variant);
  process.exit(0);
}

const sampleArgument = process.argv.find((argument) => argument.startsWith("--samples="));
const samples = sampleArgument ? Number(sampleArgument.split("=")[1]) : 7;
if (!Number.isInteger(samples) || samples < 3 || samples > 29 || !(samples & 1)) {
  throw new Error("--samples must be an odd integer from 3 through 29");
}

const firstPath = compile(outputDir);
const repeatPath = compile(repeatDir);
const firstBytes = readFileSync(firstPath);
const repeatBytes = readFileSync(repeatPath);
if (!firstBytes.equals(repeatBytes) || !WebAssembly.validate(firstBytes)) {
  throw new Error("R084 corpus is nondeterministic or invalid");
}

const runs = Object.fromEntries(variants.map((variant) => [variant, []]));
for (let sample = 0; sample < samples; sample++) {
  const order = sample % 2 === 0 ? variants : [...variants].reverse();
  for (const variant of order) {
    const child = spawnSync(process.execPath,
      [self, "--worker", `--variant=${variant}`],
      { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 });
    if (child.status !== 0) {
      throw new Error(child.stderr || child.stdout || `${variant} R084 worker failed`);
    }
    runs[variant].push(JSON.parse(child.stdout));
  }
}

const expectedHash = runs.default[0].correctness.hash;
const expectedMaps = runs.default[0].correctness.maps;
// RandomState intentionally chooses a new seed for each call, so raw hashes
// are not semantic outputs and are not expected to match or remain constant.
// The table corpus never observes hash values; its checksum must be identical.
const exactEquivalent = variants.every((variant) =>
  runs[variant].every((run) =>
  JSON.stringify(run.correctness.maps) === JSON.stringify(expectedMaps) &&
  run.mapResults.every((value) => value === runs.default[0].mapResults[0])));
if (!exactEquivalent) throw new Error("R084 variants do not produce equivalent map state");

const summarizeVariant = (variant) => ({
  hashMs: summary(runs[variant].map((run) => median(run.hashMs))),
  mapMs: summary(runs[variant].map((run) => median(run.mapMs))),
  hashMOperationsPerSecond: summary(runs[variant].map((run) =>
    HASH_ROUNDS / median(run.hashMs) / 1000)),
  mapMIterationsPerSecond: summary(runs[variant].map((run) =>
    MAP_ROUNDS / median(run.mapMs) / 1000)),
  rawRuns: runs[variant],
});
const pairedHashSpeedup = summary(runs.default.map((run, index) =>
  median(run.hashMs) / median(runs.fast[index].hashMs)));
const pairedMapSpeedup = summary(runs.default.map((run, index) =>
  median(run.mapMs) / median(runs.fast[index].mapMs)));

const report = {
  schema: 1,
  experiment: "R084",
  mechanism: "seeded-avalanche-hasher-for-integer-key-jit-state",
  productionModified: false,
  methodology: "same-wasm/fresh-process/alternating-paired-order",
  engine: runs.default[0].engine,
  samples,
  source: { path: source.slice(root.length + 1), sha256: hash(readFileSync(source)) },
  wasm: { bytes: firstBytes.length, sha256: hash(firstBytes), deterministic: true },
  corpus: { hashRounds: HASH_ROUNDS, mapEntries: MAP_ENTRIES, mapRounds: MAP_ROUNDS },
  correctness: {
    exactEquivalent,
    rawHashesAreNonsemantic: true,
    exampleDefaultHash: expectedHash,
    expectedMaps,
  },
  variants: Object.fromEntries(variants.map((variant) =>
    [variant, summarizeVariant(variant)])),
  pairedHashSpeedup,
  pairedMapSpeedup,
  gate: {
    requiredHashSpeedup: REQUIRED_HASH_SPEEDUP,
    observedHashSpeedup: pairedHashSpeedup.median,
    hashSpeedupPass: pairedHashSpeedup.median >= REQUIRED_HASH_SPEEDUP,
    admitProductCandidate: exactEquivalent &&
      pairedHashSpeedup.median >= REQUIRED_HASH_SPEEDUP,
  },
};

mkdirSync(outputDir, { recursive: true });
const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
const output = outputArgument?.split("=")[1] ?? join(outputDir, "opportunity.json");
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });

console.log(`engine: Node ${report.engine.node}, V8 ${report.engine.v8}`);
console.log(`corpus: ${report.wasm.bytes} bytes ${report.wasm.sha256}`);
console.log(`hash speedup: ${pairedHashSpeedup.median.toFixed(3)}x`);
console.log(`state-map speedup: ${pairedMapSpeedup.median.toFixed(3)}x`);
console.log(`candidate admitted: ${report.gate.admitProductCandidate}`);
console.log(`report: ${output}`);

if (!report.gate.admitProductCandidate) process.exitCode = 1;
