#!/usr/bin/env node
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { RV64 } from "../web/rv64.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const wasmPath = join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
const kernelPath = process.env.RV64_MODERN_KERNEL || join(root, "web/images/alpine/Image");
const diskPath = join(root, "web/images/alpine/alpine.ext4");
if (![wasmPath, kernelPath, diskPath].every(existsSync)) {
  console.log("SKIP Alpine boot (run web/prepare-images.sh first)");
  process.exit(0);
}

let output = "";
let sentApk = false;
let observedError;
const decoder = new TextDecoder();
const vm = await RV64.create({
  wasm: await readFile(wasmPath),
  memoryMB: 512,
  boot: {
    mode: "linux-direct",
    kernel: await readFile(kernelPath),
    disk: await readFile(diskPath),
    virtioConsole: true,
    cmdline: "console=hvc0 root=/dev/vda rw init=/rv64-init",
  },
  events: {
    console(bytes) {
      output += decoder.decode(bytes, { stream: true });
      if (process.env.RV64_BOOT_TRACE) process.stdout.write(bytes);
      if (!sentApk && output.includes("ALPINE_READY")) {
        sentApk = true;
        vm.console.send("apk update && echo APK_UPDATE_OK\n");
      }
    },
    error(error) { observedError = error; },
  },
});

assert.equal(vm.network.mode, "fetch");
assert.equal(vm.network.proxyURL, "http://10.0.2.2:8080");
await vm.start();
const deadline = performance.now() + 240_000;
while (vm.running && !output.includes("\r\nAPK_UPDATE_OK\r\n") && performance.now() < deadline) {
  await new Promise((resolveImmediate) => setImmediate(resolveImmediate));
}
await vm.stop();
await vm.destroy();

assert.ifError(observedError);
assert.match(output, /Linux version/);
assert.match(output, /ALPINE_READY/);
assert.match(output, /PROXY_CA_READY/);
assert.match(output, /OK: \d+ distinct packages available/);
assert.match(output, /\r\nAPK_UPDATE_OK\r\n/);
assert.doesNotMatch(output, /unexpected end of file/);
console.log("PASS Alpine direct boot and apk update through default HTTP proxy");
