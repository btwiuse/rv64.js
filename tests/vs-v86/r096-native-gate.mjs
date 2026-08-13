#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { evaluateR096 } from "./r096-native-gate-lib.mjs";

const [reportPath, outputPath] = process.argv.slice(2);
if (!reportPath || !outputPath) {
  throw new Error("usage: r096-native-gate.mjs REPORT.json OUTPUT.json");
}

const EXPECTED_WASM = "05daf545b5197edbb337ae2a2c2cf512342417cef530c6773be7ba2cf382f9b3";
const bytes = readFileSync(reportPath);
const report = JSON.parse(bytes);
const gate = evaluateR096(report, EXPECTED_WASM);
gate.report = {
  path: reportPath,
  sha256: createHash("sha256").update(bytes).digest("hex"),
};
gate.expectedWasmSha256 = EXPECTED_WASM;
writeFileSync(outputPath, `${JSON.stringify(gate, null, 2)}\n`, { flag: "wx" });
if (gate.integrityProblems.length) process.exitCode = 1;
