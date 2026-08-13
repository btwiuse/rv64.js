#!/usr/bin/env node

// Verify R087's corrected-cadence authoritative baseline against the accepted
// R085 scorecard without treating the harness effect as product-code speedup.

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const [currentPath, acceptedPath, outputPath] = process.argv.slice(2);
if (!currentPath || !acceptedPath) {
  throw new Error(
    "usage: r087-authoritative-gate.mjs CURRENT.json R085.json [OUTPUT.json]",
  );
}

const EXPECTED_WASM =
  "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010";
const EXPECTED_ROWS = [
  "alu", "mixed", "boot", "python", "compile", "numeric", "string",
  "bitfield", "fpemul", "fourier", "assignment", "idea", "huffman",
];
const MATCH_FLOOR = 0.9;

const [currentBytes, acceptedBytes] = await Promise.all([
  readFile(currentPath),
  readFile(acceptedPath),
]);
const current = JSON.parse(currentBytes);
const accepted = JSON.parse(acceptedBytes);
const problems = [];
const require_ = (condition, message) => {
  if (!condition) problems.push(message);
};

require_(current.authoritative === true, "current report is not authoritative");
require_(current.measurementValid === true, "current report is not measurement-valid");
require_(Array.isArray(current.problems) && current.problems.length === 0,
  "current report has problems");
require_(current.configuration?.reps === 3, "current report does not use three repetitions");
require_(JSON.stringify(current.configuration?.rows) === JSON.stringify(EXPECTED_ROWS),
  "current row set/order differs");
require_(JSON.stringify(current.configuration?.sides) ===
  JSON.stringify(["rewrite", "legacy", "v86"]), "current side set/order differs");
require_(current.configuration?.schedulerCadence?.name ===
  "public-one-slice-per-turn" &&
  current.configuration.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
  "top-level cadence is not the corrected public cadence");
require_(current.provenance?.cpuAffinity === "8-15", "CPU affinity differs");

const trials = Array.isArray(current.trials) ? current.trials : [];
require_(trials.length === 117, `expected 117 trials, found ${trials.length}`);
for (const trial of trials) {
  const prefix = `${trial.side}/${trial.row}/rep${trial.rep}`;
  require_(!trial.error && trial.result, `${prefix} failed`);
  if (!trial.result) continue;
  const runtime = trial.result.runtime || {};
  require_(trial.result.measurementEligible === true, `${prefix} is ineligible`);
  require_(runtime.schedulerCadence?.name === "public-one-slice-per-turn" &&
    runtime.schedulerCadence?.rv64SlicesPerEventLoopTurn === 1,
    `${prefix} cadence mismatch`);
  require_(runtime.diagnostic === null || runtime.diagnostic === undefined,
    `${prefix} has a diagnostic override`);
  require_(runtime.jitProof?.enabledRequested === true, `${prefix} JIT was not requested`);
  if (trial.side !== "v86") {
    require_(Array.isArray(runtime.policyProblems) && runtime.policyProblems.length === 0,
      `${prefix} policy proof failed`);
    require_(BigInt(runtime.jitProof?.generatedInstructions || 0) > 0n &&
      BigInt(runtime.jitProof?.dispatches || 0) > 0n,
      `${prefix} generated execution proof failed`);
  } else {
    require_(runtime.jitProof?.disabled === 0 && runtime.jitProof?.finalizedModules > 0,
      `${prefix} v86 generated-module proof failed`);
  }
  if (trial.side === "rewrite") {
    require_(runtime.identity?.wasmSha256 === EXPECTED_WASM,
      `${prefix} rewrite Wasm differs from R085`);
  }
  const expectedArch = trial.side === "v86" ? "i686" : "riscv64";
  require_(runtime.guest?.linux === "6.12.7" && runtime.guest?.alpine === "3.24.1" &&
    runtime.guest?.arch === expectedArch, `${prefix} modern guest mismatch`);
}

const aggregates = new Map((current.aggregates || []).map((row) => [row.key, row]));
require_(aggregates.size === EXPECTED_ROWS.length, "aggregate row count differs");
const legacyLosses = [];
const v86Losses = [];
for (const row of EXPECTED_ROWS) {
  const aggregate = aggregates.get(row);
  require_(aggregate, `${row} aggregate missing`);
  if (!aggregate) continue;
  if (!(aggregate.rewriteVsLegacy >= MATCH_FLOOR)) legacyLosses.push(row);
  if (!(aggregate.rewriteVsV86 >= MATCH_FLOOR)) v86Losses.push(row);
  for (const side of ["rewrite", "legacy", "v86"]) {
    require_(aggregate.sides?.[side]?.samples?.length === 3,
      `${row}/${side} does not have three samples`);
    require_(aggregate.sides?.[side]?.spread <= 1.25,
      `${row}/${side} sample spread exceeds 1.25x`);
  }
}
require_(legacyLosses.length === 0, `legacy parity losses: ${legacyLosses.join(",")}`);
require_(JSON.stringify(v86Losses) === JSON.stringify(["boot", "compile"]),
  `unexpected v86 loss set: ${v86Losses.join(",")}`);

const acceptedPython = accepted.aggregates?.find((row) => row.key === "python")
  ?.sides?.rewrite?.median;
const currentPython = aggregates.get("python")?.sides?.rewrite?.median;
require_(Number.isFinite(acceptedPython) && Number.isFinite(currentPython) &&
  currentPython <= acceptedPython * 1.03, "Python breaches the 3% non-regression guard");

const rowDeltas = Object.fromEntries(EXPECTED_ROWS.map((row) => {
  const before = accepted.aggregates?.find((entry) => entry.key === row)
    ?.sides?.rewrite?.median;
  const after = aggregates.get(row)?.sides?.rewrite?.median;
  return [row, {
    r085: before,
    r087: after,
    r087Speedup: Number.isFinite(before) && Number.isFinite(after) ? before / after : null,
  }];
}));

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const gate = {
  schema: 1,
  experiment: "R087",
  mechanism: "corrected-authoritative-public-scheduler-cadence",
  productBytesChanged: false,
  current: { path: resolve(currentPath), sha256: hash(currentBytes) },
  acceptedComparator: { path: resolve(acceptedPath), sha256: hash(acceptedBytes) },
  parity: {
    legacy: `${EXPECTED_ROWS.length - legacyLosses.length}/${EXPECTED_ROWS.length}`,
    v86: `${EXPECTED_ROWS.length - v86Losses.length}/${EXPECTED_ROWS.length}`,
    v86Losses,
  },
  rowDeltas,
  problems,
  pass: problems.length === 0,
};
const serialized = `${JSON.stringify(gate, null, 2)}\n`;
if (outputPath) await writeFile(outputPath, serialized, { flag: "wx" });
else process.stdout.write(serialized);
if (!gate.pass) process.exitCode = 1;
