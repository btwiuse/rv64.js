#!/usr/bin/env node

// Verify R090's instrumentation-only feedback inventory. The diagnostic's
// elapsed times are intentionally ignored; only identities and exact dynamic
// counter relationships participate in the decision.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const [reportPath, outputPath] = process.argv.slice(2);
if (!reportPath || !outputPath) {
  throw new Error("usage: r090-feedback-gate.mjs REPORT.json OUTPUT.json");
}

const EXPECTED_WASM = "085adb2da85d1e116bac5f47aa5a617b453b32f66ea39eb825b3e7fd11beaf55";
const EXPECTED_LOADER = "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385";
const EXPECTED_CADENCE = "public-one-slice-per-turn";
const EXPECTED_COMPILE_MD5 = "24eedf7e06beffd4d3ba1945585588db";
const MIN_COMPILE_COVERAGE = 0.50;
const MIN_BOOT_COVERAGE = 0.25;

const bytes = readFileSync(reportPath);
const report = JSON.parse(bytes);
const integrityProblems = [];
const check = (condition, message) => {
  if (!condition) integrityProblems.push(message);
};

check(report.authoritative === false, "proof report unexpectedly authoritative");
check(report.measurementValid === false, "proof report unexpectedly measurement-valid");
check(JSON.stringify(report.problems) === JSON.stringify([
  "rewrite/boot: proof-only run entered measurements",
  "rewrite/compile: proof-only run entered measurements",
]), "proof report has unexpected problems");
check(JSON.stringify(report.configuration?.sides) === JSON.stringify(["rewrite"]),
  "report side is not rewrite-only");
check(JSON.stringify(report.configuration?.rows) === JSON.stringify(["boot", "compile"]),
  "report rows changed");
check(report.configuration?.reps === 1, "report repetition count changed");
check(report.configuration?.schedulerCadence?.name === EXPECTED_CADENCE &&
  report.configuration.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
"top-level cadence changed");
check(report.trials?.length === 2, "expected exactly two proof trials");

const observations = {};
for (const trial of report.trials || []) {
  const result = trial.result;
  const prefix = `${trial.side}/${trial.row}`;
  check(!trial.error && result, `${prefix}: missing successful result`);
  if (!result) continue;
  check(trial.side === "rewrite" && result.side === "rewrite", `${prefix}: wrong side`);
  check(result.measurementEligible === false, `${prefix}: proof result became eligible`);
  check(result.runtime?.identity?.wasmSha256 === EXPECTED_WASM,
    `${prefix}: instrumentation Wasm changed`);
  check(result.runtime?.identity?.loaderSha256 === EXPECTED_LOADER,
    `${prefix}: loader changed`);
  check(result.runtime?.schedulerCadence?.name === EXPECTED_CADENCE &&
    result.runtime.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
  `${prefix}: cadence changed`);
  check(result.runtime?.guest?.linux === "6.12.7" &&
    result.runtime.guest.alpine === "3.24.1" &&
    result.runtime.guest.arch === "riscv64", `${prefix}: modern guest changed`);
  check(result.runtime?.requestedPolicy?.name === "production-page",
    `${prefix}: production policy changed`);
  check(result.runtime?.policyProblems?.length === 0, `${prefix}: policy problem`);
  check(BigInt(result.runtime?.jitProof?.generatedInstructions || 0) > 0n,
    `${prefix}: no generated execution proof`);

  const expectedPhases = trial.row === "boot"
    ? ["first"]
    : ["first", "prime", "steady"];
  check(JSON.stringify(Object.keys(result.phases || {})) === JSON.stringify(expectedPhases),
    `${prefix}: phase set changed`);
  if (trial.row === "compile") {
    for (const [name, phase] of Object.entries(result.phases || {})) {
      check(phase.md5 === EXPECTED_COMPILE_MD5, `${prefix}/${name}: output MD5 changed`);
    }
  }

  for (const [name, phase] of Object.entries(result.phases || {})) {
    const counters = phase.counters || {};
    const dispatches = BigInt(counters.dispatches || 0);
    const blockReturns = BigInt(counters.feedbackBlockReturns || 0);
    const explicit = BigInt(counters.feedbackExplicitMisses || 0);
    const ordinary = BigInt(counters.feedbackOrdinaryChecks || 0);
    const oneBody = BigInt(counters.feedbackOneBody || 0);
    const embedded = BigInt(counters.feedbackEmbeddedPresent || 0);
    const observed = BigInt(counters.feedbackOrdinaryObservations || 0);
    check(dispatches > 0n, `${prefix}/${name}: no generated dispatches`);
    check(ordinary + explicit === blockReturns,
      `${prefix}/${name}: ordinary + explicit != non-region returns`);
    check(embedded <= ordinary, `${prefix}/${name}: embedded exceeds ordinary checks`);
    check(observed <= ordinary, `${prefix}/${name}: observations exceed ordinary checks`);
    check(observed <= oneBody, `${prefix}/${name}: observations exceed one-body returns`);
    const coverage = Number(ordinary) / Number(dispatches);
    observations[`${trial.row}:${name}`] = {
      dispatches: dispatches.toString(),
      nonRegionReturns: blockReturns.toString(),
      explicitMisses: explicit.toString(),
      ordinaryChecks: ordinary.toString(),
      oneBodyReturns: oneBody.toString(),
      embeddedPresent: embedded.toString(),
      ordinaryObservations: observed.toString(),
      ordinaryCheckCoverage: coverage,
    };
  }
}

const bootCoverage = observations["boot:first"]?.ordinaryCheckCoverage ?? 0;
const compileCoverage = observations["compile:steady"]?.ordinaryCheckCoverage ?? 0;
const coveragePass = bootCoverage >= MIN_BOOT_COVERAGE &&
  compileCoverage >= MIN_COMPILE_COVERAGE;

const gate = {
  schema: 1,
  experiment: "R090",
  report: {
    path: reportPath,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  },
  performanceEvidence: false,
  frozenRequirements: {
    minimumBootOrdinaryCheckCoverage: MIN_BOOT_COVERAGE,
    minimumCompileSteadyOrdinaryCheckCoverage: MIN_COMPILE_COVERAGE,
  },
  observations,
  checks: {
    integrity: integrityProblems.length === 0,
    dynamicCoverage: coveragePass,
  },
  integrityProblems,
  admitMetadataPrototype: integrityProblems.length === 0 && coveragePass,
  decision: integrityProblems.length
    ? "invalid-diagnostic"
    : coveragePass
      ? "admit-one-metadata-prototype"
      : "close-feedback-metadata-and-restore-r085",
};

writeFileSync(outputPath, `${JSON.stringify(gate, null, 2)}\n`, { flag: "wx" });
if (integrityProblems.length) process.exitCode = 1;
