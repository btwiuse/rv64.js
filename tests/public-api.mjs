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
assert.equal(vm.network.mode, "none");
assert.equal(vm.network.proxyURL, undefined);
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

const direct = await RV64.create({
  wasm,
  memoryMB: 8,
  boot: {
    mode: "linux-direct",
    kernel: new Uint8Array([0x73, 0x00, 0x10, 0x00]),
  },
});
assert.equal(direct.running, false);
assert.equal(direct.instructions, 0n);
assert.equal(direct.network.mode, "fetch");
assert.match(direct.network.proxyURL, /^http:\/\/10\.0\.2\.2:/);
assert.throws(() => direct.network.receive(new Uint8Array(14)), /external mode/);
await direct.destroy();

const external = await RV64.create({
  wasm,
  memoryMB: 8,
  boot: {
    mode: "linux-direct",
    kernel: new Uint8Array([0x73, 0x00, 0x10, 0x00]),
  },
  network: { mode: "external", mac: new Uint8Array([2, 0, 0, 0, 0, 2]) },
});
assert.equal(external.network.mode, "external");
external.network.receive(new Uint8Array(14));
await external.destroy();

const RealWebSocket = globalThis.WebSocket;
class TestWebSocket {
  static OPEN = 1;
  readyState = 0;
  constructor(url, protocols) {
    this.url = url;
    this.protocols = protocols;
  }
  close() { this.readyState = 3; }
  send() {}
}
globalThis.WebSocket = TestWebSocket;
for (const network of [
  { mode: "wsproxy", url: "wss://relay.example/" },
  { mode: "wisp", url: "wisps://relay.example/" },
  { mode: "inbrowser", channel: "rv64-api-test" },
]) {
  const networkVM = await RV64.create({
    wasm,
    memoryMB: 8,
    boot: { mode: "linux-direct", kernel: new Uint8Array([0x73, 0, 0x10, 0]) },
    network,
  });
  assert.equal(networkVM.network.mode, network.mode);
  await networkVM.destroy();
}
globalThis.WebSocket = RealWebSocket;

await assert.rejects(
  RV64.create({
    wasm,
    boot: { mode: "linux-direct", kernel: new Uint8Array(4) },
    network: { mode: "wisp" },
  }),
  /wisp networking requires url/,
);

await assert.rejects(
  RV64.create({
    wasm,
    boot: { mode: "bare-metal", image: new Uint8Array(4), loadAddress: 0x80000000n },
    network: { mode: "external" },
  }),
  /bare-metal networking is not implemented/,
);
console.log("PASS stable public API lifecycle");
