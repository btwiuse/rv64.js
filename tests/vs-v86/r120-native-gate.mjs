#!/usr/bin/env node

// Apply the frozen R104/R107 verified-one-percent rule to R120. Compile is the
// target; Boot and Python are protected. This file is frozen before the
// first construction or native candidate timing sample.

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import {
  adjustedElapsedSpeedup,
  adjustedNormalizedThroughput,
} from "./amortized-cold-cost.mjs";

const [nativeArg, constructionArg, outputArg] = process.argv.slice(2);
if (!nativeArg || !constructionArg || !outputArg) {
  throw new Error("usage: r120-native-gate.mjs NATIVE.json CONSTRUCTION.json OUTPUT.json");
}

const nativePath = resolve(nativeArg);
const constructionPath = resolve(constructionArg);
const outputPath = resolve(outputArg);
const nativeBytes = await readFile(nativePath);
const constructionBytes = await readFile(constructionPath);
const native = JSON.parse(nativeBytes);
const construction = JSON.parse(constructionBytes);
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

const expected = {
  rows: ["boot", "compile", "python"],
  repetitions: 15,
  controlWasmSha256: "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
  candidateWasmSha256: "c36da489ebe3e2f15d960a1ad393b808e9ff285dc099d4988c745e0e81065b32",
  loaderSha256: "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385",
  affinity: "8-15",
  guest: { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" },
  cadence: "public-one-slice-per-turn",
  compileMd5: "24eedf7e06beffd4d3ba1945585588db",
  pythonChecksum: "832040",
  compileMedian: 1.01,
  compileLower95: 1.00,
  compileNormalizedMips: 1.01,
  protectedMedian: 0.99,
  protectedUpper95: 1.00,
};

const integrityProblems = [];
const check = (condition, message) => {
  if (!condition) integrityProblems.push(message);
};

check(native.measurementValid === true, "native report invalid");
check(native.problems?.length === 0, "native report contains problems");
check(construction.measurementValid === true, "construction report invalid");
check(construction.problems?.length === 0, "construction report contains problems");
check(
  JSON.stringify(native.configuration?.rows) === JSON.stringify(expected.rows),
  "row plan differs",
);
check(native.configuration?.reps === expected.repetitions, "native repetitions differ");
check(native.hostCpuAffinity === expected.affinity, "native affinity differs");
check(native.trials?.length === expected.rows.length * expected.repetitions * 2,
  "native trial count differs");

for (const side of ["control", "candidate"]) {
  const expectedWasm = side === "control"
    ? expected.controlWasmSha256
    : expected.candidateWasmSha256;
  const config = native.configuration?.[`${side}Config`] ?? {};
  check(
    Object.keys(config).sort().join(",") === "SCORECARD_V2_REWRITE_WASM",
    `${side} configuration keys differ`,
  );
  check(native.configuration?.wasmBySide?.[side]?.sha256 === expectedWasm,
    `${side} configured Wasm differs`);
}

check(construction.reps === expected.repetitions, "construction repetitions differ");
check(construction.control?.cpus === expected.affinity, "construction control affinity differs");
check(construction.candidate?.cpus === expected.affinity,
  "construction candidate affinity differs");
check(construction.control?.wasmSha256 === expected.controlWasmSha256,
  "construction control Wasm differs");
check(construction.candidate?.wasmSha256 === expected.candidateWasmSha256,
  "construction candidate Wasm differs");
check(construction.control?.loaderSha256 === expected.loaderSha256,
  "construction control loader differs");
check(construction.candidate?.loaderSha256 === expected.loaderSha256,
  "construction candidate loader differs");

for (const trial of native.trials ?? []) {
  const result = trial.result;
  const prefix = `${trial.side}/${trial.row}/rep${trial.rep}`;
  check(!trial.error && result, `${prefix}: successful result missing`);
  if (!result) continue;
  const expectedWasm = trial.side === "control"
    ? expected.controlWasmSha256
    : expected.candidateWasmSha256;
  check(result.runtime?.identity?.wasmSha256 === expectedWasm, `${prefix}: Wasm differs`);
  check(result.runtime?.identity?.loaderSha256 === expected.loaderSha256,
    `${prefix}: loader differs`);
  check(JSON.stringify(result.runtime?.guest) === JSON.stringify(expected.guest),
    `${prefix}: guest differs`);
  check(
    result.runtime?.schedulerCadence?.name === expected.cadence &&
      result.runtime.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
    `${prefix}: cadence differs`,
  );
  check(result.runtime?.requestedPolicy?.name === "production-page",
    `${prefix}: production policy differs`);
  check(result.runtime?.policyProblems?.length === 0, `${prefix}: policy problem`);
  check(result.measurementEligible === false,
    `${prefix}: artifact A/B leg not marked diagnostic`);
  check(BigInt(result.runtime?.jitProof?.generatedInstructions ?? 0) > 0n,
    `${prefix}: generated instructions absent`);
  check(BigInt(result.runtime?.jitProof?.dispatches ?? 0) > 0n,
    `${prefix}: generated dispatches absent`);
  if (trial.row === "compile") {
    check(result.phases?.steady?.md5 === expected.compileMd5,
      `${prefix}: Compile fingerprint differs`);
  }
  if (trial.row === "python") {
    check(result.phases?.steady?.checksum === expected.pythonChecksum,
      `${prefix}: Python fingerprint differs`);
  }
}

const debitMs = construction.accounting?.debitMs;
check(Number.isFinite(debitMs) && debitMs >= 0, "construction debit invalid");

function paired(row) {
  const phase = row === "boot" ? "first" : "steady";
  const controlElapsed = [];
  const candidateElapsed = [];
  const controlWork = [];
  const candidateWork = [];
  const controlGeneratedCoverage = [];
  const candidateGeneratedCoverage = [];
  for (let rep = 1; rep <= expected.repetitions; rep++) {
    const control = native.trials?.find((trial) =>
      trial.row === row && trial.rep === rep && trial.side === "control")?.result;
    const candidate = native.trials?.find((trial) =>
      trial.row === row && trial.rep === rep && trial.side === "candidate")?.result;
    const controlPhase = control?.phases?.[phase];
    const candidatePhase = candidate?.phases?.[phase];
    if (!Number.isFinite(controlPhase?.value) || !Number.isFinite(candidatePhase?.value)) {
      integrityProblems.push(`${row}/rep${rep}: elapsed value missing`);
      continue;
    }
    const controlInstructions = Number(controlPhase.counters?.guestInstructions);
    const candidateInstructions = Number(candidatePhase.counters?.guestInstructions);
    if (!(controlInstructions > 0) || !(candidateInstructions > 0)) {
      integrityProblems.push(`${row}/rep${rep}: work counter missing`);
      continue;
    }
    controlElapsed.push(controlPhase.value);
    candidateElapsed.push(candidatePhase.value);
    controlWork.push(controlInstructions);
    candidateWork.push(candidateInstructions);
    controlGeneratedCoverage.push(
      Number(controlPhase.counters.generatedInstructions) / controlInstructions,
    );
    candidateGeneratedCoverage.push(
      Number(candidatePhase.counters.generatedInstructions) / candidateInstructions,
    );
  }
  if (controlElapsed.length !== expected.repetitions ||
      candidateElapsed.length !== expected.repetitions) {
    integrityProblems.push(`${row}: incomplete paired values`);
  }
  return {
    phase,
    controlElapsed,
    candidateElapsed,
    controlWork,
    candidateWork,
    controlGeneratedCoverage,
    candidateGeneratedCoverage,
    elapsed: adjustedElapsedSpeedup(controlElapsed, candidateElapsed, debitMs),
    normalized: adjustedNormalizedThroughput({
      controlElapsedMs: controlElapsed,
      candidateElapsedMs: candidateElapsed,
      controlWork,
      candidateWork,
      debitMs,
    }),
  };
}

const observed = Object.fromEntries(expected.rows.map((row) => [row, paired(row)]));
const checks = {
  integrity: integrityProblems.length === 0,
  compileMedian:
    observed.compile.elapsed.adjusted.median >= expected.compileMedian,
  compileLower95:
    observed.compile.elapsed.adjusted.medianConfidence95[0] >= expected.compileLower95,
  compileNormalizedMips:
    observed.compile.normalized.candidateControlRatios.median >=
      expected.compileNormalizedMips,
  bootMedian:
    observed.boot.elapsed.adjusted.median >= expected.protectedMedian,
  bootNoEstablishedRegression:
    observed.boot.elapsed.adjusted.medianConfidence95[1] >= expected.protectedUpper95,
  pythonMedian:
    observed.python.elapsed.adjusted.median >= expected.protectedMedian,
  pythonNoEstablishedRegression:
    observed.python.elapsed.adjusted.medianConfidence95[1] >= expected.protectedUpper95,
};
const admitProductGates = Object.values(checks).every(Boolean);
const report = {
  schema: 1,
  experiment: "R120 exact interleaved fused-TLB one-percent native gate",
  frozenRequirements: expected,
  construction: {
    debitMs,
    accounting: construction.accounting,
    path: constructionPath,
    sha256: sha256(constructionBytes),
  },
  native: { path: nativePath, sha256: sha256(nativeBytes) },
  observed,
  checks,
  integrityProblems,
  admitProductGates,
  decision: admitProductGates
    ? "advance-to-browser-wanix-authority"
    : "reject-at-native-gate-and-restore-baseline",
};

await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(JSON.stringify({
  output: outputPath,
  sha256: sha256(await readFile(outputPath)),
  debitMs,
  adjusted: Object.fromEntries(expected.rows.map((row) => [row, {
    median: observed[row].elapsed.adjusted.median,
    confidence95: observed[row].elapsed.adjusted.medianConfidence95,
    normalizedMips: observed[row].normalized.candidateControlRatios.median,
  }])),
  checks,
  decision: report.decision,
}, null, 2));
if (integrityProblems.length) process.exitCode = 1;

