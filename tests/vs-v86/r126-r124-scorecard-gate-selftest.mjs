#!/usr/bin/env node

import assert from "node:assert/strict";
import {
  CANDIDATE_WASM,
  EXPECTED_ROWS,
  EXPECTED_SIDES,
  LEGACY_LOADER,
  LEGACY_WASM,
  LOADER,
  V86_COMMIT,
  V86_WASM,
  evaluateR126,
} from "./r126-r124-scorecard-gate.mjs";

const inputs = (side, row) => ({
  kernel: side === "v86" ? "x86-kernel" : "rv-kernel",
  initramfs: side === "v86" ? "x86-initrd" : "rv-initrd",
  ...(row === "boot" ? {} : { benchmark: `bench-${row}` }),
});

function identity(side) {
  if (side === "rewrite") return { loaderSha256: LOADER, wasmSha256: CANDIDATE_WASM };
  if (side === "legacy") return { loaderSha256: LEGACY_LOADER, wasmSha256: LEGACY_WASM };
  return { sourceCommit: V86_COMMIT, wasmSha256: V86_WASM };
}

function guest(side) {
  return { linux: "6.12.7", alpine: "3.24.1", arch: side === "v86" ? "i686" : "riscv64" };
}

function report({ candidate = false } = {}) {
  const ratios = Object.fromEntries(EXPECTED_ROWS.map((row) => [row,
    row === "boot" ? 0.72 : row === "compile" ? 0.76 : 1.2]));
  if (candidate) {
    ratios.boot *= 1.02;
    ratios.compile *= 1.08;
  }
  const aggregates = EXPECTED_ROWS.map((key) => {
    const rewrite = 1000 / ratios[key];
    return {
      key,
      kind: "duration",
      sides: {
        rewrite: { median: rewrite, samples: [rewrite, rewrite, rewrite], spread: 1 },
        legacy: { median: rewrite * 2, samples: [rewrite * 2, rewrite * 2, rewrite * 2], spread: 1 },
        v86: { median: 1000, samples: [1000, 1000, 1000], spread: 1 },
      },
      rewriteVsLegacy: 2,
      rewriteVsV86: ratios[key],
    };
  });
  const trials = [];
  for (const row of EXPECTED_ROWS) {
    for (let rep = 1; rep <= 3; rep++) {
      for (const side of EXPECTED_SIDES) {
        trials.push({ side, row, rep, result: {
          measurementEligible: true,
          side,
          row,
          inputSha256: inputs(side, row),
          runtime: {
            identity: candidate ? identity(side) : {
              ...identity(side),
              ...(side === "rewrite" ? { wasmSha256: "accepted" } : {}),
            },
            guest: guest(side),
            schedulerCadence: {
              name: "public-one-slice-per-turn",
              rv64SlicesPerEventLoopTurn: 1,
            },
            diagnostic: null,
            policyProblems: [],
            jitProof: { enabledRequested: true, generatedInstructions: "1", dispatches: "1" },
          },
        }});
      }
    }
  }
  return {
    authoritative: true,
    measurementValid: true,
    problems: [],
    configuration: {
      reps: 3,
      rows: EXPECTED_ROWS,
      sides: EXPECTED_SIDES,
      rewritePolicy: "production",
      v86ExecutionPreflight: true,
      schedulerCadence: { name: "public-one-slice-per-turn", rv64SlicesPerEventLoopTurn: 1 },
    },
    provenance: { cpuAffinity: "8-15" },
    aggregates,
    trials,
  };
}

function wanix() {
  return {
    measurementValid: true,
    artifacts: { archives: { candidate: { wasmSha256: CANDIDATE_WASM } } },
    rows: Object.fromEntries(["shell", "python", "sha256", "shared9p"].map((row) =>
      [row, { pairedMedianSpeedup: row === "shell" ? 0.996 : 1.02 }])),
  };
}

const baseline = report();
const passing = report({ candidate: true });
assert.equal(evaluateR126(passing, baseline, wanix()).accepted, true);

const newLoss = structuredClone(passing);
newLoss.aggregates.find((row) => row.key === "alu").rewriteVsV86 = 0.94;
assert.match(evaluateR126(newLoss, baseline, wanix()).problems.join(";"), /new v86 parity losses/);

const legacyLoss = structuredClone(passing);
legacyLoss.aggregates.find((row) => row.key === "alu").rewriteVsLegacy = 0.94;
assert.match(evaluateR126(legacyLoss, baseline, wanix()).problems.join(";"), /legacy parity losses/);

const noTarget = report({ candidate: true });
for (const key of ["boot", "compile"]) {
  const before = baseline.aggregates.find((row) => row.key === key).rewriteVsV86;
  noTarget.aggregates.find((row) => row.key === key).rewriteVsV86 = before * 1.005;
}
assert.match(evaluateR126(noTarget, baseline, wanix()).problems.join(";"), /neither Boot nor Compile/);

const protectedRegression = structuredClone(passing);
protectedRegression.aggregates.find((row) => row.key === "alu").rewriteVsV86 =
  baseline.aggregates.find((row) => row.key === "alu").rewriteVsV86 * 0.989;
assert.match(evaluateR126(protectedRegression, baseline, wanix()).problems.join(";"),
  /normalized protected speedup/);

const badWanix = wanix();
badWanix.rows.python.pairedMedianSpeedup = 0.999;
assert.match(evaluateR126(passing, baseline, badWanix).problems.join(";"),
  /shared\/bench\.py regresses/);

const badInput = structuredClone(passing);
badInput.trials[0].result.inputSha256.kernel = "wrong";
assert.match(evaluateR126(badInput, baseline, wanix()).problems.join(";"), /input identity differs/);

const badCandidate = structuredClone(passing);
badCandidate.trials.find((trial) => trial.side === "rewrite").result.runtime.identity.wasmSha256 = "wrong";
assert.match(evaluateR126(badCandidate, baseline, wanix()).problems.join(";"), /candidate Wasm differs/);

console.log("R126 scorecard gate selftest: pass, parity, target, materiality, WANIX, input, and identity guards enforced");
