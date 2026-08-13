#!/usr/bin/env node
// Real Linux gate for the async-only page-heat policy. The first workload run
// includes tier-up; the repeat measures the landed steady state. Every module
// is captured so a synchronous single-block compile cannot hide in aggregate
// timing.

import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  bootModern,
  guestCommand,
  loadModernImages,
  machineDiagnostics,
  output,
  transferBinary,
  waitForAlpine,
} from "./modern-linux-harness.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const modeArg = process.argv.find((arg) => arg.startsWith("--mode="));
const thresholdArg = process.argv.find((arg) => arg.startsWith("--threshold="));
const quantumArg = process.argv.find((arg) => arg.startsWith("--quantum="));
const workloadsArg = process.argv.find((arg) => arg.startsWith("--workloads="));
const allowNoJit = process.argv.includes("--allow-no-jit");
const mode = modeArg ? modeArg.split("=")[1] : "direct";
const threshold = thresholdArg ? Number(thresholdArg.split("=")[1]) : 200_000;
const quantum = quantumArg ? Number(quantumArg.split("=")[1]) : 32;
const requestedWorkloads = workloadsArg
  ? workloadsArg.split("=")[1].split(",").filter(Boolean)
  : ["alu", "mix"];
if (!Number.isInteger(threshold) || threshold < 32 || threshold > 0xffff_ffff) {
  throw new Error("--threshold must be an integer from 32 through 2^32-1");
}
if (!Number.isInteger(quantum) || quantum < 1 || quantum > 4096) {
  throw new Error("--quantum must be an integer from 1 through 4096");
}
if (mode !== "direct" && mode !== "opensbi") {
  throw new Error("--mode must be direct or opensbi");
}
const references = {
  alu1: "4964fd655d986771",
  alu5: "a5c58dc647744a59",
  alu: "fb7c3a58011655ba",
  mix20: "b12cac2b749d5788",
  mix: "ab036f91acaa986d",
};
if (requestedWorkloads.length === 0 || requestedWorkloads.some((name) => !(name in references))) {
  throw new Error("--workloads must be a comma-separated subset of alu1,alu5,alu,mix20,mix");
}

const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = readFileSync(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);
const binary = new Uint8Array(readFileSync(join(
  root,
  "guests/syscompute/target/riscv64gc-unknown-linux-musl/release/syscompute",
)));
const images = await loadModernImages(root);
const modules = [];

const setupStarted = performance.now();
const machine = await bootModern({
  RV64,
  wasm,
  images,
  mode,
  jit: true,
  superblock: false,
  configure: (vm) => {
    vm.ex.jit_set_page_policy(1);
    vm.ex.jit_set_page_threshold(threshold);
    vm.ex.jit_set_page_quantum(quantum);
  },
  onJitModule: (bytes, metadata) => modules.push({ bytes: bytes.length, ...metadata }),
});
const setupMs = performance.now() - setupStarted;
const bootStart = machine.vm.virtInsnCount();
const bootJitStart = machine.vm.ex.jit_stat(0);
const bootStarted = performance.now();
const ready = await waitForAlpine(machine, 300_000);
const bootMs = performance.now() - bootStarted;
assert.ok(ready, `${mode} page-policy boot failed: ${machineDiagnostics(machine)}`);
assert.match(output(machine), /Linux version 6\./);
if (mode === "opensbi") assert.match(output(machine), /OpenSBI v/);
const bootInstructions = machine.vm.virtInsnCount() - bootStart;
const bootJitInstructions = machine.vm.ex.jit_stat(0) - bootJitStart;

assert.ok(await transferBinary(
  machine,
  binary,
  "/tmp/syscompute",
  `PAGE_${mode.toUpperCase()}_BIN`,
), `syscompute transfer failed: ${machineDiagnostics(machine)}`);

const workloads = {};
for (const workload of requestedWorkloads) {
  workloads[workload] = [];
  for (let repetition = 0; repetition < 2; repetition++) {
    const startOutput = output(machine).length;
    const startIcount = machine.vm.virtInsnCount();
    const startJit = machine.vm.ex.jit_stat(0);
    const started = performance.now();
    const marker = `PAGE_${workload.toUpperCase()}_${repetition}_DONE`;
    const done = await guestCommand(
      machine,
      `/tmp/syscompute ${workload}; echo PAGE_${workload.toUpperCase()}_${repetition}_'DONE'`,
      marker,
      300_000,
    );
    const ms = performance.now() - started;
    assert.ok(done, `${mode}/${workload} failed: ${machineDiagnostics(machine)}`);
    const instructions = machine.vm.virtInsnCount() - startIcount;
    const jitInstructions = machine.vm.ex.jit_stat(0) - startJit;
    const phaseOutput = output(machine).slice(startOutput);
    assert.ok(phaseOutput.includes(references[workload]), `${workload} checksum missing`);
    workloads[workload].push({
      ms,
      instructions: instructions.toString(),
      jitInstructions: jitInstructions.toString(),
      mips: Number(instructions) / ms / 1_000,
      jitPercent: Number(jitInstructions * 10_000n / instructions) / 100,
    });
  }
}

if (!allowNoJit) assert.ok(modules.length > 0, "page policy emitted no modules");
assert.ok(
  modules.every((module) => module.kind === "async-region"),
  `synchronous module leaked into page policy: ${JSON.stringify(modules.find((m) => m.kind !== "async-region"))}`,
);
if (!allowNoJit) {
  assert.ok(machine.vm.ex.jit_stat(0) > 0n, "page policy executed no generated code");
}
const pagePolicy = Array.from({ length: 19 }, (_, index) =>
  machine.vm.ex.jit_page_policy_stat(index).toString()
);
const report = {
  schema: 1,
  methodology: "modern-linux/async-page-only/first-and-repeat/checksum-verified",
  engine: { node: process.version, v8: process.versions.v8 },
  mode,
  threshold,
  quantum,
  setupMs,
  boot: {
    ms: bootMs,
    instructions: bootInstructions.toString(),
    jitInstructions: bootJitInstructions.toString(),
    mips: Number(bootInstructions) / bootMs / 1_000,
    jitPercent: Number(bootJitInstructions * 10_000n / bootInstructions) / 100,
  },
  workloads,
  pagePolicy,
  lifecycle: {
    translateMs: Number(machine.vm.ex.jit_stat(76)) / 1e6,
    translateAttempts: machine.vm.ex.jit_stat(77).toString(),
    emittedBytes: machine.vm.ex.jit_stat(78).toString(),
    copiedMs: machine.vm.jitCopyMs ?? 0,
    compileMs: machine.vm.jitCompileMs ?? 0,
    instantiateMs: machine.vm.jitInstantiateMs ?? 0,
    publishMs: machine.vm.jitPublishMs ?? 0,
  },
  modules,
};
const json = JSON.stringify(
  report,
  (_key, value) => typeof value === "bigint" ? value.toString() : value,
  2,
) + "\n";

const outputArg = process.argv.find((arg) => arg.startsWith("--output="));
if (outputArg) writeFileSync(outputArg.split("=")[1], json);
if (process.argv.includes("--json")) process.stdout.write(json);
else {
  console.log(
    `PASS ${mode} page policy @ ${threshold}/q${quantum}: boot ${bootMs.toFixed(1)}ms, ` +
      `${report.boot.jitPercent.toFixed(1)}% generated, ${modules.length} async modules`,
  );
  for (const workload of requestedWorkloads) {
    const [first, repeat] = workloads[workload];
    console.log(
      `  ${workload}: first ${first.ms.toFixed(1)}ms (${first.jitPercent.toFixed(1)}% JIT), ` +
        `repeat ${repeat.ms.toFixed(1)}ms (${repeat.jitPercent.toFixed(1)}% JIT)`,
    );
  }
  console.log(
    `  translate ${report.lifecycle.translateMs.toFixed(2)}ms / ` +
      `${report.lifecycle.translateAttempts} attempts; frontend compile ` +
      `${report.lifecycle.compileMs.toFixed(2)}ms total`,
  );
}
