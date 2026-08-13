#!/usr/bin/env node
// Process-isolated threshold sweep for the async page policy. Each point boots
// a fresh emulator so previously compiled WebAssembly and hot V8 code cannot
// leak into another threshold's measurement.

import assert from "node:assert/strict";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { cpus } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const runner = join(here, "jit-page-policy-modern.mjs");

function option(name, fallback) {
  const arg = process.argv.find((value) => value.startsWith(`--${name}=`));
  return arg ? arg.slice(name.length + 3) : fallback;
}

const mode = option("mode", "direct");
const allowNoJit = process.argv.includes("--allow-no-jit");
const samples = Number(option("samples", "3"));
const quantum = Number(option("quantum", "32"));
const thresholds = option(
  "thresholds",
  "131072,262144,524288,1048576,2097152",
).split(",").map(Number);
const workloads = option("workloads", "alu1,alu5,mix20").split(",").filter(Boolean);
const baselinePath = option(
  "baseline",
  join(root, "target/jit-policy-traces/calibration-medium-node.json"),
);

assert.ok(mode === "direct" || mode === "opensbi", "--mode must be direct or opensbi");
assert.ok(Number.isInteger(samples) && samples >= 1 && samples <= 15,
  "--samples must be an integer from 1 through 15");
assert.ok(Number.isInteger(quantum) && quantum >= 1 && quantum <= 4096,
  "--quantum must be an integer from 1 through 4096");
assert.ok(thresholds.length > 0 && thresholds.every((value) =>
  Number.isInteger(value) && value >= 32 && value <= 0xffff_ffff),
"--thresholds must contain comma-separated integers from 32 through 2^32-1");
assert.ok(workloads.length > 0, "--workloads must not be empty");

const baseline = existsSync(baselinePath)
  ? JSON.parse(readFileSync(baselinePath, "utf8"))
  : null;
const runs = [];
for (const threshold of thresholds) {
  for (let sample = 0; sample < samples; sample++) {
    const args = [
      runner,
      `--mode=${mode}`,
      `--threshold=${threshold}`,
      `--quantum=${quantum}`,
      `--workloads=${workloads.join(",")}`,
      "--json",
    ];
    if (allowNoJit) args.push("--allow-no-jit");
    const child = spawnSync(process.execPath, args, {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 16 << 20,
    });
    if (child.status !== 0) {
      throw new Error(
        `threshold ${threshold}, sample ${sample + 1} failed:\n${child.stderr || child.stdout}`,
      );
    }
    runs.push({ sample, ...JSON.parse(child.stdout) });
  }
}

function summarizeThreshold(threshold) {
  const selected = runs.filter((run) => run.threshold === threshold);
  const result = {
    threshold,
    bootMs: summary(selected.map((run) => run.boot.ms)),
    bootJitPercent: summary(selected.map((run) => run.boot.jitPercent)),
    moduleCount: summary(selected.map((run) => run.modules.length)),
    emittedBytes: summary(selected.map((run) => Number(run.lifecycle.emittedBytes))),
    translateMs: summary(selected.map((run) => run.lifecycle.translateMs)),
    wasmCompileElapsedMs: summary(selected.map((run) => run.lifecycle.compileMs)),
    workloads: {},
  };
  for (const workload of workloads) {
    const first = selected.map((run) => run.workloads[workload][0]);
    const repeat = selected.map((run) => run.workloads[workload][1]);
    result.workloads[workload] = {
      firstMs: summary(first.map((run) => run.ms)),
      repeatMs: summary(repeat.map((run) => run.ms)),
      firstJitPercent: summary(first.map((run) => run.jitPercent)),
      repeatJitPercent: summary(repeat.map((run) => run.jitPercent)),
    };
    if (baseline?.workloads?.[workload]) {
      const interpreterFirst = baseline.workloads[workload].firstMs.median;
      const interpreterRepeat = baseline.workloads[workload].repeatMs.median;
      result.workloads[workload].firstSpeedup = summary(
        first.map((run) => interpreterFirst / run.ms),
      );
      result.workloads[workload].repeatSpeedup = summary(
        repeat.map((run) => interpreterRepeat / run.ms),
      );
    }
  }
  return result;
}

const report = {
  schema: 1,
  methodology: "fresh-node-process-per-point/modern-linux/async-page-only/two-invocations",
  engine: runs[0].engine,
  host: { platform: process.platform, arch: process.arch, cpu: cpus()[0]?.model },
  mode,
  samples,
  quantum,
  workloads,
  baseline: baseline ? baselinePath : null,
  thresholds: thresholds.map(summarizeThreshold),
  runs,
};

const outputArg = process.argv.find((arg) => arg.startsWith("--output="));
if (outputArg) writeFileSync(outputArg.split("=")[1], JSON.stringify(report, null, 2) + "\n");
if (process.argv.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  console.log(`${samples} fresh Node/V8 samples per threshold; ${mode} Linux`);
  for (const point of report.thresholds) {
    console.log(
      `threshold ${point.threshold}: boot ${point.bootMs.median.toFixed(1)}ms, ` +
        `${point.moduleCount.median} modules, ${point.translateMs.median.toFixed(2)}ms translation`,
    );
    for (const workload of workloads) {
      const value = point.workloads[workload];
      const speedup = value.firstSpeedup
        ? `, ${value.firstSpeedup.median.toFixed(2)}x/${value.repeatSpeedup.median.toFixed(2)}x vs interpreter`
        : "";
      console.log(
        `  ${workload}: ${value.firstMs.median.toFixed(1)}ms first, ` +
          `${value.repeatMs.median.toFixed(1)}ms repeat; ` +
          `${value.firstJitPercent.median.toFixed(1)}%/${value.repeatJitPercent.median.toFixed(1)}% JIT${speedup}`,
      );
    }
  }
}
