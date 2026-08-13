#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const resultsArgument = process.argv.slice(2).find((argument) => !argument.startsWith("--"));
const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
if (!resultsArgument) {
  throw new Error(
    "usage: node tests/analyze-r076-chrome-boot-pairs.mjs RESULTS_DIR --output=REPORT",
  );
}
const directory = resolve(resultsArgument);
const protocol = JSON.parse(readFileSync(join(directory, "protocol.json"), "utf8"));
const expectedBrowser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const expectedConfigurations = {
  control: { staticSystemT0: false, sampledStaticT0: false, sampledStaticT0Backoff: false },
  candidate: { staticSystemT0: false, sampledStaticT0: true, sampledStaticT0Backoff: true },
};
const expectedGuest = { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" };
const expectedTimer = {
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
const expectedThresholds = {
  minimumExecutionSpeedup: 1.05,
  minimumExecutionBootstrapLower: 1.00,
  minimumMipsRatio: 1.00,
  minimumMipsBootstrapLower: 0.97,
};
const expectedAssets = {
  page: {
    route: "/jit-modern-boot-browser.html",
    bytes: 1032,
    sha256: "bae81c20a73019dd57f5701330862f4a7b1c9d03feebbd1a7a718632e9c5d29b",
  },
  worker: {
    route: "/jit-modern-boot-browser-worker.mjs",
    bytes: 7367,
    sha256: "30c381a18b97ee9c24d54ee3a001c0232e4ff4d438036a9b782512417e60a883",
  },
  timingLibrary: {
    route: "/r076-browser-boot-lib.mjs",
    bytes: 1131,
    sha256: "8d39c83ad01e1d1003bf56ded2439d3475f925f7925f0731da64b90170f9f548",
  },
  loader: {
    route: "/web/rv64.js",
    bytes: 90264,
    sha256: "2582e18ea207ad0f5ec154e82d3fe6208faf5742f26a9d8463d8d360f168b776",
  },
  wasm: {
    route: "/rv64.wasm",
    bytes: 4329839,
    sha256: "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c",
  },
  kernel: {
    route: "/kernel",
    bytes: 4205056,
    sha256: "57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2",
  },
  initramfs: {
    route: "/initramfs",
    bytes: 64383488,
    sha256: "cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808",
  },
};
const expectedTools = {
  host: "fb05697dff42a57636c8965842f721964f444faa0b5db520c2a587f4c8c9d857",
  selftest: "516c3d1b4371fb56de8a5823c37942c11ce60cb897245827c5a95d52a4cb8995",
  runner: "d9b66a3b48a31a1154b827cb52c738319c99c5604bc98787d6c588451d024a70",
};
const expectedOrder = Array.from({ length: 7 }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "candidate"] : ["candidate", "control"],
}));

assert.equal(protocol.schema, 1, "schema");
assert.equal(protocol.experiment, "R076 Chrome execution-only modern Boot", "experiment");
assert.equal(protocol.pairs, 7, "pair count");
assert.deepEqual(protocol.order, expectedOrder, "order");
assert.equal(protocol.hostCpuAffinity, "8-15", "CPU affinity");
assert.deepEqual(protocol.expectedBrowser, expectedBrowser, "browser identity");
assert.deepEqual(protocol.configurations, expectedConfigurations, "configurations");
assert.deepEqual(protocol.timerContract, expectedTimer, "timer contract");
assert.deepEqual(protocol.guest, expectedGuest, "guest contract");
assert.deepEqual(protocol.thresholds, expectedThresholds, "thresholds");
assert.deepEqual(protocol.toolManifest, expectedTools, "tool identities");
for (const [name, expected] of Object.entries(expectedAssets)) {
  const actual = protocol.assetManifest?.[name];
  assert.ok(actual, `${name} asset missing`);
  assert.equal(actual.route, expected.route, `${name} route`);
  assert.equal(actual.bytes, expected.bytes, `${name} bytes`);
  assert.equal(actual.sha256, expected.sha256, `${name} SHA-256`);
}
const plannedAt = Date.parse(protocol.plannedAt);
assert.ok(Number.isFinite(plannedAt), "planned timestamp");

const pairPattern = /^pair-(\d+)-(control|candidate)\.json$/;
const pairFiles = readdirSync(directory).filter((name) => pairPattern.test(name));
assert.equal(pairFiles.length, 14, "exactly fourteen result files required");

function load(pair, side) {
  const path = join(directory, `pair-${pair}-${side}.json`);
  const wrapper = JSON.parse(readFileSync(path, "utf8"));
  const label = basename(path);
  const startedAt = Date.parse(wrapper.startedAt);
  const finishedAt = Date.parse(wrapper.finishedAt);
  assert.ok(Number.isFinite(startedAt) && startedAt >= plannedAt, `${label} start`);
  assert.ok(Number.isFinite(finishedAt) && finishedAt >= startedAt, `${label} finish`);
  assert.equal(wrapper.hostCpuAffinity, "8-15", `${label} affinity`);
  assert.deepEqual(wrapper.browser, expectedBrowser, `${label} browser`);
  const result = wrapper.result;
  assert.equal(result.schema, 1, `${label} result schema`);
  assert.equal(result.experiment, protocol.experiment, `${label} experiment`);
  assert.equal(result.variant, side, `${label} side`);
  assert.equal(result.userAgent, expectedBrowser.userAgent, `${label} user agent`);
  assert.deepEqual(result.guest, expectedGuest, `${label} guest`);
  assert.equal(result.outputMarkers?.ready, true, `${label} ready marker`);
  assert.equal(
    result.outputMarkers?.guest,
    "SCORECARD_V2_GUEST linux=6.12.7 alpine=3.24.1 arch=riscv64",
    `${label} guest marker`,
  );
  assert.match(result.outputSha256 ?? "", /^[0-9a-f]{64}$/, `${label} output SHA-256`);
  assert.deepEqual(result.timerBoundary, {
    setupBeforeTimer: expectedTimer.setupBeforeTimer,
    timed: expectedTimer.timed,
  }, `${label} timer boundary`);
  assert.deepEqual(result.assetHashes, {
    wasm: expectedAssets.wasm.sha256,
    kernel: expectedAssets.kernel.sha256,
    initramfs: expectedAssets.initramfs.sha256,
  }, `${label} browser asset identities`);
  assert.ok(Number.isFinite(result.timing?.ms) && result.timing.ms > 0, `${label} timing`);
  assert.equal(result.timing.quantum, expectedTimer.quantum, `${label} quantum`);
  assert.equal(result.timing.cadence, expectedTimer.cadence, `${label} cadence`);
  assert.equal(result.timing.marker, expectedTimer.marker, `${label} marker`);
  assert.ok(Number.isInteger(result.timing.pumps) && result.timing.pumps > 0, `${label} pumps`);
  assert.equal(result.timing.yields, Math.ceil(result.timing.pumps / 4), `${label} yields`);
  const instructions = BigInt(result.instructions);
  const generated = BigInt(result.counters.generated);
  const interpreted = BigInt(result.counters.interpreted);
  assert.ok(instructions > 100_000_000n, `${label} instruction count`);
  assert.equal(generated + interpreted, instructions, `${label} instruction accounting`);
  assert.ok(generated > 0n, `${label} generated retirement`);
  assert.ok(BigInt(result.counters.dispatches) > 0n, `${label} generated dispatches`);
  assert.equal(BigInt(result.counters.errors), 0n, `${label} static errors`);
  const expectedMips = Number(instructions) / result.timing.ms / 1_000;
  assert.ok(Math.abs(result.mips / expectedMips - 1) < 1e-12, `${label} MIPS accounting`);
  const candidate = side === "candidate";
  for (const field of ["staticFast", "sampled", "samples", "polls", "marks", "bypasses"]) {
    if (candidate) assert.ok(BigInt(result.counters[field]) > 0n, `${label} ${field}`);
    else assert.equal(BigInt(result.counters[field]), 0n, `${label} control ${field}`);
  }
  assert.equal(result.staticLifecycle.modulesAfter, result.staticLifecycle.modulesBefore + 1,
    `${label} one auxiliary module`);
  assert.ok(result.staticLifecycle.moduleIndex >= 0, `${label} auxiliary index`);
  assert.equal(result.staticLifecycle.systemEnabled, false, `${label} residual static`);
  assert.equal(result.staticLifecycle.sampledEnabled, candidate, `${label} sampled static`);
  assert.equal(result.staticLifecycle.sampledBackoffEnabled, candidate, `${label} backoff`);
  assert.deepEqual(result.policy, {
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
  }, `${label} production policy`);
  assert.ok(result.modules?.count > 0 && result.modules?.bytes > 0, `${label} modules`);
  return { wrapper, result, startedAt, finishedAt };
}

const pairs = expectedOrder.map(({ pair }) => ({
  pair,
  control: load(pair, "control"),
  candidate: load(pair, "candidate"),
}));
assert.equal(new Set(pairs.flatMap(({ control, candidate }) => [
  control.result.staticLifecycle.moduleIndex,
  candidate.result.staticLifecycle.moduleIndex,
])).size, 1, "auxiliary module index changed");

let previousFinished = plannedAt;
const observedOrder = [];
for (const { pair, sides } of expectedOrder) {
  for (const side of sides) {
    const run = pairs[pair - 1][side];
    assert.ok(run.startedAt >= previousFinished, `pair-${pair}-${side} chronology`);
    previousFinished = run.finishedAt;
    observedOrder.push({
      pair,
      side,
      startedAt: run.wrapper.startedAt,
      finishedAt: run.wrapper.finishedAt,
    });
  }
}

function median(values) {
  const ordered = [...values].sort((left, right) => left - right);
  return ordered[Math.floor(ordered.length / 2)];
}
function quantile(values, probability) {
  const ordered = [...values].sort((left, right) => left - right);
  return ordered[Math.floor((ordered.length - 1) * probability)];
}
function exactBootstrap95(values) {
  const sample = Array(values.length);
  const distribution = [];
  function visit(depth) {
    if (depth === sample.length) {
      distribution.push(median(sample));
      return;
    }
    for (const value of values) {
      sample[depth] = value;
      visit(depth + 1);
    }
  }
  visit(0);
  return [quantile(distribution, 0.025), quantile(distribution, 0.975)];
}
function analysisFor(control, candidate, ratio) {
  const pairedRatios = control.map((value, index) => ratio(value, candidate[index]));
  return {
    control,
    candidate,
    pairedRatios,
    controlMedian: median(control),
    candidateMedian: median(candidate),
    pairedMedian: median(pairedRatios),
    pairedMedianBootstrap95: exactBootstrap95(pairedRatios),
  };
}

const execution = analysisFor(
  pairs.map((pair) => pair.control.result.timing.ms),
  pairs.map((pair) => pair.candidate.result.timing.ms),
  (control, candidate) => control / candidate,
);
const mips = analysisFor(
  pairs.map((pair) => pair.control.result.mips),
  pairs.map((pair) => pair.candidate.result.mips),
  (control, candidate) => candidate / control,
);
const problems = [];
if (execution.pairedMedian < expectedThresholds.minimumExecutionSpeedup) {
  problems.push(
    `execution speedup ${execution.pairedMedian.toFixed(4)} below ` +
      expectedThresholds.minimumExecutionSpeedup.toFixed(2),
  );
}
if (execution.pairedMedianBootstrap95[0] < expectedThresholds.minimumExecutionBootstrapLower) {
  problems.push(
    `execution lower ${execution.pairedMedianBootstrap95[0].toFixed(4)} below ` +
      expectedThresholds.minimumExecutionBootstrapLower.toFixed(2),
  );
}
if (mips.pairedMedian < expectedThresholds.minimumMipsRatio) {
  problems.push(
    `MIPS ratio ${mips.pairedMedian.toFixed(4)} below ` +
      expectedThresholds.minimumMipsRatio.toFixed(2),
  );
}
if (mips.pairedMedianBootstrap95[0] < expectedThresholds.minimumMipsBootstrapLower) {
  problems.push(
    `MIPS lower ${mips.pairedMedianBootstrap95[0].toFixed(4)} below ` +
      expectedThresholds.minimumMipsBootstrapLower.toFixed(2),
  );
}

const report = {
  schema: 1,
  experiment: protocol.experiment,
  measurementValid: true,
  gatePassed: problems.length === 0,
  protocol: { plannedAt: protocol.plannedAt, order: observedOrder },
  method: {
    pairs: 7,
    pairing: "alternating fresh Chrome process/profile/module-Worker/modern guest",
    statistic: "paired median control/candidate execution-time speedup",
    interval: "exact paired bootstrap percentile 95%",
    thresholds: expectedThresholds,
    timerContract: expectedTimer,
  },
  browser: expectedBrowser,
  hostCpuAffinity: "8-15",
  configurations: expectedConfigurations,
  guest: expectedGuest,
  assetManifest: protocol.assetManifest,
  toolManifest: expectedTools,
  staticModuleIndex: pairs[0].control.result.staticLifecycle.moduleIndex,
  execution,
  mips,
  counters: Object.fromEntries(["control", "candidate"].map((side) => [
    side,
    Object.fromEntries([
      "instructions", "generated", "interpreted", "staticFast", "sampled", "samples",
      "polls", "marks", "bypasses", "errors",
    ].map((field) => [field, median(pairs.map((pair) => Number(
      field === "instructions" ? pair[side].result.instructions : pair[side].result.counters[field],
    )))])),
  ])),
  problems,
};
const serialized = `${JSON.stringify(report, null, 2)}\n`;
if (outputArgument) {
  writeFileSync(resolve(outputArgument.split("=", 2)[1]), serialized, { flag: "wx" });
}
process.stdout.write(serialized);
if (problems.length) throw new Error(`R076_CHROME_BOOT_GATE_FAIL: ${problems.join("; ")}`);
console.error("R076_CHROME_BOOT_GATE_PASS");
