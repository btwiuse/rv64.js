#!/usr/bin/env node

// R100's frozen seven-pair fresh-process main-module construction gate.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { summary } from "../statistics.mjs";

const REPS = 7;
const MAX_COLD_ELAPSED_REGRESSION = 1.05;
const CONTROL = "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d";
const CANDIDATE = "c36da489ebe3e2f15d960a1ad393b808e9ff285dc099d4988c745e0e81065b32";
const self = fileURLToPath(import.meta.url);
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function readU32(bytes, start) {
  let value = 0;
  let shift = 0;
  let offset = start;
  for (;;) {
    if (offset >= bytes.length || shift > 28) throw new Error("invalid Wasm u32 LEB");
    const byte = bytes[offset++];
    value |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) return { value: value >>> 0, offset };
    shift += 7;
  }
}

function codeIdentity(bytes) {
  if (bytes.length < 8 || bytes.subarray(0, 4).toString("hex") !== "0061736d") {
    throw new Error("not a Wasm binary");
  }
  let offset = 8;
  while (offset < bytes.length) {
    const id = bytes[offset++];
    const size = readU32(bytes, offset);
    const payloadStart = size.offset;
    const payloadEnd = payloadStart + size.value;
    if (payloadEnd > bytes.length) throw new Error("truncated Wasm section");
    if (id === 10) {
      const payload = bytes.subarray(payloadStart, payloadEnd);
      return { bytes: payload.length, sha256: sha256(payload) };
    }
    offset = payloadEnd;
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
  const coldElapsedMs = performance.now() - coldStarted;
  process.stdout.write(JSON.stringify({
    path,
    sha256: sha256(bytes),
    bytes: bytes.length,
    code: codeIdentity(bytes),
    imports,
    exports: WebAssembly.Module.exports(module),
    compileMs,
    instantiateMs,
    coldElapsedMs,
  }));
}

if (process.argv[2] === "--worker") {
  worker(process.argv[3]);
  process.exit(0);
}

const [controlPath, candidatePath, outputPath] = process.argv.slice(2);
if (!controlPath || !candidatePath || !outputPath) {
  throw new Error("usage: r100-cold-compile.mjs CONTROL.wasm CANDIDATE.wasm OUTPUT.json");
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
const coldElapsedRegression = candidate.coldElapsedMs.median / control.coldElapsedMs.median;
const pairedColdElapsedRatios = summary(runs.candidate.map((run, index) =>
  run.coldElapsedMs / runs.control[index].coldElapsedMs));
const problems = [];
if (control.sha256 !== CONTROL) problems.push("control is not exact executable-R085 baseline");
if (candidate.sha256 !== CANDIDATE) problems.push("candidate artifact changed");
if (control.code.sha256 === candidate.code.sha256) {
  problems.push("candidate CODE section is not executable-distinct from control");
}
if (JSON.stringify(control.imports) !== JSON.stringify(candidate.imports) ||
    JSON.stringify(control.exports) !== JSON.stringify(candidate.exports)) {
  problems.push("candidate imports or exports changed");
}
if (coldElapsedRegression > MAX_COLD_ELAPSED_REGRESSION) {
  problems.push(
    `candidate cold elapsed regression ${coldElapsedRegression} exceeds ` +
      MAX_COLD_ELAPSED_REGRESSION,
  );
}
const report = {
  schema: 1,
  experiment: "R100 interleaved fused-TLB",
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
  executableCodeDistinct: control.code.sha256 !== candidate.code.sha256,
  pass: problems.length === 0,
  problems,
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(`control cold ${control.coldElapsedMs.median.toFixed(3)} ms`);
console.log(`candidate cold ${candidate.coldElapsedMs.median.toFixed(3)} ms`);
console.log(`candidate/control ${coldElapsedRegression.toFixed(3)}x; pass=${report.pass}`);
if (problems.length) process.exitCode = 1;
