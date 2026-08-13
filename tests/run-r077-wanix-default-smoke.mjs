#!/usr/bin/env node

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
  throw new Error("usage: node tests/run-r077-wanix-default-smoke.mjs OUTPUT.json");
}
const outputPath = resolve(process.argv[2]);
assert.equal(existsSync(outputPath), false, "output already exists");

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const harness = join(root, "tests/wanix-v86-preboot-smoke.mjs");
const pagePath = join(root, "integrations/wanix/v86-rv64-three-way-r077-production.html");
const url = "http://127.0.0.1:8765/examples/v86-rv64-three-way-r077-production.html";
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
  page: "9199655ff6df7eb5f7d6077c2f85bf8a327a885fcdc270130ec57285145e4dd8",
  rv64Jit: "9f28e71af658fef6a32da9c3682f7a8b4a34c83049515dd44fd6df756ab1ead6",
  rv64Root: "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb",
};

const page = readFileSync(pagePath, "utf8");
const harnessSource = readFileSync(harness, "utf8");
assert.equal(sha256(page), expectedArtifacts.page, "workspace page changed");
assert.match(page, /rv64-jit-r077-9f28e71af658fef6\.tgz/);
assert.doesNotMatch(page, /rv64\.static-t0=/, "default smoke page contains an override");
assert.match(
  harnessSource,
  /const key = benchRepetitions === 1 \? phase : `\$\{phase\}\$\{repetition \+ 1\}`/,
  "WANIX phase-key contract changed",
);

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
assert.equal(run.status, 0, `WANIX smoke failed:\n${run.stderr}\n${run.stdout.slice(-4000)}`);

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
assert.equal(atShell.sampledEnabled, true);
assert.equal(atShell.sampledBackoffEnabled, true);
assert.ok(atShell.moduleIndex >= 0);
assert.ok(BigInt(atShell.sampledRetired) > 0n);
assert.ok(BigInt(atShell.samples) > 0n);
assert.ok(BigInt(atShell.shortSampleMarks) > 0n);
assert.ok(BigInt(atShell.shortSampleBypasses) > 0n);
assert.equal(BigInt(atShell.systemErrors), 0n);

const phase = jit.jitPhases.python;
assert.ok(phase, "Python phase JIT proof missing");
assert.equal(phase.staticSystemT0Enabled, false);
assert.equal(phase.sampledStaticT0Enabled, true);
assert.equal(phase.sampledStaticT0BackoffEnabled, true);
assert.equal(phase.staticSystemT0ModuleIndex, atShell.moduleIndex);
assert.ok(BigInt(phase.sampledStaticT0Retired) > 0n);
assert.ok(BigInt(phase.sampledStaticT0Samples) > 0n);
assert.ok(BigInt(phase.sampledStaticT0ShortBypasses) > 0n);
assert.equal(BigInt(phase.staticT0Errors), 0n);
assert.ok(BigInt(phase.generatedRetired) > 0n);

const report = {
  schema: 1,
  experiment: "R077 WANIX production-default smoke",
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
  `R077 WANIX default smoke: PASS shell-sampled=${atShell.sampledRetired} ` +
    `python-sampled=${phase.sampledStaticT0Retired} output=${outputPath}`,
);
