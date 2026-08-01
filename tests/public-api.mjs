#!/usr/bin/env node
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { RV64 } from "../web/rv64.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const wasm = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);
const events = [];
const vm = await RV64.create({
  wasm,
  memoryMB: 1,
  boot: {
    mode: "bare-metal",
    image: new Uint8Array([0x73, 0x00, 0x10, 0x00]), // ebreak
    loadAddress: 0x80000000n,
  },
  events: {
    ready: () => events.push("ready"),
    start: () => events.push("start"),
    stop: ({ reason }) => events.push(`stop:${reason}`),
  },
});

assert.equal(vm.running, false);
assert.deepEqual(events, ["ready"]);
assert.equal("ex" in vm, false);
for (const removed of [
  "bootLinux",
  "bootVirtLinux",
  "runSystem",
  "runVirtSystem",
  "consoleInput",
  "virtConsoleInput",
]) {
  assert.equal(removed in vm, false, `${removed} must not be public`);
}

const unsubscribe = vm.on("start", () => events.push("second-start"));
await vm.start();
assert.equal(vm.running, true);
while (vm.running) await new Promise((resolve) => setImmediate(resolve));
assert.deepEqual(events, ["ready", "start", "second-start", "stop:powered-off"]);
assert.ok(vm.instructions > 0n);

unsubscribe();
await vm.reset();
assert.equal(vm.instructions, 0n);
await vm.start();
while (vm.running) await new Promise((resolve) => setImmediate(resolve));
assert.equal(events.filter((event) => event === "second-start").length, 1);

await vm.destroy();
await vm.destroy();
assert.throws(() => vm.instructions, /destroyed/);
console.log("PASS stable public API lifecycle");
