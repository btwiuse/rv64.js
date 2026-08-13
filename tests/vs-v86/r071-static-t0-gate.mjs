#!/usr/bin/env node

// Machine-check the frozen native gate in
// docs/jit-rewrite/R071_CUMULATIVE_GAIN_CONFIRMATION_PROTOCOL.md. Keep this
// verifier candidate-specific: changing any constant creates a new protocol,
// and the old report must continue to be judged by these exact rules.

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const reportArg = process.argv[2];
if (!reportArg) {
  throw new Error("usage: node tests/vs-v86/r071-static-t0-gate.mjs REPORT.json");
}

const report = JSON.parse(await readFile(resolve(reportArg), "utf8"));
const EXPECTED_WASM =
  "8314d06d89bf1957548f4c6297d12aed1e4e9563fd29a31390e4393b12e97e62";
const EXPECTED_ROWS = ["boot", "compile", "python"];
const EXPECTED_CONTROL = { SCORECARD_V2_STATIC_SYSTEM_T0: "0" };
const EXPECTED_CANDIDATE = { SCORECARD_V2_STATIC_SYSTEM_T0: "1" };
const gates = Object.freeze({
  boot: { median: 1.03, lower: 1.00 },
  compile: { median: 0.97 },
  python: { median: 0.97 },
});
const problems = [];

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

if (report.schema !== 1) problems.push(`schema ${report.schema}, expected 1`);
if (report.measurementValid !== true) problems.push("underlying report is invalid");
if (report.authoritative !== false) problems.push("configuration A/B must be non-authoritative");
if (!same(report.configuration?.rows, EXPECTED_ROWS)) {
  problems.push(`rows ${JSON.stringify(report.configuration?.rows)}, expected ${JSON.stringify(EXPECTED_ROWS)}`);
}
if (report.configuration?.reps !== 5) {
  problems.push(`reps ${report.configuration?.reps}, expected 5`);
}
if (!same(report.configuration?.controlConfig, EXPECTED_CONTROL)) {
  problems.push(`control config changed: ${JSON.stringify(report.configuration?.controlConfig)}`);
}
if (!same(report.configuration?.candidateConfig, EXPECTED_CANDIDATE)) {
  problems.push(`candidate config changed: ${JSON.stringify(report.configuration?.candidateConfig)}`);
}
for (const side of ["control", "candidate"]) {
  const identity = report.configuration?.wasmBySide?.[side];
  if (identity?.sha256 !== EXPECTED_WASM) {
    problems.push(`${side} Wasm ${identity?.sha256}, expected ${EXPECTED_WASM}`);
  }
}
if (report.configuration?.wasmBySide?.control?.path !==
    report.configuration?.wasmBySide?.candidate?.path) {
  problems.push("control and candidate did not use the same main Wasm path");
}
if (report.hostProbeSpread > 1.25) {
  problems.push(`host probe spread ${report.hostProbeSpread} exceeds 1.25`);
}
if (Array.isArray(report.problems) && report.problems.length) {
  problems.push(...report.problems.map((problem) => `underlying: ${problem}`));
}

for (const row of EXPECTED_ROWS) {
  const aggregate = report.aggregates?.[row];
  const paired = aggregate?.pairedCandidateSpeedup;
  const rule = gates[row];
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

const expectedTrials = EXPECTED_ROWS.length * 5 * 2;
if (report.trials?.length !== expectedTrials) {
  problems.push(`trial count ${report.trials?.length}, expected ${expectedTrials}`);
}
for (const trial of report.trials ?? []) {
  if (!trial.result) continue;
  const lifecycle = trial.result.runtime?.staticSystemT0;
  const shouldEnable = trial.side === "candidate";
  if (!lifecycle || lifecycle.enabled !== shouldEnable ||
      lifecycle.modulesAfter !== lifecycle.modulesBefore + 1) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: invalid static-T0 lifecycle`);
  }
  const phase = trial.row === "boot" ? "first" : "steady";
  const counters = trial.result.phases?.[phase]?.counters;
  if (!counters) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: counters missing`);
    continue;
  }
  const fast = BigInt(counters.staticT0FastInstructions ?? "0");
  const errors = BigInt(counters.staticT0Errors ?? "0");
  if (shouldEnable && fast === 0n) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: no candidate fast retirement`);
  }
  if (!shouldEnable && fast !== 0n) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: control executed static T0`);
  }
  if (errors !== 0n) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: ${errors} static-T0 errors`);
  }
}

for (const row of EXPECTED_ROWS) {
  const paired = report.aggregates?.[row]?.pairedCandidateSpeedup;
  if (paired) {
    const confidence = paired.medianConfidence95 ?? [];
    console.log(
      `${row}: ${paired.median.toFixed(3)}x ` +
      `CI=[${confidence.map((value) => value.toFixed(3)).join(", ")}]`,
    );
  }
}
if (problems.length) {
  console.error(`R071_GATE_A_FAIL: ${problems.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("R071_GATE_A_PASS");
}
