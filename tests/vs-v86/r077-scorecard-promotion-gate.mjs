#!/usr/bin/env node

import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  evaluateR077ScorecardPromotion,
  R077_BASELINE_SHA256,
} from "./r077-scorecard-promotion-lib.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const baselinePath = resolve(
  root,
  "target/bench/r054-final-three-way-rerun/scorecard-v2-2026-08-08T23-01-30-777Z.json",
);

function usage() {
  return "usage: node tests/vs-v86/r077-scorecard-promotion-gate.mjs " +
    "CANDIDATE.json --output PROMOTION.json";
}

const args = process.argv.slice(2);
const outputIndex = args.indexOf("--output");
if (args.length !== 3 || outputIndex < 0 || outputIndex === args.length - 1) {
  throw new Error(usage());
}
const positional = args.filter((_, index) => index !== outputIndex && index !== outputIndex + 1);
if (positional.length !== 1) throw new Error(usage());
const candidatePath = resolve(positional[0]);
const outputPath = resolve(args[outputIndex + 1]);

const [baselineBytes, candidateBytes] = await Promise.all([
  readFile(baselinePath),
  readFile(candidatePath),
]);
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const baselineSha256 = digest(baselineBytes);
const candidateSha256 = digest(candidateBytes);
if (baselineSha256 !== R077_BASELINE_SHA256) {
  throw new Error(
    `frozen R054 baseline SHA-256 changed: ${baselineSha256} ` +
      `(expected ${R077_BASELINE_SHA256})`,
  );
}

const report = evaluateR077ScorecardPromotion({
  baseline: JSON.parse(baselineBytes),
  candidate: JSON.parse(candidateBytes),
  baselineSha256,
  candidateSha256,
});
report.inputs = {
  baseline: { path: baselinePath, sha256: baselineSha256 },
  candidate: { path: candidatePath, sha256: candidateSha256 },
};
await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);

console.log(
  `matches: copy/v86 ${report.counts.v86Matches}/13; ` +
    `legacy ${report.counts.legacyMatches}/13`,
);
console.log(
  `Boot: ${report.boot.candidateMedian?.toFixed(3)} ms; ` +
    `${report.boot.speedup?.toFixed(3)}x versus R054`,
);
console.log(`promotion report: ${outputPath}`);
if (report.gatePassed) {
  console.log("R077_SCORECARD_PROMOTION_PASS");
} else {
  console.error(`R077_SCORECARD_PROMOTION_FAIL: ${report.problems.join("; ")}`);
  process.exitCode = 1;
}
