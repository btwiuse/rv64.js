#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const args = process.argv.slice(2);
const outputIndex = args.indexOf("--output");
if (args.length !== 4 || outputIndex < 0 || outputIndex === args.length - 1) {
  throw new Error(
    "usage: node tests/vs-v86/r085-promotion-gate.mjs " +
      "R080.json R085.json --output GATE.json",
  );
}
const positional = args.filter((_, index) => index !== outputIndex && index !== outputIndex + 1);
if (positional.length !== 2) throw new Error("expected R080 and R085 report paths");
const [controlPath, candidatePath] = positional.map((path) => resolve(path));
const outputPath = resolve(args[outputIndex + 1]);
const controlBytes = readFileSync(controlPath);
const candidateBytes = readFileSync(candidatePath);
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const control = JSON.parse(controlBytes);
const candidate = JSON.parse(candidateBytes);
const problems = [];
const rows = [
  "alu", "mixed", "boot", "python", "compile", "numeric", "string",
  "bitfield", "fpemul", "fourier", "assignment", "idea", "huffman",
];
const sides = ["rewrite", "legacy", "v86"];
const expectedConfiguration = {
  sides,
  rows,
  reps: 3,
  phases: ["first", "prime", "steady"],
  scoredPhase: "steady (first for boot)",
  v86ExecutionPreflight: true,
  timeoutMs: 900000,
  rewritePolicy: "production",
  hostProbe: {
    algorithm: "pbkdf2-sha256",
    iterations: 100000,
    samples: 7,
    statistic: "minimum",
    maximumSpread: 1.25,
  },
};
const identities = {
  controlReport: "09ff8ffa27640d6992500c024fccb5f6438bb84967b6e70df1381dfbec2f2378",
  candidateReport: "d733df2124a7388876f6566db71b6f67bf7cdc4dccc2ece5e1df2903c27d7479",
  loader: "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385",
  controlWasm: "e5415db83b27b32a1f525af2aa19e93539332a274068e389a1e28ebba41d8095",
  candidateWasm: "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010",
};
const thresholds = {
  matchFloor: 0.95,
  minimumTargetSpeedup: 1.03,
  minimumGuardedSpeedup: 1 / 1.03,
};
const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);

if (sha256(controlBytes) !== identities.controlReport) problems.push("R080 report changed");
if (sha256(candidateBytes) !== identities.candidateReport) problems.push("R085 report changed");
for (const [name, report] of [["R080", control], ["R085", candidate]]) {
  if (report.schema !== 2) problems.push(`${name} schema=${report.schema}`);
  if (report.authoritative !== true) problems.push(`${name} is not authoritative`);
  if (report.measurementValid !== true) problems.push(`${name} measurement invalid`);
  if (!Array.isArray(report.problems) || report.problems.length) {
    problems.push(`${name} problems=${JSON.stringify(report.problems)}`);
  }
  if (!same(report.configuration, expectedConfiguration)) {
    problems.push(`${name} configuration changed`);
  }
  if (report.trials?.length !== rows.length * sides.length * 3) {
    problems.push(`${name} trials=${report.trials?.length}`);
  }
  if (report.provenance?.cpuAffinity !== "8-15") {
    problems.push(`${name} CPU affinity=${report.provenance?.cpuAffinity}`);
  }
  const proof = report.v86ExecutionPreflight?.result?.runtime?.jitProof?.executionProbe;
  if (!(proof?.hits > 0) || !(proof?.distinctHitIndexes > 0)) {
    problems.push(`${name} v86 execution proof missing`);
  }
}

for (const [name, report, wasm] of [
  ["R080", control, identities.controlWasm],
  ["R085", candidate, identities.candidateWasm],
]) {
  const rewriteTrials = report.trials.filter((trial) => trial.side === "rewrite");
  if (rewriteTrials.length !== rows.length * 3) problems.push(`${name} rewrite trial count`);
  for (const trial of rewriteTrials) {
    const identity = trial.result?.runtime?.identity;
    if (identity?.loaderSha256 !== identities.loader || identity?.wasmSha256 !== wasm) {
      problems.push(`${name} ${trial.row}/rep${trial.rep} runtime identity changed`);
    }
    if (!same(trial.result?.runtime?.guest, {
      linux: "6.12.7", alpine: "3.24.1", arch: "riscv64",
    })) problems.push(`${name} ${trial.row}/rep${trial.rep} guest changed`);
    const proof = trial.result?.runtime?.jitProof;
    if (!(BigInt(proof?.generatedInstructions ?? "0") > 0n) ||
        !(BigInt(proof?.dispatches ?? "0") > 0n)) {
      problems.push(`${name} ${trial.row}/rep${trial.rep} generated execution missing`);
    }
  }
}

const aggregateByKey = (report) => new Map(report.aggregates.map((row) => [row.key, row]));
const controlRows = aggregateByKey(control);
const candidateRows = aggregateByKey(candidate);
const rowResults = {};
let controlLegacyMatches = 0;
let controlV86Matches = 0;
let candidateLegacyMatches = 0;
let candidateV86Matches = 0;
for (const key of rows) {
  const before = controlRows.get(key);
  const after = candidateRows.get(key);
  if (!before || !after) {
    problems.push(`${key} aggregate missing`);
    continue;
  }
  if (before.kind !== "duration" || after.kind !== "duration") {
    problems.push(`${key} kind changed`);
  }
  const controlMs = before.sides.rewrite.median;
  const candidateMs = after.sides.rewrite.median;
  const speedup = controlMs / candidateMs;
  const guarded = speedup >= thresholds.minimumGuardedSpeedup;
  const target = key === "boot" || key === "compile";
  const targetPassed = !target || speedup >= thresholds.minimumTargetSpeedup;
  if (!guarded) problems.push(`${key} speedup=${speedup} below no-regression guard`);
  if (!targetPassed) problems.push(`${key} speedup=${speedup} below target gate`);
  if (before.rewriteVsLegacy >= thresholds.matchFloor) controlLegacyMatches++;
  if (before.rewriteVsV86 >= thresholds.matchFloor) controlV86Matches++;
  if (after.rewriteVsLegacy >= thresholds.matchFloor) candidateLegacyMatches++;
  if (after.rewriteVsV86 >= thresholds.matchFloor) candidateV86Matches++;
  rowResults[key] = {
    controlRewriteMedian: controlMs,
    candidateRewriteMedian: candidateMs,
    candidateSpeedup: speedup,
    rewriteVsLegacy: after.rewriteVsLegacy,
    rewriteVsV86: after.rewriteVsV86,
    guarded,
    target,
    targetPassed,
  };
}
if (candidateLegacyMatches !== rows.length) {
  problems.push(`R085 legacy parity=${candidateLegacyMatches}/${rows.length}`);
}
if (candidateV86Matches < controlV86Matches) {
  problems.push(`R085 v86 parity regressed ${controlV86Matches} to ${candidateV86Matches}`);
}
if (candidateV86Matches !== 11) {
  problems.push(`R085 v86 parity=${candidateV86Matches}/13 (expected cumulative 11/13)`);
}

const result = {
  schema: 1,
  experiment: "R085 fast JIT-state hash promotion gate",
  evaluated: new Date().toISOString(),
  inputs: {
    control: { path: controlPath, sha256: sha256(controlBytes) },
    candidate: { path: candidatePath, sha256: sha256(candidateBytes) },
  },
  identities,
  thresholds,
  parity: {
    control: { legacy: controlLegacyMatches, v86: controlV86Matches },
    candidate: { legacy: candidateLegacyMatches, v86: candidateV86Matches },
  },
  rows: rowResults,
  gatePassed: problems.length === 0,
  goalMet: candidate.goalMet,
  problems,
};
writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`, { flag: "wx" });
for (const key of rows) {
  const row = rowResults[key];
  console.log(`${key}: ${row.candidateSpeedup.toFixed(3)}x; ` +
    `legacy=${row.rewriteVsLegacy.toFixed(3)}x v86=${row.rewriteVsV86.toFixed(3)}x`);
}
console.log(`parity: legacy ${candidateLegacyMatches}/13, v86 ${candidateV86Matches}/13`);
if (problems.length) {
  console.error(`R085_PROMOTION_GATE_FAIL: ${problems.join("; ")}`);
  process.exitCode = 1;
} else {
  console.log("R085_PROMOTION_GATE_PASS");
}
