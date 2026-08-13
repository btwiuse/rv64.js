#!/usr/bin/env node

// R109 proof-only ordered-tree versus dense-bitset stackifier model. Every
// timed leg owns a fresh V8 process; module compile/instantiate, corpus copy,
// parsing, validation, and output rendering are outside reported spans.

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { summary } from "./statistics.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const self = fileURLToPath(import.meta.url);
const crate = join(root, "tests/stackifier-model-wasm/Cargo.toml");
const wasmPath = join(
  root,
  "tests/stackifier-model-wasm/target/wasm32-unknown-unknown/release/stackifier_model_wasm.wasm",
);
const corpusRoot = join(root, "target/bench/r109-dense-cfg/corpus");
const CFG_MARKER = 0x31474643;
const PAIRS = 7;
const WARM_CALLS = 6;
const STEADY_CALLS = 7;
const REPETITIONS = 1;

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function encodeGraph(successors, entries, duplicationLimit) {
  const words = [CFG_MARKER, successors.length, entries.length, duplicationLimit, ...entries];
  for (const outgoing of successors) words.push(outgoing.length, ...outgoing);
  const bytes = Buffer.allocUnsafe(words.length * 4);
  words.forEach((word, index) => bytes.writeUInt32LE(word >>> 0, index * 4));
  return bytes;
}

function generatedCorrectnessCorpus() {
  const records = [];
  const duplicationLimits = [0, 1, 8, 250];

  // Empty and deliberately filtered/deduplicated inputs cover boundary
  // semantics absent from the production capture.
  for (const limit of duplicationLimits) records.push(encodeGraph([], [], limit));
  records.push(encodeGraph([[0, 1, 1, 99], [0, 99]], [0, 0, 1, 99], 250));
  records.push(encodeGraph([[], [], []], [], 0));

  // Exhaust every directed adjacency matrix, every non-empty entry subset,
  // and every production-relevant duplication regime through three nodes.
  for (let nodes = 1; nodes <= 3; nodes++) {
    const adjacencyCount = 2 ** (nodes * nodes);
    for (let adjacency = 0; adjacency < adjacencyCount; adjacency++) {
      const successors = Array.from({ length: nodes }, () => []);
      for (let source = 0; source < nodes; source++) {
        for (let target = 0; target < nodes; target++) {
          const bit = source * nodes + target;
          if (adjacency & (2 ** bit)) successors[source].push(target);
        }
      }
      for (let entryMask = 1; entryMask < 2 ** nodes; entryMask++) {
        const entries = Array.from({ length: nodes }, (_, node) => node)
          .filter((node) => entryMask & (2 ** node));
        for (const limit of duplicationLimits) {
          records.push(encodeGraph(successors, entries, limit));
        }
      }
    }
  }

  // Fixed-seed sparse graphs exercise every size class through the exact
  // 512-member production cap without encoding a workload-specific shape.
  let state = 0x6d2b79f5;
  const random = () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return state >>> 0;
  };
  const forcedSizes = [1, 2, 3, 8, 32, 64, 96, 128, 192, 256, 384, 511, 512];
  for (let graph = 0; graph < 256; graph++) {
    const nodes = graph < forcedSizes.length ? forcedSizes[graph] : 1 + random() % 512;
    const successors = Array.from({ length: nodes }, () => {
      const outgoing = [];
      const count = random() % 3;
      for (let edge = 0; edge < count; edge++) outgoing.push(random() % nodes);
      return outgoing;
    });
    let entries;
    if (graph % 11 === 0) {
      entries = Array.from({ length: nodes }, (_, node) => node);
    } else {
      const selected = new Set();
      const count = 1 + random() % Math.min(nodes, 16);
      while (selected.size < count) selected.add(random() % nodes);
      entries = [...selected].sort((left, right) => left - right);
    }
    records.push(encodeGraph(successors, entries, duplicationLimits[graph % duplicationLimits.length]));
  }
  return Buffer.concat(records);
}

function accountRawCorpus(bytes) {
  let offset = 0;
  const totals = { graphs: 0, nodes: 0, edges: 0, entries: 0 };
  const u32 = () => {
    if (offset + 4 > bytes.length) throw new Error(`truncated model corpus at ${offset}`);
    const value = bytes.readUInt32LE(offset);
    offset += 4;
    return value;
  };
  while (offset < bytes.length) {
    if (u32() !== CFG_MARKER) throw new Error(`bad model corpus marker at ${offset - 4}`);
    const nodes = u32();
    const entries = u32();
    u32();
    offset += entries * 4;
    if (offset > bytes.length) throw new Error("truncated model corpus entries");
    let edges = 0;
    for (let node = 0; node < nodes; node++) {
      const count = u32();
      edges += count;
      offset += count * 4;
      if (offset > bytes.length) throw new Error("truncated model corpus edges");
    }
    totals.graphs++;
    totals.nodes += nodes;
    totals.edges += edges;
    totals.entries += entries;
  }
  if (offset !== bytes.length) throw new Error("model corpus final boundary mismatch");
  return totals;
}

function instantiate(corpus) {
  const wasm = readFileSync(wasmPath);
  const instance = new WebAssembly.Instance(new WebAssembly.Module(wasm), {});
  const { exports } = instance;
  const pointer = exports.model_input_reserve(corpus.length);
  new Uint8Array(exports.memory.buffer, pointer, corpus.length).set(corpus);
  const graphs = exports.model_prepare();
  if (graphs === 0xffff_ffff) throw new Error("model rejected corpus");
  return { exports, wasm, graphs };
}

function render(exports, dense) {
  const length = exports.model_render(dense ? 1 : 0);
  return Buffer.from(
    new Uint8Array(exports.memory.buffer, exports.model_output_ptr(), length).slice(),
  );
}

function validateCorpus(name, corpus) {
  const { exports, graphs } = instantiate(corpus);
  const failure = exports.model_validate();
  const tree = render(exports, false);
  const dense = render(exports, true);
  const expected = accountRawCorpus(corpus);
  const model = {
    graphs: Number(exports.model_account(0)),
    nodes: Number(exports.model_account(1)),
    edges: Number(exports.model_account(2)),
    entries: Number(exports.model_account(3)),
  };
  return {
    name,
    inputBytes: corpus.length,
    inputSha256: sha256(corpus),
    graphs,
    expected,
    model,
    failure,
    structureBytes: tree.length,
    treeSha256: sha256(tree),
    denseSha256: sha256(dense),
    exactStructureBytes: tree.equals(dense),
    accountingExact: JSON.stringify(expected) === JSON.stringify(model),
  };
}

function worker(side, corpusPath, repetitions) {
  const corpus = readFileSync(corpusPath);
  const { exports, wasm, graphs } = instantiate(corpus);
  const fn = side === "control" ? exports.model_run_tree : exports.model_run_dense;
  let started = performance.now();
  const firstResult = fn(repetitions);
  const firstMs = performance.now() - started;
  for (let call = 0; call < WARM_CALLS; call++) fn(repetitions);
  const steadyMs = [];
  const steadyResults = [];
  for (let call = 0; call < STEADY_CALLS; call++) {
    started = performance.now();
    steadyResults.push(fn(repetitions).toString());
    steadyMs.push(performance.now() - started);
  }
  process.stdout.write(JSON.stringify({
    side,
    corpusPath,
    corpusSha256: sha256(corpus),
    corpusBytes: corpus.length,
    graphs,
    repetitions,
    wasmSha256: sha256(wasm),
    wasmBytes: wasm.length,
    firstResult: firstResult.toString(),
    firstMs,
    steadyResults,
    steadyMs,
    node: process.version,
    v8: process.versions.v8,
    cpus: readFileSync("/proc/self/status", "utf8")
      .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
  }));
}

if (process.argv[2] === "--worker") {
  worker(process.argv[3], resolve(process.argv[4]), Number(process.argv[5]));
  process.exit(0);
}

const output = resolve(
  process.argv.find((argument) => argument.startsWith("--output="))
    ?.slice("--output=".length) ??
    join(root, "target/bench/r109-dense-cfg/model-gate.json"),
);
if (!process.argv.includes("--skip-build")) {
  const built = spawnSync(
    "cargo",
    ["build", "--release", "--target", "wasm32-unknown-unknown", "--manifest-path", crate],
    { cwd: root, encoding: "utf8" },
  );
  if (built.status !== 0) throw new Error(built.stderr || built.stdout || "model build failed");
}

const evidenceRoot = dirname(output);
mkdirSync(evidenceRoot, { recursive: true });
const bootPath = join(corpusRoot, "rewrite-boot-first.rvcfg");
const compilePaths = ["first", "prime", "steady"]
  .map((phase) => join(corpusRoot, `rewrite-compile-${phase}.rvcfg`));
const boot = readFileSync(bootPath);
const compile = Buffer.concat(compilePaths.map((path) => readFileSync(path)));
const generated = generatedCorrectnessCorpus();
const compilePath = join(evidenceRoot, "model-compile.rvcfg");
const generatedPath = join(evidenceRoot, "model-generated-correctness.rvcfg");
writeFileSync(compilePath, compile, { flag: "wx" });
writeFileSync(generatedPath, generated, { flag: "wx" });
const correctness = Buffer.concat([boot, compile, generated]);

const validation = [
  validateCorpus("boot-production", boot),
  validateCorpus("compile-production", compile),
  validateCorpus("production-exhaustive-random", correctness),
];

const timingCorpora = {
  boot: { path: bootPath, bytes: boot },
  compile: { path: compilePath, bytes: compile },
};
const runs = {
  boot: { control: [], candidate: [] },
  compile: { control: [], candidate: [] },
};
const order = [];
for (const row of Object.keys(timingCorpora)) {
  for (let pair = 0; pair < PAIRS; pair++) {
    const pairOrder = pair & 1 ? ["candidate", "control"] : ["control", "candidate"];
    order.push({ row, pair, order: pairOrder });
    for (const side of pairOrder) {
      const child = spawnSync(
        process.execPath,
        [self, "--worker", side, timingCorpora[row].path, String(REPETITIONS)],
        { cwd: root, encoding: "utf8", maxBuffer: 8 << 20 },
      );
      if (child.status !== 0) {
        throw new Error(child.stderr || child.stdout || `${row}/${side} model worker failed`);
      }
      runs[row][side].push(JSON.parse(child.stdout));
    }
  }
}

function timingSummary(row) {
  const control = runs[row].control;
  const candidate = runs[row].candidate;
  const firstSpeedups = control.map((run, index) => run.firstMs / candidate[index].firstMs);
  const steadyControl = control.map((run) => summary(run.steadyMs).median);
  const steadyCandidate = candidate.map((run) => summary(run.steadyMs).median);
  const steadySpeedups = steadyControl.map((value, index) => value / steadyCandidate[index]);
  return {
    first: {
      controlMs: summary(control.map((run) => run.firstMs)),
      candidateMs: summary(candidate.map((run) => run.firstMs)),
      pairedSpeedup: summary(firstSpeedups),
    },
    steady: {
      controlMs: summary(steadyControl),
      candidateMs: summary(steadyCandidate),
      pairedSpeedup: summary(steadySpeedups),
    },
  };
}

const timing = { boot: timingSummary("boot"), compile: timingSummary("compile") };
const problems = [];
for (const item of validation) {
  if (item.failure !== 0) problems.push(`${item.name}: structure mismatch at graph ${item.failure - 1}`);
  if (!item.exactStructureBytes) problems.push(`${item.name}: serialized structures differ`);
  if (!item.accountingExact) problems.push(`${item.name}: graph accounting differs`);
}
for (const row of Object.keys(runs)) {
  const all = [...runs[row].control, ...runs[row].candidate];
  for (const run of all) {
    if (new Set(run.steadyResults).size !== 1 || run.steadyResults[0] !== run.firstResult) {
      problems.push(`${row}/${run.side}: result fingerprint changed`);
    }
  }
  for (const field of [
    "corpusSha256", "corpusBytes", "graphs", "repetitions", "wasmSha256", "wasmBytes",
    "firstResult", "node", "v8", "cpus",
  ]) {
    if (new Set(all.map((run) => JSON.stringify(run[field]))).size !== 1) {
      problems.push(`${row}: ${field} differs between legs`);
    }
  }
}

const bootFirstLower = timing.boot.first.pairedSpeedup.medianConfidence95[0];
const bootSteadyLower = timing.boot.steady.pairedSpeedup.medianConfidence95[0];
const gate = {
  exactStructureEquality: problems.length === 0,
  bootFirstMedianAtLeast1_80: timing.boot.first.pairedSpeedup.median >= 1.80,
  bootFirstLower95AtLeast1_50: bootFirstLower >= 1.50,
  bootSteadyMedianAtLeast1_80: timing.boot.steady.pairedSpeedup.median >= 1.80,
  bootSteadyLower95AtLeast1_50: bootSteadyLower >= 1.50,
  compileFirstMedianAtLeast1_00: timing.compile.first.pairedSpeedup.median >= 1.00,
  compileSteadyMedianAtLeast1_00: timing.compile.steady.pairedSpeedup.median >= 1.00,
};
const admitted = problems.length === 0 && Object.values(gate).every(Boolean);
const modelBytes = readFileSync(wasmPath);
const report = {
  schema: 1,
  experiment: "R109 dense CFG stackifier admission model",
  methodology: "seven-alternating-fresh-V8-process-pairs/first-and-tiered-steady",
  preregistration: "docs/jit-rewrite/R109_DENSE_CFG_STACKIFIER_PROTOCOL.md",
  model: {
    pairs: PAIRS,
    repetitions: REPETITIONS,
    warmCalls: WARM_CALLS,
    steadyCalls: STEADY_CALLS,
    exhaustiveDirectedGraphsThroughNodes: 3,
    deterministicRandomGraphs: 256,
    maximumNodes: 512,
    generatedCorpus: {
      path: generatedPath,
      bytes: generated.length,
      sha256: sha256(generated),
      account: accountRawCorpus(generated),
    },
    artifact: { path: wasmPath, bytes: modelBytes.length, sha256: sha256(modelBytes) },
  },
  validation,
  order,
  timing,
  gate,
  admitted,
  problems,
  raw: runs,
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(
  `boot first ${timing.boot.first.pairedSpeedup.median.toFixed(3)}x ` +
  `[${timing.boot.first.pairedSpeedup.medianConfidence95.map((v) => v.toFixed(3)).join(",")}], ` +
  `steady ${timing.boot.steady.pairedSpeedup.median.toFixed(3)}x ` +
  `[${timing.boot.steady.pairedSpeedup.medianConfidence95.map((v) => v.toFixed(3)).join(",")}]; ` +
  `compile first ${timing.compile.first.pairedSpeedup.median.toFixed(3)}x, ` +
  `steady ${timing.compile.steady.pairedSpeedup.median.toFixed(3)}x; ` +
  `admitted=${admitted}; report=${output}; sha256=${sha256(readFileSync(output))}`,
);
if (!admitted) process.exitCode = 1;
