#!/usr/bin/env node

// Validate the deterministic R114 real-region corpus and its ABI/operator
// relationship. Timings are deliberately absent.

import { createHash } from "node:crypto";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const evidence = join(root, "target/bench/r114-lazy-internal-pc");
const corpus = resolve(process.argv[2] ?? join(evidence, "real-region"));
const censusPath = resolve(process.argv[3] ?? join(evidence, "operator-census.tsv"));
const output = resolve(process.argv[4] ?? join(evidence, "generated-shape.json"));
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

const manifestBytes = await readFile(join(corpus, "manifest.tsv"));
const manifestLines = manifestBytes.toString().trimEnd().split("\n");
const manifestHeader = manifestLines.shift().split("\t");
const manifest = manifestLines.map((line) =>
  Object.fromEntries(manifestHeader.map((field, index) => [field, line.split("\t")[index]]))
);
const byIdMode = new Map(manifest.map((row) => [`${row.id}:${row.mode}`, row]));
const controlNames = (await readdir(corpus))
  .filter((name) => name.endsWith("-structured.wasm"))
  .sort();
const problems = [];
const modules = [];

for (const controlName of controlNames) {
  const id = controlName.slice(0, -"-structured.wasm".length);
  const candidateName = `${id}-structured-lazy-pc.wasm`;
  const controlRow = byIdMode.get(`${id}:structured`);
  const candidateRow = byIdMode.get(`${id}:structured-lazy-pc`);
  if (!controlRow || !candidateRow) {
    problems.push(`${id}: missing manifest side`);
    continue;
  }
  for (const field of manifestHeader) {
    if (["mode", "wasm", "bytes"].includes(field)) continue;
    if (controlRow[field] !== candidateRow[field]) {
      problems.push(`${id}: manifest ${field} differs`);
    }
  }
  const [control, candidate] = await Promise.all([
    readFile(join(corpus, controlName)),
    readFile(join(corpus, candidateName)),
  ]);
  let controlModule;
  let candidateModule;
  try {
    controlModule = new WebAssembly.Module(control);
    candidateModule = new WebAssembly.Module(candidate);
  } catch (error) {
    problems.push(`${id}: module validation failed: ${error.message}`);
    continue;
  }
  const controlImports = WebAssembly.Module.imports(controlModule);
  const candidateImports = WebAssembly.Module.imports(candidateModule);
  const controlExports = WebAssembly.Module.exports(controlModule);
  const candidateExports = WebAssembly.Module.exports(candidateModule);
  if (JSON.stringify(controlImports) !== JSON.stringify(candidateImports)) {
    problems.push(`${id}: imports differ`);
  }
  if (JSON.stringify(controlExports) !== JSON.stringify(candidateExports)) {
    problems.push(`${id}: exports differ`);
  }
  modules.push({
    id,
    control: { bytes: control.length, sha256: sha256(control) },
    candidate: { bytes: candidate.length, sha256: sha256(candidate) },
    deltaBytes: candidate.length - control.length,
    importsExact: JSON.stringify(controlImports) === JSON.stringify(candidateImports),
    exportsExact: JSON.stringify(controlExports) === JSON.stringify(candidateExports),
  });
}

const censusBytes = await readFile(censusPath);
const censusLines = censusBytes.toString().trimEnd().split("\n");
const censusHeader = censusLines.shift().split("\t");
const totals = Object.fromEntries(
  censusLines
    .filter((line) => line.startsWith("TOTAL\t"))
    .map((line) => {
      const values = line.split("\t");
      return [values[1], Object.fromEntries(censusHeader.map((field, index) => [field, values[index]]))];
    }),
);
const numeric = (side, field) => Number(totals[side]?.[field]);
const safetyConversions = numeric("control", "br_if") - numeric("candidate", "br_if");
const operatorProof = {
  safetyConversions,
  ifDelta: numeric("candidate", "if") - numeric("control", "if"),
  brDelta: numeric("candidate", "br") - numeric("control", "br"),
  endDelta: numeric("candidate", "end") - numeric("control", "end"),
  localGetDelta: numeric("candidate", "local_get") - numeric("control", "local_get"),
  localSetDelta: numeric("candidate", "local_set") - numeric("control", "local_set"),
  localTeeDelta: numeric("candidate", "local_tee") - numeric("control", "local_tee"),
  callDelta: numeric("candidate", "call") - numeric("control", "call"),
  functionDelta: numeric("candidate", "functions") - numeric("control", "functions"),
};
if (
  safetyConversions <= 0 ||
  operatorProof.ifDelta !== safetyConversions ||
  operatorProof.brDelta !== safetyConversions ||
  operatorProof.endDelta !== safetyConversions
) {
  problems.push("safety br_if -> if/br/end relationship is not exact");
}
if (operatorProof.localGetDelta !== operatorProof.localSetDelta) {
  problems.push("deferred PC local.get/local.set deltas differ");
}
if (operatorProof.localTeeDelta !== 0 || operatorProof.callDelta !== 0 || operatorProof.functionDelta !== 0) {
  problems.push("candidate changed tees, calls, or function population");
}
if (modules.length !== 56) problems.push(`expected 56 pairs, found ${modules.length}`);

const shapeBytes = await readFile(join(corpus, "member-shapes.tsv"));
const report = {
  schema: 1,
  experiment: "R114 lazy internal-PC deterministic generated shape",
  timingEligible: false,
  corpus: {
    path: corpus,
    manifestSha256: sha256(manifestBytes),
    memberShapesSha256: sha256(shapeBytes),
    regionCount: modules.length,
    manifestRows: manifest.length,
  },
  operatorCensus: {
    path: censusPath,
    sha256: sha256(censusBytes),
    totals,
    proof: operatorProof,
  },
  aggregate: {
    controlBytes: modules.reduce((sum, module) => sum + module.control.bytes, 0),
    candidateBytes: modules.reduce((sum, module) => sum + module.candidate.bytes, 0),
  },
  abiExact: modules.every((module) => module.importsExact && module.exportsExact),
  pass: problems.length === 0,
  problems,
  modules,
};
await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(JSON.stringify({
  output,
  sha256: sha256(await readFile(output)),
  pass: report.pass,
  pairs: modules.length,
  aggregate: report.aggregate,
  operatorProof,
  problems,
}));
if (problems.length) process.exitCode = 1;
