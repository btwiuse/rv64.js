#!/usr/bin/env node

// Candidate-specific final check for the schema-3 browser report frozen in
// docs/jit-rewrite/R073_SAMPLED_ONLY_STATIC_T0_PROTOCOL.md. Raw-leg semantic
// and lifecycle proofs are enforced by analyze-wanix-r073-pairs.mjs first.

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const reportArg = process.argv[2];
if (!reportArg) {
  throw new Error("usage: node tests/vs-v86/r073-browser-gate.mjs ANALYSIS.json");
}
const report = JSON.parse(await readFile(resolve(reportArg), "utf8"));
const expectedWasm =
  "cb7ea81685b3cb9605f6e01b619f3c15005f4ef77ca00c258c20c7a519bb6df6";
const expectedPage =
  "1c70b211272fd9a843bfe52aefe804322d7260a144df7195a34363ad9f259aee";
const expectedArchive =
  "2b52e552d00929fa4c525c5b1aabc7abbce74d7d3ffe571a0e28d7d3b1cf199e";
const problems = [];

if (report.schema !== 3) problems.push(`schema ${report.schema}, expected 3`);
if (report.experiment !== "R073 sampled-only browser candidate/control") {
  problems.push(`experiment ${report.experiment}`);
}
if (report.measurementValid !== true) problems.push("underlying measurement is invalid");
if (report.gatePassed !== true) problems.push("underlying browser gate failed");
if (report.method?.pairs !== 7 || report.method?.repetitionsPerLeg !== 3) {
  problems.push("expected seven pairs and three repetitions per leg");
}
if (report.method?.maxSlowdown !== 0.03) problems.push("3% median rule changed");
if (report.method?.maximumPairedMedianBootstrapUpper !== 1.10) {
  problems.push("1.10 confidence-upper rule changed");
}
if (report.expectedMainWasmSha256 !== expectedWasm) problems.push("main Wasm changed");
if (report.expectedPageSha256 !== expectedPage) problems.push("page changed");
if (report.expectedRv64ArchiveSha256 !== expectedArchive) problems.push("archive changed");
if (report.artifacts?.page?.sha256 !== expectedPage) problems.push("observed page mismatch");
if (report.artifacts?.archives?.rv64Jit?.sha256 !== expectedArchive) {
  problems.push("observed archive mismatch");
}
if (report.problems?.length) {
  problems.push(...report.problems.map((problem) => `underlying: ${problem}`));
}

for (const phase of ["python", "sha256", "shared9p"]) {
  const result = report.phases?.[phase];
  if (
    !result ||
    result.controlRaw?.length !== 7 ||
    result.candidateRaw?.length !== 7 ||
    result.controlRaw.some((values) => values.length !== 3) ||
    result.candidateRaw.some((values) => values.length !== 3) ||
    result.pairedRatios?.length !== 7
  ) {
    problems.push(`${phase}: incomplete seven-by-three sample`);
    continue;
  }
  const upper = result.pairedMedianBootstrap95?.[1];
  if (!Number.isFinite(result.pairedMedian) || result.pairedMedian > 1.03) {
    problems.push(`${phase}: paired median ${result.pairedMedian} exceeds 1.03`);
  }
  if (!Number.isFinite(upper) || upper > 1.10) {
    problems.push(`${phase}: confidence upper ${upper} exceeds 1.10`);
  }
  console.log(
    `${phase}: ${result.pairedMedian.toFixed(3)}x ` +
      `CI=[${result.pairedMedianBootstrap95.map((value) => value.toFixed(3)).join(", ")}]`,
  );
}

if (problems.length) {
  console.error(`R073_BROWSER_GATE_FAIL: ${problems.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("R073_BROWSER_GATE_PASS");
}

