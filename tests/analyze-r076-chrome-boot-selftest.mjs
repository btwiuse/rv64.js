#!/usr/bin/env node

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const analyzer = join(root, "tests/analyze-r076-chrome-boot-pairs.mjs");
const verifier = join(root, "tests/vs-v86/r076-chrome-boot-gate.mjs");
const scratch = mkdtempSync(join(tmpdir(), "rv64-r076-analyzer-selftest-"));
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
const configurations = {
  control: { staticSystemT0: false, sampledStaticT0: false, sampledStaticT0Backoff: false },
  candidate: { staticSystemT0: false, sampledStaticT0: true, sampledStaticT0Backoff: true },
};
const timerContract = {
  setupBeforeTimer: [
    "asset-fetch-and-sha256",
    "RV64.create",
    "bootVirtLinuxDirect",
    "static-module-prepare-and-enable",
    "initial-counters",
  ],
  timed: "runTimedBoot:first-2M-pump-through-SCORECARD_V2_READY",
  quantum: "2000000",
  cadence: "yield-after-pump-1-then-every-fourth",
  marker: "SCORECARD_V2_READY",
};
const hashes = {
  page: [1032, "bae81c20a73019dd57f5701330862f4a7b1c9d03feebbd1a7a718632e9c5d29b"],
  worker: [7367, "30c381a18b97ee9c24d54ee3a001c0232e4ff4d438036a9b782512417e60a883"],
  timingLibrary: [1131, "8d39c83ad01e1d1003bf56ded2439d3475f925f7925f0731da64b90170f9f548"],
  loader: [90264, "2582e18ea207ad0f5ec154e82d3fe6208faf5742f26a9d8463d8d360f168b776"],
  wasm: [4329839, "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c"],
  kernel: [4205056, "57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2"],
  initramfs: [64383488, "cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808"],
};
const routes = {
  page: "/jit-modern-boot-browser.html",
  worker: "/jit-modern-boot-browser-worker.mjs",
  timingLibrary: "/r076-browser-boot-lib.mjs",
  loader: "/web/rv64.js",
  wasm: "/rv64.wasm",
  kernel: "/kernel",
  initramfs: "/initramfs",
};
const assets = Object.fromEntries(Object.entries(hashes).map(([name, [bytes, sha256]]) => [
  name,
  { path: `/fixture/${name}`, route: routes[name], bytes, sha256 },
]));
const tools = {
  host: "fb05697dff42a57636c8965842f721964f444faa0b5db520c2a587f4c8c9d857",
  selftest: "516c3d1b4371fb56de8a5823c37942c11ce60cb897245827c5a95d52a4cb8995",
  runner: "d9b66a3b48a31a1154b827cb52c738319c99c5604bc98787d6c588451d024a70",
};
const order = Array.from({ length: 7 }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "candidate"] : ["candidate", "control"],
}));

function protocol() {
  return {
    schema: 1,
    experiment: "R076 Chrome execution-only modern Boot",
    plannedAt: "2026-08-09T12:00:00.000Z",
    pairs: 7,
    order,
    hostCpuAffinity: "8-15",
    expectedBrowser: browser,
    configurations,
    assetManifest: assets,
    toolManifest: tools,
    timerContract,
    guest,
    thresholds: {
      minimumExecutionSpeedup: 1.05,
      minimumExecutionBootstrapLower: 1.00,
      minimumMipsRatio: 1.00,
      minimumMipsBootstrapLower: 0.97,
    },
  };
}

function result(side, options) {
  const candidate = side === "candidate";
  const ms = candidate ? (options.slowExecution ? 970 : 900) : 1_000;
  const instructions = BigInt(
    candidate && options.lowCandidateInstructions ? 140_000_000 : 180_000_000,
  );
  const generated = 90_000_000n;
  const active = candidate && !options.missingSample ? "1" : "0";
  return {
    schema: 1,
    experiment: "R076 Chrome execution-only modern Boot",
    variant: side,
    userAgent: browser.userAgent,
    hardwareConcurrency: 8,
    assetHashes: {
      wasm: hashes.wasm[1],
      kernel: hashes.kernel[1],
      initramfs: hashes.initramfs[1],
    },
    guest,
    outputSha256: "0".repeat(64),
    outputMarkers: {
      ready: true,
      guest: "SCORECARD_V2_GUEST linux=6.12.7 alpine=3.24.1 arch=riscv64",
    },
    timerBoundary: {
      setupBeforeTimer: timerContract.setupBeforeTimer,
      timed: timerContract.timed,
    },
    timing: {
      ms,
      pumps: 100,
      yields: 25,
      quantum: timerContract.quantum,
      cadence: timerContract.cadence,
      marker: timerContract.marker,
    },
    instructions: instructions.toString(),
    mips: Number(instructions) / ms / 1_000,
    counters: {
      generated: generated.toString(),
      interpreted: (instructions - generated).toString(),
      dispatches: "1",
      staticFast: active,
      sampled: active,
      samples: active,
      polls: active,
      marks: active,
      bypasses: active,
      errors: "0",
    },
    staticLifecycle: {
      moduleIndex: 0,
      modulesBefore: 0,
      modulesAfter: 1,
      systemEnabled: false,
      sampledEnabled: candidate,
      sampledBackoffEnabled: candidate,
    },
    policy: {
      enabled: "1",
      threshold: "131072",
      privilegedThresholdMultiplier: "32",
      quantum: "1024",
      controlEntriesEnabled: "1",
      privilegedControlEntriesEnabled: "0",
      stableChainEnabled: "1",
      inflightLimit: "2",
      multiPageControlPermille: "100",
      pageCap: "2",
      leaderCap: "512",
      tailChainEnabled: "1",
      regionTlbCacheEnabled: "1",
      regionTlbCacheMinAccesses: "4",
    },
    modules: { count: 1, bytes: 1, kinds: { single: 1 } },
  };
}

function writeFixture(name, options = {}) {
  const directory = join(scratch, name);
  mkdirSync(directory);
  writeFileSync(join(directory, "protocol.json"), `${JSON.stringify(protocol(), null, 2)}\n`);
  let sequence = 0;
  for (const { pair, sides } of order) {
    for (const side of sides) {
      const started = Date.parse("2026-08-09T12:00:01.000Z") + sequence * 2_000;
      sequence++;
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

function analyze(directory, output) {
  return spawnSync(process.execPath, [analyzer, directory, `--output=${output}`], {
    cwd: root,
    encoding: "utf8",
  });
}

try {
  const passing = writeFixture("passing");
  const passingReport = join(scratch, "passing.json");
  const pass = analyze(passing, passingReport);
  assert.equal(pass.status, 0, pass.stderr);
  const report = JSON.parse(readFileSync(passingReport, "utf8"));
  assert.equal(report.measurementValid, true);
  assert.equal(report.gatePassed, true);
  const verify = spawnSync(process.execPath, [verifier, passingReport], {
    cwd: root,
    encoding: "utf8",
  });
  assert.equal(verify.status, 0, verify.stderr);

  const slow = writeFixture("slow", { slowExecution: true });
  const slowReport = join(scratch, "slow.json");
  const slowResult = analyze(slow, slowReport);
  assert.equal(slowResult.status, 1);
  assert.match(slowResult.stderr, /execution speedup/);
  assert.equal(JSON.parse(readFileSync(slowReport, "utf8")).measurementValid, true);

  const lowMips = writeFixture("low-mips", { lowCandidateInstructions: true });
  const lowMipsReport = join(scratch, "low-mips.json");
  const lowMipsResult = analyze(lowMips, lowMipsReport);
  assert.equal(lowMipsResult.status, 1);
  assert.match(lowMipsResult.stderr, /MIPS ratio/);

  const invalid = writeFixture("invalid", { missingSample: true });
  const invalidResult = analyze(invalid, join(scratch, "invalid.json"));
  assert.equal(invalidResult.status, 1);
  assert.match(invalidResult.stderr, /pair-1-candidate\.json staticFast/);

  console.log(
    "R076 analyzer selftest: pass, 5% execution gate, MIPS gate, and lifecycle invalidation enforced",
  );
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
