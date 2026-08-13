#!/usr/bin/env node

import assert from "node:assert/strict";
import {
  browserExecutable,
  r076AssetManifest,
  runR076BrowserSample,
  startR076Server,
} from "./vs-v86/r076-browser-host.mjs";

const variant = process.argv[2];
if ((variant !== "control" && variant !== "candidate") || process.argv.length !== 3) {
  throw new Error("usage: node tests/r076-chrome-boot-smoke.mjs control|candidate");
}
r076AssetManifest();
const server = await startR076Server();
try {
  const sample = await runR076BrowserSample({
    origin: server.origin,
    variant,
    chrome: browserExecutable(),
  });
  const result = sample.result;
  const candidate = variant === "candidate";
  assert.deepEqual(result.guest, { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" });
  assert.equal(result.outputMarkers.ready, true);
  assert.equal(result.staticLifecycle.modulesAfter, result.staticLifecycle.modulesBefore + 1);
  assert.ok(result.staticLifecycle.moduleIndex >= 0);
  assert.equal(result.staticLifecycle.systemEnabled, false);
  assert.equal(result.staticLifecycle.sampledEnabled, candidate);
  assert.equal(result.staticLifecycle.sampledBackoffEnabled, candidate);
  assert.ok(BigInt(result.counters.generated) > 0n);
  assert.equal(BigInt(result.counters.errors), 0n);
  for (const field of ["staticFast", "sampled", "samples", "polls", "marks", "bypasses"]) {
    if (candidate) assert.ok(BigInt(result.counters[field]) > 0n, field);
    else assert.equal(BigInt(result.counters[field]), 0n, field);
  }
  console.log(
    `R076 ${variant} smoke: PASS guest=${JSON.stringify(result.guest)} ` +
      `instructions=${result.instructions} sampled=${result.counters.sampled} ` +
      `marks=${result.counters.marks} bypasses=${result.counters.bypasses}`,
  );
} finally {
  await server.close();
}
