#!/usr/bin/env node

import assert from "node:assert/strict";
import { evaluateR105, R105_REQUIREMENTS as req } from "./r105-native-gate-lib.mjs";

const wasmPath = "/immutable/r105-candidate.wasm";
const trials = [];
for (const row of req.rows) {
  for (let rep = 1; rep <= req.repetitions; rep++) {
    const order = rep & 1 ? ["control", "candidate"] : ["candidate", "control"];
    for (const [orderIndex, side] of order.entries()) {
      const phase = row === "boot" ? "first" : "steady";
      const elapsed = side === "control" ? 100 : row === "boot" ? 98 : 100.5;
      trials.push({
        row,
        rep,
        side,
        order: orderIndex + 1,
        orderVector: order,
        hostBeforeMs: 10,
        hostAfterMs: 10,
        result: {
          measurementEligible: false,
          side: "rewrite",
          row,
          phases: {
            [phase]: {
              value: elapsed,
              counters: { guestInstructions: "100000000" },
            },
          },
          runtime: {
            identity: {
              wasmSha256: req.wasmSha256,
              loaderSha256: req.loaderSha256,
            },
            schedulerCadence: {
              name: req.cadence,
              rv64SlicesPerEventLoopTurn: 1,
            },
            guest: { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" },
            requestedPolicy: { name: "production-page" },
            policyProblems: [],
            jitProof: { enabledRequested: true, generatedInstructions: "1", dispatches: "1" },
            diagnostic: {
              integratedScalarT0: side === "control" ? 0 : 1,
              rewriteWasmOverride: wasmPath,
            },
          },
        },
      });
    }
  }
}

const side = (median, fingerprints) => ({
  values: Array(req.repetitions).fill(median),
  median,
  spread: 1,
  inputs: ["same-modern-input"],
  fingerprints,
});
const aggregate = (control, candidate, speedup, confidence, fingerprints) => ({
  control: side(control, fingerprints),
  candidate: side(candidate, fingerprints),
  pairedCandidateSpeedup: {
    raw: Array(req.repetitions).fill(speedup),
    median: speedup,
    medianConfidence95: confidence,
  },
});
const report = {
  measurementValid: true,
  authoritative: false,
  problems: [],
  configuration: {
    rows: [...req.rows],
    reps: req.repetitions,
    maximumSampleSpread: 1.25,
    controlConfig: {
      SCORECARD_V2_REWRITE_WASM: wasmPath,
      SCORECARD_V2_INTEGRATED_SCALAR_T0: "0",
    },
    candidateConfig: {
      SCORECARD_V2_REWRITE_WASM: wasmPath,
      SCORECARD_V2_INTEGRATED_SCALAR_T0: "1",
    },
    wasmBySide: {
      control: { sha256: req.wasmSha256 },
      candidate: { sha256: req.wasmSha256 },
    },
  },
  hostProbeSpread: 1.01,
  hostCpuAffinity: req.hostCpuAffinity,
  trials,
  aggregates: {
    boot: aggregate(100, 98, 100 / 98, [1.01, 1.03], []),
    compile: aggregate(100, 100.5, 0.995, [0.99, 1.005], [req.compileFingerprint]),
    python: aggregate(100, 100.5, 0.995, [0.99, 1.005], [req.pythonFingerprint]),
  },
};

const pass = evaluateR105(report);
assert.equal(pass.admitProductGates, true);
assert.equal(pass.checks.bootNormalizedMips, true);

const weakBoot = structuredClone(report);
weakBoot.aggregates.boot.pairedCandidateSpeedup.median = 1.009;
assert.equal(evaluateR105(weakBoot).admitProductGates, false);

const weakBootConfidence = structuredClone(report);
weakBootConfidence.aggregates.boot.pairedCandidateSpeedup.medianConfidence95[0] = 0.999;
assert.equal(evaluateR105(weakBootConfidence).admitProductGates, false);

const fakeBootWork = structuredClone(report);
for (const trial of fakeBootWork.trials) {
  if (trial.row === "boot" && trial.side === "candidate") {
    trial.result.phases.first.counters.guestInstructions = "97000000";
  }
}
assert.equal(evaluateR105(fakeBootWork).checks.bootNormalizedMips, false);

const compileMedianRegression = structuredClone(report);
compileMedianRegression.aggregates.compile.pairedCandidateSpeedup.median = 0.989;
assert.equal(evaluateR105(compileMedianRegression).admitProductGates, false);

const compileEstablishedRegression = structuredClone(report);
compileEstablishedRegression.aggregates.compile.pairedCandidateSpeedup.medianConfidence95[1] = 0.999;
assert.equal(evaluateR105(compileEstablishedRegression).admitProductGates, false);

const pythonMedianRegression = structuredClone(report);
pythonMedianRegression.aggregates.python.pairedCandidateSpeedup.median = 0.989;
assert.equal(evaluateR105(pythonMedianRegression).admitProductGates, false);

const pythonEstablishedRegression = structuredClone(report);
pythonEstablishedRegression.aggregates.python.pairedCandidateSpeedup.medianConfidence95[1] = 0.999;
assert.equal(evaluateR105(pythonEstablishedRegression).admitProductGates, false);

const wrongOutput = structuredClone(report);
wrongOutput.aggregates.compile.candidate.fingerprints = ["wrong"];
assert.equal(evaluateR105(wrongOutput).checks.integrity, false);

const wrongDiagnostic = structuredClone(report);
wrongDiagnostic.trials.find((trial) => trial.side === "candidate")
  .result.runtime.diagnostic.integratedScalarT0 = 0;
assert.equal(evaluateR105(wrongDiagnostic).checks.integrity, false);

const extraConfiguration = structuredClone(report);
extraConfiguration.configuration.candidateConfig.SCORECARD_V2_PAGE_THRESHOLD = "1";
assert.equal(evaluateR105(extraConfiguration).checks.integrity, false);

console.log("R105 native gate selftest: PASS");
