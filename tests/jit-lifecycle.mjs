// JIT lifecycle benchmark with process-isolated cold samples.
//
// The parent launches a fresh Node process per workload sample so V8's module
// and optimizing-code caches cannot leak from one sample into the next. Each
// worker reports raw execution chunks and separately accumulated guest
// translation, generated-module copy, WebAssembly compile, instantiation, and
// table-publication time. A second set of fresh workers compiles and
// instantiates the exact captured module bytes without guest translation.
// Frozen replay intentionally measures the engine frontend only; it does not
// execute modules whose absolute state addresses belong to the capture VM.

import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const self = fileURLToPath(import.meta.url);

async function workloadWorker() {
  const { RV64Debug: RV64, Stop } = await import(join(root, "web/rv64.js"));
  const wasm = readFileSync(
    join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  );
  const elf = new Uint8Array(readFileSync(
    join(root, "guests/bench/target/riscv64gc-unknown-linux-musl/release/bench"),
  ));
  const initStart = performance.now();
  const vm = await RV64.create(wasm);
  const initMs = performance.now() - initStart;
  vm.onWrite = () => {};
  const modules = [];
  vm.onJitModule = (bytes, metadata) => {
    modules.push({
      bytes: Buffer.from(bytes).toString("base64"),
      hash: createHash("sha256").update(bytes).digest("hex"),
      metadata,
    });
  };
  if (!vm.loadElf(elf, ["bench", "fast"], 32)) throw new Error("bench ELF load failed");

  const chunks = [];
  let stop = Stop.BUDGET;
  while (stop === Stop.BUDGET && chunks.length < 100) {
    const beforeInsns = vm.userInsnCount();
    const beforeJit = vm.ex.jit_stat(0);
    const beforeDispatches = vm.ex.jit_stat(1);
    const beforeModules = vm.jitRegCount ?? 0;
    const start = performance.now();
    stop = vm.runUser(5_000_000n);
    const ms = performance.now() - start;
    const insns = vm.userInsnCount() - beforeInsns;
    chunks.push({
      ms,
      insns: insns.toString(),
      minsnPerSec: Number(insns) / ms / 1000,
      jitInsns: (vm.ex.jit_stat(0) - beforeJit).toString(),
      dispatches: (vm.ex.jit_stat(1) - beforeDispatches).toString(),
      newModules: (vm.jitRegCount ?? 0) - beforeModules,
    });
  }
  const lifecycle = vm.jitLifecycleStats();
  const active = chunks.filter((chunk) => BigInt(chunk.insns) >= 4_000_000n);
  const plateau = active.filter((chunk) => chunk.newModules === 0).slice(-10);
  process.stdout.write(JSON.stringify({
    engine: { node: process.version, v8: process.versions.v8 },
    initMs,
    stop,
    exitCode: vm.userExitCode(),
    totalInsns: vm.userInsnCount().toString(),
    lifecycle,
    chunks,
    firstChunk: chunks[0],
    plateauMinsnPerSec: plateau.map((chunk) => chunk.minsnPerSec),
    moduleHashes: modules.map((module) => module.hash),
    modules,
  }));
}

function replayWorker() {
  const input = JSON.parse(readFileSync(0, "utf8"));
  const memory = new WebAssembly.Memory({ initial: 1 });
  const table = new WebAssembly.Table({ initial: input.length, element: "anyfunc" });
  const modules = input.map((encoded) => Buffer.from(encoded, "base64"));
  const compileStart = performance.now();
  const compiled = modules.map((bytes) => new WebAssembly.Module(bytes));
  const compileMs = performance.now() - compileStart;
  const instantiateStart = performance.now();
  const instances = compiled.map((module) => new WebAssembly.Instance(module, {
    env: {
      memory,
      fp_exec: () => 0n,
      user_reservation: () => 0,
      system_reservation: () => 0,
      tlb_fill: () => -1n,
      system_bulk_copy: () => 0n,
    },
  }));
  const instantiateMs = performance.now() - instantiateStart;
  const publishStart = performance.now();
  instances.forEach((instance, index) => table.set(index, instance.exports.run));
  const publishMs = performance.now() - publishStart;
  process.stdout.write(JSON.stringify({ compileMs, instantiateMs, publishMs }));
}

if (process.argv.includes("--worker-run")) {
  await workloadWorker();
  process.exit(0);
}
if (process.argv.includes("--worker-replay")) {
  replayWorker();
  process.exit(0);
}

const sampleArg = process.argv.find((arg) => arg.startsWith("--samples="));
const samples = sampleArg ? Number(sampleArg.split("=")[1]) : 3;
if (!Number.isInteger(samples) || samples < 1 || samples > 30) {
  throw new Error("--samples must be an integer from 1 through 30");
}
const runs = [];
for (let index = 0; index < samples; index++) {
  const child = spawnSync(process.execPath, [self, "--worker-run"], {
    encoding: "utf8",
    maxBuffer: 64 << 20,
  });
  if (child.status !== 0) throw new Error(child.stderr || child.stdout);
  runs.push(JSON.parse(child.stdout));
}

const referenceHashes = runs[0].moduleHashes;
const deterministic = runs.every((run) =>
  JSON.stringify(run.moduleHashes) === JSON.stringify(referenceHashes));
const frozenInput = JSON.stringify(runs[0].modules.map((module) => module.bytes));
const replays = [];
for (let index = 0; index < samples; index++) {
  const child = spawnSync(process.execPath, [self, "--worker-replay"], {
    input: frozenInput,
    encoding: "utf8",
    maxBuffer: 64 << 20,
  });
  if (child.status !== 0) throw new Error(child.stderr || child.stdout);
  replays.push(JSON.parse(child.stdout));
}

function wasmerReplay(mode, encodedModules) {
  const directory = mkdtempSync(join(tmpdir(), "rv64-dbt-wasmer-"));
  try {
    const paths = encodedModules.map((encoded, index) => {
      const path = join(directory, `${index}.wasm`);
      writeFileSync(path, Buffer.from(encoded, "base64"));
      return path;
    });
    const validateStart = performance.now();
    for (const path of paths) {
      const result = spawnSync("wasmer", ["validate", path], { encoding: "utf8" });
      if (result.status !== 0) throw new Error(result.stderr || result.stdout);
    }
    const validateMs = performance.now() - validateStart;
    const compileStart = performance.now();
    for (let index = 0; index < paths.length; index++) {
      const result = spawnSync(
        "wasmer",
        ["compile", `--${mode}`, "-o", join(directory, `${index}.wasmu`), paths[index]],
        { encoding: "utf8" },
      );
      if (result.status !== 0) throw new Error(result.stderr || result.stdout);
    }
    return { validateMs, compileMs: performance.now() - compileStart };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

let wasmer;
if (process.argv.includes("--wasmer")) {
  const version = spawnSync("wasmer", ["--version"], { encoding: "utf8" });
  if (version.status !== 0) throw new Error("--wasmer requested but Wasmer is unavailable");
  const modes = {};
  for (const mode of ["singlepass", "cranelift"]) {
    const samplesForMode = Array.from({ length: samples }, () =>
      wasmerReplay(mode, runs[0].modules.map((module) => module.bytes)));
    modes[mode] = {
      validateMs: summary(samplesForMode.map((sample) => sample.validateMs)),
      compileMs: summary(samplesForMode.map((sample) => sample.compileMs)),
    };
  }
  wasmer = { version: version.stdout.trim(), modes };
}

const report = {
  schema: 1,
  isolation: "fresh-node-process-per-sample",
  samples,
  engine: runs[0].engine,
  moduleCount: referenceHashes.length,
  moduleHashes: referenceHashes,
  deterministicModuleOrder: deterministic,
  cold: {
    totalMs: summary(runs.map((run) =>
      run.chunks.reduce((total, chunk) => total + chunk.ms, 0))),
    firstChunkMs: summary(runs.map((run) => run.firstChunk.ms)),
    translateMs: summary(runs.map((run) => run.lifecycle.translateMs)),
    compileMs: summary(runs.map((run) => run.lifecycle.compileMs)),
    instantiateMs: summary(runs.map((run) => run.lifecycle.instantiateMs)),
    publishMs: summary(runs.map((run) => run.lifecycle.publishMs)),
    emittedBytes: runs.map((run) => run.lifecycle.emittedBytes),
    translateAttempts: runs.map((run) => run.lifecycle.translateAttempts),
  },
  steady: {
    minsnPerSec: summary(runs.flatMap((run) => run.plateauMinsnPerSec)),
    rawByProcess: runs.map((run) => run.plateauMinsnPerSec),
  },
  frozenFrontend: {
    compileMs: summary(replays.map((replay) => replay.compileMs)),
    instantiateMs: summary(replays.map((replay) => replay.instantiateMs)),
    publishMs: summary(replays.map((replay) => replay.publishMs)),
  },
  ...(wasmer ? { wasmer } : {}),
};

if (process.argv.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const f = (value) => value.toFixed(3);
  console.log(`engine: Node ${report.engine.node}, V8 ${report.engine.v8}`);
  console.log(`cold samples: ${samples} fresh processes; modules: ${report.moduleCount}; deterministic: ${deterministic}`);
  console.log(`cold total ms median/p95: ${f(report.cold.totalMs.median)} / ${f(report.cold.totalMs.p95)}`);
  console.log(`translation ms median/p95: ${f(report.cold.translateMs.median)} / ${f(report.cold.translateMs.p95)}`);
  console.log(`Wasm compile ms median/p95: ${f(report.cold.compileMs.median)} / ${f(report.cold.compileMs.p95)}`);
  console.log(`instantiate ms median/p95: ${f(report.cold.instantiateMs.median)} / ${f(report.cold.instantiateMs.p95)}`);
  console.log(`publish ms median/p95: ${f(report.cold.publishMs.median)} / ${f(report.cold.publishMs.p95)}`);
  console.log(`steady Minsn/s median/p95: ${f(report.steady.minsnPerSec.median)} / ${f(report.steady.minsnPerSec.p95)}`);
  console.log(`frozen compile ms median/p95: ${f(report.frozenFrontend.compileMs.median)} / ${f(report.frozenFrontend.compileMs.p95)}`);
  if (report.wasmer) {
    for (const [mode, values] of Object.entries(report.wasmer.modes)) {
      console.log(`Wasmer ${mode} compile ms median/p95: ${f(values.compileMs.median)} / ${f(values.compileMs.p95)}`);
    }
  }
}
