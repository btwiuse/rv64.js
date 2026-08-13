#!/usr/bin/env node

// Correctness-only rollback proof after the R077 production-default gate
// rejected sampled static T0. This deliberately uses the no-override WANIX
// page and the rebuilt distributable, then proves the mechanism stays off at
// shell and through a real generated-code Python workload.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

if (process.argv.length !== 3) {
  throw new Error("usage: node tests/run-r077-default-off-smoke.mjs OUTPUT.json");
}
const outputPath = resolve(process.argv[2]);
assert.equal(existsSync(outputPath), false, "output already exists");

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const harness = join(root, "tests/wanix-v86-preboot-smoke.mjs");
const pagePath = join(root, "integrations/wanix/v86-rv64-three-way.html");
const url = "http://127.0.0.1:8765/examples/v86-rv64-three-way.html";
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const expectedBrowser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const expectedArtifacts = {
  page: "63028a1dcc430d21b12ed346808014389de15076b4d39d5690fcaf670e8e147a",
  rv64Jit: "378219063e1b9858443f9e4c45d7c37c88831ab4a843e8dcc39a8b8d59d42b66",
  rv64Root: "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb",
};

const page = readFileSync(pagePath, "utf8");
assert.equal(sha256(page), expectedArtifacts.page, "workspace page changed");
assert.match(page, /rv64-jit\.tgz\?v=r077-rollback-37821906/);
assert.doesNotMatch(page, /rv64\.static-t0=/, "rollback page contains an override");

const run = spawnSync(process.execPath, [harness], {
  cwd: root,
  env: {
    ...process.env,
    WANIX_URL: url,
    WANIX_VM: "rv64-jit",
    WANIX_BENCH_PHASES: "python",
    WANIX_BENCH_REPETITIONS: "1",
    WANIX_BENCH_PHASE_SYNC: "1",
    WANIX_SUMMARY_ONLY: "0",
    WANIX_EXTERNAL_P9_METRICS: "1",
  },
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
});
assert.equal(run.status, 0, `WANIX rollback smoke failed:\n${run.stderr}\n${run.stdout.slice(-4000)}`);

const objects = run.stdout.split("\n").flatMap((line) => {
  if (!line.startsWith("{")) return [];
  try {
    return [JSON.parse(line)];
  } catch {
    return [];
  }
});
const summary = objects.find((value) => value.vm === "rv64-jit");
const jit = objects.find((value) => value.jitAtShell && value.jitPhases);
assert.ok(summary, "WANIX summary JSON missing");
assert.ok(jit, "WANIX JIT lifecycle JSON missing");
assert.equal(summary.hostCpuAffinity, "8-15", "smoke affinity changed");
assert.deepEqual(summary.browser, expectedBrowser, "browser identity changed");
assert.equal(summary.artifactStable, true);
assert.equal(summary.artifacts.page.sha256, expectedArtifacts.page);
assert.equal(summary.artifacts.archives.rv64Jit.sha256, expectedArtifacts.rv64Jit);
assert.equal(summary.artifacts.archives.rv64Root.sha256, expectedArtifacts.rv64Root);
assert.deepEqual(summary.jitConfiguration, {}, "adapter/runtime override was applied");
assert.equal(summary.jitPolicy, null, "page-policy override was applied");
assert.deepEqual(summary.guest, {
  machine: "riscv64",
  alpineRelease: "3.22.5",
  pythonVersion: "3.12.13",
});
assert.deepEqual(summary.correctness.python, { complete: 1, correct: 1 });

const atShell = jit.jitAtShell.staticT0;
assert.equal(atShell.systemEnabled, false);
assert.equal(atShell.sampledEnabled, false);
assert.equal(atShell.sampledBackoffEnabled, false);
assert.equal(atShell.moduleIndex, -1);
for (const field of [
  "systemFastRetired",
  "sampledRetired",
  "samples",
  "shortSampleMarks",
  "shortSampleBypasses",
  "systemErrors",
]) {
  assert.equal(BigInt(atShell[field]), 0n, `shell static-T0 ${field} was nonzero`);
}

const phase = jit.jitPhases.python;
assert.ok(phase, "Python phase JIT proof missing");
assert.equal(phase.staticSystemT0Enabled, false);
assert.equal(phase.sampledStaticT0Enabled, false);
assert.equal(phase.sampledStaticT0BackoffEnabled, false);
assert.equal(phase.staticSystemT0ModuleIndex, -1);
for (const field of [
  "staticT0FastRetired",
  "sampledStaticT0Retired",
  "sampledStaticT0Samples",
  "sampledStaticT0ShortMarks",
  "sampledStaticT0ShortBypasses",
  "staticT0Errors",
]) {
  assert.equal(BigInt(phase[field]), 0n, `Python static-T0 ${field} was nonzero`);
}
assert.ok(BigInt(phase.generatedRetired) > 0n);

const report = {
  schema: 1,
  experiment: "R077 rejected-default rollback WANIX smoke",
  measurementUse: "correctness-only; no timing conclusion",
  harnessSha256: sha256(readFileSync(harness)),
  wrapperSha256: sha256(readFileSync(fileURLToPath(import.meta.url))),
  stdoutSha256: sha256(run.stdout),
  expectedArtifacts,
  summary,
  jitAtShell: jit.jitAtShell,
  jitPhase: phase,
};
mkdirSync(dirname(outputPath), { recursive: true });
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(
  `R077 default-off rollback smoke: PASS generated=${phase.generatedRetired} ` +
    `output=${outputPath}`,
);
