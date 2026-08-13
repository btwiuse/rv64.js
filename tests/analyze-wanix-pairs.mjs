#!/usr/bin/env node

// Analyze process-isolated, alternating WANIX benchmark pairs. A pair is the
// unit of resampling, so browser/guest warm-up cannot be mixed across VMs. The
// schema-3 mode is R076's fixed three-repetition candidate/v86 product guard;
// schema 2 retains the historical one-repetition interface.

import assert from "node:assert/strict";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const directoryArgument = process.argv.slice(2).find((argument) => !argument.startsWith("--"));
if (!directoryArgument) {
  throw new Error("usage: analyze-wanix-pairs.mjs RESULTS_DIR [--max-slowdown=0.10]");
}
const resultsDirectory = resolve(directoryArgument);
const slowdownArgument = process.argv.find((argument) => argument.startsWith("--max-slowdown="));
const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
const protocol = JSON.parse(readFileSync(join(resultsDirectory, "protocol.json"), "utf8"));
assert.ok(protocol.schema === 2 || protocol.schema === 3,
  "unsupported experiment protocol schema");
const r076CandidateV86 = protocol.schema === 3;
const repetitions = r076CandidateV86 ? 3 : 1;
const expectedR076 = {
  experiment: "R076 preboot sampled-backoff candidate versus copy/v86",
  url:
    "http://127.0.0.1:8765/examples/" +
    "v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html",
  browser: {
    protocolVersion: "1.3",
    product: "Chrome/150.0.7871.186",
    revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
    userAgent:
      "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
      "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
    jsVersion: "15.0.245.21",
  },
  configuration: {
    staticSystemT0: false,
    sampledStaticT0: true,
    sampledStaticT0Backoff: true,
  },
  wasm: "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c",
  artifacts: {
    page: "7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413",
    rv64Jit: "e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c",
    rv64Root: "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb",
    v86: "7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771",
    x86Root: "09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320",
  },
  candidateProof: [
    "staticT0FastRetired",
    "sampledStaticT0Retired",
    "sampledStaticT0Samples",
    "sampledStaticT0InterruptPolls",
    "sampledStaticT0ShortMarks",
    "sampledStaticT0ShortBypasses",
  ],
  runner: "8d37e7b20186253a0b7e71e5b7c28f3d8ee3b34a49a7eb4374553c5b80ee4e80",
  harness: "525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545",
};
assert.ok(Number.isInteger(protocol.pairs) && protocol.pairs >= 3 && protocol.pairs <= 7,
  "protocol pair count");
const maxSlowdown = Number(
  slowdownArgument?.split("=", 2)[1] ?? protocol.maxSlowdown,
);
if (!Number.isFinite(maxSlowdown) || maxSlowdown < 0 || maxSlowdown > 1) {
  throw new Error("--max-slowdown must be a fraction from 0 through 1");
}
assert.equal(maxSlowdown, protocol.maxSlowdown,
  "analysis margin differs from the pre-registered protocol");

const phases = ["python", "sha256", "shared9p"];
assert.deepEqual(protocol.phases, phases, "protocol phase set");
assert.equal(protocol.repetitions, repetitions, "protocol repetition count");
assert.equal(protocol.phaseSync, true, "protocol phase synchronization");
assert.equal(protocol.jitPolicy, null, "protocol JIT policy override");
assert.ok(
  protocol.jitConfiguration &&
    typeof protocol.jitConfiguration === "object" &&
    !Array.isArray(protocol.jitConfiguration),
  "protocol JIT configuration override",
);
assert.ok(
  protocol.expectedMainWasmSha256 === null ||
    protocol.expectedMainWasmSha256 === undefined ||
    /^[0-9a-f]{64}$/.test(protocol.expectedMainWasmSha256),
  "protocol expected main Wasm SHA-256",
);
assert.deepEqual(protocol.minimumGeneratedCoverage, {
  python: 0.90,
  sha256: 0.90,
  shared9p: 0,
}, "protocol generated-coverage gates");
assert.match(protocol.cpuAffinity ?? "", /^\d+(?:[-,]\d+)*$/, "protocol CPU affinity");
if (r076CandidateV86) {
  assert.equal(protocol.experiment, expectedR076.experiment, "R076 experiment identity");
  assert.equal(protocol.pairs, 7, "R076 pair count");
  assert.equal(protocol.maxSlowdown, 0.10, "R076 noninferiority margin");
  assert.equal(protocol.url, expectedR076.url, "R076 page URL");
  assert.equal(protocol.cpuAffinity, "8-15", "R076 CPU affinity");
  assert.deepEqual(protocol.expectedBrowser, expectedR076.browser, "R076 browser identity");
  assert.deepEqual(protocol.jitConfiguration, expectedR076.configuration,
    "R076 candidate configuration");
  assert.equal(protocol.expectedMainWasmSha256, expectedR076.wasm, "R076 Wasm identity");
  assert.deepEqual(protocol.expectedArtifacts, expectedR076.artifacts,
    "R076 artifact identities");
  assert.equal(protocol.maximumPairedMedianBootstrapUpper, 1.10,
    "R076 confidence limit");
  assert.equal(protocol.jitConfigurationLifecycle, "preboot", "R076 JIT lifecycle");
  assert.deepEqual(protocol.requiredCandidateProof, expectedR076.candidateProof,
    "R076 candidate proof set");
  assert.equal(protocol.toolManifest?.runner, expectedR076.runner, "R076 runner identity");
  assert.equal(protocol.toolManifest?.harness, expectedR076.harness,
    "R076 harness identity");
}
const plannedAt = Date.parse(protocol.plannedAt);
assert.ok(Number.isFinite(plannedAt), "protocol plannedAt timestamp");
const expectedOrder = Array.from({ length: protocol.pairs }, (_, index) => ({
  pair: index + 1,
  vms: index % 2 === 0 ? ["rv64-jit", "x86"] : ["x86", "rv64-jit"],
}));
assert.deepEqual(protocol.order, expectedOrder, "protocol must alternate the first VM by pair");
const pairPattern = /^pair-(\d+)-(rv64-jit|x86)\.log$/;
const pairNumbers = [...new Set(readdirSync(resultsDirectory).flatMap((name) => {
  const match = pairPattern.exec(name);
  return match ? [Number(match[1])] : [];
}))].sort((a, b) => a - b);
assert.deepEqual(
  pairNumbers,
  Array.from({ length: protocol.pairs }, (_, index) => index + 1),
  "result pair numbers differ from the pre-registered protocol",
);

function median(values) {
  const ordered = [...values].sort((a, b) => a - b);
  const middle = Math.floor(ordered.length / 2);
  return ordered.length % 2
    ? ordered[middle]
    : (ordered[middle - 1] + ordered[middle]) / 2;
}

function geometricMean(values) {
  return Math.exp(values.reduce((sum, value) => sum + Math.log(value), 0) / values.length);
}

function quantile(values, probability) {
  const ordered = [...values].sort((a, b) => a - b);
  return ordered[Math.floor((ordered.length - 1) * probability)];
}

// Exact paired bootstrap for the small, deliberately process-expensive sample
// sets used here. Refuse an accidental combinatorial explosion rather than
// silently changing the statistical method.
function exactBootstrap95(values, statistic) {
  if (values.length > 7) {
    throw new Error("exact bootstrap supports at most seven pairs");
  }
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

function validateDigest(record, label, hasSource = false) {
  assert.equal(typeof record?.url, "string", `${label} URL`);
  assert.ok(record.url.length > 0, `${label} URL`);
  assert.ok(Number.isInteger(record?.bytes) && record.bytes > 0, `${label} byte count`);
  assert.match(record?.sha256 ?? "", /^[0-9a-f]{64}$/, `${label} SHA-256`);
  if (hasSource) {
    assert.equal(typeof record?.source, "string", `${label} binding source`);
    assert.ok(record.source.length > 0, `${label} binding source`);
  }
}

function load(pair, vm) {
  const path = join(resultsDirectory, `pair-${pair}-${vm}.log`);
  const text = readFileSync(path, "utf8");
  const lines = text.split("\n");
  const summary = JSON.parse(lines[0]);
  assert.equal(summary.vm, vm, `${basename(path)} VM identity`);
  assert.equal(summary.repetitions, repetitions, `${basename(path)} repetition count`);
  assert.equal(summary.phaseSync, true, `${basename(path)} must use phase synchronization`);
  assert.equal(summary.jitPolicy, null, `${basename(path)} has a policy override`);
  assert.deepEqual(
    summary.jitConfiguration,
    vm === "rv64-jit" ? protocol.jitConfiguration : {},
    `${basename(path)} JIT configuration`,
  );
  assert.equal(summary.hostCpuAffinity, protocol.cpuAffinity, `${basename(path)} CPU affinity`);
  if (r076CandidateV86) {
    assert.deepEqual(summary.browser, expectedR076.browser, `${basename(path)} browser identity`);
  }
  assert.equal(summary.artifactStable, true, `${basename(path)} artifact stability`);
  validateDigest(summary.artifacts?.page, `${basename(path)} page`);
  assert.equal(
    summary.artifacts.page.url,
    new URL(protocol.url).href,
    `${basename(path)} comparison page URL`,
  );
  assert.deepEqual(
    Object.keys(summary.artifacts?.archives ?? {}).sort(),
    ["rv64Jit", "rv64Root", "v86", "x86Root"],
    `${basename(path)} artifact set`,
  );
  for (const [name, record] of Object.entries(summary.artifacts.archives)) {
    validateDigest(record, `${basename(path)} ${name}`, true);
  }
  if (r076CandidateV86) {
    assert.equal(summary.artifacts.page.sha256, expectedR076.artifacts.page,
      `${basename(path)} page identity`);
    for (const name of ["rv64Jit", "rv64Root", "v86", "x86Root"]) {
      assert.equal(summary.artifacts.archives[name].sha256, expectedR076.artifacts[name],
        `${basename(path)} ${name} identity`);
    }
  }
  const startedAt = Date.parse(summary.startedAt);
  const finishedAt = Date.parse(summary.finishedAt);
  assert.ok(Number.isFinite(startedAt), `${basename(path)} startedAt timestamp`);
  assert.ok(Number.isFinite(finishedAt), `${basename(path)} finishedAt timestamp`);
  assert.ok(startedAt >= plannedAt, `${basename(path)} predates its protocol`);
  assert.ok(finishedAt >= startedAt, `${basename(path)} negative run interval`);
  assert.deepEqual(summary.phases, phases, `${basename(path)} phase set`);
  assert.match(text, /checksum=38460b78/, `${basename(path)} Python checksum`);
  assert.match(text, /e09320c5b00b34bb/, `${basename(path)} SHA-256 digest`);
  if (r076CandidateV86) {
    assert.deepEqual(summary.guest, {
      machine: vm === "x86" ? "i686" : "riscv64",
      alpineRelease: "3.22.5",
      pythonVersion: "3.12.13",
    }, `${basename(path)} guest identity`);
    assert.equal(
      summary.jitConfigurationLifecycle,
      vm === "rv64-jit" ? "preboot" : "shell",
      `${basename(path)} JIT lifecycle`,
    );
    for (const phase of phases) {
      assert.deepEqual(summary.correctness?.[phase], {
        complete: repetitions,
        correct: repetitions,
      }, `${basename(path)} ${phase} correctness`);
    }
  } else {
    assert.match(text, /3\.22\.5/, `${basename(path)} Alpine release`);
    assert.match(
      text,
      vm === "x86" ? /i686 Python 3\.12\.13/ : /riscv64 Python 3\.12\.13/,
      `${basename(path)} guest identity`,
    );
  }
  for (const phase of phases) {
    assert.equal(summary.samples[phase].length, repetitions,
      `${basename(path)} ${phase} sample count`);
    for (const value of summary.samples[phase]) {
      assert.ok(Number.isFinite(value) && value > 0, `${basename(path)} ${phase} duration`);
    }
  }

  let jit;
  let jitAtShell;
  if (vm === "rv64-jit") {
    const jitRecord = JSON.parse(lines[1]);
    jit = jitRecord.jitPhases;
    jitAtShell = jitRecord.jitAtShell;
    if (r076CandidateV86) {
      assert.ok(jitAtShell?.staticT0, `${basename(path)} shell static-T0 proof`);
      assert.equal(jitAtShell.staticT0.systemEnabled, false,
        `${basename(path)} shell residual-static enable`);
      assert.equal(jitAtShell.staticT0.sampledEnabled, true,
        `${basename(path)} shell sampled enable`);
      assert.equal(jitAtShell.staticT0.sampledBackoffEnabled, true,
        `${basename(path)} shell sampled-backoff enable`);
      assert.ok(jitAtShell.staticT0.moduleIndex >= 0, `${basename(path)} shell module`);
      assert.equal(BigInt(jitAtShell.staticT0.systemErrors), 0n,
        `${basename(path)} shell static errors`);
      for (const field of [
        "sampledRetired", "samples", "interruptPolls", "shortSampleMarks",
        "shortSampleBypasses",
      ]) {
        assert.ok(BigInt(jitAtShell.staticT0[field]) > 0n,
          `${basename(path)} shell ${field}`);
      }
    }
    for (const phase of phases) {
      for (let repetition = 1; repetition <= repetitions; repetition++) {
        const key = repetitions === 1 ? phase : `${phase}${repetition}`;
        const proof = jit[key];
        assert.ok(proof, `${basename(path)} ${key} JIT proof`);
      assert.equal(proof.pagePolicyEnabled, "1", `${basename(path)} page policy`);
      assert.equal(proof.pageThreshold, "131072", `${basename(path)} page threshold`);
      assert.equal(
        proof.privilegedPageThresholdMultiplier,
        "32",
        `${basename(path)} privileged threshold multiplier`,
      );
      assert.equal(proof.pageQuantum, "1024", `${basename(path)} page quantum`);
      assert.equal(proof.controlEntriesEnabled, "1", `${basename(path)} control entries`);
      assert.equal(
        proof.privilegedControlEntriesEnabled,
        "0",
        `${basename(path)} privileged control entries`,
      );
      assert.equal(proof.stablePageChainEnabled, "1", `${basename(path)} stable page chain`);
      assert.equal(proof.pageInflightLimit, "2", `${basename(path)} in-flight limit`);
      assert.equal(proof.regionPageCap, "2", `${basename(path)} page cap`);
      assert.equal(proof.regionLeaderCap, "512", `${basename(path)} leader cap`);
      assert.equal(
        proof.regionTailChainEnabled,
        "1",
        `${basename(path)} region tail chain`,
      );
      assert.equal(proof.regionTlbCacheEnabled, "1", `${basename(path)} region TLB cache`);
      assert.equal(
        proof.regionTlbCacheMinAccesses,
        "4",
        `${basename(path)} region TLB cache threshold`,
      );
      assert.equal(proof.multiPageEntryCap, "512", `${basename(path)} entry cap`);
      assert.equal(proof.multiPageControlPermille, "100", `${basename(path)} control gate`);
      assert.ok(BigInt(proof.chainHops) > 0n, `${basename(path)} region tail-chain activity`);
      assert.ok(
        BigInt(proof.generatedRetired) > 0n,
        `${basename(path)} generated retirement`,
      );
      assert.equal(proof.controlProfileEnabled, "0", `${basename(path)} control profile`);
      assert.ok(
        proof.generatedCoverage >= protocol.minimumGeneratedCoverage[phase],
        `${basename(path)} generated coverage`,
      );
      if (
        protocol.jitConfiguration.staticSystemT0 !== undefined ||
        protocol.jitConfiguration.sampledStaticT0 !== undefined
      ) {
        const staticEnabled = protocol.jitConfiguration.staticSystemT0 === true;
        const sampledEnabled = protocol.jitConfiguration.sampledStaticT0 === true;
        assert.equal(
          proof.staticSystemT0Enabled,
          staticEnabled,
          `${basename(path)} static-system T0 enable`,
        );
        assert.equal(
          proof.sampledStaticT0Enabled,
          sampledEnabled,
          `${basename(path)} sampled-static T0 enable`,
        );
        assert.ok(proof.staticSystemT0ModuleIndex >= 0, `${basename(path)} static module`);
        if (r076CandidateV86) {
          assert.equal(
            proof.staticSystemT0ModuleIndex,
            jitAtShell.staticT0.moduleIndex,
            `${basename(path)} ${key} static module lifecycle`,
          );
        }
        assert.equal(BigInt(proof.staticT0Errors), 0n, `${basename(path)} static errors`);
        if (r076CandidateV86) {
          assert.equal(proof.sampledStaticT0BackoffEnabled, true,
            `${basename(path)} sampled-static backoff enable`);
        }
        if (staticEnabled) {
          assert.ok(BigInt(proof.staticT0FastRetired) > 0n, `${basename(path)} static retirement`);
        }
        if (sampledEnabled) {
          assert.ok(
            BigInt(proof.sampledStaticT0Retired) > 0n,
            `${basename(path)} sampled retirement`,
          );
          assert.ok(
            BigInt(proof.sampledStaticT0Samples) > 0n,
            `${basename(path)} sampled observations`,
          );
          assert.ok(
            BigInt(proof.sampledStaticT0InterruptPolls) > 0n,
            `${basename(path)} sampled interrupt polls`,
          );
        }
        if (r076CandidateV86) {
          for (const field of expectedR076.candidateProof) {
            assert.ok(BigInt(proof[field]) > 0n, `${basename(path)} ${key} ${field}`);
          }
        }
      }
      }
    }
  } else if (r076CandidateV86) {
    assert.ok(summary.externalP9, `${basename(path)} external 9P proof`);
    assert.equal(summary.externalP9.requests, summary.externalP9.replies,
      `${basename(path)} external 9P completion`);
    assert.equal(summary.externalP9.pending, 0, `${basename(path)} external 9P pending`);
    assert.equal(summary.externalP9.tagCollisions, 0,
      `${basename(path)} external 9P tag collisions`);
    for (let repetition = 1; repetition <= repetitions; repetition++) {
      const proof = summary.externalP9Phases?.[`shared9p${repetition}`];
      assert.ok(proof?.requests > 0, `${basename(path)} shared9p${repetition} requests`);
      assert.ok(proof.replies <= proof.requests,
        `${basename(path)} shared9p${repetition} reply accounting`);
      assert.equal(proof.requests - proof.replies, proof.pending,
        `${basename(path)} shared9p${repetition} pending accounting`);
      assert.ok(proof.pending >= 0 && proof.pending <= 1,
        `${basename(path)} shared9p${repetition} pending boundary`);
      assert.equal(proof.tagCollisions, 0,
        `${basename(path)} shared9p${repetition} tag collisions`);
    }
  }
  return { summary, jit, jitAtShell, startedAt, finishedAt };
}

const pairs = pairNumbers.map((pair) => ({
  pair,
  rv64: load(pair, "rv64-jit"),
  v86: load(pair, "x86"),
}));
if (r076CandidateV86) {
  assert.equal(
    new Set(pairs.map(({ rv64 }) => rv64.jitAtShell.staticT0.moduleIndex)).size,
    1,
    "R076 prepared static module index changed between samples",
  );
}
const browserProducts = new Set(pairs.flatMap(({ rv64, v86 }) => [
  rv64.summary.browser.product,
  v86.summary.browser.product,
]));
const jsVersions = new Set(pairs.flatMap(({ rv64, v86 }) => [
  rv64.summary.browser.jsVersion,
  v86.summary.browser.jsVersion,
]));
const hostCpuAffinities = new Set(pairs.flatMap(({ rv64, v86 }) => [
  rv64.summary.hostCpuAffinity ?? null,
  v86.summary.hostCpuAffinity ?? null,
]));
const artifactSnapshots = new Set(pairs.flatMap(({ rv64, v86 }) => [
  JSON.stringify(rv64.summary.artifacts),
  JSON.stringify(v86.summary.artifacts),
]));
assert.equal(browserProducts.size, 1, "browser product changed between samples");
assert.equal(jsVersions.size, 1, "JavaScript engine changed between samples");
assert.equal(hostCpuAffinities.size, 1, "host CPU affinity changed between samples");
assert.equal(artifactSnapshots.size, 1, "comparison artifact changed between samples");

let previousFinishedAt = plannedAt;
const observedOrder = [];
for (const { pair, vms } of expectedOrder) {
  const pairResult = pairs[pair - 1];
  for (const vm of vms) {
    const run = vm === "rv64-jit" ? pairResult.rv64 : pairResult.v86;
    assert.ok(
      run.startedAt >= previousFinishedAt,
      `pair-${pair}-${vm}.log overlaps or precedes the pre-registered run order`,
    );
    previousFinishedAt = run.finishedAt;
    observedOrder.push({ pair, vm, startedAt: run.summary.startedAt, finishedAt: run.summary.finishedAt });
  }
}

const analysis = Object.fromEntries(phases.map((phase) => {
  const rv64Raw = pairs.map(({ rv64: run }) => run.summary.samples[phase]);
  const v86Raw = pairs.map(({ v86: run }) => run.summary.samples[phase]);
  const rv64 = rv64Raw.map(median);
  const v86 = v86Raw.map(median);
  const pairedRatios = rv64.map((value, index) => value / v86[index]);
  const pairedGeometricMean = geometricMean(pairedRatios);
  const pairedGeometricMeanBootstrap95 = exactBootstrap95(pairedRatios, geometricMean);
  const pairedMedian = median(pairedRatios);
  const pairedMedianBootstrap95 = exactBootstrap95(pairedRatios, median);
  const noninferiorAtRequestedMargin = r076CandidateV86
    ? pairedMedian <= 1 + maxSlowdown &&
      pairedMedianBootstrap95[1] <= protocol.maximumPairedMedianBootstrapUpper
    : pairedGeometricMeanBootstrap95[1] <= 1 + maxSlowdown;
  return [phase, {
    ...(r076CandidateV86 ? { rv64Raw, v86Raw } : {}),
    rv64,
    v86,
    pairedRatios,
    rv64Median: median(rv64),
    v86Median: median(v86),
    ratioOfMedians: median(rv64) / median(v86),
    pairedMedian,
    pairedMedianBootstrap95,
    pairedGeometricMean,
    pairedGeometricMeanBootstrap95,
    noninferiorAtRequestedMargin,
  }];
}));
const failures = phases.filter((phase) => !analysis[phase].noninferiorAtRequestedMargin);

const report = {
  ...(r076CandidateV86 ? {
    schema: 3,
    experiment: expectedR076.experiment,
    measurementValid: true,
    gatePassed: failures.length === 0,
  } : { measurementValid: failures.length === 0 }),
  protocol: {
    plannedAt: protocol.plannedAt,
    order: observedOrder,
    jitConfiguration: protocol.jitConfiguration,
    expectedMainWasmSha256: protocol.expectedMainWasmSha256 ?? null,
    ...(r076CandidateV86 ? {
      expectedArtifacts: protocol.expectedArtifacts,
      toolManifest: protocol.toolManifest,
      jitConfigurationLifecycle: protocol.jitConfigurationLifecycle,
    } : {}),
  },
  method: {
    pairs: pairs.length,
    ...(r076CandidateV86 ? { repetitionsPerLeg: repetitions } : {}),
    pairing: "alternating fresh browser process/profile per VM",
    statistic: r076CandidateV86
      ? "paired median of per-leg three-repetition RV64/v86 medians"
      : "paired geometric mean of RV64/v86 elapsed-time ratios",
    interval: "exact paired bootstrap percentile 95%",
    maxSlowdown,
    ...(r076CandidateV86 ? {
      maximumPairedMedianBootstrapUpper: protocol.maximumPairedMedianBootstrapUpper,
    } : {}),
  },
  browser: r076CandidateV86 ? expectedR076.browser : {
      product: [...browserProducts][0],
      jsVersion: [...jsVersions][0],
    },
  hostCpuAffinity: [...hostCpuAffinities][0],
  artifacts: JSON.parse([...artifactSnapshots][0]),
  phases: analysis,
};
const serializedReport = `${JSON.stringify(report, null, 2)}\n`;
if (outputArgument) {
  const outputPath = resolve(outputArgument.split("=", 2)[1]);
  writeFileSync(outputPath, serializedReport, { flag: "wx" });
}
process.stdout.write(serializedReport);

if (failures.length) {
  throw new Error(
    `${r076CandidateV86 ? "R076 WANIX-v86 " : ""}` +
      `noninferiority margin failed for: ${failures.join(", ")}`,
  );
}
if (r076CandidateV86) console.error("R076_WANIX_V86_GATE_PASS");
