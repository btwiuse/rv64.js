#!/usr/bin/env node

// Source/artifact proof for the frozen R094 long shared-9P null qualification.
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

const pageA = resolve(root, "integrations/wanix/v86-rv64-three-way-r094-long9p-a.html");
const pageB = resolve(root, "integrations/wanix/v86-rv64-three-way-r094-long9p-b.html");
const publicPage = resolve(root, "integrations/wanix/v86-rv64-three-way.html");
const archive = resolve(
  root,
  "target/bench/r085-fast-jit-state-hash/browser-assets/" +
    "rv64-jit-r085-candidate-0b953be67610e130.tgz",
);

assert.deepEqual(identity(pageA), {
  bytes: 12041,
  sha256: "6d264f61dcc274dce67cf6d22e0a4305530dd76691e1b27615e5d8356a08c98c",
});
assert.deepEqual(identity(pageB), {
  bytes: 12041,
  sha256: "dd042aa6f6c1913b552c936afb7956817f1805dc886b1d530d744ef4847c0cd8",
});
assert.deepEqual(identity(archive), {
  bytes: 1820708,
  sha256: "0b953be67610e130f79a852f86542c8400ad3a235001ec450fbdffc29ed3a61a",
});
assert.deepEqual(memberIdentity(archive, "rv64_wasm.wasm"), {
  bytes: 4279378,
  sha256: "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010",
});

const sourceA = readFileSync(pageA, "utf8");
const sourceB = readFileSync(pageB, "utf8");
const normalizedA = sourceA.replace("?v=r094-long9p-a", "?v=R094_LONG9P");
const normalizedB = sourceB.replace("?v=r094-long9p-b", "?v=R094_LONG9P");
assert.equal(normalizedA, normalizedB, "null pages differ beyond their cache query token");
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
const deployedArchive = resolve(site, "rv64", archive.split("/").at(-1));
assert.deepEqual(identity(deployedArchive), identity(archive),
  `${deployedArchive} is not exact R085`);

const runner = readFileSync(
  resolve(root, "tests/run-r094-wanix-long9p-qualification.mjs"), "utf8",
);
for (const contract of [
  "const pairs = 7;",
  "const repetitions = 3;",
  'phases: ["shared9p"]',
  "minimumPairedMedianSpeedup: 0.97",
  "maximumPairedMedianSpeedup: 1.03",
  "minimumPairedBootstrapLower: 1 / 1.10",
  "maximumPairedBootstrapUpper: 1.10",
  "maximumWithinBrowserSpread: 1.25",
  "minimumSampleSeconds: 2.0",
  "exactP9WriteBytes: 33_554_432",
  "minimumP9ReadBytes: 33_554_432",
  "maximumP9TransferBytes: 4_096",
  'WANIX_JIT_CONFIG: "{}"',
]) assert.ok(runner.includes(contract), `missing runner contract: ${contract}`);

const analyzer = readFileSync(
  resolve(root, "tests/analyze-r094-wanix-browser-pairs.mjs"), "utf8",
);
for (const contract of [
  "value >= protocol.thresholds.minimumSampleSeconds",
  "maximum / minimum <= protocol.thresholds.maximumWithinBrowserSpread",
  "proof.p9WriteBytes, protocol.thresholds.exactP9WriteBytes",
  "proof.p9ReadBytes >= protocol.thresholds.minimumP9ReadBytes",
  "proof.p9MaximumWrite, protocol.thresholds.maximumP9TransferBytes",
  "proof.p9MaximumRead, protocol.thresholds.maximumP9TransferBytes",
  "pairedMedianSpeedup > protocol.thresholds.maximumPairedMedianSpeedup",
  "pairedMedianBootstrap95[1] > protocol.thresholds.maximumPairedBootstrapUpper",
  "pairedMedianBootstrap95[0] > 1 || pairedMedianBootstrap95[1] < 1",
]) assert.ok(analyzer.includes(contract), `missing analyzer contract: ${contract}`);

console.log(
  "R094 WANIX harness selftest: exact-R085 null pages, 32 MiB fixed work, fresh 7x3 browsers, byte/accounting proofs, and two-sided variance guards enforced",
);
