#!/usr/bin/env node

// Frozen R057 opportunity gate for a dedicated generated-Wasm compiler pool.
// Production loader/runtime code is not modified by this experiment.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  isMainThread,
  parentPort,
  Worker,
  workerData,
} from "node:worker_threads";
import { summary } from "./statistics.mjs";
import { CPU_PROBE_SPEC, cpuProbe } from "./vs-v86/bench-math.mjs";
import { acquireBenchmarkLock } from "./vs-v86/bench-lock.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const self = fileURLToPath(import.meta.url);
const modes = ["current-realm", "compiler-workers"];
const compilerLanes = 2;
const chunkIterations = 4_194_304;
const warmIterations = 16_777_216;
const warmCallsBeforeYield = 8;
const warmCallsMeasured = 4;
const maxSpread = 1.25;
const bootRequiredSpeedup = 1.10;
const bootRequiredLowerBound = 1.00;
const maximumBootReadyRegression = 1.10;
const compileMinimumRetention = 0.90;
const maximumWorkerStartupMs = 100;
const emptyWasm = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);

const foregroundPath = join(root, "target/jit-monomorphic-reentry-corpus/inline.wasm");
const foregroundHash =
  "9b6263deb23033d14d6b8f81f03e4fb8b2916dfdbd856d5e134643f2510d1e4f";
const corpusSpecs = Object.freeze({
  boot: {
    manifest: join(root, "target/bench/r057-current-boot-modules/manifest.json"),
    manifestHash: "ef848ebf739b57346d49c7fa229f93d6d725276ba4d9ccdaffd7c7859ce88e3a",
    calls: 512,
    select: (occurrence) => Number(occurrence.metadata.ticket) <= 10,
  },
  compile: {
    manifest: join(root, "target/bench/r057-current-compile-modules/manifest.json"),
    manifestHash: "e26c628443828ab16e19e7dff95847569cfb05d9b251261561476871f15c4fcf",
    calls: 256,
    select: (occurrence) => occurrence.phase === "steady",
  },
});
const expectedCorpus = Object.freeze({
  boot: { modules: 10, bytes: 3_974_380 },
  compile: { modules: 15, bytes: 5_745_513 },
});

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const now = () => performance.now();
const immediate = () => new Promise((resolveImmediate) => setImmediate(resolveImmediate));

function descriptor(module) {
  return JSON.stringify({
    imports: WebAssembly.Module.imports(module),
    exports: WebAssembly.Module.exports(module),
  });
}

function spread(values) {
  const finite = values.filter(Number.isFinite);
  return finite.length ? Math.max(...finite) / Math.min(...finite) : Infinity;
}

function loadCorpus(name) {
  const spec = corpusSpecs[name];
  const manifestBytes = readFileSync(spec.manifest);
  if (hash(manifestBytes) !== spec.manifestHash) {
    throw new Error(`${name} manifest hash changed`);
  }
  const manifest = JSON.parse(manifestBytes);
  const entries = manifest.modules
    .map((module) => ({ module, occurrence: module.occurrences[0] }))
    .filter(({ occurrence }) => spec.select(occurrence))
    .map(({ module, occurrence }) => {
      const path = join(dirname(spec.manifest), module.filename);
      const file = readFileSync(path);
      const bytes = Uint8Array.from(file);
      if (bytes.byteLength !== module.bytes || hash(bytes) !== module.sha256) {
        throw new Error(`${name} module ${module.sha256} does not match its manifest`);
      }
      return {
        ticket: Number(occurrence.metadata.ticket),
        phase: occurrence.phase,
        sha256: module.sha256,
        byteLength: module.bytes,
        bytes,
      };
    })
    .sort((left, right) => left.ticket - right.ticket);
  const totalBytes = entries.reduce((total, entry) => total + entry.byteLength, 0);
  const expected = expectedCorpus[name];
  if (entries.length !== expected.modules || totalBytes !== expected.bytes) {
    throw new Error(
      `${name} corpus changed: ${entries.length} modules/${totalBytes} bytes`,
    );
  }
  return {
    name,
    manifestPath: spec.manifest,
    manifestHash: spec.manifestHash,
    calls: spec.calls,
    totalBytes,
    entries,
  };
}

class CompilerWorkerPool {
  constructor() {
    this.workers = [];
    this.pending = new Map();
    this.nextId = 1;
  }

  async start() {
    const started = now();
    this.workers = Array.from({ length: compilerLanes }, (_unused, lane) => {
      const worker = new Worker(self, {
        workerData: { role: "r057-compiler", lane },
      });
      worker.on("message", (message) => {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error(message.error));
        else pending.resolve(message);
      });
      worker.on("error", (error) => {
        for (const [id, pending] of this.pending) {
          if (pending.lane !== lane) continue;
          this.pending.delete(id);
          pending.reject(error);
        }
      });
      return worker;
    });
    const featureChecks = await Promise.all(this.workers.map((_worker, lane) =>
      this.compile(lane, Uint8Array.from(emptyWasm))));
    for (const check of featureChecks) {
      if (!(check.module instanceof WebAssembly.Module) ||
          check.workerDescriptor !== descriptor(check.module)) {
        throw new Error("WebAssembly.Module structured-clone feature check failed");
      }
    }
    return now() - started;
  }

  compile(lane, bytes) {
    const id = this.nextId++;
    const buffer = bytes.buffer;
    return new Promise((resolveCompile, rejectCompile) => {
      this.pending.set(id, { lane, resolve: resolveCompile, reject: rejectCompile });
      this.workers[lane].postMessage({ id, buffer }, [buffer]);
    });
  }

  async stop() {
    await Promise.all(this.workers.map((worker) => worker.terminate()));
    this.workers = [];
  }
}

async function startCompiler(mode) {
  const started = now();
  if (mode === "compiler-workers") {
    const pool = new CompilerWorkerPool();
    const startupMs = await pool.start();
    return {
      startupMs,
      compile: (lane, bytes) => pool.compile(lane, bytes),
      stop: () => pool.stop(),
    };
  }
  const featureChecks = await Promise.all([
    WebAssembly.compile(emptyWasm),
    WebAssembly.compile(emptyWasm),
  ]);
  if (featureChecks.some((module) => !(module instanceof WebAssembly.Module))) {
    throw new Error("current-realm WebAssembly.compile feature check failed");
  }
  return {
    startupMs: now() - started,
    compile: async (_lane, bytes) => {
      const compileStarted = now();
      const module = await WebAssembly.compile(bytes);
      return {
        module,
        workerCompileMs: null,
        workerDescriptor: descriptor(module),
        serviceMs: now() - compileStarted,
      };
    },
    stop: async () => {},
  };
}

async function warmForeground(instance) {
  const { run } = instance.exports;
  if (typeof run !== "function") throw new Error("foreground module has no run export");
  for (let index = 0; index < warmCallsBeforeYield; index++) run(warmIterations);
  for (let index = 0; index < 16; index++) await immediate();
  const times = [];
  const results = [];
  for (let index = 0; index < warmCallsMeasured; index++) {
    const started = now();
    results.push(run(warmIterations).toString());
    times.push(now() - started);
    await immediate();
  }
  if (new Set(results).size !== 1) throw new Error("foreground warm results changed");
  return { times, spread: spread(times), result: results[0] };
}

async function runConcurrentCorpus(mode, compiler, foreground, corpus) {
  const retainedModules = [];
  const records = [];
  let nextEntry = 0;
  const started = now();

  const runLane = async (lane) => {
    while (nextEntry < corpus.entries.length) {
      const entry = corpus.entries[nextEntry++];
      const compileStarted = now();
      const result = await compiler.compile(lane, entry.bytes);
      const compileLatencyMs = now() - compileStarted;
      const completedAtMs = now() - started;
      if (!(result.module instanceof WebAssembly.Module)) {
        throw new Error(`${mode}/${corpus.name}/${entry.ticket} returned no module`);
      }
      const mainDescriptor = descriptor(result.module);
      if (mainDescriptor !== result.workerDescriptor) {
        throw new Error(`${mode}/${corpus.name}/${entry.ticket} descriptor changed in clone`);
      }
      retainedModules.push(result.module);
      records.push({
        ticket: entry.ticket,
        phase: entry.phase,
        sha256: entry.sha256,
        bytes: entry.byteLength,
        compileLatencyMs,
        completedAtMs,
        serviceMs: result.serviceMs,
        workerCompileMs: result.workerCompileMs,
        descriptorSha256: hash(mainDescriptor),
      });
    }
  };

  const lanes = Array.from({ length: compilerLanes }, (_unused, lane) => runLane(lane));
  const { run } = foreground.exports;
  let foregroundCallMs = 0;
  let checksum = 0x9e37_79b9_7f4a_7c15n;
  for (let call = 0; call < corpus.calls; call++) {
    const callStarted = now();
    const result = BigInt.asUintN(64, run(chunkIterations));
    foregroundCallMs += now() - callStarted;
    checksum = BigInt.asUintN(
      64,
      (checksum * 0x1000_0000_01b3n) ^ result ^ BigInt(call),
    );
    await immediate();
  }
  const foregroundWallMs = now() - started;
  await Promise.all(lanes);
  const totalMakespanMs = now() - started;
  records.sort((left, right) => left.ticket - right.ticket);
  const expectedTickets = corpus.entries.map((entry) => entry.ticket);
  if (JSON.stringify(records.map((record) => record.ticket)) !== JSON.stringify(expectedTickets)) {
    throw new Error(`${mode}/${corpus.name} did not return the complete ticket stream`);
  }
  return {
    calls: corpus.calls,
    iterationsPerCall: chunkIterations,
    totalIterations: corpus.calls * chunkIterations,
    checksum: `0x${checksum.toString(16).padStart(16, "0")}`,
    foregroundCallMs,
    foregroundWallMs,
    compileReadyMs: records.length
      ? Math.max(...records.map((record) => record.completedAtMs))
      : 0,
    lastModuleReadyMs: records.length
      ? Math.max(...records.map((record) => record.completedAtMs))
      : 0,
    totalMakespanMs,
    moduleCount: records.length,
    moduleBytes: records.reduce((total, record) => total + record.bytes, 0),
    descriptorHashes: records.map((record) => record.descriptorSha256),
    records,
    retainedModuleCount: retainedModules.length,
  };
}

async function sample(mode) {
  if (!modes.includes(mode)) throw new Error(`unknown R057 mode ${mode}`);
  const foregroundBytes = readFileSync(foregroundPath);
  if (hash(foregroundBytes) !== foregroundHash || !WebAssembly.validate(foregroundBytes)) {
    throw new Error("R057 foreground module changed or no longer validates");
  }
  const corpora = Object.fromEntries(Object.keys(corpusSpecs).map((name) =>
    [name, loadCorpus(name)]));
  const foregroundModule = new WebAssembly.Module(foregroundBytes);
  const foreground = new WebAssembly.Instance(foregroundModule);
  const warm = await warmForeground(foreground);
  const compiler = await startCompiler(mode);
  try {
    const boot = await runConcurrentCorpus(mode, compiler, foreground, corpora.boot);
    const compile = await runConcurrentCorpus(mode, compiler, foreground, corpora.compile);
    return {
      mode,
      engine: { node: process.version, v8: process.versions.v8 },
      affinity: readFileSync("/proc/self/status", "utf8")
        .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
      foreground: { bytes: foregroundBytes.length, sha256: foregroundHash, warm },
      compiler: { lanes: compilerLanes, startupMs: compiler.startupMs },
      corpora: Object.fromEntries(Object.entries(corpora).map(([name, corpus]) => [name, {
        manifestHash: corpus.manifestHash,
        moduleCount: corpus.entries.length,
        moduleBytes: corpus.totalBytes,
        tickets: corpus.entries.map((entry) => entry.ticket),
        hashes: corpus.entries.map((entry) => entry.sha256),
      }])),
      rows: { boot, compile },
    };
  } finally {
    await compiler.stop();
  }
}

async function compilerWorker() {
  parentPort.on("message", ({ id, buffer }) => {
    try {
      const bytes = new Uint8Array(buffer);
      const started = now();
      const module = new WebAssembly.Module(bytes);
      const workerCompileMs = now() - started;
      parentPort.postMessage({
        id,
        module,
        workerCompileMs,
        workerDescriptor: descriptor(module),
        serviceMs: workerCompileMs,
      });
    } catch (error) {
      parentPort.postMessage({ id, error: error?.stack ?? String(error) });
    }
  });
}

function pairedSpeedup(runs, row, metric) {
  return summary(runs["current-realm"].map((control, index) =>
    control.rows[row][metric] / runs["compiler-workers"][index].rows[row][metric]));
}

async function orchestrate() {
  const sampleArgument = process.argv.find((argument) => argument.startsWith("--samples="));
  const samples = sampleArgument ? Number(sampleArgument.split("=")[1]) : 7;
  if (!Number.isInteger(samples) || samples < 3 || samples > 29 || !(samples & 1)) {
    throw new Error("--samples must be an odd integer from 3 through 29");
  }
  const artifacts = process.env.ARTIFACTS;
  if (!artifacts) throw new Error("set ARTIFACTS to acquire the benchmark lock");

  const corpora = Object.fromEntries(Object.keys(corpusSpecs).map((name) => {
    const corpus = loadCorpus(name);
    for (const entry of corpus.entries) {
      if (!WebAssembly.validate(entry.bytes)) {
        throw new Error(`${name} module ${entry.sha256} does not validate`);
      }
    }
    return [name, {
      manifest: corpus.manifestPath,
      manifestHash: corpus.manifestHash,
      modules: corpus.entries.length,
      bytes: corpus.totalBytes,
      tickets: corpus.entries.map((entry) => entry.ticket),
      hashes: corpus.entries.map((entry) => entry.sha256),
    }];
  }));

  const releaseLock = await acquireBenchmarkLock(resolve(artifacts));
  const runs = Object.fromEntries(modes.map((mode) => [mode, []]));
  const hostProbes = [];
  try {
    cpuProbe();
    for (let pair = 0; pair < samples; pair++) {
      const order = pair % 2 === 0 ? modes : [...modes].reverse();
      for (const mode of order) {
        const before = cpuProbe();
        const child = spawnSync(
          process.execPath,
          [self, "--sample", `--mode=${mode}`],
          { cwd: root, encoding: "utf8", maxBuffer: 64 << 20 },
        );
        const after = cpuProbe();
        hostProbes.push({ pair: pair + 1, mode, before, after });
        if (child.status !== 0) {
          throw new Error(child.stderr || child.stdout || `${mode} sample failed`);
        }
        runs[mode].push(JSON.parse(child.stdout));
        process.stderr.write(`pair ${pair + 1}/${samples} ${mode} complete\n`);
      }
    }
  } finally {
    await releaseLock();
  }

  const referenceEngine = runs["current-realm"][0].engine;
  const exactInputs = modes.every((mode) => runs[mode].every((run) =>
    run.engine.node === referenceEngine.node && run.engine.v8 === referenceEngine.v8 &&
    run.affinity === runs["current-realm"][0].affinity &&
    Object.keys(corpora).every((name) =>
      run.corpora[name].manifestHash === corpora[name].manifestHash &&
      JSON.stringify(run.corpora[name].hashes) === JSON.stringify(corpora[name].hashes))));
  const warmStable = modes.every((mode) => runs[mode].every((run) =>
    run.foreground.warm.spread <= maxSpread));
  const checksumStable = Object.keys(corpora).every((row) => {
    const values = modes.flatMap((mode) => runs[mode].map((run) => run.rows[row].checksum));
    return new Set(values).size === 1;
  });
  const descriptorsStable = Object.keys(corpora).every((row) => {
    const values = modes.flatMap((mode) => runs[mode].map((run) =>
      JSON.stringify(run.rows[row].descriptorHashes)));
    return new Set(values).size === 1;
  });
  const completeModules = modes.every((mode) => runs[mode].every((run) =>
    Object.keys(corpora).every((row) =>
      run.rows[row].moduleCount === corpora[row].modules &&
      run.rows[row].moduleBytes === corpora[row].bytes &&
      run.rows[row].retainedModuleCount === corpora[row].modules)));
  const hostValues = hostProbes.flatMap((probe) => [probe.before, probe.after]);
  const hostProbeSpread = spread(hostValues);

  const paired = Object.fromEntries(Object.keys(corpora).map((row) => [row,
    Object.fromEntries([
      "foregroundCallMs",
      "foregroundWallMs",
      "compileReadyMs",
      "lastModuleReadyMs",
      "totalMakespanMs",
    ].map((metric) => [metric, pairedSpeedup(runs, row, metric)])),
  ]));
  const workerStartupMs = summary(runs["compiler-workers"].map((run) =>
    run.compiler.startupMs));

  const bootCallLower = paired.boot.foregroundCallMs.medianConfidence95?.[0] ?? 0;
  const bootWallLower = paired.boot.foregroundWallMs.medianConfidence95?.[0] ?? 0;
  const gate = {
    exactInputs,
    allModulesValid: true,
    completeModules,
    descriptorsStable,
    checksumStable,
    warmStable,
    maximumWarmSpread: maxSpread,
    hostProbeSpread,
    maximumHostProbeSpread: maxSpread,
    hostStable: hostProbeSpread <= maxSpread,
    boot: {
      requiredForegroundSpeedup: bootRequiredSpeedup,
      foregroundCallSpeedup: paired.boot.foregroundCallMs.median,
      foregroundCallLower: bootCallLower,
      foregroundCallPass:
        paired.boot.foregroundCallMs.median >= bootRequiredSpeedup &&
        bootCallLower >= bootRequiredLowerBound,
      foregroundWallSpeedup: paired.boot.foregroundWallMs.median,
      foregroundWallLower: bootWallLower,
      foregroundWallPass:
        paired.boot.foregroundWallMs.median >= bootRequiredSpeedup &&
        bootWallLower >= bootRequiredLowerBound,
      maximumReadyRegression: maximumBootReadyRegression,
      readyRatio: paired.boot.compileReadyMs.median,
      readyPass: paired.boot.compileReadyMs.median >= 1 / maximumBootReadyRegression,
    },
    compile: {
      minimumRetention: compileMinimumRetention,
      foregroundCallRatio: paired.compile.foregroundCallMs.median,
      foregroundWallRatio: paired.compile.foregroundWallMs.median,
      readyRatio: paired.compile.compileReadyMs.median,
      makespanRatio: paired.compile.totalMakespanMs.median,
    },
    maximumWorkerStartupMs,
    workerStartupMs: workerStartupMs.median,
    workerStartupPass: workerStartupMs.median <= maximumWorkerStartupMs,
  };
  gate.compile.pass = [
    gate.compile.foregroundCallRatio,
    gate.compile.foregroundWallRatio,
    gate.compile.readyRatio,
    gate.compile.makespanRatio,
  ].every((ratio) => ratio >= compileMinimumRetention);
  gate.admitProductionPrototype =
    gate.exactInputs && gate.allModulesValid && gate.completeModules &&
    gate.descriptorsStable && gate.checksumStable && gate.warmStable &&
    gate.hostStable && gate.boot.foregroundCallPass &&
    gate.boot.foregroundWallPass && gate.boot.readyPass &&
    gate.compile.pass && gate.workerStartupPass;

  const report = {
    schema: 1,
    experiment: "R057",
    mechanism: "dedicated-generated-wasm-compiler-worker-pool",
    productionModified: false,
    methodology:
      "frozen-real-modules/fixed-foreground-wasm/fresh-process/alternating-paired-order",
    samples,
    engine: referenceEngine,
    affinity: runs["current-realm"][0].affinity,
    compilerLanes,
    foreground: {
      path: foregroundPath,
      sha256: foregroundHash,
      chunkIterations,
      warmIterations,
    },
    corpora,
    cpuProbe: CPU_PROBE_SPEC,
    hostProbes,
    hostProbeSpread,
    runs,
    paired,
    workerStartupMs,
    gate,
  };

  const output = join(root, "target/bench/r057-compiler-worker-corpus.json");
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
  console.log(
    `Boot foreground call/wall ${paired.boot.foregroundCallMs.median.toFixed(3)}x/` +
    `${paired.boot.foregroundWallMs.median.toFixed(3)}x`,
  );
  console.log(
    `Boot ready ${paired.boot.compileReadyMs.median.toFixed(3)}x; ` +
    `Compile call/wall/ready ${paired.compile.foregroundCallMs.median.toFixed(3)}x/` +
    `${paired.compile.foregroundWallMs.median.toFixed(3)}x/` +
    `${paired.compile.compileReadyMs.median.toFixed(3)}x`,
  );
  console.log(`admit production prototype: ${gate.admitProductionPrototype}`);
  console.log(`saved ${output}`);
}

if (!isMainThread && workerData?.role === "r057-compiler") {
  await compilerWorker();
} else if (process.argv.includes("--sample")) {
  const mode = process.argv.find((argument) => argument.startsWith("--mode="))
    ?.split("=")[1];
  process.stdout.write(JSON.stringify(await sample(mode)));
} else {
  await orchestrate();
}
