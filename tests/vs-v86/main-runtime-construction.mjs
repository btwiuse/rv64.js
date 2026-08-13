#!/usr/bin/env node

// Measure the real RV64Debug.create path in fresh processes. File I/O, loader
// import, hashing, and process startup are outside the reported interval.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { constructionDebit } from "./amortized-cold-cost.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const self = fileURLToPath(import.meta.url);
const loaderPath = resolve(root, "web/rv64.js");
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

async function worker(wasmPath) {
  const bytes = readFileSync(wasmPath);
  const loaderBytes = readFileSync(loaderPath);
  const { RV64Debug } = await import(pathToFileURL(loaderPath).href);
  const started = performance.now();
  const vm = await RV64Debug.create(bytes);
  const createMs = performance.now() - started;
  const exports = Object.keys(vm.ex).sort().map((name) => ({
    name,
    kind: vm.ex[name] instanceof WebAssembly.Memory
      ? "memory"
      : vm.ex[name] instanceof WebAssembly.Table
        ? "table"
        : vm.ex[name] instanceof WebAssembly.Global
          ? "global"
          : typeof vm.ex[name],
  }));
  process.stdout.write(JSON.stringify({
    wasmPath: resolve(wasmPath),
    wasmSha256: sha256(bytes),
    wasmBytes: bytes.length,
    loaderSha256: sha256(loaderBytes),
    createMs,
    exports,
    memoryBytes: vm.ex.memory.buffer.byteLength,
    cpus: readFileSync("/proc/self/status", "utf8")
      .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
  }));
}

if (process.argv[2] === "--worker") {
  await worker(process.argv[3]);
  process.exit(0);
}

const [controlPath, candidatePath, outputPath, repsText = "15"] = process.argv.slice(2);
if (!controlPath || !candidatePath || !outputPath) {
  throw new Error(
    "usage: main-runtime-construction.mjs CONTROL.wasm CANDIDATE.wasm OUTPUT.json [REPS]",
  );
}
const reps = Number(repsText);
if (!Number.isSafeInteger(reps) || reps < 7 || reps > 101 || !(reps & 1)) {
  throw new Error("REPS must be an odd integer from 7 through 101");
}

const runs = { control: [], candidate: [] };
const order = [];
for (let rep = 0; rep < reps; rep++) {
  const pairOrder = rep & 1 ? ["candidate", "control"] : ["control", "candidate"];
  order.push(pairOrder);
  for (const side of pairOrder) {
    const path = side === "control" ? controlPath : candidatePath;
    const child = spawnSync(process.execPath, [self, "--worker", path], {
      encoding: "utf8",
      maxBuffer: 8 << 20,
    });
    if (child.status !== 0) {
      throw new Error(child.stderr || child.stdout || `${side} construction worker failed`);
    }
    runs[side].push(JSON.parse(child.stdout));
  }
}

const controlTimes = runs.control.map((run) => run.createMs);
const candidateTimes = runs.candidate.map((run) => run.createMs);
const accounting = constructionDebit(controlTimes, candidateTimes);
const stableWithin = (side, field) => {
  const values = runs[side].map((run) => JSON.stringify(run[field]));
  return new Set(values).size === 1;
};
const problems = [];
for (const side of ["control", "candidate"]) {
  for (const field of ["wasmSha256", "wasmBytes", "loaderSha256", "exports", "memoryBytes", "cpus"]) {
    if (!stableWithin(side, field)) problems.push(`${side} ${field} changed between repetitions`);
  }
}
if (runs.control[0].loaderSha256 !== runs.candidate[0].loaderSha256) {
  problems.push("loader differs between construction legs");
}
if (runs.control[0].cpus !== runs.candidate[0].cpus) {
  problems.push("CPU affinity differs between construction legs");
}
const report = {
  schema: 1,
  methodology: "alternating-fresh-process-real-RV64Debug.create-pairs",
  node: process.version,
  v8: process.versions.v8,
  reps,
  order,
  control: runs.control[0],
  candidate: runs.candidate[0],
  accounting,
  raw: runs,
  measurementValid: problems.length === 0,
  problems,
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(
  `construction control=${accounting.control.median.toFixed(3)} ms ` +
  `candidate=${accounting.candidate.median.toFixed(3)} ms ` +
  `debit=${accounting.debitMs.toFixed(3)} ms valid=${report.measurementValid}`,
);
if (problems.length) process.exitCode = 1;
