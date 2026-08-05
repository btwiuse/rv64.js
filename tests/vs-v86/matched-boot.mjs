#!/usr/bin/env node

import { spawn } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const artifacts = resolve(process.env.ARTIFACTS || join(root, "target/bench"));
const repetitions = Number(process.env.REPS || 5);
if (!Number.isInteger(repetitions) || repetitions < 1) throw new Error("REPS must be positive");

const run = (side) => new Promise((resolveRun, reject) => {
  const child = spawn(
    process.execPath,
    ["--max-old-space-size=4096", join(root, "tests/vs-v86/matched-boot-worker.mjs"), side],
    { env: { ...process.env, ARTIFACTS: artifacts } },
  );
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (data) => { stdout += data; });
  child.stderr.on("data", (data) => { stderr += data; });
  child.on("close", (code) => {
    if (code !== 0) reject(new Error(`${side} failed (${code}): ${stderr || stdout}`));
    else resolveRun(JSON.parse(stdout.trim().split("\n").at(-1)));
  });
});

const trials = [];
for (let repetition = 0; repetition < repetitions; repetition++) {
  const order = repetition % 2 ? ["v86", "rv64"] : ["rv64", "v86"];
  for (const side of order) {
    const result = await run(side);
    result.repetition = repetition + 1;
    trials.push(result);
    process.stderr.write(
      `[matched-boot] ${repetition + 1}/${repetitions} ${side}: ` +
      `${result.milestones.ready.toFixed(1)} ms\n`,
    );
  }
}

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
};
const summary = Object.fromEntries(["rv64", "v86"].map((side) => {
  const rows = trials.filter((trial) => trial.side === side);
  return [side, {
    loadMs: median(rows.map((row) => row.loadMs)),
    createMs: median(rows.map((row) => row.createMs)),
    milestones: Object.fromEntries(
      ["firmware", "kernel", "root", "ready"]
        .filter((key) => rows.every((row) => row.milestones[key] !== undefined))
        .map((key) => [key, median(rows.map((row) => row.milestones[key]))]),
    ),
  }];
}));

const report = {
  schema: 1,
  generatedAt: new Date().toISOString(),
  repetitions,
  contract: {
    linux: "6.12.7",
    alpine: "3.24.1 alpine-base",
    memoryMB: 512,
    initramfs: "uncompressed newc",
    rv64Kernel: "uncompressed Image",
    v86Kernel: "bzImage (required by the SeaBIOS Linux loader)",
    rv64Firmware: "OpenSBI 1.4",
    v86Firmware: "SeaBIOS",
  },
  trials,
  summary,
};
const output = join(artifacts, "matched-boot.json");
await writeFile(output, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
console.log(`wrote ${output}`);
