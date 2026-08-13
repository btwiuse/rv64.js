#!/usr/bin/env node

// Candidate-specific verifier for R072's established five-pair rewrite/v86
// browser guard. The generic analyzer validates every raw leg and runtime
// proof; this final check freezes the R072 identities and 10% product margin.

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const reportArg = process.argv[2];
if (!reportArg) {
  throw new Error("usage: node tests/vs-v86/r072-browser-v86-gate.mjs ANALYSIS.json");
}
const report = JSON.parse(await readFile(resolve(reportArg), "utf8"));
const expectedConfig = { staticSystemT0: true, sampledStaticT0: true };
const expectedWasm =
  "cb7ea81685b3cb9605f6e01b619f3c15005f4ef77ca00c258c20c7a519bb6df6";
const expectedPage =
  "28957e0d5ce381184addb291805ba26a6e64d421a51882c4ae56e0512a82cd3d";
const expectedArchive =
  "159fc55c4337345a685252e384d64be39fc50c743b4478e2b864289ad8bb8690";
const problems = [];

if (report.measurementValid !== true) problems.push("underlying report is invalid");
if (report.method?.pairs !== 5) problems.push(`pairs ${report.method?.pairs}, expected 5`);
if (report.method?.maxSlowdown !== 0.10) {
  problems.push(`maximum slowdown ${report.method?.maxSlowdown}, expected 0.10`);
}
if (JSON.stringify(report.protocol?.jitConfiguration) !== JSON.stringify(expectedConfig)) {
  problems.push(`JIT configuration changed: ${JSON.stringify(report.protocol?.jitConfiguration)}`);
}
if (report.protocol?.expectedMainWasmSha256 !== expectedWasm) {
  problems.push(`main Wasm ${report.protocol?.expectedMainWasmSha256}, expected ${expectedWasm}`);
}
if (report.artifacts?.page?.sha256 !== expectedPage) {
  problems.push(`page ${report.artifacts?.page?.sha256}, expected ${expectedPage}`);
}
if (report.artifacts?.archives?.rv64Jit?.sha256 !== expectedArchive) {
  problems.push(`archive ${report.artifacts?.archives?.rv64Jit?.sha256}, expected ${expectedArchive}`);
}
for (const phase of ["python", "sha256", "shared9p"]) {
  const result = report.phases?.[phase];
  if (!result || result.rv64?.length !== 5 || result.v86?.length !== 5) {
    problems.push(`${phase}: expected five complete pairs`);
    continue;
  }
  if (result.noninferiorAtRequestedMargin !== true) {
    problems.push(`${phase}: failed established 10% browser margin`);
  }
  const upper = result.pairedGeometricMeanBootstrap95?.[1];
  if (!Number.isFinite(upper) || upper > 1.10) {
    problems.push(`${phase}: upper bound ${upper} exceeds 1.10`);
  }
  console.log(
    `${phase}: ${result.pairedGeometricMean.toFixed(3)} ` +
      `CI=[${result.pairedGeometricMeanBootstrap95.map((value) => value.toFixed(3)).join(", ")}]`,
  );
}
if (problems.length) {
  console.error(`R072_BROWSER_V86_GATE_FAIL: ${problems.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("R072_BROWSER_V86_GATE_PASS");
}

