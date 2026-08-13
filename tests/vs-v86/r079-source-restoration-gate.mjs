#!/usr/bin/env node

import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { evaluateR079SourceRestoration } from "./r079-source-restoration-lib.mjs";

const args = process.argv.slice(2);
const outputIndex = args.indexOf("--output");
if (args.length !== 3 || outputIndex < 0 || outputIndex === args.length - 1) {
  throw new Error(
    "usage: node tests/vs-v86/r079-source-restoration-gate.mjs " +
      "REPORT.json --output GATE.json",
  );
}
const positional = args.filter((_, index) => index !== outputIndex && index !== outputIndex + 1);
if (positional.length !== 1) throw new Error("expected exactly one report path");
const reportPath = resolve(positional[0]);
const outputPath = resolve(args[outputIndex + 1]);
const bytes = await readFile(reportPath);
const sha256 = createHash("sha256").update(bytes).digest("hex");
const result = evaluateR079SourceRestoration(JSON.parse(bytes), sha256);
result.inputReportPath = reportPath;
await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(result, null, 2)}\n`);
for (const [key, row] of Object.entries(result.rows)) {
  console.log(
    `${key}: ${row.pairedMedian?.toFixed(3)}x ` +
      `CI=[${row.confidence95?.map((value) => value.toFixed(3)).join(", ")}]`,
  );
}
console.log(`gate report: ${outputPath}`);
if (result.gatePassed) {
  console.log("R079_SOURCE_RESTORATION_GATE_PASS");
} else {
  console.error(`R079_SOURCE_RESTORATION_GATE_FAIL: ${result.problems.join("; ")}`);
  process.exitCode = 1;
}
