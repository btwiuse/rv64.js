#!/usr/bin/env node

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const analyzer = join(root, "tests/analyze-wanix-r075-pairs.mjs");
const verifier = join(root, "tests/vs-v86/r075-browser-gate.mjs");
const scratch = mkdtempSync(join(tmpdir(), "rv64-r075-analyzer-selftest-"));
const wasm = "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c";
const archive = "e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c";
const harness = "525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545";
const rv64Root = "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb";
const x86Root = "09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320";
const v86Archive = "7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771";
const pages = {
  control: "a2f43c06f86507c267c36fb3922079d11d44072ab0622526e5577f69448e976f",
  candidate: "7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413",
};
const urls = {
  control:
    "http://127.0.0.1:8765/examples/" +
    "v86-rv64-three-way-r075-control-e0c1971d1ecd4d4f.html",
  candidate:
    "http://127.0.0.1:8765/examples/" +
    "v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html",
};
const configurations = {
  control: {
    staticSystemT0: false,
    sampledStaticT0: false,
    sampledStaticT0Backoff: false,
  },
  candidate: {
    staticSystemT0: false,
    sampledStaticT0: true,
    sampledStaticT0Backoff: true,
  },
};
const browser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const phases = ["python", "sha256", "shared9p"];
const order = Array.from({ length: 7 }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "candidate"] : ["candidate", "control"],
}));

function artifact(source, url, sha256) {
  return { source, url, bytes: 1, sha256 };
}

function proof(candidate) {
  const active = candidate ? "1" : "0";
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
    chainHops: "1",
    generatedRetired: "100",
    generatedCoverage: 0.99,
    staticT0Errors: "0",
    staticSystemT0ModuleIndex: 822,
    staticSystemT0Enabled: false,
    sampledStaticT0Enabled: candidate,
    sampledStaticT0BackoffEnabled: candidate,
    staticT0FastRetired: active,
    sampledStaticT0Retired: active,
    sampledStaticT0Samples: active,
    sampledStaticT0InterruptPolls: active,
    sampledStaticT0ShortMarks: active,
    sampledStaticT0ShortBypasses: active,
  };
}

function protocol() {
  return {
    schema: 5,
    experiment: "R075 preboot sampled-backoff browser candidate/control",
    plannedAt: "2026-08-09T10:00:00.000Z",
    pairs: 7,
    maxSlowdown: 0.03,
    urls,
    cpuAffinity: "8-15",
    expectedBrowser: browser,
    vm: "rv64-jit",
    phases,
    repetitions: 3,
    phaseSync: true,
    configurations,
    expectedMainWasmSha256: wasm,
    expectedPageSha256: pages,
    expectedRv64ArchiveSha256: archive,
    expectedHarnessSha256: harness,
    maximumWithinSideSpread: null,
    maximumPairedMedianBootstrapUpper: 1.10,
    minimumShellSpeedup: 1.10,
    minimumShellMedianBootstrapLower: 1.00,
    jitConfigurationLifecycle: "preboot",
    minimumGeneratedCoverage: { python: 0.90, sha256: 0.90, shared9p: 0 },
    requiredCandidateProof: [
      "staticT0FastRetired",
      "sampledStaticT0Retired",
      "sampledStaticT0Samples",
      "sampledStaticT0InterruptPolls",
      "sampledStaticT0ShortMarks",
      "sampledStaticT0ShortBypasses",
    ],
    order,
  };
}

function writeFixture(name, options = {}) {
  const directory = join(scratch, name);
  mkdirSync(directory);
  writeFileSync(join(directory, "protocol.json"), `${JSON.stringify(protocol(), null, 2)}\n`);
  let sequence = 0;
  for (const { pair, sides } of order) {
    for (const side of sides) {
      const candidate = side === "candidate";
      const started = Date.parse("2026-08-09T10:00:01.000Z") + sequence * 2_000;
      const finished = started + 1_000;
      sequence++;
      const phaseRatio = candidate && options.slowPhase === "shared9p" ? 1.04 : 1;
      const samples = {
        python: [1, 1, 1],
        sha256: [1, 1, 1],
        shared9p: [phaseRatio, phaseRatio, phaseRatio],
      };
      const summary = {
        vm: "rv64-jit",
        startedAt: new Date(started).toISOString(),
        finishedAt: new Date(finished).toISOString(),
        hostCpuAffinity: "8-15",
        browser,
        artifactStable: true,
        artifacts: {
          page: { url: urls[side], bytes: 1, sha256: pages[side] },
          archives: {
            rv64Jit: artifact("../rv64/rv64-jit.tgz", "http://host/rv64-jit.tgz", archive),
            rv64Root: artifact("../rv64/root.tgz", "http://host/rv64-root.tgz", rv64Root),
            v86: artifact("../v86/v86.tgz", "http://host/v86.tgz", v86Archive),
            x86Root: artifact("../x86/root.tgz", "http://host/x86-root.tgz", x86Root),
          },
        },
        guest: {
          machine: "riscv64",
          alpineRelease: "3.22.5",
          pythonVersion: "3.12.13",
        },
        phases,
        repetitions: 3,
        phaseSync: true,
        jitPolicy: null,
        jitConfiguration: configurations[side],
        jitConfigurationLifecycle: "preboot",
        shellMs: candidate ? (options.slowShell ? 28_800 : 25_000) : 30_000,
        samples,
        correctness: Object.fromEntries(phases.map((phase) => [
          phase,
          { complete: 3, correct: 3 },
        ])),
      };
      const shellActive = candidate ? "1" : "0";
      const shell = {
        systemEnabled: false,
        sampledEnabled: candidate,
        sampledBackoffEnabled: candidate,
        systemErrors: "0",
        moduleIndex: 822,
        sampledRetired: shellActive,
        samples: shellActive,
        interruptPolls: shellActive,
        shortSampleMarks: shellActive,
        shortSampleBypasses:
          candidate && options.missingShellBypass ? "0" : shellActive,
      };
      const jitPhases = {};
      for (const phase of phases) {
        for (let repetition = 1; repetition <= 3; repetition++) {
          jitPhases[`${phase}${repetition}`] = proof(candidate);
        }
      }
      const text = [
        JSON.stringify(summary),
        JSON.stringify({ jitAtShell: { staticT0: shell }, jitPhases }),
        "riscv64 Python 3.12.13",
        "checksum=38460b78",
        "e09320c5b00b34bb",
        "",
      ].join("\n");
      writeFileSync(join(directory, `pair-${pair}-${side}.log`), text);
    }
  }
  return directory;
}

function analyze(directory, report) {
  return spawnSync(process.execPath, [analyzer, directory, `--output=${report}`], {
    cwd: root,
    encoding: "utf8",
  });
}

try {
  const passing = writeFixture("passing");
  const passingReport = join(scratch, "passing-analysis.json");
  const pass = analyze(passing, passingReport);
  assert.equal(pass.status, 0, pass.stderr);
  const passReport = JSON.parse(readFileSync(passingReport, "utf8"));
  assert.equal(passReport.measurementValid, true);
  assert.equal(passReport.gatePassed, true);
  assert.equal(passReport.shell.pairedMedian, 1.2);
  const verifyPass = spawnSync(process.execPath, [verifier, passingReport], {
    cwd: root,
    encoding: "utf8",
  });
  assert.equal(verifyPass.status, 0, verifyPass.stderr);

  const slowShell = writeFixture("slow-shell", { slowShell: true });
  const slowShellReport = join(scratch, "slow-shell-analysis.json");
  const shellFailure = analyze(slowShell, slowShellReport);
  assert.equal(shellFailure.status, 1);
  assert.match(shellFailure.stderr, /shell: paired speedup/);
  assert.equal(JSON.parse(readFileSync(slowShellReport, "utf8")).measurementValid, true);

  const slowPhase = writeFixture("slow-phase", { slowPhase: "shared9p" });
  const slowPhaseReport = join(scratch, "slow-phase-analysis.json");
  const phaseFailure = analyze(slowPhase, slowPhaseReport);
  assert.equal(phaseFailure.status, 1);
  assert.match(phaseFailure.stderr, /shared9p: paired median/);

  const invalidLifecycle = writeFixture("invalid-lifecycle", { missingShellBypass: true });
  const lifecycleFailure = analyze(
    invalidLifecycle,
    join(scratch, "invalid-lifecycle-analysis.json"),
  );
  assert.equal(lifecycleFailure.status, 1);
  assert.match(lifecycleFailure.stderr, /shell shortSampleBypasses/);

  console.log(
    "R075 analyzer selftest: pass verdict, shell gate, phase gate, and lifecycle invalidation enforced",
  );
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
