#!/usr/bin/env node

// Apply the frozen R104/R107 decision rule to the complete R109 native A/B.

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import {
  adjustedElapsedSpeedup,
  adjustedNormalizedThroughput,
} from "./amortized-cold-cost.mjs";

const [nativeArg, constructionArg, outputArg] = process.argv.slice(2);
if (!nativeArg || !constructionArg || !outputArg) {
  throw new Error("usage: r109-native-gate.mjs NATIVE.json CONSTRUCTION.json OUTPUT.json");
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
  candidateWasmSha256: "b335bb69559a7bdfaefe2234c5e84414487efd095c623d10774ba6086efabef8",
  loaderSha256: "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385",
  affinity: "8-15",
  guest: { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" },
  cadence: "public-one-slice-per-turn",
  compileMd5: "24eedf7e06beffd4d3ba1945585588db",
  pythonChecksum: "832040",
  bootMedian: 1.01,
  bootLower95: 1.00,
  bootNormalizedMips: 1.01,
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
if (construction.control?.cpus !== expected.affinity || construction.candidate?.cpus !== expected.affinity) {
  integrityProblems.push("construction affinity differs");
}
if (construction.control?.wasmSha256 !== expected.controlWasmSha256) {
  integrityProblems.push("construction control Wasm differs");
}
if (construction.candidate?.wasmSha256 !== expected.candidateWasmSha256) {
  integrityProblems.push("construction candidate Wasm differs");
}
if (construction.control?.loaderSha256 !== expected.loaderSha256 ||
    construction.candidate?.loaderSha256 !== expected.loaderSha256) {
  integrityProblems.push("construction loader differs");
}

for (const trial of native.trials ?? []) {
  const result = trial.result;
  if (!result) continue;
  const expectedWasm = trial.side === "control"
    ? expected.controlWasmSha256
    : expected.candidateWasmSha256;
  if (result.runtime?.identity?.wasmSha256 !== expectedWasm) {
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
    controlElapsed.push(controlPhase.value);
    candidateElapsed.push(candidatePhase.value);
    controlWork.push(Number(controlPhase.counters.guestInstructions));
    candidateWork.push(Number(candidatePhase.counters.guestInstructions));
  }
  const elapsed = adjustedElapsedSpeedup(controlElapsed, candidateElapsed, debitMs);
  const normalized = adjustedNormalizedThroughput({
    controlElapsedMs: controlElapsed,
    candidateElapsedMs: candidateElapsed,
    controlWork,
    candidateWork,
    debitMs,
  });
  return { phase, controlElapsed, candidateElapsed, controlWork, candidateWork, elapsed, normalized };
}

const observed = Object.fromEntries(expected.rows.map((row) => [row, paired(row)]));
const checks = {
  integrity: integrityProblems.length === 0,
  bootMedian: observed.boot.elapsed.adjusted.median >= expected.bootMedian,
  bootLower95:
    observed.boot.elapsed.adjusted.medianConfidence95[0] >= expected.bootLower95,
  bootNormalizedMips:
    observed.boot.normalized.candidateControlRatios.median >= expected.bootNormalizedMips,
  compileMedian:
    observed.compile.elapsed.adjusted.median >= expected.protectedMedian,
  compileNoEstablishedRegression:
    observed.compile.elapsed.adjusted.medianConfidence95[1] >= expected.protectedUpper95,
  pythonMedian:
    observed.python.elapsed.adjusted.median >= expected.protectedMedian,
  pythonNoEstablishedRegression:
    observed.python.elapsed.adjusted.medianConfidence95[1] >= expected.protectedUpper95,
};
const admitProductGates = Object.values(checks).every(Boolean);
const report = {
  schema: 1,
  experiment: "R109 dense CFG stackifier native gate",
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
