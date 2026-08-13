#!/usr/bin/env node

// Verify R088's proof-only corrected-cadence profile and its frozen admission
// rule. Profile durations are deliberately not read as performance evidence.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const [reportPath, analysisPath, outputPath] = process.argv.slice(2);
if (!reportPath || !analysisPath || !outputPath) {
  throw new Error("usage: r088-profile-gate.mjs REPORT.json ANALYSIS.json OUTPUT.json");
}

const EXPECTED_WASM = "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010";
const EXPECTED_LOADER = "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385";
const EXPECTED_CLASSIFIER = "efce26fcf275550efaa26c2d95d22e4e71d2243b189db2c9d2245b92e6181be5";
const EXPECTED_CADENCE = "public-one-slice-per-turn";
const LOCAL_SPEEDUP = 1.494;
const MIN_BOOT_FRACTION = 0.09;
const MIN_PROJECTED_SPEEDUP = 1.03;

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const reportBytes = readFileSync(reportPath);
const analysisBytes = readFileSync(analysisPath);
const report = JSON.parse(reportBytes);
const analysis = JSON.parse(analysisBytes);
const problems = [];
const check = (condition, message) => {
  if (!condition) problems.push(message);
};

check(report.authoritative === false, "proof report unexpectedly authoritative");
check(report.measurementValid === false, "proof report unexpectedly measurement-valid");
check(
  JSON.stringify(report.problems) === JSON.stringify([
    "rewrite/boot: proof-only run entered measurements",
    "rewrite/compile: proof-only run entered measurements",
  ]),
  "proof report has unexpected problems",
);
check(report.configuration?.sides?.length === 1 &&
  report.configuration.sides[0] === "rewrite", "report side is not rewrite-only");
check(JSON.stringify(report.configuration?.rows) === JSON.stringify(["boot", "compile"]),
  "report rows are not exact boot,compile");
check(report.configuration?.reps === 1, "report repetition count changed");
check(report.configuration?.schedulerCadence?.name === EXPECTED_CADENCE,
  "top-level cadence changed");
check(report.configuration?.schedulerCadence?.rv64SlicesPerEventLoopTurn === 1,
  "top-level cadence does not use one slice");
check(report.trials?.length === 2, "expected exactly two proof trials");

for (const trial of report.trials || []) {
  const result = trial.result;
  const prefix = `${trial.side}/${trial.row}`;
  check(trial.side === "rewrite", `${prefix}: wrong side`);
  check(result?.measurementEligible === false, `${prefix}: proof result became eligible`);
  check(result?.runtime?.schedulerCadence?.name === EXPECTED_CADENCE,
    `${prefix}: runtime cadence changed`);
  check(result?.runtime?.schedulerCadence?.rv64SlicesPerEventLoopTurn === 1,
    `${prefix}: runtime cadence does not use one slice`);
  check(result?.runtime?.identity?.wasmSha256 === EXPECTED_WASM,
    `${prefix}: runtime Wasm changed`);
  check(result?.runtime?.identity?.loaderSha256 === EXPECTED_LOADER,
    `${prefix}: loader changed`);
  check(result?.runtime?.guest?.linux === "6.12.7" &&
    result.runtime.guest.alpine === "3.24.1" &&
    result.runtime.guest.arch === "riscv64", `${prefix}: modern guest changed`);
  check(result?.runtime?.requestedPolicy?.name === "production-page",
    `${prefix}: requested policy changed`);
  check(result?.runtime?.policyProblems?.length === 0, `${prefix}: policy problem`);
  check(BigInt(result?.runtime?.jitProof?.generatedInstructions || 0) > 0n,
    `${prefix}: no generated execution proof`);
  const expectedPhases = trial.row === "boot"
    ? ["first"]
    : ["first", "prime", "steady"];
  check(JSON.stringify(Object.keys(result?.phases || {})) === JSON.stringify(expectedPhases),
    `${prefix}: phase set changed`);
  for (const phase of Object.values(result?.phases || {})) {
    check(phase?.engineProfile?.intervalMicroseconds === 250,
      `${prefix}: profiler interval changed`);
    check((phase?.engineProfile?.samples || 0) > 0, `${prefix}: empty CPU profile`);
  }
}

check(analysis.experiment === "R088", "analysis experiment identity changed");
check(analysis.performanceEvidence === false, "analysis marked as performance evidence");
check(analysis.schedulerCadence?.name === EXPECTED_CADENCE,
  "analysis cadence changed");
check(analysis.schedulerCadence?.rv64SlicesPerEventLoopTurn === 1,
  "analysis cadence does not use one slice");
check(analysis.classifier?.sha256 === EXPECTED_CLASSIFIER,
  "closure classifier identity changed");
check(analysis.profiles?.length === 4, "expected four phase profiles");

const expectedProfiles = [
  "rewrite-boot-first.cpuprofile",
  "rewrite-compile-first.cpuprofile",
  "rewrite-compile-prime.cpuprofile",
  "rewrite-compile-steady.cpuprofile",
];
check(JSON.stringify((analysis.profiles || []).map((profile) => profile.file)) ===
  JSON.stringify(expectedProfiles), "profile set or order changed");
for (const profile of analysis.profiles || []) {
  check(profile.missingSamples === 0, `${profile.file}: missing samples`);
  if (profile.file === "rewrite-boot-first.cpuprofile" ||
      profile.file === "rewrite-compile-steady.cpuprofile") {
    check(profile.explained95?.fraction >= 0.95,
      `${profile.file}: operation families explain less than 95%`);
  }
}

const boot = (analysis.profiles || []).find((profile) =>
  profile.file === "rewrite-boot-first.cpuprofile");
const reentry = boot?.families?.find((family) =>
  family.family === "interpreter loop with exact generated re-entry");
const bootFraction = reentry?.fraction ?? 0;
const projectedSpeedup = 1 / (1 - bootFraction + bootFraction / LOCAL_SPEEDUP);
check(bootFraction >= MIN_BOOT_FRACTION,
  `Boot exact re-entry fraction ${bootFraction} is below ${MIN_BOOT_FRACTION}`);
check(projectedSpeedup >= MIN_PROJECTED_SPEEDUP,
  `projected Boot speedup ${projectedSpeedup} is below ${MIN_PROJECTED_SPEEDUP}`);

const gate = {
  schema: 1,
  experiment: "R088",
  report: { path: reportPath, sha256: sha256(reportBytes) },
  analysis: { path: analysisPath, sha256: sha256(analysisBytes) },
  performanceEvidence: false,
  cadence: EXPECTED_CADENCE,
  attribution: {
    bootExactReentryFraction: bootFraction,
    frozenLocalSpeedup: LOCAL_SPEEDUP,
    projectedBootSpeedup: projectedSpeedup,
    minimumFraction: MIN_BOOT_FRACTION,
    minimumProjectedSpeedup: MIN_PROJECTED_SPEEDUP,
  },
  admitExactReentryPrototype: problems.length === 0,
  pass: problems.length === 0,
  problems,
};
writeFileSync(outputPath, `${JSON.stringify(gate, null, 2)}\n`, { flag: "wx" });
if (problems.length) process.exitCode = 1;
