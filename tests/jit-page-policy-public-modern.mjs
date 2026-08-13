#!/usr/bin/env node
// Stable-API production wiring gate: RV64.create must select the measured page
// policy before modern Linux starts, and its normal event-driven scheduler must
// allow async generated code to land and execute.

import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { RV64, RV64Debug } from "../web/rv64.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const wasmPath = join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
const kernelPath = process.env.RV64_MODERN_KERNEL || join(root, "web/images/alpine/Image");
const diskPath = process.env.RV64_ALPINE_DISK || join(root, "web/images/alpine/alpine.ext4");
if (![wasmPath, kernelPath, diskPath].every(existsSync)) {
  console.log("SKIP stable page-policy Linux gate (prepare modern images first)");
  process.exit(0);
}

let core;
const originalCreate = RV64Debug.create;
RV64Debug.create = async (...args) => (core = await originalCreate.apply(RV64Debug, args));
let output = "";
let observedError;
const decoder = new TextDecoder();
let vm;
try {
  vm = await RV64.create({
    wasm: await readFile(wasmPath),
    memoryMB: 512,
    boot: {
      mode: "linux-direct",
      kernel: await readFile(kernelPath),
      disk: await readFile(diskPath),
      cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
    },
    network: { mode: "none" },
    events: {
      console(bytes) { output += decoder.decode(bytes, { stream: true }); },
      error(error) { observedError = error; },
    },
  });
} finally {
  RV64Debug.create = originalCreate;
}

assert.equal(core.ex.jit_page_policy_stat(0), 1n);
assert.equal(core.ex.jit_page_policy_stat(1), 131_072n);
assert.equal(core.ex.jit_page_policy_stat(2), 1_024n);
assert.equal(core.ex.jit_page_policy_stat(23), 1n);
assert.equal(core.ex.jit_page_policy_stat(25), 2n);
assert.equal(core.ex.jit_page_policy_stat(34), 100n);
assert.equal(core.ex.jit_page_policy_stat(38), 2n);
assert.equal(core.ex.jit_page_policy_stat(39), 512n);
assert.equal(core.ex.jit_page_policy_stat(45), 32n);
assert.equal(core.ex.jit_page_policy_stat(50), 0n);
assert.equal(core.ex.jit_page_policy_stat(51), 1n);
await vm.start();
const deadline = performance.now() + 120_000;
while (vm.running && !output.includes("ALPINE_READY") && performance.now() < deadline) {
  await new Promise((resolve) => setImmediate(resolve));
}
// Readiness can precede the first asynchronous module on slower Wasm engines,
// especially now that transient S-mode code needs 32x the user threshold.
// Repeat the same documented userspace phase until its asynchronously built
// entry gets a chance to execute; this tests tier-up/landing, not scheduler
// luck at the instant the shell prints ALPINE_READY.
const generatedAtReady = core.ex.jit_stat(0);
let completedWorkPhases = 0;
for (let phase = 1; phase <= 8 && core.ex.jit_stat(0) === 0n; phase++) {
  const marker = `PAGE_POLICY_WORK_${phase}_DONE`;
  vm.console.send(
    `i=0; while [ $i -lt 20000 ]; do i=$((i+1)); done; echo ${marker}\n`,
  );
  const phaseDeadline = performance.now() + 30_000;
  while (vm.running && !output.includes(marker) && performance.now() < phaseDeadline) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.match(output, new RegExp(marker));
  completedWorkPhases = phase;
  const settleDeadline = performance.now() + 5_000;
  while (vm.running && core.ex.sys_pending_builds() !== 0 && performance.now() < settleDeadline) {
    await new Promise((resolve) => setImmediate(resolve));
  }
}
await vm.stop();
const generated = core.ex.jit_stat(0);
const modules = core.jitRegCount ?? 0;
await vm.destroy();

assert.ifError(observedError);
assert.match(output, /Linux version 6\./);
assert.match(output, /ALPINE_READY/);
assert.ok(
  generatedAtReady > 0n || completedWorkPhases > 0,
  "neither Boot nor an explicit userspace phase exercised generated execution",
);
assert.ok(
  generated > 0n,
  `stable page policy executed no generated code: modules=${modules} ` +
    `issued=${core.ex.jit_page_policy_stat(11)} landed=${core.ex.jit_page_policy_stat(12)} ` +
    `userCandidates=${core.ex.jit_page_policy_stat(48)} ` +
    `privilegedCandidates=${core.ex.jit_page_policy_stat(49)}`,
);
assert.ok(modules > 0, "stable page policy published no modules during boot");
console.log(`PASS stable page-policy Linux — ${generated} generated instructions, ${modules} modules`);
