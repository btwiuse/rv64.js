#!/usr/bin/env node
// Deterministic Wasm-JIT boot gate for the supported Linux 6.12/Alpine 3.24
// machine. Both firmware paths must reach a shell and execute generated code.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  bootModern,
  guestCommand,
  loadModernImages,
  machineDiagnostics,
  missingModernImages,
  output,
  waitForAlpine,
} from "./modern-linux-harness.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const missing = missingModernImages(root);
if (missing.length) {
  console.log(`SKIP modern Linux JIT boot (run web/prepare-images.sh; missing ${missing.join(", ")})`);
  process.exit(process.env.REQUIRE_ALL === "1" ? 2 : 0);
}
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);
const images = await loadModernImages(root);

for (const mode of ["direct", "opensbi"]) {
  const modules = [];
  const machine = await bootModern({
    RV64,
    wasm,
    images,
    mode,
    jit: true,
    superblock: true,
    onJitModule: (bytes, metadata) => modules.push({ bytes: bytes.length, ...metadata }),
  });
  const ready = await waitForAlpine(machine);
  assert.ok(ready, `${mode} boot timeout: ${machineDiagnostics(machine)}`);
  assert.match(output(machine), /Linux version 6\./);
  if (mode === "opensbi") assert.match(output(machine), /OpenSBI v/);
  else assert.doesNotMatch(output(machine), /OpenSBI v/);
  assert.ok(await guestCommand(
    machine,
    `i=0; while [ $i -lt 20000 ]; do i=$((i+1)); done; ` +
      `echo MODERN_${mode.toUpperCase()}_'OK'`,
    `MODERN_${mode.toUpperCase()}_OK`,
    120_000,
  ), `${mode} shell command timeout: ${machineDiagnostics(machine)}`);
  const total = machine.vm.virtInsnCount();
  const retired = machine.vm.ex.jit_stat(0);
  const entries = machine.vm.ex.jit_stat(3);
  const landed = machine.vm.ex.jit_stat(13);
  const batches = machine.vm.ex.jit_stat(43);
  const regions = modules.filter((module) => module.kind === "async-region");
  assert.ok(total > 30_000_000n, `${mode} retired too little work: ${total}`);
  assert.ok(retired > 0n, `${mode} executed no generated instructions`);
  assert.ok(entries > 0n, `${mode} installed no generated entries`);
  assert.ok(landed + batches > 0n, `${mode} exercised no T2 module`);
  if (mode === "direct") {
    assert.equal(machine.vm.ex.virt_unsupported_sbi_ext(), 0n);
  }
  console.log(
    `PASS modern Linux ${mode}: total=${total} jit-retired=${retired} ` +
      `jit-dispatches=${machine.vm.ex.jit_stat(1)} entries=${entries} ` +
      `landed=${landed} batches=${batches} region-modules=${regions.length} ` +
      `region-bytes-max=${Math.max(0, ...regions.map((region) => region.bytes))}`,
  );
}
