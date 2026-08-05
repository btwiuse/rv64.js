#!/usr/bin/env node
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { RV64Debug } from "../web/rv64.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const useOpenSbi = process.argv.includes("--opensbi");
const paths = {
  wasm: join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  kernel: process.env.RV64_MODERN_KERNEL || join(root, "web/images/modern/Image"),
  disk: join(root, "web/images/modern/alpine.ext4"),
  ...(useOpenSbi ? { opensbi: join(root, "web/images/modern/opensbi.bin") } : {}),
};
if (!Object.values(paths).every(existsSync)) {
  console.log("SKIP SBI/apk profile (run web/prepare-modern-images.sh first)");
  process.exit(0);
}

const bytes = async (path) => new Uint8Array(await readFile(path));
const [wasm, kernel, disk, opensbi] = await Promise.all([
  bytes(paths.wasm),
  bytes(paths.kernel),
  bytes(paths.disk),
  useOpenSbi ? bytes(paths.opensbi) : undefined,
]);
const vm = await RV64Debug.create(wasm);
let output = "";
let apkStartedAt;
let apkStartedInsns;
let apkStartedSbi;
let apkFinishedAt;
let lastNetworkEndAt;
let downloaded = 0;
let requests = 0;
const decoder = new TextDecoder();

vm.onWrite = (_fd, bytes) => {
  output += decoder.decode(bytes, { stream: true });
  if (apkStartedAt === undefined && output.includes("ALPINE_READY")) {
    apkStartedAt = performance.now();
    apkStartedInsns = vm.virtInsnCount();
    apkStartedSbi = vm.virtSbiCallCounts();
    vm.virtConsoleInput(new TextEncoder().encode("apk update --no-progress && echo APK_PROFILE_DONE\n"));
  }
  if (output.includes("\r\nAPK_PROFILE_DONE\r\n")) {
    apkFinishedAt ??= performance.now();
  }
};
vm.onNetworkTraffic = (detail) => {
  if (apkStartedAt === undefined) return;
  if (detail.type === "request") requests++;
  if (detail.type === "download") downloaded += detail.bytes;
  if (detail.type === "end") lastNetworkEndAt = performance.now();
};
const bootOptions = {
  kernel,
  disk,
  cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init rv64.network=fetch",
  ramMB: 512,
  net: true,
  proxy: true,
};
if (useOpenSbi) vm.bootVirtLinux({ ...bootOptions, opensbi });
else vm.bootVirtLinuxDirect(bootOptions);

const deadline = performance.now() + 240_000;
while (apkFinishedAt === undefined && performance.now() < deadline) {
  if (vm.runVirtSystem(2_000_000n)) break;
  await new Promise((resolve) => setImmediate(resolve));
}
if (apkFinishedAt === undefined) {
  throw new Error(`apk profile timed out\n${output.slice(-4000)}`);
}

const finishedSbi = vm.virtSbiCallCounts();
const sbiDelta = Object.fromEntries(
  Object.keys(finishedSbi).map((name) => [
    name,
    Number(finishedSbi[name] - apkStartedSbi[name]),
  ]),
);
const totalMs = apkFinishedAt - apkStartedAt;
const networkMs = lastNetworkEndAt - apkStartedAt;
const processingMs = apkFinishedAt - lastNetworkEndAt;
console.log(JSON.stringify({
  mode: useOpenSbi ? "opensbi" : "direct",
  requests,
  downloaded,
  totalMs,
  networkMs,
  processingMs,
  retiredInstructions: Number(vm.virtInsnCount() - apkStartedInsns),
  sbiCalls: sbiDelta,
}, null, 2));
