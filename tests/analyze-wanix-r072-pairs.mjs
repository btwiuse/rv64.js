#!/usr/bin/env node

// Validate and analyze the candidate-specific sample produced by
// run-wanix-r072-pairs.mjs. The constants are deliberately frozen here too:
// changing one creates a new protocol and cannot alter an existing verdict.

import assert from "node:assert/strict";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const resultsArgument = process.argv.slice(2).find((argument) => !argument.startsWith("--"));
const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
if (!resultsArgument) {
  throw new Error("usage: node tests/analyze-wanix-r072-pairs.mjs RESULTS_DIR --output=REPORT");
}
const resultsDirectory = resolve(resultsArgument);
const protocol = JSON.parse(readFileSync(join(resultsDirectory, "protocol.json"), "utf8"));
const phases = ["python", "sha256", "shared9p"];
assert.ok(
  protocol.schema === 1 || protocol.schema === 2 || protocol.schema === 3 ||
    protocol.schema === 4 || protocol.schema === 5,
  "unsupported R072/R073/R074/R075 browser protocol schema",
);
const sampledOnlyR073 = protocol.schema === 3;
const shortBackoffR074 = protocol.schema === 4;
const prebootR075 = protocol.schema === 5;
const strengthenedBrowserProtocol = sampledOnlyR073 || shortBackoffR074 || prebootR075;
const establishedValidityConfirmation = protocol.schema === 2;
const expectedWasm = shortBackoffR074 || prebootR075
  ? "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c"
  : "cb7ea81685b3cb9605f6e01b619f3c15005f4ef77ca00c258c20c7a519bb6df6";
const expectedPairs = strengthenedBrowserProtocol ? 7 : 5;
const expectedRepetitions = strengthenedBrowserProtocol ? 3 : 1;
const configurations = shortBackoffR074 || prebootR075 ? {
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
} : {
  control: { staticSystemT0: false, sampledStaticT0: false },
  candidate: {
    staticSystemT0: sampledOnlyR073 ? false : true,
    sampledStaticT0: true,
  },
};
const expectedPage = prebootR075
  ? {
    control: "a2f43c06f86507c267c36fb3922079d11d44072ab0622526e5577f69448e976f",
    candidate: "7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413",
  }
  : shortBackoffR074
  ? "bdc4827f2a9b86eee1ce4443a9914eae4ef8e5c4ff8329b81973343feccb1a64"
  : sampledOnlyR073
    ? "1c70b211272fd9a843bfe52aefe804322d7260a144df7195a34363ad9f259aee"
    : "28957e0d5ce381184addb291805ba26a6e64d421a51882c4ae56e0512a82cd3d";
const expectedArchive = prebootR075
  ? "e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c"
  : shortBackoffR074
  ? "917ddcad15a15fa6560c480b9b19ccc2d39ec52ceed65030c94c79f0805df2a9"
  : sampledOnlyR073
    ? "2b52e552d00929fa4c525c5b1aabc7abbce74d7d3ffe571a0e28d7d3b1cf199e"
    : "159fc55c4337345a685252e384d64be39fc50c743b4478e2b864289ad8bb8690";
const expectedRv64Root =
  "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb";
const expectedX86Root =
  "09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320";
const expectedV86Archive =
  "7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771";
const expectedBrowser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const expectedHarness = prebootR075
  ? "525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545"
  : "c3f32eee15012ecc53da541bb3e3b1bda798ae1983d2b3a1e1bcd90dcb4e7495";
const expectedCandidateProof = [
  "staticT0FastRetired",
  "sampledStaticT0Retired",
  "sampledStaticT0Samples",
  "sampledStaticT0InterruptPolls",
  ...(shortBackoffR074 || prebootR075 ? [
    "sampledStaticT0ShortMarks",
    "sampledStaticT0ShortBypasses",
  ] : []),
];
assert.equal(
  protocol.experiment,
  prebootR075
    ? "R075 preboot sampled-backoff browser candidate/control"
    : shortBackoffR074
    ? "R074 short-sample backoff browser candidate/control"
    : sampledOnlyR073
      ? "R073 sampled-only browser candidate/control"
    : establishedValidityConfirmation
      ? "R072 independent browser confirmation"
      : "R072 browser candidate/control",
  "experiment identity",
);
assert.equal(protocol.pairs, expectedPairs, "browser pair count changed");
assert.equal(protocol.maxSlowdown, 0.03, "browser margin changed");
assert.equal(protocol.vm, "rv64-jit", "browser VM changed");
assert.deepEqual(protocol.phases, phases, "phase set changed");
assert.equal(protocol.repetitions, expectedRepetitions, "repetition count changed");
assert.equal(protocol.phaseSync, true, "phase synchronization changed");
assert.deepEqual(protocol.configurations, configurations, "browser configurations changed");
if (prebootR075) {
  assert.deepEqual(protocol.urls, {
    control:
      "http://127.0.0.1:8765/examples/" +
      "v86-rv64-three-way-r075-control-e0c1971d1ecd4d4f.html",
    candidate:
      "http://127.0.0.1:8765/examples/" +
      "v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html",
  }, "preboot page URLs changed");
  assert.equal(protocol.jitConfigurationLifecycle, "preboot", "JIT lifecycle changed");
  assert.equal(protocol.minimumShellSpeedup, 1.10, "shell speedup gate changed");
  assert.equal(
    protocol.minimumShellMedianBootstrapLower,
    1.00,
    "shell confidence-lower gate changed",
  );
}
if (shortBackoffR074 || prebootR075) {
  assert.deepEqual(protocol.expectedBrowser, expectedBrowser, "browser identity changed");
  assert.equal(protocol.expectedHarnessSha256, expectedHarness, "WANIX harness changed");
}
assert.equal(protocol.expectedMainWasmSha256, expectedWasm, "main Wasm identity changed");
assert.deepEqual(protocol.expectedPageSha256, expectedPage, "page identity changed");
assert.equal(
  protocol.expectedRv64ArchiveSha256,
  expectedArchive,
  "archive identity changed",
);
assert.equal(
  protocol.maximumWithinSideSpread,
  establishedValidityConfirmation || strengthenedBrowserProtocol ? null : 1.25,
  "sample-spread protocol changed",
);
assert.equal(
  protocol.maximumPairedMedianBootstrapUpper ?? null,
  strengthenedBrowserProtocol ? 1.10 : null,
  "paired-confidence protocol changed",
);
if (establishedValidityConfirmation) {
  assert.deepEqual(protocol.supersedesInvalidReport, {
    path: "target/jit-policy-traces/wanix-r072-cb7ea816-chrome-20260809-config-ab/analysis.json",
    sha256: "91da7722e3289f586c89c5fd01c623c0e82c95c1d15b9a13b989e86ef5b08776",
    reason: "new 1.25 within-side cap was incompatible with established shared-9P variance",
  }, "invalid predecessor identity");
}
assert.deepEqual(
  protocol.minimumGeneratedCoverage,
  { python: 0.90, sha256: 0.90, shared9p: 0 },
  "generated-coverage gate changed",
);
assert.deepEqual(
  protocol.requiredCandidateProof,
  expectedCandidateProof,
  "candidate lifecycle proof set changed",
);
assert.match(protocol.cpuAffinity ?? "", /^\d+(?:[-,]\d+)*$/, "CPU affinity");
const plannedAt = Date.parse(protocol.plannedAt);
assert.ok(Number.isFinite(plannedAt), "plannedAt timestamp");
const expectedOrder = Array.from({ length: expectedPairs }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "candidate"] : ["candidate", "control"],
}));
assert.deepEqual(protocol.order, expectedOrder, "pair order changed");

const pairPattern = /^pair-(\d+)-(control|candidate)\.log$/;
const pairNumbers = [...new Set(readdirSync(resultsDirectory).flatMap((name) => {
  const match = pairPattern.exec(name);
  return match ? [Number(match[1])] : [];
}))].sort((left, right) => left - right);
assert.deepEqual(
  pairNumbers,
  Array.from({ length: expectedPairs }, (_, index) => index + 1),
  "result pair set differs from protocol",
);

function median(values) {
  const ordered = [...values].sort((left, right) => left - right);
  return ordered[Math.floor(ordered.length / 2)];
}
function quantile(values, probability) {
  const ordered = [...values].sort((left, right) => left - right);
  return ordered[Math.floor((ordered.length - 1) * probability)];
}
function exactBootstrap95(values, statistic) {
  const sample = Array(values.length);
  const distribution = [];
  function visit(depth) {
    if (depth === sample.length) {
      distribution.push(statistic(sample));
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
function spread(values) {
  return Math.max(...values) / Math.min(...values);
}
function validateDigest(record, label, hasSource = false) {
  assert.equal(typeof record?.url, "string", `${label} URL`);
  assert.ok(record.url.length > 0, `${label} URL`);
  assert.ok(Number.isInteger(record?.bytes) && record.bytes > 0, `${label} byte count`);
  assert.match(record?.sha256 ?? "", /^[0-9a-f]{64}$/, `${label} SHA-256`);
  if (hasSource) {
    assert.equal(typeof record?.source, "string", `${label} source`);
    assert.ok(record.source.length > 0, `${label} source`);
  }
}

function load(pair, side) {
  const path = join(resultsDirectory, `pair-${pair}-${side}.log`);
  const text = readFileSync(path, "utf8");
  const lines = text.split("\n");
  const summary = JSON.parse(lines[0]);
  assert.equal(summary.vm, "rv64-jit", `${basename(path)} VM identity`);
  assert.equal(summary.repetitions, expectedRepetitions, `${basename(path)} repetitions`);
  assert.equal(summary.phaseSync, true, `${basename(path)} phase synchronization`);
  assert.equal(summary.jitPolicy, null, `${basename(path)} policy override`);
  assert.deepEqual(
    summary.jitConfiguration,
    configurations[side],
    `${basename(path)} candidate configuration`,
  );
  assert.equal(summary.hostCpuAffinity, protocol.cpuAffinity, `${basename(path)} CPU affinity`);
  if (shortBackoffR074 || prebootR075) {
    assert.deepEqual(summary.browser, expectedBrowser, `${basename(path)} browser identity`);
  }
  assert.equal(summary.artifactStable, true, `${basename(path)} artifact stability`);
  validateDigest(summary.artifacts?.page, `${basename(path)} page`);
  assert.equal(
    summary.artifacts.page.url,
    new URL(prebootR075 ? protocol.urls[side] : protocol.url).href,
    `${basename(path)} page URL`,
  );
  assert.equal(
    summary.artifacts.page.sha256,
    prebootR075 ? expectedPage[side] : expectedPage,
    `${basename(path)} page identity`,
  );
  assert.deepEqual(
    Object.keys(summary.artifacts?.archives ?? {}).sort(),
    ["rv64Jit", "rv64Root", "v86", "x86Root"],
    `${basename(path)} artifact set`,
  );
  for (const [name, record] of Object.entries(summary.artifacts.archives)) {
    validateDigest(record, `${basename(path)} ${name}`, true);
  }
  assert.equal(
    summary.artifacts.archives.rv64Jit.sha256,
    expectedArchive,
    `${basename(path)} RV64 archive identity`,
  );
  if (strengthenedBrowserProtocol) {
    // Three synchronized repetitions push the boot-time Alpine banner out of
    // the deliberately short terminal tail. The smoke harness has already
    // waited for the exact 3.22.5 response before it can emit this summary;
    // independently bind that response to the frozen modern-Alpine root bytes
    // instead of requiring a line the logger intentionally did not retain.
    assert.equal(
      summary.artifacts.archives.rv64Root.sha256,
      expectedRv64Root,
      `${basename(path)} modern Alpine root identity`,
    );
    if (shortBackoffR074 || prebootR075) {
      assert.equal(
        summary.artifacts.archives.x86Root.sha256,
        expectedX86Root,
        `${basename(path)} matched x86 root identity`,
      );
      assert.equal(
        summary.artifacts.archives.v86.sha256,
        expectedV86Archive,
        `${basename(path)} frozen v86 archive identity`,
      );
      assert.deepEqual(
        summary.guest,
        { machine: "riscv64", alpineRelease: "3.22.5", pythonVersion: "3.12.13" },
        `${basename(path)} guest identity`,
      );
    }
  }
  const startedAt = Date.parse(summary.startedAt);
  const finishedAt = Date.parse(summary.finishedAt);
  assert.ok(Number.isFinite(startedAt) && startedAt >= plannedAt, `${basename(path)} start`);
  assert.ok(Number.isFinite(finishedAt) && finishedAt >= startedAt, `${basename(path)} finish`);
  assert.deepEqual(summary.phases, phases, `${basename(path)} phases`);
  assert.match(text, /checksum=38460b78/, `${basename(path)} Python checksum`);
  assert.match(text, /e09320c5b00b34bb/, `${basename(path)} SHA-256 digest`);
  if (strengthenedBrowserProtocol) {
    for (const phase of phases) {
      assert.deepEqual(
        summary.correctness?.[phase],
        { complete: expectedRepetitions, correct: expectedRepetitions },
        `${basename(path)} ${phase} correctness count`,
      );
    }
  }
  if (!strengthenedBrowserProtocol) {
    assert.match(text, /3\.22\.5/, `${basename(path)} Alpine release`);
  }
  assert.match(text, /riscv64 Python 3\.12\.13/, `${basename(path)} guest identity`);
  const jitRecord = JSON.parse(lines[1]);
  const jit = jitRecord.jitPhases;
  const jitAtShell = jitRecord.jitAtShell;
  if (prebootR075) {
    assert.equal(
      summary.jitConfigurationLifecycle,
      "preboot",
      `${basename(path)} JIT configuration lifecycle`,
    );
    assert.ok(jitAtShell, `${basename(path)} shell JIT proof`);
    const shellStatic = jitAtShell.staticT0;
    assert.ok(shellStatic, `${basename(path)} shell static-T0 proof`);
    const candidate = side === "candidate";
    assert.equal(shellStatic.systemEnabled, false, `${basename(path)} shell residual-static enable`);
    assert.equal(shellStatic.sampledEnabled, candidate, `${basename(path)} shell sampled enable`);
    assert.equal(
      shellStatic.sampledBackoffEnabled,
      candidate,
      `${basename(path)} shell sampled-backoff enable`,
    );
    assert.equal(BigInt(shellStatic.systemErrors), 0n, `${basename(path)} shell static errors`);
    assert.ok(shellStatic.moduleIndex >= 0, `${basename(path)} shell auxiliary module`);
    for (const field of [
      "sampledRetired", "samples", "interruptPolls", "shortSampleMarks",
      "shortSampleBypasses",
    ]) {
      if (candidate) {
        assert.ok(BigInt(shellStatic[field]) > 0n, `${basename(path)} shell ${field}`);
      } else {
        assert.equal(BigInt(shellStatic[field]), 0n, `${basename(path)} shell control ${field}`);
      }
    }
  }
  for (const phase of phases) {
    assert.equal(
      summary.samples[phase].length,
      expectedRepetitions,
      `${basename(path)} ${phase} sample count`,
    );
    for (const value of summary.samples[phase]) {
      assert.ok(value > 0, `${basename(path)} ${phase} duration`);
    }
    for (let repetition = 1; repetition <= expectedRepetitions; repetition++) {
      const key = expectedRepetitions === 1 ? phase : `${phase}${repetition}`;
      const proof = jit[key];
      assert.ok(proof, `${basename(path)} ${key} JIT proof`);
      assert.equal(proof.pagePolicyEnabled, "1", `${basename(path)} page policy`);
      assert.equal(proof.pageThreshold, "131072", `${basename(path)} page threshold`);
      assert.equal(proof.privilegedPageThresholdMultiplier, "32", `${basename(path)} privileged threshold`);
      assert.equal(proof.pageQuantum, "1024", `${basename(path)} page quantum`);
      assert.equal(proof.controlEntriesEnabled, "1", `${basename(path)} control entries`);
      assert.equal(proof.privilegedControlEntriesEnabled, "0", `${basename(path)} privileged controls`);
      assert.equal(proof.stablePageChainEnabled, "1", `${basename(path)} stable chain`);
      assert.equal(proof.pageInflightLimit, "2", `${basename(path)} in-flight limit`);
      assert.equal(proof.regionPageCap, "2", `${basename(path)} page cap`);
      assert.equal(proof.regionLeaderCap, "512", `${basename(path)} leader cap`);
      assert.equal(proof.regionTailChainEnabled, "1", `${basename(path)} tail chain`);
      assert.equal(proof.regionTlbCacheEnabled, "1", `${basename(path)} region TLB cache`);
      assert.equal(proof.regionTlbCacheMinAccesses, "4", `${basename(path)} region TLB threshold`);
      assert.equal(proof.multiPageEntryCap, "512", `${basename(path)} entry cap`);
      assert.equal(proof.multiPageControlPermille, "100", `${basename(path)} control gate`);
      assert.ok(BigInt(proof.chainHops) > 0n, `${basename(path)} chain activity`);
      assert.ok(BigInt(proof.generatedRetired) > 0n, `${basename(path)} generated retirement`);
      assert.ok(
        proof.generatedCoverage >= protocol.minimumGeneratedCoverage[phase],
        `${basename(path)} generated coverage`,
      );
      assert.equal(BigInt(proof.staticT0Errors), 0n, `${basename(path)} static-T0 errors`);
      assert.ok(proof.staticSystemT0ModuleIndex >= 0, `${basename(path)} auxiliary module`);
      const candidate = side === "candidate";
      assert.equal(
        proof.staticSystemT0Enabled,
        candidate && !strengthenedBrowserProtocol,
        `${basename(path)} residual-static enable`,
      );
      assert.equal(proof.sampledStaticT0Enabled, candidate, `${basename(path)} sampled enable`);
      if (shortBackoffR074 || prebootR075) {
        assert.equal(
          proof.sampledStaticT0BackoffEnabled,
          candidate,
          `${basename(path)} short-sample backoff enable`,
        );
      }
      for (const field of protocol.requiredCandidateProof) {
        if (candidate) assert.ok(BigInt(proof[field]) > 0n, `${basename(path)} ${key} ${field}`);
        else assert.equal(BigInt(proof[field]), 0n, `${basename(path)} ${key} control ${field}`);
      }
    }
  }
  return { summary, jit, jitAtShell, startedAt, finishedAt };
}

const pairs = pairNumbers.map((pair) => ({
  pair,
  control: load(pair, "control"),
  candidate: load(pair, "candidate"),
}));
const browserProducts = new Set(pairs.flatMap(({ control, candidate }) => [
  control.summary.browser.product,
  candidate.summary.browser.product,
]));
const jsVersions = new Set(pairs.flatMap(({ control, candidate }) => [
  control.summary.browser.jsVersion,
  candidate.summary.browser.jsVersion,
]));
const artifactSnapshots = new Set(pairs.flatMap(({ control, candidate }) => [
  JSON.stringify(control.summary.artifacts),
  JSON.stringify(candidate.summary.artifacts),
]));
const artifactSnapshotsBySide = Object.fromEntries(["control", "candidate"].map((side) => [
  side,
  new Set(pairs.map((pair) => JSON.stringify(pair[side].summary.artifacts))),
]));
const archiveSnapshots = new Set(pairs.flatMap(({ control, candidate }) => [
  JSON.stringify(control.summary.artifacts.archives),
  JSON.stringify(candidate.summary.artifacts.archives),
]));
assert.equal(browserProducts.size, 1, "browser product changed between samples");
assert.equal(jsVersions.size, 1, "JavaScript engine changed between samples");
if (prebootR075) {
  assert.equal(artifactSnapshotsBySide.control.size, 1, "control artifact changed between samples");
  assert.equal(
    artifactSnapshotsBySide.candidate.size,
    1,
    "candidate artifact changed between samples",
  );
  assert.equal(archiveSnapshots.size, 1, "comparison archives changed between sides or samples");
  assert.equal(
    new Set(pairs.flatMap(({ control, candidate }) => [
      control.jitAtShell.staticT0.moduleIndex,
      candidate.jitAtShell.staticT0.moduleIndex,
    ])).size,
    1,
    "prepared static module index changed between sides or samples",
  );
} else {
  assert.equal(artifactSnapshots.size, 1, "comparison artifact changed between samples");
}

let previousFinishedAt = plannedAt;
const observedOrder = [];
for (const { pair, sides } of expectedOrder) {
  const pairResult = pairs[pair - 1];
  for (const side of sides) {
    const run = pairResult[side];
    assert.ok(run.startedAt >= previousFinishedAt, `pair-${pair}-${side} chronology`);
    previousFinishedAt = run.finishedAt;
    observedOrder.push({
      pair,
      side,
      startedAt: run.summary.startedAt,
      finishedAt: run.summary.finishedAt,
    });
  }
}

const analysis = {};
const problems = [];
let shell = null;
if (prebootR075) {
  const control = pairs.map((pair) => pair.control.summary.shellMs);
  const candidate = pairs.map((pair) => pair.candidate.summary.shellMs);
  for (const [side, values] of [["control", control], ["candidate", candidate]]) {
    assert.ok(
      values.every((value) => Number.isFinite(value) && value > 0),
      `${side} shell durations`,
    );
  }
  const pairedRatios = control.map((value, index) => value / candidate[index]);
  const pairedMedian = median(pairedRatios);
  const pairedMedianBootstrap95 = exactBootstrap95(pairedRatios, median);
  const passes = pairedMedian >= protocol.minimumShellSpeedup &&
    pairedMedianBootstrap95[0] >= protocol.minimumShellMedianBootstrapLower;
  if (pairedMedian < protocol.minimumShellSpeedup) {
    problems.push(
      `shell: paired speedup ${pairedMedian.toFixed(4)} below ` +
        `${protocol.minimumShellSpeedup.toFixed(2)}`,
    );
  }
  if (pairedMedianBootstrap95[0] < protocol.minimumShellMedianBootstrapLower) {
    problems.push(
      `shell: lower bound ${pairedMedianBootstrap95[0].toFixed(4)} below ` +
        `${protocol.minimumShellMedianBootstrapLower.toFixed(2)}`,
    );
  }
  shell = {
    control,
    candidate,
    pairedRatios,
    controlMedian: median(control),
    candidateMedian: median(candidate),
    pairedMedian,
    pairedMedianBootstrap95,
    controlSpread: spread(control),
    candidateSpread: spread(candidate),
    passes,
  };
}
for (const phase of phases) {
  const controlRaw = pairs.map((pair) => pair.control.summary.samples[phase]);
  const candidateRaw = pairs.map((pair) => pair.candidate.summary.samples[phase]);
  const control = controlRaw.map(median);
  const candidate = candidateRaw.map(median);
  const pairedRatios = candidate.map((value, index) => value / control[index]);
  const pairedMedian = median(pairedRatios);
  const controlSpread = spread(control);
  const candidateSpread = spread(candidate);
  const passes = pairedMedian <= 1 + protocol.maxSlowdown;
  if (!passes) problems.push(`${phase}: paired median ${pairedMedian.toFixed(4)} exceeds 1.03`);
  const pairedMedianBootstrap95 = exactBootstrap95(pairedRatios, median);
  if (
    strengthenedBrowserProtocol &&
    pairedMedianBootstrap95[1] > protocol.maximumPairedMedianBootstrapUpper
  ) {
    problems.push(
      `${phase}: upper bound ${pairedMedianBootstrap95[1].toFixed(4)} exceeds 1.10`,
    );
  }
  if (!establishedValidityConfirmation && !strengthenedBrowserProtocol &&
      controlSpread > protocol.maximumWithinSideSpread) {
    problems.push(`${phase}: control spread ${controlSpread.toFixed(3)} exceeds 1.25`);
  }
  if (!establishedValidityConfirmation && !strengthenedBrowserProtocol &&
      candidateSpread > protocol.maximumWithinSideSpread) {
    problems.push(`${phase}: candidate spread ${candidateSpread.toFixed(3)} exceeds 1.25`);
  }
  analysis[phase] = {
    controlRaw,
    candidateRaw,
    control,
    candidate,
    pairedRatios,
    controlMedian: median(control),
    candidateMedian: median(candidate),
    pairedMedian,
    pairedMedianBootstrap95,
    controlSpread,
    candidateSpread,
    passes,
  };
}

const report = {
  schema: protocol.schema,
  experiment: protocol.experiment,
  measurementValid: strengthenedBrowserProtocol ? true : problems.length === 0,
  gatePassed: problems.length === 0,
  protocol: { plannedAt: protocol.plannedAt, order: observedOrder },
  method: {
    pairs: expectedPairs,
    repetitionsPerLeg: expectedRepetitions,
    pairing: "alternating fresh browser process/profile and RV64 guest",
    statistic: strengthenedBrowserProtocol
      ? "paired median of per-leg three-repetition medians"
      : "paired median candidate/control elapsed-time ratio",
    interval: "exact paired bootstrap percentile 95%",
    maxSlowdown: 0.03,
    maximumWithinSideSpread: protocol.maximumWithinSideSpread,
    maximumPairedMedianBootstrapUpper:
      protocol.maximumPairedMedianBootstrapUpper ?? null,
    ...(prebootR075 ? {
      shellStatistic: "paired median control/candidate shell-time speedup",
      minimumShellSpeedup: protocol.minimumShellSpeedup,
      minimumShellMedianBootstrapLower: protocol.minimumShellMedianBootstrapLower,
      jitConfigurationLifecycle: "preboot",
    } : {}),
  },
  browser: shortBackoffR074 || prebootR075
    ? expectedBrowser
    : { product: [...browserProducts][0], jsVersion: [...jsVersions][0] },
  hostCpuAffinity: protocol.cpuAffinity,
  ...(prebootR075 ? {
    configurations,
    expectedPageUrls: protocol.urls,
  } : {}),
  expectedMainWasmSha256: expectedWasm,
  expectedPageSha256: expectedPage,
  expectedRv64ArchiveSha256: expectedArchive,
  ...(shortBackoffR074 || prebootR075 ? { expectedHarnessSha256: expectedHarness } : {}),
  artifacts: prebootR075
    ? {
      control: JSON.parse([...artifactSnapshotsBySide.control][0]),
      candidate: JSON.parse([...artifactSnapshotsBySide.candidate][0]),
    }
    : JSON.parse([...artifactSnapshots][0]),
  ...(prebootR075 ? {
    shell,
    shellStaticModuleIndex: pairs[0].control.jitAtShell.staticT0.moduleIndex,
  } : {}),
  phases: analysis,
  problems,
};
const serialized = `${JSON.stringify(report, null, 2)}\n`;
if (outputArgument) {
  writeFileSync(resolve(outputArgument.split("=", 2)[1]), serialized, { flag: "wx" });
}
process.stdout.write(serialized);
if (problems.length) {
  const label = prebootR075 ? "R075" : shortBackoffR074 ? "R074" :
    sampledOnlyR073 ? "R073" : "R072";
  throw new Error(`${label}_BROWSER_GATE_FAIL: ${problems.join("; ")}`);
}
const label = prebootR075 ? "R075" : shortBackoffR074 ? "R074" :
  sampledOnlyR073 ? "R073" : "R072";
console.error(`${label}_BROWSER_GATE_PASS`);
