#!/usr/bin/env node

// Construct and correctness-check G003's finite standard LLVM optimization
// level screen without collecting any performance sample.

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
  join(root, "target/bench/interpreter-g003-model-v1"));
const zig = process.env.ZIG || "/home/linuxbrew/.linuxbrew/bin/zig";
const levels = Object.freeze([
  { name: "o1", flag: "-O1" },
  { name: "o2", flag: "-O2" },
  { name: "o3", flag: "-O3" },
  { name: "os", flag: "-Os" },
  { name: "oz", flag: "-Oz" },
]);

if (existsSync(output)) {
  throw new Error(`refusing to replace G003 model directory: ${output}`);
}
mkdirSync(output, { recursive: true });

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const quote = (word) => /^[A-Za-z0-9_./,:=+-]+$/.test(word)
  ? word
  : `'${word.replaceAll("'", "'\\''")}'`;
const commonArguments = Object.freeze([
  "cc",
  "-target", "wasm32-freestanding",
  "-nostdlib",
  "-Wl,--no-entry",
  "-Wl,--export-memory",
  "-Wl,--strip-all",
]);

function compile(level, path) {
  const arguments_ = [...commonArguments, level.flag, "-o", path, source];
  const child = spawnSync(zig, arguments_, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 16 << 20,
  });
  if (child.status !== 0) {
    throw new Error(child.stderr || child.stdout || `G003 ${level.name} compilation failed`);
  }
  return [zig, ...arguments_];
}

const commands = [];
const artifacts = {};
for (const level of levels) {
  const directory = join(output, level.name);
  mkdirSync(directory, { recursive: true });
  const modelPath = join(directory, "model.wasm");
  const repeatPath = join(directory, "model-repeat.wasm");
  commands.push(compile(level, modelPath), compile(level, repeatPath));
  artifacts[level.name] = {
    level,
    modelPath,
    repeatPath,
    bytes: readFileSync(modelPath),
    repeatBytes: readFileSync(repeatPath),
  };
}
writeFileSync(
  join(output, "BUILD-COMMANDS.txt"),
  `${commands.map((command) => command.map(quote).join(" ")).join("\n")}\n`,
  { flag: "wx" },
);

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
  "model_error_index",
  "model_error_expected",
  "model_error_actual",
  "wrap_block_count",
  "cache_slots",
  "block_limit",
  "normalized_record_bytes",
]);

function fresh(bytes) {
  if (!WebAssembly.validate(bytes)) throw new Error("G003 model does not validate");
  const module = new WebAssembly.Module(bytes);
  const exports_ = new WebAssembly.Instance(module).exports;
  const missing = requiredExports.filter((name) => !(name in exports_));
  if (missing.length) throw new Error(`G003 exports missing: ${missing.join(", ")}`);
  const error = exports_.init_model();
  if (error !== 0 || exports_.model_error() !== 0) {
    throw new Error(`G003 model initialization failed: ${error}`);
  }
  return exports_;
}

function stateBytes(exports_) {
  const count = Number(exports_.state_word_count());
  const bytes = Buffer.alloc(count * 8);
  for (let index = 0; index < count; index++) {
    bytes.writeBigUInt64LE(BigInt.asUintN(64, exports_.state_word(index)), index * 8);
  }
  return bytes;
}

function state(exports_, result) {
  const bytes = stateBytes(exports_);
  const resultHex = BigInt.asUintN(64, result).toString(16).padStart(16, "0");
  return {
    result: `0x${resultHex}`,
    stateSha256: sha256(bytes),
    fingerprint: sha256(Buffer.concat([bytes, Buffer.from(resultHex)])),
  };
}

function pair(controlBytes, candidateBytes, functionName, argument, configure) {
  const control = fresh(controlBytes);
  const candidate = fresh(candidateBytes);
  configure?.(control);
  configure?.(candidate);
  control.reset_state();
  candidate.reset_state();
  const controlResult = argument === undefined
    ? control[functionName]()
    : control[functionName](argument);
  const candidateResult = argument === undefined
    ? candidate[functionName]()
    : candidate[functionName](argument);
  const left = state(control, controlResult);
  const right = state(candidate, candidateResult);
  return { equal: left.fingerprint === right.fingerprint, control: left, candidate: right };
}

function readUleb(bytes, cursor) {
  let value = 0;
  let shift = 0;
  while (true) {
    if (cursor.offset >= bytes.length || shift > 35) throw new Error("invalid Wasm uleb");
    const byte = bytes[cursor.offset++];
    value += (byte & 0x7f) * 2 ** shift;
    if ((byte & 0x80) === 0) return value;
    shift += 7;
  }
}

function codeShape(bytes) {
  const cursor = { offset: 8 };
  while (cursor.offset < bytes.length) {
    const section = bytes[cursor.offset++];
    const sectionBytes = readUleb(bytes, cursor);
    const start = cursor.offset;
    const end = start + sectionBytes;
    if (end > bytes.length) throw new Error("Wasm section exceeds module");
    if (section !== 10) {
      cursor.offset = end;
      continue;
    }
    const functions = readUleb(bytes, cursor);
    let totalBodyBytes = 0;
    let totalLocals = 0;
    const localTypes = {};
    for (let functionIndex = 0; functionIndex < functions; functionIndex++) {
      const bodyBytes = readUleb(bytes, cursor);
      const bodyEnd = cursor.offset + bodyBytes;
      totalBodyBytes += bodyBytes;
      const groups = readUleb(bytes, cursor);
      for (let group = 0; group < groups; group++) {
        const count = readUleb(bytes, cursor);
        const type = bytes[cursor.offset++];
        totalLocals += count;
        const key = `0x${type.toString(16)}`;
        localTypes[key] = (localTypes[key] || 0) + count;
      }
      cursor.offset = bodyEnd;
    }
    if (cursor.offset !== end) throw new Error("G003 CODE section boundary mismatch");
    return { sectionBytes, functions, totalBodyBytes, totalLocals, localTypes };
  }
  throw new Error("G003 model has no CODE section");
}

function disassemblyShape(path) {
  const child = spawnSync("llvm-objdump", ["-d", path], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 << 20,
  });
  if (child.status !== 0) throw new Error(child.stderr || "llvm-objdump failed");
  const text = child.stdout;
  const count = (pattern) => [...text.matchAll(pattern)].length;
  return {
    sha256: sha256(Buffer.from(text)),
    instructions: count(/^\s*[0-9a-f]+:\s+/gm),
    calls: count(/\bcall(?:_indirect)?\b/g),
    branchTables: count(/\bbr_table\b/g),
    i32Loads: count(/\bi32\.load(?:8_u|16_u)?\b/g),
    i64Loads: count(/\bi64\.load\b/g),
    i32Stores: count(/\bi32\.store(?:8|16)?\b/g),
    i64Stores: count(/\bi64\.store\b/g),
  };
}

const controlBytes = artifacts.o3.bytes;
const inspection = fresh(controlBytes);
const flatCount = Number(inspection.flat_count());
const records = [];
const raw = [];
for (let index = 0; index < flatCount; index++) {
  const values = Array.from({ length: 9 }, (_, field) => inspection.flat_field(index, field));
  const length = Number(values[5]);
  const bytes = Array.from({ length }, (_, byte) => Number(inspection.flat_raw_byte(index, byte)));
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
const rawSha256 = sha256(Buffer.from(raw));
const normalizedSha256 = sha256(Buffer.from(JSON.stringify(records)));

const correctness = {};
const shapes = {};
for (const level of levels) {
  const artifact = artifacts[level.name];
  const module = new WebAssembly.Module(artifact.bytes);
  const levelInspection = fresh(artifact.bytes);
  const levelRecords = [];
  const levelRaw = [];
  for (let index = 0; index < Number(levelInspection.flat_count()); index++) {
    const values = Array.from({ length: 9 }, (_, field) => levelInspection.flat_field(index, field));
    const length = Number(values[5]);
    const bytes = Array.from({ length }, (_, byte) =>
      Number(levelInspection.flat_raw_byte(index, byte)));
    levelRaw.push(...bytes);
    levelRecords.push({
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

  const singleFailures = [];
  for (let index = 0; index < flatCount; index++) {
    const result = pair(controlBytes, artifact.bytes, "run_single_control", index);
    if (!result.equal) singleFailures.push({ index, result });
  }
  const full = [0, 1, 3, 257].map((rounds) => ({
    rounds,
    ...pair(controlBytes, artifact.bytes, "run_control", rounds),
  }));
  const wrap = pair(controlBytes, artifact.bytes, "run_wrap_control", 2);
  const straddle = pair(controlBytes, artifact.bytes, "run_straddle_control");
  const mutation = [0, 1].map((version) => ({
    version,
    ...pair(
      controlBytes,
      artifact.bytes,
      "run_mutation_control",
      undefined,
      (exports_) => exports_.set_mutation_version(version),
    ),
  }));
  correctness[level.name] = {
    full,
    singleFailures,
    wrap,
    straddle,
    mutation,
    pass: full.every((sample) => sample.equal) && singleFailures.length === 0 &&
      wrap.equal && straddle.equal && mutation.every((sample) => sample.equal),
  };
  shapes[level.name] = {
    flag: level.flag,
    bytes: artifact.bytes.length,
    sha256: sha256(artifact.bytes),
    deterministic: artifact.bytes.equals(artifact.repeatBytes),
    repeatSha256: sha256(artifact.repeatBytes),
    valid: WebAssembly.validate(artifact.bytes),
    imports: WebAssembly.Module.imports(module),
    exports: WebAssembly.Module.exports(module),
    code: codeShape(artifact.bytes),
    disassembly: disassemblyShape(artifact.modelPath),
    rawSha256: sha256(Buffer.from(levelRaw)),
    normalizedSha256: sha256(Buffer.from(JSON.stringify(levelRecords))),
  };
}

const sourceBytes = readFileSync(source);
const selfBytes = readFileSync(self);
const buildCommandsBytes = readFileSync(join(output, "BUILD-COMMANDS.txt"));
const checks = {
  exactSource: sha256(sourceBytes) ===
    "27bfc111495af24e39a4f2c3e7233ac690a20e2456099dd2cab3a1e2453a0128",
  exactLevels: levels.map(({ name, flag }) => `${name}:${flag}`).join(",") ===
    "o1:-O1,o2:-O2,o3:-O3,os:-Os,oz:-Oz",
  deterministicArtifacts: Object.values(shapes).every((shape) => shape.deterministic),
  validArtifacts: Object.values(shapes).every((shape) => shape.valid),
  exactExports: Object.values(shapes).every((shape) =>
    shape.exports.map(({ name }) => name).sort().join(",") ===
      [...requiredExports].sort().join(",")),
  noImports: Object.values(shapes).every((shape) => shape.imports.length === 0),
  exactO3: shapes.o3.sha256 ===
    "63f2fb590d20260c01d55186c53d8b38f9722f6798cdba6a40846de87f400026",
  exactRawStream: rawSha256 ===
    "987912d44c5d5b1f25ca26f57ed298ba9d21c1f8e8bef6ae7e535b87a2315c0f" &&
    Object.values(shapes).every((shape) => shape.rawSha256 === rawSha256),
  exactNormalizedStream: normalizedSha256 ===
    "52ed0a9f402bc8e66a038d852f1afd65336b031d9233d15844c38cf320f5284a" &&
    Object.values(shapes).every((shape) => shape.normalizedSha256 === normalizedSha256),
  exactInstructionCount: flatCount === 252,
  completeCorrectness: Object.values(correctness).every((result) => result.pass),
};
const pass = Object.values(checks).every(Boolean);

const report = {
  schema: 1,
  experiment: "G003 finite standard LLVM optimization-level model freeze",
  timingCollected: false,
  productionModified: false,
  compiler: {
    path: zig,
    version: spawnSync(zig, ["version"], { encoding: "utf8" }).stdout.trim(),
    sha256: sha256(readFileSync(zig)),
    commonArguments,
    levels,
  },
  artifacts: {
    source: { path: source, bytes: sourceBytes.length, sha256: sha256(sourceBytes) },
    preparationHarness: { path: self, bytes: selfBytes.length, sha256: sha256(selfBytes) },
    buildCommands: {
      path: join(output, "BUILD-COMMANDS.txt"),
      bytes: buildCommandsBytes.length,
      sha256: sha256(buildCommandsBytes),
    },
  },
  stream: { instructions: flatCount, rawSha256, normalizedSha256 },
  shapes,
  correctness,
  checks,
  pass,
  decision: pass
    ? "eligible-to-freeze-g003-timing-harness-before-first-sample"
    : "stop-g003-before-timing",
};
const reportPath = join(output, "freeze.json");
writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
const manifestPaths = [source, self, join(output, "BUILD-COMMANDS.txt")];
for (const level of levels) {
  manifestPaths.push(artifacts[level.name].modelPath, artifacts[level.name].repeatPath);
}
manifestPaths.push(reportPath);
writeFileSync(
  join(output, "SHA256SUMS"),
  `${manifestPaths.map((path) => `${sha256(readFileSync(path))}  ${path}`).join("\n")}\n`,
  { flag: "wx" },
);

for (const level of levels) {
  const shape = shapes[level.name];
  console.log(`G003 ${level.name}: ${shape.bytes} bytes ${shape.sha256}`);
}
console.log(`G003 raw stream: ${rawSha256}`);
console.log(`G003 normalized stream: ${normalizedSha256}`);
console.log(`G003 preparation checks pass=${pass}`);
console.log(`G003 freeze report: ${reportPath}`);
if (!pass) process.exitCode = 1;
