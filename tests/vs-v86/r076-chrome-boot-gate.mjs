#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const reportArg = process.argv[2];
if (!reportArg) {
  throw new Error("usage: node tests/vs-v86/r076-chrome-boot-gate.mjs ANALYSIS.json");
}
const report = JSON.parse(await readFile(resolve(reportArg), "utf8"));
const expectedThresholds = {
  minimumExecutionSpeedup: 1.05,
  minimumExecutionBootstrapLower: 1.00,
  minimumMipsRatio: 1.00,
  minimumMipsBootstrapLower: 0.97,
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

if (report.schema !== 1) problems.push(`schema ${report.schema}, expected 1`);
if (report.experiment !== "R076 Chrome execution-only modern Boot") {
  problems.push(`experiment ${report.experiment}`);
}
if (report.measurementValid !== true) problems.push("underlying measurement invalid");
if (report.gatePassed !== true) problems.push("underlying gate failed");
if (report.method?.pairs !== 7) problems.push("expected seven pairs");
if (!same(report.method?.thresholds, expectedThresholds)) problems.push("thresholds changed");
if (!same(report.browser, expectedBrowser)) problems.push("browser changed");
if (report.hostCpuAffinity !== "8-15") problems.push("CPU affinity changed");
if (!same(report.guest, { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" })) {
  problems.push("guest contract changed");
}
if (!Number.isInteger(report.staticModuleIndex) || report.staticModuleIndex < 0) {
  problems.push("static module proof missing");
}
if (report.problems?.length) {
  problems.push(...report.problems.map((problem) => `underlying: ${problem}`));
}

const execution = report.execution;
if (!execution || execution.control?.length !== 7 || execution.candidate?.length !== 7 ||
    execution.pairedRatios?.length !== 7) {
  problems.push("execution sample incomplete");
} else {
  const lower = execution.pairedMedianBootstrap95?.[0];
  if (!Number.isFinite(execution.pairedMedian) || execution.pairedMedian < 1.05) {
    problems.push(`execution speedup ${execution.pairedMedian} below 1.05`);
  }
  if (!Number.isFinite(lower) || lower < 1.00) {
    problems.push(`execution lower ${lower} below 1.00`);
  }
  console.log(
    `execution: ${execution.pairedMedian.toFixed(3)}x ` +
      `CI=[${execution.pairedMedianBootstrap95.map((value) => value.toFixed(3)).join(", ")}]`,
  );
}

const mips = report.mips;
if (!mips || mips.control?.length !== 7 || mips.candidate?.length !== 7 ||
    mips.pairedRatios?.length !== 7) {
  problems.push("MIPS sample incomplete");
} else {
  const lower = mips.pairedMedianBootstrap95?.[0];
  if (!Number.isFinite(mips.pairedMedian) || mips.pairedMedian < 1.00) {
    problems.push(`MIPS ratio ${mips.pairedMedian} below 1.00`);
  }
  if (!Number.isFinite(lower) || lower < 0.97) {
    problems.push(`MIPS lower ${lower} below 0.97`);
  }
  console.log(
    `MIPS: ${mips.pairedMedian.toFixed(3)}x ` +
      `CI=[${mips.pairedMedianBootstrap95.map((value) => value.toFixed(3)).join(", ")}]`,
  );
}

for (const side of ["control", "candidate"]) {
  const counters = report.counters?.[side];
  if (!counters || !(counters.instructions > 100_000_000) || !(counters.generated > 0)) {
    problems.push(`${side} aggregate proof missing`);
    continue;
  }
  for (const field of ["staticFast", "sampled", "samples", "polls", "marks", "bypasses"]) {
    if (side === "candidate" && !(counters[field] > 0)) {
      problems.push(`candidate ${field} proof missing`);
    }
    if (side === "control" && counters[field] !== 0) {
      problems.push(`control ${field} activity nonzero`);
    }
  }
  if (counters.errors !== 0) problems.push(`${side} static errors nonzero`);
}

if (problems.length) {
  console.error(`R076_CHROME_BOOT_GATE_FAIL: ${problems.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("R076_CHROME_BOOT_GATE_PASS");
}
