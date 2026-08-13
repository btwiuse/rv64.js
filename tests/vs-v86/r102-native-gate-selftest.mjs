#!/usr/bin/env node

import assert from "node:assert/strict";
import { evaluateR102, R102_REQUIREMENTS as req } from "./r102-native-gate-lib.mjs";

const trials = [];
for (const row of req.rows) {
  for (let rep = 1; rep <= req.repetitions; rep++) {
    for (const side of ["control", "candidate"]) {
      const phase = row === "boot" ? "first" : "steady";
      const elapsed = side === "control" ? 100 : row === "compile" ? 96 : 101;
      trials.push({
        row,
        rep,
        side,
        hostBeforeMs: 10,
        hostAfterMs: 10,
        result: {
          phases: {
            [phase]: {
              value: elapsed,
              counters: { guestInstructions: "100000000" },
            },
          },
          runtime: {
            identity: {
              wasmSha256: side === "control"
                ? req.controlWasmSha256
                : req.candidateWasmSha256,
              loaderSha256: req.loaderSha256,
            },
            schedulerCadence: {
              name: req.cadence,
              rv64SlicesPerEventLoopTurn: 1,
            },
            guest: { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" },
            requestedPolicy: { name: "production-page" },
            policyProblems: [],
            jitProof: { generatedInstructions: "1" },
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
const aggregate = (control, candidate, speedup, fingerprints) => ({
  control: side(control, fingerprints),
  candidate: side(candidate, fingerprints),
  pairedCandidateSpeedup: {
    raw: Array(req.repetitions).fill(speedup),
    median: speedup,
    medianConfidence95: [speedup, speedup],
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
    controlConfig: { SCORECARD_V2_REWRITE_WASM: "/control.wasm" },
    candidateConfig: { SCORECARD_V2_REWRITE_WASM: "/candidate.wasm" },
    wasmBySide: {
      control: { sha256: req.controlWasmSha256 },
      candidate: { sha256: req.candidateWasmSha256 },
    },
  },
  hostProbeSpread: 1.01,
  hostCpuAffinity: req.hostCpuAffinity,
  trials,
  aggregates: {
    boot: aggregate(100, 101, 100 / 101, []),
    compile: aggregate(100, 96, 100 / 96, [req.compileFingerprint]),
    python: aggregate(100, 101, 100 / 101, [req.pythonFingerprint]),
  },
};

const pass = evaluateR102(report);
assert.equal(pass.admitProductGates, true);
assert.equal(pass.checks.compileNormalizedMips, true);

const weakCompile = structuredClone(report);
weakCompile.aggregates.compile.pairedCandidateSpeedup.median = 1.02;
assert.equal(evaluateR102(weakCompile).admitProductGates, false);

const weakConfidence = structuredClone(report);
weakConfidence.aggregates.compile.pairedCandidateSpeedup.medianConfidence95[0] = 0.999;
assert.equal(evaluateR102(weakConfidence).admitProductGates, false);

const wrongOutput = structuredClone(report);
wrongOutput.aggregates.compile.candidate.fingerprints = ["wrong"];
assert.equal(evaluateR102(wrongOutput).checks.integrity, false);

const pythonRegression = structuredClone(report);
pythonRegression.aggregates.python.candidate.median = 104;
assert.equal(evaluateR102(pythonRegression).admitProductGates, false);

const fakeSpeedup = structuredClone(report);
for (const trial of fakeSpeedup.trials) {
  if (trial.row === "compile" && trial.side === "candidate") {
    trial.result.phases.steady.counters.guestInstructions = "90000000";
  }
}
assert.equal(evaluateR102(fakeSpeedup).checks.compileNormalizedMips, false);

const duplicateTrial = structuredClone(report);
duplicateTrial.trials[1] = structuredClone(duplicateTrial.trials[0]);
assert.equal(evaluateR102(duplicateTrial).checks.integrity, false);

const wrongHash = structuredClone(report);
wrongHash.configuration.wasmBySide.candidate.sha256 = "0".repeat(64);
assert.equal(evaluateR102(wrongHash).checks.integrity, false);

const wrongRepetitions = structuredClone(report);
wrongRepetitions.configuration.reps = 5;
assert.equal(evaluateR102(wrongRepetitions).checks.integrity, false);

console.log("R102 native gate selftest: PASS");
