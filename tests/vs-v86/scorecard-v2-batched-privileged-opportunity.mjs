#!/usr/bin/env node

// Proof-only R068 opportunity probe. Trace the exact modern Boot with the JIT
// fully bypassed, package architecture-selected privileged pages into several
// multi-entry Wasm geometries, and compile each unique module in a fresh V8
// process after guest execution has stopped. Nothing is published or run.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const artifacts = process.env.ARTIFACTS ? resolve(process.env.ARTIFACTS) : null;
const out = process.env.OUT ? resolve(process.env.OUT) : null;
if (!artifacts || !out) throw new Error("set ARTIFACTS and OUT");

const timeoutMs = Number(process.env.TIMEOUT_MS || 900_000);
const traceSlice = 16_384;
const threshold = 200_000n;
const productionPrivilegedThreshold = 4_194_304n;
const leadersPerPage = 64;
const productionInterpreted = 111_363_000;
const interpreterMips = 62;
const generatedMips = 661;
const acceptedBootMs = 2260.5;
const requiredCoveredInstructions = 15_500_000;
const r046OnePageBytes = 5_167_859;
const geometries = [
  { id: "memory-4", pages: 4, stateMode: 0, role: "sensitivity" },
  { id: "memory-8", pages: 8, stateMode: 0, role: "primary" },
  { id: "memory-16", pages: 16, stateMode: 0, role: "sensitivity" },
  { id: "register-structured-8", pages: 8, stateMode: 1, role: "shape-control" },
];
if (!Number.isFinite(timeoutMs) || timeoutMs < 60_000) {
  throw new Error("TIMEOUT_MS must be at least 60000");
}

const paths = {
  loader: join(root, "web/rv64.js"),
  wasm: join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  kernel: join(root, "web/images/alpine/Image"),
  initramfs: join(artifacts, "scorecard-v2-modern-riscv64.cpio"),
  freshCompile: join(root, "tests/vs-v86/fresh-wasm-compile.mjs"),
};
const [loaderBytes, wasm, kernel, initramfs, freshCompileBytes] = await Promise.all([
  readFile(paths.loader),
  readFile(paths.wasm),
  readFile(paths.kernel),
  readFile(paths.initramfs),
  readFile(paths.freshCompile),
]);
const moduleDir = `${out}.modules`;
await mkdir(dirname(out), { recursive: true });
await mkdir(moduleDir, { recursive: true });

const { RV64Debug } = await import(pathToFileURL(paths.loader).href);
const vm = await RV64Debug.create(wasm);
for (const name of [
  "policy_trace_emit_batch",
  "policy_trace_emit_batch_stat",
  "policy_trace_emit_batch_page_stat",
]) {
  if (typeof vm.ex[name] !== "function") {
    throw new Error(`diagnostic runtime lacks ${name}`);
  }
}

let output = "";
const decoder = new TextDecoder();
vm.onWrite = (_fd, bytes) => { output += decoder.decode(bytes, { stream: true }); };
vm.ex.jit_set_enabled(0);
vm.ex.policy_trace_set_enabled(1);
vm.bootVirtLinuxDirect({
  kernel: new Uint8Array(kernel),
  initrd: new Uint8Array(initramfs),
  ramMB: 512,
  cmdline: "console=ttyS0 rdinit=/init",
});

const tick = () => new Promise((done) => setImmediate(done));
const traceStarted = performance.now();
for (let iteration = 0; !output.includes("SCORECARD_V2_READY"); iteration++) {
  if (vm.runVirtSystem(BigInt(traceSlice))) {
    throw new Error("guest powered off before SCORECARD_V2_READY");
  }
  if ((iteration & 15) === 0) await tick();
  if (performance.now() - traceStarted > timeoutMs) {
    throw new Error(`boot trace timed out: ${output.slice(-3000)}`);
  }
}
vm.ex.policy_trace_set_enabled(0);
const traceElapsedMs = performance.now() - traceStarted;

const metaField = (field) => vm.ex.policy_trace_meta(field);
const meta = {
  schema: metaField(0),
  observedInstructions: metaField(4),
  touchedPages: metaField(5),
  events: metaField(6),
  droppedEvents: metaField(7),
  outsideRamInstructions: metaField(8),
  quantum: metaField(9),
  contexts: metaField(13),
};
if (meta.schema !== 3n || meta.droppedEvents !== 0n || meta.outsideRamInstructions !== 0n) {
  throw new Error(`invalid trace metadata ${JSON.stringify(meta, (_key, value) =>
    typeof value === "bigint" ? value.toString() : value)}`);
}

const pages = Array.from({ length: Number(meta.touchedPages) }, (_, index) => ({
  touchedIndex: index,
  pa: vm.ex.policy_trace_page_stat(index, 0),
  page: vm.ex.policy_trace_page_stat(index, 1),
  total: vm.ex.policy_trace_page_stat(index, 2),
  first: vm.ex.policy_trace_page_stat(index, 3),
  last: vm.ex.policy_trace_page_stat(index, 4),
  uniquePcs: vm.ex.policy_trace_page_stat(index, 5),
  uniqueEntries: vm.ex.policy_trace_page_stat(index, 6),
  codeConflicts: vm.ex.policy_trace_page_stat(index, 11),
}));
const contexts = Array.from({ length: Number(meta.contexts) }, (_, index) => ({
  index,
  pa: vm.ex.policy_trace_context_stat(index, 0),
  page: vm.ex.policy_trace_context_stat(index, 1),
  vpage: vm.ex.policy_trace_context_stat(index, 2),
  satp: vm.ex.policy_trace_context_stat(index, 3),
  mode: Number(vm.ex.policy_trace_context_stat(index, 4)),
  total: vm.ex.policy_trace_context_stat(index, 5),
  first: vm.ex.policy_trace_context_stat(index, 6),
  last: vm.ex.policy_trace_context_stat(index, 7),
  uniquePcs: vm.ex.policy_trace_context_stat(index, 8),
  uniqueEntries: vm.ex.policy_trace_context_stat(index, 9),
}));

const mappingGroups = new Map();
for (const context of contexts) {
  const key = `${context.page}:${context.vpage}`;
  const group = mappingGroups.get(key) ?? {
    page: context.page,
    pa: context.pa,
    vpage: context.vpage,
    total: 0n,
    modeTotals: new Map(),
    representative: context.index,
  };
  group.total += context.total;
  group.modeTotals.set(context.mode, (group.modeTotals.get(context.mode) ?? 0n) + context.total);
  group.representative = Math.min(group.representative, context.index);
  mappingGroups.set(key, group);
}
const hottestMapping = new Map();
for (const group of mappingGroups.values()) {
  const key = group.page.toString();
  const previous = hottestMapping.get(key);
  if (!previous || group.total > previous.total ||
      (group.total === previous.total && group.vpage < previous.vpage)) {
    hottestMapping.set(key, group);
  }
}
for (const group of hottestMapping.values()) {
  group.mode = [...group.modeTotals]
    .sort((left, right) => left[1] === right[1]
      ? left[0] - right[0]
      : left[1] > right[1] ? -1 : 1)[0][0];
  delete group.modeTotals;
}

const heatEvents = new Map();
const crossings = new Map();
for (let index = 0; index < Number(meta.events); index++) {
  const kind = Number(vm.ex.policy_trace_event_stat(index, 9));
  if ((kind & 1) === 0) continue;
  const at = vm.ex.policy_trace_event_stat(index, 0);
  const page = vm.ex.policy_trace_event_stat(index, 2).toString();
  const heat = vm.ex.policy_trace_event_stat(index, 3);
  const history = heatEvents.get(page) ?? [];
  history.push({ at, heat });
  heatEvents.set(page, history);
  if (heat >= threshold && !crossings.has(page)) crossings.set(page, at);
}

const conflictedEligiblePages = pages
  .filter((page) => page.total >= threshold && page.codeConflicts !== 0n)
  .map((page) => ({
    page: page.page,
    pa: page.pa,
    total: page.total,
    codeConflicts: page.codeConflicts,
  }));
const selectedPages = pages
  .filter((page) => page.total >= threshold)
  .map((page) => ({
    ...page,
    mapping: hottestMapping.get(page.page.toString()),
    crossingAt: crossings.get(page.page.toString()),
  }))
  .filter((page) => page.mapping && page.mapping.mode !== 0 &&
    page.crossingAt !== undefined && page.codeConflicts === 0n)
  .sort((left, right) => left.crossingAt === right.crossingAt
    ? left.page < right.page ? -1 : left.page > right.page ? 1 : 0
    : left.crossingAt < right.crossingAt ? -1 : 1);

const pack = (pageCount) => {
  const batches = [];
  let current = [];
  for (const page of selectedPages) {
    const duplicate = current.some((member) => member.mapping.vpage === page.mapping.vpage);
    if (current.length >= pageCount || duplicate) {
      batches.push(current);
      current = [];
    }
    current.push(page);
  }
  if (current.length) batches.push(current);
  if (batches.length > 1 && batches.at(-1).length === 1) {
    const tail = batches.pop()[0];
    const previous = batches.at(-1);
    if (previous.some((member) => member.mapping.vpage === tail.mapping.vpage)) {
      throw new Error("deterministic singleton tail has a duplicate virtual page");
    }
    previous.push(tail);
  }
  return batches;
};

const stageContextIndices = (indices) => {
  const bytes = new Uint8Array(indices.length * 4);
  const view = new DataView(bytes.buffer);
  indices.forEach((index, offset) => view.setUint32(offset * 4, index, true));
  const ptr = vm.ex.staging_alloc(bytes.length);
  new Uint8Array(vm.ex.memory.buffer, ptr, bytes.length).set(bytes);
};

const readLebU32 = (bytes, cursor) => {
  let result = 0;
  let shift = 0;
  for (;;) {
    if (cursor.offset >= bytes.length || shift >= 35) throw new Error("invalid Wasm LEB");
    const byte = bytes[cursor.offset++];
    result |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) return result >>> 0;
    shift += 7;
  }
};

const codeGeometry = (bytes) => {
  if (bytes.length < 8 || bytes[0] !== 0 || bytes[1] !== 0x61 ||
      bytes[2] !== 0x73 || bytes[3] !== 0x6d) {
    throw new Error("invalid Wasm header");
  }
  const cursor = { offset: 8 };
  while (cursor.offset < bytes.length) {
    const id = bytes[cursor.offset++];
    const size = readLebU32(bytes, cursor);
    const end = cursor.offset + size;
    if (end > bytes.length) throw new Error("truncated Wasm section");
    if (id !== 10) {
      cursor.offset = end;
      continue;
    }
    const count = readLebU32(bytes, cursor);
    const bodies = [];
    for (let index = 0; index < count; index++) {
      const bodyBytes = readLebU32(bytes, cursor);
      bodies.push(bodyBytes);
      cursor.offset += bodyBytes;
      if (cursor.offset > end) throw new Error("truncated Wasm function body");
    }
    if (cursor.offset !== end) throw new Error("unexpected trailing Wasm code bytes");
    return {
      functions: count,
      maxFunctionBytes: bodies.length ? Math.max(...bodies) : 0,
      medianFunctionBytes: bodies.length
        ? [...bodies].sort((a, b) => a - b)[Math.floor(bodies.length / 2)]
        : 0,
    };
  }
  throw new Error("Wasm module has no code section");
};

const freshCompile = (modulePath) => {
  const stdout = execFileSync(process.execPath, [paths.freshCompile, modulePath], {
    encoding: "utf8",
    timeout: 180_000,
    maxBuffer: 1024 * 1024,
  });
  return JSON.parse(stdout);
};

const heatAt = (page, at) => {
  const history = heatEvents.get(page.toString()) ?? [];
  let low = 0;
  let high = history.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (history[middle].at <= at) low = middle + 1;
    else high = middle;
  }
  return low === 0 ? 0n : history[low - 1].heat;
};

const traceInstructionsPerMs = Number(meta.observedInstructions) / traceElapsedMs;
const geometryResults = [];
for (const geometry of geometries) {
  const batches = pack(geometry.pages);
  const emitted = [];
  for (let batchIndex = 0; batchIndex < batches.length; batchIndex++) {
    const members = batches[batchIndex];
    stageContextIndices(members.map((page) => page.mapping.representative));
    const translateStarted = performance.now();
    const ok = vm.ex.policy_trace_emit_batch(
      members.length,
      leadersPerPage,
      geometry.stateMode,
    ) !== 0;
    const translationMs = performance.now() - translateStarted;
    const stat = (field) => vm.ex.policy_trace_emit_batch_stat(field);
    if (!ok) {
      throw new Error(`${geometry.id} batch ${batchIndex} emission failed status=${stat(0)}`);
    }
    const bytesLength = Number(stat(6));
    const bytes = new Uint8Array(
      vm.ex.memory.buffer,
      vm.ex.jit_out_ptr(),
      bytesLength,
    ).slice();
    if (!WebAssembly.validate(bytes)) {
      throw new Error(`${geometry.id} batch ${batchIndex} emitted invalid Wasm`);
    }
    const modulePath = join(moduleDir, `${geometry.id}-${String(batchIndex).padStart(2, "0")}.wasm`);
    await writeFile(modulePath, bytes);
    const compile = freshCompile(modulePath);
    const pageDiagnostics = Array.from({ length: Number(stat(2)) }, (_, index) => ({
      contextIndex: Number(vm.ex.policy_trace_emit_batch_page_stat(index, 0)),
      pa: vm.ex.policy_trace_emit_batch_page_stat(index, 1),
      vpage: vm.ex.policy_trace_emit_batch_page_stat(index, 2),
      entryEvents: vm.ex.policy_trace_emit_batch_page_stat(index, 3),
      coveredEntryEvents: vm.ex.policy_trace_emit_batch_page_stat(index, 4),
      seedEntries: vm.ex.policy_trace_emit_batch_page_stat(index, 5),
      discoveredLeaders: vm.ex.policy_trace_emit_batch_page_stat(index, 6),
      pageInstructions: vm.ex.policy_trace_emit_batch_page_stat(index, 7),
    }));
    const issueAt = members.reduce((latest, page) =>
      page.crossingAt > latest ? page.crossingAt : latest, 0n);
    const compileDelayInstructions = BigInt(Math.ceil(compile.compileMs * traceInstructionsPerMs));
    const readyAt = issueAt + compileDelayInstructions;
    let readyCoveredInstructions = 0n;
    for (const pageDiagnostic of pageDiagnostics) {
      const member = members.find((page) =>
        page.mapping.representative === pageDiagnostic.contextIndex);
      if (!member) throw new Error("emitter returned an unknown page diagnostic");
      const observedAtReady = heatAt(member.page, readyAt) > threshold
        ? heatAt(member.page, readyAt)
        : threshold;
      const earlyWindowEnd = member.total < productionPrivilegedThreshold
        ? member.total
        : productionPrivilegedThreshold;
      const remaining = earlyWindowEnd > observedAtReady
        ? earlyWindowEnd - observedAtReady
        : 0n;
      const covered = pageDiagnostic.entryEvents === 0n
        ? 0n
        : remaining * pageDiagnostic.coveredEntryEvents / pageDiagnostic.entryEvents;
      pageDiagnostic.remainingAtReady = remaining;
      pageDiagnostic.readyCoveredInstructions = covered;
      readyCoveredInstructions += covered;
    }
    emitted.push({
      batchIndex,
      memberPages: members.map((page) => page.page),
      memberContextIndices: members.map((page) => page.mapping.representative),
      status: stat(0),
      requestedPages: stat(1),
      includedPages: stat(2),
      seedEntries: stat(3),
      discoveredLeaders: stat(4),
      emittedEntries: stat(5),
      bytes: stat(6),
      stateMode: stat(7),
      leadersPerPage: stat(8),
      codeConflicts: stat(9),
      sha256: createHash("sha256").update(bytes).digest("hex"),
      modulePath,
      translationMs,
      compile,
      codeGeometry: codeGeometry(bytes),
      issueAt,
      compileDelayInstructions,
      readyAt,
      readyCoveredInstructions,
      pageDiagnostics,
    });
  }

  const sumNumber = (field) => emitted.reduce((sum, row) => sum + Number(row[field]), 0);
  const sumBigInt = (field) => emitted.reduce((sum, row) => sum + BigInt(row[field]), 0n);
  const entryEvents = emitted.reduce((sum, row) =>
    sum + row.pageDiagnostics.reduce((inner, page) => inner + page.entryEvents, 0n), 0n);
  const coveredEntryEvents = emitted.reduce((sum, row) =>
    sum + row.pageDiagnostics.reduce((inner, page) => inner + page.coveredEntryEvents, 0n), 0n);
  const readyCoveredInstructions = sumBigInt("readyCoveredInstructions");
  const projectedSavedMs = Number(readyCoveredInstructions) *
    (1 / (interpreterMips * 1e6) - 1 / (generatedMips * 1e6)) * 1000;
  const aggregate = {
    id: geometry.id,
    role: geometry.role,
    configuredPagesPerModule: geometry.pages,
    stateMode: geometry.stateMode,
    modules: emitted.length,
    pages: emitted.reduce((sum, row) => sum + Number(row.includedPages), 0),
    entryEvents,
    coveredEntryEvents,
    entryCoverage: entryEvents === 0n ? 0 : Number(coveredEntryEvents) / Number(entryEvents),
    readyCoveredInstructions,
    readyCoveredFractionOfProductionInterpreted:
      Number(readyCoveredInstructions) / productionInterpreted,
    projectedSavedMs,
    projectedBootSpeedup: acceptedBootMs / (acceptedBootMs - projectedSavedMs),
    totalBytes: sumNumber("bytes"),
    maxModuleBytes: Math.max(...emitted.map((row) => Number(row.bytes))),
    totalFunctions: emitted.reduce((sum, row) => sum + row.codeGeometry.functions, 0),
    maxFunctionBytes: Math.max(...emitted.map((row) => row.codeGeometry.maxFunctionBytes)),
    totalTranslationMs: emitted.reduce((sum, row) => sum + row.translationMs, 0),
    totalFreshCompileMs: emitted.reduce((sum, row) => sum + row.compile.compileMs, 0),
    maxFreshCompileMs: Math.max(...emitted.map((row) => row.compile.compileMs)),
    allValid: emitted.every((row) => row.compile.valid),
    codeConflicts: sumBigInt("codeConflicts"),
  };
  aggregate.gates = geometry.role === "primary" ? {
    allModulesValidateAndCompile: aggregate.allValid,
    entryCoverageAtLeast95Percent: aggregate.entryCoverage >= 0.95,
    readyCoveredAtLeast15_5M:
      Number(aggregate.readyCoveredInstructions) >= requiredCoveredInstructions,
    noMoreThan13CompileJobs: aggregate.modules <= 13,
    moduleAtMost1MiB: aggregate.maxModuleBytes <= 1024 * 1024,
    functionAtMost256KiB: aggregate.maxFunctionBytes <= 256 * 1024,
    bytesAtMost110PercentOfR046:
      aggregate.totalBytes <= Math.floor(r046OnePageBytes * 1.10),
    traceAndCodeClean:
      meta.droppedEvents === 0n && meta.outsideRamInstructions === 0n &&
      selectedPages.every((page) => page.codeConflicts === 0n) && aggregate.codeConflicts === 0n,
  } : null;
  aggregate.admitted = aggregate.gates
    ? Object.values(aggregate.gates).every(Boolean)
    : false;
  geometryResults.push({ configuration: geometry, aggregate, emitted });
}

const primary = geometryResults.find((geometry) => geometry.configuration.role === "primary");
const result = {
  format: "rv64-scorecard-v2-batched-privileged-opportunity-v1",
  capturedAt: new Date().toISOString(),
  proofOnly: true,
  measurementEligible: false,
  protocol: "docs/jit-rewrite/R068_BATCHED_PRIVILEGED_T0_PROTOCOL.md",
  inputs: Object.fromEntries(Object.entries({
    loader: loaderBytes,
    wasm,
    kernel,
    initramfs,
    freshCompile: freshCompileBytes,
  }).map(([name, bytes]) => [name, createHash("sha256").update(bytes).digest("hex")])),
  engine: { node: process.version, v8: process.versions.v8 },
  configuration: {
    traceSlice,
    threshold,
    productionPrivilegedThreshold,
    leadersPerPage,
    geometries,
    productionInterpreted,
    interpreterMips,
    generatedMips,
    acceptedBootMs,
    requiredCoveredInstructions,
    r046OnePageBytes,
  },
  traceElapsedMs,
  traceInstructionsPerMs,
  meta,
  conflictedEligiblePages,
  selectedPages: selectedPages.map((page) => ({
    page: page.page,
    pa: page.pa,
    total: page.total,
    crossingAt: page.crossingAt,
    uniquePcs: page.uniquePcs,
    uniqueEntries: page.uniqueEntries,
    codeConflicts: page.codeConflicts,
    mapping: page.mapping,
  })),
  geometries: geometryResults,
  admission: {
    primary: primary.configuration.id,
    gates: primary.aggregate.gates,
    admitted: primary.aggregate.admitted,
  },
  consoleTail: output.slice(-1000),
};

const json = JSON.stringify(result, (_key, value) =>
  typeof value === "bigint" ? value.toString() : value, 2) + "\n";
await writeFile(out, json);
process.stdout.write(JSON.stringify({
  out,
  selectedPages: selectedPages.length,
  traceElapsedMs,
  primary: primary.aggregate,
  admission: result.admission,
}, (_key, value) => typeof value === "bigint" ? value.toString() : value, 2) + "\n");
