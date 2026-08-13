#!/usr/bin/env node

// Frozen verifier for R087's same-R085 historical/public pump-cadence A/B.

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const reportPath = process.argv[2];
if (!reportPath) {
  throw new Error("usage: r087-public-cadence-gate.mjs REPORT.json [OUTPUT.json]");
}
const outputPath = process.argv[3];
const EXPECTED_WASM =
  "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010";
const EXPECTED_ROWS = ["boot", "compile", "python"];
const EXPECTED_REPS = 5;
const MIN_PAIRED_SPEEDUP = 0.97;

const bytes = await readFile(reportPath);
const report = JSON.parse(bytes);
const problems = [];
const require_ = (condition, message) => {
  if (!condition) problems.push(message);
};
function normalize(value) {
  if (Array.isArray(value)) return value.map(normalize);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.keys(value).sort().map((key) =>
      [key, normalize(value[key])]));
  }
  return value;
}
const canonical = (value) => JSON.stringify(normalize(value));

require_(report.measurementValid === true, "source report is not measurement-valid");
require_(Array.isArray(report.problems) && report.problems.length === 0,
  "source report has problems");
require_(JSON.stringify(report.configuration?.rows) === JSON.stringify(EXPECTED_ROWS),
  "row set/order differs");
require_(report.configuration?.reps === EXPECTED_REPS, "repetition count differs");
require_(canonical(report.configuration?.controlConfig) === canonical({
  SCORECARD_V2_HISTORICAL_BATCHED_PUMPS: "1",
}), "control is not the historical cadence only");
require_(canonical(report.configuration?.candidateConfig) === canonical({}),
  "candidate is not the ordinary default");
for (const side of ["control", "candidate"]) {
  require_(report.configuration?.wasmBySide?.[side]?.sha256 === EXPECTED_WASM,
    `${side} configured Wasm differs from R085`);
}
require_(Number.isFinite(report.hostProbeSpread) && report.hostProbeSpread <= 1.25,
  "host probe spread exceeds 1.25x");

const trials = Array.isArray(report.trials) ? report.trials : [];
require_(trials.length === EXPECTED_ROWS.length * EXPECTED_REPS * 2,
  `expected 30 trials, found ${trials.length}`);

for (const row of EXPECTED_ROWS) {
  const rowTrials = trials.filter((trial) => trial.row === row);
  require_(rowTrials.length === EXPECTED_REPS * 2, `${row} does not have ten trials`);
  for (let rep = 1; rep <= EXPECTED_REPS; rep++) {
    const pair = rowTrials.filter((trial) => trial.rep === rep);
    const expectedOrder = rep & 1 ? ["control", "candidate"] : ["candidate", "control"];
    require_(pair.length === 2, `${row}/rep${rep} is incomplete`);
    require_(JSON.stringify(pair.map((trial) => trial.side)) === JSON.stringify(expectedOrder),
      `${row}/rep${rep} order is not alternating`);
  }
}

for (const trial of trials) {
  const prefix = `${trial.side}/${trial.row}/rep${trial.rep}`;
  require_(!trial.error && trial.result, `${prefix} failed`);
  if (!trial.result) continue;
  const result = trial.result;
  const runtime = result.runtime || {};
  const proof = runtime.jitProof || {};
  const cadence = runtime.schedulerCadence || {};
  require_(result.side === "rewrite" && result.row === trial.row,
    `${prefix} worker identity mismatch`);
  require_(runtime.identity?.wasmSha256 === EXPECTED_WASM,
    `${prefix} runtime Wasm mismatch`);
  require_(runtime.guest?.linux === "6.12.7" && runtime.guest?.alpine === "3.24.1" &&
    runtime.guest?.arch === "riscv64", `${prefix} modern guest mismatch`);
  require_(runtime.requestedPolicy?.name === "production-page",
    `${prefix} did not request production policy`);
  require_(Array.isArray(runtime.policyProblems) && runtime.policyProblems.length === 0,
    `${prefix} production policy proof failed`);
  require_(BigInt(proof.generatedInstructions || 0) > 0n &&
    BigInt(proof.dispatches || 0) > 0n, `${prefix} generated execution proof failed`);
  require_(proof.enabledRequested === true, `${prefix} did not request JIT execution`);

  if (trial.side === "control") {
    require_(result.measurementEligible === false,
      `${prefix} historical diagnostic was marked eligible`);
    require_(cadence.name === "historical-four-slices-per-turn" &&
      cadence.rv64SlicesPerEventLoopTurn === 4, `${prefix} historical cadence mismatch`);
    require_(canonical(runtime.diagnostic) === canonical({ historicalBatchedPumps: true }),
      `${prefix} has unexpected diagnostic state`);
  } else {
    require_(result.measurementEligible === true,
      `${prefix} corrected default was marked ineligible`);
    require_(cadence.name === "public-one-slice-per-turn" &&
      cadence.rv64SlicesPerEventLoopTurn === 1, `${prefix} public cadence mismatch`);
    require_(runtime.diagnostic === null, `${prefix} candidate has a diagnostic override`);
  }

  const phase = trial.row === "boot" ? "first" : "steady";
  require_(Number.isFinite(result.phases?.[phase]?.value) &&
    result.phases[phase].value > 0, `${prefix} scored timing is missing`);
  if (trial.row === "compile") {
    require_(result.phases?.steady?.md5 === "24eedf7e06beffd4d3ba1945585588db",
      `${prefix} compile output mismatch`);
  }
  if (trial.row === "python") {
    require_(result.phases?.steady?.checksum === "832040",
      `${prefix} Python output mismatch`);
  }
}

for (const row of EXPECTED_ROWS) {
  const aggregate = report.aggregates?.[row];
  const paired = aggregate?.pairedCandidateSpeedup;
  require_(paired?.raw?.length === EXPECTED_REPS, `${row} paired sample is incomplete`);
  require_(Number.isFinite(paired?.median) && paired.median >= MIN_PAIRED_SPEEDUP,
    `${row} paired median breaches the 3% guard`);
  require_(aggregate?.control?.spread <= 1.25 && aggregate?.candidate?.spread <= 1.25,
    `${row} sample spread exceeds 1.25x`);
  require_(canonical(aggregate?.control?.inputs) === canonical(aggregate?.candidate?.inputs),
    `${row} input fingerprints differ`);
  require_(canonical(aggregate?.control?.fingerprints) ===
    canonical(aggregate?.candidate?.fingerprints), `${row} output fingerprints differ`);
}

const gate = {
  schema: 1,
  experiment: "R087",
  mechanism: "scorecard-public-one-slice-event-loop-cadence",
  productBytesChanged: false,
  report: {
    path: resolve(reportPath),
    sha256: createHash("sha256").update(bytes).digest("hex"),
  },
  thresholds: {
    repetitions: EXPECTED_REPS,
    minimumPairedCandidateSpeedup: MIN_PAIRED_SPEEDUP,
    maximumHostAndSampleSpread: 1.25,
  },
  pairedCandidateSpeedup: Object.fromEntries(EXPECTED_ROWS.map((row) => [
    row,
    report.aggregates?.[row]?.pairedCandidateSpeedup,
  ])),
  checks: {
    exactR085AllLegs: !problems.some((problem) => problem.includes("Wasm")),
    exactModernGuestAllLegs: !problems.some((problem) => problem.includes("guest")),
    productionPolicyAllLegs: !problems.some((problem) => problem.includes("policy")),
    generatedExecutionAllLegs: !problems.some((problem) => problem.includes("generated")),
    exactCadenceAndEligibilityAllLegs: !problems.some((problem) =>
      problem.includes("cadence") || problem.includes("eligible") ||
      problem.includes("diagnostic")),
    outputAndInputFingerprints: !problems.some((problem) =>
      problem.includes("output") || problem.includes("input")),
    threePercentGuard: !problems.some((problem) => problem.includes("3% guard")),
  },
  problems,
  pass: problems.length === 0,
};

const serialized = `${JSON.stringify(gate, null, 2)}\n`;
if (outputPath) {
  await writeFile(outputPath, serialized, { flag: "wx" });
} else {
  process.stdout.write(serialized);
}
if (!gate.pass) process.exitCode = 1;
