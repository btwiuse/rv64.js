#!/usr/bin/env node

// R106's frozen seven-pair fresh-process main-module construction gate.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { summary } from "../statistics.mjs";

const REPS = 7;
const MAX_COLD_ELAPSED_REGRESSION = 1.05;
const CONTROL = "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d";
const CANDIDATE = "6571795ef19c275aa8f9ae3831c9da3eae74aa7e9ea90b550ca664b0cc165165";
const CONTROL_CODE = "b90313ef0c2f85c93ac38a60d434151e2b012deea15fbe05ea60c5702421fc69";
const CANDIDATE_CODE = "a0fca518060729b2b8bbfc2e2924886545de781a9112d98b741584fbb0034f5f";
const DIAGNOSTIC_EXPORTS = new Set([
  "jit_set_integrated_scalar_t0",
  "jit_set_yield_on_pending",
]);
const self = fileURLToPath(import.meta.url);
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function readU32(bytes, cursor) {
  let value = 0;
  let shift = 0;
  while (true) {
    if (cursor.offset >= bytes.length || shift > 28) throw new Error("invalid Wasm u32 LEB");
    const byte = bytes[cursor.offset++];
    value |= (byte & 0x7f) << shift;
    if (!(byte & 0x80)) return value >>> 0;
    shift += 7;
  }
}

function codeIdentity(bytes) {
  const cursor = { offset: 8 };
  while (cursor.offset < bytes.length) {
    const id = bytes[cursor.offset++];
    const size = readU32(bytes, cursor);
    const end = cursor.offset + size;
    if (end > bytes.length) throw new Error("truncated Wasm section");
    if (id === 10) {
      const code = bytes.subarray(cursor.offset, end);
      return { bytes: code.length, sha256: sha256(code) };
    }
    cursor.offset = end;
  }
  throw new Error("Wasm module has no CODE section");
}

function worker(path) {
  const bytes = readFileSync(path);
  if (!WebAssembly.validate(bytes)) throw new Error(`${path}: invalid Wasm`);
  const coldStarted = performance.now();
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
    code: codeIdentity(bytes),
    imports,
    exports: WebAssembly.Module.exports(module),
    compileMs,
    instantiateMs,
    coldElapsedMs: performance.now() - coldStarted,
  }));
}

if (process.argv[2] === "--worker") {
  worker(process.argv[3]);
  process.exit(0);
}

const [controlPath, candidatePath, outputPath] = process.argv.slice(2);
if (!controlPath || !candidatePath || !outputPath) {
  throw new Error("usage: r106-cold-compile.mjs CONTROL.wasm CANDIDATE.wasm OUTPUT.json");
}

const runs = { control: [], candidate: [] };
const order = [];
for (let rep = 0; rep < REPS; rep += 1) {
  const pairOrder = rep % 2 === 0
    ? ["control", "candidate"]
    : ["candidate", "control"];
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
  code: runs[side][0].code,
  imports: runs[side][0].imports,
  exports: runs[side][0].exports,
  compileMs: summary(runs[side].map((run) => run.compileMs)),
  instantiateMs: summary(runs[side].map((run) => run.instantiateMs)),
  coldElapsedMs: summary(runs[side].map((run) => run.coldElapsedMs)),
  raw: runs[side],
});
const control = summarize("control");
const candidate = summarize("candidate");
const coldElapsedRegression = candidate.coldElapsedMs.median /
  control.coldElapsedMs.median;
const pairedColdElapsedRatios = summary(runs.candidate.map((run, index) =>
  run.coldElapsedMs / runs.control[index].coldElapsedMs));
const controlExports = control.exports.map((entry) => JSON.stringify(entry));
const candidateProductExports = candidate.exports
  .filter((entry) => !DIAGNOSTIC_EXPORTS.has(entry.name))
  .map((entry) => JSON.stringify(entry));
const diagnosticExports = candidate.exports
  .filter((entry) => DIAGNOSTIC_EXPORTS.has(entry.name));
const problems = [];
if (control.sha256 !== CONTROL) problems.push("control is not exact d9f baseline");
if (candidate.sha256 !== CANDIDATE) problems.push("candidate artifact changed");
if (control.code.sha256 !== CONTROL_CODE) problems.push("control CODE section changed");
if (candidate.code.sha256 !== CANDIDATE_CODE) problems.push("candidate CODE section changed");
if (JSON.stringify(control.imports) !== JSON.stringify(candidate.imports)) {
  problems.push("candidate imports changed");
}
if (JSON.stringify(controlExports) !== JSON.stringify(candidateProductExports) ||
    diagnosticExports.length !== 2 ||
    diagnosticExports.some((entry) => entry.kind !== "function")) {
  problems.push("candidate exports changed beyond the two diagnostic setters");
}
if (coldElapsedRegression > MAX_COLD_ELAPSED_REGRESSION) {
  problems.push(
    `candidate cold elapsed regression ${coldElapsedRegression} exceeds ` +
      MAX_COLD_ELAPSED_REGRESSION,
  );
}
const report = {
  schema: 1,
  experiment: "R106 balanced scalar/publication pipeline",
  methodology: "seven-alternating-fresh-process-compile-and-instantiate-pairs",
  node: process.version,
  v8: process.versions.v8,
  cpus: readFileSync("/proc/self/status", "utf8")
    .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
  reps: REPS,
  order,
  maximumCandidateColdElapsedRegression: MAX_COLD_ELAPSED_REGRESSION,
  control,
  candidate,
  coldElapsedRegression,
  pairedColdElapsedRatios,
  pass: problems.length === 0,
  problems,
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(`control cold ${control.coldElapsedMs.median.toFixed(3)} ms`);
console.log(`candidate cold ${candidate.coldElapsedMs.median.toFixed(3)} ms`);
console.log(`candidate/control ${coldElapsedRegression.toFixed(3)}x; pass=${report.pass}`);
if (problems.length) process.exitCode = 1;
