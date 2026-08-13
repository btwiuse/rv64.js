#!/usr/bin/env node

// Source/artifact proof for the frozen R092 WANIX browser A/B guard. This
// performs no guest execution and records no elapsed performance values.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const site = "/tmp/rv64-three-way-site.8uVz6K";
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const identity = (path) => ({ bytes: readFileSync(path).byteLength, sha256: sha256(readFileSync(path)) });
const memberIdentity = (path, member) => {
  const bytes = execFileSync("tar", ["-xOf", path, member], { maxBuffer: 16 * 1024 * 1024 });
  return { bytes: bytes.byteLength, sha256: sha256(bytes) };
};

const controlPage = resolve(
  root,
  "integrations/wanix/v86-rv64-three-way-r085-candidate-0b953be67610e130.html",
);
const candidatePage = resolve(
  root,
  "integrations/wanix/v86-rv64-three-way-r092-candidate-e0488c48b3ea3980.html",
);
const controlArchive = resolve(
  root,
  "target/bench/r085-fast-jit-state-hash/browser-assets/" +
    "rv64-jit-r085-candidate-0b953be67610e130.tgz",
);
const candidateArchive = resolve(
  root,
  "target/bench/r092-member-range/browser-assets/" +
    "rv64-jit-r092-member-range-e0488c48b3ea3980.tgz",
);

assert.deepEqual(identity(controlPage), {
  bytes: 12043,
  sha256: "2a1fc2603e2be84e483bd678bf223048bc09c8fbc7e2feb1286d6337fd1b395a",
});
assert.deepEqual(identity(candidatePage), {
  bytes: 12046,
  sha256: "7c07a642f3170928b9f45ff80f03dcd1ee6276e3a40add617248e36d8f2a43b6",
});
assert.deepEqual(identity(controlArchive), {
  bytes: 1820708,
  sha256: "0b953be67610e130f79a852f86542c8400ad3a235001ec450fbdffc29ed3a61a",
});
assert.deepEqual(identity(candidateArchive), {
  bytes: 1825414,
  sha256: "e0488c48b3ea398060a42854715883eea4b34c96a13d397c5068f8a8e1c4029b",
});

for (const member of ["rv64.js", "rv64-jit-vm.wasm"]) {
  assert.deepEqual(
    memberIdentity(candidateArchive, member),
    memberIdentity(controlArchive, member),
    `${member} differs between configurations`,
  );
}
assert.equal(
  memberIdentity(controlArchive, "rv64_wasm.wasm").sha256,
  "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010",
);
assert.equal(
  memberIdentity(candidateArchive, "rv64_wasm.wasm").sha256,
  "5baeccb5c5feaf2d3f7605fd42f741f9cbaa89e566a86c0bbea201a3c6389023",
);

const normalizedControlPage = readFileSync(controlPage, "utf8").replace(
  "rv64-jit-r085-candidate-0b953be67610e130.tgz?v=0b953be67610e130",
  "RV64_ARCHIVE",
);
const normalizedCandidatePage = readFileSync(candidatePage, "utf8").replace(
  "rv64-jit-r092-member-range-e0488c48b3ea3980.tgz?v=e0488c48b3ea3980",
  "RV64_ARCHIVE",
);
assert.equal(normalizedCandidatePage, normalizedControlPage, "pages differ beyond archive identity");

for (const [local, deployed] of [
  [controlPage, resolve(site, "examples", controlPage.split("/").at(-1))],
  [candidatePage, resolve(site, "examples", candidatePage.split("/").at(-1))],
  [controlArchive, resolve(site, "rv64", controlArchive.split("/").at(-1))],
  [candidateArchive, resolve(site, "rv64", candidateArchive.split("/").at(-1))],
]) assert.deepEqual(identity(deployed), identity(local), `${deployed} is not the frozen local asset`);

const runner = readFileSync(resolve(root, "tests/run-r092-wanix-browser-pairs.mjs"), "utf8");
for (const contract of [
  "const pairs = 7;",
  "const repetitions = 3;",
  'phases: ["python", "sha256", "shared9p"]',
  "minimumPairedMedianSpeedup: 1 / 1.03",
  "minimumPairedBootstrapLower: 1 / 1.10",
  'WANIX_JIT_CONFIG: "{}"',
]) assert.ok(runner.includes(contract), `missing runner contract: ${contract}`);

console.log(
  "R092 WANIX harness selftest: immutable product artifacts, one-link page delta, fresh 7x3 browsers, and regression guards enforced",
);
