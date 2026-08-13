// Frozen generated-code benchmark for the three general multi-entry state modes.
//
// Every measurement worker is a fresh engine process. Corpus generation and
// guest translation happen in the parent before timing. Each worker reports
// WebAssembly compile, instantiate, first execution, warm-up calls, and raw
// post-warm execution samples separately. The guest cycle and visible state
// are identical between variants.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { median, summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const self = fileURLToPath(import.meta.url);
const corpusDir = join(root, "target/jit-backend-corpus");

const PC_ADDR = 256;
const RETIRED_ADDR = 264;
const FUEL_ADDR = 272;
const ENTRY_PC = 0x1000n;
const CYCLE_INSNS = 24n;
const CYCLE_SUM = 21n;
// One invocation stays below the emitter's defensive 65,536-dispatch cap.
// Steady samples aggregate many identical invocations to get a stable timing
// interval while also giving engines regular tier-publication boundaries.
const FIRST_FUEL = 216_000n;
const WARM_FUEL = 216_000n;
const STEADY_FUEL = 216_000n;
const STEADY_REPETITIONS = 120;
const STEADY_INSNS = STEADY_FUEL * BigInt(STEADY_REPETITIONS);
const LOAD_TAGS = 512;
const LOAD_OFFSETS = LOAD_TAGS + 4096 * 8;
const STORE_TAGS = LOAD_OFFSETS + 4096 * 8;
const STORE_OFFSETS = STORE_TAGS + 4096 * 8;
const ACCESS_CONTEXT = STORE_OFFSETS + 4096 * 8;
const ROW_OFFSET = 2 * 8;
const HOST_DATA = 0x24000;
const TRANSLATION_OFFSET = HOST_DATA - 0x2000;

const variants = [
  "cached",
  "lazy",
  "direct",
  "materialized",
  "tailcall",
  "cached-memory",
  "cached-memory-no-tlb",
  "lazy-memory",
  "direct-memory",
  "materialized-memory",
  "tailcall-memory",
];
const configs = {
  cached: { cycleInsns: CYCLE_INSNS, cycleEffect: CYCLE_SUM, memory: false },
  lazy: { cycleInsns: CYCLE_INSNS, cycleEffect: CYCLE_SUM, memory: false },
  direct: { cycleInsns: CYCLE_INSNS, cycleEffect: CYCLE_SUM, memory: false },
  materialized: { cycleInsns: CYCLE_INSNS, cycleEffect: CYCLE_SUM, memory: false },
  tailcall: { cycleInsns: CYCLE_INSNS, cycleEffect: CYCLE_SUM, memory: false },
  "cached-memory": { cycleInsns: 120n, cycleEffect: 6n, memory: true },
  "cached-memory-no-tlb": { cycleInsns: 120n, cycleEffect: 6n, memory: true },
  "lazy-memory": { cycleInsns: 120n, cycleEffect: 6n, memory: true },
  "direct-memory": { cycleInsns: 120n, cycleEffect: 6n, memory: true },
  "materialized-memory": { cycleInsns: 120n, cycleEffect: 6n, memory: true },
  "tailcall-memory": { cycleInsns: 120n, cycleEffect: 6n, memory: true },
};

const now = () => process.hrtime.bigint();
const elapsedMs = (start) => Number(process.hrtime.bigint() - start) / 1e6;

function reset(memory, fuel, config) {
  new Uint8Array(memory.buffer, 0, FUEL_ADDR + 8).fill(0);
  const view = new DataView(memory.buffer);
  view.setBigUint64(PC_ADDR, ENTRY_PC, true);
  view.setBigUint64(RETIRED_ADDR, 0n, true);
  view.setBigUint64(FUEL_ADDR, fuel, true);
  if (config.memory) {
    view.setBigUint64(20 * 8, 0x2000n, true); // x20 = guest data address
    view.setBigUint64(LOAD_TAGS + ROW_OFFSET, 0x2000n, true);
    view.setBigInt64(LOAD_OFFSETS + ROW_OFFSET, BigInt(TRANSLATION_OFFSET), true);
    view.setBigUint64(STORE_TAGS + ROW_OFFSET, 0x2000n, true);
    view.setBigInt64(STORE_OFFSETS + ROW_OFFSET, BigInt(TRANSLATION_OFFSET), true);
    view.setBigUint64(ACCESS_CONTEXT, 0n, true);
    for (let index = 0; index < 8; index++) {
      view.setBigUint64(HOST_DATA + index * 8, 0x1000n + BigInt(index), true);
      view.setBigUint64(HOST_DATA + 64 + index * 8, 0n, true);
    }
  }
}

function verify(memory, fuel, config, label) {
  const view = new DataView(memory.buffer);
  const retired = view.getBigUint64(RETIRED_ADDR, true);
  const pc = view.getBigUint64(PC_ADDR, true);
  const observed = view.getBigUint64(8, true);
  const expected = (fuel / config.cycleInsns) * config.cycleEffect;
  const copied = !config.memory || Array.from({ length: 8 }, (_, index) =>
    view.getBigUint64(HOST_DATA + 64 + index * 8, true) === 0x1000n + BigInt(index)
  ).every(Boolean);
  if (retired !== fuel || pc !== ENTRY_PC || observed !== expected || !copied) {
    throw new Error(
      `${label}: state mismatch retired=${retired} pc=0x${pc.toString(16)} ` +
        `effect=${observed} expected=${expected} copied=${copied}`,
    );
  }
  return observed;
}

async function worker(variant) {
  const config = configs[variant];
  const bytes = readFileSync(join(corpusDir, `${variant}.wasm`));
  let start = now();
  const module = new WebAssembly.Module(bytes);
  const compileMs = elapsedMs(start);
  const memory = new WebAssembly.Memory({ initial: config.memory ? 3 : 1 });
  start = now();
  const instance = new WebAssembly.Instance(module, { env: { memory } });
  const instantiateMs = elapsedMs(start);
  const run = instance.exports.run;

  reset(memory, FIRST_FUEL, config);
  start = now();
  run(0);
  const firstMs = elapsedMs(start);
  let checksum = verify(memory, FIRST_FUEL, config, "first");

  const warmMs = [];
  for (let index = 0; index < 8; index++) {
    reset(memory, WARM_FUEL, config);
    start = now();
    run(0);
    warmMs.push(elapsedMs(start));
    checksum ^= verify(memory, WARM_FUEL, config, `warm-${index}`);
    // V8 and browser engines tier Wasm between invocations rather than using
    // universal on-stack replacement. Let background optimization publish.
    await new Promise((resolve) => setImmediate(resolve));
  }

  const steadyMs = [];
  for (let index = 0; index < 7; index++) {
    start = now();
    for (let repetition = 0; repetition < STEADY_REPETITIONS; repetition++) {
      reset(memory, STEADY_FUEL, config);
      run(0);
    }
    steadyMs.push(elapsedMs(start));
    checksum ^= verify(memory, STEADY_FUEL, config, `steady-${index}`);
  }

  process.stdout.write(JSON.stringify({
    variant,
    engine: { node: process.version, v8: process.versions.v8 },
    bytes: bytes.length,
    hash: createHash("sha256").update(bytes).digest("hex"),
    compileMs,
    instantiateMs,
    firstMs,
    warmMs,
    steadyMs,
    checksum: checksum.toString(),
  }));
}

if (process.argv.includes("--worker")) {
  const variant = process.argv.find((arg) => arg.startsWith("--variant="))
    ?.split("=")[1];
  if (!variants.includes(variant)) {
    throw new Error(`worker requires one of: ${variants.join(", ")}`);
  }
  await worker(variant);
  process.exit(0);
}

const sampleArg = process.argv.find((arg) => arg.startsWith("--samples="));
const samples = sampleArg ? Number(sampleArg.split("=")[1]) : 7;
if (!Number.isInteger(samples) || samples < 1 || samples > 30) {
  throw new Error("--samples must be an integer from 1 through 30");
}

const generated = spawnSync(
  "cargo",
  ["run", "--release", "-q", "-p", "rv64-dbt", "--example", "emit_backend_corpus", "--", corpusDir],
  { cwd: root, encoding: "utf8" },
);
if (generated.status !== 0) {
  throw new Error(generated.stderr || generated.stdout || "corpus generation failed");
}

const runs = Object.fromEntries(variants.map((variant) => [variant, []]));
for (let sample = 0; sample < samples; sample++) {
  const order = sample % 2 === 0 ? variants : [...variants].reverse();
  for (const variant of order) {
    const child = spawnSync(
      process.execPath,
      [self, "--worker", `--variant=${variant}`],
      { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 },
    );
    if (child.status !== 0) {
      throw new Error(child.stderr || child.stdout || `${variant} worker failed`);
    }
    runs[variant].push(JSON.parse(child.stdout));
  }
}

function summarizeVariant(variant) {
  const variantRuns = runs[variant];
  return {
    bytes: variantRuns[0].bytes,
    hash: variantRuns[0].hash,
    deterministic: variantRuns.every((run) => run.hash === variantRuns[0].hash),
    compileMs: summary(variantRuns.map((run) => run.compileMs)),
    instantiateMs: summary(variantRuns.map((run) => run.instantiateMs)),
    firstMs: summary(variantRuns.map((run) => run.firstMs)),
    steadyMs: summary(variantRuns.map((run) => median(run.steadyMs))),
    steadyMInsnPerSec: summary(variantRuns.map((run) =>
      Number(STEADY_INSNS) / median(run.steadyMs) / 1000)),
    rawRuns: variantRuns,
  };
}

const report = {
  schema: 1,
  methodology: "frozen-bytes/fresh-process/alternating-paired-order",
  samples,
  engine: runs.cached[0].engine,
  fuel: {
    first: FIRST_FUEL.toString(),
    warm: WARM_FUEL.toString(),
    steadyPerInvocation: STEADY_FUEL.toString(),
    steadyRepetitions: STEADY_REPETITIONS,
    steadyTotal: STEADY_INSNS.toString(),
  },
  variants: Object.fromEntries(variants.map((variant) =>
    [variant, summarizeVariant(variant)])),
};
const pairedSpeedup = (faster, baseline) => summary(runs[faster].map((run, index) =>
  median(runs[baseline][index].steadyMs) / median(run.steadyMs)));
report.pairedSteadySpeedup = {
  cachedState: pairedSpeedup("cached", "materialized"),
  lazyVsEager: pairedSpeedup("lazy", "cached"),
  directVsBalanced: pairedSpeedup("direct", "cached"),
  tailCallVsMaterialized: pairedSpeedup("tailcall", "materialized"),
  cachedStateWithMemory: pairedSpeedup("cached-memory", "materialized-memory"),
  lazyVsEagerWithMemory: pairedSpeedup("lazy-memory", "cached-memory-no-tlb"),
  directVsBalancedWithMemory: pairedSpeedup("direct-memory", "cached-memory-no-tlb"),
  tailCallVsMaterializedWithMemory: pairedSpeedup("tailcall-memory", "materialized-memory"),
  translationCache: pairedSpeedup("cached-memory", "cached-memory-no-tlb"),
};

const outputArg = process.argv.find((arg) => arg.startsWith("--output="));
if (outputArg) {
  writeFileSync(outputArg.split("=")[1], `${JSON.stringify(report, null, 2)}\n`);
}

if (process.argv.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const f = (value) => value.toFixed(3);
  console.log(`engine: Node ${report.engine.node}, V8 ${report.engine.v8}`);
  console.log(`samples: ${samples}; ${report.methodology}`);
  for (const variant of variants) {
    const row = report.variants[variant];
    console.log(
      `${variant}: ${row.bytes} bytes; compile ${f(row.compileMs.median)} ms; ` +
        `first ${f(row.firstMs.median)} ms; steady ` +
        `${f(row.steadyMInsnPerSec.median)} Minsn/s`,
    );
  }
  console.log(
    `paired speedups: state=${f(report.pairedSteadySpeedup.cachedState.median)}x; ` +
      `lazy/eager=${f(report.pairedSteadySpeedup.lazyVsEager.median)}x; ` +
      `direct/balanced=${f(report.pairedSteadySpeedup.directVsBalanced.median)}x; ` +
      `tail/materialized=${f(report.pairedSteadySpeedup.tailCallVsMaterialized.median)}x; ` +
      `state+memory=${f(report.pairedSteadySpeedup.cachedStateWithMemory.median)}x; ` +
      `lazy/eager+memory=${f(report.pairedSteadySpeedup.lazyVsEagerWithMemory.median)}x; ` +
      `direct/balanced+memory=${f(report.pairedSteadySpeedup.directVsBalancedWithMemory.median)}x; ` +
      `tail/materialized+memory=${f(report.pairedSteadySpeedup.tailCallVsMaterializedWithMemory.median)}x; ` +
      `translation-cache=${f(report.pairedSteadySpeedup.translationCache.median)}x`,
  );
}
