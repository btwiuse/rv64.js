#!/usr/bin/env node

// Seven alternating fresh-process cold compile/instantiate pairs for R091.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { summary } from "../statistics.mjs";

const REPS = 7;
const MAX_COMPILE_REGRESSION = 1.05;
const CONTROL = "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010";
const CANDIDATE = "a8f14136e7d217f4e71aec2c52020f749c476ee2531268f0bab7adfff2e42c75";
const self = fileURLToPath(import.meta.url);
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function worker(path) {
  const bytes = readFileSync(path);
  if (!WebAssembly.validate(bytes)) throw new Error(`${path}: invalid Wasm`);
  let started = performance.now();
  const module = new WebAssembly.Module(bytes);
  const compileMs = performance.now() - started;
  const imports = WebAssembly.Module.imports(module);
  const importsObject = {};
  for (const entry of imports) {
    if (entry.kind !== "function") throw new Error(`${path}: unexpected ${entry.kind} import`);
    (importsObject[entry.module] ??= {})[entry.name] = () => 0;
  }
  started = performance.now();
  new WebAssembly.Instance(module, importsObject);
  const instantiateMs = performance.now() - started;
  process.stdout.write(JSON.stringify({
    path,
    sha256: sha256(bytes),
    bytes: bytes.length,
    imports,
    exports: WebAssembly.Module.exports(module),
    compileMs,
    instantiateMs,
  }));
}

if (process.argv[2] === "--worker") {
  worker(process.argv[3]);
  process.exit(0);
}

const [controlPath, candidatePath, outputPath] = process.argv.slice(2);
if (!controlPath || !candidatePath || !outputPath) {
  throw new Error("usage: r091-cold-compile.mjs CONTROL.wasm CANDIDATE.wasm OUTPUT.json");
}

const runs = { control: [], candidate: [] };
const order = [];
for (let rep = 0; rep < REPS; rep++) {
  const pairOrder = rep % 2 === 0 ? ["control", "candidate"] : ["candidate", "control"];
  order.push(pairOrder);
  for (const side of pairOrder) {
    const path = side === "control" ? controlPath : candidatePath;
    const child = spawnSync(process.execPath, [self, "--worker", path], {
      encoding: "utf8",
      maxBuffer: 8 << 20,
    });
    if (child.status !== 0) {
      throw new Error(child.stderr || child.stdout || `${side} cold worker failed`);
    }
    runs[side].push(JSON.parse(child.stdout));
  }
}

const summarize = (side) => ({
  path: runs[side][0].path,
  sha256: runs[side][0].sha256,
  bytes: runs[side][0].bytes,
  imports: runs[side][0].imports,
  exports: runs[side][0].exports,
  compileMs: summary(runs[side].map((run) => run.compileMs)),
  instantiateMs: summary(runs[side].map((run) => run.instantiateMs)),
  raw: runs[side],
});
const control = summarize("control");
const candidate = summarize("candidate");
const compileRegression = candidate.compileMs.median / control.compileMs.median;
const pairedCompileRatios = summary(runs.candidate.map((run, index) =>
  run.compileMs / runs.control[index].compileMs));
const problems = [];
if (control.sha256 !== CONTROL) problems.push("control is not exact R085");
if (candidate.sha256 !== CANDIDATE) problems.push("candidate artifact changed");
if (JSON.stringify(control.imports) !== JSON.stringify(candidate.imports) ||
    JSON.stringify(control.exports) !== JSON.stringify(candidate.exports)) {
  problems.push("candidate imports or exports changed");
}
if (compileRegression > MAX_COMPILE_REGRESSION) {
  problems.push(`candidate compile regression ${compileRegression} exceeds ${MAX_COMPILE_REGRESSION}`);
}
const report = {
  schema: 1,
  experiment: "R091",
  methodology: "seven-alternating-fresh-process-cold-main-module-pairs",
  node: process.version,
  v8: process.versions.v8,
  cpus: readFileSync("/proc/self/status", "utf8")
    .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
  reps: REPS,
  order,
  maximumCandidateCompileRegression: MAX_COMPILE_REGRESSION,
  control,
  candidate,
  compileRegression,
  pairedCompileRatios,
  pass: problems.length === 0,
  problems,
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(`control compile ${control.compileMs.median.toFixed(3)} ms`);
console.log(`candidate compile ${candidate.compileMs.median.toFixed(3)} ms`);
console.log(`candidate/control ${compileRegression.toFixed(3)}x; pass=${report.pass}`);
if (problems.length) process.exitCode = 1;
