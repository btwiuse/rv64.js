#!/usr/bin/env node

// Candidate-specific final check for the schema-5 preboot browser report
// frozen in docs/jit-rewrite/R075_PREBOOT_STATIC_T0_PROTOCOL.md. The analyzer
// first enforces every raw-leg guest, artifact, lifecycle, and JIT proof.

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const reportArg = process.argv[2];
if (!reportArg) {
  throw new Error("usage: node tests/vs-v86/r075-browser-gate.mjs ANALYSIS.json");
}
const report = JSON.parse(await readFile(resolve(reportArg), "utf8"));
const expectedWasm =
  "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c";
const expectedPages = {
  control: "a2f43c06f86507c267c36fb3922079d11d44072ab0622526e5577f69448e976f",
  candidate: "7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413",
};
const expectedPageUrls = {
  control:
    "http://127.0.0.1:8765/examples/" +
    "v86-rv64-three-way-r075-control-e0c1971d1ecd4d4f.html",
  candidate:
    "http://127.0.0.1:8765/examples/" +
    "v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html",
};
const expectedArchive =
  "e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c";
const expectedHarness =
  "525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545";
const expectedConfigurations = {
  control: {
    staticSystemT0: false,
    sampledStaticT0: false,
    sampledStaticT0Backoff: false,
  },
  candidate: {
    staticSystemT0: false,
    sampledStaticT0: true,
    sampledStaticT0Backoff: true,
  },
};
const expectedBrowser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const problems = [];
const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);

if (report.schema !== 5) problems.push(`schema ${report.schema}, expected 5`);
if (report.experiment !== "R075 preboot sampled-backoff browser candidate/control") {
  problems.push(`experiment ${report.experiment}`);
}
if (report.measurementValid !== true) problems.push("underlying measurement is invalid");
if (report.gatePassed !== true) problems.push("underlying browser gate failed");
if (report.method?.pairs !== 7 || report.method?.repetitionsPerLeg !== 3) {
  problems.push("expected seven pairs and three repetitions per leg");
}
if (report.method?.maxSlowdown !== 0.03) problems.push("3% phase median rule changed");
if (report.method?.maximumPairedMedianBootstrapUpper !== 1.10) {
  problems.push("1.10 phase confidence-upper rule changed");
}
if (report.method?.minimumShellSpeedup !== 1.10) {
  problems.push("1.10 shell speedup rule changed");
}
if (report.method?.minimumShellMedianBootstrapLower !== 1.00) {
  problems.push("1.00 shell confidence-lower rule changed");
}
if (report.method?.jitConfigurationLifecycle !== "preboot") {
  problems.push("preboot lifecycle changed");
}
if (!same(report.configurations, expectedConfigurations)) problems.push("configurations changed");
if (!same(report.expectedPageUrls, expectedPageUrls)) problems.push("page URLs changed");
if (report.expectedMainWasmSha256 !== expectedWasm) problems.push("main Wasm changed");
if (!same(report.expectedPageSha256, expectedPages)) problems.push("pages changed");
if (report.expectedRv64ArchiveSha256 !== expectedArchive) problems.push("archive changed");
if (report.expectedHarnessSha256 !== expectedHarness) problems.push("WANIX harness changed");
if (!same(report.browser, expectedBrowser)) problems.push("browser identity changed");
if (!Number.isInteger(report.shellStaticModuleIndex) || report.shellStaticModuleIndex < 0) {
  problems.push("prepared static module proof missing");
}
for (const side of ["control", "candidate"]) {
  const artifacts = report.artifacts?.[side];
  if (artifacts?.page?.sha256 !== expectedPages[side]) {
    problems.push(`${side} observed page mismatch`);
  }
  if (artifacts?.archives?.rv64Jit?.sha256 !== expectedArchive) {
    problems.push(`${side} observed archive mismatch`);
  }
}
if (report.problems?.length) {
  problems.push(...report.problems.map((problem) => `underlying: ${problem}`));
}

const shell = report.shell;
if (
  !shell || shell.control?.length !== 7 || shell.candidate?.length !== 7 ||
  shell.pairedRatios?.length !== 7
) {
  problems.push("shell: incomplete seven-pair sample");
} else {
  const lower = shell.pairedMedianBootstrap95?.[0];
  if (!Number.isFinite(shell.pairedMedian) || shell.pairedMedian < 1.10) {
    problems.push(`shell: paired speedup ${shell.pairedMedian} below 1.10`);
  }
  if (!Number.isFinite(lower) || lower < 1.00) {
    problems.push(`shell: confidence lower ${lower} below 1.00`);
  }
  console.log(
    `shell: ${shell.pairedMedian.toFixed(3)}x ` +
      `CI=[${shell.pairedMedianBootstrap95.map((value) => value.toFixed(3)).join(", ")}]`,
  );
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
  console.error(`R075_BROWSER_GATE_FAIL: ${problems.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("R075_BROWSER_GATE_PASS");
}
