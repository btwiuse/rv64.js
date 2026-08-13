#!/usr/bin/env node

// Proof-only R046 opportunity probe for a compact asynchronous early-Wasm
// tier. The exact modern scorecard guest runs with generated execution off so
// physical-page heat and entry observations are not perturbed by an existing
// tier. After READY, the diagnostic runtime emits but never publishes candidate
// modules. Host compilation happens only after guest execution has stopped.

import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const artifacts = process.env.ARTIFACTS ? resolve(process.env.ARTIFACTS) : null;
if (!artifacts) throw new Error("set ARTIFACTS");

const timeoutMs = Number(process.env.TIMEOUT_MS || 900_000);
const traceSlice = Number(process.env.TRACE_SLICE || 16_384);
const earlyThreshold = Number(process.env.EARLY_THRESHOLD || 200_000);
const compileTop = Number(process.env.COMPILE_TOP || 12);
const emitLimit = Number(process.env.EMIT_LIMIT || 256);
const leaderCaps = (process.env.LEADER_CAPS || "32,64,128")
  .split(",").map(Number);
const stateModes = (process.env.STATE_MODES || "0,1")
  .split(",").map(Number);
for (const [name, value] of Object.entries({ timeoutMs, traceSlice, earlyThreshold, compileTop, emitLimit })) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`${name} must be non-negative`);
}
if (timeoutMs < 60_000 || traceSlice < 1 || earlyThreshold < 1 || emitLimit < 1 ||
    leaderCaps.some((value) => !Number.isSafeInteger(value) || value < 2 || value > 512) ||
    stateModes.some((value) => ![0, 1].includes(value))) {
  throw new Error("invalid timeout, threshold, leader cap, or state mode");
}

const paths = {
  loader: join(root, "web/rv64.js"),
  wasm: join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  kernel: join(root, "web/images/alpine/Image"),
  initramfs: join(artifacts, "scorecard-v2-modern-riscv64.cpio"),
};
const [loaderBytes, wasm, kernel, initramfs] = await Promise.all([
  readFile(paths.loader), readFile(paths.wasm), readFile(paths.kernel), readFile(paths.initramfs),
]);
const { RV64Debug } = await import(pathToFileURL(paths.loader).href);
const vm = await RV64Debug.create(wasm);
for (const name of ["policy_trace_emit_context", "policy_trace_emit_stat"]) {
  if (typeof vm.ex[name] !== "function") throw new Error(`diagnostic runtime lacks ${name}`);
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
const started = performance.now();
for (let iteration = 0; !output.includes("SCORECARD_V2_READY"); iteration++) {
  if (vm.runVirtSystem(BigInt(traceSlice))) throw new Error("guest powered off before READY");
  if ((iteration & 15) === 0) await tick();
  if (performance.now() - started > timeoutMs) {
    throw new Error(`boot trace timed out: ${output.slice(-3000)}`);
  }
}
vm.ex.policy_trace_set_enabled(0);
const traceElapsedMs = performance.now() - started;

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
if (meta.schema !== 2n || meta.droppedEvents !== 0n || meta.outsideRamInstructions !== 0n) {
  throw new Error(`invalid trace metadata ${JSON.stringify(meta, (_k, v) =>
    typeof v === "bigint" ? v.toString() : v)}`);
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

// Pick the hottest virtual mapping for each physical page, aggregating SATP
// and privilege contexts exactly as the production (VA, PA) page key does.
const mappingGroups = new Map();
for (const context of contexts) {
  const key = `${context.page}:${context.vpage}`;
  const group = mappingGroups.get(key) ?? {
    page: context.page, pa: context.pa, vpage: context.vpage,
    total: 0n, modeTotals: new Map(), representative: context.index,
  };
  group.total += context.total;
  group.modeTotals.set(context.mode, (group.modeTotals.get(context.mode) ?? 0n) + context.total);
  mappingGroups.set(key, group);
}
const hottestMapping = new Map();
for (const group of mappingGroups.values()) {
  const previous = hottestMapping.get(group.page.toString());
  if (!previous || group.total > previous.total) hottestMapping.set(group.page.toString(), group);
}
for (const group of hottestMapping.values()) {
  group.mode = [...group.modeTotals].sort((a, b) => a[1] === b[1] ? 0 : a[1] > b[1] ? -1 : 1)[0][0];
  delete group.modeTotals;
}

const thresholds = [32_768, 65_536, 131_072, 200_000, 262_144, 524_288,
  1_048_576, 4_194_304];
const crossings = new Map(thresholds.map((threshold) => [threshold, new Map()]));
for (let index = 0; index < Number(meta.events); index++) {
  const kind = Number(vm.ex.policy_trace_event_stat(index, 9));
  if ((kind & 1) === 0) continue;
  const at = vm.ex.policy_trace_event_stat(index, 0);
  const page = vm.ex.policy_trace_event_stat(index, 2).toString();
  const heat = vm.ex.policy_trace_event_stat(index, 3);
  for (const threshold of thresholds) {
    const map = crossings.get(threshold);
    if (heat >= BigInt(threshold) && !map.has(page)) map.set(page, at);
  }
}

const sum = (values) => values.reduce((total, value) => total + value, 0n);
const observed = meta.observedInstructions;
const productionInterpreted = 113_433_642n;
const thresholdOpportunity = thresholds.map((threshold) => {
  const selected = pages.filter((page) => page.total >= BigInt(threshold));
  const postThreshold = sum(selected.map((page) => page.total - BigInt(threshold)));
  const weightedCrossNumerator = sum(selected.map((page) =>
    (crossings.get(threshold).get(page.page.toString()) ?? page.last) *
      (page.total - BigInt(threshold))));
  return {
    threshold,
    pages: selected.length,
    totalInstructionsOnSelectedPages: sum(selected.map((page) => page.total)),
    postThresholdInstructions: postThreshold,
    postThresholdShareObserved: Number(postThreshold) / Number(observed),
    multipleOfProductionInterpreted: Number(postThreshold) / Number(productionInterpreted),
    weightedCrossingIcount: postThreshold === 0n ? 0 :
      Number(weightedCrossNumerator / postThreshold),
  };
});

const selectedPages = pages
  .filter((page) => page.total >= BigInt(earlyThreshold))
  .map((page) => ({ ...page, mapping: hottestMapping.get(page.page.toString()) }))
  .filter((page) => page.mapping)
  .sort((a, b) => a.total === b.total ? 0 : a.total > b.total ? -1 : 1)
  .slice(0, emitLimit);

const stateNames = new Map([[0, "memory"], [1, "register-structured"]]);
const emitted = [];
for (const stateMode of stateModes) {
  for (const leaderCap of leaderCaps) {
    for (let rank = 0; rank < selectedPages.length; rank++) {
      const page = selectedPages[rank];
      const t0 = performance.now();
      const ok = vm.ex.policy_trace_emit_context(
        page.mapping.representative, leaderCap, stateMode,
      ) !== 0;
      const translated = performance.now();
      const stat = (field) => vm.ex.policy_trace_emit_stat(field);
      const bytesLength = Number(stat(6));
      let valid = false;
      let compileMs = null;
      if (ok && bytesLength > 0) {
        const bytes = new Uint8Array(vm.ex.memory.buffer, vm.ex.jit_out_ptr(), bytesLength).slice();
        valid = WebAssembly.validate(bytes);
        if (!valid) throw new Error(`invalid candidate module state=${stateMode} cap=${leaderCap}`);
        if (rank < compileTop) {
          const compileStarted = performance.now();
          await WebAssembly.compile(bytes);
          compileMs = performance.now() - compileStarted;
        }
      }
      emitted.push({
        state: stateNames.get(stateMode), stateMode, leaderCap, rank,
        page: page.page, pa: page.pa, vpage: page.mapping.vpage,
        pageInstructions: page.total, mappingInstructions: page.mapping.total,
        mode: page.mapping.mode, ok, valid,
        seedEntries: stat(3), discoveredLeaders: stat(4), emittedEntries: stat(5),
        bytes: stat(6), entryEvents: stat(9), coveredEntryEvents: stat(10),
        translationMs: translated - t0, compileMs,
      });
      if ((rank & 15) === 15) await tick();
    }
  }
}

const median = (values) => {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length & 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};
const percentile = (values, fraction) => {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(fraction * sorted.length))];
};
const emissionAggregates = [];
for (const stateMode of stateModes) {
  for (const leaderCap of leaderCaps) {
    const rows = emitted.filter((row) => row.stateMode === stateMode && row.leaderCap === leaderCap);
    const successful = rows.filter((row) => row.ok);
    const bytes = successful.map((row) => Number(row.bytes));
    const compile = successful.map((row) => row.compileMs).filter((value) => value !== null);
    const entryEvents = sum(successful.map((row) => row.entryEvents));
    const coveredEntryEvents = sum(successful.map((row) => row.coveredEntryEvents));
    const eligiblePageInstructions = sum(successful.map((row) => row.pageInstructions));
    const earlyWindowInstructions = sum(successful.map((row) => {
      // User mappings already qualify for the optimized tier at 131,072 and
      // offer no honest 200k->optimized window. Privileged mappings use the
      // production 32x multiplier and are the Boot target.
      const optimizedThreshold = row.mode === 0 ? 131_072n : 4_194_304n;
      const upper = row.pageInstructions < optimizedThreshold
        ? row.pageInstructions : optimizedThreshold;
      return upper > BigInt(earlyThreshold) ? upper - BigInt(earlyThreshold) : 0n;
    }));
    const entryCoverage = entryEvents === 0n ? 0 : Number(coveredEntryEvents) / Number(entryEvents);
    const optimisticCoveredWindow = Number(earlyWindowInstructions) * entryCoverage;
    emissionAggregates.push({
      state: stateNames.get(stateMode), stateMode, leaderCap,
      attempts: rows.length, successes: successful.length,
      eligiblePageInstructions,
      earlyWindowInstructions,
      entryEvents, coveredEntryEvents, entryCoverage,
      optimisticCoveredWindowInstructions: optimisticCoveredWindow,
      fractionOfProductionInterpreted: optimisticCoveredWindow / Number(productionInterpreted),
      bytes: {
        total: bytes.reduce((total, value) => total + value, 0),
        mean: bytes.length ? bytes.reduce((total, value) => total + value, 0) / bytes.length : null,
        median: median(bytes), p90: percentile(bytes, 0.9), max: bytes.length ? Math.max(...bytes) : null,
      },
      translationMs: {
        median: median(successful.map((row) => row.translationMs)),
        p90: percentile(successful.map((row) => row.translationMs), 0.9),
      },
      compileMs: {
        samples: compile.length, median: median(compile), p90: percentile(compile, 0.9),
        total: compile.reduce((total, value) => total + value, 0),
      },
      opportunityGate: {
        meanModuleAtMost100KiB: bytes.length > 0 &&
          bytes.reduce((total, value) => total + value, 0) / bytes.length <= 100 * 1024,
        coversAtLeast40PercentOfProductionInterpreted:
          optimisticCoveredWindow >= Number(productionInterpreted) * 0.4,
      },
    });
  }
}

const result = {
  format: "rv64-scorecard-v2-early-wasm-opportunity-v1",
  capturedAt: new Date().toISOString(),
  proofOnly: true,
  inputs: Object.fromEntries(Object.entries({ loaderBytes, wasm, kernel, initramfs })
    .map(([name, bytes]) => [name === "loaderBytes" ? "loader" : name,
      createHash("sha256").update(bytes).digest("hex")])),
  configuration: { traceSlice, earlyThreshold, compileTop, emitLimit, leaderCaps, stateModes,
    productionInterpretedInstructions: productionInterpreted },
  traceElapsedMs,
  meta,
  thresholdOpportunity,
  selectedPages: selectedPages.map((page) => ({
    page: page.page, pa: page.pa, total: page.total, uniqueEntries: page.uniqueEntries,
    codeConflicts: page.codeConflicts,
    mapping: page.mapping,
  })),
  emissionAggregates,
  emitted,
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
