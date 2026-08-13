#!/usr/bin/env node

// Source/artifact proof for the frozen R124 WANIX artifact A/B guard.
// This performs no guest execution and records no elapsed performance values.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const site = "/tmp/rv64-three-way-site.8uVz6K";
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const identity = (path) => {
  const bytes = readFileSync(path);
  return { bytes: bytes.byteLength, sha256: sha256(bytes) };
};
const memberIdentity = (path, member) => {
  const bytes = execFileSync("tar", ["-xOf", path, member], { maxBuffer: 16 * 1024 * 1024 });
  return { bytes: bytes.byteLength, sha256: sha256(bytes) };
};

const pageA = resolve(root, "integrations/wanix/v86-rv64-three-way-r124-control.html");
const pageB = resolve(root, "integrations/wanix/v86-rv64-three-way-r124-candidate.html");
const publicPage = resolve(root, "integrations/wanix/v86-rv64-three-way.html");
const controlArchive = resolve(
  root,
  "target/bench/r124-rvc-bank-hybrid/wanix-assets/" +
    "rv64-jit-r124-control-9d0bf45cdbcffcc0.tgz",
);
const candidateArchive = resolve(
  root,
  "target/bench/r124-rvc-bank-hybrid/wanix-assets/" +
    "rv64-jit-r124-candidate-76c7139ba38c2f65.tgz",
);

assert.deepEqual(identity(pageA), {
  bytes: 12042,
  sha256: "ac3b9c63e67b0a46b95a0abe7b79b44868a0be4d23bdd51078ba024c805bf5da",
});
assert.deepEqual(identity(pageB), {
  bytes: 12044,
  sha256: "f5086330565fd781ab7f5929af9fad896b74d3cc9e39c65e9aaa5013b5bcf525",
});
assert.deepEqual(identity(controlArchive), {
  bytes: 1820649,
  sha256: "9d0bf45cdbcffcc06f68ac48a5e5692e548c5c9a4b310a236dc4bcbb8086a98d",
});
assert.deepEqual(identity(candidateArchive), {
  bytes: 1821157,
  sha256: "76c7139ba38c2f658d981ebd24bbeeb0308e1acf0a5b593a9b0784d32f9127d8",
});
assert.deepEqual(memberIdentity(controlArchive, "rv64_wasm.wasm"), {
  bytes: 4279380,
  sha256: "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
});
assert.deepEqual(memberIdentity(candidateArchive, "rv64_wasm.wasm"), {
  bytes: 4281786,
  sha256: "d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59",
});
for (const member of ["rv64.js", "rv64-jit-vm.wasm"]) {
  assert.deepEqual(memberIdentity(controlArchive, member),
    memberIdentity(candidateArchive, member), `${member} differs`);
}

const sourceA = readFileSync(pageA, "utf8");
const sourceB = readFileSync(pageB, "utf8");
const normalizedA = sourceA.replace(
  "rv64-jit-r124-control-9d0bf45cdbcffcc0.tgz?v=9d0bf45cdbcffcc0", "RV64_ARCHIVE");
const normalizedB = sourceB.replace(
  "rv64-jit-r124-candidate-76c7139ba38c2f65.tgz?v=76c7139ba38c2f65", "RV64_ARCHIVE");
assert.equal(normalizedA, normalizedB, "pages differ beyond archive identity");
for (const source of [sourceA, sourceB]) {
  const sharedIo = source.match(/def shared_io\(\):([\s\S]*?)\nprint\(/)?.[1];
  assert.ok(sharedIo, "shared_io workload missing");
  assert.ok(sharedIo.includes("total = 32 * 1024 * 1024"), "shared_io is not 32 MiB");
  assert.ok(sharedIo.includes('open(path, "wb", buffering=0)'), "write is buffered");
  assert.ok(sharedIo.includes('open(path, "rb", buffering=0)'), "read is buffered");
  assert.ok(sharedIo.includes("os.unlink(path)"), "shared_io cleanup missing");
}
const publicSharedIo = readFileSync(publicPage, "utf8")
  .match(/def shared_io\(\):([\s\S]*?)\nprint\(/)?.[1];
assert.ok(publicSharedIo?.includes("total = 4 * 1024 * 1024"),
  "ordinary public page must retain its 4 MiB shared_io workload");

for (const page of [pageA, pageB]) {
  const deployed = resolve(site, "examples", page.split("/").at(-1));
  assert.deepEqual(identity(deployed), identity(page), `${deployed} is not the frozen local page`);
}
for (const archive of [controlArchive, candidateArchive]) {
  const deployedArchive = resolve(site, "rv64", archive.split("/").at(-1));
  assert.deepEqual(identity(deployedArchive), identity(archive),
    `${deployedArchive} is not the frozen local artifact`);
}

const runner = readFileSync(
  resolve(root, "tests/run-r124-wanix-browser-pairs.mjs"), "utf8",
);
for (const contract of [
  "const pairs = 7;",
  "const repetitions = 3;",
  'phases: ["python", "sha256", "shared9p"]',
  "minimumPairedMedianSpeedup: 0.99",
  "minimumPairedBootstrapUpper: 1.00",
  "minimumPhaseBootstrapLower: 1 / 1.10",
  "maximumWithinBrowserSpread: 1.25",
  "minimumShared9pSampleSeconds: 2.0",
  "exactP9WriteBytes: 33_554_432",
  "minimumP9ReadBytes: 33_554_432",
  "maximumP9TransferBytes: 4_096",
  'WANIX_JIT_CONFIG: "{}"',
]) assert.ok(runner.includes(contract), `missing runner contract: ${contract}`);

const analyzer = readFileSync(
  resolve(root, "tests/analyze-r124-wanix-browser-pairs.mjs"), "utf8",
);
for (const contract of [
  "value >= protocol.thresholds.minimumShared9pSampleSeconds",
  "maximum / minimum <= protocol.thresholds.maximumWithinBrowserSpread",
  "proof.p9WriteBytes, protocol.thresholds.exactP9WriteBytes",
  "proof.p9ReadBytes >= protocol.thresholds.minimumP9ReadBytes",
  "proof.p9MaximumWrite, protocol.thresholds.maximumP9TransferBytes",
  "proof.p9MaximumRead, protocol.thresholds.maximumP9TransferBytes",
  "pairedMedianSpeedup < protocol.thresholds.minimumPairedMedianSpeedup",
  "pairedMedianBootstrap95[1] < protocol.thresholds.minimumPairedBootstrapUpper",
  "pairedMedianBootstrap95[0] < protocol.thresholds.minimumPhaseBootstrapLower",
]) assert.ok(analyzer.includes(contract), `missing analyzer contract: ${contract}`);

console.log(
  "R124 WANIX harness selftest: exact A/B pages and archives, unchanged Python, qualified 32 MiB 9P work, fresh 7x3 browsers, and one-percent guards enforced",
);
