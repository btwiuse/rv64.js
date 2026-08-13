#!/usr/bin/env node

import assert from "node:assert/strict";
import { evaluateR096, R096_REQUIREMENTS } from "./r096-native-gate-lib.mjs";

const hash = "a".repeat(64);
const trials = [];
for (const row of R096_REQUIREMENTS.rows) {
  for (let rep = 1; rep <= R096_REQUIREMENTS.repetitions; rep++) {
    for (const side of ["control", "candidate"]) {
      const phase = row === "boot" ? "first" : "steady";
      trials.push({
        row, rep, side,
        hostBeforeMs: 1,
        hostAfterMs: 1,
        result: {
          runtime: {
            identity: { wasmSha256: hash },
            schedulerCadence: {
              name: R096_REQUIREMENTS.cadence,
              rv64SlicesPerEventLoopTurn: 1,
            },
            guest: { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" },
            requestedPolicy: { name: "production-page" },
            effectivePolicy: { tailChainAccountingEnabled: side === "control" ? "1" : "0" },
            policyProblems: [],
            jitProof: { generatedInstructions: "1" },
            diagnostic: { tailChainAccounting: side === "control" ? 1 : 0 },
          },
          phases: { [phase]: { counters: { chainHops: side === "control" ? "10" : "0" } } },
        },
      });
    }
  }
}
const side = (median) => ({
  values: Array(R096_REQUIREMENTS.repetitions).fill(median),
  median,
  inputs: ["input"],
  fingerprints: ["output"],
});
const aggregate = (control, candidate, speedup) => ({
  control: side(control),
  candidate: side(candidate),
  pairedCandidateSpeedup: {
    raw: Array(R096_REQUIREMENTS.repetitions).fill(speedup),
    median: speedup,
    medianConfidence95: [speedup, speedup],
  },
});
const report = {
  measurementValid: true,
  authoritative: false,
  problems: [],
  configuration: {
    rows: [...R096_REQUIREMENTS.rows],
    reps: R096_REQUIREMENTS.repetitions,
    controlConfig: { SCORECARD_V2_TAIL_CHAIN_ACCOUNTING: "1" },
    candidateConfig: { SCORECARD_V2_TAIL_CHAIN_ACCOUNTING: "0" },
    wasmBySide: { control: { sha256: hash }, candidate: { sha256: hash } },
  },
  hostProbeSpread: 1.01,
  trials,
  aggregates: {
    boot: aggregate(100, 101, 1 / 1.01),
    compile: aggregate(100, 98, 100 / 98),
    python: aggregate(100, 100, 1),
  },
};
const pass = evaluateR096(report, hash);
assert.equal(pass.advance, true);
const noCompileProof = structuredClone(report);
noCompileProof.aggregates.compile.pairedCandidateSpeedup.medianConfidence95[0] = 0.999;
assert.equal(evaluateR096(noCompileProof, hash).advance, false);
const badAccounting = structuredClone(report);
badAccounting.trials.find((trial) => trial.side === "candidate")
  .result.phases.first.counters.chainHops = "1";
assert.equal(evaluateR096(badAccounting, hash).advance, false);
const pythonRegression = structuredClone(report);
pythonRegression.aggregates.python.candidate.median = 103;
assert.equal(evaluateR096(pythonRegression, hash).advance, false);
console.log("R096 native gate selftest: ok");
