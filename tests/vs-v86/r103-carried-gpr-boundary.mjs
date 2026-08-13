#!/usr/bin/env node

// R103 proof-only two-instance model of the current materialized generated
// boundary versus a fixed x1-x31 + PC/retirement/fuel carried tail-call ABI.

import { spawnSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { summary } from "../statistics.mjs";

const REPS = 7;
const LANES = 34; // x1-x31, PC, retired, fuel
const STORED_LANES = 33; // fuel is invocation-constant
const HOPS = 8_000_000;
const SAMPLES = 3;
const WARMUP_HOPS = 100_000;
const WARMUP_RUNS = 10;
const MAX_SPREAD = 1.10;
const BASELINE_COMPILE_MS = 954.483;
const STEADY_CHAIN_HOPS = 8_558_835;
const STEADY_CHAIN_GPR_OPS_LOWER = 331_208_236;
const FIXED_CHAIN_MEMORY_OPS_PER_HOP = 5; // PC/retired/fuel loads + PC/retired stores
const MODEL_MEMORY_OPS_PER_HOP = LANES + STORED_LANES;
const self = fileURLToPath(import.meta.url);

const uleb = (value) => {
  const bytes = [];
  do {
    let byte = value & 0x7f;
    value = Math.floor(value / 128);
    if (value) byte |= 0x80;
    bytes.push(byte);
  } while (value);
  return bytes;
};
const sleb = (value) => {
  const bytes = [];
  let more = true;
  while (more) {
    let byte = value & 0x7f;
    value >>= 7;
    more = !((value === 0 && !(byte & 0x40)) ||
      (value === -1 && (byte & 0x40)));
    if (more) byte |= 0x80;
    bytes.push(byte);
  }
  return bytes;
};
const vec = (entries) => [...uleb(entries.length), ...entries.flat()];
const str = (value) => vec([...new TextEncoder().encode(value)]);
const section = (id, bytes) => [id, ...uleb(bytes.length), ...bytes];
const type = (params, results = []) => [0x60, ...vec(params), ...vec(results)];
const localGet = (index) => [0x20, ...uleb(index)];
const localSet = (index) => [0x21, ...uleb(index)];
const i32Const = (value) => [0x41, ...sleb(value)];
const i64Const = (value) => [0x42, ...sleb(value)];
const i64Load = (offset) => [0x29, ...uleb(3), ...uleb(offset)];
const i64Store = (offset) => [0x37, ...uleb(3), ...uleb(offset)];
const returnCall = (index) => [0x12, ...uleb(index)];
const body = (locals, instructions) => {
  const bytes = [...vec(locals), ...instructions];
  return [...uleb(bytes.length), ...bytes];
};
const module = (sections) => new Uint8Array([
  0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, ...sections.flat(),
]);
const memoryImport = () => [
  ...str("env"), ...str("memory"), 0x02, 0x00, ...uleb(1),
];
const functionImport = (name, typeIndex) => [
  ...str("env"), ...str(name), 0x00, ...uleb(typeIndex),
];
const functionExport = (name, index) => [
  ...str(name), 0x00, ...uleb(index),
];

function trampolineBytes(carried) {
  const I32 = 0x7f;
  const I64 = 0x7e;
  const entryParams = carried ? [I32, ...Array(LANES).fill(I64)] : [I32];
  const tailParams = [...entryParams, I32];
  const instructions = [];
  for (let index = 0; index < tailParams.length; index++) {
    instructions.push(...localGet(index));
  }
  // The last operand is the table index; all preceding operands are the
  // fixed generated-entry signature.
  instructions.push(0x13, ...uleb(0), ...uleb(0), 0x0b);
  return module([
    section(1, vec([type(entryParams), type(tailParams)])),
    section(2, vec([[
      ...str("env"), ...str("table"), 0x01, 0x70, 0x00, ...uleb(0),
    ]])),
    section(3, vec([uleb(1)])),
    section(7, vec([functionExport("tail", 0)])),
    section(10, vec([body([], instructions)])),
  ]);
}

function memoryModuleBytes(target) {
  const I32 = 0x7f;
  const I64 = 0x7e;
  const instructions = [];
  for (let lane = 0; lane < LANES; lane++) {
    instructions.push(...localGet(0), ...i64Load(lane * 8), ...localSet(1 + lane));
  }
  // Exercise every architectural GPR lane, toggle PC, and retire one unit.
  for (let lane = 0; lane < 31; lane++) {
    instructions.push(
      ...localGet(1 + lane), ...i64Const(lane + 1), 0x7c, ...localSet(1 + lane),
    );
  }
  instructions.push(...localGet(32), ...i64Const(1), 0x85, ...localSet(32));
  instructions.push(...localGet(33), ...i64Const(1), 0x7c, ...localSet(33));
  for (let lane = 0; lane < STORED_LANES; lane++) {
    instructions.push(...localGet(0), ...localGet(1 + lane), ...i64Store(lane * 8));
  }
  instructions.push(
    ...localGet(33), ...localGet(34), 0x54, // retired < fuel
    0x04, 0x40,
    ...localGet(0), ...i32Const(target), ...returnCall(0),
    0x0b,
    0x0b,
  );
  return module([
    section(1, vec([type([I32]), type([I32, I32])])),
    section(2, vec([memoryImport(), functionImport("tail", 1)])),
    section(3, vec([uleb(0)])),
    section(7, vec([functionExport("run", 1)])),
    section(10, vec([body([[...uleb(LANES), I64]], instructions)])),
  ]);
}

function carriedModuleBytes(target) {
  const I32 = 0x7f;
  const I64 = 0x7e;
  const carriedParams = [I32, ...Array(LANES).fill(I64)];
  const tailParams = [...carriedParams, I32];
  const wrapper = [...localGet(0)];
  for (let lane = 0; lane < LANES; lane++) {
    wrapper.push(...localGet(0), ...i64Load(lane * 8));
  }
  // One imported function precedes two definitions, so carried is index 2.
  wrapper.push(...returnCall(2), 0x0b);

  const carried = [];
  for (let lane = 0; lane < 31; lane++) {
    carried.push(
      ...localGet(1 + lane), ...i64Const(lane + 1), 0x7c, ...localSet(1 + lane),
    );
  }
  carried.push(...localGet(32), ...i64Const(1), 0x85, ...localSet(32));
  carried.push(...localGet(33), ...i64Const(1), 0x7c, ...localSet(33));
  carried.push(
    ...localGet(33), ...localGet(34), 0x5a, // retired >= fuel
    0x04, 0x40,
  );
  for (let lane = 0; lane < STORED_LANES; lane++) {
    carried.push(...localGet(0), ...localGet(1 + lane), ...i64Store(lane * 8));
  }
  carried.push(0x0f, 0x0b); // return; end final-commit arm
  for (let index = 0; index < carriedParams.length; index++) {
    carried.push(...localGet(index));
  }
  carried.push(...i32Const(target), ...returnCall(0), 0x0b);

  return module([
    section(1, vec([type([I32]), type(carriedParams), type(tailParams)])),
    section(2, vec([memoryImport(), functionImport("tail", 2)])),
    section(3, vec([uleb(0), uleb(1)])),
    section(7, vec([
      functionExport("run", 1),
      functionExport("carried", 2),
    ])),
    section(10, vec([body([], wrapper), body([], carried)])),
  ]);
}

function instantiate(variant) {
  const carried = variant === "carried";
  const memory = new WebAssembly.Memory({ initial: 1 });
  const table = new WebAssembly.Table({ initial: 2, element: "anyfunc" });
  const trampolineBytesValue = trampolineBytes(carried);
  const trampoline = new WebAssembly.Instance(
    new WebAssembly.Module(trampolineBytesValue),
    { env: { table } },
  ).exports.tail;
  const moduleBytes = [0, 1].map((index) => carried
    ? carriedModuleBytes(index ^ 1)
    : memoryModuleBytes(index ^ 1));
  const instances = moduleBytes.map((bytes) => new WebAssembly.Instance(
    new WebAssembly.Module(bytes),
    { env: { memory, tail: trampoline } },
  ));
  for (let index = 0; index < instances.length; index++) {
    table.set(index, carried ? instances[index].exports.carried : instances[index].exports.run);
  }
  return {
    memory,
    run: instances[0].exports.run,
    wasmBytes: {
      trampoline: trampolineBytesValue.length,
      generated: moduleBytes.map((bytes) => bytes.length),
      total: trampolineBytesValue.length + moduleBytes.reduce((sum, bytes) => sum + bytes.length, 0),
    },
  };
}

function initialize(memory, hops) {
  const state = new BigUint64Array(memory.buffer, 0, LANES);
  for (let lane = 0; lane < 32; lane++) state[lane] = BigInt(lane + 1);
  state[32] = 0n; // retired
  state[33] = BigInt(hops); // fuel
}

function checksum(memory) {
  const state = new BigUint64Array(memory.buffer, 0, LANES);
  let hash = 0xcbf29ce484222325n;
  for (const value of state) {
    hash ^= value;
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return `0x${hash.toString(16).padStart(16, "0")}`;
}

function runWorker(variant, shapeOnly) {
  if (variant !== "memory" && variant !== "carried") {
    throw new Error(`unknown worker variant ${variant}`);
  }
  const instance = instantiate(variant);
  for (let run = 0; run < WARMUP_RUNS; run++) {
    initialize(instance.memory, WARMUP_HOPS);
    instance.run(0);
  }
  if (shapeOnly) {
    initialize(instance.memory, 1_000_000);
    instance.run(0);
  }
  const rawMs = [];
  const fingerprints = [];
  for (let sample = 0; sample < SAMPLES; sample++) {
    initialize(instance.memory, HOPS);
    const started = performance.now();
    instance.run(0);
    rawMs.push(performance.now() - started);
    fingerprints.push(checksum(instance.memory));
  }
  const elapsed = summary(rawMs);
  const result = {
    variant,
    engine: { node: process.version, v8: process.versions.v8 },
    topology: "two-generated-instances-one-table-owning-tail-trampoline",
    lanes: LANES,
    storedLanes: STORED_LANES,
    hops: HOPS,
    samples: SAMPLES,
    warmup: { runs: WARMUP_RUNS, hops: WARMUP_HOPS },
    wasmBytes: instance.wasmBytes,
    elapsedMs: elapsed,
    nsPerHop: elapsed.median * 1e6 / HOPS,
    spread: elapsed.max / elapsed.min,
    fingerprints: [...new Set(fingerprints)],
  };
  process.stdout.write(`R103_WORKER_JSON ${JSON.stringify(result)}\n`);
}

if (process.argv[2] === "--worker") {
  runWorker(process.argv[3], process.argv.includes("--shape"));
  process.exit(0);
}

if (process.argv[2] === "--selftest") {
  for (const variant of ["memory", "carried"]) {
    const instance = instantiate(variant);
    for (const hops of [1, 2, 3, 17, 1000]) {
      initialize(instance.memory, hops);
      instance.run(0);
      if (new BigUint64Array(instance.memory.buffer, 0, LANES)[32] !== BigInt(hops)) {
        throw new Error(`${variant}: retirement mismatch at ${hops}`);
      }
    }
  }
  const fingerprints = ["memory", "carried"].map((variant) => {
    const instance = instantiate(variant);
    initialize(instance.memory, 1000);
    instance.run(0);
    return checksum(instance.memory);
  });
  if (fingerprints[0] !== fingerprints[1]) throw new Error("variant checksum mismatch");
  console.log(`R103 carried-GPR boundary selftest: PASS (${fingerprints[0]})`);
  process.exit(0);
}

const [outputPath] = process.argv.slice(2);
if (!outputPath) {
  throw new Error("usage: r103-carried-gpr-boundary.mjs OUTPUT.json");
}

function child(variant, trace = false) {
  const args = trace
    ? ["--trace-wasm-compilation-times", self, "--worker", variant, "--shape"]
    : [self, "--worker", variant];
  const run = spawnSync(process.execPath, args, {
    encoding: "utf8",
    maxBuffer: 32 << 20,
  });
  if (run.status !== 0) throw new Error(run.stderr || run.stdout);
  const line = run.stdout.split(/\r?\n/).find((value) => value.startsWith("R103_WORKER_JSON "));
  if (!line) throw new Error(`${variant}: worker result missing`);
  return { result: JSON.parse(line.slice("R103_WORKER_JSON ".length)), trace: run.stderr + run.stdout };
}

const shape = {};
for (const variant of ["memory", "carried"]) {
  const traced = child(variant, true);
  const events = [...traced.trace.matchAll(
    /Compiled function .*?#(\d+) using (Liftoff|TurboFan), took (\d+) μs.*?bodysize (\d+) codesize (\d+)/g,
  )].map((match) => ({
    functionIndex: Number(match[1]),
    compiler: match[2],
    microseconds: Number(match[3]),
    bodyBytes: Number(match[4]),
    codeBytes: Number(match[5]),
  }));
  shape[variant] = {
    liftoffEvents: events.filter((event) => event.compiler === "Liftoff").length,
    turboFanEvents: events.filter((event) => event.compiler === "TurboFan").length,
    largestBodies: events.toSorted((left, right) => right.bodyBytes - left.bodyBytes).slice(0, 8),
    worker: traced.result,
  };
}

const pairs = [];
for (let rep = 0; rep < REPS; rep++) {
  const order = rep & 1 ? ["carried", "memory"] : ["memory", "carried"];
  const results = {};
  for (const variant of order) results[variant] = child(variant).result;
  pairs.push({ rep: rep + 1, order, ...results });
}

const memoryNs = pairs.map((pair) => pair.memory.nsPerHop);
const carriedNs = pairs.map((pair) => pair.carried.nsPerHop);
const pairedSpeedups = pairs.map((pair) => pair.memory.nsPerHop / pair.carried.nsPerHop);
const pairedSavingsNs = pairs.map((pair) => pair.memory.nsPerHop - pair.carried.nsPerHop);
const memory = summary(memoryNs);
const carried = summary(carriedNs);
const speedup = summary(pairedSpeedups);
const saving = summary(pairedSavingsNs);
const attributableOpsPerHop =
  STEADY_CHAIN_GPR_OPS_LOWER / STEADY_CHAIN_HOPS + FIXED_CHAIN_MEMORY_OPS_PER_HOP;
const opportunityScale = Math.min(1, attributableOpsPerHop / MODEL_MEMORY_OPS_PER_HOP);
const conservativeSavingNs = Math.max(0, saving.medianConfidence95[0]) * opportunityScale;
const conservativeSavedMs = conservativeSavingNs * STEADY_CHAIN_HOPS / 1e6;
const projectedCompileSpeedup = BASELINE_COMPILE_MS /
  (BASELINE_COMPILE_MS - conservativeSavedMs);
const fingerprints = new Set(pairs.flatMap((pair) => [
  ...pair.memory.fingerprints,
  ...pair.carried.fingerprints,
]));
const checks = {
  shapeLiftoff: shape.memory.liftoffEvents >= 2 && shape.carried.liftoffEvents >= 2,
  shapeTurboFan: shape.memory.turboFanEvents >= 2 && shape.carried.turboFanEvents >= 2,
  exactOutput: fingerprints.size === 1,
  completePairs: pairs.length === REPS,
  withinWorkerSpread: pairs.every((pair) =>
    pair.memory.spread <= MAX_SPREAD && pair.carried.spread <= MAX_SPREAD),
  sideSpread: memory.max / memory.min <= MAX_SPREAD && carried.max / carried.min <= MAX_SPREAD,
  positiveMedian: speedup.median > 1,
  positiveLowerBound: speedup.medianConfidence95[0] > 1,
  projectedWholeCompile: projectedCompileSpeedup >= 1.03,
};
const pass = Object.values(checks).every(Boolean);
const report = {
  schema: 1,
  experiment: "R103 carried-GPR cross-module boundary opportunity",
  performanceEvidence: "local ordinary-tiered-engine proof only",
  frozen: {
    repetitions: REPS,
    lanes: LANES,
    storedLanes: STORED_LANES,
    hops: HOPS,
    samplesPerFreshProcess: SAMPLES,
    warmupHops: WARMUP_HOPS,
    warmupRuns: WARMUP_RUNS,
    maximumSpread: MAX_SPREAD,
    baselineCompileMs: BASELINE_COMPILE_MS,
    steadyChainHops: STEADY_CHAIN_HOPS,
    steadyChainGprOperationsLowerBound: STEADY_CHAIN_GPR_OPS_LOWER,
    fixedChainMemoryOperationsPerHop: FIXED_CHAIN_MEMORY_OPS_PER_HOP,
    modelMemoryOperationsPerHop: MODEL_MEMORY_OPS_PER_HOP,
    minimumProjectedCompileSpeedup: 1.03,
  },
  shape,
  pairs,
  observed: {
    memoryNsPerHop: memory,
    carriedNsPerHop: carried,
    pairedSpeedup: speedup,
    pairedSavingNs: saving,
    attributableMemoryOperationsPerHop: attributableOpsPerHop,
    opportunityScale,
    conservativeSavingNsPerHop: conservativeSavingNs,
    conservativeSavedCompileMs: conservativeSavedMs,
    projectedCompileSpeedup,
    fingerprint: [...fingerprints],
  },
  checks,
  pass,
  decision: pass
    ? "admit-one-frozen-carried-gpr-product-design"
    : "close-carried-gpr-boundary-and-restore-baseline",
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(`memory ${memory.median.toFixed(3)} ns/hop`);
console.log(`carried ${carried.median.toFixed(3)} ns/hop`);
console.log(
  `paired ${speedup.median.toFixed(3)}x ` +
  `[${speedup.medianConfidence95.map((value) => value.toFixed(3)).join(", ")}]`,
);
console.log(
  `conservative whole-Compile projection ${projectedCompileSpeedup.toFixed(4)}x; pass=${pass}`,
);
if (!pass) process.exitCode = 1;
