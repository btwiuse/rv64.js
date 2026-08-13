#!/usr/bin/env node
// jit_set_enabled(0) is a performance baseline, not merely a compilation
// threshold. A modern Virt slice must go directly through VirtMachine's
// interpreter driver without creating or entering the JIT dispatcher.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

const vm = await RV64.create(wasm);
vm.onWrite = () => {};
vm.ex.jit_set_enabled(0);
vm.bootVirtLinuxDirect({
  // jal x0, 0: an indefinitely executable S-mode kernel is enough to verify
  // that the public budget reaches the direct interpreter unchanged.
  kernel: new Uint8Array([0x6f, 0x00, 0x00, 0x00]),
  ramMB: 32,
});

const budget = 100_000n;
assert.equal(vm.runVirtSystem(budget), false);
assert.equal(vm.virtInsnCount(), budget);
assert.equal(vm.ex.jit_stat(0), 0n, "disabled mode retired generated code");
assert.equal(vm.ex.jit_stat(1), 0n, "disabled mode entered generated dispatch");
assert.equal(vm.ex.jit_stat(3), 0n, "disabled mode populated the JIT cache");
assert.equal(vm.ex.jit_stat(4), 0n, "disabled mode entered JIT fallback slicing");
assert.equal(vm.ex.jit_stat(77), 0n, "disabled mode attempted translation");
assert.equal(vm.jitRegCount ?? 0, 0, "disabled mode registered a Wasm module");

console.log(`PASS true JIT bypass — ${budget} instructions in one direct interpreter slice`);
