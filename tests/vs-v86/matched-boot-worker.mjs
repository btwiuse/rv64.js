#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const artifacts = resolve(process.env.ARTIFACTS || join(root, "target/bench"));
const side = process.argv[2];
if (side !== "rv64" && side !== "v86") throw new Error("side must be rv64 or v86");
const jit = process.env.DISABLE_JIT !== "1";

const decoder = new TextDecoder();
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const milestones = {};
let serial = "";
let started;
const observe = (bytes) => {
  serial += decoder.decode(bytes, { stream: true });
  const elapsedMs = performance.now() - started;
  if (milestones.firmware === undefined && /OpenSBI v/.test(serial)) {
    milestones.firmware = elapsedMs;
  }
  if (milestones.kernel === undefined && /Linux version/.test(serial)) {
    milestones.kernel = elapsedMs;
  }
  if (milestones.root === undefined && /MATCHED_ROOT_READY/.test(serial)) {
    milestones.root = elapsedMs;
  }
  if (milestones.ready === undefined && /ALPINE_READY/.test(serial)) {
    milestones.ready = elapsedMs;
  }
};

async function rv64() {
  const loadStarted = performance.now();
  const [wasm, opensbi, kernel, initrd] = await Promise.all([
    readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm")),
    readFile(join(artifacts, "matched-opensbi.bin")),
    readFile(join(artifacts, "matched-linux-rv64-Image")),
    readFile(join(artifacts, "matched-alpine-riscv64.cpio")),
  ]);
  const loadMs = performance.now() - loadStarted;
  const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
  const createStarted = performance.now();
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.onWrite = (_fd, bytes) => observe(bytes);
  vm.bootVirtLinux({
    opensbi,
    kernel,
    initrd,
    ramMB: 512,
    cmdline: "console=ttyS0 rdinit=/init",
  });
  const createMs = performance.now() - createStarted;
  started = performance.now();
  const deadline = started + 120_000;
  while (milestones.ready === undefined && performance.now() < deadline) {
    vm.runVirtSystem(2_000_000n);
  }
  const instructions = vm.virtInsnCount().toString();
  if (milestones.ready === undefined) {
    throw new Error(`rv64 boot timeout:\n${serial.slice(-4000)}`);
  }
  return {
    side,
    jit,
    loadMs,
    createMs,
    milestones,
    instructions,
    wasmSha256: sha256(wasm),
    inputSha256: {
      firmware: sha256(opensbi),
      kernel: sha256(kernel),
      initramfs: sha256(initrd),
    },
  };
}

const exactBuffer = (bytes) => bytes.buffer.slice(
  bytes.byteOffset,
  bytes.byteOffset + bytes.byteLength,
);

async function v86() {
  const v86dir = resolve(process.env.V86DIR || join(artifacts, "v86"));
  process.chdir(v86dir);
  const loadStarted = performance.now();
  const [bios, vgaBios, kernel, initrd] = await Promise.all([
    readFile(join(v86dir, "bios/seabios.bin")),
    readFile(join(v86dir, "bios/vgabios.bin")),
    readFile(join(artifacts, "matched-linux-x86-bzImage")),
    readFile(join(artifacts, "matched-alpine-x86.cpio")),
  ]);
  const loadMs = performance.now() - loadStarted;
  const { V86 } = await import(pathToFileURL(join(v86dir, "src/main.js")));
  const createStarted = performance.now();
  const emulator = new V86({
    bios: { buffer: exactBuffer(bios) },
    vga_bios: { buffer: exactBuffer(vgaBios) },
    bzimage: { buffer: exactBuffer(kernel) },
    initrd: { buffer: exactBuffer(initrd) },
    cmdline: "console=ttyS0 rdinit=/init mitigations=off tsc=reliable",
    autostart: false,
    memory_size: 512 * 1024 * 1024,
    disable_jit: jit ? 0 : 1,
    log_level: 0,
  });
  emulator.add_listener("serial0-output-byte", (byte) => {
    observe(Uint8Array.of(byte));
  });
  await new Promise((resolveReady) => emulator.add_listener("emulator-ready", resolveReady));
  const createMs = performance.now() - createStarted;
  started = performance.now();
  emulator.run();
  await new Promise((resolveDone, reject) => {
    const poll = setInterval(() => {
      if (milestones.ready !== undefined) {
        clearInterval(poll);
        clearTimeout(timeout);
        resolveDone();
      }
    }, 5);
    const timeout = setTimeout(() => {
      clearInterval(poll);
      reject(new Error(`v86 boot timeout:\n${serial.slice(-4000)}`));
    }, 120_000);
  });
  emulator.destroy();
  return {
    side,
    jit,
    loadMs,
    createMs,
    milestones,
    inputSha256: {
      firmware: sha256(bios),
      vgaFirmware: sha256(vgaBios),
      kernel: sha256(kernel),
      initramfs: sha256(initrd),
    },
  };
}

console.log(JSON.stringify(await (side === "rv64" ? rv64() : v86())));
