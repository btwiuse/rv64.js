#!/usr/bin/env node

// Synthetic decision tests for the frozen R125 WANIX analyzer. No guest or
// browser work is performed and none of these values are performance evidence.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const analyzer = resolve(root, "tests/analyze-r125-wanix-browser-pairs.mjs");
const plannedAt = Date.parse("2026-08-10T00:00:00.000Z");
const browser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent: "synthetic-selftest",
  jsVersion: "15.0.245.21",
};
const phases = ["python", "sha256", "shared9p"];
const thresholds = {
  minimumPairedMedianSpeedup: 0.99,
  minimumPairedBootstrapUpper: 1.00,
  minimumPhaseBootstrapLower: 1 / 1.10,
  maximumWithinBrowserSpread: 1.25,
  minimumShared9pSampleSeconds: 2.0,
  exactP9WriteBytes: 33_554_432,
  minimumP9ReadBytes: 33_554_432,
  maximumP9TransferBytes: 4_096,
  minimumGeneratedCoverage: { python: 0.90, sha256: 0.90, shared9p: 0 },
};
const expectedPhasePolicy = {
  pagePolicyEnabled: "1",
  pageThreshold: "131072",
  privilegedPageThresholdMultiplier: "32",
  pageQuantum: "1024",
  controlEntriesEnabled: "1",
  privilegedControlEntriesEnabled: "0",
  stablePageChainEnabled: "1",
  pageInflightLimit: "2",
  multiPageEntryCap: "512",
  multiPageControlPermille: "100",
  controlProfileEnabled: "0",
  regionPageCap: "2",
  regionLeaderCap: "512",
  regionTailChainEnabled: "1",
  regionTlbCacheEnabled: "1",
  regionTlbCacheMinAccesses: "4",
};
const order = Array.from({ length: 7 }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "candidate"] : ["candidate", "control"],
}));
const pages = {
  control: { sha256: "1".repeat(64) },
  candidate: { sha256: "2".repeat(64) },
};
const archives = {
  control: {
    sha256: "3".repeat(64),
    members: {
      loader: { sha256: "4".repeat(64) },
      adapter: { sha256: "5".repeat(64) },
      rv64Wasm: { sha256: "6".repeat(64) },
    },
  },
  candidate: {
    sha256: "7".repeat(64),
    members: {
      loader: { sha256: "4".repeat(64) },
      adapter: { sha256: "5".repeat(64) },
      rv64Wasm: { sha256: "8".repeat(64) },
    },
  },
};
const commonArchiveSha256 = {
  v86: "9".repeat(64), rv64Root: "a".repeat(64), x86Root: "b".repeat(64),
};

function makeProof(phase) {
  return {
    instructions: "1000",
    generatedRetired: "950",
    interpreterRetired: "50",
    generatedCoverage: 0.95,
    ...expectedPhasePolicy,
    p9WriteBytes: phase === "shared9p" ? 33_554_432 : 0,
    p9ReadBytes: phase === "shared9p" ? 33_558_528 : 0,
    p9MaximumWrite: phase === "shared9p" ? 4_096 : 0,
    p9MaximumRead: phase === "shared9p" ? 4_096 : 0,
    staticT0FastRetired: "0",
    staticT0SlowRetired: "0",
    staticT0Errors: "0",
    sampledStaticT0Retired: "0",
    sampledStaticT0Samples: "0",
  };
}

function makeFixture(mutate = () => {}) {
  const directory = mkdtempSync(join(tmpdir(), "r125-wanix-analyzer-"));
  const protocol = {
    schema: 1,
    experiment: "R125 fixed-bank WANIX browser guard with single-flight adapter",
    plannedAt: new Date(plannedAt).toISOString(),
    pairs: 7,
    repetitions: 3,
    order,
    hostCpuAffinity: "8-15",
    expectedBrowser: browser,
    phases,
    configurations: { control: {}, candidate: {} },
    pages,
    archives,
    commonArchiveSha256,
    integrationGuest: { linux: "6.12.7", alpine: "3.22.5", python: "3.12.13" },
    deployed: {},
    thresholds,
  };
  writeFileSync(join(directory, "protocol.json"), `${JSON.stringify(protocol)}\n`);

  let sequence = 0;
  for (const { pair, sides } of order) {
    for (const side of sides) {
      const started = plannedAt + (++sequence * 2_000);
      const speedup = 1.01;
      const multiplier = side === "control" ? 1 : 1 / speedup;
      const summary = {
        vm: "rv64-jit",
        startedAt: new Date(started).toISOString(),
        finishedAt: new Date(started + 1_000).toISOString(),
        hostCpuAffinity: "8-15",
        browser,
        artifactStable: true,
        artifacts: {
          page: { sha256: pages[side].sha256 },
          archives: {
            rv64Jit: { sha256: archives[side].sha256 },
            v86: { sha256: commonArchiveSha256.v86 },
            rv64Root: { sha256: commonArchiveSha256.rv64Root },
            x86Root: { sha256: commonArchiveSha256.x86Root },
          },
        },
        guest: { machine: "riscv64", alpineRelease: "3.22.5", pythonVersion: "3.12.13" },
        phases,
        repetitions: 3,
        phaseSync: true,
        jitConfiguration: {},
        jitPolicy: null,
        shellMs: 100 * multiplier,
        benchMs: 100,
        samples: {
          python: [2 * multiplier, 2 * multiplier, 2 * multiplier],
          sha256: [4 * multiplier, 4 * multiplier, 4 * multiplier],
          shared9p: [24 * multiplier, 24 * multiplier, 24 * multiplier],
        },
        correctness: Object.fromEntries(phases.map((phase) =>
          [phase, { complete: 3, correct: 3 }])),
      };
      const jit = {
        jitAtShell: {
          loader: { modules: 1 },
          staticT0: { supported: false },
          pagePolicy: {
            enabled: "1", threshold: "131072",
            regionTlbCacheEnabled: "1", regionTlbCacheMinAccesses: "4",
          },
        },
        jitPhases: Object.fromEntries(phases.flatMap((phase) =>
          [1, 2, 3].map((repetition) => [`${phase}${repetition}`, makeProof(phase)]))),
      };
      mutate({ summary, jit, pair, side });
      writeFileSync(join(directory, `pair-${pair}-${side}.log`),
        `${JSON.stringify(summary)}\n${JSON.stringify(jit)}\n`);
    }
  }
  return directory;
}

function analyze(directory, suffix) {
  const output = join(directory, `${suffix}.json`);
  return spawnSync(process.execPath, [analyzer, directory, `--output=${output}`], {
    encoding: "utf8",
  });
}

const directories = [];
try {
  const valid = makeFixture();
  directories.push(valid);
  const validResult = analyze(valid, "valid");
  assert.equal(validResult.status, 0, validResult.stderr);
  assert.match(validResult.stdout, /R125_WANIX_BROWSER_GATE_PASS/);

  const establishedRegression = makeFixture(({ summary, side }) => {
    if (side !== "candidate") return;
    summary.shellMs = 100 / 0.995;
    for (const phase of phases) summary.samples[phase] = summary.samples[phase].map(
      (_value, index) => ({ python: 2, sha256: 4, shared9p: 24 }[phase] / 0.995),
    );
  });
  directories.push(establishedRegression);
  const regressionResult = analyze(establishedRegression, "regression");
  assert.notEqual(regressionResult.status, 0);
  assert.match(regressionResult.stderr, /upper .* establishes regression/);

  const badP9 = makeFixture(({ jit, pair, side }) => {
    if (pair === 1 && side === "candidate") jit.jitPhases.shared9p1.p9WriteBytes--;
  });
  directories.push(badP9);
  const badP9Result = analyze(badP9, "bad-p9");
  assert.notEqual(badP9Result.status, 0);
  assert.match(badP9Result.stderr, /P9 write bytes/);

  const inactiveJit = makeFixture(({ jit, pair, side }) => {
    if (pair === 1 && side === "candidate") jit.jitPhases.python1.generatedRetired = "0";
  });
  directories.push(inactiveJit);
  const inactiveResult = analyze(inactiveJit, "inactive-jit");
  assert.notEqual(inactiveResult.status, 0);
  assert.match(inactiveResult.stderr, /generated > 0n/);

  const unstable = makeFixture(({ summary, pair, side }) => {
    if (pair === 1 && side === "candidate") summary.samples.sha256[2] *= 1.30;
  });
  directories.push(unstable);
  const unstableResult = analyze(unstable, "unstable");
  assert.notEqual(unstableResult.status, 0);
  assert.match(unstableResult.stderr, /within-browser spread/);
} finally {
  for (const directory of directories) rmSync(directory, { recursive: true, force: true });
}

console.log("R125 WANIX analyzer selftest: pass, confidence regression, P9 bytes, active JIT, and spread guards enforced");
