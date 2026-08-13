#!/usr/bin/env node

// Run the single frozen R121 diagnostic Compile worker and validate exact
// dispatch-counter closure. Instrumented elapsed values are retained only for
// auditability and are explicitly ineligible as performance evidence.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const outputDir = resolve(
  process.argv.find((value) => value.startsWith("--output-dir="))?.slice(13) ??
    "target/bench/r121-runtime-dispatch/counter-census",
);
const artifacts = resolve(
  process.env.ARTIFACTS ?? join(root, "target/bench"),
);
const wasmPath = join(
  root,
  "target/wasm32-unknown-unknown/release/rv64_wasm.wasm",
);
const EXPECTED = Object.freeze({
  wasmSha256: "a6685df5fe127678a2c4d11becf81140a5a784fd6ee71e42bcecaeadf992698e",
  loaderSha256: "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385",
  kernelSha256: "57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2",
  initramfsSha256: "cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808",
  compileMd5: "24eedf7e06beffd4d3ba1945585588db",
});

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function big(value) {
  return BigInt(value);
}

function phaseClosure(name, phase) {
  const c = phase.counters.r121DispatchDiagnostic;
  if (!c) throw new Error(`${name}: missing R121 dispatch counters`);
  const visits = big(c.visits);
  const directHits = big(c.directHits);
  const emptyMisses = big(c.emptyMisses);
  const collisionMisses = big(c.collisionMisses);
  const unverifiedMisses = big(c.unverifiedMisses);
  const staleGenerationMisses = big(c.staleGenerationMisses);
  const missReasons =
    emptyMisses + collisionMisses + unverifiedMisses + staleGenerationMisses;
  const cacheBlockHits = big(c.cacheBlockHits);
  const cacheBlacklistHits = big(c.cacheBlacklistHits);
  const cacheAbsent = big(c.cacheAbsent);
  const selfProofOk = big(c.selfProofOk);
  const selfProofFail = big(c.selfProofFail);
  const regionProofVisits = big(c.regionProofVisits);
  const regionProofFail = big(c.regionProofFail);
  const refills = big(c.refills);
  const mappingDrops = big(c.mappingDrops);

  assert.equal(visits, directHits + missReasons, `${name}: visit closure`);
  assert.equal(
    missReasons,
    cacheBlockHits + cacheBlacklistHits + cacheAbsent,
    `${name}: fallback lookup closure`,
  );
  assert.equal(cacheBlockHits, selfProofOk + selfProofFail, `${name}: self proof closure`);
  assert.equal(cacheBlockHits, refills + mappingDrops, `${name}: block outcome closure`);
  assert(regionProofFail <= regionProofVisits, `${name}: region proof bounds`);
  assert(mappingDrops >= selfProofFail, `${name}: every self failure drops`);
  assert(mappingDrops >= regionProofFail, `${name}: every region failure drops`);
  assert(
    mappingDrops <= selfProofFail + regionProofFail,
    `${name}: unexplained mapping drop`,
  );

  const ratio = (value, total) => total === 0n ? 0 : Number(value) / Number(total);
  return {
    visits: visits.toString(),
    directHits: directHits.toString(),
    fallbacks: missReasons.toString(),
    fallbackFraction: ratio(missReasons, visits),
    missReasons: {
      empty: emptyMisses.toString(),
      collision: collisionMisses.toString(),
      unverified: unverifiedMisses.toString(),
      staleGeneration: staleGenerationMisses.toString(),
      emptyFractionOfFallback: ratio(emptyMisses, missReasons),
      collisionFractionOfFallback: ratio(collisionMisses, missReasons),
      unverifiedFractionOfFallback: ratio(unverifiedMisses, missReasons),
      staleGenerationFractionOfFallback: ratio(staleGenerationMisses, missReasons),
    },
    authoritativeLookup: {
      compiledBlock: cacheBlockHits.toString(),
      blacklist: cacheBlacklistHits.toString(),
      absent: cacheAbsent.toString(),
      compiledBlockFractionOfFallback: ratio(cacheBlockHits, missReasons),
    },
    mappingProof: {
      selfOk: selfProofOk.toString(),
      selfFail: selfProofFail.toString(),
      regionVisits: regionProofVisits.toString(),
      regionFail: regionProofFail.toString(),
      refills: refills.toString(),
      drops: mappingDrops.toString(),
    },
    mmu: phase.counters.mmu,
  };
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
const child = spawnSync(command[0], command.slice(1), {
  cwd: root,
  env: {
    ...process.env,
    ARTIFACTS: artifacts,
    SCORECARD_V2_TIMEOUT_MS: process.env.SCORECARD_V2_TIMEOUT_MS ?? "900000",
  },
  encoding: "utf8",
  maxBuffer: 64 << 20,
});
if (child.status !== 0) {
  throw new Error(
    `diagnostic worker failed (${child.status})\n${child.stderr}\n${child.stdout}`,
  );
}
const resultLine = child.stdout.split("\n").find((line) => line.startsWith("RESULT_JSON "));
if (!resultLine) throw new Error("diagnostic worker emitted no RESULT_JSON");
const result = JSON.parse(resultLine.slice("RESULT_JSON ".length));

assert.equal(result.side, "rewrite");
assert.equal(result.row, "compile");
assert.equal(result.measurementEligible, true, "underlying workload is ineligible");
assert.equal(result.runtime.identity.wasmSha256, EXPECTED.wasmSha256);
assert.equal(result.runtime.identity.loaderSha256, EXPECTED.loaderSha256);
assert.equal(result.runtime.guest.linux, "6.12.7");
assert.equal(result.runtime.guest.alpine, "3.24.1");
assert.equal(result.runtime.guest.arch, "riscv64");
assert.equal(result.runtime.schedulerCadence.name, "public-one-slice-per-turn");
assert.equal(result.inputSha256.kernel, EXPECTED.kernelSha256);
assert.equal(result.inputSha256.initramfs, EXPECTED.initramfsSha256);
for (const name of ["first", "prime", "steady"]) {
  assert.equal(result.phases[name].md5, EXPECTED.compileMd5, `${name}: compile MD5`);
  assert(big(result.phases[name].counters.generatedInstructions) > 0n, `${name}: no JIT`);
}
assert(result.settle.every((row) => row.complete), "compilation did not settle");

const phases = Object.fromEntries(
  ["first", "prime", "steady"].map((name) => [
    name,
    phaseClosure(name, result.phases[name]),
  ]),
);
const report = {
  schema: 1,
  experiment: "R121 exact dispatch miss-cause census",
  performanceEvidence: false,
  elapsedValuesExcluded: true,
  command,
  environment: {
    ARTIFACTS: artifacts,
    SCORECARD_V2_TIMEOUT_MS: process.env.SCORECARD_V2_TIMEOUT_MS ?? "900000",
  },
  identities: {
    wasm: EXPECTED.wasmSha256,
    loader: EXPECTED.loaderSha256,
    kernel: EXPECTED.kernelSha256,
    initramfs: EXPECTED.initramfsSha256,
  },
  workload: {
    measurementEligible: result.measurementEligible,
    guest: result.runtime.guest,
    cadence: result.runtime.schedulerCadence,
    md5: Object.fromEntries(
      Object.entries(result.phases).map(([name, phase]) => [name, phase.md5]),
    ),
    generatedInstructions: result.runtime.jitProof.generatedInstructions,
    dispatches: result.runtime.jitProof.dispatches,
    settleComplete: result.settle.every((row) => row.complete),
  },
  phases,
  rawResult: result,
};

writeFileSync(join(outputDir, "worker.stdout.log"), child.stdout, { flag: "wx" });
writeFileSync(join(outputDir, "worker.stderr.log"), child.stderr, { flag: "wx" });
writeFileSync(join(outputDir, "counter-census.json"), `${JSON.stringify(report, null, 2)}\n`, {
  flag: "wx",
});
process.stdout.write(`R121_COUNTER_CENSUS ${JSON.stringify({ outputDir, phases })}\n`);
