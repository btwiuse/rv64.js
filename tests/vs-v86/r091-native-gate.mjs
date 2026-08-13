#!/usr/bin/env node

// Verify R091's frozen five-pair product gate. Either target row may supply
// the accepted >=3% end-to-end win; both target rows and Python are guarded.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const [reportPath, outputPath] = process.argv.slice(2);
if (!reportPath || !outputPath) {
  throw new Error("usage: r091-native-gate.mjs REPORT.json OUTPUT.json");
}

const CONTROL = "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010";
const CANDIDATE = "a8f14136e7d217f4e71aec2c52020f749c476ee2531268f0bab7adfff2e42c75";
const CADENCE = "public-one-slice-per-turn";
const REPS = 5;
const MIN_TARGET_MEDIAN = 1.03;
const MIN_TARGET_LOWER = 0.98;
const MAX_ELAPSED_RATIO = 1.03;
const MAX_HOST_SPREAD = 1.10;

const bytes = readFileSync(reportPath);
const report = JSON.parse(bytes);
const integrityProblems = [];
const check = (condition, message) => {
  if (!condition) integrityProblems.push(message);
};

check(report.measurementValid === true, "config A/B report is not measurement-valid");
check(report.authoritative === false, "config A/B report unexpectedly authoritative");
check(report.problems?.length === 0, "config A/B report contains problems");
check(JSON.stringify(report.configuration?.rows) === JSON.stringify(["boot", "compile", "python"]),
  "row set or order changed");
check(report.configuration?.reps === REPS, "repetition count changed");
check(report.configuration?.wasmBySide?.control?.sha256 === CONTROL,
  "control is not exact R085");
check(report.configuration?.wasmBySide?.candidate?.sha256 === CANDIDATE,
  "candidate artifact changed");
check(report.hostProbeSpread <= MAX_HOST_SPREAD, "host probe spread exceeds 1.10x");
check(report.trials?.length === 3 * 2 * REPS, "expected exactly 30 retained trials");

for (const trial of report.trials || []) {
  const result = trial.result;
  const prefix = `${trial.side}/${trial.row}/rep${trial.rep}`;
  const expectedHash = trial.side === "control" ? CONTROL : CANDIDATE;
  check(!trial.error && result, `${prefix}: missing successful result`);
  if (!result) continue;
  check(result.runtime?.identity?.wasmSha256 === expectedHash,
    `${prefix}: runtime artifact changed`);
  check(result.runtime?.schedulerCadence?.name === CADENCE &&
    result.runtime.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
  `${prefix}: scheduler cadence changed`);
  check(result.runtime?.guest?.linux === "6.12.7" &&
    result.runtime.guest.alpine === "3.24.1" &&
    result.runtime.guest.arch === "riscv64", `${prefix}: modern guest changed`);
  check(result.runtime?.requestedPolicy?.name === "production-page",
    `${prefix}: production policy changed`);
  check(result.runtime?.policyProblems?.length === 0, `${prefix}: policy problem`);
  check(BigInt(result.runtime?.jitProof?.generatedInstructions || 0) > 0n,
    `${prefix}: generated execution not proved`);
  check(trial.hostBeforeMs > 0 && trial.hostAfterMs > 0,
    `${prefix}: host probes missing`);
}

for (const row of ["boot", "compile", "python"]) {
  const aggregate = report.aggregates?.[row];
  check(aggregate, `${row}: aggregate missing`);
  if (!aggregate) continue;
  check(aggregate.control?.values?.length === REPS &&
    aggregate.candidate?.values?.length === REPS, `${row}: incomplete side samples`);
  check(aggregate.pairedCandidateSpeedup?.raw?.length === REPS,
    `${row}: incomplete paired samples`);
  check(aggregate.control?.spread <= report.configuration.maximumSampleSpread &&
    aggregate.candidate?.spread <= report.configuration.maximumSampleSpread,
  `${row}: sample spread exceeds frozen scorecard limit`);
  check(JSON.stringify(aggregate.control?.inputs) === JSON.stringify(aggregate.candidate?.inputs),
    `${row}: input identities differ`);
  check(JSON.stringify(aggregate.control?.fingerprints) ===
    JSON.stringify(aggregate.candidate?.fingerprints), `${row}: output fingerprints differ`);
}

const qualifies = (row) => row?.pairedCandidateSpeedup?.median >= MIN_TARGET_MEDIAN &&
  row?.pairedCandidateSpeedup?.medianConfidence95?.[0] >= MIN_TARGET_LOWER;
const elapsedRatio = (row) => row?.candidate?.median / row?.control?.median;
const boot = report.aggregates?.boot;
const compile = report.aggregates?.compile;
const python = report.aggregates?.python;
const bootQualifies = qualifies(boot);
const compileQualifies = qualifies(compile);
const targetWin = bootQualifies || compileQualifies;
const bootGuard = elapsedRatio(boot) <= MAX_ELAPSED_RATIO;
const compileGuard = elapsedRatio(compile) <= MAX_ELAPSED_RATIO;
const pythonGuard = elapsedRatio(python) <= MAX_ELAPSED_RATIO;
const admitBrowser = integrityProblems.length === 0 && targetWin && bootGuard &&
  compileGuard && pythonGuard;

const observedRow = (row) => ({
  controlMedianMs: row?.control.median,
  candidateMedianMs: row?.candidate.median,
  elapsedRatio: elapsedRatio(row),
  pairedSpeedup: row?.pairedCandidateSpeedup?.median,
  pairedConfidence95: row?.pairedCandidateSpeedup?.medianConfidence95,
});
const gate = {
  schema: 1,
  experiment: "R091",
  report: {
    path: reportPath,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  },
  frozenRequirements: {
    eitherBootOrCompileMedianSpeedup: MIN_TARGET_MEDIAN,
    winningTargetLowerBound: MIN_TARGET_LOWER,
    maximumElapsedRatioEveryRow: MAX_ELAPSED_RATIO,
    maximumHostSpread: MAX_HOST_SPREAD,
  },
  observed: {
    hostProbeSpread: report.hostProbeSpread,
    boot: observedRow(boot),
    compile: observedRow(compile),
    python: observedRow(python),
  },
  checks: {
    integrity: integrityProblems.length === 0,
    bootQualifies,
    compileQualifies,
    targetWin,
    bootGuard,
    compileGuard,
    pythonGuard,
  },
  integrityProblems,
  admitBrowser,
  decision: admitBrowser ? "escalate-to-browser" : "reject-and-restore-r085",
};
writeFileSync(outputPath, `${JSON.stringify(gate, null, 2)}\n`, { flag: "wx" });
if (integrityProblems.length) process.exitCode = 1;
