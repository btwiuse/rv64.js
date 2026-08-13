#!/usr/bin/env node

// Source/artifact proof for the fixed R076 candidate/v86 WANIX guard. This
// performs no guest execution and records no elapsed performance values.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const site = "/tmp/rv64-three-way-site.8uVz6K";
const pagePath = resolve(
  site,
  "examples/v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html",
);
const v86Archive = resolve(site, "v86/v86.tgz");
const rv64Archive = resolve(site, "rv64/rv64-jit-r075-e0c1971d1ecd4d4f.tgz");
const rv64Root = resolve(site, "extras/dist/wanix-linux-rv64.tgz");
const x86Root = resolve(site, "extras/dist/wanix-linux-x86.tgz");
const runnerPath = resolve(root, "tests/run-wanix-pairs.mjs");
const wrapperPath = resolve(root, "tests/run-r076-wanix-v86-pairs.mjs");
const harnessPath = resolve(root, "tests/wanix-v86-preboot-smoke.mjs");
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

for (const [path, expected] of [
  [rv64Archive, "e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c"],
  [rv64Root, "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb"],
  [v86Archive, "7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771"],
  [x86Root, "09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320"],
]) {
  assert.equal(sha256(readFileSync(path)), expected, `${path} changed`);
}

assert.equal(
  sha256(readFileSync(runnerPath)),
  "8d37e7b20186253a0b7e71e5b7c28f3d8ee3b34a49a7eb4374553c5b80ee4e80",
  "shared runner changed",
);
assert.equal(
  sha256(readFileSync(wrapperPath)),
  "15cf55e647c285558046367fcff4f6a083cd7b3c47de540585f41b45d84b4220",
  "fixed runner wrapper changed",
);
assert.equal(
  sha256(readFileSync(harnessPath)),
  "525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545",
  "WANIX harness changed",
);

const page = readFileSync(pagePath);
assert.equal(page.byteLength, 12045, "candidate page size");
assert.equal(
  sha256(page),
  "7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413",
  "candidate page changed",
);
const pageText = page.toString("utf8");
assert.equal((pageText.match(/rv64\.static-t0=sampled-backoff/g) ?? []).length, 1,
  "candidate preboot selector");
for (const binding of [
  "../rv64/rv64-jit-r075-e0c1971d1ecd4d4f.tgz",
  "../v86/v86.tgz",
  "../extras/dist/wanix-linux-rv64.tgz",
  "../extras/dist/wanix-linux-x86.tgz",
]) {
  assert.ok(pageText.includes(binding), `missing page binding ${binding}`);
}

const runnerSource = readFileSync(runnerPath, "utf8");
for (const sourceContract of [
  "const pairs = r076CandidateV86 ? 7",
  "const repetitions = r076CandidateV86 ? 3 : 1",
  "if (r076CandidateV86) environment.WANIX_JIT_PREBOOT = \"1\";",
  "sampledStaticT0Backoff: true",
  "maximumPairedMedianBootstrapUpper: 1.10",
]) {
  assert.ok(runnerSource.includes(sourceContract), `runner contract missing: ${sourceContract}`);
}
const wrapperSource = readFileSync(wrapperPath, "utf8");
assert.ok(wrapperSource.includes("process.argv.length !== 3"), "wrapper admits extra knobs");
assert.ok(wrapperSource.includes('process.argv.push("--r076-candidate-v86")'),
  "wrapper does not select fixed mode");

const extract = (archive, member) => {
  const result = spawnSync("tar", ["-xOf", archive, member], {
    maxBuffer: 64 * 1024 * 1024,
  });
  assert.equal(result.status, 0, result.stderr.toString());
  return result.stdout;
};
assert.equal(
  sha256(extract(rv64Archive, "rv64_wasm.wasm")),
  "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c",
  "RV64 archive main Wasm changed",
);
const v86Wasm = extract(v86Archive, "./v86.wasm");
const v86Exports = new Set(WebAssembly.Module.exports(new WebAssembly.Module(v86Wasm))
  .map(({ name }) => name));
for (const name of [
  "get_jit_config", "set_jit_config", "jit_get_cache_size", "codegen_finalize_finished",
]) {
  assert.ok(v86Exports.has(name), `copy/v86 archive lacks ${name}`);
}
const wrapperStrings = spawnSync("strings", [], {
  input: extract(v86Archive, "./v86-vm.wasm"),
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
});
assert.equal(wrapperStrings.status, 0, wrapperStrings.stderr);
assert.match(wrapperStrings.stdout, /disable_jit && this\.set_jit_config\(0, 1\)/,
  "copy/v86 wrapper no longer makes JIT disabling conditional");
assert.match(wrapperStrings.stdout, /WebAssembly\.instantiate\(f, \{ e: this\.jit_imports \}\)/,
  "copy/v86 wrapper lacks generated-module instantiation path");

const response = await fetch(
  "http://127.0.0.1:8765/examples/" +
    "v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html",
  { cache: "no-store" },
);
assert.equal(response.status, 200, "comparison server response");
const servedPage = Buffer.from(await response.arrayBuffer());
assert.equal(sha256(servedPage), sha256(page), "served candidate page differs from frozen file");

console.log(
  "R076 WANIX-v86 harness selftest: fixed 7x3 preboot mode, artifacts, and copy/v86 JIT-capable default enforced",
);
