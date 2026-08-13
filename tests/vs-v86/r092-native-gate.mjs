#!/usr/bin/env node

// Verify R092's frozen five-pair product gate. Compile must supply the accepted
// >=3% end-to-end win; Boot and Python remain strict elapsed-time guards.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const [reportPath, outputPath] = process.argv.slice(2);
if (!reportPath || !outputPath) {
  throw new Error("usage: r092-native-gate.mjs REPORT.json OUTPUT.json");
}

const CONTROL = "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010";
const CANDIDATE = "5baeccb5c5feaf2d3f7605fd42f741f9cbaa89e566a86c0bbea201a3c6389023";
const LOADER = "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385";
const CADENCE = "public-one-slice-per-turn";
const COMPILE_MD5 = "24eedf7e06beffd4d3ba1945585588db";
const REPS = 5;
const MIN_COMPILE_MEDIAN = 1.03;
const MIN_COMPILE_LOWER = 0.98;
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
  // scorecard-v2-worker deliberately marks an explicit Wasm-path override as
  // proof-only. The config-A/B parent is the measurement authority: it hashes
  // both artifacts, balances order, and emits a valid engineering report.
  // Require that override to be the sole diagnostic so no feature switch can
  // enter this product comparison unnoticed.
  const diagnostics = result.runtime?.diagnostic ?? {};
  check(result.measurementEligible === false,
    `${prefix}: artifact-override worker was unexpectedly scorecard-eligible`);
  check(Object.keys(diagnostics).length === 1 &&
    diagnostics.rewriteWasmOverride === report.configuration.wasmBySide[trial.side].path,
  `${prefix}: artifact override was not the sole diagnostic`);
  check(result.runtime?.identity?.wasmSha256 === expectedHash,
    `${prefix}: runtime artifact changed`);
  check(result.runtime?.identity?.loaderSha256 === LOADER,
    `${prefix}: loader changed`);
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

  if (trial.row === "compile") {
    for (const phase of ["first", "prime", "steady"]) {
      const phaseResult = result.phases?.[phase];
      check(phaseResult?.md5 === COMPILE_MD5, `${prefix}/${phase}: Compile MD5 changed`);
      const selected = BigInt(phaseResult?.counters?.memberRangeMembersTranslated || 0);
      if (trial.side === "candidate") {
        check(selected > 0n, `${prefix}/${phase}: candidate selected no range members`);
      } else {
        check(selected === 0n, `${prefix}/${phase}: R085 control reports range selection`);
      }
    }
  }
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

const elapsedRatio = (row) => row?.candidate?.median / row?.control?.median;
const boot = report.aggregates?.boot;
const compile = report.aggregates?.compile;
const python = report.aggregates?.python;
const compileQualifies = compile?.pairedCandidateSpeedup?.median >= MIN_COMPILE_MEDIAN &&
  compile?.pairedCandidateSpeedup?.medianConfidence95?.[0] >= MIN_COMPILE_LOWER;
const bootGuard = elapsedRatio(boot) <= MAX_ELAPSED_RATIO;
const compileGuard = elapsedRatio(compile) <= MAX_ELAPSED_RATIO;
const pythonGuard = elapsedRatio(python) <= MAX_ELAPSED_RATIO;
const admitCorrectnessAndBrowser = integrityProblems.length === 0 && compileQualifies &&
  bootGuard && compileGuard && pythonGuard;

const observedRow = (row) => ({
  controlMedianMs: row?.control.median,
  candidateMedianMs: row?.candidate.median,
  elapsedRatio: elapsedRatio(row),
  pairedSpeedup: row?.pairedCandidateSpeedup?.median,
  pairedConfidence95: row?.pairedCandidateSpeedup?.medianConfidence95,
});
const gate = {
  schema: 1,
  experiment: "R092",
  report: {
    path: reportPath,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  },
  frozenRequirements: {
    compileMedianSpeedup: MIN_COMPILE_MEDIAN,
    compileLowerBound: MIN_COMPILE_LOWER,
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
    compileQualifies,
    bootGuard,
    compileGuard,
    pythonGuard,
  },
  integrityProblems,
  admitCorrectnessAndBrowser,
  decision: admitCorrectnessAndBrowser
    ? "escalate-to-full-correctness-and-browser"
    : "reject-and-restore-r085",
};
writeFileSync(outputPath, `${JSON.stringify(gate, null, 2)}\n`, { flag: "wx" });
if (integrityProblems.length) process.exitCode = 1;
