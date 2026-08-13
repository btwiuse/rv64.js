#!/usr/bin/env node

// Build, inspect, and correctness-check G002 without collecting performance.
// The resulting identities must be copied into the frozen timing harness
// before that harness is executed for the first time.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
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
const source = join(root, "crates/rv64-dbt/examples/g002_local_gpr_model.rs");
const outputArgument = process.argv.find((argument) => argument.startsWith("--output="));
const output = resolve(outputArgument?.slice("--output=".length) ??
  join(root, "target/bench/interpreter-g002-model-v1"));

if (existsSync(output)) {
  throw new Error(`refusing to replace G002 model directory: ${output}`);
}
mkdirSync(output, { recursive: true });

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const quote = (word) => /^[A-Za-z0-9_./,:=+-]+$/.test(word)
  ? word
  : `'${word.replaceAll("'", "'\\''")}'`;
const generationA = join(output, "generation-a");
const generationB = join(output, "generation-b");

function generate(directory) {
  const command = [
    "cargo", "run", "-q", "-p", "rv64-dbt", "--example",
    "g002_local_gpr_model", "--", directory,
  ];
  const child = spawnSync(command[0], command.slice(1), {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 16 << 20,
  });
  if (child.status !== 0) {
    throw new Error(child.stderr || child.stdout || "G002 generation failed");
  }
  return command;
}

const commands = [generate(generationA), generate(generationB)];
const artifactNames = Object.freeze([
  "model.wasm",
  "records.bin",
  "schedule.tsv",
  "shape.tsv",
]);
for (const name of artifactNames) {
  copyFileSync(join(generationA, name), join(output, name));
}
writeFileSync(
  join(output, "BUILD-COMMANDS.txt"),
  `${commands.map((command) => command.map(quote).join(" ")).join("\n")}\n`,
  { flag: "wx" },
);

const modelPath = join(output, "model.wasm");
const recordsPath = join(output, "records.bin");
const schedulePath = join(output, "schedule.tsv");
const shapePath = join(output, "shape.tsv");
const modelBytes = readFileSync(modelPath);
const repeatModelBytes = readFileSync(join(generationB, "model.wasm"));
const recordsBytes = readFileSync(recordsPath);
const repeatRecordsBytes = readFileSync(join(generationB, "records.bin"));
const scheduleBytes = readFileSync(schedulePath);
const repeatScheduleBytes = readFileSync(join(generationB, "schedule.tsv"));
const shapeBytes = readFileSync(shapePath);
const repeatShapeBytes = readFileSync(join(generationB, "shape.tsv"));

if (!WebAssembly.validate(modelBytes)) throw new Error("G002 model does not validate");
const module = new WebAssembly.Module(modelBytes);
const requiredExports = Object.freeze([
  "memory",
  "run_control",
  "run_treatment",
  "reset_state",
  "state_word",
  "record_at",
  "record_count",
  "state_word_count",
]);

function fresh() {
  const instance = new WebAssembly.Instance(module);
  const missing = requiredExports.filter((name) => !(name in instance.exports));
  if (missing.length) throw new Error(`G002 model exports missing: ${missing.join(", ")}`);
  return instance.exports;
}

function state(exports_, result) {
  const words = Number(exports_.state_word_count());
  const bytes = Buffer.alloc(words * 8);
  for (let index = 0; index < words; index++) {
    bytes.writeBigUInt64LE(BigInt.asUintN(64, exports_.state_word(index)), index * 8);
  }
  const resultHex = BigInt.asUintN(64, result).toString(16).padStart(16, "0");
  return {
    result: `0x${resultHex}`,
    stateSha256: sha256(bytes),
    fingerprint: sha256(Buffer.concat([bytes, Buffer.from(resultHex)])),
    x0: `0x${bytes.readBigUInt64LE(0).toString(16)}`,
  };
}

function runPair(rounds, start, count) {
  const control = fresh();
  const treatment = fresh();
  control.reset_state();
  treatment.reset_state();
  const controlState = state(
    control,
    control.run_control(rounds, start, count),
  );
  const treatmentState = state(
    treatment,
    treatment.run_treatment(rounds, start, count),
  );
  return {
    rounds,
    start,
    count,
    equal: controlState.fingerprint === treatmentState.fingerprint,
    control: controlState,
    treatment: treatmentState,
  };
}

const inspection = fresh();
const recordCount = Number(inspection.record_count());
const stateWordCount = Number(inspection.state_word_count());
const records = [];
const roleCounts = {
  rd: Array(32).fill(0),
  rs1: Array(32).fill(0),
  rs2: Array(32).fill(0),
};
const rs1ByRd = Array.from({ length: 32 }, () => Array(32).fill(0));
let formulaMatches = true;
for (let index = 0; index < recordCount; index++) {
  const packed = Number(inspection.record_at(index)) >>> 0;
  const lo = index & 31;
  const hi = index >>> 5;
  const expected = (
    ((hi + 5 * lo + 1) & 31) |
    (((7 * hi + 13 * lo + 3) & 31) << 5) |
    (lo << 10) |
    ((((17 * hi + 11 * lo) % 63) + 1) << 15) |
    (((0x5a3 * hi + 0x31d * lo + 0x155) & 0x7ff) << 21)
  ) >>> 0;
  formulaMatches &&= packed === expected;
  const record = {
    index,
    packed: `0x${packed.toString(16).padStart(8, "0")}`,
    rd: (packed >>> 10) & 31,
    rs1: packed & 31,
    rs2: (packed >>> 5) & 31,
    shift: (packed >>> 15) & 63,
    salt: packed >>> 21,
  };
  records.push(record);
  roleCounts.rd[record.rd]++;
  roleCounts.rs1[record.rs1]++;
  roleCounts.rs2[record.rs2]++;
  rs1ByRd[record.rd][record.rs1]++;
}

const decodedRecords = Buffer.alloc(recordCount * 4);
for (let index = 0; index < recordCount; index++) {
  decodedRecords.writeUInt32LE(Number(inspection.record_at(index)) >>> 0, index * 4);
}

const shapeLines = shapeBytes.toString("utf8").trimEnd().split("\n");
const shapeHeader = shapeLines[0].split("\t");
const shapes = Object.fromEntries(shapeLines.slice(1).map((line) => {
  const values = line.split("\t");
  return [values[0], Object.fromEntries(shapeHeader.slice(1).map((name, index) => [
    name,
    name === "br_tables" ? values[index + 1] : Number(values[index + 1]),
  ]))];
}));

const zero = runPair(0, 0, recordCount);
const singleFailures = [];
for (let index = 0; index < recordCount; index++) {
  const pair = runPair(1, index, 1);
  if (!pair.equal) singleFailures.push({ index, pair });
}
const full = [1, 257].map((rounds) => runPair(rounds, 0, recordCount));
const windows = [
  runPair(37, 500, 40),
  runPair(19, 1_000, 24),
];

const sourceBytes = readFileSync(source);
const selfBytes = readFileSync(self);
const buildCommandsBytes = readFileSync(join(output, "BUILD-COMMANDS.txt"));
const checks = {
  deterministicModule: modelBytes.equals(repeatModelBytes),
  deterministicRecords: recordsBytes.equals(repeatRecordsBytes),
  deterministicSchedule: scheduleBytes.equals(repeatScheduleBytes),
  deterministicShape: shapeBytes.equals(repeatShapeBytes),
  validModule: WebAssembly.validate(modelBytes),
  exactExports: requiredExports.every((name) => name in inspection),
  exactRecordCount: recordCount === 1_024 && recordsBytes.length === 4_096,
  exactStateWords: stateWordCount === 32,
  embeddedRecords: recordsBytes.equals(decodedRecords),
  frozenFormula: formulaMatches,
  roleBalance: Object.values(roleCounts).every((counts) => counts.every((count) => count === 32)),
  rs1PermutationPerRd: rs1ByRd.every((counts) => counts.every((count) => count === 1)),
  shiftsNonzero: records.every((record) => record.shift >= 1 && record.shift <= 63),
  controlShape: shapes.control?.i32_locals === 6 &&
    shapes.control?.i64_locals === 4 &&
    shapes.control?.i32_loads === 1 &&
    shapes.control?.i64_loads === 34 &&
    shapes.control?.i64_stores === 1 &&
    shapes.control?.br_tables === "" &&
    shapes.control?.calls === 0,
  treatmentShape: shapes.treatment?.i32_locals === 6 &&
    shapes.treatment?.i64_locals === 35 &&
    shapes.treatment?.i32_loads === 1 &&
    shapes.treatment?.i64_loads === 63 &&
    shapes.treatment?.i64_stores === 31 &&
    shapes.treatment?.br_tables === "33,33,33" &&
    shapes.treatment?.calls === 0,
  exactLocalDelta: shapes.treatment?.i64_locals - shapes.control?.i64_locals === 31,
  sharedLogicalDriver: sourceBytes.includes("fn driver(storage: Storage)") &&
    sourceBytes.includes("dynamic_state_get(&mut function, storage") &&
    sourceBytes.includes("dynamic_state_set(") &&
    sourceBytes.includes("code.function(&driver(Storage::Memory))") &&
    sourceBytes.includes("code.function(&driver(Storage::Local))"),
  zeroState: zero.equal && zero.control.x0 === "0x0" && zero.treatment.x0 === "0x0",
  everySingleRecord: singleFailures.length === 0,
  fullStates: full.every((sample) => sample.equal),
  windowStates: windows.every((sample) => sample.equal),
};
const pass = Object.values(checks).every(Boolean);

const report = {
  schema: 1,
  experiment: "G002 complete local-GPR standalone model freeze",
  timingCollected: false,
  productionModified: false,
  runtime: { node: process.version, v8: process.versions.v8 },
  artifacts: {
    source: { path: source, bytes: sourceBytes.length, sha256: sha256(sourceBytes) },
    preparationHarness: { path: self, bytes: selfBytes.length, sha256: sha256(selfBytes) },
    buildCommands: {
      path: join(output, "BUILD-COMMANDS.txt"),
      bytes: buildCommandsBytes.length,
      sha256: sha256(buildCommandsBytes),
    },
    model: { path: modelPath, bytes: modelBytes.length, sha256: sha256(modelBytes) },
    records: { path: recordsPath, bytes: recordsBytes.length, sha256: sha256(recordsBytes) },
    schedule: { path: schedulePath, bytes: scheduleBytes.length, sha256: sha256(scheduleBytes) },
    shape: { path: shapePath, bytes: shapeBytes.length, sha256: sha256(shapeBytes) },
  },
  schedule: {
    records: recordCount,
    roleCounts,
    rs1PermutationPerRd: rs1ByRd.every((counts) => counts.every((count) => count === 1)),
    normalizedSha256: sha256(Buffer.from(JSON.stringify(records))),
  },
  shapes,
  correctness: { zero, singleFailures, full, windows },
  checks,
  pass,
  decision: pass
    ? "eligible-to-freeze-g002-timing-harness-before-first-sample"
    : "stop-g002-before-timing",
};
const reportPath = join(output, "freeze.json");
writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
const manifestPaths = [
  source,
  self,
  join(output, "BUILD-COMMANDS.txt"),
  modelPath,
  recordsPath,
  schedulePath,
  shapePath,
  reportPath,
];
writeFileSync(
  join(output, "SHA256SUMS"),
  `${manifestPaths.map((path) => `${sha256(readFileSync(path))}  ${path}`).join("\n")}\n`,
  { flag: "wx" },
);

console.log(`G002 model: ${modelBytes.length} bytes ${sha256(modelBytes)}`);
console.log(`G002 records: ${recordCount} ${sha256(recordsBytes)}`);
console.log(`G002 normalized schedule: ${report.schedule.normalizedSha256}`);
console.log(`G002 preparation checks pass=${pass}`);
console.log(`G002 freeze report: ${reportPath}`);
if (!pass) process.exitCode = 1;
