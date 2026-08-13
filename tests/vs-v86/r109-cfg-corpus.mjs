#!/usr/bin/env node

// Parse and freeze the proof-only R109 stackifier corpus. The raw files are a
// sequence of little-endian CFG1 records emitted immediately before the
// production tree stackifier runs. This script validates every boundary and
// records enough shape information to audit the standalone dense-set model.

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const evidenceRoot = resolve(process.argv[2] || join(root, "target/bench/r109-dense-cfg"));
const corpusRoot = join(evidenceRoot, "corpus");
const output = resolve(process.argv[3] || join(evidenceRoot, "corpus-manifest.json"));
const marker = 0x31474643;

const captures = [
  { filename: "rewrite-boot-first.rvcfg", row: "boot", phase: "first" },
  { filename: "rewrite-compile-first.rvcfg", row: "compile", phase: "first" },
  { filename: "rewrite-compile-prime.rvcfg", row: "compile", phase: "prime" },
  { filename: "rewrite-compile-steady.rvcfg", row: "compile", phase: "steady" },
];

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function increment(histogram, key) {
  histogram[key] = (histogram[key] || 0) + 1;
}

function sortedHistogram(histogram) {
  return Object.fromEntries(
    Object.entries(histogram).sort(([left], [right]) => Number(left) - Number(right)),
  );
}

function parseCapture(bytes, capture) {
  let offset = 0;
  const graphs = [];
  const histogram = { nodes: {}, entries: {}, duplicationLimits: {}, outdegree: {} };
  const totals = {
    graphs: 0,
    nodes: 0,
    rawEntries: 0,
    validEntries: 0,
    duplicateEntries: 0,
    rawEdges: 0,
    internalEdges: 0,
    externalEdges: 0,
    duplicateEdges: 0,
    selfEdges: 0,
  };
  const maxima = { nodes: 0, entries: 0, rawEdges: 0, outdegree: 0 };

  const u32 = () => {
    if (offset + 4 > bytes.length) {
      throw new Error(`${capture.filename}: truncated u32 at byte ${offset}`);
    }
    const value = bytes.readUInt32LE(offset);
    offset += 4;
    return value;
  };

  while (offset < bytes.length) {
    const recordStart = offset;
    const recordMarker = u32();
    if (recordMarker !== marker) {
      throw new Error(
        `${capture.filename}: bad marker 0x${recordMarker.toString(16)} at byte ${recordStart}`,
      );
    }
    const nodeCount = u32();
    const entryCount = u32();
    const duplicationLimit = u32();
    if (nodeCount > 512) {
      throw new Error(`${capture.filename}: graph ${graphs.length} has ${nodeCount} nodes`);
    }

    let validEntries = 0;
    const entrySet = new Set();
    for (let index = 0; index < entryCount; index++) {
      const entry = u32();
      if (entry < nodeCount) validEntries++;
      if (entrySet.has(entry)) totals.duplicateEntries++;
      entrySet.add(entry);
    }

    let rawEdges = 0;
    let internalEdges = 0;
    let externalEdges = 0;
    let duplicateEdges = 0;
    let selfEdges = 0;
    let maxOutdegree = 0;
    for (let node = 0; node < nodeCount; node++) {
      const outdegree = u32();
      rawEdges += outdegree;
      maxOutdegree = Math.max(maxOutdegree, outdegree);
      increment(histogram.outdegree, outdegree);
      const targets = new Set();
      for (let index = 0; index < outdegree; index++) {
        const target = u32();
        if (target < nodeCount) internalEdges++;
        else externalEdges++;
        if (target === node) selfEdges++;
        if (targets.has(target)) duplicateEdges++;
        targets.add(target);
      }
    }

    const recordEnd = offset;
    increment(histogram.nodes, nodeCount);
    increment(histogram.entries, entryCount);
    increment(histogram.duplicationLimits, duplicationLimit);
    totals.graphs++;
    totals.nodes += nodeCount;
    totals.rawEntries += entryCount;
    totals.validEntries += validEntries;
    totals.rawEdges += rawEdges;
    totals.internalEdges += internalEdges;
    totals.externalEdges += externalEdges;
    totals.duplicateEdges += duplicateEdges;
    totals.selfEdges += selfEdges;
    maxima.nodes = Math.max(maxima.nodes, nodeCount);
    maxima.entries = Math.max(maxima.entries, entryCount);
    maxima.rawEdges = Math.max(maxima.rawEdges, rawEdges);
    maxima.outdegree = Math.max(maxima.outdegree, maxOutdegree);
    graphs.push({
      index: graphs.length,
      byteOffset: recordStart,
      bytes: recordEnd - recordStart,
      sha256: sha256(bytes.subarray(recordStart, recordEnd)),
      nodes: nodeCount,
      entries: entryCount,
      validEntries,
      duplicationLimit,
      rawEdges,
      internalEdges,
      externalEdges,
      maxOutdegree,
    });
  }

  if (offset !== bytes.length || totals.graphs === 0) {
    throw new Error(`${capture.filename}: invalid final boundary or empty capture`);
  }
  return {
    ...capture,
    bytes: bytes.length,
    sha256: sha256(bytes),
    totals,
    maxima,
    histograms: Object.fromEntries(
      Object.entries(histogram).map(([name, values]) => [name, sortedHistogram(values)]),
    ),
    graphs,
  };
}

const files = [];
for (const capture of captures) {
  const bytes = await readFile(join(corpusRoot, capture.filename));
  files.push(parseCapture(bytes, capture));
}

const artifactPaths = {
  instrumentedWasm: join(evidenceRoot, "instrumented-rv64_wasm.wasm"),
  loader: join(root, "web/rv64.js"),
  kernel: join(root, "web/images/alpine/Image"),
  initramfs: join(root, "target/bench/scorecard-v2-modern-riscv64.cpio"),
};
const artifacts = {};
for (const [name, path] of Object.entries(artifactPaths)) {
  const bytes = await readFile(path);
  artifacts[name] = { path, bytes: bytes.length, sha256: sha256(bytes) };
}

const manifest = {
  format: "rv64-r109-stackifier-corpus-v1",
  createdAt: new Date().toISOString(),
  proofOnly: true,
  measurementEligible: false,
  captureSemantics: {
    boot: "timer-start through post-ready JIT settle",
    compile: "each FIRST/PRIME/STEADY phase start through its post-phase JIT settle",
    source: "raw stackify(successors, entries, duplication_limit) input",
  },
  runtime: {
    node: process.version,
    guest: { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" },
    scheduler: "public-one-slice-per-turn",
  },
  workloadProof: {
    compileObjectMd5EveryPhase: "24eedf7e06beffd4d3ba1945585588db",
    compileBenchmarkSha256: "27f4ff857b0df5284637cbcdc8d4890bbc975a13a4495d847e2e670ddb9e8d71",
    jitGeneratedCodeExecuted: true,
    productionPolicyProblems: [],
  },
  invariants: {
    maximumProductionMembers: 512,
    syntheticEntryNode: "N (ordered after real nodes)",
    sourceOrdering: "ascending BTreeMap/BTreeSet order",
  },
  artifacts,
  files,
};

await writeFile(output, `${JSON.stringify(manifest, null, 2)}\n`, { flag: "wx" });
console.log(JSON.stringify({ output, sha256: sha256(await readFile(output)), files: files.length }));
