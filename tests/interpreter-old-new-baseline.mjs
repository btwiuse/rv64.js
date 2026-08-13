#!/usr/bin/env node
// Fast structural/performance baseline for the exact RV64 cores packaged by
// `make -C integrations/wanix comparison`. Each side gets a fresh runtime
// process, pair order alternates, and every JIT activity counter must stay zero.

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const self = fileURLToPath(import.meta.url);

function argument(name, fallback) {
  const prefix = `--${name}=`;
  return process.argv.find((value) => value.startsWith(prefix))?.slice(prefix.length) ?? fallback;
}

const variants = {
  legacy: join(root, "integrations/wanix/dist/rv64-legacy"),
  rewrite: join(root, "integrations/wanix/dist/rv64-jit"),
};

const warmup = BigInt(argument("warmup", "10000000"));
const budget = BigInt(argument("budget", "100000000"));
const slice = BigInt(argument("slice", "2000000"));
if (warmup < 0n || budget <= 0n || slice <= 0n) {
  throw new Error("warmup must be non-negative; budget and slice must be positive");
}

async function runWorker(side) {
  const base = variants[side];
  if (!base) throw new Error(`unknown side: ${side}`);
  const loaderPath = join(base, "rv64.js");
  const wasmPath = join(base, "rv64_wasm.wasm");
  const [loader, wasm] = await Promise.all([
    import(pathToFileURL(loaderPath)),
    readFile(wasmPath),
  ]);
  const initialized = performance.now();
  const vm = await loader.RV64Debug.create(wasm);
  const initMs = performance.now() - initialized;
  vm.onWrite = () => {};
  vm.ex.jit_set_enabled?.(0);
  vm.bootVirtLinuxDirect({
    // jal x0, 0: the smallest indefinitely executable modern-Virt kernel.
    kernel: new Uint8Array([0x6f, 0, 0, 0]),
    ramMB: 32,
  });

  if (warmup) vm.runVirtSystem(warmup);
  const before = vm.virtInsnCount();
  let remaining = budget;
  const started = performance.now();
  while (remaining) {
    const quantum = remaining < slice ? remaining : slice;
    if (vm.runVirtSystem(quantum)) throw new Error(`${side} unexpectedly powered off`);
    remaining -= quantum;
  }
  const elapsedMs = performance.now() - started;
  const instructions = vm.virtInsnCount() - before;
  const activity = {
    retired: vm.ex.jit_stat?.(0) ?? 0n,
    dispatches: vm.ex.jit_stat?.(1) ?? 0n,
    entries: vm.ex.jit_stat?.(3) ?? 0n,
    fallbackSlices: vm.ex.jit_stat?.(4) ?? 0n,
    translationAttempts: vm.ex.jit_stat?.(77) ?? 0n,
    registrations: BigInt(vm.jitRegCount ?? 0),
  };
  if (instructions !== budget) {
    throw new Error(`${side} retired ${instructions}, expected ${budget}`);
  }
  for (const [name, value] of Object.entries(activity)) {
    if (value !== 0n) throw new Error(`${side} ${name} was ${value}, expected zero`);
  }
  return {
    side,
    initMs,
    elapsedMs,
    mips: Number(instructions) / elapsedMs / 1000,
    instructions: instructions.toString(),
    activity: Object.fromEntries(
      Object.entries(activity).map(([name, value]) => [name, value.toString()]),
    ),
    artifacts: {
      loader: createHash("sha256").update(await readFile(loaderPath)).digest("hex"),
      wasm: createHash("sha256").update(wasm).digest("hex"),
    },
  };
}

if (process.argv.includes("--worker")) {
  console.log(JSON.stringify(await runWorker(argument("side", ""))));
} else {
  const samples = Number(argument("samples", "3"));
  if (!Number.isInteger(samples) || samples < 1 || samples > 15) {
    throw new Error("samples must be an integer from 1 through 15");
  }
  const pairs = [];
  for (let sample = 0; sample < samples; sample++) {
    const pair = { sample: sample + 1 };
    const order = sample % 2 ? ["rewrite", "legacy"] : ["legacy", "rewrite"];
    pair.order = order;
    for (const side of order) {
      const child = spawnSync(process.execPath, [
        self, "--worker", `--side=${side}`,
        `--warmup=${warmup}`, `--budget=${budget}`, `--slice=${slice}`,
      ], { cwd: root, encoding: "utf8", maxBuffer: 1 << 20 });
      if (child.status !== 0) {
        throw new Error(child.stderr || child.stdout || `${side} worker exited ${child.status}`);
      }
      pair[side] = JSON.parse(child.stdout);
    }
    pair.rewriteOverLegacy = pair.rewrite.mips / pair.legacy.mips;
    pairs.push(pair);
  }
  const sorted = pairs.map((pair) => pair.rewriteOverLegacy).sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  const median = sorted.length % 2
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
  console.log(JSON.stringify({
    schema: 1,
    environment: { node: process.version, v8: process.versions.v8 },
    method: { samples, warmup: warmup.toString(), budget: budget.toString(), slice: slice.toString() },
    rewriteOverLegacyMedian: median,
    pairs,
  }, null, 2));
}
