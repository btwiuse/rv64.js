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
vm.setJitEnabled(false);
vm.setJitEnabled(true);
assert.throws(() => vm.setJitEnabled(1), /boolean/);
const initialJitStats = vm.jitStats();
assert.equal(initialJitStats.instructions, "0");
assert.equal(initialJitStats.accountedInstructions, "0");
assert.equal(initialJitStats.generatedCoverage, 0);
assert.equal(initialJitStats.pagePolicy.enabled, "1");
assert.equal(initialJitStats.pagePolicy.threshold, "131072");
assert.equal(initialJitStats.pagePolicy.privilegedThresholdMultiplier, "32");
assert.equal(initialJitStats.pagePolicy.privilegedControlEntriesEnabled, "0");
assert.equal(initialJitStats.pagePolicy.stableChainEnabled, "1");
assert.equal(initialJitStats.pagePolicy.controlEntriesEnabled, "1");
assert.equal(initialJitStats.pagePolicy.controlProfileEnabled, "0");
assert.equal(initialJitStats.pagePolicy.inflightLimit, "2");
assert.equal(initialJitStats.pagePolicy.multiPageControlPermille, "100");
assert.equal(initialJitStats.pagePolicy.regionPageCap, "2");
assert.equal(initialJitStats.pagePolicy.regionLeaderCap, "512");
assert.equal(initialJitStats.pagePolicy.regionTailChainEnabled, "1");
assert.equal(initialJitStats.staticT0.supported, false);
assert.equal(initialJitStats.staticT0.systemFastRetired, "0");
vm.configureJit({
  policy: "adaptive",
  pageThreshold: 200_000,
  privilegedPageThresholdMultiplier: 7,
  pageQuantum: 32,
  regionLeaderCap: 64,
  regionPageCap: 2,
  pageInflightLimit: 2,
  multiPageEntryCap: 16,
  directDispatch: true,
  lazyState: true,
  pageRebuild: false,
  controlEntries: true,
  privilegedControlEntries: true,
  stablePageChain: false,
  controlProfile: true,
  cfgBlocks: true,
  structuredCfg: true,
  regionTailChain: false,
});
assert.equal(vm.jitStats().pagePolicy.enabled, "0");
assert.equal(vm.jitStats().pagePolicy.threshold, "200000");
assert.equal(vm.jitStats().pagePolicy.privilegedThresholdMultiplier, "7");
assert.equal(vm.jitStats().pagePolicy.rebuildEnabled, "0");
assert.equal(vm.jitStats().pagePolicy.controlEntriesEnabled, "1");
assert.equal(vm.jitStats().pagePolicy.privilegedControlEntriesEnabled, "1");
assert.equal(vm.jitStats().pagePolicy.stableChainEnabled, "0");
assert.equal(vm.jitStats().pagePolicy.controlProfileEnabled, "1");
assert.equal(vm.jitStats().pagePolicy.inflightLimit, "2");
assert.equal(vm.jitStats().pagePolicy.multiPageEntryCap, "16");
assert.equal(vm.jitStats().pagePolicy.regionPageCap, "2");
assert.equal(vm.jitStats().pagePolicy.regionLeaderCap, "64");
vm.configureJit({
  policy: "page",
  directDispatch: false,
  lazyState: false,
  pageRebuild: true,
  controlEntries: false,
  controlProfile: false,
});
assert.equal(vm.jitStats().pagePolicy.rebuildEnabled, "1");
assert.equal(vm.jitStats().pagePolicy.controlEntriesEnabled, "0");
assert.equal(vm.jitStats().pagePolicy.controlProfileEnabled, "0");
assert.throws(() => vm.configureJit({ policy: "unknown" }), /page or adaptive/);
assert.throws(() => vm.configureJit({ pageQuantum: 0 }), /pageQuantum/);
assert.throws(
  () => vm.configureJit({ privilegedPageThresholdMultiplier: 0 }),
  /privilegedPageThresholdMultiplier/,
);
assert.throws(() => vm.configureJit({ pageInflightLimit: 0 }), /pageInflightLimit/);
assert.throws(() => vm.configureJit({ multiPageEntryCap: 513 }), /multiPageEntryCap/);
assert.throws(() => vm.configureJit({ directDispatch: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ pageRebuild: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ controlEntries: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ privilegedControlEntries: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ stablePageChain: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ controlProfile: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ cfgBlocks: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ structuredCfg: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ tlbHash: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ regionTailChain: 1 }), /boolean/);
assert.throws(() => vm.configureJit({ staticSystemT0: false }), /experiment was rejected/);
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
const completedJitStats = vm.jitStats();
assert.equal(completedJitStats.instructions, vm.instructions.toString());
assert.equal(
  completedJitStats.accountedInstructions,
  (BigInt(completedJitStats.generated.retired) +
    BigInt(completedJitStats.interpreter.retired)).toString(),
);

unsubscribe();
await vm.reset();
assert.equal(vm.instructions, 0n);
await vm.start();
while (vm.running) await new Promise((resolve) => setImmediate(resolve));
assert.equal(events.filter((event) => event === "second-start").length, 1);

await vm.destroy();
await vm.destroy();
assert.throws(() => vm.instructions, /destroyed/);
assert.throws(() => vm.jitStats(), /destroyed/);
assert.throws(() => vm.configureJit({ policy: "page" }), /destroyed/);

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
const defaultModules = direct.jitStats().loader.modules;
assert.equal(defaultModules, 0);
assert.equal(direct.jitStats().staticT0.supported, false);
assert.throws(() => direct.network.receive(new Uint8Array(14)), /external mode/);
const modulesBeforeDirectReset = direct.jitStats().loader.modules;
await direct.reset();
assert.equal(direct.jitStats().loader.modules, modulesBeforeDirectReset);
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
