#!/usr/bin/env node

// R115 historical-audit gate. The first report isolates instance ownership;
// the second asks whether the same embedded executor beats an identical
// artifact with that executor disabled.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const [boundaryPath, executorPath, outputPath] = process.argv.slice(2);
if (!boundaryPath || !executorPath || !outputPath) {
  throw new Error(
    "usage: r115-same-instance-gate.mjs BOUNDARY.json EXECUTOR.json OUTPUT.json",
  );
}

const MAIN = "a4c0c34f67e4e78a57af372746ea1917f78ea69d8bf5094378cb96b3ee0dfe82";
const AUX = "2be7aab637b0105fbe1e8a8b515263c0fa1f7b419b8329b829ebdd5b7ea563f5";
const LOADER = "4dad0ea90b4d6829ec8d5308fc720cf5c00ede8954af79db5fb4284ebfb9d636";
const ROWS = ["boot", "compile", "python"];
const REPS = 7;
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const boundaryBytes = readFileSync(boundaryPath);
const executorBytes = readFileSync(executorPath);
const boundary = JSON.parse(boundaryBytes);
const executor = JSON.parse(executorBytes);
const problems = [];
const check = (condition, message) => {
  if (!condition) problems.push(message);
};
const aggregate = (report, row) => report.aggregates?.[row];
const paired = (report, row) => aggregate(report, row)?.pairedCandidateSpeedup;
const without = (object, keys) => Object.fromEntries(
  Object.entries(object ?? {}).filter(([key]) => !keys.includes(key)),
);

function checkCommon(report, label) {
  check(JSON.stringify(report.configuration?.rows) === JSON.stringify(ROWS),
    `${label}: row set changed`);
  check(report.configuration?.reps === REPS, `${label}: repetition count changed`);
  check(report.trials?.length === ROWS.length * REPS * 2,
    `${label}: expected 42 trials`);
  check(report.hostProbeSpread <= 1.10, `${label}: host spread exceeds 1.10x`);
  for (const trial of report.trials ?? []) {
    const prefix = `${label}/${trial.side}/${trial.row}/rep${trial.rep}`;
    check(!trial.error && trial.result, `${prefix}: unsuccessful leg`);
    if (!trial.result) continue;
    const runtime = trial.result.runtime;
    check(runtime?.identity?.wasmSha256 === MAIN, `${prefix}: main identity changed`);
    check(runtime?.identity?.externalT0Sha256 === AUX, `${prefix}: aux identity changed`);
    check(runtime?.identity?.loaderSha256 === LOADER, `${prefix}: loader identity changed`);
    const phase = trial.row === "boot" ? "first" : "steady";
    const counters = trial.result.phases?.[phase]?.counters;
    check(counters, `${prefix}: counters missing`);
    check(BigInt(counters?.externalT0Errors ?? -1) === 0n, `${prefix}: Tier-0 errors`);
    check(BigInt(runtime?.jitProof?.generatedInstructions ?? 0) > 0n,
      `${prefix}: ordinary generated execution missing`);
  }
  for (const row of ROWS) {
    const rowAggregate = aggregate(report, row);
    check(rowAggregate?.control?.values?.length === REPS, `${label}/${row}: control count`);
    check(rowAggregate?.candidate?.values?.length === REPS, `${label}/${row}: candidate count`);
    check(paired(report, row)?.raw?.length === REPS, `${label}/${row}: pair count`);
    check(JSON.stringify(rowAggregate?.control?.inputs) ===
      JSON.stringify(rowAggregate?.candidate?.inputs), `${label}/${row}: inputs differ`);
    check(JSON.stringify(rowAggregate?.control?.fingerprints) ===
      JSON.stringify(rowAggregate?.candidate?.fingerprints), `${label}/${row}: outputs differ`);
    for (let rep = 1; rep <= REPS; rep++) {
      const control = report.trials.find((trial) =>
        trial.row === row && trial.rep === rep && trial.side === "control")?.result;
      const candidate = report.trials.find((trial) =>
        trial.row === row && trial.rep === rep && trial.side === "candidate")?.result;
      const phase = row === "boot" ? "first" : "steady";
      const controlWork = Number(control?.phases?.[phase]?.counters?.guestInstructions);
      const candidateWork = Number(candidate?.phases?.[phase]?.counters?.guestInstructions);
      check(Number.isFinite(controlWork) && Number.isFinite(candidateWork) &&
        candidateWork / controlWork >= 0.995 && candidateWork / controlWork <= 1.005,
      `${label}/${row}/rep${rep}: guest work differs`);
    }
  }
}

checkCommon(boundary, "boundary");
check(boundary.measurementValid === true && boundary.problems?.length === 0,
  "boundary: report is not valid");
const boundaryControl = boundary.configuration?.controlConfig;
const boundaryCandidate = boundary.configuration?.candidateConfig;
check(JSON.stringify(without(boundaryControl, ["SCORECARD_V2_R115_T0_MODE"])) ===
  JSON.stringify(without(boundaryCandidate, ["SCORECARD_V2_R115_T0_MODE"])),
"boundary: legs differ by more than instance mode");
check(boundaryControl?.SCORECARD_V2_R115_T0_MODE === "external",
  "boundary: control mode is not external");
check(boundaryCandidate?.SCORECARD_V2_R115_T0_MODE === "embedded",
  "boundary: candidate mode is not embedded");
check(boundaryControl?.SCORECARD_V2_EXTERNAL_T0_ENABLED === "1" &&
  boundaryCandidate?.SCORECARD_V2_EXTERNAL_T0_ENABLED === "1",
"boundary: Tier-0 was not enabled on both legs");
for (const trial of boundary.trials ?? []) {
  const phase = trial.row === "boot" ? "first" : "steady";
  check(BigInt(trial.result?.phases?.[phase]?.counters?.externalT0FastInstructions ?? 0) > 0n,
    `boundary/${trial.side}/${trial.row}/rep${trial.rep}: no Tier-0 execution`);
}

const boundaryChecks = {
  bootMedian: paired(boundary, "boot")?.median >= 1.01,
  bootLowerBound: paired(boundary, "boot")?.medianConfidence95?.[0] >= 1.00,
  compileGuard: paired(boundary, "compile")?.median >= 0.99,
  pythonGuard: paired(boundary, "python")?.median >= 0.99,
};

checkCommon(executor, "executor");
check(executor.hostProbeSpread <= 1.10, "executor: unstable host");
check(JSON.stringify(executor.problems) ===
  JSON.stringify(["control/compile: sample spread 1.286 exceeds 1.25"]),
"executor: unexpected validity problem set");
const executorControl = executor.configuration?.controlConfig;
const executorCandidate = executor.configuration?.candidateConfig;
check(JSON.stringify(without(executorControl, ["SCORECARD_V2_EXTERNAL_T0_ENABLED"])) ===
  JSON.stringify(without(executorCandidate, ["SCORECARD_V2_EXTERNAL_T0_ENABLED"])),
"executor: legs differ by more than enable cell");
check(executorControl?.SCORECARD_V2_R115_T0_MODE === "embedded" &&
  executorCandidate?.SCORECARD_V2_R115_T0_MODE === "embedded",
"executor: both legs did not select embedded mode");
check(executorControl?.SCORECARD_V2_EXTERNAL_T0_ENABLED === "0" &&
  executorCandidate?.SCORECARD_V2_EXTERNAL_T0_ENABLED === "1",
"executor: enable cells are not zero/one");
for (const trial of executor.trials ?? []) {
  const phase = trial.row === "boot" ? "first" : "steady";
  const fast = BigInt(
    trial.result?.phases?.[phase]?.counters?.externalT0FastInstructions ?? -1,
  );
  check(trial.side === "candidate" ? fast > 0n : fast === 0n,
    `executor/${trial.side}/${trial.row}/rep${trial.rep}: enable proof failed`);
}

const executorBoot = paired(executor, "boot");
const executorChecks = {
  completeBootSample:
    aggregate(executor, "boot")?.control?.spread <= 1.10 &&
    aggregate(executor, "boot")?.candidate?.spread <= 1.10 &&
    executorBoot?.raw?.length === REPS,
  bootMedianRegresses: executorBoot?.median < 0.99,
  bootUpperBoundBelowParity: executorBoot?.medianConfidence95?.[1] < 1.00,
  noOpenRowQualifies:
    !ROWS.slice(0, 2).some((row) =>
      paired(executor, row)?.median >= 1.01 &&
      paired(executor, row)?.medianConfidence95?.[0] >= 1.00),
};

const boundaryEstablished = problems.length === 0 &&
  Object.values(boundaryChecks).every(Boolean);
const executorRejected = problems.length === 0 &&
  Object.values(executorChecks).every(Boolean);
const result = {
  schema: 1,
  experiment: "R115 same-instance Tier-0 historical audit",
  reports: {
    boundary: { path: boundaryPath, sha256: digest(boundaryBytes) },
    executor: { path: executorPath, sha256: digest(executorBytes) },
  },
  frozenIdentities: { main: MAIN, auxiliary: AUX, loader: LOADER },
  boundary: {
    measurementValid: boundary.measurementValid,
    hostProbeSpread: boundary.hostProbeSpread,
    observed: Object.fromEntries(ROWS.map((row) => [row, paired(boundary, row)])),
    checks: boundaryChecks,
    established: boundaryEstablished,
  },
  executor: {
    measurementValid: executor.measurementValid,
    hostProbeSpread: executor.hostProbeSpread,
    reportProblems: executor.problems,
    observed: Object.fromEntries(ROWS.map((row) => [row, paired(executor, row)])),
    checks: executorChecks,
    rejected: executorRejected,
  },
  integrityProblems: problems,
  decision: boundaryEstablished && executorRejected
    ? "cross-instance-tax-established; embedded-executor-rejected"
    : "invalid-or-inconclusive",
};
writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`, { flag: "wx" });
console.log(`R115: ${result.decision}`);
console.log(JSON.stringify({ boundary: result.boundary, executor: result.executor }, null, 2));
if (problems.length) process.exitCode = 1;
