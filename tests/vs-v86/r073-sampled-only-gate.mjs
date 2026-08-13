#!/usr/bin/env node

// Machine-check the candidate-specific native gate frozen in
// docs/jit-rewrite/R073_SAMPLED_ONLY_STATIC_T0_PROTOCOL.md.

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const reportArg = process.argv[2];
if (!reportArg) {
  throw new Error("usage: node tests/vs-v86/r073-sampled-only-gate.mjs REPORT.json");
}
const report = JSON.parse(await readFile(resolve(reportArg), "utf8"));
const expectedWasm =
  "cb7ea81685b3cb9605f6e01b619f3c15005f4ef77ca00c258c20c7a519bb6df6";
const expectedRows = ["boot", "compile", "python"];
const expectedControl = {
  SCORECARD_V2_STATIC_SYSTEM_T0: "0",
  SCORECARD_V2_SAMPLED_STATIC_T0: "0",
};
const expectedCandidate = {
  SCORECARD_V2_STATIC_SYSTEM_T0: "0",
  SCORECARD_V2_SAMPLED_STATIC_T0: "1",
};
const rules = {
  boot: { median: 1.10, lower: 1.00 },
  compile: { median: 0.97 },
  python: { median: 0.97 },
};
const problems = [];
const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);

if (report.schema !== 1) problems.push(`schema ${report.schema}, expected 1`);
if (report.measurementValid !== true) problems.push("underlying report is invalid");
if (report.authoritative !== false) problems.push("configuration A/B must be non-authoritative");
if (!same(report.configuration?.rows, expectedRows)) problems.push("row set changed");
if (report.configuration?.reps !== 5) problems.push(`reps ${report.configuration?.reps}, expected 5`);
if (!same(report.configuration?.controlConfig, expectedControl)) {
  problems.push(`control config changed: ${JSON.stringify(report.configuration?.controlConfig)}`);
}
if (!same(report.configuration?.candidateConfig, expectedCandidate)) {
  problems.push(`candidate config changed: ${JSON.stringify(report.configuration?.candidateConfig)}`);
}
for (const side of ["control", "candidate"]) {
  if (report.configuration?.wasmBySide?.[side]?.sha256 !== expectedWasm) {
    problems.push(`${side} Wasm identity changed`);
  }
}
if (
  report.configuration?.wasmBySide?.control?.path !==
  report.configuration?.wasmBySide?.candidate?.path
) {
  problems.push("control and candidate did not use the same main Wasm path");
}
if (!Number.isFinite(report.hostProbeSpread) || report.hostProbeSpread > 1.25) {
  problems.push(`host probe spread ${report.hostProbeSpread} exceeds 1.25`);
}
if (report.problems?.length) {
  problems.push(...report.problems.map((problem) => `underlying: ${problem}`));
}

for (const row of expectedRows) {
  const paired = report.aggregates?.[row]?.pairedCandidateSpeedup;
  const rule = rules[row];
  if (!paired || !Number.isFinite(paired.median)) {
    problems.push(`${row}: paired result missing`);
    continue;
  }
  if (paired.raw?.length !== 5) problems.push(`${row}: expected five complete pairs`);
  if (paired.median < rule.median) {
    problems.push(`${row}: median ${paired.median.toFixed(4)}x below ${rule.median.toFixed(2)}x`);
  }
  if (rule.lower !== undefined) {
    const lower = paired.medianConfidence95?.[0];
    if (!Number.isFinite(lower) || lower < rule.lower) {
      problems.push(`${row}: lower bound ${lower} below ${rule.lower.toFixed(2)}x`);
    }
  }
}

if (report.trials?.length !== expectedRows.length * 5 * 2) {
  problems.push(`trial count ${report.trials?.length}, expected 30`);
}
for (const trial of report.trials ?? []) {
  if (!trial.result) continue;
  const candidate = trial.side === "candidate";
  const lifecycle = trial.result.runtime?.staticSystemT0;
  if (
    !lifecycle ||
    lifecycle.enabled !== false ||
    lifecycle.sampled !== candidate ||
    lifecycle.modulesAfter !== lifecycle.modulesBefore + 1
  ) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: invalid sampled-only lifecycle`);
  }
  const phase = trial.row === "boot" ? "first" : "steady";
  const counters = trial.result.phases?.[phase]?.counters;
  if (!counters) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: counters missing`);
    continue;
  }
  const fast = BigInt(counters.staticT0FastInstructions ?? "0");
  const errors = BigInt(counters.staticT0Errors ?? "0");
  const sampled = BigInt(counters.staticT0SampledInstructions ?? "0");
  const samples = BigInt(counters.staticT0Samples ?? "0");
  const polls = BigInt(counters.staticT0InterruptPolls ?? "0");
  if (candidate && (fast === 0n || sampled === 0n || samples === 0n || polls === 0n)) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: incomplete sampled-only proof`);
  }
  if (!candidate && (fast !== 0n || sampled !== 0n || samples !== 0n || polls !== 0n)) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: control executed static T0`);
  }
  if (errors !== 0n) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: ${errors} static-T0 errors`);
  }
}

for (const row of expectedRows) {
  const paired = report.aggregates?.[row]?.pairedCandidateSpeedup;
  if (paired) {
    console.log(
      `${row}: ${paired.median.toFixed(3)}x ` +
        `CI=[${paired.medianConfidence95.map((value) => value.toFixed(3)).join(", ")}]`,
    );
  }
}
if (problems.length) {
  console.error(`R073_NATIVE_GATE_FAIL: ${problems.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("R073_NATIVE_GATE_PASS");
}

