#!/usr/bin/env node

// Structural tests for the WANIX parity evidence gate. Synthetic timing data
// is intentionally boring; this exercises rejection of mixed artifacts,
// missing affinity, chronology violations, and post-hoc sample extension.

import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const analyzer = fileURLToPath(new URL("./analyze-wanix-pairs.mjs", import.meta.url));
const pageUrl = "http://127.0.0.1:8765/examples/v86-rv64-three-way.html";
const plannedAt = Date.parse("2026-08-07T00:00:00.000Z");
const phases = ["python", "sha256", "shared9p"];
const order = [
  { pair: 1, vms: ["rv64-jit", "x86"] },
  { pair: 2, vms: ["x86", "rv64-jit"] },
  { pair: 3, vms: ["rv64-jit", "x86"] },
];
const artifacts = {
  page: { url: pageUrl, bytes: 12000, sha256: "1".repeat(64) },
  archives: {
    rv64Jit: { source: "../rv64/rv64-jit.tgz?v=test", url: "http://127.0.0.1:8765/rv64/rv64-jit.tgz?v=test", bytes: 1, sha256: "2".repeat(64) },
    rv64Root: { source: "../extras/dist/wanix-linux-rv64.tgz", url: "http://127.0.0.1:8765/extras/dist/wanix-linux-rv64.tgz", bytes: 1, sha256: "3".repeat(64) },
    v86: { source: "../v86/v86.tgz", url: "http://127.0.0.1:8765/v86/v86.tgz", bytes: 1, sha256: "4".repeat(64) },
    x86Root: { source: "../extras/dist/wanix-linux-x86.tgz", url: "http://127.0.0.1:8765/extras/dist/wanix-linux-x86.tgz", bytes: 1, sha256: "5".repeat(64) },
  },
};

function makeFixture(mutate = () => {}) {
  const directory = mkdtempSync(join(tmpdir(), "wanix-pair-analysis-"));
  writeFileSync(join(directory, "protocol.json"), `${JSON.stringify({
    schema: 2,
    plannedAt: new Date(plannedAt).toISOString(),
    pairs: 3,
    maxSlowdown: 0.10,
    url: pageUrl,
    cpuAffinity: "8-15",
    browserExecutable: null,
    phases,
    repetitions: 1,
    phaseSync: true,
    jitPolicy: null,
    jitConfiguration: {},
    minimumGeneratedCoverage: {
      python: 0.90,
      sha256: 0.90,
      shared9p: 0,
    },
    order,
  }, null, 2)}\n`);

  let sequence = 0;
  for (const { pair, vms } of order) {
    for (const vm of vms) {
      const started = plannedAt + (++sequence * 2_000);
      const summary = {
        vm,
        startedAt: new Date(started).toISOString(),
        finishedAt: new Date(started + 1_000).toISOString(),
        hostCpuAffinity: "8-15",
        browser: { product: "Chrome/150.0.test", jsVersion: "15.0.test" },
        artifactStable: true,
        artifacts: structuredClone(artifacts),
        phases,
        repetitions: 1,
        phaseSync: true,
        jitPolicy: null,
        jitConfiguration: {},
        samples: { python: [1], sha256: [2], shared9p: [1] },
      };
      const jit = {
        jitPhases: Object.fromEntries(phases.map((phase) => [phase, {
          pagePolicyEnabled: "1",
          pageThreshold: "131072",
          privilegedPageThresholdMultiplier: "32",
          pageQuantum: "1024",
          controlEntriesEnabled: "1",
          privilegedControlEntriesEnabled: "0",
          stablePageChainEnabled: "1",
          pageInflightLimit: "2",
          regionPageCap: "2",
          regionLeaderCap: "512",
          regionTailChainEnabled: "1",
          regionTlbCacheEnabled: "1",
          regionTlbCacheMinAccesses: "4",
          multiPageEntryCap: "512",
          multiPageControlPermille: "100",
          controlProfileEnabled: "0",
          chainHops: "1",
          generatedRetired: "1000",
          generatedCoverage: 0.95,
        }])),
      };
      mutate(summary, { pair, vm }, jit);
      const terminal = [
        "checksum=38460b78",
        "e09320c5b00b34bb",
        "3.22.5",
        vm === "x86" ? "i686 Python 3.12.13" : "riscv64 Python 3.12.13",
      ].join("\n");
      const lines = [JSON.stringify(summary)];
      if (vm === "rv64-jit") lines.push(JSON.stringify(jit));
      lines.push(terminal, "");
      writeFileSync(join(directory, `pair-${pair}-${vm}.log`), lines.join("\n"));
    }
  }
  return directory;
}

function analyze(directory) {
  return spawnSync(process.execPath, [analyzer, directory], { encoding: "utf8" });
}

const directories = [];
try {
  const valid = makeFixture();
  directories.push(valid);
  const validResult = analyze(valid);
  assert.equal(validResult.status, 0, validResult.stderr);
  assert.equal(JSON.parse(validResult.stdout).phases.shared9p.noninferiorAtRequestedMargin, true);

  const mixed = makeFixture((summary, run) => {
    if (run.pair === 3 && run.vm === "x86") {
      summary.artifacts.archives.rv64Jit.sha256 = "f".repeat(64);
    }
  });
  directories.push(mixed);
  const mixedResult = analyze(mixed);
  assert.notEqual(mixedResult.status, 0);
  assert.match(mixedResult.stderr, /artifact changed between samples/);

  const reordered = makeFixture((summary, run) => {
    if (run.pair === 2 && run.vm === "x86") {
      summary.startedAt = "2026-08-07T00:00:20.000Z";
      summary.finishedAt = "2026-08-07T00:00:21.000Z";
    }
  });
  directories.push(reordered);
  const reorderedResult = analyze(reordered);
  assert.notEqual(reorderedResult.status, 0);
  assert.match(reorderedResult.stderr, /pre-registered run order/);

  const unpinned = makeFixture((summary, run) => {
    if (run.pair === 1 && run.vm === "rv64-jit") summary.hostCpuAffinity = null;
  });
  directories.push(unpinned);
  const unpinnedResult = analyze(unpinned);
  assert.notEqual(unpinnedResult.status, 0);
  assert.match(unpinnedResult.stderr, /CPU affinity/);

  const changedPolicy = makeFixture((_summary, run, jit) => {
    if (run.pair === 1 && run.vm === "rv64-jit") {
      jit.jitPhases.python.privilegedPageThresholdMultiplier = "1";
    }
  });
  directories.push(changedPolicy);
  const changedPolicyResult = analyze(changedPolicy);
  assert.notEqual(changedPolicyResult.status, 0);
  assert.match(changedPolicyResult.stderr, /privileged threshold multiplier/);

  const extended = makeFixture();
  directories.push(extended);
  writeFileSync(join(extended, "pair-4-rv64-jit.log"), "post-hoc sample\n");
  writeFileSync(join(extended, "pair-4-x86.log"), "post-hoc sample\n");
  const extendedResult = analyze(extended);
  assert.notEqual(extendedResult.status, 0);
  assert.match(extendedResult.stderr, /pre-registered protocol/);
} finally {
  for (const directory of directories) rmSync(directory, { recursive: true, force: true });
}

console.log("wanix pair analyzer selftest: artifact, affinity, chronology, and sample plan enforced");
