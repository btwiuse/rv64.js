#!/usr/bin/env node
// Runnable Node example for the browser-demo Alpine Linux machine.
// Build rv64-wasm first, then prepare the images as documented in
// README.md. Ctrl-D stops the host; guest input is forwarded from stdin.

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { RV64 } from "../web/rv64.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const until = process.env.RV64_UNTIL;
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const bytes = async (...parts) => new Uint8Array(await readFile(join(root, ...parts)));
let output = "";
const decoder = new TextDecoder();

const [kernel, disk] = await Promise.all([
  process.env.RV64_KERNEL
    ? new Uint8Array(await readFile(process.env.RV64_KERNEL))
    : bytes("web/images/alpine/Image"),
  bytes("web/images/alpine/alpine.ext4"),
]);
const boot = {
  mode: "linux-direct",
  kernel,
  disk,
  cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
};

const vm = await RV64.create({
  wasm,
  memoryMB: 512,
  boot,
  events: {
    console(data) {
      process.stdout.write(data);
      output += decoder.decode(data, { stream: true });
    },
  },
});

process.stdin.setRawMode?.(true);
process.stdin.resume();
process.stdin.on("data", (data) => vm.console.send(new Uint8Array(data)));

let stopped = false;
process.stdin.on("end", () => { if (!until) stopped = true; });
await vm.start();
while (!stopped && vm.running && !(until && output.includes(until))) {
  await new Promise((resolve) => setImmediate(resolve));
}
await vm.stop();
await vm.destroy();
if (until && !output.includes(until)) process.exitCode = 1;
