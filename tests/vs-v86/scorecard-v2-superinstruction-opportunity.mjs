#!/usr/bin/env node

// Proof-only R047 opportunity probe for a precompiled multi-instruction
// baseline. The modern scorecard guest runs with generated execution disabled;
// the diagnostic runtime counts normalized scalar-RV64 opcode sequences within
// page-local dynamic basic blocks. No candidate executes and no generated Wasm
// module is constructed or published.

import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const artifacts = process.env.ARTIFACTS ? resolve(process.env.ARTIFACTS) : null;
if (!artifacts) throw new Error("set ARTIFACTS");

const timeoutMs = Number(process.env.TIMEOUT_MS || 900_000);
const traceSlice = Number(process.env.TRACE_SLICE || 16_384);
const compileSamples = Number(process.env.COMPILE_SAMPLES || 3);
const replayFrom = process.env.REPLAY_FROM ? resolve(process.env.REPLAY_FROM) : null;
const traceWorkload = process.env.TRACE_WORKLOAD || "boot";
const librarySizes = (process.env.LIBRARY_SIZES || "8,16,32,64,128,256,512")
  .split(",").map(Number);
if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 60_000 ||
    !Number.isSafeInteger(traceSlice) || traceSlice < 1 ||
    !Number.isSafeInteger(compileSamples) || compileSamples < 1 || compileSamples > 9 ||
    librarySizes.some((value) => !Number.isSafeInteger(value) || value < 1 || value > 4096) ||
    !new Set(["boot", "compile", "python"]).has(traceWorkload)) {
  throw new Error("invalid timeout, trace slice, compile sample count, or library sizes");
}

const paths = {
  loader: join(root, "web/rv64.js"),
  wasm: join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  acceptedWasm: join(root,
    "target/bench/wasm-candidates/r043-post-diagnostic-clean-d93345139c5a.wasm"),
  kernel: join(root, "web/images/alpine/Image"),
  initramfs: join(artifacts, "scorecard-v2-modern-riscv64.cpio"),
};
const [loaderBytes, wasm, acceptedWasm, kernel, initramfs] = await Promise.all([
  readFile(paths.loader), readFile(paths.wasm), readFile(paths.acceptedWasm),
  readFile(paths.kernel), readFile(paths.initramfs),
]);
const hashes = Object.fromEntries(Object.entries({ loaderBytes, wasm, acceptedWasm, kernel, initramfs })
  .map(([name, bytes]) => [name === "loaderBytes" ? "loader" : name,
    createHash("sha256").update(bytes).digest("hex")]));

let replaySource = null;
let replaySourceHash = null;
if (replayFrom) {
  const sourceBytes = await readFile(replayFrom);
  replaySourceHash = createHash("sha256").update(sourceBytes).digest("hex");
  replaySource = JSON.parse(sourceBytes);
  if (replaySource.format !== "rv64-scorecard-v2-superinstruction-opportunity-v1" ||
      replaySource.meta?.schema !== "1" || !Array.isArray(replaySource.topTriples) ||
      replaySource.topTriples.length < 256) {
    throw new Error("REPLAY_FROM is not a complete first-pass R047 opportunity report");
  }
  for (const name of ["loader", "acceptedWasm", "kernel", "initramfs"]) {
    if (replaySource.inputs?.[name] !== hashes[name]) {
      throw new Error(`REPLAY_FROM ${name} differs from the current exact input`);
    }
  }
}

const { RV64Debug } = await import(pathToFileURL(paths.loader).href);
const vm = await RV64Debug.create(wasm);
for (const name of [
  "super_trace_finish", "super_trace_meta", "super_trace_op_ptr",
  "super_trace_pair_ptr", "super_trace_triple_ptr", "super_trace_run_histogram_ptr",
  "super_trace_selection_clear", "super_trace_select_triple", "super_trace_replay_stat",
]) {
  if (typeof vm.ex[name] !== "function") throw new Error(`diagnostic runtime lacks ${name}`);
}

if (replaySource) {
  if (vm.ex.super_trace_selection_clear() !== 1) {
    throw new Error("could not initialize exact replay selection");
  }
  const seen = new Set();
  for (const [index, pattern] of replaySource.topTriples.slice(0, 256).entries()) {
    if (!Number.isSafeInteger(pattern.key) || pattern.key < 0 ||
        pattern.key >= 62 ** 3 || seen.has(pattern.key) ||
        vm.ex.super_trace_select_triple(pattern.key, index + 1) !== 1) {
      throw new Error(`invalid selected triple at popularity rank ${index + 1}`);
    }
    seen.add(pattern.key);
  }
}

let output = "";
const decoder = new TextDecoder();
vm.onWrite = (_fd, bytes) => { output += decoder.decode(bytes, { stream: true }); };
vm.ex.jit_set_enabled(0);
if (traceWorkload === "boot") vm.ex.policy_trace_set_enabled(1);
vm.bootVirtLinuxDirect({
  kernel: new Uint8Array(kernel),
  initrd: new Uint8Array(initramfs),
  ramMB: 512,
  cmdline: "console=ttyS0 rdinit=/init",
});

const tick = () => new Promise((done) => setImmediate(done));
async function runUntil(marker, label, offset = 0) {
  const waitStarted = performance.now();
  for (let iteration = 0; output.indexOf(marker, offset) === -1; iteration++) {
    if (vm.runVirtSystem(BigInt(traceSlice))) throw new Error(`guest powered off during ${label}`);
    if ((iteration & 15) === 0) await tick();
    if (performance.now() - waitStarted > timeoutMs) {
      throw new Error(`${label} timed out: ${output.slice(-3000)}`);
    }
  }
}

let workloadEvidence = null;
let started = performance.now();
await runUntil("SCORECARD_V2_READY", "boot trace");
if (traceWorkload !== "boot") {
  vm.virtConsoleInput(new TextEncoder().encode(
    "stty -echo 2>/dev/null; echo SCORECARD_V2_SHELL_\"READY\"\n",
  ));
  await runUntil("SCORECARD_V2_SHELL_READY", "shell setup");
  if (vm.ex.policy_trace_set_enabled(1) !== undefined) {
    throw new Error("unexpected policy trace enable return value");
  }
  const offset = output.length;
  const workloads = {
    compile: {
      command: "echo SCORECARD_V2_TRACE_\"BEGIN\"; rm -f /tmp/w.o; " +
        "echo RUN_START; /opt/scorecard/tcc -c /opt/scorecard/w.c -o /tmp/w.o " +
        "2>/tmp/tcc-error; rc=$?; echo RUN_DONE; md5sum /tmp/w.o; " +
        "echo SCORECARD_V2_\"EXIT\"=$rc; echo SCORECARD_V2_TRACE_\"COMPLETE\"\n",
      validate(segment) {
        const md5 = segment.match(/([0-9a-f]{32})\s+\/tmp\/w\.o/)?.[1] ?? null;
        const exit = segment.match(/SCORECARD_V2_EXIT=(\d+)/)?.[1] ?? null;
        if (exit !== "0" || md5 !== "24eedf7e06beffd4d3ba1945585588db") {
          throw new Error(`compile workload failed: ${segment.slice(-3000)}`);
        }
        return { exit, md5 };
      },
    },
    python: {
      command: "echo SCORECARD_V2_TRACE_\"BEGIN\"; " +
        "/usr/bin/python3 /opt/scorecard/fib.py; rc=$?; " +
        "echo SCORECARD_V2_\"EXIT\"=$rc; echo SCORECARD_V2_TRACE_\"COMPLETE\"\n",
      validate(segment) {
        const checksum = segment.match(/fib\(30\)=\s*(\d+)/)?.[1] ?? null;
        const exit = segment.match(/SCORECARD_V2_EXIT=(\d+)/)?.[1] ?? null;
        if (exit !== "0" || checksum !== "832040") {
          throw new Error(`python workload failed: ${segment.slice(-3000)}`);
        }
        return { exit, checksum };
      },
    },
  };
  const workload = workloads[traceWorkload];
  started = performance.now();
  vm.virtConsoleInput(new TextEncoder().encode(workload.command));
  await runUntil("SCORECARD_V2_TRACE_COMPLETE", `${traceWorkload} trace`, offset);
  workloadEvidence = workload.validate(output.slice(offset));
}
vm.ex.super_trace_finish();
vm.ex.policy_trace_set_enabled(0);
const traceElapsedMs = performance.now() - started;

const metaValue = (field) => vm.ex.super_trace_meta(field);
const meta = {
  schema: metaValue(0),
  totalInstructions: metaValue(1),
  eligibleInstructions: metaValue(2),
  runs: metaValue(3),
  pairEvents: metaValue(4),
  tripleEvents: metaValue(5),
  maxRun: metaValue(6),
  operationKinds: metaValue(7),
  runBins: metaValue(8),
  replayLibraries: metaValue(9),
};
if (meta.schema !== 2n || meta.totalInstructions === 0n || meta.operationKinds !== 62n ||
    meta.replayLibraries !== (replaySource ? 6n : 0n)) {
  throw new Error(`invalid superinstruction metadata ${JSON.stringify(meta,
    (_key, value) => typeof value === "bigint" ? value.toString() : value)}`);
}

const operationNames = [
  "LUI", "AUIPC", "JAL", "JALR", "BEQ", "BNE", "BLT", "BGE", "BLTU", "BGEU",
  "LB", "LH", "LW", "LD", "LBU", "LHU", "LWU", "SB", "SH", "SW", "SD",
  "ADDI", "SLTI", "SLTIU", "XORI", "ORI", "ANDI", "SLLI", "SRLI", "SRAI",
  "ADDIW", "SLLIW", "SRLIW", "SRAIW", "ADD", "SLL", "SLT", "SLTU", "XOR",
  "SRL", "OR", "AND", "SUB", "SRA", "MUL", "MULH", "MULHSU", "MULHU",
  "DIV", "DIVU", "REM", "REMU", "ADDW", "SLLW", "SRLW", "SUBW", "SRAW",
  "MULW", "DIVW", "DIVUW", "REMW", "REMUW",
];
if (operationNames.length !== Number(meta.operationKinds)) throw new Error("operation table mismatch");

function u64View(pointer, length, label) {
  if (!pointer || pointer % 8 !== 0) throw new Error(`invalid ${label} pointer ${pointer}`);
  return new BigUint64Array(vm.ex.memory.buffer, pointer, length);
}
const operations = u64View(vm.ex.super_trace_op_ptr(), operationNames.length, "operation");
const pairs = u64View(vm.ex.super_trace_pair_ptr(), operationNames.length ** 2, "pair");
const triples = u64View(vm.ex.super_trace_triple_ptr(), operationNames.length ** 3, "triple");
const runHistogram = u64View(vm.ex.super_trace_run_histogram_ptr(), Number(meta.runBins), "run");

const exactReplay = replaySource ? Array.from({ length: Number(meta.replayLibraries) }, (_, index) => {
  const limit = vm.ex.super_trace_replay_stat(index, 0);
  const dispatches = vm.ex.super_trace_replay_stat(index, 1);
  const groups = vm.ex.super_trace_replay_stat(index, 2);
  const covered = vm.ex.super_trace_replay_stat(index, 3);
  const savings = vm.ex.super_trace_replay_stat(index, 4);
  if (dispatches + savings !== meta.eligibleInstructions || covered !== groups * 3n ||
      savings !== groups * 2n) {
    throw new Error(`inconsistent exact replay accounting for library ${limit}`);
  }
  const ineligible = meta.totalInstructions - meta.eligibleInstructions;
  return {
    handlers: Number(limit),
    eligibleDispatches: dispatches,
    resultingAllDispatches: dispatches + ineligible,
    fusedGroups: groups,
    coveredInstructions: covered,
    dispatchSavings: savings,
    averageFusedGroupLength: groups === 0n ? 0 : Number(covered) / Number(groups),
    fractionOfEligibleDispatches: Number(savings) / Number(meta.eligibleInstructions),
    fractionOfAllDispatches: Number(savings) / Number(meta.totalInstructions),
    clearsFortyPercentAllDispatchGate: Number(savings) >= Number(meta.totalInstructions) * 0.4,
  };
}) : [];

const operationCounts = operationNames.map((name, id) => ({ name, id, count: operations[id] }))
  .sort((left, right) => left.count === right.count ? left.id - right.id :
    left.count > right.count ? -1 : 1);
const pairPatterns = [];
for (let key = 0; key < pairs.length; key++) {
  if (pairs[key] === 0n) continue;
  const first = Math.floor(key / operationNames.length);
  const second = key % operationNames.length;
  pairPatterns.push({ key, ops: [first, second], pattern: `${operationNames[first]} ${operationNames[second]}`,
    count: pairs[key], optimisticDispatchSavings: pairs[key] });
}
pairPatterns.sort((left, right) => left.count === right.count ? left.key - right.key :
  left.count > right.count ? -1 : 1);

const triplePatterns = [];
for (let key = 0; key < triples.length; key++) {
  if (triples[key] === 0n) continue;
  const first = Math.floor(key / (operationNames.length ** 2));
  const remainder = key % (operationNames.length ** 2);
  const second = Math.floor(remainder / operationNames.length);
  const third = remainder % operationNames.length;
  triplePatterns.push({
    key, ops: [first, second, third],
    pattern: `${operationNames[first]} ${operationNames[second]} ${operationNames[third]}`,
    count: triples[key], optimisticDispatchSavings: triples[key] * 2n,
  });
}
triplePatterns.sort((left, right) =>
  left.optimisticDispatchSavings === right.optimisticDispatchSavings ? left.key - right.key :
    left.optimisticDispatchSavings > right.optimisticDispatchSavings ? -1 : 1);

const runRows = Array.from(runHistogram, (count, length) => ({
  length: length === runHistogram.length - 1 ? `${length}+` : length,
  modeledLength: length,
  runs: count,
  instructions: count * BigInt(length),
})).filter((row) => row.runs !== 0n);
const modeledRunInstructions = runRows.reduce((total, row) => total + row.instructions, 0n);
const fixedWidthBounds = [2, 3, 4].map((width) => {
  const resultingDispatches = runRows.reduce((total, row) =>
    total + row.runs * BigInt(Math.ceil(row.modeledLength / width)), 0n);
  const savings = modeledRunInstructions - resultingDispatches;
  return {
    width, resultingDispatches, dispatchSavings: savings,
    fractionOfEligibleDispatches: Number(savings) / Number(meta.eligibleInstructions),
    fractionOfAllDispatches: Number(savings) / Number(meta.totalInstructions),
    overflowRunsModeledAt: runHistogram.length - 1,
  };
});

const tripleIdealSavings = fixedWidthBounds.find((row) => row.width === 3).dispatchSavings;
const libraryBounds = librarySizes.map((handlers) => {
  const selected = triplePatterns.slice(0, handlers);
  const overlappingSavings = selected.reduce((total, row) =>
    total + row.optimisticDispatchSavings, 0n);
  const cappedSavings = overlappingSavings < tripleIdealSavings
    ? overlappingSavings : tripleIdealSavings;
  return {
    handlers,
    selectedOccurrences: selected.reduce((total, row) => total + row.count, 0n),
    overlappingDispatchSavings: overlappingSavings,
    cappedOptimisticDispatchSavings: cappedSavings,
    cappedFractionOfEligibleDispatches:
      Number(cappedSavings) / Number(meta.eligibleInstructions),
    cappedFractionOfAllDispatches: Number(cappedSavings) / Number(meta.totalInstructions),
    estimatedCodeBytesAt512PerHandler: handlers * 512,
    clearsFortyPercentAllDispatchGate:
      Number(cappedSavings) >= Number(meta.totalInstructions) * 0.4,
  };
});

const fortyPercentTarget = Number(meta.totalInstructions) * 0.4;
let cumulativeSavings = 0;
let minimumOptimisticTripleHandlers = null;
for (let index = 0; index < triplePatterns.length; index++) {
  cumulativeSavings += Number(triplePatterns[index].optimisticDispatchSavings);
  if (Math.min(cumulativeSavings, Number(tripleIdealSavings)) >= fortyPercentTarget) {
    minimumOptimisticTripleHandlers = index + 1;
    break;
  }
}

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length & 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};
async function coldCompileSample(path) {
  const program = [
    "const fs=require('fs'); const {performance}=require('perf_hooks');",
    "const bytes=fs.readFileSync(process.argv[1]); const start=performance.now();",
    "WebAssembly.compile(bytes).then(()=>process.stdout.write(String(performance.now()-start)));",
  ].join(" ");
  return await new Promise((done, reject) => {
    const child = spawn(process.execPath, ["-e", program, path], { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code !== 0) reject(new Error(`compile child failed ${code}: ${stderr}`));
      else done(Number(stdout));
    });
  });
}
const compileLatency = { acceptedMs: [], diagnosticMs: [] };
for (let index = 0; index < compileSamples; index++) {
  const order = index & 1 ? ["diagnostic", "accepted"] : ["accepted", "diagnostic"];
  for (const side of order) {
    compileLatency[`${side}Ms`].push(await coldCompileSample(
      side === "accepted" ? paths.acceptedWasm : paths.wasm));
  }
}
compileLatency.acceptedMedianMs = median(compileLatency.acceptedMs);
compileLatency.diagnosticMedianMs = median(compileLatency.diagnosticMs);

const result = {
  format: "rv64-scorecard-v2-superinstruction-opportunity-v1",
  capturedAt: new Date().toISOString(),
  proofOnly: true,
  inputs: hashes,
  configuration: {
    traceSlice, compileSamples, librarySizes, traceWorkload,
    replayFrom: replayFrom ? { path: replayFrom, sha256: replaySourceHash } : null,
    dispatchRemovalGate: 0.4,
    minimumAverageGroupLength: 3,
    maximumAdmittedHandlers: 256,
    codeBudgetBytesPerHandler: 512,
  },
  traceElapsedMs,
  workloadEvidence,
  meta: {
    ...meta,
    eligibleFraction: Number(meta.eligibleInstructions) / Number(meta.totalInstructions),
    modeledRunInstructions,
  },
  currentMainWasm: {
    acceptedBytes: acceptedWasm.length,
    diagnosticBytes: wasm.length,
    diagnosticAddedBytes: wasm.length - acceptedWasm.length,
    compileLatency,
  },
  fixedWidthBounds,
  minimumOptimisticTripleHandlers,
  libraryBounds,
  exactNonOverlappingReplay: exactReplay,
  opportunityGate: {
    fixedWidthTriplesCanRemoveFortyPercent:
      fixedWidthBounds.find((row) => row.width === 3).fractionOfAllDispatches >= 0.4,
    atMost256TriplePatternsCanRemoveFortyPercentOptimistically:
      libraryBounds.find((row) => row.handlers === 256)?.clearsFortyPercentAllDispatchGate ?? false,
    atMost256TriplePatternsCanRemoveFortyPercentExactly:
      exactReplay.find((row) => row.handlers === 256)?.clearsFortyPercentAllDispatchGate ?? false,
    exactReplayMaintainsMinimumAverageGroupLength:
      exactReplay.find((row) => row.handlers === 256)?.averageFusedGroupLength >= 3 || false,
    requiresExactNonOverlappingReplayBeforeImplementation: !replaySource,
  },
  operationCounts,
  topPairs: pairPatterns.slice(0, 128),
  topTriples: triplePatterns.slice(0, 256),
  uniquePairPatterns: pairPatterns.length,
  uniqueTriplePatterns: triplePatterns.length,
  runHistogram: runRows,
  consoleTail: output.slice(-1000),
};

const json = JSON.stringify(result, (_key, value) =>
  typeof value === "bigint" ? value.toString() : value, 2) + "\n";
if (process.env.OUT) {
  const out = resolve(process.env.OUT);
  await mkdir(dirname(out), { recursive: true });
  await writeFile(out, json);
}
process.stdout.write(json);
