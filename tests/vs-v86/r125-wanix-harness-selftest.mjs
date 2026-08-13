#!/usr/bin/env node

// Source/artifact proof for the frozen R125 WANIX artifact A/B guard.
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

const pageA = resolve(root, "integrations/wanix/v86-rv64-three-way-r125-control.html");
const pageB = resolve(root, "integrations/wanix/v86-rv64-three-way-r125-candidate.html");
const publicPage = resolve(root, "integrations/wanix/v86-rv64-three-way.html");
const controlArchive = resolve(
  root,
  "target/bench/r125-wanix-singleflight/assets/" +
    "rv64-jit-r125-control-5fe22762302f6414.tgz",
);
const candidateArchive = resolve(
  root,
  "target/bench/r125-wanix-singleflight/assets/" +
    "rv64-jit-r125-candidate-88274081a25ee03a.tgz",
);

assert.deepEqual(identity(pageA), {
  bytes: 12042,
  sha256: "0fa38205e9c6306564f9b1f0d438816f24caef5bd113d8a8a86a0fddc4ba08f0",
});
assert.deepEqual(identity(pageB), {
  bytes: 12044,
  sha256: "2da494b4c379e1deebca322cb7526a11a7fa33873836a43811456767ef0dc8c3",
});
assert.deepEqual(identity(controlArchive), {
  bytes: 1820749,
  sha256: "5fe22762302f6414c4b9c3aa6f85010996cf7bce8eb7ff4da3eb37099059a75e",
});
assert.deepEqual(identity(candidateArchive), {
  bytes: 1820896,
  sha256: "88274081a25ee03a0d3e926bc2a0d9a90c53e00cce665c5a98a51b95c58720a4",
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
assert.deepEqual(memberIdentity(controlArchive, "rv64-jit-vm.wasm"), {
  bytes: 3530750,
  sha256: "bba6baaf8ddaa8245b00b1dd3d0db7bf72ffc7e48a1e9db3c0e2ab7f3bde8bbe",
});

const sourceA = readFileSync(pageA, "utf8");
const sourceB = readFileSync(pageB, "utf8");
const normalizedA = sourceA.replace(
  "rv64-jit-r125-control-5fe22762302f6414.tgz?v=5fe22762302f6414", "RV64_ARCHIVE");
const normalizedB = sourceB.replace(
  "rv64-jit-r125-candidate-88274081a25ee03a.tgz?v=88274081a25ee03a", "RV64_ARCHIVE");
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
  resolve(root, "tests/run-r125-wanix-browser-pairs.mjs"), "utf8",
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
  "bba6baaf8ddaa8245b00b1dd3d0db7bf72ffc7e48a1e9db3c0e2ab7f3bde8bbe",
]) assert.ok(runner.includes(contract), `missing runner contract: ${contract}`);

const analyzer = readFileSync(
  resolve(root, "tests/analyze-r125-wanix-browser-pairs.mjs"), "utf8",
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
  "R125 WANIX harness selftest: exact A/B pages, common single-flight adapter, unchanged Python, qualified 32 MiB 9P work, fresh 7x3 browsers, and one-percent guards enforced",
);
