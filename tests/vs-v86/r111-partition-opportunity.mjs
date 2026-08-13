#!/usr/bin/env node

// R111 Gate A: apply the prospectively frozen SCC/32-member/24-state rule to
// the preserved production CFGs and deterministic compiler-generated regions.
// This is static opportunity evidence only; it never measures elapsed time.

import { createHash } from "node:crypto";
import assert from "node:assert/strict";
import {
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, join, resolve } from "node:path";

const root = resolve(new URL("../..", import.meta.url).pathname);
function argument(name, fallback) {
  const prefix = `--${name}=`;
  return process.argv.find((value) => value.startsWith(prefix))?.slice(prefix.length) ?? fallback;
}

const realRoot = resolve(argument(
  "real-root",
  join(root, "target/bench/r111-partition-model/real-region-c"),
));
const repeatRoot = resolve(argument(
  "repeat-root",
  join(root, "target/bench/r111-partition-model/real-region-d"),
));
const referenceRoot = join(root, "target/bench/r109-dense-cfg/real-region-control");
const productionRoot = join(root, "target/bench/r109-dense-cfg/corpus");
const output = resolve(argument(
  "output",
  join(root, "target/bench/r111-partition-model/opportunity-a.json"),
));

const MEMBER_LIMIT = 32;
const STATE_LIMIT = 24;
const CFG_MARKER = 0x31474643;

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const popcount = (input) => {
  let value = input >>> 0;
  let count = 0;
  while (value !== 0) {
    value &= value - 1;
    count++;
  }
  return count;
};

function directoryManifest(directory) {
  return readdirSync(directory)
    .filter((name) => statSync(join(directory, name)).isFile())
    .sort()
    .map((name) => {
      const bytes = readFileSync(join(directory, name));
      return { name, bytes: bytes.length, sha256: sha256(bytes) };
    });
}

function parseTsv(path) {
  const lines = readFileSync(path, "utf8").trimEnd().split("\n");
  const columns = lines[0].split("\t");
  return lines.slice(1).map((line) => Object.fromEntries(
    line.split("\t").map((value, index) => [columns[index], value]),
  ));
}

function readUleb(bytes, cursor) {
  let value = 0;
  let shift = 0;
  while (true) {
    if (cursor.offset >= bytes.length || shift > 35) throw new Error("invalid uleb");
    const byte = bytes[cursor.offset++];
    value += (byte & 0x7f) * 2 ** shift;
    if ((byte & 0x80) === 0) return value;
    shift += 7;
  }
}

function codeShape(bytes) {
  if (bytes.length < 8 || bytes.readUInt32LE(0) !== 0x6d736100) {
    throw new Error("invalid Wasm module");
  }
  const cursor = { offset: 8 };
  while (cursor.offset < bytes.length) {
    const section = bytes[cursor.offset++];
    const sectionBytes = readUleb(bytes, cursor);
    const end = cursor.offset + sectionBytes;
    if (end > bytes.length) throw new Error("Wasm section outside module");
    if (section !== 10) {
      cursor.offset = end;
      continue;
    }
    const count = readUleb(bytes, cursor);
    const functions = [];
    for (let index = 0; index < count; index++) {
      const bodyBytes = readUleb(bytes, cursor);
      const bodyStart = cursor.offset;
      const bodyEnd = bodyStart + bodyBytes;
      const groups = readUleb(bytes, cursor);
      let locals = 0;
      const localTypes = {};
      for (let group = 0; group < groups; group++) {
        const amount = readUleb(bytes, cursor);
        const type = bytes[cursor.offset++];
        locals += amount;
        localTypes[`0x${type.toString(16)}`] =
          (localTypes[`0x${type.toString(16)}`] || 0) + amount;
      }
      if (cursor.offset > bodyEnd) throw new Error("locals exceed function body");
      functions.push({ index, bodyBytes, locals, localTypes });
      cursor.offset = bodyEnd;
    }
    if (cursor.offset !== end) throw new Error("code section boundary mismatch");
    return functions;
  }
  throw new Error("module has no code section");
}

function stronglyConnected(successors) {
  const count = successors.length;
  const indices = new Int32Array(count).fill(-1);
  const low = new Int32Array(count);
  const onStack = new Uint8Array(count);
  const stack = [];
  const components = [];
  let nextIndex = 0;
  function visit(node) {
    indices[node] = nextIndex;
    low[node] = nextIndex++;
    stack.push(node);
    onStack[node] = 1;
    for (const target of successors[node]) {
      if (indices[target] < 0) {
        visit(target);
        low[node] = Math.min(low[node], low[target]);
      } else if (onStack[target]) {
        low[node] = Math.min(low[node], indices[target]);
      }
    }
    if (low[node] !== indices[node]) return;
    const component = [];
    while (true) {
      const member = stack.pop();
      onStack[member] = 0;
      component.push(member);
      if (member === node) break;
    }
    component.sort((left, right) => left - right);
    components.push(component);
  }
  for (let node = 0; node < count; node++) {
    if (indices[node] < 0) visit(node);
  }
  return components.sort((left, right) => left[0] - right[0]);
}

function emptyUnion() {
  return { x: 0, f: 0, fcsr: false };
}

function unionWith(left, right) {
  return {
    x: (left.x | right.x) >>> 0,
    f: (left.f | right.f) >>> 0,
    fcsr: left.fcsr || right.fcsr,
  };
}

function unionCount(union) {
  return popcount(union.x & 0xffff_fffe) + popcount(union.f) + Number(union.fcsr);
}

function shapeUnion(shape) {
  return {
    x: (shape.readX | shape.writeX) >>> 0,
    f: (shape.readF | shape.writeF) >>> 0,
    fcsr: shape.readFcsr || shape.writeFcsr,
  };
}

function partitionGraph(successors, shapes = null) {
  const components = stronglyConnected(successors);
  const componentRows = components.map((members) => {
    const union = shapes
      ? members.reduce((value, member) => unionWith(value, shapeUnion(shapes[member])), emptyUnion())
      : emptyUnion();
    return {
      members,
      union,
      stateValues: shapes ? unionCount(union) : 0,
      oversize: members.length > MEMBER_LIMIT || (shapes && unionCount(union) > STATE_LIMIT),
    };
  });
  const clusters = [];
  for (const component of componentRows) {
    const previous = clusters.at(-1);
    const combinedUnion = previous
      ? unionWith(previous.union, component.union)
      : component.union;
    const fits = previous &&
      previous.members.length + component.members.length <= MEMBER_LIMIT &&
      (!shapes || unionCount(combinedUnion) <= STATE_LIMIT);
    if (fits) {
      previous.members.push(...component.members);
      previous.components.push(componentRows.indexOf(component));
      previous.union = combinedUnion;
      previous.stateValues = shapes ? unionCount(combinedUnion) : 0;
      previous.oversize ||= component.oversize;
    } else {
      clusters.push({
        members: [...component.members],
        components: [componentRows.indexOf(component)],
        union: component.union,
        stateValues: component.stateValues,
        oversize: component.oversize,
      });
    }
  }
  const assignment = new Int32Array(successors.length);
  for (let cluster = 0; cluster < clusters.length; cluster++) {
    for (const member of clusters[cluster].members) assignment[member] = cluster;
  }
  let edges = 0;
  let cutEdges = 0;
  for (let source = 0; source < successors.length; source++) {
    for (const target of successors[source]) {
      edges++;
      if (assignment[source] !== assignment[target]) cutEdges++;
    }
  }
  return { components: componentRows, clusters, assignment: [...assignment], edges, cutEdges };
}

function runSelfTests() {
  const chain = Array.from({ length: 64 }, (_, node) => node === 63 ? [] : [node + 1]);
  const chainPartition = partitionGraph(chain);
  assert.equal(chainPartition.components.length, 64);
  assert.deepEqual(chainPartition.clusters.map((cluster) => cluster.members.length), [32, 32]);
  assert.equal(chainPartition.edges, 63);
  assert.equal(chainPartition.cutEdges, 1);

  const cycle = Array.from({ length: 64 }, (_, node) => [(node + 1) % 64]);
  const cyclePartition = partitionGraph(cycle);
  assert.equal(cyclePartition.components.length, 1);
  assert.equal(cyclePartition.clusters.length, 1);
  assert.equal(cyclePartition.clusters[0].oversize, true);
  assert.equal(cyclePartition.cutEdges, 0);

  const shape = (readX = 0, readF = 0) => ({
    readX: readX >>> 0,
    writeX: 0,
    readF: readF >>> 0,
    writeF: 0,
    readFcsr: false,
    writeFcsr: false,
  });
  const statePartition = partitionGraph(
    [[1], [2], []],
    [shape(0x0000_1ffe), shape(0x01ff_e000), shape(0, 1)],
  );
  assert.deepEqual(statePartition.clusters.map((cluster) => cluster.members), [[0, 1], [2]]);
  assert.deepEqual(statePartition.clusters.map((cluster) => cluster.stateValues), [24, 1]);
  assert.equal(statePartition.cutEdges, 1);

  process.stdout.write("PASS R111 partition opportunity selftest\n");
}

if (process.argv.includes("--selftest")) {
  runSelfTests();
  process.exit(0);
}

function parseCfgCapture(path, row, phase) {
  const bytes = readFileSync(path);
  let offset = 0;
  const graphs = [];
  const u32 = () => {
    if (offset + 4 > bytes.length) throw new Error(`${basename(path)} truncated at ${offset}`);
    const value = bytes.readUInt32LE(offset);
    offset += 4;
    return value;
  };
  while (offset < bytes.length) {
    if (u32() !== CFG_MARKER) throw new Error(`${basename(path)} marker mismatch`);
    const nodes = u32();
    const entryCount = u32();
    const duplicationLimit = u32();
    const entries = Array.from({ length: entryCount }, u32);
    const successors = [];
    for (let source = 0; source < nodes; source++) {
      const degree = u32();
      const targets = Array.from({ length: degree }, u32);
      if (targets.some((target) => target >= nodes)) {
        throw new Error(`${basename(path)} has external target in frozen production graph`);
      }
      successors.push(targets);
    }
    const partition = partitionGraph(successors);
    graphs.push({
      index: graphs.length,
      nodes,
      entries,
      duplicationLimit,
      components: partition.components.map((component) => ({
        members: component.members,
        oversize: component.oversize,
      })),
      clusters: partition.clusters.map((cluster) => ({
        members: cluster.members,
        components: cluster.components,
        oversize: cluster.oversize,
      })),
      assignment: partition.assignment,
      edges: partition.edges,
      cutEdges: partition.cutEdges,
    });
  }
  return {
    filename: basename(path),
    row,
    phase,
    bytes: bytes.length,
    sha256: sha256(bytes),
    graphs,
  };
}

const realManifest = parseTsv(join(realRoot, "manifest.tsv"));
const repeatManifest = parseTsv(join(repeatRoot, "manifest.tsv"));
const shapeRows = parseTsv(join(realRoot, "member-shapes.tsv"));
const referenceManifest = parseTsv(join(referenceRoot, "manifest.tsv"));
const deterministicManifest = directoryManifest(realRoot);
const repeatedManifest = directoryManifest(repeatRoot);
const corpusDeterministic = JSON.stringify(deterministicManifest) === JSON.stringify(repeatedManifest);

const modes = new Set(realManifest.map((row) => row.mode));
if (realManifest.length !== 336 || modes.size !== 6 || repeatManifest.length !== 336) {
  throw new Error("real-region corpus does not contain 56 regions x 6 frozen modes");
}
const currentLegacyRows = realManifest.filter((row) => row.mode !== "structured")
  .map((row) => Object.values(row).join("\t"));
const referenceRows = referenceManifest.map((row) => Object.values(row).join("\t"));
const existingManifestIdentical = JSON.stringify(currentLegacyRows) === JSON.stringify(referenceRows);
let existingModuleMismatches = 0;
for (const row of referenceManifest) {
  const current = readFileSync(join(realRoot, row.wasm));
  const reference = readFileSync(join(referenceRoot, row.wasm));
  if (!current.equals(reference)) existingModuleMismatches++;
}

const shapesByRegion = new Map();
for (const row of shapeRows) {
  const shape = {
    member: Number(row.member),
    pc: BigInt(`0x${row.pc}`),
    readX: Number.parseInt(row.read_x, 16) >>> 0,
    writeX: Number.parseInt(row.write_x, 16) >>> 0,
    readF: Number.parseInt(row.read_f, 16) >>> 0,
    writeF: Number.parseInt(row.write_f, 16) >>> 0,
    readFcsr: row.read_fcsr === "1",
    writeFcsr: row.write_fcsr === "1",
    i32Values: Number(row.i32_values),
    i64Values: Number(row.i64_values),
    retired: Number(row.retired),
    successorPcs: row.successors
      ? row.successors.split(",").map((pc) => BigInt(`0x${pc}`))
      : [],
  };
  const shapes = shapesByRegion.get(row.id) || [];
  shapes.push(shape);
  shapesByRegion.set(row.id, shapes);
}

const eagerByRegion = new Map(realManifest
  .filter((row) => row.mode === "eager")
  .map((row) => [row.id, row]));
const structuredByRegion = new Map(realManifest
  .filter((row) => row.mode === "structured")
  .map((row) => [row.id, row]));

const realRegions = [];
for (const [id, eager] of eagerByRegion) {
  const structured = structuredByRegion.get(id);
  const shapes = (shapesByRegion.get(id) || []).sort((left, right) => left.member - right.member);
  if (!structured || shapes.length !== Number(eager.entries)) {
    throw new Error(`${id}: missing structured module or member shapes`);
  }
  const memberForPc = new Map(shapes.map((shape) => [shape.pc.toString(), shape.member]));
  const successors = shapes.map((shape) => shape.successorPcs
    .map((pc) => memberForPc.get(pc.toString()))
    .filter((member) => member !== undefined));
  const partition = partitionGraph(successors, shapes);
  const wholeUnion = shapes.reduce(
    (value, shape) => unionWith(value, shapeUnion(shape)),
    emptyUnion(),
  );
  const wholeStateValues = unionCount(wholeUnion);
  const maxI32 = Math.max(...shapes.map((shape) => shape.i32Values));
  const maxI64 = Math.max(...shapes.map((shape) => shape.i64Values));
  const moduleBytes = readFileSync(join(realRoot, structured.wasm));
  const functions = codeShape(moduleBytes);
  if (functions.length !== 1) throw new Error(`${id}: structured control must have one function`);
  const controlLocals = functions[0].locals;
  const fixedExtras = controlLocals - wholeStateValues - maxI32 - maxI64;
  if (fixedExtras < 0) throw new Error(`${id}: negative fixed local count`);
  const clusters = partition.clusters.map((cluster, index) => {
    const clusterShapes = cluster.members.map((member) => shapes[member]);
    const clusterMaxI32 = Math.max(...clusterShapes.map((shape) => shape.i32Values));
    const clusterMaxI64 = Math.max(...clusterShapes.map((shape) => shape.i64Values));
    return {
      index,
      members: cluster.members,
      components: cluster.components,
      stateValues: cluster.stateValues,
      maxI32: clusterMaxI32,
      maxI64: clusterMaxI64,
      estimatedLocals: fixedExtras + cluster.stateValues + clusterMaxI32 + clusterMaxI64,
      oversize: cluster.oversize,
    };
  });
  const candidateMaxLocals = Math.max(...clusters.map((cluster) => cluster.estimatedLocals));
  const meanClusterState = clusters.reduce((sum, cluster) => sum + cluster.stateValues, 0) /
    clusters.length;
  realRegions.push({
    id,
    workload: eager.workload,
    function: eager.function,
    pages: Number(eager.pages),
    leaderCap: Number(eager.leader_cap),
    members: shapes.length,
    eagerBytes: Number(eager.bytes),
    structuredBytes: moduleBytes.length,
    structuredSha256: sha256(moduleBytes),
    wholeStateValues,
    maxI32,
    maxI64,
    controlLocals,
    fixedExtras,
    components: partition.components.map((component) => ({
      members: component.members,
      stateValues: component.stateValues,
      oversize: component.oversize,
    })),
    clusters,
    assignment: partition.assignment,
    edges: partition.edges,
    cutEdges: partition.cutEdges,
    meanClusterState,
    candidateMaxLocals,
    stateRatio: wholeStateValues === 0 ? 1 : meanClusterState / wholeStateValues,
    footprintReduction: 1 - candidateMaxLocals / controlLocals,
    split: clusters.length >= 2,
    hasOversize: clusters.some((cluster) => cluster.oversize),
  });
}

realRegions.sort((left, right) => left.id.localeCompare(right.id));
const totalEagerBytes = realRegions.reduce((sum, region) => sum + region.eagerBytes, 0);
const eligibleEagerBytes = realRegions
  .filter((region) => region.split)
  .reduce((sum, region) => sum + region.eagerBytes, 0);
const oversizeEligibleBytes = realRegions
  .filter((region) => region.split && region.hasOversize)
  .reduce((sum, region) => sum + region.eagerBytes, 0);
const weightedStateRatio = realRegions.reduce(
  (sum, region) => sum + region.eagerBytes * region.stateRatio,
  0,
) / totalEagerBytes;
const weightedFootprintReduction = realRegions.reduce(
  (sum, region) => sum + region.eagerBytes * region.footprintReduction,
  0,
) / totalEagerBytes;

const productionFiles = [
  ["rewrite-boot-first.rvcfg", "boot", "first"],
  ["rewrite-compile-first.rvcfg", "compile", "first"],
  ["rewrite-compile-prime.rvcfg", "compile", "prime"],
  ["rewrite-compile-steady.rvcfg", "compile", "steady"],
].map(([filename, row, phase]) =>
  parseCfgCapture(join(productionRoot, filename), row, phase));

function productionSummary(row) {
  const files = productionFiles.filter((file) => file.row === row);
  const graphs = files.flatMap((file) => file.graphs);
  const edges = graphs.reduce((sum, graph) => sum + graph.edges, 0);
  const cutEdges = graphs.reduce((sum, graph) => sum + graph.cutEdges, 0);
  return {
    files: files.length,
    graphs: graphs.length,
    nodes: graphs.reduce((sum, graph) => sum + graph.nodes, 0),
    edges,
    cutEdges,
    cutFraction: edges === 0 ? 0 : cutEdges / edges,
    splitGraphs: graphs.filter((graph) => graph.clusters.length >= 2).length,
    oversizeGraphs: graphs.filter((graph) =>
      graph.clusters.some((cluster) => cluster.oversize)).length,
  };
}

const production = {
  boot: productionSummary("boot"),
  compile: productionSummary("compile"),
  files: productionFiles,
};
const metrics = {
  regions: realRegions.length,
  members: realRegions.reduce((sum, region) => sum + region.members, 0),
  totalEagerBytes,
  splitRegions: realRegions.filter((region) => region.split).length,
  eligibleEagerBytes,
  eligibleByteFraction: eligibleEagerBytes / totalEagerBytes,
  weightedStateRatio,
  weightedFootprintReduction,
  oversizeEligibleBytes,
  oversizeEligibleByteFraction: eligibleEagerBytes === 0
    ? 1
    : oversizeEligibleBytes / eligibleEagerBytes,
};
const gates = {
  corpusDeterministic,
  existingManifestIdentical,
  existingModulesIdentical: existingModuleMismatches === 0,
  exactCorpusShape: realRegions.length === 56 &&
    metrics.members === 6258 && production.boot.graphs === 15 && production.compile.graphs === 118,
  eligibleByteFraction: metrics.eligibleByteFraction >= 0.75,
  weightedStateRatio: metrics.weightedStateRatio <= 0.80,
  weightedFootprintReduction: metrics.weightedFootprintReduction >= 0.15,
  bootCutFraction: production.boot.cutFraction <= 0.125,
  compileCutFraction: production.compile.cutFraction <= 0.125,
  oversizeEligibleByteFraction: metrics.oversizeEligibleByteFraction <= 0.20,
};
const problems = Object.entries(gates)
  .filter(([, pass]) => !pass)
  .map(([name]) => name);

const report = {
  schema: 1,
  experiment: "R111 same-module partition static opportunity",
  performanceEvidence: false,
  rule: {
    sccAtomic: true,
    componentOrder: "ascending minimum dense member ID",
    memberLimit: MEMBER_LIMIT,
    stateValueLimit: STATE_LIMIT,
  },
  inputs: {
    realManifest: sha256(readFileSync(join(realRoot, "manifest.tsv"))),
    memberShapes: sha256(readFileSync(join(realRoot, "member-shapes.tsv"))),
    referenceManifest: sha256(readFileSync(join(referenceRoot, "manifest.tsv"))),
    production: Object.fromEntries(productionFiles.map((file) => [file.filename, file.sha256])),
  },
  reproduction: {
    corpusDeterministic,
    files: deterministicManifest.length,
    existingManifestIdentical,
    existingModuleMismatches,
  },
  metrics,
  production: {
    boot: production.boot,
    compile: production.compile,
  },
  gates,
  problems,
  pass: problems.length === 0,
  realRegions,
  productionFiles,
};

writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`R111_PARTITION_OPPORTUNITY ${JSON.stringify({
  output,
  metrics,
  production: { boot: production.boot, compile: production.compile },
  gates,
  problems,
  pass: report.pass,
})}\n`);
