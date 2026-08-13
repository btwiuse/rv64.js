#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { evaluateR102 } from "./r102-native-gate-lib.mjs";

const [reportPath, outputPath] = process.argv.slice(2);
if (!reportPath || !outputPath) {
  throw new Error("usage: r102-native-gate.mjs REPORT.json OUTPUT.json");
}
const bytes = readFileSync(reportPath);
const result = evaluateR102(JSON.parse(bytes));
result.report = {
  path: reportPath,
  sha256: createHash("sha256").update(bytes).digest("hex"),
};
writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`, { flag: "wx" });
console.log(`R102 decision: ${result.decision}`);
if (result.integrityProblems.length) {
  console.error(result.integrityProblems.join("; "));
  process.exitCode = 1;
}
