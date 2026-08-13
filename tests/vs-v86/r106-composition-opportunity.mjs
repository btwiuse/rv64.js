#!/usr/bin/env node

// Deterministic pre-timing admission proof for R106. This combines only the
// already-frozen medians and counters from R064 and R105. It is a projection,
// never product-performance evidence.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const [outputPath] = process.argv.slice(2);
if (!outputPath) {
  throw new Error("usage: r106-composition-opportunity.mjs OUTPUT.json");
}

const inputs = Object.freeze({
  publication: {
    path: "target/bench/r064-pending-publication-screen/" +
      "config-ab-2026-08-09T02-08-11-937Z.json",
    sha256: "63d9481cd333f4988279010141207f0572908c2eab84a62b21f54f5376c9270b",
    wasmSha256: "dfc9026b165c63cce40dce031a8fb70b7fab5d9e5a98531294a0f8c8b34ff053",
  },
  scalar: {
    path: "target/bench/r105-integrated-scalar-t0/native-ab/" +
      "config-ab-2026-08-10T06-30-41-984Z.json",
    sha256: "1ee0190a3521ce54ee34f39ccb1b63ddd2a4915491fcef71fd1a79ae86ba964d",
    wasmSha256: "0593567eb75dfe29dd06cf0cabf0747abfa3b217080e2dd2e8c72ca192469a2d",
  },
});
const rows = ["boot", "compile", "python"];
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const phaseFor = (row) => row === "boot" ? "first" : "steady";

function load(spec) {
  const bytes = readFileSync(spec.path);
  if (sha256(bytes) !== spec.sha256) throw new Error(`${spec.path}: identity changed`);
  const report = JSON.parse(bytes);
  if (!report.measurementValid || report.problems?.length) {
    throw new Error(`${spec.path}: source report is not measurement-valid`);
  }
  for (const side of ["control", "candidate"]) {
    if (report.configuration?.wasmBySide?.[side]?.sha256 !== spec.wasmSha256) {
      throw new Error(`${spec.path}: ${side} Wasm identity changed`);
    }
  }
  return report;
}

function interpreterRatio(report, row) {
  const phase = phaseFor(row);
  const ratios = [];
  for (let rep = 1; rep <= report.configuration.reps; rep++) {
    const trial = (side) => report.trials.find((item) =>
      item.row === row && item.rep === rep && item.side === side)?.result?.phases?.[phase];
    const control = trial("control");
    const candidate = trial("candidate");
    if (!control || !candidate) throw new Error(`${row}/rep${rep}: missing pair`);
    ratios.push(
      Number(BigInt(candidate.counters.interpreterInstructions)) /
        Number(BigInt(control.counters.interpreterInstructions)),
    );
  }
  return { raw: ratios, median: median(ratios) };
}

const publication = load(inputs.publication);
const scalar = load(inputs.scalar);
const evidence = {};
for (const row of rows) {
  const publicationSpeedup = publication.aggregates[row].pairedCandidateSpeedup.median;
  const scalarSpeedup = scalar.aggregates[row].pairedCandidateSpeedup.median;
  evidence[row] = {
    publication: {
      pairedSpeedup: publicationSpeedup,
      interpreterCandidateControl: interpreterRatio(publication, row),
    },
    scalar: {
      pairedSpeedup: scalarSpeedup,
      interpreterCandidateControl: interpreterRatio(scalar, row),
    },
    independentMedianProductProjection: publicationSpeedup * scalarSpeedup,
  };
}

const checks = {
  scalarBootMechanism: evidence.boot.scalar.pairedSpeedup >= 1.03,
  scalarExposesCompilePublication:
    evidence.compile.scalar.interpreterCandidateControl.median >= 1.01,
  scalarExposesPythonPublication:
    evidence.python.scalar.interpreterCandidateControl.median >= 1.03,
  publicationImprovesCompile: evidence.compile.publication.pairedSpeedup >= 1.03,
  publicationImprovesPython: evidence.python.publication.pairedSpeedup >= 1.03,
  publicationReducesCompileInterpreter:
    evidence.compile.publication.interpreterCandidateControl.median <= 0.90,
  publicationReducesPythonInterpreter:
    evidence.python.publication.interpreterCandidateControl.median <= 0.70,
  projectedBoot: evidence.boot.independentMedianProductProjection >= 1.02,
  projectedCompile: evidence.compile.independentMedianProductProjection >= 1.02,
  projectedPython: evidence.python.independentMedianProductProjection >= 1.02,
};
const admitOneFrozenComposition = Object.values(checks).every(Boolean);
const report = {
  schema: 1,
  experiment: "R106 balanced scalar/publication pipeline opportunity",
  role: "historical-mechanism-admission-only; no timing credit",
  inputs,
  evidence,
  checks,
  admitOneFrozenComposition,
  decision: admitOneFrozenComposition
    ? "admit-one-indivisible-current-baseline-causal-candidate"
    : "do-not-implement-r106",
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(`R106 opportunity: ${report.decision}`);
for (const row of rows) {
  console.log(
    `${row}: scalar=${evidence[row].scalar.pairedSpeedup.toFixed(4)}x ` +
      `publication=${evidence[row].publication.pairedSpeedup.toFixed(4)}x ` +
      `projection=${evidence[row].independentMedianProductProjection.toFixed(4)}x`,
  );
}
if (!admitOneFrozenComposition) process.exitCode = 1;
