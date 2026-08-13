#!/usr/bin/env node

// Synthetic structural tests for R076's fixed seven-pair, three-repetition
// candidate/v86 guard. These fixtures contain no measured performance data.

import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const analyzer = fileURLToPath(
  new URL("./analyze-r076-wanix-v86-pairs.mjs", import.meta.url),
);
const verifier = fileURLToPath(
  new URL("./vs-v86/r076-wanix-v86-gate.mjs", import.meta.url),
);
const url =
  "http://127.0.0.1:8765/examples/" +
  "v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html";
const browser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const configuration = {
  staticSystemT0: false,
  sampledStaticT0: true,
  sampledStaticT0Backoff: true,
};
const digests = {
  page: "7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413",
  rv64Jit: "e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c",
  rv64Root: "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb",
  v86: "7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771",
  x86Root: "09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320",
};
const phases = ["python", "sha256", "shared9p"];
const order = Array.from({ length: 7 }, (_, index) => ({
  pair: index + 1,
  vms: index % 2 === 0 ? ["rv64-jit", "x86"] : ["x86", "rv64-jit"],
}));
const plannedAt = Date.parse("2026-08-09T12:00:00.000Z");
const artifacts = {
  page: { url, bytes: 12045, sha256: digests.page },
  archives: {
    rv64Jit: {
      source: "../rv64/rv64-jit-r075-e0c1971d1ecd4d4f.tgz",
      url: "http://127.0.0.1:8765/rv64/rv64-jit-r075-e0c1971d1ecd4d4f.tgz",
      bytes: 1831853,
      sha256: digests.rv64Jit,
    },
    rv64Root: {
      source: "../extras/dist/wanix-linux-rv64.tgz",
      url: "http://127.0.0.1:8765/extras/dist/wanix-linux-rv64.tgz",
      bytes: 28069154,
      sha256: digests.rv64Root,
    },
    v86: {
      source: "../v86/v86.tgz",
      url: "http://127.0.0.1:8765/v86/v86.tgz",
      bytes: 1680818,
      sha256: digests.v86,
    },
    x86Root: {
      source: "../extras/dist/wanix-linux-x86.tgz",
      url: "http://127.0.0.1:8765/extras/dist/wanix-linux-x86.tgz",
      bytes: 28785067,
      sha256: digests.x86Root,
    },
  },
};

function phaseProof() {
  return {
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
    chainHops: "10",
    generatedRetired: "1000",
    generatedCoverage: 0.95,
    staticSystemT0Enabled: false,
    sampledStaticT0Enabled: true,
    sampledStaticT0BackoffEnabled: true,
    staticSystemT0ModuleIndex: 822,
    staticT0Errors: "0",
    staticT0FastRetired: "100",
    sampledStaticT0Retired: "100",
    sampledStaticT0Samples: "10",
    sampledStaticT0InterruptPolls: "10",
    sampledStaticT0ShortMarks: "2",
    sampledStaticT0ShortBypasses: "3",
  };
}

function makeFixture(mutate = () => {}) {
  const directory = mkdtempSync(join(tmpdir(), "r076-wanix-v86-analysis-"));
  const protocol = {
    schema: 3,
    experiment: "R076 preboot sampled-backoff candidate versus copy/v86",
    plannedAt: new Date(plannedAt).toISOString(),
    pairs: 7,
    maxSlowdown: 0.10,
    url,
    cpuAffinity: "8-15",
    browserExecutable: "/usr/bin/google-chrome",
    expectedBrowser: browser,
    phases,
    repetitions: 3,
    phaseSync: true,
    jitPolicy: null,
    jitConfiguration: configuration,
    expectedMainWasmSha256:
      "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c",
    expectedArtifacts: digests,
    maximumPairedMedianBootstrapUpper: 1.10,
    jitConfigurationLifecycle: "preboot",
    requiredCandidateProof: [
      "staticT0FastRetired",
      "sampledStaticT0Retired",
      "sampledStaticT0Samples",
      "sampledStaticT0InterruptPolls",
      "sampledStaticT0ShortMarks",
      "sampledStaticT0ShortBypasses",
    ],
    toolManifest: {
      runner: "8d37e7b20186253a0b7e71e5b7c28f3d8ee3b34a49a7eb4374553c5b80ee4e80",
      harness: "525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545",
    },
    minimumGeneratedCoverage: { python: 0.90, sha256: 0.90, shared9p: 0 },
    order,
  };
  writeFileSync(join(directory, "protocol.json"), `${JSON.stringify(protocol, null, 2)}\n`);

  let sequence = 0;
  for (const { pair, vms } of order) {
    for (const vm of vms) {
      const started = plannedAt + (++sequence * 2_000);
      const rv64 = vm === "rv64-jit";
      const value = rv64 ? 0.80 : 1.00;
      const summary = {
        vm,
        startedAt: new Date(started).toISOString(),
        finishedAt: new Date(started + 1_000).toISOString(),
        hostCpuAffinity: "8-15",
        browser,
        artifactStable: true,
        artifacts: structuredClone(artifacts),
        guest: {
          machine: rv64 ? "riscv64" : "i686",
          alpineRelease: "3.22.5",
          pythonVersion: "3.12.13",
        },
        phases,
        repetitions: 3,
        phaseSync: true,
        jitPolicy: null,
        jitConfiguration: rv64 ? configuration : {},
        jitConfigurationLifecycle: rv64 ? "preboot" : "shell",
        samples: Object.fromEntries(phases.map((phase) => [phase, [value, value, value]])),
        correctness: Object.fromEntries(phases.map((phase) => [phase, {
          complete: 3,
          correct: 3,
        }])),
        ...(rv64 ? {} : {
          externalP9: {
            requests: 30, replies: 30, pending: 0, tagCollisions: 0,
          },
          externalP9Phases: Object.fromEntries([1, 2, 3].map((repetition) => [
            `shared9p${repetition}`,
            { requests: 10, replies: 10, pending: 0, tagCollisions: 0 },
          ])),
        }),
      };
      const jit = rv64 ? {
        jitAtShell: {
          staticT0: {
            systemEnabled: false,
            sampledEnabled: true,
            sampledBackoffEnabled: true,
            moduleIndex: 822,
            systemErrors: "0",
            sampledRetired: "100",
            samples: "10",
            interruptPolls: "10",
            shortSampleMarks: "2",
            shortSampleBypasses: "3",
          },
        },
        jitPhases: Object.fromEntries(phases.flatMap((phase) =>
          [1, 2, 3].map((repetition) => [`${phase}${repetition}`, phaseProof()]))),
      } : null;
      if (pair === 3 && vm === "x86") {
        summary.externalP9Phases.shared9p1.replies = 9;
        summary.externalP9Phases.shared9p1.pending = 1;
      }
      mutate({ summary, jit, pair, vm });
      const lines = [JSON.stringify(summary)];
      if (jit) lines.push(JSON.stringify(jit));
      lines.push("checksum=38460b78", "e09320c5b00b34bb", "");
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
  const validReport = JSON.parse(validResult.stdout);
  assert.equal(validReport.measurementValid, true);
  assert.equal(validReport.gatePassed, true);
  assert.equal(validReport.phases.python.pairedMedian, 0.8);
  const validAnalysis = join(valid, "analysis.json");
  writeFileSync(validAnalysis, validResult.stdout);
  const validVerification = spawnSync(process.execPath, [verifier, validAnalysis], {
    encoding: "utf8",
  });
  assert.equal(validVerification.status, 0, validVerification.stderr);

  const slow = makeFixture(({ summary, vm }) => {
    if (vm === "rv64-jit") {
      summary.samples = Object.fromEntries(phases.map((phase) => [phase, [1.11, 1.11, 1.11]]));
    }
  });
  directories.push(slow);
  const slowResult = analyze(slow);
  assert.notEqual(slowResult.status, 0);
  assert.match(slowResult.stderr, /noninferiority margin failed/);
  const slowReport = JSON.parse(slowResult.stdout);
  assert.equal(slowReport.measurementValid, true);
  assert.equal(slowReport.gatePassed, false);
  const slowAnalysis = join(slow, "analysis.json");
  writeFileSync(slowAnalysis, slowResult.stdout);
  const slowVerification = spawnSync(process.execPath, [verifier, slowAnalysis], {
    encoding: "utf8",
  });
  assert.notEqual(slowVerification.status, 0);
  assert.match(slowVerification.stderr, /R076_WANIX_V86_GATE_FAIL/);

  const lifecycle = makeFixture(({ jit, pair, vm }) => {
    if (pair === 4 && vm === "rv64-jit") {
      jit.jitAtShell.staticT0.sampledBackoffEnabled = false;
    }
  });
  directories.push(lifecycle);
  const lifecycleResult = analyze(lifecycle);
  assert.notEqual(lifecycleResult.status, 0);
  assert.match(lifecycleResult.stderr, /shell sampled-backoff enable/);

  const incomplete = makeFixture(({ summary, pair, vm }) => {
    if (pair === 2 && vm === "x86") summary.samples.python.pop();
  });
  directories.push(incomplete);
  const incompleteResult = analyze(incomplete);
  assert.notEqual(incompleteResult.status, 0);
  assert.match(incompleteResult.stderr, /python sample count/);

  const p9Boundary = makeFixture(({ summary, pair, vm }) => {
    if (pair === 4 && vm === "x86") {
      summary.externalP9Phases.shared9p2.replies = 8;
      summary.externalP9Phases.shared9p2.pending = 2;
    }
  });
  directories.push(p9Boundary);
  const p9BoundaryResult = analyze(p9Boundary);
  assert.notEqual(p9BoundaryResult.status, 0);
  assert.match(p9BoundaryResult.stderr, /shared9p2 pending boundary/);

  const mixed = makeFixture(({ summary, pair, vm }) => {
    if (pair === 7 && vm === "x86") summary.artifacts.archives.v86.sha256 = "f".repeat(64);
  });
  directories.push(mixed);
  const mixedResult = analyze(mixed);
  assert.notEqual(mixedResult.status, 0);
  assert.match(mixedResult.stderr, /v86 identity/);
} finally {
  for (const directory of directories) rmSync(directory, { recursive: true, force: true });
}

console.log(
  "R076 WANIX-v86 analyzer selftest: pass, 10% gate, lifecycle, repetitions, and artifacts enforced",
);
