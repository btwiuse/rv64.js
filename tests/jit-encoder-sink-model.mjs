#!/usr/bin/env node

// Proof-only model for replacing wasm-encoder's general Instruction enum
// dispatch with its typed InstructionSink API. Each side runs in a fresh V8
// process; main-module compile/instantiate is outside every reported span.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { summary } from "./statistics.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const self = fileURLToPath(import.meta.url);
const crate = resolve(root, "tests/encoder-sink-wasm/Cargo.toml");
const wasm = resolve(
  root,
  "tests/encoder-sink-wasm/target/wasm32-unknown-unknown/release/encoder_sink_wasm.wasm",
);
const GROUPS = 4096;
const ITERATIONS = 16;
const WARM_CALLS = 8;
const STEADY_CALLS = 7;
const PAIRS = 7;

function worker(side) {
  const bytes = readFileSync(wasm);
  const module = new WebAssembly.Module(bytes);
  const instance = new WebAssembly.Instance(module, {});
  const fn = side === "control" ? instance.exports.encode_enum : instance.exports.encode_sink;
  if (typeof fn !== "function") throw new Error(`${side} model export missing`);
  let started = performance.now();
  const firstResult = fn(GROUPS, ITERATIONS);
  const firstMs = performance.now() - started;
  for (let call = 0; call < WARM_CALLS; call++) fn(GROUPS, ITERATIONS);
  const steadyMs = [];
  const steadyResults = [];
  for (let call = 0; call < STEADY_CALLS; call++) {
    started = performance.now();
    steadyResults.push(fn(GROUPS, ITERATIONS).toString());
    steadyMs.push(performance.now() - started);
  }
  const exact = instance.exports.exact_bytes(GROUPS);
  const bodyBytes = instance.exports.body_bytes(GROUPS);
  process.stdout.write(JSON.stringify({
    side,
    wasmSha256: createHash("sha256").update(bytes).digest("hex"),
    wasmBytes: bytes.length,
    bodyBytes,
    exact,
    firstResult: firstResult.toString(),
    firstMs,
    steadyResults,
    steadyMs,
    node: process.version,
    v8: process.versions.v8,
    cpus: readFileSync("/proc/self/status", "utf8")
      .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
  }));
}

if (process.argv[2] === "--worker") {
  worker(process.argv[3]);
  process.exit(0);
}

const output = process.argv.find((argument) => argument.startsWith("--output="))
  ?.slice("--output=".length);
if (!process.argv.includes("--skip-build")) {
  const built = spawnSync(
    "cargo",
    ["build", "--release", "--target", "wasm32-unknown-unknown", "--manifest-path", crate],
    { cwd: root, encoding: "utf8" },
  );
  if (built.status !== 0) throw new Error(built.stderr || built.stdout || "model build failed");
}

const runs = { control: [], candidate: [] };
const order = [];
for (let pair = 0; pair < PAIRS; pair++) {
  const pairOrder = pair & 1 ? ["candidate", "control"] : ["control", "candidate"];
  order.push(pairOrder);
  for (const side of pairOrder) {
    const child = spawnSync(process.execPath, [self, "--worker", side], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 8 << 20,
    });
    if (child.status !== 0) throw new Error(child.stderr || child.stdout || `${side} failed`);
    runs[side].push(JSON.parse(child.stdout));
  }
}

const problems = [];
for (const side of ["control", "candidate"]) {
  for (const run of runs[side]) {
    if (run.exact !== 1) problems.push(`${side}: output bytes differ`);
    if (new Set(run.steadyResults).size !== 1 || run.steadyResults[0] !== run.firstResult) {
      problems.push(`${side}: result fingerprint changed`);
    }
  }
}
for (const field of ["wasmSha256", "wasmBytes", "bodyBytes", "firstResult", "node", "v8", "cpus"]) {
  if (new Set([...runs.control, ...runs.candidate].map((run) => JSON.stringify(run[field]))).size !== 1) {
    problems.push(`${field} differs between legs`);
  }
}
const firstSpeedups = runs.control.map((run, index) =>
  run.firstMs / runs.candidate[index].firstMs);
const steadyControl = runs.control.map((run) => summary(run.steadyMs).median);
const steadyCandidate = runs.candidate.map((run) => summary(run.steadyMs).median);
const steadySpeedups = steadyControl.map((value, index) => value / steadyCandidate[index]);
const report = {
  schema: 1,
  experiment: "typed wasm instruction sink opportunity model",
  methodology: "seven-alternating-fresh-V8-process-pairs/first-and-tiered-steady",
  model: { groups: GROUPS, instructionsPerGroup: 21, iterations: ITERATIONS, warmCalls: WARM_CALLS },
  order,
  artifact: runs.control[0],
  first: {
    controlMs: summary(runs.control.map((run) => run.firstMs)),
    candidateMs: summary(runs.candidate.map((run) => run.firstMs)),
    pairedSpeedup: summary(firstSpeedups),
  },
  steady: {
    controlMs: summary(steadyControl),
    candidateMs: summary(steadyCandidate),
    pairedSpeedup: summary(steadySpeedups),
  },
  raw: runs,
  measurementValid: problems.length === 0,
  problems,
};
if (output) {
  mkdirSync(dirname(resolve(output)), { recursive: true });
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
}
console.log(
  `first ${report.first.pairedSpeedup.median.toFixed(3)}x ` +
  `[${report.first.pairedSpeedup.medianConfidence95.map((v) => v.toFixed(3)).join(",")}]; ` +
  `steady ${report.steady.pairedSpeedup.median.toFixed(3)}x ` +
  `[${report.steady.pairedSpeedup.medianConfidence95.map((v) => v.toFixed(3)).join(",")}]; ` +
  `valid=${report.measurementValid}`,
);
if (problems.length) process.exitCode = 1;
