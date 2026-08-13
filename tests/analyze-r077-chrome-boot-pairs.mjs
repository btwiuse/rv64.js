#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const resultsArgument = process.argv.slice(2).find((argument) => !argument.startsWith("--"));
const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
if (!resultsArgument) {
  throw new Error(
    "usage: node tests/analyze-r077-chrome-boot-pairs.mjs RESULTS_DIR --output=REPORT",
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
const expectedGuest = { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" };
const expectedTimer = {
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
const expectedThresholds = {
  minimumExecutionSpeedup: 1.05,
  minimumExecutionBootstrapLower: 1.00,
  minimumMipsRatio: 1.00,
  minimumMipsBootstrapLower: 0.97,
};
const expectedAssets = {
  page: {
    route: "/jit-modern-boot-production-browser.html",
    bytes: 1063,
    sha256: "23b74fad059e2a9bbf0bce452bb1a5298af444d886f5fb92738318644133a0c5",
  },
  worker: {
    route: "/jit-modern-boot-production-browser-worker.mjs",
    bytes: 7861,
    sha256: "a9ea4021697a29ea771d3e9d57c3439e0e9289f9a8d25df8f6abfbf83c40d45b",
  },
  timingLibrary: {
    route: "/r077-browser-boot-lib.mjs",
    bytes: 1132,
    sha256: "05edc84de1e4b8df83b6b3ea0ba8474230f0f8cb61d2c0a0807392c3a42d4d57",
  },
  loader: {
    route: "/web/rv64.js",
    bytes: 92379,
    sha256: "d949d8641dd4048ed031c7293ddf9d7b7c911dbc89aa9fa0c29487c21687718b",
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
  host: "c533f528d69d7f43075e84ced676fdc8491ee29c90de16ebe2872a9657f60843",
  selftest: "a89b42c94fcb0eeb4408eeb97277009935a7ed4e3f6d82a3f306010ba3b1f18e",
  runner: "952e122a1fcee3865bf144cb7730373ccf5927427bf1b3bbe47e0e61f750ebdc",
};
const expectedOrder = Array.from({ length: 7 }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "production"] : ["production", "control"],
}));

assert.equal(protocol.schema, 1, "schema");
assert.equal(
  protocol.experiment,
  "R077 production-default Chrome execution-only modern Boot",
  "experiment",
);
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

const pairPattern = /^pair-(\d+)-(control|production)\.json$/;
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
  const production = side === "production";
  for (const field of ["staticFast", "sampled", "samples", "polls", "marks", "bypasses"]) {
    if (production) assert.ok(BigInt(result.counters[field]) > 0n, `${label} ${field}`);
    else assert.equal(BigInt(result.counters[field]), 0n, `${label} control ${field}`);
  }
  assert.equal(result.staticLifecycle.productionHelper, true, `${label} production helper`);
  assert.equal(result.staticLifecycle.registeredModules, 1, `${label} registration count`);
  assert.equal(result.staticLifecycle.modulesAfter, result.staticLifecycle.modulesBefore + 1,
    `${label} one auxiliary module`);
  assert.ok(result.staticLifecycle.moduleIndex >= 0, `${label} auxiliary index`);
  assert.equal(result.staticLifecycle.systemEnabled, false, `${label} residual static`);
  assert.equal(result.staticLifecycle.sampledEnabled, production, `${label} sampled static`);
  assert.equal(result.staticLifecycle.sampledBackoffEnabled, production, `${label} backoff`);
  assert.equal(result.staticLifecycle.controlOverride, !production, `${label} control override`);
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
  production: load(pair, "production"),
}));
assert.equal(new Set(pairs.flatMap(({ control, production }) => [
  control.result.staticLifecycle.moduleIndex,
  production.result.staticLifecycle.moduleIndex,
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
function analysisFor(control, production, ratio) {
  const pairedRatios = control.map((value, index) => ratio(value, production[index]));
  return {
    control,
    production,
    pairedRatios,
    controlMedian: median(control),
    productionMedian: median(production),
    pairedMedian: median(pairedRatios),
    pairedMedianBootstrap95: exactBootstrap95(pairedRatios),
  };
}

const execution = analysisFor(
  pairs.map((pair) => pair.control.result.timing.ms),
  pairs.map((pair) => pair.production.result.timing.ms),
  (control, production) => control / production,
);
const mips = analysisFor(
  pairs.map((pair) => pair.control.result.mips),
  pairs.map((pair) => pair.production.result.mips),
  (control, production) => production / control,
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
    statistic: "paired median control/production execution-time speedup",
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
  counters: Object.fromEntries(["control", "production"].map((side) => [
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
if (problems.length) throw new Error(`R077_CHROME_BOOT_GATE_FAIL: ${problems.join("; ")}`);
console.error("R077_CHROME_BOOT_GATE_PASS");
