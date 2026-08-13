#!/usr/bin/env node

// Compile and inspect the frozen G001 standalone model without collecting a
// performance sample. This creates the identities that must be copied into
// the protocol and timing harness before the first timed process is allowed.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");
const self = fileURLToPath(import.meta.url);
const source = join(here, "interpreter-g001/g001-fencei-decode-model.c");
const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
const output = resolve(outputArgument?.slice("--output=".length) ??
  join(root, "target/bench/interpreter-g001-model-v1"));
const zig = process.env.ZIG || "/home/linuxbrew/.linuxbrew/bin/zig";

if (existsSync(output)) {
  throw new Error(`refusing to replace G001 model directory: ${output}`);
}
mkdirSync(output, { recursive: true });

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const modulePath = join(output, "model.wasm");
const repeatPath = join(output, "model-repeat.wasm");
const commonArguments = Object.freeze([
  "cc",
  "-target", "wasm32-freestanding",
  "-O3",
  "-nostdlib",
  "-Wl,--no-entry",
  "-Wl,--export-memory",
  "-Wl,--strip-all",
]);

function compile(path) {
  const arguments_ = [...commonArguments, "-o", path, source];
  const child = spawnSync(zig, arguments_, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 8 << 20,
  });
  if (child.status !== 0) {
    throw new Error(child.stderr || child.stdout || "G001 model compilation failed");
  }
  return [zig, ...arguments_];
}

const commands = [compile(modulePath), compile(repeatPath)];
const moduleBytes = readFileSync(modulePath);
const repeatBytes = readFileSync(repeatPath);
if (!WebAssembly.validate(moduleBytes) || !WebAssembly.validate(repeatBytes)) {
  throw new Error("G001 model does not validate");
}

const quote = (word) => /^[A-Za-z0-9_./,:=+-]+$/.test(word)
  ? word
  : `'${word.replaceAll("'", "'\\''")}'`;
writeFileSync(join(output, "BUILD-COMMANDS.txt"),
  `${commands.map((command) => command.map(quote).join(" ")).join("\n")}\n`,
  { flag: "wx" });

const requiredExports = Object.freeze([
  "memory",
  "init_model",
  "reset_state",
  "reset_cache",
  "run_control",
  "run_treatment",
  "run_single_control",
  "run_single_treatment",
  "run_wrap_control",
  "run_wrap_treatment",
  "run_straddle_control",
  "run_straddle_treatment",
  "set_mutation_version",
  "run_mutation_control",
  "run_mutation_treatment",
  "fence_i",
  "set_fetch_context",
  "bump_map_generation",
  "state_word_count",
  "state_word",
  "flat_count",
  "block_count",
  "block_pc_at",
  "block_length_at",
  "flat_field",
  "flat_raw_byte",
  "compressed_bucket_for_op",
  "control_for_op",
  "cache_index_for_pc",
  "cache_hits",
  "cache_misses",
  "ifetch_generation_value",
  "model_error",
  "wrap_block_count",
  "cache_slots",
  "block_limit",
  "normalized_record_bytes",
]);

const module = new WebAssembly.Module(moduleBytes);
function fresh() {
  const instance = new WebAssembly.Instance(module);
  const missing = requiredExports.filter((name) => !(name in instance.exports));
  if (missing.length) throw new Error(`G001 model exports missing: ${missing.join(", ")}`);
  const exports_ = instance.exports;
  const error = exports_.init_model();
  if (error !== 0) throw new Error(`G001 model initialization failed: ${error}`);
  return exports_;
}

function stateBytes(exports_) {
  const count = exports_.state_word_count();
  const bytes = Buffer.alloc(count * 8);
  for (let index = 0; index < count; index++) {
    bytes.writeBigUInt64LE(BigInt.asUintN(64, exports_.state_word(index)), index * 8);
  }
  return bytes;
}

function state(exports_, result) {
  const bytes = stateBytes(exports_);
  return {
    result: `0x${BigInt.asUintN(64, result).toString(16).padStart(16, "0")}`,
    stateSha256: sha256(bytes),
    fingerprint: sha256(Buffer.concat([
      bytes,
      Buffer.from(BigInt.asUintN(64, result).toString(16).padStart(16, "0")),
    ])),
  };
}

function runPair(controlName, treatmentName, argument) {
  const control = fresh();
  const treatment = fresh();
  control.reset_state();
  treatment.reset_state();
  treatment.reset_cache();
  const controlResult = argument === undefined
    ? control[controlName]()
    : control[controlName](argument);
  const treatmentResult = argument === undefined
    ? treatment[treatmentName]()
    : treatment[treatmentName](argument);
  const left = state(control, controlResult);
  const right = state(treatment, treatmentResult);
  return {
    equal: left.fingerprint === right.fingerprint,
    control: left,
    treatment: right,
    treatmentCache: {
      hits: treatment.cache_hits().toString(),
      misses: treatment.cache_misses().toString(),
    },
  };
}

const inspection = fresh();
const flatCount = inspection.flat_count();
const blockCount = inspection.block_count();
const records = [];
const raw = [];
for (let index = 0; index < flatCount; index++) {
  const values = Array.from({ length: 9 }, (_, field) => inspection.flat_field(index, field));
  const length = Number(values[5]);
  const bytes = Array.from({ length }, (_, byte) => inspection.flat_raw_byte(index, byte));
  raw.push(...bytes);
  records.push({
    index,
    op: Number(values[0]),
    rd: Number(values[1]),
    rs1: Number(values[2]),
    rs2: Number(values[3]),
    imm: BigInt.asIntN(64, values[4]).toString(),
    length,
    pc: `0x${values[6].toString(16)}`,
    block: Number(values[7]),
    position: Number(values[8]),
    raw: Buffer.from(bytes).toString("hex"),
  });
}

const blockRecords = Array.from({ length: blockCount }, (_, index) => {
  const pc = inspection.block_pc_at(index);
  return {
    index,
    pc: `0x${pc.toString(16)}`,
    pageOffset: Number(pc & 0xfffn),
    slot: inspection.cache_index_for_pc(pc),
    length: inspection.block_length_at(index),
  };
});
const op32Counts = Array(62).fill(0);
const compressedBucketCounts = Array(19).fill(0);
const compressedSemanticCounts = Array(32).fill(0);
for (const record of records) {
  if (record.op < 62) op32Counts[record.op]++;
  else {
    const bucket = inspection.compressed_bucket_for_op(record.op);
    compressedBucketCounts[bucket]++;
    compressedSemanticCounts[record.op - 62]++;
  }
}
const pageBandCounts = Array(16).fill(0);
for (const block of blockRecords) pageBandCounts[block.pageOffset >>> 8]++;
const branchOutcomes = { taken: 0, notTaken: 0 };
for (const record of records) {
  let taken = null;
  if (record.op === 4) taken = record.rs1 === record.rs2;
  else if (record.op === 5) taken = record.rs1 !== record.rs2;
  else if (record.op === 6 || record.op === 8) taken = record.rs1 === 30 && record.rs2 === 31;
  else if (record.op === 7 || record.op === 9) taken = !(record.rs1 === 30 && record.rs2 === 31);
  else if (record.op === 82) taken = record.rs1 === 8;
  else if (record.op === 83) taken = record.rs1 === 9;
  if (taken === true) branchOutcomes.taken++;
  else if (taken === false) branchOutcomes.notTaken++;
}

const zero = runPair("run_control", "run_treatment", 0);
const full = [1, 3, 257].map((rounds) => ({
  rounds,
  ...runPair("run_control", "run_treatment", rounds),
}));
const singleFailures = [];
for (let index = 0; index < flatCount; index++) {
  const pair = runPair("run_single_control", "run_single_treatment", index);
  if (!pair.equal) singleFailures.push({ index, op: records[index].op, pair });
}
const wrap = runPair("run_wrap_control", "run_wrap_treatment", 2);
const straddle = runPair("run_straddle_control", "run_straddle_treatment");

const oldControl = fresh();
oldControl.reset_state();
const oldState = state(oldControl, oldControl.run_mutation_control());
const treatmentMutation = fresh();
treatmentMutation.reset_cache();
treatmentMutation.reset_state();
const fillState = state(treatmentMutation, treatmentMutation.run_mutation_treatment());
treatmentMutation.set_mutation_version(1);
treatmentMutation.reset_state();
const staleState = state(treatmentMutation, treatmentMutation.run_mutation_treatment());
const newControl = fresh();
newControl.set_mutation_version(1);
newControl.reset_state();
const newState = state(newControl, newControl.run_mutation_control());
treatmentMutation.reset_state();
const generationBeforeFence = treatmentMutation.ifetch_generation_value();
treatmentMutation.fence_i();
const generationAfterFence = treatmentMutation.ifetch_generation_value();
const afterFenceState = state(
  treatmentMutation,
  treatmentMutation.run_mutation_treatment(),
);
const mutation = {
  fillMatchesOld: fillState.fingerprint === oldState.fingerprint,
  staleBeforeFenceMatchesOld: staleState.fingerprint === oldState.fingerprint,
  oldAndNewDiffer: oldState.fingerprint !== newState.fingerprint,
  afterFenceMatchesNew: afterFenceState.fingerprint === newState.fingerprint,
  generationBeforeFence: generationBeforeFence.toString(),
  generationAfterFence: generationAfterFence.toString(),
  cacheHits: treatmentMutation.cache_hits().toString(),
  cacheMisses: treatmentMutation.cache_misses().toString(),
};

const keyProbe = fresh();
keyProbe.reset_cache();
keyProbe.reset_state();
keyProbe.run_treatment(1);
const afterCold = {
  hits: keyProbe.cache_hits(),
  misses: keyProbe.cache_misses(),
};
keyProbe.reset_state();
keyProbe.run_treatment(1);
const afterHit = {
  hits: keyProbe.cache_hits(),
  misses: keyProbe.cache_misses(),
};
keyProbe.set_fetch_context(1n);
keyProbe.reset_state();
keyProbe.run_treatment(1);
const afterContext = {
  hits: keyProbe.cache_hits(),
  misses: keyProbe.cache_misses(),
};
keyProbe.bump_map_generation();
keyProbe.reset_state();
keyProbe.run_treatment(1);
const afterMap = {
  hits: keyProbe.cache_hits(),
  misses: keyProbe.cache_misses(),
};
keyProbe.fence_i();
keyProbe.reset_state();
keyProbe.run_treatment(1);
const afterFence = {
  hits: keyProbe.cache_hits(),
  misses: keyProbe.cache_misses(),
};
const keyProbeJson = Object.fromEntries(Object.entries({
  afterCold,
  afterHit,
  afterContext,
  afterMap,
  afterFence,
}).map(([name, counters]) => [name, {
  hits: counters.hits.toString(),
  misses: counters.misses.toString(),
}]));

const finalByBlock = new Map();
for (const record of records) finalByBlock.set(record.block, record);
const sourceBytes = readFileSync(source);
const selfBytes = readFileSync(self);
const buildCommandBytes = readFileSync(join(output, "BUILD-COMMANDS.txt"));
const checks = {
  deterministicModule: moduleBytes.equals(repeatBytes),
  validModule: WebAssembly.validate(moduleBytes),
  exactExports: requiredExports.every((name) => name in inspection),
  initialization: inspection.model_error() === 0,
  exactGeometry: inspection.cache_slots() === 64 &&
    inspection.block_limit() === 32 && inspection.normalized_record_bytes() === 32,
  exactInstructionCount: flatCount === 252,
  capacity: blockCount <= 64,
  blockLengths: blockRecords.every((block) => block.length >= 1 && block.length <= 32) &&
    blockRecords.reduce((sum, block) => sum + block.length, 0) === flatCount,
  blockBoundaries: blockRecords.every((block) => {
    const final = finalByBlock.get(block.index);
    return final && (inspection.control_for_op(final.op) === 1 || block.length === 32);
  }),
  noMainPageStraddles: records.every((record) =>
    (Number(BigInt(record.pc) & 0xfffn) + record.length) <= 4096),
  uniqueMainSlots: new Set(blockRecords.map((block) => block.slot)).size === blockCount,
  balancedPageBands: Math.max(...pageBandCounts) - Math.min(...pageBandCounts) <= 1,
  completeBalanced32BitClass: op32Counts.every((count) => count === 1),
  completeBalancedCompressedFamilies: compressedBucketCounts.every((count) => count === 10),
  completeCompressedSemanticClass: compressedSemanticCounts.every((count) => count >= 1),
  balancedConditionalOutcomes: branchOutcomes.taken === 13 && branchOutcomes.notTaken === 13,
  zeroState: zero.equal,
  everySingleOperation: singleFailures.length === 0,
  fullStates: full.every((sample) => sample.equal),
  cacheWrapState: wrap.equal && inspection.wrap_block_count() === 129,
  pageStraddleFallbackState: straddle.equal,
  mutationOldBeforeFenceNewAfter: Object.entries(mutation)
    .filter(([, value]) => typeof value === "boolean")
    .every(([, value]) => value) && generationAfterFence === generationBeforeFence + 1n,
  completeKeyInvalidation: afterCold.misses === BigInt(blockCount) && afterCold.hits === 0n &&
    afterHit.misses === BigInt(blockCount) && afterHit.hits === BigInt(blockCount) &&
    afterContext.misses === BigInt(blockCount * 2) &&
    afterMap.misses === BigInt(blockCount * 3) &&
    afterFence.misses === BigInt(blockCount * 4),
};
const pass = Object.values(checks).every(Boolean);

const report = {
  schema: 1,
  experiment: "G001 FENCE.I-coherent decoded-interpreter standalone model freeze",
  timingCollected: false,
  productionModified: false,
  compiler: {
    path: zig,
    version: spawnSync(zig, ["version"], { encoding: "utf8" }).stdout.trim(),
    sha256: sha256(readFileSync(zig)),
    arguments: commonArguments,
  },
  runtime: { node: process.version, v8: process.versions.v8 },
  artifacts: {
    source: { path: source, bytes: sourceBytes.length, sha256: sha256(sourceBytes) },
    preparationHarness: { path: self, bytes: selfBytes.length, sha256: sha256(selfBytes) },
    buildCommands: {
      path: join(output, "BUILD-COMMANDS.txt"),
      bytes: buildCommandBytes.length,
      sha256: sha256(buildCommandBytes),
    },
    model: { path: modulePath, bytes: moduleBytes.length, sha256: sha256(moduleBytes) },
    repeatModel: { path: repeatPath, bytes: repeatBytes.length, sha256: sha256(repeatBytes) },
  },
  stream: {
    instructions: flatCount,
    blocks: blockCount,
    rawBytes: raw.length,
    rawSha256: sha256(Buffer.from(raw)),
    normalizedSha256: sha256(Buffer.from(JSON.stringify(records))),
    op32Counts,
    compressedBucketCounts,
    compressedSemanticCounts,
    branchOutcomes,
    pageBandCounts,
    blockRecords,
  },
  correctness: {
    zero,
    full,
    singleFailures,
    wrap,
    straddle,
    mutation,
    keyProbe: keyProbeJson,
  },
  checks,
  pass,
  decision: pass
    ? "eligible-to-freeze-timing-harness-before-first-sample"
    : "stop-g001-before-timing",
};
const reportPath = join(output, "freeze.json");
writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
const sums = [
  source,
  self,
  join(output, "BUILD-COMMANDS.txt"),
  modulePath,
  repeatPath,
  reportPath,
].map((path) => `${sha256(readFileSync(path))}  ${path}`).join("\n");
writeFileSync(join(output, "SHA256SUMS"), `${sums}\n`, { flag: "wx" });

console.log(`G001 model: ${moduleBytes.length} bytes ${sha256(moduleBytes)}`);
console.log(`G001 stream: ${flatCount} instructions in ${blockCount} blocks`);
console.log(`G001 raw stream: ${sha256(Buffer.from(raw))}`);
console.log(`G001 normalized stream: ${sha256(Buffer.from(JSON.stringify(records)))}`);
console.log(`G001 preparation checks pass=${pass}`);
console.log(`G001 freeze report: ${reportPath}`);
if (!pass) process.exitCode = 1;
