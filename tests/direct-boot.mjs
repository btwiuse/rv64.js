#!/usr/bin/env node
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { RV64 } from "../web/rv64.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const wasmPath = join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
const kernelPath = resolve(
  root,
  process.env.RV64_MODERN_KERNEL || "web/images/alpine/Image",
);
const diskPath = join(root, "web/images/alpine/debian.ext4");
if (![wasmPath, kernelPath, diskPath].every(existsSync)) {
  console.log("SKIP direct Linux boot (prepare modern images first)");
  process.exit(0);
}

let output = "";
let observedError;
const decoder = new TextDecoder();
const vm = await RV64.create({
  wasm: await readFile(wasmPath),
  memoryMB: 512,
  boot: {
    mode: "linux-direct",
    kernel: await readFile(kernelPath),
    disk: await readFile(diskPath),
    cmdline: "console=ttyS0 root=/dev/vda rw init=/binit.sh",
  },
  network: { mode: "none" },
  events: {
    console: (bytes) => {
      output += decoder.decode(bytes, { stream: true });
      if (process.env.RV64_BOOT_TRACE) process.stdout.write(bytes);
    },
    error: (error) => { observedError = error; },
  },
});

await vm.start();
const deadline = performance.now() + 180_000;
while (vm.running && !output.includes("BENCH_READY") && performance.now() < deadline) {
  await new Promise((resolveImmediate) => setImmediate(resolveImmediate));
}
await vm.stop();
const instructions = vm.instructions;
await vm.destroy();

assert.ifError(observedError);
assert.match(output, /Linux version/);
assert.match(output, /SBI specification v2\.0 detected/);
assert.match(output, /SBI implementation ID=0x52563634 Version=0x1/);
assert.match(output, /VFS: Mounted root/);
assert.match(output, /BENCH_READY/);
assert.doesNotMatch(output, /OpenSBI v/);
assert.ok(instructions > 30_000_000n);
console.log(`PASS direct Linux boot — ${instructions} instructions`);
