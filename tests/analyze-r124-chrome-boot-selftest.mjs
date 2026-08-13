#!/usr/bin/env node

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const analyzer = join(root, "tests/analyze-r124-chrome-boot-pairs.mjs");
const scratch = mkdtempSync(join(tmpdir(), "rv64-r124-chrome-analyzer-"));
const browser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const guest = { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" };
const wasm = {
  control: "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
  candidate: "d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59",
};
const kernel = "57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2";
const initramfs = "cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808";
const order = Array.from({ length: 7 }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "candidate"] : ["candidate", "control"],
}));
const policy = {
  enabled: "1", threshold: "131072", privilegedThresholdMultiplier: "32",
  quantum: "1024", controlEntriesEnabled: "1", privilegedControlEntriesEnabled: "0",
  stableChainEnabled: "1", inflightLimit: "2", multiPageControlPermille: "100",
  pageCap: "2", leaderCap: "512", tailChainEnabled: "1",
  regionTlbCacheEnabled: "1", regionTlbCacheMinAccesses: "4",
};

function protocol() {
  return {
    schema: 1,
    experiment: "R124 artifact A/B Chrome dual-clock modern Boot",
    plannedAt: "2026-08-10T12:00:00.000Z",
    pairs: 7,
    order,
    hostCpuAffinity: "8-15",
    expectedBrowser: browser,
    guest,
    thresholds: {
      minimumExecutionMedianSpeedup: 0.99,
      minimumExecutionBootstrapUpper: 1.00,
      minimumConstructionMedianSpeedup: 0.99,
      minimumConstructionBootstrapUpper: 1.00,
    },
    assetManifest: {
      kernel: { sha256: kernel },
      initramfs: { sha256: initramfs },
    },
    configurations: {},
    timerContract: {},
  };
}

function result(side, options) {
  const candidate = side === "candidate";
  const executionMs = candidate ? (options.executionRegression ? 1_020 : 980) : 1_000;
  const constructionToMarkerMs = candidate
    ? (options.constructionRegression ? 1_070 : options.executionRegression ? 1_040 : 1_010)
    : 1_050;
  const instructions = 180_000_000n;
  const generated = 90_000_000n;
  return {
    schema: 1,
    experiment: "R124 artifact A/B Chrome dual-clock modern Boot",
    variant: side,
    userAgent: browser.userAgent,
    hardwareConcurrency: 8,
    assetHashes: { wasm: wasm[side], kernel, initramfs },
    guest,
    outputSha256: "0".repeat(64),
    outputMarkers: {
      ready: true,
      guest: "SCORECARD_V2_GUEST linux=6.12.7 alpine=3.24.1 arch=riscv64",
    },
    timing: {
      ms: executionMs,
      constructionToMarkerMs,
      pumps: 100,
      yields: 25,
      quantum: "2000000",
      cadence: "yield-after-pump-1-then-every-fourth",
      marker: "SCORECARD_V2_READY",
    },
    instructions: instructions.toString(),
    counters: {
      generated: generated.toString(),
      interpreted: (instructions - generated).toString(),
      dispatches: "1",
      staticFast: "0",
      sampled: "0",
      errors: "0",
    },
    policy,
    modules: { count: 1, bytes: 1, kinds: { region: 1 } },
  };
}

function writeFixture(name, options = {}) {
  const directory = join(scratch, name);
  mkdirSync(directory);
  writeFileSync(join(directory, "protocol.json"), `${JSON.stringify(protocol(), null, 2)}\n`);
  let sequence = 0;
  for (const { pair, sides } of order) {
    for (const side of sides) {
      const started = Date.parse("2026-08-10T12:00:01.000Z") + sequence++ * 2_000;
      writeFileSync(join(directory, `pair-${pair}-${side}.json`), `${JSON.stringify({
        startedAt: new Date(started).toISOString(),
        finishedAt: new Date(started + 1_000).toISOString(),
        browser,
        hostCpuAffinity: "8-15",
        result: result(side, options),
      }, null, 2)}\n`);
    }
  }
  return directory;
}

function analyze(name, options = {}) {
  const directory = writeFixture(name, options);
  const output = join(scratch, `${name}.json`);
  const child = spawnSync(process.execPath, [analyzer, directory, `--output=${output}`], {
    cwd: root,
    encoding: "utf8",
  });
  return { child, report: JSON.parse(readFileSync(output, "utf8")) };
}

try {
  const passing = analyze("passing");
  assert.equal(passing.child.status, 0, passing.child.stderr);
  assert.equal(passing.report.gatePassed, true);

  const execution = analyze("execution-regression", { executionRegression: true });
  assert.equal(execution.child.status, 1);
  assert.match(execution.child.stderr, /execution speedup/);
  assert.equal(execution.report.gatePassed, false);

  const construction = analyze("construction-regression", { constructionRegression: true });
  assert.equal(construction.child.status, 1);
  assert.match(construction.child.stderr, /construction speedup/);
  assert.equal(construction.report.gatePassed, false);

  console.log("R124 Chrome analyzer selftest: both protected R107 clocks enforced");
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
