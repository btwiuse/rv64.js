#!/usr/bin/env node
// Structured full-system boot profiler. Image loading is timed separately from
// execution so network/storage changes cannot be mistaken for emulator wins.
//
//   node tests/boot-profile.mjs fast --reps 3
//   node --max-old-space-size=2048 tests/boot-profile.mjs modern --reps 5
//   node tests/boot-profile.mjs modern --out target/boot-modern.json

import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";
import { RV64Debug as RV64 } from "../web/rv64.js";
import { BootTimeline, summarizeTrials } from "./boot-profile-lib.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const presetName = args.shift() || "modern";
let repetitions = 1;
let outputPath;
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--reps") repetitions = Number(args[++i]);
  else if (args[i] === "--out") outputPath = resolve(args[++i]);
  else throw new Error(`unknown argument: ${args[i]}`);
}
if (!Number.isInteger(repetitions) || repetitions < 1) {
  throw new Error("--reps must be a positive integer");
}

const presets = {
  fast: {
    files: [
      "web/images/bbl64.bin",
      "web/images/kernel-riscv64.bin",
      "web/images/root-riscv64.bin",
    ],
    markers: {
      firstOutput: /./s,
      kernel: /Linux version/,
      rootMounted: /VFS: Mounted root/,
      ready: /~ #/,
    },
    boot(vm, [bios, kernel, disk]) {
      vm.bootLinux({ bios, kernel, disk });
    },
    run(vm) { return vm.runSystem(3_000_000n); },
    instructions(vm) { return vm.sysInsnCount(); },
  },
  modern: {
    files: [
      "web/images/alpine/opensbi.bin",
      process.env.RV64_MODERN_KERNEL || "web/images/alpine/Image",
      "web/images/alpine/debian.ext4",
    ],
    markers: {
      firstOutput: /./s,
      firmware: /OpenSBI v/,
      firmwareComplete: /Boot HART MEDELEG/,
      kernel: /Linux version/,
      rootMounted: /VFS: Mounted root/,
      ready: /BENCH_READY/,
    },
    boot(vm, [opensbi, kernel, disk]) {
      vm.bootVirtLinux({
        opensbi,
        kernel,
        disk,
        ramMB: 512,
        cmdline: "console=ttyS0 root=/dev/vda rw init=/binit.sh",
      });
    },
    run(vm) { return vm.runVirtSystem(2_000_000n); },
    runBeforeKernel(vm) { return vm.runVirtSystem(100_000n); },
    instructions(vm) { return vm.virtInsnCount(); },
    pc(vm) { return vm.virtPc(); },
  },
};
presets["modern-direct"] = {
  files: [
    process.env.RV64_MODERN_KERNEL || "web/images/alpine/Image",
    "web/images/alpine/debian.ext4",
  ],
  markers: {
    firstOutput: /./s,
    kernel: /Linux version/,
    rootMounted: /VFS: Mounted root/,
    ready: /BENCH_READY/,
  },
  boot(vm, [kernel, disk]) {
    vm.bootVirtLinuxDirect({
      kernel,
      disk,
      ramMB: 512,
      cmdline: "console=ttyS0 root=/dev/vda rw init=/binit.sh",
    });
  },
  run(vm) { return vm.runVirtSystem(2_000_000n); },
  instructions(vm) { return vm.virtInsnCount(); },
  pc(vm) { return vm.virtPc(); },
};
presets["modern-alpine"] = {
  files: [
    process.env.RV64_MODERN_KERNEL || "web/images/alpine/Image",
    "web/images/alpine/alpine.ext4",
  ],
  markers: {
    firstOutput: /./s,
    kernel: /Linux version/,
    rootMounted: /VFS: Mounted root/,
    ready: /ALPINE_READY/,
  },
  boot(vm, [kernel, disk]) {
    vm.bootVirtLinuxDirect({
      kernel,
      disk,
      ramMB: 512,
      cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
    });
  },
  run(vm) { return vm.runVirtSystem(2_000_000n); },
  instructions(vm) { return vm.virtInsnCount(); },
  pc(vm) { return vm.virtPc(); },
};
const preset = presets[presetName];
if (!preset) {
  throw new Error(
    "preset must be 'fast', 'modern', 'modern-direct', or 'modern-alpine'",
  );
}

const loadStarted = performance.now();
const [wasm, ...images] = await Promise.all([
  readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm")),
  ...preset.files.map((file) => readFile(resolve(root, file))),
]);
const assetLoadMs = performance.now() - loadStarted;
const decoder = new TextDecoder();
const trials = [];

for (let repetition = 1; repetition <= repetitions; repetition++) {
  const wasmStarted = performance.now();
  const vm = await RV64.create(wasm);
  const wasmCreateMs = performance.now() - wasmStarted;
  const timeline = new BootTimeline(preset.markers);
  vm.onWrite = (_fd, bytes) => {
    timeline.write(
      decoder.decode(bytes, { stream: true }),
      () => preset.instructions(vm),
    );
  };

  const buildStarted = performance.now();
  preset.boot(vm, images.map((image) => new Uint8Array(image)));
  const machineBuildMs = performance.now() - buildStarted;
  if (preset.pc && preset.pc(vm) >= 0x8020_0000n) {
    timeline.mark("kernelEntry", () => preset.instructions(vm));
  }
  const deadline = performance.now() + 180_000;
  let poweredOff = false;
  while (!timeline.reached("ready") && !poweredOff && performance.now() < deadline) {
    poweredOff = preset.runBeforeKernel && !timeline.reached("kernelEntry")
      ? preset.runBeforeKernel(vm)
      : preset.run(vm);
    if (
      preset.pc &&
      !timeline.reached("kernelEntry") &&
      preset.pc(vm) >= 0x8020_0000n
    ) {
      timeline.mark("kernelEntry", () => preset.instructions(vm));
    }
    await new Promise((resolveImmediate) => setImmediate(resolveImmediate));
  }

  const trial = {
    repetition,
    wasmCreateMs,
    machineBuildMs,
    poweredOff,
    ready: timeline.reached("ready"),
    milestones: timeline.milestones,
  };
  trials.push(trial);
  const ready = trial.milestones.ready;
  process.stderr.write(
    `[boot-profile] ${presetName} ${repetition}/${repetitions}: ` +
      (ready
        ? `${ready.elapsedMs.toFixed(1)} ms, ${ready.instructions} instructions\n`
        : `FAILED (${poweredOff ? "powered off" : "timeout"})\n`),
  );
  if (!trial.ready) break;
}

const report = {
  schema: 1,
  generatedAt: new Date().toISOString(),
  preset: presetName,
  repetitions,
  assets: {
    loadMs: assetLoadMs,
    wasmBytes: wasm.length,
    images: preset.files.map((file, index) => ({ file, bytes: images[index].length })),
  },
  trials,
  summary: summarizeTrials(trials),
};
const json = `${JSON.stringify(report, null, 2)}\n`;
if (outputPath) await writeFile(outputPath, json);
process.stdout.write(json);
if (trials.length !== repetitions || trials.some((trial) => !trial.ready)) {
  process.exitCode = 1;
}
