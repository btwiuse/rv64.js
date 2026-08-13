#!/usr/bin/env node

import assert from "node:assert/strict";
import {
  browserExecutable,
  r077AssetManifest,
  runR077BrowserSample,
  startR077Server,
} from "./vs-v86/r077-browser-host.mjs";

const variant = process.argv[2];
if ((variant !== "control" && variant !== "production") || process.argv.length !== 3) {
  throw new Error("usage: node tests/r077-chrome-boot-smoke.mjs control|production");
}
r077AssetManifest();
const server = await startR077Server();
try {
  const sample = await runR077BrowserSample({
    origin: server.origin,
    variant,
    chrome: browserExecutable(),
  });
  const result = sample.result;
  const production = variant === "production";
  assert.deepEqual(result.guest, { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" });
  assert.equal(result.outputMarkers.ready, true);
  assert.equal(result.staticLifecycle.productionHelper, true);
  assert.equal(result.staticLifecycle.registeredModules, 1);
  assert.equal(result.staticLifecycle.modulesAfter, result.staticLifecycle.modulesBefore + 1);
  assert.ok(result.staticLifecycle.moduleIndex >= 0);
  assert.equal(result.staticLifecycle.systemEnabled, false);
  assert.equal(result.staticLifecycle.sampledEnabled, production);
  assert.equal(result.staticLifecycle.sampledBackoffEnabled, production);
  assert.equal(result.staticLifecycle.controlOverride, !production);
  assert.ok(BigInt(result.counters.generated) > 0n);
  assert.equal(BigInt(result.counters.errors), 0n);
  for (const field of ["staticFast", "sampled", "samples", "polls", "marks", "bypasses"]) {
    if (production) assert.ok(BigInt(result.counters[field]) > 0n, field);
    else assert.equal(BigInt(result.counters[field]), 0n, field);
  }
  console.log(
    `R077 ${variant} smoke: PASS guest=${JSON.stringify(result.guest)} ` +
      `instructions=${result.instructions} sampled=${result.counters.sampled} ` +
      `marks=${result.counters.marks} bypasses=${result.counters.bypasses}`,
  );
} finally {
  await server.close();
}
