#!/usr/bin/env node
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { Worker as NodeWorker } from "node:worker_threads";
import { fileURLToPath } from "node:url";
import { RV64 } from "../web/rv64.js";

const bootstrap = new URL("./web-worker-node-bootstrap.mjs", import.meta.url);
class BrowserWorker {
  constructor(target) {
    this.worker = new NodeWorker(bootstrap, {
      workerData: { target: new URL(target).href },
    });
    this.worker.on("message", (data) => this.onmessage?.({ data }));
    this.worker.on("messageerror", () => this.onmessageerror?.());
    this.worker.on("error", (error) => this.onerror?.({ error, message: error.message }));
  }
  postMessage(message, transfers) {
    this.worker.postMessage(message, transfers);
  }
  terminate() {
    void this.worker.terminate();
  }
}
globalThis.Worker = BrowserWorker;

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const wasm = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);
const events = [];
const vm = await RV64.create({
  wasm,
  memoryMB: 1,
  execution: { mode: "worker", statisticsIntervalMs: 50 },
  boot: {
    mode: "bare-metal",
    image: new Uint8Array([0x73, 0x00, 0x10, 0x00]),
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
assert.deepEqual(events, ["ready"]);
await vm.start();
while (vm.running) await new Promise((resolve) => setTimeout(resolve, 5));
await new Promise((resolve) => setTimeout(resolve, 60));
assert.deepEqual(events, ["ready", "start", "stop:powered-off"]);
assert.ok(vm.instructions > 0n);

await vm.reset();
await new Promise((resolve) => setTimeout(resolve, 60));
assert.equal(vm.instructions, 0n);
await vm.destroy();
await vm.destroy();
assert.throws(() => vm.instructions, /destroyed/);

await assert.rejects(
  RV64.create({
    wasm: new Response(wasm),
    execution: { mode: "worker" },
    boot: {
      mode: "bare-metal",
      image: new Uint8Array(4),
      loadAddress: 0x80000000n,
    },
  }),
  /Response in worker execution mode/,
);

console.log("PASS Worker public API lifecycle");
