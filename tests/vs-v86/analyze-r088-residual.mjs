#!/usr/bin/env node

// Preserve R086's frozen closure classifier while labeling the corrected-
// cadence collection independently. Keeping this wrapper small makes the
// dependency and the experiment identity explicit in the resulting report.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const arguments_ = process.argv.slice(2);
const outputPath = arguments_
  .find((argument) => argument.startsWith("--output="))
  ?.slice("--output=".length);
const paths = arguments_.filter((argument) => !argument.startsWith("--"));
if (!outputPath || !paths.length) {
  throw new Error(
    "usage: analyze-r088-residual.mjs --output=REPORT.json PROFILE.cpuprofile [...]",
  );
}

const here = dirname(fileURLToPath(import.meta.url));
const analyzer = join(here, "analyze-r086-residual.mjs");
const analyzerBytes = readFileSync(analyzer);
const child = spawnSync(process.execPath, [analyzer, ...paths], {
  encoding: "utf8",
  maxBuffer: 64 << 20,
});
if (child.status !== 0) {
  throw new Error(child.stderr || child.stdout || "R086 classifier failed");
}

const report = JSON.parse(child.stdout);
report.experiment = "R088";
report.mechanism = "exact-R085-public-cadence-closure-aware-residual-attribution";
report.schedulerCadence = {
  name: "public-one-slice-per-turn",
  rv64SlicesPerEventLoopTurn: 1,
};
report.classifier = {
  path: "tests/vs-v86/analyze-r086-residual.mjs",
  sha256: createHash("sha256").update(analyzerBytes).digest("hex"),
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
