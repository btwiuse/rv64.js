#!/usr/bin/env node

// Run and validate the single frozen R124 count-only modern Compile worker.
// The emitted counters change generated Wasm and every elapsed value is
// explicitly excluded from performance use.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const root = resolve(dirname(scriptPath), "../..");
const outputDir = resolve(
  process.argv.find((value) => value.startsWith("--output-dir="))?.slice(13) ??
    "target/bench/r124-rvc-bank-hybrid/dynamic-census",
);
const artifacts = resolve(process.env.ARTIFACTS ?? join(root, "target/bench"));
const wasmPath = join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");

const EXPECTED = Object.freeze({
  wasmSha256: "ad3693a711902fc93dc7d87e0757a7e97cdf9aa3a926ad92fd11c65a6aea5e9d",
  dbtLibSha256: "818040b9dbf6ba863b51356a4a3e4088766c1b892d646db588c202cf8f3ff4f7",
  dbtWasmSha256: "02100aa53d1a9d5a17c8828fd42798ee5654739299334c00e2d46de2b21b5a6e",
  wasmSourceSha256: "68ff03f628ac724cc496f795acf6faa60181c595049259ee22ae1dec96bd80f9",
  workerSha256: "e2ac3f82c50339d0fdcd1a5e17432489ba4864cf85050b74d4ee0f8223c7565f",
  harnessLibSha256: "377f32f4a5fbd467f4b262ecb8472febf8e3960a12ff40496c91c59009c29186",
  protocolSha256: "b54cd742c9da22bfd07fad4a1be4dcce2251cd6d188ef6871cfcce5c98d18e15",
  staticAnalyzerSha256: "30226db59f6831d36e5817b1f6082ebaed9546655436a5cb67c3dccdc65b49c1",
  staticReportSha256: "8ed3a6a3dcf49ca64e064da375d530527c54f1d8828a3217e4ef6f8b59b1f6ee",
  baselineDbtLibSha256: "ba4972333a293e37d03b66f0932bc31a3453360077fcb9e5e2ba1766a4811360",
  baselineDbtWasmSha256: "b5e9c11ec1bfa1e92245e6bac003c4af0c6bac4b813d58344d8276940d6a1e99",
  baselineWasmSourceSha256: "1da35e70bc9c957fd184f1cd9e6772d6d0a7380398aa7fc17369581b53036339",
  baselineWorkerSha256: "346c240378c8763053b7cddd5a093476eb0661ccddb5b8ef47b5f1698c77a175",
  loaderSha256: "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385",
  kernelSha256: "57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2",
  initramfsSha256: "cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808",
  compileMd5: "24eedf7e06beffd4d3ba1945585588db",
});

const FROZEN_SOURCES = Object.freeze({
  "dbt-lib.instrumented.rs": "crates/rv64-dbt/src/lib.rs",
  "dbt-wasm.instrumented.rs": "crates/rv64-dbt/src/wasm.rs",
  "wasm-lib.instrumented.rs": "crates/rv64-wasm/src/lib.rs",
  "scorecard-v2-worker.instrumented.mjs": "tests/vs-v86/scorecard-v2-worker.mjs",
  "scorecard-v2-lib.mjs": "tests/vs-v86/scorecard-v2-lib.mjs",
  "protocol.md": "docs/jit-rewrite/R124_RVC_BANK_HYBRID_STATE_ATTRIBUTION_PROTOCOL.md",
  "static-census.mjs": "tests/vs-v86/r124-rvc-bank-static-census.mjs",
  "static-census-a.json": "target/bench/r124-rvc-bank-hybrid/static-census-a.json",
  "static-census-b.json": "target/bench/r124-rvc-bank-hybrid/static-census-b.json",
  "dbt-lib.baseline.rs":
    "target/bench/r124-rvc-bank-hybrid/instrumented-source/dbt-lib.baseline.rs",
  "dbt-wasm.baseline.rs":
    "target/bench/r124-rvc-bank-hybrid/instrumented-source/dbt-wasm.baseline.rs",
  "wasm-lib.baseline.rs":
    "target/bench/r124-rvc-bank-hybrid/instrumented-source/wasm-lib.baseline.rs",
  "scorecard-v2-worker.baseline.mjs":
    "target/bench/r124-rvc-bank-hybrid/instrumented-source/scorecard-v2-worker.baseline.mjs",
});

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function big(value, label) {
  assert(
    (typeof value === "number" && Number.isSafeInteger(value) && value >= 0) ||
      (typeof value === "string" && /^\d+$/.test(value)),
    `${label}: invalid nonnegative counter ${value}`,
  );
  return BigInt(value);
}

function ratio(numerator, denominator) {
  return denominator === 0n ? null : Number(numerator) / Number(denominator);
}

function analyzePhase(name, phase) {
  assert(phase.profile, `${name}: missing execution profile`);
  const mix = phase.profile.executionMix;
  assert(mix, `${name}: missing execution mix`);
  const read = (key) => big(mix[key], `${name}/${key}`);
  const currentEntryResident = read("r124CurrentEntryResident");
  const currentEntryMaterialized = read("r124CurrentEntryMaterialized");
  const currentExitResident = read("r124CurrentExitResident");
  const currentExitMaterialized = read("r124CurrentExitMaterialized");
  const hybridMemberReadMaterialized = read("r124HybridMemberReadMaterialized");
  const hybridMemberOutputMaterialized = read("r124HybridMemberOutputMaterialized");
  const hybridPreciseOutputMaterialized = read("r124HybridPreciseOutputMaterialized");
  const structuredMemberEntries = read("structuredMemberEntries");
  const structuredScheduledInstructions = read("structuredScheduledInstructions");

  const currentEntry = currentEntryResident + currentEntryMaterialized;
  const currentExit = currentExitResident + currentExitMaterialized;
  const currentBoundary = currentEntry + currentExit;
  const projectedHybrid =
    currentEntryResident +
    currentExitResident +
    hybridMemberReadMaterialized +
    hybridMemberOutputMaterialized +
    hybridPreciseOutputMaterialized;
  assert(currentEntry > 0n, `${name}: no current GPR entry operations`);
  assert(currentExit > 0n, `${name}: no current GPR exit operations`);
  assert(structuredMemberEntries > 0n, `${name}: no structured member execution`);
  assert(structuredScheduledInstructions > 0n, `${name}: no structured scheduled work`);
  assert(
    currentEntryResident + currentEntryMaterialized === currentEntry,
    `${name}: entry classification closure`,
  );
  assert(
    currentExitResident + currentExitMaterialized === currentExit,
    `${name}: exit classification closure`,
  );

  const projectedRatio = ratio(projectedHybrid, currentBoundary);
  const originalTarget = name === "first" ? 0.85 : 0.80;
  return {
    current: {
      entryResident: currentEntryResident.toString(),
      entryMaterialized: currentEntryMaterialized.toString(),
      entry: currentEntry.toString(),
      exitResident: currentExitResident.toString(),
      exitMaterialized: currentExitMaterialized.toString(),
      exit: currentExit.toString(),
      boundaryOperations: currentBoundary.toString(),
    },
    projectedHybrid: {
      residentEntryAndExit: (currentEntryResident + currentExitResident).toString(),
      materializedMemberReads: hybridMemberReadMaterialized.toString(),
      materializedNormalOutputs: hybridMemberOutputMaterialized.toString(),
      materializedPreciseOutputs: hybridPreciseOutputMaterialized.toString(),
      operations: projectedHybrid.toString(),
      ratioToCurrent: projectedRatio,
      reductionFraction: 1 - projectedRatio,
    },
    execution: {
      structuredMemberEntries: structuredMemberEntries.toString(),
      structuredScheduledInstructions: structuredScheduledInstructions.toString(),
      projectedOperationsPerMember: ratio(projectedHybrid, structuredMemberEntries),
      currentBoundaryOperationsPerMember: ratio(currentBoundary, structuredMemberEntries),
    },
    originalProxyTarget: {
      maximumRatio: originalTarget,
      met: projectedRatio <= originalTarget,
      decisionUse: "reported diagnostic only after owner-directed R104 amendment",
    },
  };
}

function selftest() {
  const result = analyzePhase("steady", {
    profile: {
      executionMix: {
        r124CurrentEntryResident: 10,
        r124CurrentEntryMaterialized: 30,
        r124CurrentExitResident: 5,
        r124CurrentExitMaterialized: 15,
        r124HybridMemberReadMaterialized: 20,
        r124HybridMemberOutputMaterialized: 10,
        r124HybridPreciseOutputMaterialized: 1,
        structuredMemberEntries: 10,
        structuredScheduledInstructions: 100,
      },
    },
  });
  assert.equal(result.current.boundaryOperations, "60");
  assert.equal(result.projectedHybrid.operations, "46");
  assert.equal(result.projectedHybrid.ratioToCurrent, 46 / 60);
  assert.equal(result.originalProxyTarget.met, true);
  assert.throws(
    () => analyzePhase("first", { profile: { executionMix: {} } }),
    /invalid nonnegative counter/,
  );
  process.stdout.write("R124_DYNAMIC_CENSUS_SELFTEST_PASS\n");
}

if (process.argv.includes("--selftest")) {
  selftest();
  process.exit(0);
}

const sourceChecks = {
  dbtLibSha256: sha256(join(root, FROZEN_SOURCES["dbt-lib.instrumented.rs"])),
  dbtWasmSha256: sha256(join(root, FROZEN_SOURCES["dbt-wasm.instrumented.rs"])),
  wasmSourceSha256: sha256(join(root, FROZEN_SOURCES["wasm-lib.instrumented.rs"])),
  workerSha256: sha256(join(root, FROZEN_SOURCES["scorecard-v2-worker.instrumented.mjs"])),
  harnessLibSha256: sha256(join(root, FROZEN_SOURCES["scorecard-v2-lib.mjs"])),
  protocolSha256: sha256(join(root, FROZEN_SOURCES["protocol.md"])),
  staticAnalyzerSha256: sha256(join(root, FROZEN_SOURCES["static-census.mjs"])),
  staticReportASha256: sha256(join(root, FROZEN_SOURCES["static-census-a.json"])),
  staticReportBSha256: sha256(join(root, FROZEN_SOURCES["static-census-b.json"])),
  baselineDbtLibSha256: sha256(join(root, FROZEN_SOURCES["dbt-lib.baseline.rs"])),
  baselineDbtWasmSha256: sha256(join(root, FROZEN_SOURCES["dbt-wasm.baseline.rs"])),
  baselineWasmSourceSha256: sha256(join(root, FROZEN_SOURCES["wasm-lib.baseline.rs"])),
  baselineWorkerSha256: sha256(
    join(root, FROZEN_SOURCES["scorecard-v2-worker.baseline.mjs"]),
  ),
};
for (const [name, value] of Object.entries(sourceChecks)) {
  const expectedName = name.startsWith("staticReport") ? "staticReportSha256" : name;
  assert.equal(value, EXPECTED[expectedName], `${name} mismatch`);
}
assert.equal(sha256(wasmPath), EXPECTED.wasmSha256, "instrumented Wasm mismatch");

mkdirSync(outputDir, { recursive: true });
const command = [
  "taskset",
  "-c",
  "8-15",
  process.execPath,
  "tests/vs-v86/scorecard-v2-worker.mjs",
  "rewrite",
  "compile",
];
const environment = {
  ARTIFACTS: artifacts,
  SCORECARD_V2_TIMEOUT_MS: process.env.SCORECARD_V2_TIMEOUT_MS ?? "900000",
  SCORECARD_V2_PROFILE: "1",
  SCORECARD_V2_PROFILE_SHIFT: "8",
};
const child = spawnSync(command[0], command.slice(1), {
  cwd: root,
  env: { ...process.env, ...environment },
  encoding: "utf8",
  maxBuffer: 64 << 20,
});
if (child.status !== 0) {
  throw new Error(
    `R124 diagnostic worker failed (${child.status})\n${child.stderr}\n${child.stdout}`,
  );
}
const resultLine = child.stdout.split("\n").find((line) => line.startsWith("RESULT_JSON "));
if (!resultLine) throw new Error("R124 diagnostic worker emitted no RESULT_JSON");
const result = JSON.parse(resultLine.slice("RESULT_JSON ".length));

assert.equal(result.side, "rewrite");
assert.equal(result.row, "compile");
assert.equal(result.measurementEligible, false, "profiled worker must be ineligible");
assert.deepEqual(result.runtime.diagnostic, {
  executionProfile: true,
  profileSampleShift: 8,
});
assert.equal(result.runtime.identity.wasmSha256, EXPECTED.wasmSha256);
assert.equal(result.runtime.identity.loaderSha256, EXPECTED.loaderSha256);
assert.equal(result.runtime.guest.linux, "6.12.7");
assert.equal(result.runtime.guest.alpine, "3.24.1");
assert.equal(result.runtime.guest.arch, "riscv64");
assert.equal(result.runtime.schedulerCadence.name, "public-one-slice-per-turn");
assert.equal(result.runtime.requestedPolicy.name, "production-page");
assert.equal(result.runtime.policyProblems.length, 0, "production policy mismatch");
assert.equal(result.runtime.jitProof.requirement, "generated-code-executed");
assert(big(result.runtime.jitProof.generatedInstructions, "total generated instructions") > 0n);
assert.equal(result.inputSha256.kernel, EXPECTED.kernelSha256);
assert.equal(result.inputSha256.initramfs, EXPECTED.initramfsSha256);
assert(result.settle.every((entry) => entry.complete), "JIT did not settle");

for (const name of ["first", "prime", "steady"]) {
  assert.equal(result.phases[name].md5, EXPECTED.compileMd5, `${name}: compile MD5`);
  assert(
    big(result.phases[name].counters.generatedInstructions, `${name}: generated`) > 0n,
    `${name}: no generated execution`,
  );
}
const phases = Object.fromEntries(
  ["first", "prime", "steady"].map((name) => [name, analyzePhase(name, result.phases[name])]),
);

const report = {
  schema: 1,
  experiment: "R124 RV64C-bank hybrid structured-state dynamic operation census",
  performanceEvidence: false,
  elapsedValuesExcluded: true,
  proxyTargetsAreDiagnostic: true,
  command,
  environment,
  identities: {
    ...sourceChecks,
    runnerSha256: sha256(scriptPath),
    wasmSha256: EXPECTED.wasmSha256,
    loaderSha256: EXPECTED.loaderSha256,
    kernelSha256: EXPECTED.kernelSha256,
    initramfsSha256: EXPECTED.initramfsSha256,
  },
  workload: {
    measurementEligible: result.measurementEligible,
    guest: result.runtime.guest,
    cadence: result.runtime.schedulerCadence,
    requestedPolicy: result.runtime.requestedPolicy,
    effectivePolicy: result.runtime.effectivePolicy,
    md5: Object.fromEntries(
      Object.entries(result.phases).map(([name, phase]) => [name, phase.md5]),
    ),
    generatedInstructions: result.runtime.jitProof.generatedInstructions,
    dispatches: result.runtime.jitProof.dispatches,
    settleComplete: result.settle.every((entry) => entry.complete),
  },
  phases,
  rawResult: result,
};

const sourceDir = join(outputDir, "instrumented-source");
mkdirSync(sourceDir);
for (const [name, path] of Object.entries(FROZEN_SOURCES)) {
  copyFileSync(join(root, path), join(sourceDir, name));
}
copyFileSync(scriptPath, join(sourceDir, "dynamic-census.mjs"));
copyFileSync(wasmPath, join(outputDir, "rv64_wasm.instrumented.wasm"));
writeFileSync(join(outputDir, "worker.stdout.log"), child.stdout, { flag: "wx" });
writeFileSync(join(outputDir, "worker.stderr.log"), child.stderr, { flag: "wx" });
writeFileSync(join(outputDir, "dynamic-census.json"), `${JSON.stringify(report, null, 2)}\n`, {
  flag: "wx",
});
process.stdout.write(`R124_DYNAMIC_CENSUS ${JSON.stringify({ outputDir, phases })}\n`);
