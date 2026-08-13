#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const EXPECTED_ROWS = [
  "alu", "mixed", "boot", "python", "compile", "numeric", "string",
  "bitfield", "fpemul", "fourier", "assignment", "idea", "huffman",
];
export const EXPECTED_SIDES = ["rewrite", "legacy", "v86"];
export const CANDIDATE_WASM =
  "d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59";
export const LOADER =
  "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385";
export const LEGACY_LOADER =
  "54df79c8b35cf50bcee34c4af02d7eb02b09e0439b717ee75bb830e733595b12";
export const LEGACY_WASM =
  "274aaab5799386956a8c509434961c4a426066f8fc9f520e994c210affd61709";
export const V86_WASM =
  "4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1";
export const V86_COMMIT = "2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f";
export const BASELINE_SHA256 =
  "1d822f1c1f37a81d00ee5b85dfb7d90f773eaeca79c87222ac0e5ab353e887c7";
export const R125_SHA256 =
  "9e051d1fd4b23c7b440134778e54a500c5fe6eb0cdc058e0d8bf3db359491868";

const MATCH_FLOOR = 0.95;
const MATERIAL_FLOOR = 0.99;
const TARGET_GAIN = 1.01;

function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) =>
      `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function lossSet(aggregates, key) {
  return EXPECTED_ROWS.filter((row) => !(aggregates.get(row)?.[key] >= MATCH_FLOOR));
}

function trialKey(trial) {
  return `${trial.side}/${trial.row}`;
}

export function evaluateR126(current, baseline, wanix) {
  const problems = [];
  const require_ = (condition, message) => {
    if (!condition) problems.push(message);
  };

  require_(current?.authoritative === true, "current report is not authoritative");
  require_(current?.measurementValid === true, "current report is not measurement-valid");
  require_(Array.isArray(current?.problems) && current.problems.length === 0,
    "current report has internal problems");
  require_(current?.configuration?.reps === 3, "current report does not use three repetitions");
  require_(canonical(current?.configuration?.rows) === canonical(EXPECTED_ROWS),
    "current row set/order differs");
  require_(canonical(current?.configuration?.sides) === canonical(EXPECTED_SIDES),
    "current side set/order differs");
  require_(current?.configuration?.rewritePolicy === "production",
    "current rewrite policy is not production");
  require_(current?.configuration?.v86ExecutionPreflight === true,
    "v86 execution preflight is disabled");
  require_(current?.configuration?.schedulerCadence?.name === "public-one-slice-per-turn" &&
    current.configuration.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
  "current cadence is not public one-slice-per-turn");
  require_(current?.provenance?.cpuAffinity === "8-15", "current CPU affinity differs");

  const trials = Array.isArray(current?.trials) ? current.trials : [];
  require_(trials.length === 117, `expected 117 trials, found ${trials.length}`);
  const baselineInputs = new Map();
  for (const trial of baseline?.trials || []) {
    const key = trialKey(trial);
    const value = canonical(trial.result?.inputSha256);
    const prior = baselineInputs.get(key);
    if (prior !== undefined && prior !== value) {
      problems.push(`accepted baseline input identity varies for ${key}`);
    }
    baselineInputs.set(key, value);
  }

  for (const trial of trials) {
    const prefix = `${trial.side}/${trial.row}/rep${trial.rep}`;
    const result = trial.result;
    require_(!trial.error && result, `${prefix} failed`);
    if (!result) continue;
    require_(result.measurementEligible === true, `${prefix} is ineligible`);
    require_(result.side === trial.side && result.row === trial.row,
      `${prefix} result label differs`);
    require_(canonical(result.inputSha256) === baselineInputs.get(trialKey(trial)),
      `${prefix} input identity differs from accepted baseline`);
    require_(result.runtime?.schedulerCadence?.name === "public-one-slice-per-turn" &&
      result.runtime.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
    `${prefix} cadence differs`);
    require_(result.runtime?.diagnostic == null, `${prefix} has a diagnostic override`);
    require_(result.runtime?.jitProof?.enabledRequested === true,
      `${prefix} did not request JIT execution`);

    const identity = result.runtime?.identity || {};
    const guest = result.runtime?.guest || {};
    if (trial.side === "rewrite") {
      require_(identity.loaderSha256 === LOADER, `${prefix} loader identity differs`);
      require_(identity.wasmSha256 === CANDIDATE_WASM, `${prefix} candidate Wasm differs`);
      require_(guest.linux === "6.12.7" && guest.alpine === "3.24.1" &&
        guest.arch === "riscv64", `${prefix} modern guest differs`);
      require_(Array.isArray(result.runtime?.policyProblems) &&
        result.runtime.policyProblems.length === 0, `${prefix} product policy differs`);
      require_(BigInt(result.runtime?.jitProof?.generatedInstructions || 0) > 0n &&
        BigInt(result.runtime?.jitProof?.dispatches || 0) > 0n,
      `${prefix} lacks generated execution`);
    } else if (trial.side === "legacy") {
      require_(identity.loaderSha256 === LEGACY_LOADER, `${prefix} legacy loader differs`);
      require_(identity.wasmSha256 === LEGACY_WASM, `${prefix} legacy Wasm differs`);
      require_(guest.linux === "6.12.7" && guest.alpine === "3.24.1" &&
        guest.arch === "riscv64", `${prefix} modern guest differs`);
    } else if (trial.side === "v86") {
      require_(identity.wasmSha256 === V86_WASM && identity.sourceCommit === V86_COMMIT,
        `${prefix} copy/v86 identity differs`);
      require_(guest.linux === "6.12.7" && guest.alpine === "3.24.1" &&
        guest.arch === "i686", `${prefix} modern guest differs`);
    } else {
      problems.push(`${prefix} unknown side`);
    }
  }

  const currentRows = new Map((current?.aggregates || []).map((row) => [row.key, row]));
  const baselineRows = new Map((baseline?.aggregates || []).map((row) => [row.key, row]));
  require_(currentRows.size === EXPECTED_ROWS.length, "current aggregate count differs");
  require_(baselineRows.size === EXPECTED_ROWS.length, "baseline aggregate count differs");

  const rows = {};
  for (const key of EXPECTED_ROWS) {
    const now = currentRows.get(key);
    const before = baselineRows.get(key);
    require_(now && before, `${key} aggregate missing`);
    if (!now || !before) continue;
    for (const side of EXPECTED_SIDES) {
      require_(now.sides?.[side]?.samples?.length === 3,
        `${key}/${side} does not have three samples`);
      require_(now.sides?.[side]?.spread <= 1.25,
        `${key}/${side} sample spread exceeds 1.25x`);
    }
    const normalizedToV86 = now.rewriteVsV86 / before.rewriteVsV86;
    const rawRewriteSpeedup = before.sides.rewrite.median / now.sides.rewrite.median;
    rows[key] = {
      currentRewrite: now.sides.rewrite.median,
      currentLegacy: now.sides.legacy.median,
      currentV86: now.sides.v86.median,
      rewriteVsLegacy: now.rewriteVsLegacy,
      rewriteVsV86: now.rewriteVsV86,
      acceptedRewrite: before.sides.rewrite.median,
      acceptedV86: before.sides.v86.median,
      normalizedToV86,
      rawRewriteSpeedup,
    };
    require_(normalizedToV86 >= MATERIAL_FLOOR,
      `${key} normalized protected speedup ${normalizedToV86} below 0.99x`);
  }

  const legacyLosses = lossSet(currentRows, "rewriteVsLegacy");
  const baselineV86Losses = lossSet(baselineRows, "rewriteVsV86");
  const currentV86Losses = lossSet(currentRows, "rewriteVsV86");
  require_(legacyLosses.length === 0, `legacy parity losses: ${legacyLosses.join(",")}`);
  require_(baselineV86Losses.join(",") === "boot,compile",
    `accepted baseline v86 losses differ: ${baselineV86Losses.join(",")}`);
  require_(currentV86Losses.every((row) => baselineV86Losses.includes(row)),
    `new v86 parity losses: ${currentV86Losses.filter((row) =>
      !baselineV86Losses.includes(row)).join(",")}`);

  const targetSpeedups = {
    boot: rows.boot?.normalizedToV86,
    compile: rows.compile?.normalizedToV86,
  };
  require_(Object.values(targetSpeedups).some((value) => value >= TARGET_GAIN),
    `neither Boot nor Compile improves at least 1%: ${canonical(targetSpeedups)}`);

  require_(wanix?.measurementValid === true, "R125 WANIX measurement is not valid");
  require_(wanix?.artifacts?.archives?.candidate?.wasmSha256 === CANDIDATE_WASM,
    "R125 WANIX candidate identity differs");
  for (const row of ["shell", "python", "sha256", "shared9p"]) {
    require_(wanix?.rows?.[row]?.pairedMedianSpeedup >= MATERIAL_FLOOR,
      `R125 ${row} is below the 0.99x material floor`);
  }
  require_(wanix?.rows?.python?.pairedMedianSpeedup >= 1,
    "R125 /shared/bench.py regresses");

  return {
    schema: 1,
    experiment: "R126 exact-R124 untouched scorecard promotion",
    policy: {
      matchFloor: MATCH_FLOOR,
      materialProtectedFloor: MATERIAL_FLOOR,
      targetGain: TARGET_GAIN,
      targetRows: ["boot", "compile"],
      scorecardNormalization: "(current rewrite/v86) / (accepted R087 rewrite/v86)",
      wanix: "sealed R125; original analyzer output preserved",
    },
    measurementValid: current?.measurementValid === true,
    accepted: problems.length === 0,
    parity: {
      legacy: `${EXPECTED_ROWS.length - legacyLosses.length}/${EXPECTED_ROWS.length}`,
      v86: `${EXPECTED_ROWS.length - currentV86Losses.length}/${EXPECTED_ROWS.length}`,
      v86Losses: currentV86Losses,
    },
    targetSpeedups,
    rows,
    problems,
  };
}

function main() {
  const [currentPath, baselinePath, wanixPath, outputPath] = process.argv.slice(2);
  if (!currentPath || !baselinePath || !wanixPath || !outputPath) {
    throw new Error("usage: r126-r124-scorecard-gate.mjs CURRENT BASELINE R125 OUTPUT");
  }
  const currentBytes = readFileSync(currentPath);
  const baselineBytes = readFileSync(baselinePath);
  const wanixBytes = readFileSync(wanixPath);
  if (sha256(baselineBytes) !== BASELINE_SHA256) {
    throw new Error(`accepted baseline hash differs: ${sha256(baselineBytes)}`);
  }
  if (sha256(wanixBytes) !== R125_SHA256) {
    throw new Error(`sealed R125 hash differs: ${sha256(wanixBytes)}`);
  }
  const gate = evaluateR126(
    JSON.parse(currentBytes),
    JSON.parse(baselineBytes),
    JSON.parse(wanixBytes),
  );
  gate.inputs = {
    current: { path: currentPath, sha256: sha256(currentBytes) },
    acceptedBaseline: { path: baselinePath, sha256: sha256(baselineBytes) },
    r125: { path: wanixPath, sha256: sha256(wanixBytes) },
  };
  writeFileSync(outputPath, `${JSON.stringify(gate, null, 2)}\n`, { flag: "wx" });
  for (const key of EXPECTED_ROWS) {
    const row = gate.rows[key];
    console.log(`${key}: normalized=${row?.normalizedToV86?.toFixed(4)}x ` +
      `rewrite/v86=${row?.rewriteVsV86?.toFixed(4)}x`);
  }
  console.log(`parity: legacy=${gate.parity.legacy} v86=${gate.parity.v86}`);
  if (gate.problems.length) {
    throw new Error(`R126_SCORECARD_REJECT: ${gate.problems.join("; ")}`);
  }
  console.log("R126_SCORECARD_ACCEPT");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
