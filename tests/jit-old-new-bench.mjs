#!/usr/bin/env node
// Black-box comparison of the previous and clean-room RV64-to-Wasm JITs.
//
// Both variants are built from the same upstream commit. Every headline trial
// is a fresh Node process and pair order alternates. Generated-module capture
// is a separate, untimed run; exact captured bytes are then compiled and
// instantiated in additional fresh processes. The previous compiler source is
// never imported into or inspected by this harness.

import { createHash } from "node:crypto";
import {
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import { acquireBenchmarkLock } from "./vs-v86/bench-lock.mjs";
import { alternatingOrder, cpuProbe } from "./vs-v86/bench-math.mjs";
import { summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = join(here, "..");
const self = fileURLToPath(import.meta.url);
const tick = () => new Promise((resolveTick) => setImmediate(resolveTick));

function argument(name, fallback) {
  const prefix = `--${name}=`;
  return process.argv.find((value) => value.startsWith(prefix))?.slice(prefix.length) ?? fallback;
}

function absolute(path, base = repositoryRoot) {
  return isAbsolute(path) ? path : resolve(base, path);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function fileIdentity(path) {
  const bytes = readFileSync(path);
  return { path, bytes: bytes.length, sha256: sha256(bytes) };
}

function jsonSafe(value) {
  if (typeof value === "bigint") return value.toString();
  if (Array.isArray(value)) return value.map(jsonSafe);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, entry]) => [key, jsonSafe(entry)]));
  }
  return value;
}

function runtimeInfo() {
  return {
    execPath: process.execPath,
    node: process.version,
    v8: process.versions.v8 ?? null,
    bun: process.versions.bun ?? null,
    deno: process.versions.deno ?? null,
    webkit: process.versions.webkit ?? null,
  };
}

function jitStats(vm) {
  const stat = (index) => vm.ex.jit_stat ? vm.ex.jit_stat(index).toString() : null;
  return {
    retired: stat(0),
    dispatches: stat(1),
    singleEntries: stat(2),
    systemEntries: stat(3),
    regionIssued: stat(12),
    regionLanded: stat(13),
    pendingRegions: stat(17),
    batchCount: stat(43),
    batchMembers: stat(44),
    regionCalls: stat(48),
    regionInsns: stat(49),
    loader: {
      registrations: vm.jitRegCount ?? 0,
      bytes: vm.jitRegBytes ?? 0,
      compileLikeMs: vm.jitRegMs ?? 0,
      totalMs: vm.jitRegTotalMs ?? 0,
      blocks: vm.jitBlocks ?? 0,
      batches: vm.jitBatches ?? 0,
    },
    lifecycle: jsonSafe(vm.jitLifecycleStats?.() ?? null),
  };
}

function installModuleCapture(outputDirectory) {
  mkdirSync(outputDirectory, { recursive: true });
  const originalModule = WebAssembly.Module;
  const originalCompile = WebAssembly.compile;
  const records = [];
  let active = true;

  const record = (source, api) => {
    if (!active) return;
    const view = source instanceof ArrayBuffer
      ? new Uint8Array(source)
      : new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
    const bytes = Buffer.from(view);
    const hash = sha256(bytes);
    const file = `${String(records.length).padStart(5, "0")}-${hash.slice(0, 16)}.wasm`;
    writeFileSync(join(outputDirectory, file), bytes);
    records.push({ sequence: records.length, api, file, bytes: bytes.length, sha256: hash });
  };

  WebAssembly.Module = new Proxy(originalModule, {
    construct(target, args, newTarget) {
      record(args[0], "Module");
      return Reflect.construct(target, args, newTarget);
    },
  });
  WebAssembly.compile = function (...args) {
    record(args[0], "compile");
    return Reflect.apply(originalCompile, this, args);
  };

  return {
    finish(metadata) {
      active = false;
      WebAssembly.Module = originalModule;
      WebAssembly.compile = originalCompile;
      const manifest = {
        schema: 1,
        metadata,
        moduleCount: records.length,
        totalBytes: records.reduce((total, recordEntry) => total + recordEntry.bytes, 0),
        modules: records,
      };
      const manifestPath = join(outputDirectory, "manifest.json");
      writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
      return {
        manifestPath,
        schema: manifest.schema,
        metadata: manifest.metadata,
        moduleCount: manifest.moduleCount,
        totalBytes: manifest.totalBytes,
      };
    },
  };
}

async function settlePendingRegions(vm, timeoutMs = 15_000) {
  if (!vm.ex.sys_pending_builds) return true;
  const deadline = performance.now() + timeoutMs;
  while (vm.ex.sys_pending_builds() !== 0 && performance.now() < deadline) await tick();
  return vm.ex.sys_pending_builds() === 0;
}

async function loadVariant(variantRoot) {
  const loaderPath = join(variantRoot, "web/rv64.js");
  const wasmPath = join(
    variantRoot,
    "target/wasm32-unknown-unknown/release/rv64_wasm.wasm",
  );
  const loader = await import(pathToFileURL(loaderPath).href);
  const wasm = readFileSync(wasmPath);
  return {
    ...loader,
    wasm,
    identities: {
      loader: fileIdentity(loaderPath),
      wasm: fileIdentity(wasmPath),
    },
  };
}

async function runUserWorker({
  variant,
  variantRoot,
  artifacts,
  workload,
  captureDirectory,
  userQuantum,
}) {
  const file = workload === "user-alu" ? "alu.rv64" : "rvbench_fs.rv64";
  const elfPath = join(artifacts, "xbench", file);
  const elf = readFileSync(elfPath);
  const loaded = await loadVariant(variantRoot);
  const initStarted = performance.now();
  const vm = await loaded.RV64Debug.create(loaded.wasm);
  const coreInitMs = performance.now() - initStarted;
  vm.ex.jit_set_enabled(1);
  vm.onWrite = (_fd, bytes) => {
    output += new TextDecoder().decode(bytes);
  };
  const capture = captureDirectory ? installModuleCapture(captureDirectory) : null;
  let output = "";
  vm.loadElf(new Uint8Array(elf), [file, "benchmark"]);
  const runStarted = performance.now();
  let stop;
  let calls = 0;
  do {
    stop = vm.runUser(userQuantum);
    calls++;
  } while (stop !== loaded.Stop.EXITED && calls < 1_000_000);
  const runMs = performance.now() - runStarted;
  const insns = vm.userInsnCount();
  if (stop !== loaded.Stop.EXITED || vm.userExitCode() !== 0) {
    throw new Error(`${variant}/${workload} did not exit cleanly: stop=${stop} exit=${vm.userExitCode()}`);
  }
  const captureResult = capture?.finish({ variant, workload });
  const stats = jitStats(vm);
  return {
    schema: 1,
    variant,
    workload,
    engine: runtimeInfo(),
    identities: { ...loaded.identities, input: fileIdentity(elfPath) },
    timing: {
      coreInitMs,
      runMs,
      minsnPerSec: Number(insns) / runMs / 1000,
    },
    result: {
      exited: true,
      exitCode: vm.userExitCode(),
      calls,
      runQuantum: userQuantum.toString(),
      insns: insns.toString(),
      outputSha256: sha256(output),
      outputTail: output.slice(-300),
    },
    jit: stats,
    coveragePercent: Number(BigInt(stats.retired ?? "0") * 10_000n / insns) / 100,
    capture: captureResult ?? null,
    process: {
      memoryUsage: process.memoryUsage(),
      resourceUsage: process.resourceUsage?.() ?? null,
    },
  };
}

async function pumpModern(vm, done, timeoutMs) {
  const started = performance.now();
  for (let iteration = 0; !done(); iteration++) {
    if (vm.runVirtSystem(2_000_000n)) return done();
    const pending = vm.ex.sys_pending_builds?.() ?? 0;
    if (pending !== 0 || (iteration & 15) === 0) await tick();
    if (performance.now() - started > timeoutMs) return false;
  }
  return true;
}

async function pumpLegacy(vm, done, timeoutMs) {
  const started = performance.now();
  for (let iteration = 0; !done(); iteration++) {
    if (vm.runSystem(5_000_000n)) return done();
    const pending = vm.ex.sys_pending_builds?.() ?? 0;
    if (pending !== 0 || (iteration & 15) === 0) await tick();
    if (performance.now() - started > timeoutMs) return false;
  }
  return true;
}

async function runLegacyWorker({
  variant,
  variantRoot,
  legacyImagesDirectory,
  workload,
  captureDirectory,
}) {
  const [bios, kernel, disk] = ["bbl64.bin", "kernel-riscv64.bin", "root-riscv64.bin"]
    .map((file) => readFileSync(join(legacyImagesDirectory, file)));
  const loaded = await loadVariant(variantRoot);
  const initStarted = performance.now();
  const vm = await loaded.RV64Debug.create(loaded.wasm);
  const coreInitMs = performance.now() - initStarted;
  let output = "";
  const decoder = new TextDecoder();
  vm.onWrite = (_fd, bytes) => {
    output += decoder.decode(bytes, { stream: true });
  };
  vm.ex.jit_set_enabled(1);
  vm.ex.sys_set_superblock(1);
  vm.ex.jit_set_sb_spacing?.(0);
  const capture = captureDirectory ? installModuleCapture(captureDirectory) : null;
  const stageStarted = performance.now();
  vm.bootLinux({
    bios: new Uint8Array(bios),
    kernel: new Uint8Array(kernel),
    disk: new Uint8Array(disk),
    ramMB: 128,
  });
  const stageMs = performance.now() - stageStarted;
  const bootRunStarted = performance.now();
  const ready = await pumpLegacy(vm, () => output.includes("~ #"), 180_000);
  const bootRunMs = performance.now() - bootRunStarted;
  if (!ready) {
    throw new Error(
      `${variant}/${workload} boot timeout pc=0x${vm.ex.sys_pc().toString(16)} ` +
      `insns=${vm.sysInsnCount()} tail=${JSON.stringify(output.slice(-300))}`,
    );
  }

  const beforeInsns = vm.sysInsnCount();
  const beforeJit = vm.ex.jit_stat(0);
  const beforeDispatches = vm.ex.jit_stat(1);
  const outputStart = output.length;
  vm.consoleInput(new TextEncoder().encode(
    "dd if=/dev/zero bs=1k count=4096 2>/dev/null | md5sum\n",
  ));
  const workStarted = performance.now();
  const complete = await pumpLegacy(
    vm,
    () => /[0-9a-f]{32}/.test(output.slice(outputStart)),
    180_000,
  );
  const workMs = performance.now() - workStarted;
  if (!complete) {
    throw new Error(
      `${variant}/${workload} work timeout pc=0x${vm.ex.sys_pc().toString(16)} ` +
      `insns=${vm.sysInsnCount()} tail=${JSON.stringify(output.slice(-300))}`,
    );
  }
  const workOutput = output.slice(outputStart);
  const digest = workOutput.match(/[0-9a-f]{32}/)?.[0] ?? null;
  const workInsns = vm.sysInsnCount() - beforeInsns;
  const workJitInsns = vm.ex.jit_stat(0) - beforeJit;
  const workDispatches = vm.ex.jit_stat(1) - beforeDispatches;
  const pendingSettled = await settlePendingRegions(vm);
  const captureResult = capture?.finish({ variant, workload, mode: "legacy" });
  const stats = jitStats(vm);
  return {
    schema: 1,
    variant,
    workload,
    mode: "legacy",
    engine: runtimeInfo(),
    identities: {
      ...loaded.identities,
      bios: fileIdentity(join(legacyImagesDirectory, "bbl64.bin")),
      kernel: fileIdentity(join(legacyImagesDirectory, "kernel-riscv64.bin")),
      disk: fileIdentity(join(legacyImagesDirectory, "root-riscv64.bin")),
    },
    timing: {
      coreInitMs,
      stageMs,
      bootRunMs,
      coldToReadyMs: coreInitMs + stageMs + bootRunMs,
      workMs,
      workMInsnPerSec: Number(workInsns) / workMs / 1000,
    },
    result: {
      ready,
      complete,
      digest,
      totalInsns: vm.sysInsnCount().toString(),
      workInsns: workInsns.toString(),
      workJitInsns: workJitInsns.toString(),
      workDispatches: workDispatches.toString(),
      workOutputSha256: sha256(workOutput),
      outputTail: output.slice(-300),
      pendingSettled,
    },
    jit: stats,
    workCoveragePercent: Number(workJitInsns * 10_000n / workInsns) / 100,
    workInsnsPerDispatch: Number(workInsns / (workDispatches || 1n)),
    capture: captureResult ?? null,
    process: {
      memoryUsage: process.memoryUsage(),
      resourceUsage: process.resourceUsage?.() ?? null,
    },
  };
}

async function runModernWorker({
  variant,
  variantRoot,
  imagesDirectory,
  workload,
  captureDirectory,
}) {
  const mode = workload === "modern-opensbi" ? "opensbi" : "direct";
  const [kernel, disk, opensbi] = ["Image", "alpine.ext4", "opensbi.bin"]
    .map((file) => readFileSync(join(imagesDirectory, file)));
  const loaded = await loadVariant(variantRoot);
  const initStarted = performance.now();
  const vm = await loaded.RV64Debug.create(loaded.wasm);
  const coreInitMs = performance.now() - initStarted;
  let output = "";
  const decoder = new TextDecoder();
  vm.onWrite = (_fd, bytes) => {
    output += decoder.decode(bytes, { stream: true });
  };
  vm.ex.jit_set_enabled(1);
  vm.ex.sys_set_superblock(1);
  vm.ex.jit_set_sb_spacing?.(0);
  const capture = captureDirectory ? installModuleCapture(captureDirectory) : null;

  const bootOptions = {
    kernel: new Uint8Array(kernel),
    disk: new Uint8Array(disk),
    cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
    ramMB: 512,
  };
  const stageStarted = performance.now();
  if (mode === "opensbi") {
    vm.bootVirtLinux({ ...bootOptions, opensbi: new Uint8Array(opensbi) });
  } else {
    vm.bootVirtLinuxDirect(bootOptions);
  }
  const stageMs = performance.now() - stageStarted;
  const bootRunStarted = performance.now();
  const ready = await pumpModern(vm, () => output.includes("ALPINE_READY"), 180_000);
  const bootRunMs = performance.now() - bootRunStarted;
  if (!ready) {
    throw new Error(
      `${variant}/${workload} boot timeout pc=0x${vm.virtPc().toString(16)} ` +
      `insns=${vm.virtInsnCount()} tail=${JSON.stringify(output.slice(-300))}`,
    );
  }

  const beforeInsns = vm.virtInsnCount();
  const beforeJit = vm.ex.jit_stat(0);
  const beforeDispatches = vm.ex.jit_stat(1);
  const marker = "OLD_NEW_200010000";
  const command =
    "i=0; s=0; while [ $i -lt 20000 ]; do i=$((i+1)); s=$((s+i)); done; " +
    "echo OLD_NEW_${s}";
  const outputStart = output.length;
  vm.virtConsoleInput(new TextEncoder().encode(`${command}\n`));
  const workStarted = performance.now();
  const complete = await pumpModern(
    vm,
    () => output.slice(outputStart).includes(marker),
    180_000,
  );
  const workMs = performance.now() - workStarted;
  if (!complete) {
    throw new Error(
      `${variant}/${workload} work timeout pc=0x${vm.virtPc().toString(16)} ` +
      `insns=${vm.virtInsnCount()} tail=${JSON.stringify(output.slice(-300))}`,
    );
  }
  const workInsns = vm.virtInsnCount() - beforeInsns;
  const workJitInsns = vm.ex.jit_stat(0) - beforeJit;
  const workDispatches = vm.ex.jit_stat(1) - beforeDispatches;
  const pendingSettled = await settlePendingRegions(vm);
  const captureResult = capture?.finish({ variant, workload, mode });
  const stats = jitStats(vm);
  return {
    schema: 1,
    variant,
    workload,
    mode,
    engine: runtimeInfo(),
    identities: {
      ...loaded.identities,
      kernel: fileIdentity(join(imagesDirectory, "Image")),
      disk: fileIdentity(join(imagesDirectory, "alpine.ext4")),
      opensbi: mode === "opensbi" ? fileIdentity(join(imagesDirectory, "opensbi.bin")) : null,
    },
    timing: {
      coreInitMs,
      stageMs,
      bootRunMs,
      coldToReadyMs: coreInitMs + stageMs + bootRunMs,
      workMs,
      workMInsnPerSec: Number(workInsns) / workMs / 1000,
    },
    result: {
      ready,
      complete,
      marker,
      totalInsns: vm.virtInsnCount().toString(),
      workInsns: workInsns.toString(),
      workJitInsns: workJitInsns.toString(),
      workDispatches: workDispatches.toString(),
      workOutputSha256: sha256(output.slice(outputStart)),
      outputTail: output.slice(-300),
      pendingSettled,
      unsupportedSbiExtension: vm.ex.virt_unsupported_sbi_ext?.().toString() ?? null,
    },
    jit: stats,
    workCoveragePercent: Number(workJitInsns * 10_000n / workInsns) / 100,
    workInsnsPerDispatch: Number(workInsns / (workDispatches || 1n)),
    capture: captureResult ?? null,
    process: {
      memoryUsage: process.memoryUsage(),
      resourceUsage: process.resourceUsage?.() ?? null,
    },
  };
}

function frozenImports() {
  return {
    env: {
      memory: new WebAssembly.Memory({ initial: 1 }),
      tlb_fill: () => -1n,
      system_bulk_copy: () => 0n,
      fp_exec: () => 0n,
      user_reservation: () => 0,
      system_reservation: () => 0,
      chain_next: () => {},
      __indirect_function_table: new WebAssembly.Table({
        initial: 65_536,
        element: "anyfunc",
      }),
    },
  };
}

function runFrozenWorker(manifestPath) {
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const directory = dirname(manifestPath);
  const bytes = manifest.modules.map((module) => readFileSync(join(directory, module.file)));
  const compileStarted = performance.now();
  const modules = bytes.map((moduleBytes) => new WebAssembly.Module(moduleBytes));
  const compileMs = performance.now() - compileStarted;
  const imports = frozenImports();
  const instantiateStarted = performance.now();
  const instances = modules.map((module) => new WebAssembly.Instance(module, imports));
  const instantiateMs = performance.now() - instantiateStarted;
  process.stdout.write(JSON.stringify({
    engine: runtimeInfo(),
    moduleCount: modules.length,
    totalBytes: bytes.reduce((total, moduleBytes) => total + moduleBytes.length, 0),
    compileMs,
    instantiateMs,
    exportCount: instances.reduce(
      (total, instance) => total + Object.keys(instance.exports).length,
      0,
    ),
  }));
}

if (process.argv.includes("--worker")) {
  const variant = argument("variant");
  const workload = argument("workload");
  const variantRoot = absolute(argument("variant-root"));
  const artifacts = absolute(argument("artifacts"));
  const imagesDirectory = absolute(argument("images"));
  const legacyImagesDirectory = absolute(argument("legacy-images"));
  const captureDirectoryArg = argument("capture-directory", "");
  const captureDirectory = captureDirectoryArg ? absolute(captureDirectoryArg) : null;
  const options = {
    variant,
    variantRoot,
    artifacts,
    imagesDirectory,
    legacyImagesDirectory,
    workload,
    captureDirectory,
    userQuantum: BigInt(argument("user-quantum", "50000000000")),
  };
  const result = workload.startsWith("user-")
    ? await runUserWorker(options)
    : workload.startsWith("legacy-")
    ? await runLegacyWorker(options)
    : await runModernWorker(options);
  process.stdout.write(JSON.stringify(result));
  process.exit(0);
}

if (process.argv.includes("--frozen-worker")) {
  runFrozenWorker(absolute(argument("manifest")));
  process.exit(0);
}

const oldRoot = absolute(argument("old-root", "target/jit-old-new/previous"));
const newRoot = absolute(argument("new-root", repositoryRoot));
const artifacts = absolute(argument("artifacts", "target/bench"));
const imagesDirectory = absolute(argument("images", "web/images/alpine"));
const legacyImagesDirectory = absolute(argument("legacy-images", "web/images"));
const samples = Number(argument("samples", "7"));
const frozenSamples = Number(argument("frozen-samples", "5"));
const userQuantum = BigInt(argument("user-quantum", "50000000000"));
const workloads = argument(
  "workloads",
  "user-alu,user-mixed,legacy-md5,modern-direct,modern-opensbi",
).split(",").filter(Boolean);
const captureWorkloads = new Set(argument(
  "capture-workloads",
  "user-alu,user-mixed,legacy-md5,modern-direct",
).split(",").filter(Boolean));
const outputPath = absolute(argument("output", "target/jit-old-new-report.json"));
if (!Number.isInteger(samples) || samples < 3 || samples > 15) {
  throw new Error("--samples must be an integer from 3 through 15");
}
if (!Number.isInteger(frozenSamples) || frozenSamples < 1 || frozenSamples > 15) {
  throw new Error("--frozen-samples must be an integer from 1 through 15");
}
if (userQuantum <= 0n) throw new Error("--user-quantum must be positive");
const knownWorkloads = new Set([
  "user-alu",
  "user-mixed",
  "legacy-md5",
  "modern-direct",
  "modern-opensbi",
]);
if (workloads.some((workload) => !knownWorkloads.has(workload))) {
  throw new Error(`unknown workload in --workloads: ${workloads.join(",")}`);
}

const variants = {
  old: { root: oldRoot },
  new: { root: newRoot },
};
for (const [name, variant] of Object.entries(variants)) {
  variant.loader = fileIdentity(join(variant.root, "web/rv64.js"));
  variant.wasm = fileIdentity(join(
    variant.root,
    "target/wasm32-unknown-unknown/release/rv64_wasm.wasm",
  ));
  variant.name = name;
}

function runChild(args) {
  // Preserve explicitly selected V8/Wasm flags for tier-sensitivity runs.
  // The normal benchmark has an empty execArgv and remains unchanged.
  const runtimeArgs = process.versions.deno
    ? ["run", "-A", self, ...args]
    : [...process.execArgv, self, ...args];
  const child = spawnSync(process.execPath, runtimeArgs, {
    cwd: repositoryRoot,
    encoding: "utf8",
    maxBuffer: 128 << 20,
  });
  if (child.status !== 0) {
    throw new Error(child.stderr || child.stdout || `worker exited ${child.status}`);
  }
  return JSON.parse(child.stdout);
}

function runHeadlineWorker(side, workload) {
  return runChild([
    "--worker",
    `--variant=${side}`,
    `--variant-root=${variants[side].root}`,
    `--artifacts=${artifacts}`,
    `--images=${imagesDirectory}`,
    `--legacy-images=${legacyImagesDirectory}`,
    `--workload=${workload}`,
    `--user-quantum=${userQuantum}`,
  ]);
}

function summarize(values) {
  return summary(values.filter((value) => Number.isFinite(value)));
}

function metric(trial, name) {
  return trial.result.timing[name];
}

function pairSummary(pairs, oldMetric, newMetric = oldMetric) {
  return summarize(pairs.map((pair) => metric(pair.old, oldMetric) / metric(pair.new, newMetric)));
}

function sideSummary(pairs, side, fields) {
  return Object.fromEntries(fields.map((field) => [field, summarize(
    pairs.map((pair) => metric(pair[side], field)),
  )]));
}

const releaseLock = await acquireBenchmarkLock(artifacts);
const probes = [];
const rawPairs = Object.fromEntries(workloads.map((workload) => [workload, []]));
const captures = {};
const frozen = {};
let failure;
try {
  cpuProbe();
  for (const workload of workloads) {
    process.stderr.write(`[old/new ${workload}]`);
    for (let sample = 0; sample < samples; sample++) {
      const pair = { sample: sample + 1, order: alternatingOrder(sample, "old", "new") };
      for (const side of pair.order) {
        process.stderr.write(` ${sample + 1}${side === "old" ? "o" : "n"}`);
        const before = cpuProbe();
        const result = runHeadlineWorker(side, workload);
        const after = cpuProbe();
        probes.push({ workload, sample: sample + 1, side, before, after });
        pair[side] = { probeBeforeMs: before, probeAfterMs: after, result };
      }
      rawPairs[workload].push(pair);
    }
    process.stderr.write(" ok\n");
  }

  const captureTag = new Date().toISOString().replaceAll(/[:.]/g, "-");
  for (const workload of workloads.filter((entry) => captureWorkloads.has(entry))) {
    captures[workload] = {};
    for (const side of ["old", "new"]) {
      process.stderr.write(`[capture ${workload}/${side}]`);
      const captureDirectory = join(
        repositoryRoot,
        "target/jit-old-new",
        `corpus-${captureTag}`,
        side,
        workload,
      );
      const result = runChild([
        "--worker",
        `--variant=${side}`,
        `--variant-root=${variants[side].root}`,
        `--artifacts=${artifacts}`,
        `--images=${imagesDirectory}`,
        `--legacy-images=${legacyImagesDirectory}`,
        `--workload=${workload}`,
        `--user-quantum=${userQuantum}`,
        `--capture-directory=${captureDirectory}`,
      ]);
      captures[workload][side] = result.capture;
      process.stderr.write(` ${result.capture.moduleCount} modules\n`);
    }

    frozen[workload] = { rawPairs: [] };
    for (let sample = 0; sample < frozenSamples; sample++) {
      const pair = { sample: sample + 1, order: alternatingOrder(sample, "old", "new") };
      for (const side of pair.order) {
        const before = cpuProbe();
        const result = runChild([
          "--frozen-worker",
          `--manifest=${captures[workload][side].manifestPath}`,
        ]);
        const after = cpuProbe();
        probes.push({ workload: `frozen-${workload}`, sample: sample + 1, side, before, after });
        pair[side] = { probeBeforeMs: before, probeAfterMs: after, result };
      }
      frozen[workload].rawPairs.push(pair);
    }
    for (const side of ["old", "new"]) {
      frozen[workload][side] = {
        moduleCount: captures[workload][side].moduleCount,
        totalBytes: captures[workload][side].totalBytes,
        compileMs: summarize(frozen[workload].rawPairs.map((pair) => pair[side].result.compileMs)),
        instantiateMs: summarize(
          frozen[workload].rawPairs.map((pair) => pair[side].result.instantiateMs),
        ),
      };
    }
    frozen[workload].pairedNewSpeedup = {
      compile: summarize(frozen[workload].rawPairs.map(
        (pair) => pair.old.result.compileMs / pair.new.result.compileMs,
      )),
      instantiate: summarize(frozen[workload].rawPairs.map(
        (pair) => pair.old.result.instantiateMs / pair.new.result.instantiateMs,
      )),
    };
  }
} catch (error) {
  failure = error;
} finally {
  await releaseLock();
}
if (failure) throw failure;

const issues = [];
const probeValues = probes.flatMap((probe) => [probe.before, probe.after]);
const probeSpread = Math.max(...probeValues) / Math.min(...probeValues);
if (probeSpread > 1.25) issues.push(`global CPU probe spread ${probeSpread.toFixed(3)}x > 1.25x`);
for (const probe of probes) {
  const drift = Math.max(probe.before, probe.after) / Math.min(probe.before, probe.after);
  if (drift > 1.25) {
    issues.push(
      `${probe.workload} sample ${probe.sample} ${probe.side} probe drift ${drift.toFixed(3)}x`,
    );
  }
}

const workloadReports = {};
for (const workload of workloads) {
  const pairs = rawPairs[workload];
  const user = workload.startsWith("user-");
  const fields = user
    ? ["coreInitMs", "runMs", "minsnPerSec"]
    : ["coreInitMs", "stageMs", "bootRunMs", "coldToReadyMs", "workMs", "workMInsnPerSec"];
  const oldCoveragePercent = summarize(pairs.map((pair) => user
    ? pair.old.result.coveragePercent
    : pair.old.result.workCoveragePercent));
  const newCoveragePercent = summarize(pairs.map((pair) => user
    ? pair.new.result.coveragePercent
    : pair.new.result.workCoveragePercent));
  const report = {
    old: sideSummary(pairs, "old", fields),
    new: sideSummary(pairs, "new", fields),
    pairedNewSpeedup: user
      ? {
        run: pairSummary(pairs, "runMs"),
        coreInit: pairSummary(pairs, "coreInitMs"),
      }
      : {
        coldToReady: pairSummary(pairs, "coldToReadyMs"),
        bootRun: pairSummary(pairs, "bootRunMs"),
        shellWork: pairSummary(pairs, "workMs"),
        coreInit: pairSummary(pairs, "coreInitMs"),
      },
    oldCoveragePercent,
    newCoveragePercent,
    jitComparable: oldCoveragePercent.median > 0 && newCoveragePercent.median > 0,
    rawPairs: pairs,
  };
  workloadReports[workload] = report;

  const identities = pairs.flatMap((pair) => [pair.old.result.identities, pair.new.result.identities]);
  const inputHashes = new Set(identities.map((identity) =>
    identity.input?.sha256 ?? identity.kernel?.sha256).filter(Boolean));
  if (inputHashes.size !== 1) issues.push(`${workload}: input hashes differ`);
  if (user) {
    const outputHashes = new Set(pairs.flatMap((pair) => [
      pair.old.result.result.outputSha256,
      pair.new.result.result.outputSha256,
    ]));
    const instructionCounts = new Set(pairs.flatMap((pair) => [
      pair.old.result.result.insns,
      pair.new.result.result.insns,
    ]));
    if (outputHashes.size !== 1) issues.push(`${workload}: output hashes differ`);
    if (instructionCounts.size !== 1) issues.push(`${workload}: instruction counts differ`);
  } else {
    for (const pair of pairs) {
      for (const side of ["old", "new"]) {
        if (!pair[side].result.result.ready || !pair[side].result.result.complete) {
          issues.push(`${workload} sample ${pair.sample} ${side}: incomplete guest work`);
        }
        if (!pair[side].result.result.pendingSettled) {
          issues.push(`${workload} sample ${pair.sample} ${side}: pending builds did not settle`);
        }
      }
    }
    if (workload === "legacy-md5") {
      const digests = new Set(pairs.flatMap((pair) => [
        pair.old.result.result.digest,
        pair.new.result.result.digest,
      ]));
      if (digests.size !== 1 || digests.has(null)) {
        issues.push(`${workload}: guest digests differ or are missing`);
      }
    }
    if (workload === "modern-direct") {
      for (const pair of pairs) {
        for (const side of ["old", "new"]) {
          if (pair[side].result.result.unsupportedSbiExtension !== "0") {
            issues.push(
              `${workload} sample ${pair.sample} ${side}: unsupported SBI extension count is ` +
              `${pair[side].result.result.unsupportedSbiExtension}`,
            );
          }
        }
      }
    }
  }
}

const report = {
  schema: 1,
  createdAt: new Date().toISOString(),
  methodology: {
    headline: "fresh-node-process-per-side/alternating-paired-order/untouched-runtime",
    generatedModules: "separate-untimed-global-Wasm-constructor-capture",
    frozenFrontend: "fresh-node-process-per-corpus/alternating-paired-order",
    speedupConvention: "old-time/new-time; greater than 1 means the rewrite is faster",
    samples,
    frozenSamples,
    userQuantum: userQuantum.toString(),
    cpuProbeTolerance: 1.25,
  },
  environment: {
    runtime: runtimeInfo(),
    node: process.version,
    v8: process.versions.v8,
    platform: process.platform,
    arch: process.arch,
    cpuProbe: { spread: probeSpread, raw: probes },
  },
  variants,
  workloads: workloadReports,
  captures,
  frozen,
  validity: { valid: issues.length === 0, issues },
};
mkdirSync(dirname(outputPath), { recursive: true });
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`);

const f = (value) => value.toFixed(3);
const runtimeLabel = process.versions.bun
  ? `Bun ${process.versions.bun} (WebKit ${process.versions.webkit})`
  : process.versions.deno
    ? `Deno ${process.versions.deno}, V8 ${process.versions.v8}`
    : `Node ${process.version}, V8 ${process.versions.v8}`;
console.log(`${samples} paired fresh-process samples; ${runtimeLabel}`);
console.log(`CPU probe spread ${probeSpread.toFixed(3)}x; ${issues.length ? "INVALID" : "valid"}`);
for (const workload of workloads) {
  const values = workloadReports[workload];
  if (workload.startsWith("user-")) {
    console.log(
      `${workload}: old ${f(values.old.runMs.median)} ms, new ${f(values.new.runMs.median)} ms, ` +
      `paired new speedup ${f(values.pairedNewSpeedup.run.median)}x; ` +
      `coverage ${f(values.oldCoveragePercent.median)}% -> ${f(values.newCoveragePercent.median)}%`,
    );
  } else {
    const comparisonKind = values.jitComparable
      ? "JIT-vs-JIT"
      : "product comparison only (one side has zero generated-code coverage)";
    console.log(
      `${workload}: cold old ${f(values.old.coldToReadyMs.median)} ms, ` +
      `new ${f(values.new.coldToReadyMs.median)} ms, ` +
      `speedup ${f(values.pairedNewSpeedup.coldToReady.median)}x; ` +
      `work old ${f(values.old.workMs.median)} ms, new ${f(values.new.workMs.median)} ms, ` +
      `speedup ${f(values.pairedNewSpeedup.shellWork.median)}x; ` +
      `coverage ${f(values.oldCoveragePercent.median)}% -> ${f(values.newCoveragePercent.median)}%; ` +
      comparisonKind,
    );
  }
}
for (const [workload, values] of Object.entries(frozen)) {
  console.log(
    `frozen ${workload}: old ${values.old.moduleCount} modules/${values.old.totalBytes} bytes, ` +
    `new ${values.new.moduleCount}/${values.new.totalBytes}; compile speedup ` +
    `${f(values.pairedNewSpeedup.compile.median)}x`,
  );
}
console.log(`report: ${outputPath}`);
