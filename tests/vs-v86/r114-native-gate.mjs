#!/usr/bin/env node

// Apply the frozen R104/R107 decision rule to the R114 same-artifact causal
// A/B. Compile is the target; Boot and Python are protected.

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import {
  adjustedElapsedSpeedup,
  adjustedNormalizedThroughput,
} from "./amortized-cold-cost.mjs";

const [nativeArg, constructionArg, outputArg] = process.argv.slice(2);
if (!nativeArg || !constructionArg || !outputArg) {
  throw new Error("usage: r114-native-gate.mjs NATIVE.json CONSTRUCTION.json OUTPUT.json");
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
  causalWasmSha256: "471a34059e4b7f6028b3d52c4a0dd84d6919e4dd147c160dfb26fe00295f0ebf",
  baselineWasmSha256: "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
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
if (!native.measurementValid || native.problems?.length) integrityProblems.push("native report invalid");
if (!construction.measurementValid || construction.problems?.length) {
  integrityProblems.push("construction report invalid");
}
if (JSON.stringify(native.configuration?.rows) !== JSON.stringify(expected.rows)) {
  integrityProblems.push("row plan differs");
}
if (native.configuration?.reps !== expected.repetitions) integrityProblems.push("repetitions differ");
if (native.hostCpuAffinity !== expected.affinity) integrityProblems.push("native affinity differs");
const expectedConfigs = {
  control: "0",
  candidate: "1",
};
for (const side of ["control", "candidate"]) {
  const config = native.configuration?.[`${side}Config`] ?? {};
  if (config.SCORECARD_V2_LAZY_INTERNAL_PC !== expectedConfigs[side]) {
    integrityProblems.push(`${side} lazy-PC configuration differs`);
  }
  if (Object.keys(config).sort().join(",") !==
      ["SCORECARD_V2_LAZY_INTERNAL_PC", "SCORECARD_V2_REWRITE_WASM"].sort().join(",")) {
    integrityProblems.push(`${side} configuration keys differ`);
  }
  if (native.configuration?.wasmBySide?.[side]?.sha256 !== expected.causalWasmSha256) {
    integrityProblems.push(`${side} configured Wasm differs`);
  }
}
if (construction.reps !== expected.repetitions) integrityProblems.push("construction repetitions differ");
if (construction.control?.cpus !== expected.affinity || construction.candidate?.cpus !== expected.affinity) {
  integrityProblems.push("construction affinity differs");
}
if (construction.control?.wasmSha256 !== expected.causalWasmSha256 ||
    construction.candidate?.wasmSha256 !== expected.causalWasmSha256) {
  integrityProblems.push("construction did not use the same causal Wasm artifact");
}
if (construction.control?.loaderSha256 !== expected.loaderSha256 ||
    construction.candidate?.loaderSha256 !== expected.loaderSha256) {
  integrityProblems.push("construction loader differs");
}

for (const trial of native.trials ?? []) {
  const result = trial.result;
  if (!result) continue;
  if (result.runtime?.identity?.wasmSha256 !== expected.causalWasmSha256) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: Wasm differs`);
  }
  if (result.runtime?.identity?.loaderSha256 !== expected.loaderSha256) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: loader differs`);
  }
  if (JSON.stringify(result.runtime?.guest) !== JSON.stringify(expected.guest)) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: guest differs`);
  }
  if (result.runtime?.schedulerCadence?.name !== expected.cadence) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: cadence differs`);
  }
  if (result.runtime?.policyProblems?.length) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: production policy problem`);
  }
  if (result.runtime?.diagnostic?.lazyInternalPc !== Number(expectedConfigs[trial.side])) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: causal switch proof differs`);
  }
  if (result.measurementEligible !== false) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: causal leg not labeled diagnostic`);
  }
  if (!(BigInt(result.runtime?.jitProof?.generatedInstructions ?? 0) > 0n) ||
      !(BigInt(result.runtime?.jitProof?.dispatches ?? 0) > 0n)) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: generated execution absent`);
  }
  if (trial.row === "compile" && result.phases?.steady?.md5 !== expected.compileMd5) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: compile fingerprint differs`);
  }
  if (trial.row === "python" && result.phases?.steady?.checksum !== expected.pythonChecksum) {
    integrityProblems.push(`${trial.row}/${trial.side}/${trial.rep}: Python fingerprint differs`);
  }
}
if (native.trials?.length !== expected.rows.length * expected.repetitions * 2) {
  integrityProblems.push("native trial count differs");
}

const debitMs = construction.accounting?.debitMs;
if (!Number.isFinite(debitMs) || debitMs < 0) integrityProblems.push("construction debit invalid");

function paired(row) {
  const phase = row === "boot" ? "first" : "steady";
  const controlElapsed = [];
  const candidateElapsed = [];
  const controlWork = [];
  const candidateWork = [];
  const controlGeneratedCoverage = [];
  const candidateGeneratedCoverage = [];
  for (let rep = 1; rep <= expected.repetitions; rep++) {
    const control = native.trials.find((trial) =>
      trial.row === row && trial.rep === rep && trial.side === "control")?.result;
    const candidate = native.trials.find((trial) =>
      trial.row === row && trial.rep === rep && trial.side === "candidate")?.result;
    const controlPhase = control?.phases?.[phase];
    const candidatePhase = candidate?.phases?.[phase];
    if (!Number.isFinite(controlPhase?.value) || !Number.isFinite(candidatePhase?.value)) {
      integrityProblems.push(`${row}/rep${rep}: elapsed value missing`);
      continue;
    }
    const controlInstructions = Number(controlPhase.counters.guestInstructions);
    const candidateInstructions = Number(candidatePhase.counters.guestInstructions);
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
  const elapsed = adjustedElapsedSpeedup(controlElapsed, candidateElapsed, debitMs);
  const normalized = adjustedNormalizedThroughput({
    controlElapsedMs: controlElapsed,
    candidateElapsedMs: candidateElapsed,
    controlWork,
    candidateWork,
    debitMs,
  });
  return {
    phase,
    controlElapsed,
    candidateElapsed,
    controlWork,
    candidateWork,
    controlGeneratedCoverage,
    candidateGeneratedCoverage,
    elapsed,
    normalized,
  };
}

const observed = Object.fromEntries(expected.rows.map((row) => [row, paired(row)]));
const checks = {
  integrity: integrityProblems.length === 0,
  compileMedian: observed.compile.elapsed.adjusted.median >= expected.compileMedian,
  compileLower95:
    observed.compile.elapsed.adjusted.medianConfidence95[0] >= expected.compileLower95,
  compileNormalizedMips:
    observed.compile.normalized.candidateControlRatios.median >= expected.compileNormalizedMips,
  bootMedian: observed.boot.elapsed.adjusted.median >= expected.protectedMedian,
  bootNoEstablishedRegression:
    observed.boot.elapsed.adjusted.medianConfidence95[1] >= expected.protectedUpper95,
  pythonMedian: observed.python.elapsed.adjusted.median >= expected.protectedMedian,
  pythonNoEstablishedRegression:
    observed.python.elapsed.adjusted.medianConfidence95[1] >= expected.protectedUpper95,
};
const admitProductGates = Object.values(checks).every(Boolean);
const report = {
  schema: 1,
  experiment: "R114 lazy internal-PC native gate",
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
    ? "advance-to-clean-product-browser-wanix-authority"
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
