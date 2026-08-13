#!/usr/bin/env node

// R116 Gate A: census the one prospectively frozen selective-residency rule
// over the immutable compiler-produced real-region corpus. No candidate is
// emitted or timed here.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const root = resolve(new URL("../..", import.meta.url).pathname);
const corpus = resolve(process.argv[2] ||
  join(root, "target/bench/r111-partition-model/real-region-c"));
const output = resolve(process.argv[3] ||
  join(root, "target/bench/r116-selective-state/opportunity.json"));
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

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
  assert.equal(bytes.readUInt32LE(0), 0x6d736100, `${label}: invalid Wasm`);
  const cursor = { offset: 8 };
  while (cursor.offset < bytes.length) {
    const id = bytes[cursor.offset++];
    const size = readUleb(bytes, cursor);
    const end = cursor.offset + size;
    if (id !== 10) {
      cursor.offset = end;
      continue;
    }
    const count = readUleb(bytes, cursor);
    assert.equal(count, 1, `${label}: expected one function`);
    const bodySize = readUleb(bytes, cursor);
    const bodyEnd = cursor.offset + bodySize;
    const groups = readUleb(bytes, cursor);
    let locals = 0;
    const byType = {};
    for (let group = 0; group < groups; group++) {
      const amount = readUleb(bytes, cursor);
      const type = `0x${bytes[cursor.offset++].toString(16)}`;
      locals += amount;
      byType[type] = (byType[type] || 0) + amount;
    }
    assert.ok(cursor.offset <= bodyEnd && bodyEnd <= end, `${label}: invalid code body`);
    return { locals, byType };
  }
  throw new Error(`${label}: missing code section`);
}

function stronglyConnected(successors) {
  const indices = new Int32Array(successors.length).fill(-1);
  const low = new Int32Array(successors.length);
  const active = new Uint8Array(successors.length);
  const stack = [];
  const components = [];
  let next = 0;
  function visit(node) {
    indices[node] = low[node] = next++;
    stack.push(node);
    active[node] = 1;
    for (const target of successors[node]) {
      if (indices[target] < 0) {
        visit(target);
        low[node] = Math.min(low[node], low[target]);
      } else if (active[target]) {
        low[node] = Math.min(low[node], indices[target]);
      }
    }
    if (low[node] !== indices[node]) return;
    const members = [];
    for (;;) {
      const member = stack.pop();
      active[member] = 0;
      members.push(member);
      if (member === node) break;
    }
    components.push(members.sort((a, b) => a - b));
  }
  for (let node = 0; node < successors.length; node++) {
    if (indices[node] < 0) visit(node);
  }
  return components;
}

function bits(mask, prefix, skipZero = false) {
  const values = [];
  for (let index = skipZero ? 1 : 0; index < 32; index++) {
    if ((mask >>> index) & 1) values.push(`${prefix}${index}`);
  }
  return values;
}

function percentile(values, fraction) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor((sorted.length - 1) * fraction)];
}

const manifestPath = join(corpus, "manifest.tsv");
const shapesPath = join(corpus, "member-shapes.tsv");
const manifest = parseTsv(manifestPath);
const shapeRows = parseTsv(shapesPath);
const structured = manifest.filter((row) => row.mode === "structured");
const eager = new Map(manifest.filter((row) => row.mode === "eager")
  .map((row) => [row.id, row]));
const byRegion = new Map();
for (const row of shapeRows) {
  const shape = {
    member: Number(row.member),
    pc: row.pc,
    readX: Number.parseInt(row.read_x, 16) >>> 0,
    writeX: Number.parseInt(row.write_x, 16) >>> 0,
    readF: Number.parseInt(row.read_f, 16) >>> 0,
    writeF: Number.parseInt(row.write_f, 16) >>> 0,
    readFcsr: row.read_fcsr === "1",
    writeFcsr: row.write_fcsr === "1",
    successors: row.successors ? row.successors.split(",") : [],
  };
  const rows = byRegion.get(row.id) || [];
  rows.push(shape);
  byRegion.set(row.id, rows);
}

const regions = [];
for (const module of structured) {
  const shapes = (byRegion.get(module.id) || []).sort((a, b) => a.member - b.member);
  const eagerRow = eager.get(module.id);
  assert.ok(eagerRow, `${module.id}: missing eager row`);
  assert.equal(shapes.length, Number(module.entries), `${module.id}: member count`);
  const memberByPc = new Map(shapes.map((shape) => [shape.pc, shape.member]));
  const successors = shapes.map((shape) => shape.successors
    .map((pc) => memberByPc.get(pc))
    .filter((member) => member !== undefined));
  const cyclic = new Set();
  for (const component of stronglyConnected(successors)) {
    if (component.length > 1 || successors[component[0]].includes(component[0])) {
      for (const member of component) cyclic.add(member);
    }
  }

  const references = new Map();
  const reads = new Set();
  const writes = new Set();
  const add = (name, member, read, write) => {
    if (!references.has(name)) references.set(name, new Set());
    references.get(name).add(member);
    if (read) reads.add(name);
    if (write) writes.add(name);
  };
  for (const shape of shapes) {
    const readNames = [
      ...bits(shape.readX, "x", true),
      ...bits(shape.readF, "f"),
      ...(shape.readFcsr ? ["fcsr"] : []),
    ];
    const writeNames = [
      ...bits(shape.writeX, "x", true),
      ...bits(shape.writeF, "f"),
      ...(shape.writeFcsr ? ["fcsr"] : []),
    ];
    for (const name of new Set([...readNames, ...writeNames])) {
      add(name, shape.member, readNames.includes(name), writeNames.includes(name));
    }
  }
  const cold = [...references].filter(([, members]) => {
    if (members.size !== 1) return false;
    return !cyclic.has([...members][0]);
  }).map(([name]) => name).sort();
  const coldSet = new Set(cold);
  const coldReads = cold.filter((name) => reads.has(name)).length;
  const coldWrites = cold.filter((name) => writes.has(name)).length;
  const cyclicCold = cold.filter((name) =>
    [...references.get(name)].some((member) => cyclic.has(member))).length;
  const wholeState = references.size;
  const residentState = wholeState - cold.length;
  const wasm = readFileSync(join(corpus, module.wasm));
  const localShape = functionLocals(wasm, basename(module.wasm));
  assert.ok(localShape.locals >= cold.length, `${module.id}: cold locals exceed total`);
  regions.push({
    id: module.id,
    workload: module.workload,
    function: module.function,
    pages: Number(module.pages),
    leaderCap: Number(module.leader_cap),
    members: shapes.length,
    cyclicMembers: cyclic.size,
    eagerBytes: Number(eagerRow.bytes),
    structuredBytes: wasm.length,
    structuredSha256: sha256(wasm),
    currentLocals: localShape.locals,
    candidateLocals: localShape.locals - cold.length,
    wholeState,
    residentState,
    coldState: cold.length,
    coldReads,
    coldWrites,
    replacedEntryExitOps: coldReads + coldWrites,
    candidateMemberOps: coldReads + coldWrites,
    cyclicCold,
    cold,
  });
}
regions.sort((a, b) => a.id.localeCompare(b.id));

const totalWeight = regions.reduce((sum, region) => sum + region.eagerBytes, 0);
const weighted = (field) => regions.reduce(
  (sum, region) => sum + region.eagerBytes * region[field], 0,
) / totalWeight;
const weightedRatio = (numerator, denominator) => regions.reduce(
  (sum, region) => sum + region.eagerBytes *
    (region[denominator] === 0 ? 1 : region[numerator] / region[denominator]),
  0,
) / totalWeight;
const eligibleWeight = regions.filter((region) => region.coldState > 0)
  .reduce((sum, region) => sum + region.eagerBytes, 0);
const metrics = {
  regions: regions.length,
  members: regions.reduce((sum, region) => sum + region.members, 0),
  eagerBytes: totalWeight,
  eligibleRegions: regions.filter((region) => region.coldState > 0).length,
  eligibleByteFraction: eligibleWeight / totalWeight,
  weightedWholeState: weighted("wholeState"),
  weightedResidentState: weighted("residentState"),
  weightedResidentStateRatio: weightedRatio("residentState", "wholeState"),
  weightedCurrentLocals: weighted("currentLocals"),
  weightedCandidateLocals: weighted("candidateLocals"),
  weightedLocalFootprintReduction: 1 - weightedRatio("candidateLocals", "currentLocals"),
  coldState: {
    min: Math.min(...regions.map((region) => region.coldState)),
    p50: percentile(regions.map((region) => region.coldState), 0.50),
    p95: percentile(regions.map((region) => region.coldState), 0.95),
    max: Math.max(...regions.map((region) => region.coldState)),
  },
  cyclicCold: regions.reduce((sum, region) => sum + region.cyclicCold, 0),
  operationBoundViolations: regions.filter((region) =>
    region.candidateMemberOps > region.replacedEntryExitOps).length,
};
const gates = {
  exactCorpus: metrics.regions === 56 && metrics.members === 6258,
  oneFunctionPerStructuredModule: regions.length === structured.length,
  eligibleByteFraction: metrics.eligibleByteFraction >= 0.75,
  residentStateRatio: metrics.weightedResidentStateRatio <= 0.80,
  localFootprintReduction: metrics.weightedLocalFootprintReduction >= 0.05,
  noCyclicColdState: metrics.cyclicCold === 0,
  boundedMemoryOperations: metrics.operationBoundViolations === 0,
};
const problems = Object.entries(gates).filter(([, pass]) => !pass).map(([name]) => name);
const report = {
  schema: 1,
  experiment: "R116 selective state-residency static opportunity",
  performanceEvidence: false,
  rule: {
    coldReferenceMembers: 1,
    coldMemberMustBeAcyclic: true,
    alwaysResident: ["pc", "retired", "fuel"],
    selectors: [],
  },
  inputs: {
    manifest: sha256(readFileSync(manifestPath)),
    memberShapes: sha256(readFileSync(shapesPath)),
  },
  metrics,
  gates,
  problems,
  pass: problems.length === 0,
  regions,
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
console.log(`R116_SELECTIVE_STATE ${JSON.stringify({ output, metrics, gates, problems, pass: report.pass })}`);
