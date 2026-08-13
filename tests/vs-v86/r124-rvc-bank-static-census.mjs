#!/usr/bin/env node

// R124 Gate A1: project the one frozen RV64C-bank residency rule over the
// immutable real-region corpus. This emits no candidate and measures no time.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const primary = resolve(process.argv[2] ||
  join(root, "target/bench/r111-partition-model/real-region-c"));
const repeat = resolve(process.argv[3] ||
  join(root, "target/bench/r111-partition-model/real-region-d"));
const output = resolve(process.argv[4] ||
  join(root, "target/bench/r124-rvc-bank-hybrid/static-census-a.json"));

const X0 = 1;
const RESIDENT_X = (X0 | (1 << 1) | (1 << 2) | (0xff << 8)) >>> 0;
const ARCH_X = 0xffff_fffe >>> 0;
const MATERIALIZED_X = (ARCH_X & ~RESIDENT_X) >>> 0;
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function popcount(input) {
  let value = input >>> 0;
  let count = 0;
  while (value !== 0) {
    value &= value - 1;
    count++;
  }
  return count;
}

function hex(mask) {
  return (mask >>> 0).toString(16).padStart(8, "0");
}

function parseTsv(path) {
  const lines = readFileSync(path, "utf8").trimEnd().split("\n");
  const columns = lines.shift().split("\t");
  return lines.map((line) => Object.fromEntries(
    line.split("\t").map((value, index) => [columns[index], value]),
  ));
}

function readUleb(bytes, cursor) {
  let value = 0;
  let shift = 0;
  for (;;) {
    assert.ok(cursor.offset < bytes.length && shift <= 35, "invalid uleb");
    const byte = bytes[cursor.offset++];
    value += (byte & 0x7f) * 2 ** shift;
    if ((byte & 0x80) === 0) return value;
    shift += 7;
  }
}

function functionLocals(bytes, label) {
  assert.ok(WebAssembly.validate(bytes), `${label}: invalid Wasm`);
  assert.equal(bytes.readUInt32LE(0), 0x6d736100, `${label}: bad magic`);
  const cursor = { offset: 8 };
  while (cursor.offset < bytes.length) {
    const id = bytes[cursor.offset++];
    const size = readUleb(bytes, cursor);
    const end = cursor.offset + size;
    assert.ok(end <= bytes.length, `${label}: section outside module`);
    if (id !== 10) {
      cursor.offset = end;
      continue;
    }
    const functions = readUleb(bytes, cursor);
    assert.equal(functions, 1, `${label}: expected one defined function`);
    const bodySize = readUleb(bytes, cursor);
    const bodyEnd = cursor.offset + bodySize;
    const groups = readUleb(bytes, cursor);
    let total = 0;
    let i32 = 0;
    let i64 = 0;
    let v128 = 0;
    for (let group = 0; group < groups; group++) {
      const amount = readUleb(bytes, cursor);
      const type = bytes[cursor.offset++];
      total += amount;
      if (type === 0x7f) i32 += amount;
      else if (type === 0x7e) i64 += amount;
      else if (type === 0x7b) v128 += amount;
      else throw new Error(`${label}: unexpected local type 0x${type.toString(16)}`);
    }
    assert.ok(cursor.offset <= bodyEnd && bodyEnd <= end, `${label}: invalid body`);
    return { functions, total, i32, i64, v128 };
  }
  throw new Error(`${label}: no code section`);
}

function runSelftest() {
  assert.equal(hex(RESIDENT_X), "0000ff07");
  assert.equal(popcount(RESIDENT_X & ARCH_X), 10);
  assert.equal(popcount(MATERIALIZED_X), 21);
  assert.equal(popcount(0xf0f0_0001), 9);
  assert.equal((RESIDENT_X | MATERIALIZED_X) >>> 0, 0xffff_ffff);
  assert.equal((RESIDENT_X & MATERIALIZED_X) >>> 0, 0);
  process.stdout.write("R124 static census selftest: PASS\n");
}

if (process.argv.includes("--selftest")) {
  runSelftest();
  process.exit(0);
}

const primaryManifestBytes = readFileSync(join(primary, "manifest.tsv"));
const repeatManifestBytes = readFileSync(join(repeat, "manifest.tsv"));
const primaryShapesBytes = readFileSync(join(primary, "member-shapes.tsv"));
const repeatShapesBytes = readFileSync(join(repeat, "member-shapes.tsv"));
assert.ok(primaryManifestBytes.equals(repeatManifestBytes), "manifest repeat differs");
assert.ok(primaryShapesBytes.equals(repeatShapesBytes), "member shapes repeat differs");

const manifest = parseTsv(join(primary, "manifest.tsv"));
const shapes = parseTsv(join(primary, "member-shapes.tsv"));
const modes = [...new Set(manifest.map((row) => row.mode))].sort();
assert.equal(manifest.length, 336, "expected 56 regions x 6 modes");
assert.deepEqual(modes, ["direct", "eager", "lazy", "memory", "structured", "tailcall"]);

const structured = manifest.filter((row) => row.mode === "structured");
const eager = new Map(manifest.filter((row) => row.mode === "eager")
  .map((row) => [row.id, row]));
assert.equal(structured.length, 56, "expected 56 structured modules");
assert.equal(shapes.length, 6258, "expected 6,258 members");

const shapesByRegion = new Map();
for (const row of shapes) {
  const rows = shapesByRegion.get(row.id) || [];
  rows.push({
    member: Number(row.member),
    readX: Number.parseInt(row.read_x, 16) >>> 0,
    writeX: Number.parseInt(row.write_x, 16) >>> 0,
    readF: Number.parseInt(row.read_f, 16) >>> 0,
    writeF: Number.parseInt(row.write_f, 16) >>> 0,
    readFcsr: row.read_fcsr === "1",
    writeFcsr: row.write_fcsr === "1",
  });
  shapesByRegion.set(row.id, rows);
}

const regions = [];
let validatedModules = 0;
let repeatedModules = 0;
for (const row of structured) {
  const members = (shapesByRegion.get(row.id) || []).sort((a, b) => a.member - b.member);
  assert.equal(members.length, Number(row.entries), `${row.id}: member count`);
  const eagerRow = eager.get(row.id);
  assert.ok(eagerRow, `${row.id}: missing eager weight`);
  let readX = 0;
  let writeX = 0;
  let readF = 0;
  let writeF = 0;
  let needFcsr = false;
  let writeFcsr = false;
  let materializedMemberReads = 0;
  let materializedMemberOutputs = 0;
  let residentMemberReads = 0;
  let residentMemberOutputs = 0;
  for (const member of members) {
    readX |= member.readX;
    writeX |= member.writeX;
    readF |= member.readF;
    writeF |= member.writeF;
    needFcsr ||= member.readFcsr || member.writeFcsr;
    writeFcsr ||= member.writeFcsr;
    materializedMemberReads += popcount(member.readX & MATERIALIZED_X);
    materializedMemberOutputs += popcount(member.writeX & MATERIALIZED_X);
    residentMemberReads += popcount(member.readX & RESIDENT_X & ARCH_X);
    residentMemberOutputs += popcount(member.writeX & RESIDENT_X & ARCH_X);
  }
  readX &= ARCH_X;
  writeX &= ARCH_X;
  const needX = (readX | writeX) >>> 0;
  const needF = (readF | writeF) >>> 0;
  const residentNeedX = needX & RESIDENT_X;
  const materializedNeedX = needX & MATERIALIZED_X;
  assert.equal((residentNeedX | materializedNeedX) >>> 0, needX,
    `${row.id}: unclassified integer state`);
  assert.equal((residentNeedX & materializedNeedX) >>> 0, 0,
    `${row.id}: overlapping integer state`);

  const bytes = readFileSync(join(primary, row.wasm));
  const repeatBytes = readFileSync(join(repeat, row.wasm));
  assert.ok(bytes.equals(repeatBytes), `${row.id}: repeated module differs`);
  repeatedModules++;
  const locals = functionLocals(bytes, row.wasm);
  validatedModules++;
  const removedI64Locals = popcount(materializedNeedX);
  assert.ok(locals.i64 >= removedI64Locals, `${row.id}: projected i64 underflow`);
  const projectedLocals = {
    functions: 1,
    total: locals.total - removedI64Locals,
    i32: locals.i32,
    i64: locals.i64 - removedI64Locals,
    v128: locals.v128,
  };
  regions.push({
    id: row.id,
    workload: row.workload,
    function: row.function,
    pages: Number(row.pages),
    leaderCap: Number(row.leader_cap),
    members: members.length,
    eagerBytes: Number(eagerRow.bytes),
    structuredBytes: bytes.length,
    structuredSha256: sha256(bytes),
    masks: {
      readX: hex(readX),
      writeX: hex(writeX),
      needX: hex(needX),
      residentNeedX: hex(residentNeedX),
      materializedNeedX: hex(materializedNeedX),
      readF: hex(readF),
      writeF: hex(writeF),
      needF: hex(needF),
      needFcsr,
      writeFcsr,
    },
    stateValues: {
      integerCurrent: popcount(needX),
      integerResident: popcount(residentNeedX),
      integerMaterialized: removedI64Locals,
      fpCurrent: popcount(needF),
      fcsrCurrent: Number(needFcsr),
    },
    locals: { current: locals, projected: projectedLocals, removedI64Locals },
    staticOperations: {
      currentBoundaryEntryLoads: popcount(needX),
      currentBoundaryExitStores: popcount(writeX),
      residentBoundaryEntryLoads: popcount(needX & RESIDENT_X),
      residentBoundaryExitStores: popcount(writeX & RESIDENT_X),
      materializedMemberReads,
      materializedMemberOutputs,
      residentMemberReads,
      residentMemberOutputs,
    },
  });
}

const weighted = (selector) => {
  const denominator = regions.reduce((sum, row) => sum + row.eagerBytes, 0);
  return regions.reduce((sum, row) => sum + row.eagerBytes * selector(row), 0) / denominator;
};
const currentLocals = weighted((row) => row.locals.current.total);
const projectedLocals = weighted((row) => row.locals.projected.total);
const localReduction = 1 - projectedLocals / currentLocals;
const metrics = {
  regions: regions.length,
  members: regions.reduce((sum, row) => sum + row.members, 0),
  validatedModules,
  repeatedModules,
  eagerBytes: regions.reduce((sum, row) => sum + row.eagerBytes, 0),
  eagerByteWeightedCurrentLocals: currentLocals,
  eagerByteWeightedProjectedLocals: projectedLocals,
  eagerByteWeightedLocalReduction: localReduction,
  eagerByteWeightedCurrentIntegerState: weighted((row) => row.stateValues.integerCurrent),
  eagerByteWeightedResidentIntegerState: weighted((row) => row.stateValues.integerResident),
  eagerByteWeightedMaterializedIntegerState: weighted((row) =>
    row.stateValues.integerMaterialized),
  totalStaticCurrentBoundaryOperations: regions.reduce((sum, row) => sum +
    row.staticOperations.currentBoundaryEntryLoads +
    row.staticOperations.currentBoundaryExitStores, 0),
  totalStaticResidentBoundaryOperations: regions.reduce((sum, row) => sum +
    row.staticOperations.residentBoundaryEntryLoads +
    row.staticOperations.residentBoundaryExitStores, 0),
  totalStaticMaterializedMemberOperations: regions.reduce((sum, row) => sum +
    row.staticOperations.materializedMemberReads +
    row.staticOperations.materializedMemberOutputs, 0),
};
const gates = {
  exactCorpusPopulation: metrics.regions === 56 && metrics.members === 6258,
  allModulesValidateAndRepeat: validatedModules === 56 && repeatedModules === 56,
  localFootprintReduction: localReduction >= 0.10,
};
const problems = Object.entries(gates).filter(([, pass]) => !pass).map(([name]) => name);
const report = {
  schema: 1,
  experiment: "R124 frozen RV64C-bank hybrid-state static census",
  performanceEvidence: false,
  productModified: false,
  rule: {
    residentXMask: hex(RESIDENT_X & ARCH_X),
    residentX: [1, 2, 8, 9, 10, 11, 12, 13, 14, 15],
    materializedXMask: hex(MATERIALIZED_X),
    fpState: "unchanged-eager",
    selectors: [],
  },
  inputs: {
    manifestSha256: sha256(primaryManifestBytes),
    memberShapesSha256: sha256(primaryShapesBytes),
    repeatManifestSha256: sha256(repeatManifestBytes),
    repeatMemberShapesSha256: sha256(repeatShapesBytes),
  },
  metrics,
  gates,
  problems,
  pass: problems.length === 0,
  decision: problems.length === 0
    ? "admit-measurement-ineligible-dynamic-operation-census"
    : "close-rvc-bank-hybrid-before-hot-instrumentation",
  regions,
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`R124_STATIC ${JSON.stringify({ output, metrics, gates, problems, pass: report.pass })}\n`);
