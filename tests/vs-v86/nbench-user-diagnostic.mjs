#!/usr/bin/env node

// Diagnostic only: run BYTEmark's first kernel through the user-mode engine.
// This removes Linux timer delivery from the equation while retaining the
// same RV64 ELF and generated instruction implementations. It is never a
// scorecard input because copy/v86 has no equivalent user-mode path.

import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const artifacts = resolve(process.env.ARTIFACTS || "");
if (!process.env.ARTIFACTS) throw new Error("set ARTIFACTS");
const jit = process.env.DISABLE_JIT !== "1";
const timeoutMs = Number(process.env.TIMEOUT_MS || 60_000);
const [{ RV64Debug }, wasm, elf] = await Promise.all([
  import(join(root, "web/rv64.js")),
  readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm")),
  readFile(join(artifacts, "nbench.rv64")),
]);

const vm = await RV64Debug.create(wasm);
vm.ex.jit_set_enabled(jit ? 1 : 0);
let output = "";
const decoder = new TextDecoder();
vm.onWrite = (_fd, bytes) => {
  output += decoder.decode(bytes, { stream: true });
};
if (!vm.loadElf(new Uint8Array(elf), ["nbench"], 64)) {
  throw new Error("failed to load nbench.rv64");
}

const started = performance.now();
let calls = 0;
const numericComplete = () => /NUMERIC SORT\s+:\s+[\d.e+]+/.test(output);
while (!numericComplete() && performance.now() - started < timeoutMs) {
  vm.runUser(2_000_000n);
  calls++;
  if ((calls & 15) === 0) await new Promise((done) => setImmediate(done));
}
console.log(JSON.stringify({
  jit,
  complete: numericComplete(),
  hostMs: performance.now() - started,
  calls,
  instructions: vm.userInsnCount().toString(),
  generated: vm.ex.jit_stat(0).toString(),
  dispatches: vm.ex.jit_stat(1).toString(),
  tail: output.slice(-1200),
}, null, 2));
if (!numericComplete()) process.exitCode = 1;
