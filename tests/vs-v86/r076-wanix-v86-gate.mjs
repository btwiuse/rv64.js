#!/usr/bin/env node

// Final identity and performance verifier for R076's fixed seven-pair,
// three-repetition candidate-versus-copy/v86 WANIX guard.

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const reportArg = process.argv[2];
if (!reportArg || process.argv.length !== 3) {
  throw new Error("usage: node tests/vs-v86/r076-wanix-v86-gate.mjs ANALYSIS.json");
}
const report = JSON.parse(await readFile(resolve(reportArg), "utf8"));
const expectedBrowser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const expectedConfiguration = {
  staticSystemT0: false,
  sampledStaticT0: true,
  sampledStaticT0Backoff: true,
};
const expectedArtifacts = {
  page: "7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413",
  rv64Jit: "e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c",
  rv64Root: "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb",
  v86: "7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771",
  x86Root: "09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320",
};
const expectedTools = {
  runner: "8d37e7b20186253a0b7e71e5b7c28f3d8ee3b34a49a7eb4374553c5b80ee4e80",
  harness: "525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545",
};
const problems = [];

if (report.schema !== 3) problems.push(`schema ${report.schema}, expected 3`);
if (report.experiment !== "R076 preboot sampled-backoff candidate versus copy/v86") {
  problems.push(`experiment changed: ${report.experiment}`);
}
if (report.measurementValid !== true) problems.push("underlying measurement is invalid");
if (report.gatePassed !== true) problems.push("underlying gate did not pass");
if (report.method?.pairs !== 7) problems.push(`pairs ${report.method?.pairs}, expected 7`);
if (report.method?.repetitionsPerLeg !== 3) {
  problems.push(`repetitions ${report.method?.repetitionsPerLeg}, expected 3`);
}
if (report.method?.maxSlowdown !== 0.10) {
  problems.push(`margin ${report.method?.maxSlowdown}, expected 0.10`);
}
if (report.method?.maximumPairedMedianBootstrapUpper !== 1.10) {
  problems.push(
    `confidence limit ${report.method?.maximumPairedMedianBootstrapUpper}, expected 1.10`,
  );
}
if (report.hostCpuAffinity !== "8-15") {
  problems.push(`CPU affinity ${report.hostCpuAffinity}, expected 8-15`);
}
if (JSON.stringify(report.browser) !== JSON.stringify(expectedBrowser)) {
  problems.push(`browser changed: ${JSON.stringify(report.browser)}`);
}
if (JSON.stringify(report.protocol?.jitConfiguration) !== JSON.stringify(expectedConfiguration)) {
  problems.push(`candidate configuration changed: ${JSON.stringify(report.protocol?.jitConfiguration)}`);
}
if (report.protocol?.expectedMainWasmSha256 !==
    "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c") {
  problems.push(`main Wasm changed: ${report.protocol?.expectedMainWasmSha256}`);
}
if (JSON.stringify(report.protocol?.expectedArtifacts) !== JSON.stringify(expectedArtifacts)) {
  problems.push(`artifact manifest changed: ${JSON.stringify(report.protocol?.expectedArtifacts)}`);
}
if (JSON.stringify(report.protocol?.toolManifest) !== JSON.stringify(expectedTools)) {
  problems.push(`tool manifest changed: ${JSON.stringify(report.protocol?.toolManifest)}`);
}
if (report.protocol?.jitConfigurationLifecycle !== "preboot") {
  problems.push(`candidate lifecycle ${report.protocol?.jitConfigurationLifecycle}, expected preboot`);
}
if (report.artifacts?.page?.sha256 !== expectedArtifacts.page) {
  problems.push(`page bytes changed: ${report.artifacts?.page?.sha256}`);
}
for (const name of ["rv64Jit", "rv64Root", "v86", "x86Root"]) {
  if (report.artifacts?.archives?.[name]?.sha256 !== expectedArtifacts[name]) {
    problems.push(`${name} bytes changed: ${report.artifacts?.archives?.[name]?.sha256}`);
  }
}
for (const phase of ["python", "sha256", "shared9p"]) {
  const result = report.phases?.[phase];
  if (!result || result.rv64?.length !== 7 || result.v86?.length !== 7) {
    problems.push(`${phase}: expected seven complete paired observations`);
    continue;
  }
  if (!result.rv64Raw?.every((values) => values.length === 3) ||
      !result.v86Raw?.every((values) => values.length === 3)) {
    problems.push(`${phase}: expected three repetitions in every leg`);
  }
  if (result.noninferiorAtRequestedMargin !== true) {
    problems.push(`${phase}: failed frozen 10% product margin`);
  }
  if (!Number.isFinite(result.pairedMedian) || result.pairedMedian > 1.10) {
    problems.push(`${phase}: paired median ${result.pairedMedian} exceeds 1.10`);
  }
  const upper = result.pairedMedianBootstrap95?.[1];
  if (!Number.isFinite(upper) || upper > 1.10) {
    problems.push(`${phase}: bootstrap upper ${upper} exceeds 1.10`);
  }
  console.log(
    `${phase}: ${result.pairedMedian.toFixed(3)} ` +
      `CI=[${result.pairedMedianBootstrap95.map((value) => value.toFixed(3)).join(", ")}]`,
  );
}

if (problems.length) {
  console.error(`R076_WANIX_V86_GATE_FAIL: ${problems.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("R076_WANIX_V86_GATE_PASS");
}
