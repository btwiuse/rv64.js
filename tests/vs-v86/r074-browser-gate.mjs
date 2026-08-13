#!/usr/bin/env node

// Candidate-specific final check for the schema-4 browser report frozen in
// docs/jit-rewrite/R074_SHORT_SAMPLE_BACKOFF_PROTOCOL.md. Raw-leg semantic,
// guest, artifact, and lifecycle proofs are enforced by the analyzer first.

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const reportArg = process.argv[2];
if (!reportArg) {
  throw new Error("usage: node tests/vs-v86/r074-browser-gate.mjs ANALYSIS.json");
}
const report = JSON.parse(await readFile(resolve(reportArg), "utf8"));
const expectedWasm =
  "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c";
const expectedPage =
  "bdc4827f2a9b86eee1ce4443a9914eae4ef8e5c4ff8329b81973343feccb1a64";
const expectedArchive =
  "917ddcad15a15fa6560c480b9b19ccc2d39ec52ceed65030c94c79f0805df2a9";
const expectedBrowser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const expectedHarness =
  "c3f32eee15012ecc53da541bb3e3b1bda798ae1983d2b3a1e1bcd90dcb4e7495";
const problems = [];

if (report.schema !== 4) problems.push(`schema ${report.schema}, expected 4`);
if (report.experiment !== "R074 short-sample backoff browser candidate/control") {
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
if (report.expectedHarnessSha256 !== expectedHarness) problems.push("WANIX harness changed");
if (JSON.stringify(report.browser) !== JSON.stringify(expectedBrowser)) {
  problems.push("browser identity changed");
}
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
  console.error(`R074_BROWSER_GATE_FAIL: ${problems.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("R074_BROWSER_GATE_PASS");
}
