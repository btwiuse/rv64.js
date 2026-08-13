#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const directory = resolve(process.argv[2] ?? "");
const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
if (!process.argv[2] || !outputArgument) {
  throw new Error(
    "usage: node tests/analyze-r125-wanix-browser-pairs.mjs RESULTS_DIR --output=REPORT",
  );
}
const protocol = JSON.parse(readFileSync(join(directory, "protocol.json"), "utf8"));
assert.equal(protocol.schema, 1);
assert.equal(protocol.experiment,
  "R125 fixed-bank WANIX browser guard with single-flight adapter");
assert.equal(protocol.pairs, 7);
assert.equal(protocol.repetitions, 3);
assert.equal(protocol.hostCpuAffinity, "8-15");
assert.deepEqual(protocol.phases, ["python", "sha256", "shared9p"]);
assert.deepEqual(protocol.configurations, { control: {}, candidate: {} });
assert.deepEqual(protocol.thresholds, {
  minimumPairedMedianSpeedup: 0.99,
  minimumPairedBootstrapUpper: 1.00,
  minimumPhaseBootstrapLower: 1 / 1.10,
  maximumWithinBrowserSpread: 1.25,
  minimumShared9pSampleSeconds: 2.0,
  exactP9WriteBytes: 33_554_432,
  minimumP9ReadBytes: 33_554_432,
  maximumP9TransferBytes: 4_096,
  minimumGeneratedCoverage: { python: 0.90, sha256: 0.90, shared9p: 0 },
});
assert.equal(protocol.archives.control.members.loader.sha256,
  protocol.archives.candidate.members.loader.sha256);
assert.equal(protocol.archives.control.members.adapter.sha256,
  protocol.archives.candidate.members.adapter.sha256);
assert.notEqual(protocol.archives.control.members.rv64Wasm.sha256,
  protocol.archives.candidate.members.rv64Wasm.sha256);
const files = readdirSync(directory).filter((name) => /^pair-\d+-(control|candidate)\.log$/.test(name));
assert.equal(files.length, 14, "exactly fourteen result logs required");
const expectedPhasePolicy = {
  pagePolicyEnabled: "1",
  pageThreshold: "131072",
  privilegedPageThresholdMultiplier: "32",
  pageQuantum: "1024",
  controlEntriesEnabled: "1",
  privilegedControlEntriesEnabled: "0",
  stablePageChainEnabled: "1",
  pageInflightLimit: "2",
  multiPageEntryCap: "512",
  multiPageControlPermille: "100",
  controlProfileEnabled: "0",
  regionPageCap: "2",
  regionLeaderCap: "512",
  regionTailChainEnabled: "1",
  regionTlbCacheEnabled: "1",
  regionTlbCacheMinAccesses: "4",
};

const runs = [];
let previousFinished = Date.parse(protocol.plannedAt);
for (const { pair, sides } of protocol.order) {
  for (const side of sides) {
    const path = join(directory, `pair-${pair}-${side}.log`);
    const lines = readFileSync(path, "utf8").split("\n");
    const summary = JSON.parse(lines[0]);
    const jit = JSON.parse(lines[1]);
    const started = Date.parse(summary.startedAt);
    const finished = Date.parse(summary.finishedAt);
    assert.ok(started >= previousFinished, `${pair}/${side} chronology`);
    assert.ok(finished >= started, `${pair}/${side} finish`);
    previousFinished = finished;
    assert.equal(summary.vm, "rv64-jit");
    assert.equal(summary.hostCpuAffinity, "8-15");
    assert.deepEqual(summary.browser, protocol.expectedBrowser);
    assert.equal(summary.artifactStable, true);
    assert.equal(summary.artifacts.page.sha256, protocol.pages[side].sha256);
    assert.equal(summary.artifacts.archives.rv64Jit.sha256, protocol.archives[side].sha256);
    for (const name of ["v86", "rv64Root", "x86Root"]) {
      assert.equal(
        summary.artifacts.archives[name].sha256,
        protocol.commonArchiveSha256[name],
        `${pair}/${side} ${name}`,
      );
    }
    assert.deepEqual(summary.guest, {
      machine: "riscv64", alpineRelease: "3.22.5", pythonVersion: "3.12.13",
    });
    assert.deepEqual(summary.phases, protocol.phases);
    assert.equal(summary.repetitions, 3);
    assert.equal(summary.phaseSync, true);
    assert.deepEqual(summary.jitConfiguration, {});
    assert.equal(summary.jitPolicy, null);
    assert.ok(summary.shellMs > 0 && summary.benchMs > 0);
    for (const phase of protocol.phases) {
      assert.equal(summary.samples[phase].length, 3, `${pair}/${side} ${phase} samples`);
      assert.ok(summary.samples[phase].every((value) => value > 0));
      const minimum = Math.min(...summary.samples[phase]);
      const maximum = Math.max(...summary.samples[phase]);
      assert.ok(maximum / minimum <= protocol.thresholds.maximumWithinBrowserSpread,
        `${pair}/${side} ${phase} within-browser spread ${maximum / minimum}`);
      if (phase === "shared9p") {
        assert.ok(summary.samples[phase].every(
          (value) => value >= protocol.thresholds.minimumShared9pSampleSeconds),
        `${pair}/${side} ${phase} reused a short sample`);
      }
      assert.deepEqual(summary.correctness[phase], { complete: 3, correct: 3 });
    }
    assert.ok(jit.jitAtShell.loader.modules > 0, `${pair}/${side} JIT modules at shell`);
    assert.equal(jit.jitAtShell.staticT0.supported, false);
    assert.equal(jit.jitAtShell.pagePolicy.enabled, "1");
    assert.equal(jit.jitAtShell.pagePolicy.threshold, "131072");
    assert.equal(jit.jitAtShell.pagePolicy.regionTlbCacheEnabled, "1");
    assert.equal(jit.jitAtShell.pagePolicy.regionTlbCacheMinAccesses, "4");
    for (let repetition = 1; repetition <= 3; repetition++) {
      for (const phase of protocol.phases) {
        const key = `${phase}${repetition}`;
        const proof = jit.jitPhases[key];
        assert.ok(proof, `${pair}/${side} ${key} proof`);
        const instructions = BigInt(proof.instructions);
        const generated = BigInt(proof.generatedRetired);
        const interpreted = BigInt(proof.interpreterRetired);
        assert.ok(instructions > 0n && generated > 0n);
        assert.equal(generated + interpreted, instructions);
        for (const [field, expected] of Object.entries(expectedPhasePolicy)) {
          assert.equal(proof[field], expected, `${pair}/${side} ${key} ${field}`);
        }
        assert.ok(proof.generatedCoverage >=
          protocol.thresholds.minimumGeneratedCoverage[phase]);
        if (phase === "shared9p") {
          assert.equal(proof.p9WriteBytes, protocol.thresholds.exactP9WriteBytes,
            `${pair}/${side} ${key} P9 write bytes`);
          assert.ok(proof.p9ReadBytes >= protocol.thresholds.minimumP9ReadBytes,
            `${pair}/${side} ${key} P9 read bytes`);
          assert.equal(proof.p9MaximumWrite, protocol.thresholds.maximumP9TransferBytes,
            `${pair}/${side} ${key} maximum P9 write`);
          assert.equal(proof.p9MaximumRead, protocol.thresholds.maximumP9TransferBytes,
            `${pair}/${side} ${key} maximum P9 read`);
        }
        for (const field of [
          "staticT0FastRetired", "staticT0SlowRetired", "staticT0Errors",
          "sampledStaticT0Retired", "sampledStaticT0Samples",
        ]) assert.equal(BigInt(proof[field]), 0n, `${pair}/${side} ${key} ${field}`);
      }
    }
    runs.push({ pair, side, summary, jit, started, finished });
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
const valueFor = (run, row) => row === "shell"
  ? run.summary.shellMs
  : median(run.summary.samples[row]);
const rows = {};
const problems = [];
for (const row of ["shell", ...protocol.phases]) {
  const paired = protocol.order.map(({ pair }) => {
    const control = runs.find((run) => run.pair === pair && run.side === "control");
    const candidate = runs.find((run) => run.pair === pair && run.side === "candidate");
    const controlValue = valueFor(control, row);
    const candidateValue = valueFor(candidate, row);
    return { pair, control: controlValue, candidate: candidateValue,
      speedup: controlValue / candidateValue };
  });
  const speedups = paired.map((pair) => pair.speedup);
  const pairedMedianSpeedup = median(speedups);
  const pairedMedianBootstrap95 = exactBootstrap95(speedups);
  if (pairedMedianSpeedup < protocol.thresholds.minimumPairedMedianSpeedup) {
    problems.push(`${row} paired speedup ${pairedMedianSpeedup} below median guard`);
  }
  if (pairedMedianBootstrap95[1] < protocol.thresholds.minimumPairedBootstrapUpper) {
    problems.push(`${row} upper ${pairedMedianBootstrap95[1]} establishes regression`);
  }
  if (row !== "shell" &&
      pairedMedianBootstrap95[0] < protocol.thresholds.minimumPhaseBootstrapLower) {
    problems.push(`${row} lower ${pairedMedianBootstrap95[0]} exceeds qualified variance guard`);
  }
  const withinBrowserSpreads = row === "shell" ? null : Object.fromEntries(runs.map((run) => {
    const samples = run.summary.samples[row];
    return [`pair-${run.pair}-${run.side}`, Math.max(...samples) / Math.min(...samples)];
  }));
  rows[row] = {
    controlMedian: median(paired.map((pair) => pair.control)),
    candidateMedian: median(paired.map((pair) => pair.candidate)),
    pairedMedianSpeedup,
    pairedMedianBootstrap95,
    withinBrowserSpreads,
    paired,
  };
}
const report = {
  schema: 1,
  experiment: protocol.experiment,
  measurementValid: true,
  gatePassed: problems.length === 0,
  method: {
    pairs: 7,
    repetitionsPerBrowser: 3,
    comparison: "exact R124 control versus fixed-register-bank candidate; common R125 adapter fix",
    pairing: "alternating fresh Chrome process/profile/WANIX Worker/RV64 guest",
    perBrowserStatistic: "median of three guest samples",
    pairedStatistic: "median control/candidate elapsed-time speedup",
    interval: "exact paired bootstrap percentile 95%",
    thresholds: protocol.thresholds,
  },
  browser: protocol.expectedBrowser,
  hostCpuAffinity: protocol.hostCpuAffinity,
  integrationGuest: protocol.integrationGuest,
  artifacts: { pages: protocol.pages, archives: protocol.archives, deployed: protocol.deployed },
  rows,
  problems,
};
writeFileSync(resolve(outputArgument.split("=", 2)[1]), `${JSON.stringify(report, null, 2)}\n`, {
  flag: "wx",
});
for (const [row, result] of Object.entries(rows)) {
  console.log(`${row}: ${result.pairedMedianSpeedup.toFixed(3)}x ` +
    `CI=[${result.pairedMedianBootstrap95.map((value) => value.toFixed(3)).join(", ")}]`);
}
if (problems.length) throw new Error(`R125_WANIX_BROWSER_GATE_FAIL: ${problems.join("; ")}`);
console.log("R125_WANIX_BROWSER_GATE_PASS");
