#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const directory = resolve(process.argv[2] ?? "");
const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
if (!process.argv[2] || !outputArgument) {
  throw new Error(
    "usage: node tests/analyze-r124-chrome-boot-pairs.mjs RESULTS_DIR --output=REPORT",
  );
}
const protocol = JSON.parse(readFileSync(join(directory, "protocol.json"), "utf8"));
const expectedGuest = { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" };
const expectedPolicy = {
  enabled: "1", threshold: "131072", privilegedThresholdMultiplier: "32",
  quantum: "1024", controlEntriesEnabled: "1", privilegedControlEntriesEnabled: "0",
  stableChainEnabled: "1", inflightLimit: "2", multiPageControlPermille: "100",
  pageCap: "2", leaderCap: "512", tailChainEnabled: "1",
  regionTlbCacheEnabled: "1", regionTlbCacheMinAccesses: "4",
};
assert.equal(protocol.schema, 1);
assert.equal(protocol.experiment, "R124 artifact A/B Chrome dual-clock modern Boot");
assert.equal(protocol.pairs, 7);
assert.equal(protocol.hostCpuAffinity, "8-15");
assert.deepEqual(protocol.guest, expectedGuest);
assert.deepEqual(protocol.thresholds, {
  minimumExecutionMedianSpeedup: 0.99,
  minimumExecutionBootstrapUpper: 1.00,
  minimumConstructionMedianSpeedup: 0.99,
  minimumConstructionBootstrapUpper: 1.00,
});
const pairFiles = readdirSync(directory).filter((name) =>
  /^pair-\d+-(control|candidate)\.json$/.test(name));
assert.equal(pairFiles.length, 14, "exactly fourteen result files required");

const expectedWasm = {
  control: "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
  candidate: "d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59",
};
const runs = [];
let previousFinished = Date.parse(protocol.plannedAt);
for (const { pair, sides } of protocol.order) {
  for (const side of sides) {
    const wrapper = JSON.parse(readFileSync(join(directory, `pair-${pair}-${side}.json`), "utf8"));
    const result = wrapper.result;
    const started = Date.parse(wrapper.startedAt);
    const finished = Date.parse(wrapper.finishedAt);
    assert.ok(started >= previousFinished, `pair ${pair} ${side} chronology`);
    assert.ok(finished >= started, `pair ${pair} ${side} finish`);
    previousFinished = finished;
    assert.equal(wrapper.hostCpuAffinity, "8-15");
    assert.deepEqual(wrapper.browser, protocol.expectedBrowser);
    assert.equal(result.experiment, protocol.experiment);
    assert.equal(result.variant, side);
    assert.deepEqual(result.guest, expectedGuest);
    assert.equal(result.assetHashes.wasm, expectedWasm[side]);
    assert.equal(result.assetHashes.kernel, protocol.assetManifest.kernel.sha256);
    assert.equal(result.assetHashes.initramfs, protocol.assetManifest.initramfs.sha256);
    assert.equal(result.outputMarkers.ready, true);
    assert.ok(result.timing.ms > 0);
    assert.ok(result.timing.constructionToMarkerMs > result.timing.ms);
    assert.equal(result.timing.quantum, "2000000");
    assert.equal(result.timing.cadence, "yield-after-pump-1-then-every-fourth");
    assert.equal(result.timing.marker, "SCORECARD_V2_READY");
    assert.equal(result.timing.yields, Math.ceil(result.timing.pumps / 4));
    const instructions = BigInt(result.instructions);
    const generated = BigInt(result.counters.generated);
    const interpreted = BigInt(result.counters.interpreted);
    assert.ok(instructions > 100_000_000n);
    assert.equal(generated + interpreted, instructions);
    assert.ok(generated > 0n && BigInt(result.counters.dispatches) > 0n);
    for (const field of ["staticFast", "sampled", "errors"]) {
      assert.equal(BigInt(result.counters[field]), 0n, `${side} ${field}`);
    }
    assert.deepEqual(result.policy, expectedPolicy);
    assert.ok(result.modules.count > 0 && result.modules.bytes > 0);
    runs.push({ pair, side, started, finished, result });
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
    if (depth === sample.length) { distribution.push(median(sample)); return; }
    for (const value of values) { sample[depth] = value; visit(depth + 1); }
  }
  visit(0);
  return [quantile(distribution, 0.025), quantile(distribution, 0.975)];
}
const paired = protocol.order.map(({ pair }) => {
  const control = runs.find((run) => run.pair === pair && run.side === "control").result;
  const candidate = runs.find((run) => run.pair === pair && run.side === "candidate").result;
  return {
    pair,
    execution: {
      controlMs: control.timing.ms,
      candidateMs: candidate.timing.ms,
      speedup: control.timing.ms / candidate.timing.ms,
    },
    constructionToMarker: {
      controlMs: control.timing.constructionToMarkerMs,
      candidateMs: candidate.timing.constructionToMarkerMs,
      speedup:
        control.timing.constructionToMarkerMs / candidate.timing.constructionToMarkerMs,
    },
  };
});
const summarizeClock = (name) => {
  const values = paired.map((pair) => pair[name].speedup);
  return {
    controlMedianMs: median(paired.map((pair) => pair[name].controlMs)),
    candidateMedianMs: median(paired.map((pair) => pair[name].candidateMs)),
    pairedMedianSpeedup: median(values),
    pairedMedianBootstrap95: exactBootstrap95(values),
  };
};
const clocks = {
  execution: summarizeClock("execution"),
  constructionToMarker: summarizeClock("constructionToMarker"),
};
const problems = [];
if (clocks.execution.pairedMedianSpeedup < protocol.thresholds.minimumExecutionMedianSpeedup) {
  problems.push(`execution speedup ${clocks.execution.pairedMedianSpeedup} below median guard`);
}
if (
  clocks.execution.pairedMedianBootstrap95[1] <
    protocol.thresholds.minimumExecutionBootstrapUpper
) {
  problems.push(
    `execution upper ${clocks.execution.pairedMedianBootstrap95[1]} establishes regression`,
  );
}
if (
  clocks.constructionToMarker.pairedMedianSpeedup <
    protocol.thresholds.minimumConstructionMedianSpeedup
) {
  problems.push(
    `construction speedup ${clocks.constructionToMarker.pairedMedianSpeedup} below median guard`,
  );
}
if (
  clocks.constructionToMarker.pairedMedianBootstrap95[1] <
    protocol.thresholds.minimumConstructionBootstrapUpper
) {
  problems.push(
    `construction upper ${clocks.constructionToMarker.pairedMedianBootstrap95[1]} ` +
      "establishes regression",
  );
}
const outputHashes = new Set(runs.map((run) => run.result.outputSha256));
const report = {
  schema: 1,
  experiment: protocol.experiment,
  measurementValid: true,
  gatePassed: problems.length === 0,
  method: {
    pairs: 7,
    pairing: "alternating fresh Chrome process/profile/Worker/modern guest",
    statistic: "paired median control/candidate speedup for both browser clocks",
    interval: "exact paired bootstrap percentile 95%",
    thresholds: protocol.thresholds,
  },
  browser: protocol.expectedBrowser,
  hostCpuAffinity: protocol.hostCpuAffinity,
  guest: expectedGuest,
  assetManifest: protocol.assetManifest,
  paired,
  clocks,
  // Linux boot text contains nondeterministic timing/random-init fields. The
  // exact guest identity and ready markers above are the correctness contract;
  // retain every full-output digest for audit without requiring equality.
  outputSha256: [...outputHashes],
  problems,
};
writeFileSync(resolve(outputArgument.split("=", 2)[1]), `${JSON.stringify(report, null, 2)}\n`, {
  flag: "wx",
});
console.log(
  `R124 Chrome Boot execution: ${clocks.execution.pairedMedianSpeedup.toFixed(3)}x ` +
  `CI=[${clocks.execution.pairedMedianBootstrap95
    .map((value) => value.toFixed(3)).join(", ")}]`,
);
console.log(
  `R124 Chrome Boot construction-to-marker: ` +
  `${clocks.constructionToMarker.pairedMedianSpeedup.toFixed(3)}x ` +
  `CI=[${clocks.constructionToMarker.pairedMedianBootstrap95
    .map((value) => value.toFixed(3)).join(", ")}]`,
);
if (problems.length) throw new Error(`R124_CHROME_BOOT_GATE_FAIL: ${problems.join("; ")}`);
console.log("R124_CHROME_BOOT_GATE_PASS");
