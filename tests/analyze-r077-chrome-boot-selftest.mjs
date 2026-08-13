#!/usr/bin/env node

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const analyzer = join(root, "tests/analyze-r077-chrome-boot-pairs.mjs");
const verifier = join(root, "tests/vs-v86/r077-chrome-boot-gate.mjs");
const scratch = mkdtempSync(join(tmpdir(), "rv64-r077-analyzer-selftest-"));
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
  control: {
    productionHelper: true,
    controlOverride: true,
    staticSystemT0: false,
    sampledStaticT0: false,
    sampledStaticT0Backoff: false,
  },
  production: {
    productionHelper: true,
    controlOverride: false,
    staticSystemT0: false,
    sampledStaticT0: true,
    sampledStaticT0Backoff: true,
  },
};
const timerContract = {
  setupBeforeTimer: [
    "asset-fetch-and-sha256",
    "RV64.create",
    "bootVirtLinuxDirect",
    "production-helper-and-control-override",
    "initial-counters",
  ],
  timed: "runTimedBoot:first-2M-pump-through-SCORECARD_V2_READY",
  quantum: "2000000",
  cadence: "yield-after-pump-1-then-every-fourth",
  marker: "SCORECARD_V2_READY",
};
const hashes = {
  page: [1063, "23b74fad059e2a9bbf0bce452bb1a5298af444d886f5fb92738318644133a0c5"],
  worker: [7861, "a9ea4021697a29ea771d3e9d57c3439e0e9289f9a8d25df8f6abfbf83c40d45b"],
  timingLibrary: [1132, "05edc84de1e4b8df83b6b3ea0ba8474230f0f8cb61d2c0a0807392c3a42d4d57"],
  loader: [92379, "d949d8641dd4048ed031c7293ddf9d7b7c911dbc89aa9fa0c29487c21687718b"],
  wasm: [4329839, "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c"],
  kernel: [4205056, "57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2"],
  initramfs: [64383488, "cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808"],
};
const routes = {
  page: "/jit-modern-boot-production-browser.html",
  worker: "/jit-modern-boot-production-browser-worker.mjs",
  timingLibrary: "/r077-browser-boot-lib.mjs",
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
  host: "c533f528d69d7f43075e84ced676fdc8491ee29c90de16ebe2872a9657f60843",
  selftest: "a89b42c94fcb0eeb4408eeb97277009935a7ed4e3f6d82a3f306010ba3b1f18e",
  runner: "952e122a1fcee3865bf144cb7730373ccf5927427bf1b3bbe47e0e61f750ebdc",
};
const order = Array.from({ length: 7 }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "production"] : ["production", "control"],
}));

function protocol() {
  return {
    schema: 1,
    experiment: "R077 production-default Chrome execution-only modern Boot",
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
  const production = side === "production";
  const ms = production ? (options.slowExecution ? 970 : 900) : 1_000;
  const instructions = BigInt(
    production && options.lowCandidateInstructions ? 140_000_000 : 180_000_000,
  );
  const generated = 90_000_000n;
  const active = production && !options.missingSample ? "1" : "0";
  return {
    schema: 1,
    experiment: "R077 production-default Chrome execution-only modern Boot",
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
      productionHelper: true,
      registeredModules: 1,
      moduleIndex: 0,
      modulesBefore: 0,
      modulesAfter: 1,
      systemEnabled: false,
      sampledEnabled: production,
      sampledBackoffEnabled: production,
      controlOverride: !production,
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
  assert.match(invalidResult.stderr, /pair-1-production\.json staticFast/);

  console.log(
    "R077 analyzer selftest: pass, 5% execution gate, MIPS gate, and lifecycle invalidation enforced",
  );
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
