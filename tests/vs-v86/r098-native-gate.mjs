#!/usr/bin/env node

// Verify R098's frozen five-pair native product gate and distinguish a valid
// rejection from an invalid measurement.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const [reportPath, outputPath] = process.argv.slice(2);
if (!reportPath || !outputPath) {
  throw new Error("usage: r098-native-gate.mjs REPORT.json OUTPUT.json");
}

const CONTROL = "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d";
const CANDIDATE = "b2e2831bb7851f6ce0c2cd58fba6a9f6f78e77a9e7c428192ed3270968553453";
const LOADER = "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385";
const CADENCE = "public-one-slice-per-turn";
const REPS = 5;
const MIN_BOOT_MEDIAN = 1.03;
const MIN_BOOT_LOWER = 1.00;
const MAX_GUARD_ELAPSED_RATIO = 1.03;
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
check(
  JSON.stringify(report.configuration?.rows) === JSON.stringify(["boot", "compile", "python"]),
  "row set or order changed",
);
check(report.configuration?.reps === REPS, "repetition count changed");
check(report.configuration?.wasmBySide?.control?.sha256 === CONTROL, "control artifact changed");
check(
  report.configuration?.wasmBySide?.candidate?.sha256 === CANDIDATE,
  "candidate artifact changed",
);
check(report.hostProbeSpread <= MAX_HOST_SPREAD, "host probe spread exceeds 1.10x");
check(report.hostCpuAffinity === "8-15", "CPU affinity changed");
check(report.trials?.length === 3 * 2 * REPS, "expected exactly 30 retained trials");

for (const trial of report.trials || []) {
  const result = trial.result;
  const prefix = `${trial.side}/${trial.row}/rep${trial.rep}`;
  const expectedHash = trial.side === "control" ? CONTROL : CANDIDATE;
  check(!trial.error && result, `${prefix}: missing successful result`);
  if (!result) continue;
  check(result.runtime?.identity?.wasmSha256 === expectedHash, `${prefix}: runtime artifact changed`);
  check(result.runtime?.identity?.loaderSha256 === LOADER, `${prefix}: loader changed`);
  check(
    result.runtime?.schedulerCadence?.name === CADENCE &&
      result.runtime.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
    `${prefix}: scheduler cadence changed`,
  );
  check(
    result.runtime?.guest?.linux === "6.12.7" &&
      result.runtime.guest.alpine === "3.24.1" &&
      result.runtime.guest.arch === "riscv64",
    `${prefix}: modern guest changed`,
  );
  check(
    result.runtime?.requestedPolicy?.name === "production-page",
    `${prefix}: production policy changed`,
  );
  check(result.runtime?.policyProblems?.length === 0, `${prefix}: policy problem`);
  check(
    BigInt(result.runtime?.jitProof?.generatedInstructions || 0) > 0n,
    `${prefix}: generated execution not proved`,
  );
  check(trial.hostBeforeMs > 0 && trial.hostAfterMs > 0, `${prefix}: host probes missing`);
}

for (const row of ["boot", "compile", "python"]) {
  const aggregate = report.aggregates?.[row];
  check(aggregate, `${row}: aggregate missing`);
  if (!aggregate) continue;
  check(
    aggregate.control?.values?.length === REPS && aggregate.candidate?.values?.length === REPS,
    `${row}: incomplete side samples`,
  );
  check(
    aggregate.pairedCandidateSpeedup?.raw?.length === REPS,
    `${row}: incomplete paired samples`,
  );
  check(
    aggregate.control?.spread <= report.configuration.maximumSampleSpread &&
      aggregate.candidate?.spread <= report.configuration.maximumSampleSpread,
    `${row}: sample spread exceeds frozen scorecard limit`,
  );
  check(
    JSON.stringify(aggregate.control?.inputs) === JSON.stringify(aggregate.candidate?.inputs),
    `${row}: input identities differ`,
  );
  check(
    JSON.stringify(aggregate.control?.fingerprints) ===
      JSON.stringify(aggregate.candidate?.fingerprints),
    `${row}: output fingerprints differ`,
  );
}

const boot = report.aggregates?.boot;
const compile = report.aggregates?.compile;
const python = report.aggregates?.python;
const bootMedian = boot?.pairedCandidateSpeedup?.median ?? 0;
const bootLower = boot?.pairedCandidateSpeedup?.medianConfidence95?.[0] ?? 0;
const bootMedianPass = bootMedian >= MIN_BOOT_MEDIAN;
const bootLowerPass = bootLower >= MIN_BOOT_LOWER;
const compileGuardPass =
  (compile?.candidate.median / compile?.control.median) <= MAX_GUARD_ELAPSED_RATIO;
const pythonGuardPass =
  (python?.candidate.median / python?.control.median) <= MAX_GUARD_ELAPSED_RATIO;
const admitBrowser =
  integrityProblems.length === 0 &&
  bootMedianPass &&
  bootLowerPass &&
  compileGuardPass &&
  pythonGuardPass;

const gate = {
  schema: 1,
  experiment: "R098",
  mechanism: "absolute-interrupt-poll-deadline",
  report: {
    path: reportPath,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  },
  frozenRequirements: {
    bootMedianSpeedup: MIN_BOOT_MEDIAN,
    bootLowerBound: MIN_BOOT_LOWER,
    maximumGuardElapsedRatio: MAX_GUARD_ELAPSED_RATIO,
    maximumHostSpread: MAX_HOST_SPREAD,
  },
  observed: {
    hostProbeSpread: report.hostProbeSpread,
    boot: {
      controlMedianMs: boot?.control.median,
      candidateMedianMs: boot?.candidate.median,
      pairedSpeedup: bootMedian,
      pairedConfidence95: boot?.pairedCandidateSpeedup?.medianConfidence95,
    },
    compile: {
      controlMedianMs: compile?.control.median,
      candidateMedianMs: compile?.candidate.median,
      pairedSpeedup: compile?.pairedCandidateSpeedup?.median,
      pairedConfidence95: compile?.pairedCandidateSpeedup?.medianConfidence95,
    },
    python: {
      controlMedianMs: python?.control.median,
      candidateMedianMs: python?.candidate.median,
      pairedSpeedup: python?.pairedCandidateSpeedup?.median,
      pairedConfidence95: python?.pairedCandidateSpeedup?.medianConfidence95,
    },
  },
  checks: {
    integrity: integrityProblems.length === 0,
    bootMedian: bootMedianPass,
    bootLowerBound: bootLowerPass,
    compileGuard: compileGuardPass,
    pythonGuard: pythonGuardPass,
  },
  integrityProblems,
  admitBrowser,
  decision: admitBrowser ? "escalate-to-browser" : "reject-and-restore-r085-equivalent",
};
writeFileSync(outputPath, `${JSON.stringify(gate, null, 2)}\n`, { flag: "wx" });
if (integrityProblems.length) process.exitCode = 1;
