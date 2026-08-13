// Process-isolated production region-size sweep on Linux 6.12/Alpine.
// Inputs are loaded before timed boot, caps alternate order by sample, and
// each run retains raw lifecycle, module-size, instruction, and shell-work data.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const self = fileURLToPath(import.meta.url);

async function worker(cap, pageCap) {
  const {
    bootModern, guestCommand, loadModernImages, machineDiagnostics,
    missingModernImages, waitForAlpine,
  } = await import("./modern-linux-harness.mjs");
  const missing = missingModernImages(root, "direct");
  if (missing.length) throw new Error(`missing modern images: ${missing.join(", ")}`);
  const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
  const wasm = readFileSync(
    join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  );
  const images = await loadModernImages(root);
  const regions = [];
  let moduleCount = 0;
  let moduleBytes = 0;
  const bootStarted = performance.now();
  const machine = await bootModern({
    RV64,
    wasm,
    images,
    mode: "direct",
    jit: true,
    superblock: true,
    configure: (vm) => {
      vm.ex.jit_set_region_leader_cap(cap);
      vm.ex.jit_set_region_page_cap(pageCap);
      vm.ex.jit_set_sb_spacing(0);
    },
    onJitModule: (bytes, metadata) => {
      moduleCount++;
      moduleBytes += bytes.length;
      if (metadata.kind === "async-region") {
        regions.push({
          bytes: bytes.length,
          hash: createHash("sha256").update(bytes).digest("hex"),
          ...Object.fromEntries(Object.entries(metadata).map(([key, value]) => [
            key, typeof value === "bigint" ? value.toString() : value,
          ])),
        });
      }
    },
  });
  if (!await waitForAlpine(machine)) {
    throw new Error(`cap ${cap} boot timeout: ${machineDiagnostics(machine)}`);
  }
  const bootMs = performance.now() - bootStarted;
  const beforeInsns = machine.vm.virtInsnCount();
  const beforeJit = machine.vm.ex.jit_stat(0);
  const beforeDispatches = machine.vm.ex.jit_stat(1);
  const workStarted = performance.now();
  const complete = await guestCommand(
    machine,
    "i=0; s=0; while [ $i -lt 20000 ]; do i=$((i+1)); s=$((s+i)); done; " +
      "echo REGION_POLICY_${s}",
    "REGION_POLICY_200010000",
    180_000,
  );
  const workMs = performance.now() - workStarted;
  if (!complete) throw new Error(`cap ${cap} workload timeout: ${machineDiagnostics(machine)}`);
  // One event-loop turn allows the last async compilation promise to publish.
  await new Promise((resolve) => setImmediate(resolve));
  process.stdout.write(JSON.stringify({
    cap,
    pageCap,
    engine: { node: process.version, v8: process.versions.v8 },
    bootMs,
    workMs,
    workInsns: (machine.vm.virtInsnCount() - beforeInsns).toString(),
    workJitInsns: (machine.vm.ex.jit_stat(0) - beforeJit).toString(),
    workDispatches: (machine.vm.ex.jit_stat(1) - beforeDispatches).toString(),
    totalInsns: machine.vm.virtInsnCount().toString(),
    lifecycle: machine.vm.jitLifecycleStats(),
    stats: {
      entries: machine.vm.ex.jit_stat(3).toString(),
      regionIssued: machine.vm.ex.jit_stat(12).toString(),
      regionLanded: machine.vm.ex.jit_stat(13).toString(),
      regionCalls: machine.vm.ex.jit_stat(48).toString(),
      regionInsns: machine.vm.ex.jit_stat(49).toString(),
    },
    moduleCount,
    moduleBytes,
    regions,
    regionModuleCount: regions.length,
    regionBytes: regions.map((module) => module.bytes),
  }));
}

if (process.argv.includes("--worker")) {
  const cap = Number(process.argv.find((argument) => argument.startsWith("--cap="))?.split("=")[1]);
  const pageCap = Number(
    process.argv.find((argument) => argument.startsWith("--page-cap="))?.split("=")[1] ?? 3,
  );
  await worker(cap, pageCap);
  process.exit(0);
}

const capsArg = process.argv.find((argument) => argument.startsWith("--caps="));
const caps = (capsArg ? capsArg.split("=")[1].split(",") : ["64", "128", "256", "512"])
  .map(Number);
if (caps.some((cap) => !Number.isInteger(cap) || cap < 2 || cap > 512)) {
  throw new Error("--caps must contain integers from 2 through 512");
}
const samplesArg = process.argv.find((argument) => argument.startsWith("--samples="));
const samples = samplesArg ? Number(samplesArg.split("=")[1]) : 3;
if (!Number.isInteger(samples) || samples < 1 || samples > 10) {
  throw new Error("--samples must be an integer from 1 through 10");
}
const pageCapArg = process.argv.find((argument) => argument.startsWith("--page-cap="));
const pageCap = pageCapArg ? Number(pageCapArg.split("=")[1]) : 3;
if (!Number.isInteger(pageCap) || pageCap < 1 || pageCap > 3) {
  throw new Error("--page-cap must be an integer from 1 through 3");
}

const raw = Object.fromEntries(caps.map((cap) => [cap, []]));
for (let sample = 0; sample < samples; sample++) {
  const order = sample & 1 ? [...caps].reverse() : caps;
  for (const cap of order) {
    const child = spawnSync(
      process.execPath,
      [self, "--worker", `--cap=${cap}`, `--page-cap=${pageCap}`],
      {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 64 << 20,
      },
    );
    if (child.status !== 0) throw new Error(child.stderr || child.stdout);
    raw[cap].push(JSON.parse(child.stdout));
  }
}

const report = {
  schema: 1,
  methodology: "fresh-node-and-vm-per-cap-sample/alternating-cap-order/modern-direct-linux-6.12",
  samples,
  pageCap,
  engine: raw[caps[0]][0].engine,
  caps: Object.fromEntries(caps.map((cap) => {
    const runs = raw[cap];
    return [cap, {
      bootMs: summary(runs.map((run) => run.bootMs)),
      workMs: summary(runs.map((run) => run.workMs)),
      workMInsnPerSec: summary(runs.map((run) =>
        Number(run.workInsns) / run.workMs / 1000)),
      workJitPercent: summary(runs.map((run) =>
        Number(BigInt(run.workJitInsns) * 10_000n / BigInt(run.workInsns)) / 100)),
      workInsnsPerDispatch: summary(runs.map((run) =>
        Number(BigInt(run.workInsns) / BigInt(run.workDispatches || "1")))),
      translateMs: summary(runs.map((run) => run.lifecycle.systemTranslateMs)),
      compileMs: summary(runs.map((run) => run.lifecycle.compileMs)),
      emittedBytes: summary(runs.map((run) => run.lifecycle.systemEmittedBytes)),
      regionModuleCount: summary(runs.map((run) => run.regionModuleCount)),
      regionBytesMax: summary(runs.map((run) => Math.max(0, ...run.regionBytes))),
      regionBytesTotal: summary(runs.map((run) =>
        run.regionBytes.reduce((total, bytes) => total + bytes, 0))),
      rawRuns: runs,
    }];
  })),
};
const outputArg = process.argv.find((argument) => argument.startsWith("--output="));
if (outputArg) writeFileSync(outputArg.split("=")[1], `${JSON.stringify(report, null, 2)}\n`);
if (process.argv.includes("--json")) console.log(JSON.stringify(report, null, 2));
else {
  const f = (value) => value.toFixed(3);
  console.log(
    `${samples} modern Linux samples per cap; page cap ${pageCap}; V8 ${report.engine.v8}`,
  );
  for (const cap of caps) {
    const value = report.caps[cap];
    console.log(
      `cap ${cap}: boot ${f(value.bootMs.median)} ms; work ${f(value.workMs.median)} ms; ` +
        `${f(value.workMInsnPerSec.median)} Minsn/s; JIT ${f(value.workJitPercent.median)}%; ` +
        `region max ${Math.round(value.regionBytesMax.median)} bytes; ` +
        `compile ${f(value.compileMs.median)} ms`,
    );
  }
}
