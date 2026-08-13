#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  GUEST_CONTRACT,
  INTERPRETER_SIDES,
  LEGACY_MODERN_COMPARATOR,
  LEGACY_RELEASE,
  NBENCH_WORKLOAD_CONTRACT,
  ROWS,
  balancedOrder,
  configureRvPolicy,
  embeddedBenchmarkSha256,
  embeddedWorkloadSha256,
  median,
  parseGuestIdentity,
  parseExecutionMode,
  parseNbench,
  phasesFor,
  sampleSpread,
  sha256,
  speedRatio,
  validateGuestIdentity,
} from "./scorecard-v2-lib.mjs";
import {
  cadenceDiagnostic,
  cadenceRecord,
  parsePumpCadence,
  shouldYieldAfterPump,
} from "./scorecard-v2-cadence.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

assert.equal(median([9, 1, 5]), 5);
assert.equal(median([4, 2]), 3);
assert.equal(sampleSpread([10, 12.5]), 1.25);
assert.equal(speedRatio("duration", 2, 4), 2);
assert.equal(speedRatio("throughput", 4, 2), 2);
assert.deepEqual(phasesFor({ family: "boot" }), ["first"]);
assert.deepEqual(phasesFor({ family: "nbench" }), ["first", "prime", "steady"]);
assert.deepEqual(phasesFor({ family: "compute" }), ["first", "prime", "steady"]);
assert.equal(parseExecutionMode(), "jit");
assert.equal(parseExecutionMode("interpreter"), "interpreter");
assert.deepEqual(INTERPRETER_SIDES, ["rewrite", "v86"]);
assert.throws(() => parseExecutionMode("disabled"), /SCORECARD_V2_EXECUTION_MODE/);

const publicCadence = parsePumpCadence({});
assert.deepEqual(cadenceRecord(publicCadence), {
  name: "public-one-slice-per-turn",
  rv64SlicesPerEventLoopTurn: 1,
  reference: "public-rv64-and-v86-event-driven",
});
assert.equal(publicCadence.diagnostic, false);
assert.equal(cadenceDiagnostic(publicCadence), null);
assert.deepEqual(
  Array.from({ length: 9 }, (_, iteration) =>
    shouldYieldAfterPump(iteration, publicCadence)),
  Array(9).fill(true),
);

const explicitCadence = parsePumpCadence({ SCORECARD_V2_YIELD_EVERY_PUMP: "1" });
assert.equal(explicitCadence.everyPump, true);
assert.equal(explicitCadence.diagnostic, true);
assert.deepEqual(cadenceDiagnostic(explicitCadence), { yieldEveryPump: true });

const historicalCadence = parsePumpCadence({
  SCORECARD_V2_HISTORICAL_BATCHED_PUMPS: "1",
});
assert.deepEqual(cadenceRecord(historicalCadence), {
  name: "historical-four-slices-per-turn",
  rv64SlicesPerEventLoopTurn: 4,
  reference: "public-rv64-and-v86-event-driven",
});
assert.equal(historicalCadence.diagnostic, true);
assert.deepEqual(
  Array.from({ length: 9 }, (_, iteration) =>
    shouldYieldAfterPump(iteration, historicalCadence)),
  [true, false, false, false, true, false, false, false, true],
);
assert.throws(
  () => parsePumpCadence({ SCORECARD_V2_YIELD_EVERY_PUMP: "0" }),
  /must be 1/,
);
assert.throws(
  () => parsePumpCadence({
    SCORECARD_V2_YIELD_EVERY_PUMP: "1",
    SCORECARD_V2_HISTORICAL_BATCHED_PUMPS: "1",
  }),
  /conflict/,
);
assert.throws(() => shouldYieldAfterPump(-1, publicCadence), /non-negative/);
for (const policy of ["production", "compat"]) {
  const calls = [];
  const vm = {
    tailCallsSupported: true,
    ex: {
      jit_set_enabled: (value) => calls.push(["enabled", value]),
      jit_set_page_policy: (value) => calls.push(["page", value]),
      jit_set_region_tail_chain: (value) => calls.push(["tail", value]),
      sys_set_superblock: (value) => calls.push(["superblock", value]),
      jit_set_multi_latch: (value) => calls.push(["multi", value]),
    },
  };
  const configured = configureRvPolicy(vm, "rewrite", policy);
  assert.deepEqual(calls.slice(0, 2), [["enabled", 1], ["page", policy === "production" ? 1 : 0]]);
  assert.equal(configured.name, policy === "production" ? "production-page" : "compat-superblock");
}

function newcEntry(name, data, ino = 1) {
  const nameBytes = Buffer.from(`${name}\0`);
  const fields = [ino, 0o100755, 0, 0, 1, 0, data.length, 0, 0, 0, 0, nameBytes.length, 0];
  const header = Buffer.from(`070701${fields.map((value) => value.toString(16).padStart(8, "0")).join("")}`);
  const namePadding = Buffer.alloc((4 - ((header.length + nameBytes.length) & 3)) & 3);
  const dataPadding = Buffer.alloc((4 - (data.length & 3)) & 3);
  return Buffer.concat([header, nameBytes, namePadding, data, dataPadding]);
}
const embeddedAlu = Buffer.from("executed guest bytes");
const archive = Buffer.concat([
  newcEntry("opt/scorecard/alu", embeddedAlu),
  newcEntry("TRAILER!!!", Buffer.alloc(0), 2),
]);
assert.equal(
  embeddedBenchmarkSha256({ family: "compute", key: "alu" }, archive),
  sha256(embeddedAlu),
);
const nbenchBytes = Buffer.from("matched nbench executable");
const workloadContract = Buffer.from('{"variant":"scorecard-fixed-work-data32-v3"}\n');
const dataTransform = Buffer.from("fixed-width transform\n");
const workTransform = Buffer.from("fixed-work transform\n");
const implementationSource = Buffer.from("rv64 fastmem implementation\n");
const nbenchArchive = Buffer.concat([
  newcEntry("opt/scorecard/nbench-fixed", nbenchBytes),
  newcEntry("opt/scorecard/nbench-workload-contract.json", workloadContract, 2),
  newcEntry("opt/scorecard/nbench-fixed-data32.patch", dataTransform, 3),
  newcEntry("opt/scorecard/nbench-fixed-work.patch", workTransform, 4),
  newcEntry("opt/scorecard/nbench-rv64-fastmem.c", implementationSource, 5),
  newcEntry("TRAILER!!!", Buffer.alloc(0), 6),
]);
assert.deepEqual(
  embeddedWorkloadSha256({ family: "nbench" }, nbenchArchive),
  {
    benchmark: sha256(nbenchBytes),
    workloadContract: sha256(workloadContract),
    workloadTransforms: sha256(Buffer.concat([dataTransform, workTransform])),
    implementationSources: sha256(implementationSource),
  },
);

assert.deepEqual(balancedOrder(0, ["rewrite", "legacy", "v86"]), ["rewrite", "legacy", "v86"]);
assert.deepEqual(balancedOrder(1, ["rewrite", "legacy", "v86"]), ["legacy", "v86", "rewrite"]);
assert.deepEqual(balancedOrder(2, ["rewrite", "legacy", "v86"]), ["v86", "rewrite", "legacy"]);

const nbench = parseNbench(`
NUMERIC SORT    : 123.5 : 1.0
STRING SORT     :
                : 4.25 : 1.0
Number of runs: 8
Absolute standard deviation: 0.125
`, "STRING SORT");
assert.equal(nbench.value, 4.25);
assert.equal(nbench.internal.sampleCount, 8);
assert.equal(nbench.internal.standardDeviation, 0.125);
assert.equal(nbench.internal.confidencePassed, true);

const identity = parseGuestIdentity(
  `SCORECARD_V2_GUEST linux=${GUEST_CONTRACT.linux} alpine=${GUEST_CONTRACT.alpine} arch=riscv64`,
);
assert.deepEqual(validateGuestIdentity(identity, "rewrite"), []);
assert.match(validateGuestIdentity(identity, "v86").join(" "), /i686/);

assert.equal(LEGACY_RELEASE.modernVirtJit, false);
assert.equal(LEGACY_MODERN_COMPARATOR.modernVirtJit, true);
assert.equal(LEGACY_MODERN_COMPARATOR.sourceCommit, "5b896f9");

const worker = await readFile(join(root, "tests/vs-v86/scorecard-v2-worker.mjs"), "utf8");
assert.equal(worker.includes("-c/tmp/C"), false, "nbench uppercases absolute command-file paths");
assert.match(worker, /shouldYieldAfterPump\(iteration, pumpCadence\)/);
assert.equal(
  worker.includes("yieldEveryPumpDiagnostic || (iteration & 3)"),
  false,
  "worker duplicates the historical cadence instead of using the shared helper",
);
for (const forbidden of [
  "bbl64.bin",
  "kernel-riscv64.bin",
  "root-riscv64.bin",
  "deb-riscv64.ext4",
  "root-nbench.bin",
]) {
  assert.equal(worker.includes(forbidden), false, `v2 worker references forbidden legacy guest: ${forbidden}`);
}
assert.match(worker, /!rewriteWasmOverride/, "Wasm overrides must remain diagnostic-only");

const runner = await readFile(join(root, "tests/vs-v86/scorecard-v2.mjs"), "utf8");
for (const required of [
  "web/images/alpine/Image",
  "matched-linux-x86-bzImage",
  "bios/seabios.bin",
  "bios/vgabios.bin",
]) {
  assert.equal(runner.includes(required), true, `scorecard preflight omits ${required}`);
}

const adapter = await readFile(join(root, "tests/vs-v86/legacy-modern-virt.patch"), "utf8");
const patchedFiles = [...adapter.matchAll(/^--- a\/(.+)$/gm)].map((match) => match[1]);
assert.deepEqual(patchedFiles, [
  "crates/rv64-system/src/virt.rs",
  "crates/rv64-wasm/src/lib.rs",
]);
assert.equal(adapter.includes("--- a/crates/rv64-jit/"), false);

const nbenchBuild = await readFile(join(root, "tests/vs-v86/mk-bench-bins.sh"), "utf8");
assert.equal(nbenchBuild.includes("root-riscv64.bin"), false);
assert.match(nbenchBuild, /SCORECARD_FIXED_DATA32/);
assert.match(nbenchBuild, /nbench-native\.rv64/);
assert.deepEqual(
  ROWS.filter((row) => row.family === "nbench").map((row) => row.nbenchFlag),
  ["DONUMSORT", "DOSTRINGSORT", "DOBITFIELD", "DOEMF", "DOFOUR", "DOASSIGN", "DOIDEA", "DOHUFF"],
);
const contractDocument = JSON.parse(await readFile(
  join(root, "tests/vs-v86/nbench-workload-contract-v3.json"),
  "utf8",
));
assert.equal(contractDocument.variant, NBENCH_WORKLOAD_CONTRACT.variant);
assert.deepEqual(contractDocument.rows, NBENCH_WORKLOAD_CONTRACT.fixedParameters);

console.log("scorecard-v2 selftest: ok");
