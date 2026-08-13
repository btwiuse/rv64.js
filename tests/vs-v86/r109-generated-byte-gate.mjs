#!/usr/bin/env node

// Compare deterministic real-RV64 translation outputs from the test-only
// ordered-tree selector and the production dense stackifier.

import { createHash } from "node:crypto";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const evidence = join(root, "target/bench/r109-dense-cfg");
const controlRoot = resolve(process.argv[2] || join(evidence, "real-region-control"));
const candidateRoot = resolve(process.argv[3] || join(evidence, "real-region-candidate"));
const output = resolve(process.argv[4] || join(evidence, "generated-byte-gate.json"));

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const controlManifest = await readFile(join(controlRoot, "manifest.tsv"));
const candidateManifest = await readFile(join(candidateRoot, "manifest.tsv"));
const controlNames = (await readdir(controlRoot)).filter((name) => name.endsWith(".wasm")).sort();
const candidateNames = (await readdir(candidateRoot)).filter((name) => name.endsWith(".wasm")).sort();
const problems = [];
if (!controlManifest.equals(candidateManifest)) problems.push("manifests differ");
if (JSON.stringify(controlNames) !== JSON.stringify(candidateNames)) problems.push("module names differ");

const modules = [];
for (const name of controlNames) {
  const [control, candidate] = await Promise.all([
    readFile(join(controlRoot, name)),
    readFile(join(candidateRoot, name)),
  ]);
  const controlSha256 = sha256(control);
  const candidateSha256 = sha256(candidate);
  const exact = control.equals(candidate);
  if (!exact) problems.push(`${name}: bytes differ`);
  modules.push({
    name,
    bytes: control.length,
    controlSha256,
    candidateSha256,
    exact,
  });
}

const inputs = {};
for (const [name, path] of Object.entries({
  rvbench: join(root, "target/bench/xbench/rvbench.rv64"),
  alpineMusl: join(root, "target/bench/alpine-riscv64/lib/ld-musl-riscv64.so.1"),
})) {
  const bytes = await readFile(path);
  inputs[name] = { path, bytes: bytes.length, sha256: sha256(bytes) };
}

const report = {
  schema: 1,
  experiment: "R109 deterministic generated-Wasm byte gate",
  control: "test-only ordered BTreeMap/BTreeSet stackifier",
  candidate: "production fixed dense bit-matrix/bit-set stackifier",
  generator: "cargo run --release -p rv64-dbt --example emit_real_region_corpus",
  geometries: "seven page/leader geometries, five state modes, two real RV64 ELF inputs",
  inputs,
  manifest: {
    bytes: controlManifest.length,
    controlSha256: sha256(controlManifest),
    candidateSha256: sha256(candidateManifest),
    exact: controlManifest.equals(candidateManifest),
  },
  moduleCount: modules.length,
  exactModuleCount: modules.filter((module) => module.exact).length,
  byteExact: problems.length === 0,
  problems,
  modules,
};
await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(JSON.stringify({
  output,
  sha256: sha256(await readFile(output)),
  modules: modules.length,
  byteExact: report.byteExact,
  problems,
}));
if (problems.length) process.exitCode = 1;
