#!/usr/bin/env node

// Proof-only page-decoding opportunity probe for the exact modern scorecard
// boot. The guest runs with generated execution disabled and the existing
// instruction tracer enabled. Nothing reported here is a performance result:
// the tracer deliberately adds work. Exact dynamic page/transfer counts are
// used only to decide whether a page-guarded baseline tier is worth building.

import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const artifacts = process.env.ARTIFACTS
  ? resolve(process.env.ARTIFACTS)
  : null;
if (!artifacts) throw new Error("set ARTIFACTS");

const timeoutMs = Number(process.env.TIMEOUT_MS || 900_000);
if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 60_000) {
  throw new Error("TIMEOUT_MS must be an integer >= 60000");
}

const paths = {
  loader: join(root, "web/rv64.js"),
  wasm: join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  kernel: join(root, "web/images/alpine/Image"),
  initramfs: join(artifacts, "scorecard-v2-modern-riscv64.cpio"),
};
const [loaderBytes, wasm, kernel, initramfs] = await Promise.all([
  readFile(paths.loader),
  readFile(paths.wasm),
  readFile(paths.kernel),
  readFile(paths.initramfs),
]);
const { RV64Debug } = await import(pathToFileURL(paths.loader).href);
const vm = await RV64Debug.create(wasm);

let output = "";
const decoder = new TextDecoder();
vm.onWrite = (_fd, bytes) => {
  output += decoder.decode(bytes, { stream: true });
};
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
  if (vm.runVirtSystem(2_000_000n)) {
    throw new Error("guest powered off before SCORECARD_V2_READY");
  }
  if ((iteration & 15) === 0) await tick();
  if (performance.now() - started > timeoutMs) {
    throw new Error("scorecard boot trace timed out: " + output.slice(-3000));
  }
}
vm.ex.policy_trace_set_enabled(0);

const metaField = (field) => vm.ex.policy_trace_meta(field);
const meta = {
  schema: metaField(0),
  observedInstructions: metaField(4),
  touchedPages: metaField(5),
  events: metaField(6),
  droppedEvents: metaField(7),
  outsideRamInstructions: metaField(8),
  contexts: metaField(13),
};
if (meta.schema !== 2n) throw new Error("unsupported policy trace schema");
if (meta.droppedEvents !== 0n) throw new Error("policy trace event cap exhausted");
if (meta.outsideRamInstructions !== 0n) {
  throw new Error("guest executed instructions outside traced RAM");
}

const pages = Array.from({ length: Number(meta.touchedPages) }, (_, index) => ({
  pa: vm.ex.policy_trace_page_stat(index, 0),
  page: vm.ex.policy_trace_page_stat(index, 1),
  total: vm.ex.policy_trace_page_stat(index, 2),
  first: vm.ex.policy_trace_page_stat(index, 3),
  last: vm.ex.policy_trace_page_stat(index, 4),
  uniquePcs: vm.ex.policy_trace_page_stat(index, 5),
  uniqueEntries: vm.ex.policy_trace_page_stat(index, 6),
  transfers: vm.ex.policy_trace_page_stat(index, 7),
  backedges: vm.ex.policy_trace_page_stat(index, 8),
  crossPageExits: vm.ex.policy_trace_page_stat(index, 9),
}));

const sum = (values) => values.reduce((total, value) => total + value, 0n);
const observed = meta.observedInstructions;
const sorted = [...pages].sort((left, right) =>
  left.total === right.total ? 0 : left.total > right.total ? -1 : 1
);
const share = (value) => Number(value) / Number(observed);
const topShare = (count) => share(sum(sorted.slice(0, count).map((page) => page.total)));
const thresholdCoverage = [1, 16, 64, 256, 1024, 4096, 16384, 65536, 131072]
  .map((threshold) => {
    const selected = pages.filter((page) => page.total >= BigInt(threshold));
    return {
      threshold,
      pages: selected.length,
      coverage: share(sum(selected.map((page) => page.total))),
    };
  });

const transfers = sum(pages.map((page) => page.transfers));
const crossPageExits = sum(pages.map((page) => page.crossPageExits));
// This deliberately double-counts cross-page control transfers. Therefore the
// event count is an upper bound and the resulting instruction span is a safe
// lower bound for work amortized by one decoded-block/page guard.
const guardEventsUpperBound = transfers + crossPageExits;
const result = {
  format: "rv64-scorecard-v2-page-opportunity-v1",
  capturedAt: new Date().toISOString(),
  proofOnly: true,
  elapsedMs: performance.now() - started,
  inputs: Object.fromEntries(Object.entries({ loaderBytes, wasm, kernel, initramfs })
    .map(([name, bytes]) => [
      name === "loaderBytes" ? "loader" : name,
      createHash("sha256").update(bytes).digest("hex"),
    ])),
  meta,
  totals: {
    transfers,
    backedges: sum(pages.map((page) => page.backedges)),
    crossPageExits,
    guardEventsUpperBound,
    instructionsPerGuardLowerBound:
      Number(observed) / Number(guardEventsUpperBound || 1n),
    uniquePhysicalPcSlots: sum(pages.map((page) => page.uniquePcs)),
    uniquePhysicalEntries: sum(pages.map((page) => page.uniqueEntries)),
  },
  concentration: {
    top1: topShare(1),
    top10: topShare(10),
    top100: topShare(100),
    thresholdCoverage,
  },
  hottestPages: sorted.slice(0, 20),
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
