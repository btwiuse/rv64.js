#!/usr/bin/env node
// Runnable Node example for both browser-demo Linux machine presets.
// Build rv64-wasm first, then prepare the selected images as documented in
// README.md. Ctrl-D stops the host; guest input is forwarded from stdin.

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { RV64 } from "../web/rv64.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const preset = process.argv[2] || "fast";
const until = process.env.RV64_UNTIL;
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const bytes = async (...parts) => new Uint8Array(await readFile(join(root, ...parts)));
let output = "";
const decoder = new TextDecoder();

let boot;
let memoryMB;
if (preset === "fast") {
  const [firmware, kernel, disk] = await Promise.all([
    bytes("web/images/bbl64.bin"),
    bytes("web/images/kernel-riscv64.bin"),
    bytes("web/images/root-riscv64.bin"),
  ]);
  boot = { mode: "firmware", firmware, kernel, disk };
  memoryMB = 128;
} else if (preset === "modern" || preset === "modern-direct") {
  const [kernel, disk] = await Promise.all([
    process.env.RV64_MODERN_KERNEL
      ? new Uint8Array(await readFile(process.env.RV64_MODERN_KERNEL))
      : bytes("web/images/modern/Image"),
    bytes("web/images/modern/alpine.ext4"),
  ]);
  boot = {
    mode: preset === "modern-direct" ? "linux-direct" : "firmware",
    kernel,
    disk,
    cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
  };
  if (preset === "modern") {
    boot.firmware = await bytes("web/images/modern/opensbi.bin");
  }
  memoryMB = 512;
} else {
  throw new Error("preset must be 'fast', 'modern', or 'modern-direct'");
}

const vm = await RV64.create({
  wasm,
  memoryMB,
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
