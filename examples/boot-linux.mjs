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
const vm = await RV64.create(wasm);
let output = "";
const decoder = new TextDecoder();
vm.onWrite = (_fd, data) => {
  process.stdout.write(data);
  output += decoder.decode(data, { stream: true });
};

let run;
let input;
if (preset === "fast") {
  const [bios, kernel, disk] = await Promise.all([
    bytes("web/images/bbl64.bin"),
    bytes("web/images/kernel-riscv64.bin"),
    bytes("web/images/root-riscv64.bin"),
  ]);
  vm.bootLinux({ bios, kernel, disk });
  run = () => vm.runSystem(3_000_000n);
  input = (data) => vm.consoleInput(data);
} else if (preset === "modern") {
  const [opensbi, kernel, disk] = await Promise.all([
    bytes("web/images/modern/opensbi.bin"),
    bytes("web/images/modern/Image"),
    bytes("web/images/modern/debian.ext4"),
  ]);
  vm.bootVirtLinux({
    opensbi,
    kernel,
    disk,
    ramMB: 512,
    cmdline: "console=ttyS0 root=/dev/vda rw init=/binit.sh",
  });
  run = () => vm.runVirtSystem(2_000_000n);
  input = (data) => vm.virtConsoleInput(data);
} else {
  throw new Error("preset must be 'fast' or 'modern'");
}

process.stdin.setRawMode?.(true);
process.stdin.resume();
process.stdin.on("data", (data) => input(new Uint8Array(data)));

let stopped = false;
process.stdin.on("end", () => { if (!until) stopped = true; });
while (!stopped && !run() && !(until && output.includes(until))) {
  await new Promise((resolve) => setImmediate(resolve));
}
if (until && !output.includes(until)) process.exitCode = 1;
