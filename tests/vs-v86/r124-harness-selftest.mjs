#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { BOOT_QUANTUM, runTimedBoot, yieldsAfterPump } from "./r076-browser-boot-lib.mjs";
import { r124AssetManifest } from "./r124-browser-host.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const budgets = [];
const yieldedAfter = [];
let pumps = 0;
let clock = 0;
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

const worker = readFileSync(
  join(root, "tests/jit-modern-boot-r124-browser-worker.mjs"),
  "utf8",
);
const sourceOrder = [
  "fetchBytes(`/rv64-${variant}.wasm`)",
  "const constructionStarted = performance.now()",
  "await RV64.create(wasm)",
  "vm.ex.jit_set_enabled(1)",
  "vm.ex.jit_set_page_policy(1)",
  "vm.bootVirtLinuxDirect({",
  "const beforeStats = vm.jitStats()",
  "const timing = await runTimedBoot({",
  "const constructionToMarkerMs = performance.now() - constructionStarted",
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
assert.match(worker, /constructionToMarkerMs/);
assert.match(worker, /jit_set_region_tlb_cache\?\.\(1\)/);
assert.match(worker, /jit_set_region_tlb_cache_min_accesses\?\.\(4\)/);
assert.match(worker, /if \(vm\.tailCallsSupported\) vm\.ex\.jit_set_region_tail_chain\?\.\(1\)/);

const manifest = r124AssetManifest();
assert.equal(
  manifest.wasmControl.sha256,
  "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
);
assert.equal(
  manifest.wasmCandidate.sha256,
  "d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59",
);
for (const name of [
  "loader", "wasmControl", "wasmCandidate", "kernel", "initramfs",
  "page", "worker", "timingLibrary",
]) {
  assert.match(manifest[name].sha256, /^[0-9a-f]{64}$/);
  assert.ok(manifest[name].bytes > 0);
}

console.log(
  "R124 browser harness selftest: separate artifact identities, production policy, " +
    "dual R107 clocks, scorecard timer boundary, and 2M\/1-5-9 cadence enforced",
);
