#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BOOT_QUANTUM,
  runTimedBoot,
  yieldsAfterPump,
} from "./r076-browser-boot-lib.mjs";
import { r076AssetManifest } from "./r076-browser-host.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const budgets = [];
let pumps = 0;
let clock = 0;
const yieldedAfter = [];
const result = await runTimedBoot({
  vm: {
    runVirtSystem(budget) {
      budgets.push(budget);
      pumps++;
      return false;
    },
  },
  ready: () => pumps === 10,
  nextTask: async () => yieldedAfter.push(pumps),
  now: () => clock++,
  timeoutMs: 1_000,
});
assert.equal(BOOT_QUANTUM, 2_000_000n);
assert.equal(result.pumps, 10);
assert.equal(result.yields, 3);
assert.deepEqual(yieldedAfter, [1, 5, 9]);
assert.ok(budgets.every((budget) => budget === BOOT_QUANTUM));
assert.deepEqual(
  Array.from({ length: 10 }, (_, index) => yieldsAfterPump(index)),
  [true, false, false, false, true, false, false, false, true, false],
);

const worker = readFileSync(join(root, "tests/jit-modern-boot-browser-worker.mjs"), "utf8");
const sourceOrder = [
  "fetchBytes(\"/rv64.wasm\")",
  "await RV64.create(wasm)",
  "vm.ex.jit_set_page_policy(1)",
  "vm.bootVirtLinuxDirect({",
  "vm.ex.jit_static_t0_system_prepare()",
  "const beforeStats = vm.jitStats()",
  "const timing = await runTimedBoot({",
  "const afterStats = vm.jitStats()",
].map((needle) => {
  const index = worker.indexOf(needle);
  assert.ok(index >= 0, `worker source marker missing: ${needle}`);
  return index;
});
assert.deepEqual(sourceOrder, [...sourceOrder].sort((left, right) => left - right));
assert.match(worker, /ready: \(\) => output\.includes\("SCORECARD_V2_READY"\)/);
assert.match(worker, /cmdline: "console=ttyS0 rdinit=\/init"/);
assert.match(worker, /ramMB: 512/);
assert.match(worker, /vm\.ex\.jit_set_enabled\(1\)/);
assert.match(worker, /vm\.ex\.jit_set_region_tlb_cache\?\.\(1\)/);
assert.match(worker, /vm\.ex\.jit_set_region_tlb_cache_min_accesses\?\.\(4\)/);
assert.match(worker, /if \(vm\.tailCallsSupported\) vm\.ex\.jit_set_region_tail_chain\?\.\(1\)/);
assert.match(worker, /systemEnabled: false/);
assert.match(worker, /sampledEnabled: variant === "candidate"/);
assert.match(worker, /sampledBackoffEnabled: variant === "candidate"/);

const authoritative = readFileSync(
  join(root, "tests/vs-v86/scorecard-v2-worker.mjs"),
  "utf8",
);
assert.match(authoritative, /vm\.runVirtSystem\(2_000_000n\)/);
assert.match(authoritative, /const shouldYield = yieldEveryPumpDiagnostic \|\| \(iteration & 3\) === 0/);
assert.match(authoritative, /console\.text\.includes\("SCORECARD_V2_READY"\)/);

const manifest = r076AssetManifest();
for (const name of ["loader", "wasm", "kernel", "initramfs", "page", "worker", "timingLibrary"]) {
  assert.match(manifest[name].sha256, /^[0-9a-f]{64}$/);
  assert.ok(manifest[name].bytes > 0);
}

console.log(
  "R076 harness selftest: scorecard timer boundary, 2M quantum, 1/5/9 yield cadence, marker, and assets enforced",
);
