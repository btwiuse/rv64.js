#!/usr/bin/env node

import assert from "node:assert/strict";
import {
  evaluateR077ScorecardPromotion,
  R077_BASELINE_SHA256,
  R077_REWRITE_LOADER_SHA256,
  R077_REWRITE_WASM_SHA256,
  R077_ROWS,
  R077_SIDES,
} from "./r077-scorecard-promotion-lib.mjs";

function configuration() {
  return {
    sides: [...R077_SIDES],
    rows: [...R077_ROWS],
    reps: 3,
    phases: ["first", "prime", "steady"],
    scoredPhase: "steady (first for boot)",
    v86ExecutionPreflight: true,
    timeoutMs: 900000,
    rewritePolicy: "production",
  };
}

function result(row, side) {
  const runtime = {
    identity: side === "rewrite"
      ? {
          loaderSha256: R077_REWRITE_LOADER_SHA256,
          wasmSha256: R077_REWRITE_WASM_SHA256,
        }
      : { wasmSha256: `${side}-synthetic` },
    jitProof: {
      enabledRequested: true,
      generatedInstructions: "123456",
    },
  };
  if (side === "rewrite") {
    runtime.staticSystemT0 = {
      production: true,
      enabled: false,
      sampled: true,
      sampledBackoff: true,
      registeredModules: 1,
      index: 7,
      modulesBefore: 2,
      modulesAfter: 3,
    };
    runtime.staticT0Proof = {
      sampledInstructions: "1000",
      samples: "100",
      shortMarks: "10",
      shortBypasses: "20",
      errors: "0",
    };
  }
  return {
    schema: 2,
    measurementEligible: true,
    side,
    row,
    runtime,
  };
}

function report({ baseline = false } = {}) {
  const aggregates = R077_ROWS.map((key) => {
    const rewrite = baseline ? 100 : key === "boot" ? 94 : 100;
    const legacy = rewrite * 1.10;
    const v86 = rewrite;
    return {
      key,
      kind: "duration",
      sides: {
        rewrite: { median: rewrite },
        legacy: { median: legacy },
        v86: { median: v86 },
      },
      rewriteVsLegacy: legacy / rewrite,
      rewriteVsV86: v86 / rewrite,
    };
  });
  const trials = [];
  for (const row of R077_ROWS) {
    for (const side of R077_SIDES) {
      for (let rep = 1; rep <= 3; rep++) {
        trials.push({ row, side, rep, result: result(row, side) });
      }
    }
  }
  return {
    schema: 2,
    created: "2026-08-09T00:00:00.000Z",
    authoritative: true,
    measurementValid: true,
    goalMet: false,
    configuration: configuration(),
    aggregates,
    trials,
    v86ExecutionPreflight: {
      result: {
        side: "v86",
        measurementEligible: false,
        runtime: {
          jitProof: {
            enabledRequested: true,
            finalizedModules: 7,
            executionProbe: {
              active: true,
              hits: 7,
              distinctHitIndexes: 7,
            },
          },
        },
      },
    },
    problems: [],
  };
}

function evaluate(candidate, baseline = report({ baseline: true }), baselineSha256 = R077_BASELINE_SHA256) {
  return evaluateR077ScorecardPromotion({ baseline, candidate, baselineSha256 });
}

function reject(name, mutate, needle) {
  const candidate = report();
  mutate(candidate);
  const evaluated = evaluate(candidate);
  assert.equal(evaluated.gatePassed, false, `${name} must fail`);
  assert(
    evaluated.problems.some((problem) => problem.includes(needle)),
    `${name}: expected problem containing ${JSON.stringify(needle)}; got ${evaluated.problems.join("; ")}`,
  );
}

const accepted = evaluate(report());
assert.equal(accepted.gatePassed, true, accepted.problems.join("; "));
assert.equal(accepted.counts.v86Matches, 13);
assert.equal(accepted.counts.legacyMatches, 13);
assert(accepted.boot.speedup > 1.05);

reject("score drop", (candidate) => {
  const row = candidate.aggregates.find((entry) => entry.key === "alu");
  row.sides.v86.median = 90;
  row.rewriteVsV86 = 0.90;
  // Two additional losses cross the frozen 11/13 floor.
  for (const key of ["mixed", "python"]) {
    const other = candidate.aggregates.find((entry) => entry.key === key);
    other.sides.v86.median = 90;
    other.rewriteVsV86 = 0.90;
  }
}, "copy/v86 matches=10/13");

reject("row regression", (candidate) => {
  const row = candidate.aggregates.find((entry) => entry.key === "alu");
  row.sides.rewrite.median = 106;
  row.sides.legacy.median = 116.6;
  row.sides.v86.median = 106;
  row.rewriteVsLegacy = 1.1;
  row.rewriteVsV86 = 1;
}, "exceeds R054 +5% limit");

reject("Boot below 5%", (candidate) => {
  const row = candidate.aggregates.find((entry) => entry.key === "boot");
  row.sides.rewrite.median = 96;
  row.sides.legacy.median = 105.6;
  row.sides.v86.median = 96;
  row.rewriteVsLegacy = 1.1;
  row.rewriteVsV86 = 1;
}, "Boot speedup=");

reject("lifecycle missing", (candidate) => {
  delete candidate.trials.find((trial) => trial.side === "rewrite").result.runtime.staticSystemT0;
}, "staticSystemT0.production");

reject("static errors", (candidate) => {
  candidate.trials.find((trial) => trial.side === "rewrite")
    .result.runtime.staticT0Proof.errors = "1";
}, "staticT0Proof.errors=1");

reject("identity mismatch", (candidate) => {
  candidate.trials.find((trial) => trial.side === "rewrite")
    .result.runtime.identity.loaderSha256 = "wrong";
}, "loader identity changed");

reject("v86 proof missing", (candidate) => {
  candidate.v86ExecutionPreflight.result.runtime.jitProof.executionProbe.hits = 0;
}, "v86 execution proof missing");

const wrongBaselineHash = evaluate(report(), report({ baseline: true }), "wrong");
assert.equal(wrongBaselineHash.gatePassed, false);
assert(wrongBaselineHash.problems.some((problem) => problem.includes("baseline SHA-256")));

console.log("r077 scorecard promotion selftest: PASS");
