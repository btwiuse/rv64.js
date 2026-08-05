#!/usr/bin/env node

import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { RV64Debug as RV64 } from "../web/rv64.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const paths = [
  "target/wasm32-unknown-unknown/release/rv64_wasm.wasm",
  "web/images/bbl64.bin",
  "web/images/kernel-riscv64.bin",
  "web/images/root-riscv64.bin",
].map((path) => join(root, path));

if (!paths.every(existsSync)) {
  console.log("SKIP TinyEMU config extraction (legacy images are missing)");
  process.exit(0);
}

const [wasm, bios, kernel, disk] = await Promise.all(
  paths.map((path) => readFile(path)),
);
const vm = await RV64.create(wasm);
const decoder = new TextDecoder();
const encoder = new TextEncoder();
let output = "";
vm.onWrite = (_fd, bytes) => {
  output += decoder.decode(bytes, { stream: true });
};
vm.bootLinux({ bios, kernel, disk });

const runUntil = (marker, maxSlices) => {
  for (let i = 0; i < maxSlices; i++) {
    vm.runSystem(3_000_000n);
    if (output.includes(marker)) return true;
  }
  return false;
};

if (!runUntil("~ #", 1000)) {
  throw new Error(`TinyEMU guest did not reach a shell:\n${output.slice(-4000)}`);
}

output = "";
vm.consoleInput(encoder.encode(
  "echo RV64_CONFIG_BEGIN; " +
  "if test -r /proc/config.gz; then zcat /proc/config.gz; " +
  "else echo RV64_CONFIG_MISSING; uname -a; ls -l /proc/config.gz /boot/config* 2>&1; fi; " +
  "echo RV64_CONFIG_END\n",
));
if (!runUntil("RV64_CONFIG_END", 1000)) {
  throw new Error(`TinyEMU config query timed out:\n${output.slice(-4000)}`);
}

const begin = output.indexOf("RV64_CONFIG_BEGIN");
const end = output.lastIndexOf("RV64_CONFIG_END");
const body = output.slice(begin + "RV64_CONFIG_BEGIN".length, end)
  .replace(/^\r?\n/, "")
  .replace(/\r/g, "");
const destination = join(root, "target/tinyemu-kernel.config");
await writeFile(destination, body);
console.log(`wrote ${destination} (${body.length} bytes)`);
console.log(body.includes("RV64_CONFIG_MISSING") ? body : "TinyEMU kernel config extracted");
