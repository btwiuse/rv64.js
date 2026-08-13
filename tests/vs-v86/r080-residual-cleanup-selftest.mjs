#!/usr/bin/env node

import assert from "node:assert/strict";
import {
  evaluateR080SourceRestoration,
  R080_CANDIDATE_WASM_SHA256,
  R080_CONTROL_WASM_SHA256,
  R080_LOADER_SHA256,
  R080_REPS,
  R080_ROWS,
} from "./r080-residual-cleanup-lib.mjs";

function workerResult(row, side) {
  const phase = {
    value: row === "boot" ? (side === "candidate" ? 90 : 100) : 100,
    counters: {
      staticT0FastInstructions: "0",
      staticT0SlowInstructions: "0",
      staticT0Errors: "0",
      staticT0SampledInstructions: "0",
      staticT0Samples: "0",
      staticT0ShortMarks: "0",
      staticT0ShortBypasses: "0",
    },
  };
  return {
    schema: 2,
    measurementEligible: side === "candidate",
    side: "rewrite",
    row,
    phases: row === "boot"
      ? { first: phase }
      : { first: structuredClone(phase), prime: structuredClone(phase), steady: phase },
    runtime: {
      identity: {
        loaderSha256: R080_LOADER_SHA256,
        wasmSha256: side === "control"
          ? R080_CONTROL_WASM_SHA256
          : R080_CANDIDATE_WASM_SHA256,
      },
      guest: { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" },
      jitProof: {
        generatedInstructions: "1000",
        dispatches: "10",
      },
    },
  };
}

function sideAggregate(values) {
  return {
    values,
    median: values[2],
    spread: Math.max(...values) / Math.min(...values),
    fingerprints: ["correct"],
    inputs: ["same"],
  };
}

function synthetic() {
  const trials = [];
  for (const row of R080_ROWS) {
    for (let rep = 1; rep <= R080_REPS; rep++) {
      for (const side of ["control", "candidate"]) {
        trials.push({ row, rep, side, result: workerResult(row, side) });
      }
    }
  }
  const aggregates = {};
  for (const row of R080_ROWS) {
    const control = [100, 101, 102, 103, 104];
    const candidate = row === "boot"
      ? [90, 91, 92, 93, 94]
      : [100, 101, 102, 103, 104];
    const paired = control.map((value, index) => value / candidate[index]);
    aggregates[row] = {
      phase: row === "boot" ? "first" : "steady",
      control: sideAggregate(control),
      candidate: sideAggregate(candidate),
      candidateSpeedup: 102 / candidate[2],
      pairedCandidateSpeedup: {
        min: Math.min(...paired),
        median: paired[2],
        medianConfidence95: [Math.min(...paired), Math.max(...paired)],
        max: Math.max(...paired),
        raw: paired,
      },
    };
  }
  return {
    schema: 1,
    authoritative: false,
    measurementValid: true,
    purpose: "rewrite runtime-configuration or artifact A/B diagnostic",
    hostCpuAffinity: "8-15",
    hostProbeSpread: 1.02,
    configuration: {
      rows: [...R080_ROWS],
      reps: R080_REPS,
      controlConfig: {
        SCORECARD_V2_REWRITE_WASM: "/frozen/r054-promoted.wasm",
      },
      candidateConfig: {},
      wasmBySide: {
        control: { path: "/control.wasm", sha256: R080_CONTROL_WASM_SHA256 },
        candidate: { path: "/candidate.wasm", sha256: R080_CANDIDATE_WASM_SHA256 },
      },
      maximumSampleSpread: 1.25,
      hostProbe: { maximumSpread: 1.25 },
    },
    aggregates,
    trials,
    problems: [],
  };
}

const accepted = evaluateR080SourceRestoration(synthetic(), "synthetic");
assert.equal(accepted.gatePassed, true, accepted.problems.join("; "));

function reject(name, mutate, needle) {
  const report = synthetic();
  mutate(report);
  const result = evaluateR080SourceRestoration(report, "synthetic");
  assert.equal(result.gatePassed, false, `${name} must fail`);
  assert(
    result.problems.some((problem) => problem.includes(needle)),
    `${name}: expected ${needle}; got ${result.problems.join("; ")}`,
  );
}

reject("Boot noninferiority", (report) => {
  report.aggregates.boot.pairedCandidateSpeedup.median = 0.96;
}, "boot: paired median");
reject("Boot confidence", (report) => {
  report.aggregates.boot.pairedCandidateSpeedup.medianConfidence95[0] = 0.94;
}, "boot: confidence lower");
reject("guard", (report) => {
  report.aggregates.compile.pairedCandidateSpeedup.median = 0.96;
}, "compile: paired median");
reject("identity", (report) => {
  report.trials[0].result.runtime.identity.loaderSha256 = "wrong";
}, "loader=wrong");
reject("static activity", (report) => {
  report.trials[0].result.phases.first.counters.staticT0Samples = "1";
}, "staticT0Samples=1");
reject("missing trial", (report) => {
  report.trials.pop();
}, "trials=29");
reject("affinity", (report) => {
  report.hostCpuAffinity = "0-63";
}, "CPU affinity=0-63");

console.log("r080 residual cleanup selftest: PASS");
