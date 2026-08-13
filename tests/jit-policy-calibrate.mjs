#!/usr/bin/env node
// Calibrate the offline policy model with the exact JIT-disabled Wasm
// interpreter. Each sample is a fresh Node/V8 process; boot warms the emulator
// module naturally, then two invocations expose workload repeatability.

import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { cpus } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const self = fileURLToPath(import.meta.url);
const WORKLOADS = ["alu1", "alu5", "alu", "mix20", "mix"];
const REF = {
  alu1: "4964fd655d986771",
  alu5: "a5c58dc647744a59",
  alu: "fb7c3a58011655ba",
  mix20: "b12cac2b749d5788",
  mix: "ab036f91acaa986d",
};

async function worker() {
  const {
    bootModern,
    guestCommand,
    loadModernImages,
    machineDiagnostics,
    output,
    transferBinary,
    waitForAlpine,
  } = await import("./modern-linux-harness.mjs");
  const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
  const wasm = readFileSync(
    join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  );
  const binary = new Uint8Array(readFileSync(
    join(
      root,
      "guests/syscompute/target/riscv64gc-unknown-linux-musl/release/syscompute",
    ),
  ));
  const images = await loadModernImages(root);

  const setupStarted = performance.now();
  const machine = await bootModern({ RV64, wasm, images, mode: "direct", jit: false });
  const setupMs = performance.now() - setupStarted;
  const bootStartIcount = machine.vm.virtInsnCount();
  const bootStarted = performance.now();
  const ready = await waitForAlpine(machine, 300_000);
  const bootMs = performance.now() - bootStarted;
  assert.ok(ready, `calibration boot failed: ${machineDiagnostics(machine)}`);
  const bootInstructions = machine.vm.virtInsnCount() - bootStartIcount;

  assert.ok(await transferBinary(
    machine,
    binary,
    "/tmp/syscompute",
    "POLICY_CAL_BIN",
  ), `calibration transfer failed: ${machineDiagnostics(machine)}`);

  const workloads = {};
  for (const workload of WORKLOADS) {
    workloads[workload] = [];
    for (let repetition = 0; repetition < 2; repetition++) {
      const startOutput = output(machine).length;
      const startIcount = machine.vm.virtInsnCount();
      const started = performance.now();
      const marker = `CAL_${workload.toUpperCase()}_${repetition}_DONE`;
      const done = await guestCommand(
        machine,
        `/tmp/syscompute ${workload}; echo CAL_${workload.toUpperCase()}_${repetition}_'DONE'`,
        marker,
        300_000,
      );
      const ms = performance.now() - started;
      assert.ok(done, `${workload} failed: ${machineDiagnostics(machine)}`);
      const instructions = machine.vm.virtInsnCount() - startIcount;
      const phaseOutput = output(machine).slice(startOutput);
      assert.ok(
        phaseOutput.includes(REF[workload]),
        `${workload} checksum missing: ${JSON.stringify(phaseOutput.slice(-300))}`,
      );
      workloads[workload].push({
        ms,
        instructions: instructions.toString(),
        mips: Number(instructions) / ms / 1_000,
      });
    }
  }
  assert.equal(machine.vm.ex.jit_stat(0), 0n);
  assert.equal(machine.vm.ex.jit_stat(1), 0n);
  process.stdout.write(JSON.stringify({
    engine: { node: process.version, v8: process.versions.v8 },
    setupMs,
    bootMs,
    bootInstructions: bootInstructions.toString(),
    bootMips: Number(bootInstructions) / bootMs / 1_000,
    workloads,
  }));
}

if (process.argv.includes("--worker")) {
  await worker();
  process.exit(0);
}

const samplesArg = process.argv.find((arg) => arg.startsWith("--samples="));
const samples = samplesArg ? Number(samplesArg.split("=")[1]) : 3;
if (!Number.isInteger(samples) || samples < 1 || samples > 15) {
  throw new Error("--samples must be an integer from 1 through 15");
}
const raw = [];
for (let sample = 0; sample < samples; sample++) {
  const child = spawnSync(process.execPath, [self, "--worker"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 4 << 20,
  });
  if (child.status !== 0) throw new Error(child.stderr || child.stdout);
  raw.push(JSON.parse(child.stdout));
}

const report = {
  schema: 1,
  methodology: "fresh-node-process/direct-linux-boot/exact-jit-bypass/two-invocations-per-workload",
  samples,
  engine: raw[0].engine,
  host: { platform: process.platform, arch: process.arch, cpu: cpus()[0]?.model },
  setupMs: summary(raw.map((run) => run.setupMs)),
  bootMs: summary(raw.map((run) => run.bootMs)),
  bootMips: summary(raw.map((run) => run.bootMips)),
  workloads: Object.fromEntries(WORKLOADS.map((workload) => [workload, {
    firstMs: summary(raw.map((run) => run.workloads[workload][0].ms)),
    repeatMs: summary(raw.map((run) => run.workloads[workload][1].ms)),
    firstMips: summary(raw.map((run) => run.workloads[workload][0].mips)),
    repeatMips: summary(raw.map((run) => run.workloads[workload][1].mips)),
  }])),
  raw,
};

const outputArg = process.argv.find((arg) => arg.startsWith("--output="));
if (outputArg) writeFileSync(outputArg.split("=")[1], JSON.stringify(report, null, 2) + "\n");
if (process.argv.includes("--json")) console.log(JSON.stringify(report, null, 2));
else {
  console.log(`${samples} fresh Node/V8 samples; exact JIT bypass`);
  console.log(`boot: ${report.bootMs.median.toFixed(1)}ms, ${report.bootMips.median.toFixed(2)} MIPS`);
  for (const workload of WORKLOADS) {
    const result = report.workloads[workload];
    console.log(
      `${workload}: first ${result.firstMs.median.toFixed(1)}ms / ` +
        `${result.firstMips.median.toFixed(2)} MIPS; repeat ` +
        `${result.repeatMs.median.toFixed(1)}ms / ${result.repeatMips.median.toFixed(2)} MIPS`,
    );
  }
}
