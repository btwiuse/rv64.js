#!/usr/bin/env node
// Capture opt-in interpreter traces from the supported modern Linux machine.
// Examples:
//   node tests/jit-policy-trace.mjs --mode=direct --workloads=none
//   node tests/jit-policy-trace.mjs --mode=both --workloads=alu,mix

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  bootModern,
  guestCommand,
  loadModernImages,
  machineDiagnostics,
  missingModernImages,
  output,
  transferBinary,
  waitForAlpine,
} from "./modern-linux-harness.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const options = Object.fromEntries(process.argv.slice(2).map((arg) => {
  const [key, ...value] = arg.replace(/^--/, "").split("=");
  return [key, value.length ? value.join("=") : "true"];
}));
const modes = (options.mode || "direct").split(",").flatMap((mode) =>
  mode === "both" ? ["direct", "opensbi"] : [mode]
);
const workloads = (options.workloads || "none") === "none"
  ? []
  : (options.workloads || "").split(",");
const timeoutMs = Number(options.timeout || 1_200_000);
const outDir = resolve(root, options.out || "target/jit-policy-traces");
const syscomputePath = resolve(
  root,
  options.syscompute ||
    "guests/syscompute/target/riscv64gc-unknown-linux-musl/release/syscompute",
);

for (const mode of modes) {
  if (mode !== "direct" && mode !== "opensbi") {
    throw new Error(`unknown mode ${mode}; use direct, opensbi, or both`);
  }
}
for (const workload of workloads) {
  if (workload !== "alu" && workload !== "mix") {
    throw new Error(`unknown workload ${workload}; use alu and/or mix`);
  }
}

const missing = [...new Set(modes.flatMap((mode) => missingModernImages(root, mode)))];
if (missing.length) {
  throw new Error(`missing modern Linux images: ${missing.join(", ")}`);
}

const wasmPath = join(
  root,
  "target/wasm32-unknown-unknown/release/rv64_wasm.wasm",
);
const wasm = await readFile(wasmPath);
const images = await loadModernImages(root);
const syscompute = workloads.length ? new Uint8Array(await readFile(syscomputePath)) : null;
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
await mkdir(outDir, { recursive: true });

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const hashes = {
  wasm: hash(wasm),
  kernel: hash(images.kernel),
  disk: hash(images.disk),
  opensbi: hash(images.opensbi),
  ...(syscompute ? { syscompute: hash(syscompute) } : {}),
};
let gitCommit = "unknown";
try {
  gitCommit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  }).trim();
} catch {
  // The Wasm hash remains the authoritative executable identity.
}

function traceMeta(vm) {
  const get = (field) => vm.ex.policy_trace_meta(field);
  return {
    schema: get(0),
    enabled: get(1),
    originIcount: get(2),
    lastIcount: get(3),
    observedInstructions: get(4),
    touchedPages: get(5),
    events: get(6),
    droppedEvents: get(7),
    outsideRamInstructions: get(8),
    eventQuantum: get(9),
    ramBase: get(10),
    ramPages: get(11),
    eventCap: get(12),
    contexts: get(13),
  };
}

function extractTrace(vm, context) {
  const meta = traceMeta(vm);
  assert.equal(meta.schema, 2n, "unsupported policy trace schema");
  assert.equal(meta.droppedEvents, 0n, "trace event cap was exhausted");
  assert.equal(meta.outsideRamInstructions, 0n, "guest executed outside RAM");

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
    eventCount: vm.ex.policy_trace_page_stat(index, 10),
  }));
  const contexts = Array.from({ length: Number(meta.contexts) }, (_, index) => ({
    pa: vm.ex.policy_trace_context_stat(index, 0),
    page: vm.ex.policy_trace_context_stat(index, 1),
    vpage: vm.ex.policy_trace_context_stat(index, 2),
    satp: vm.ex.policy_trace_context_stat(index, 3),
    mode: vm.ex.policy_trace_context_stat(index, 4),
    total: vm.ex.policy_trace_context_stat(index, 5),
    first: vm.ex.policy_trace_context_stat(index, 6),
    last: vm.ex.policy_trace_context_stat(index, 7),
    uniquePcs: vm.ex.policy_trace_context_stat(index, 8),
    uniqueEntries: vm.ex.policy_trace_context_stat(index, 9),
    eventCount: vm.ex.policy_trace_context_stat(index, 10),
  }));
  const events = Array.from({ length: Number(meta.events) }, (_, index) => ({
    at: vm.ex.policy_trace_event_stat(index, 0),
    pa: vm.ex.policy_trace_event_stat(index, 1),
    page: vm.ex.policy_trace_event_stat(index, 2),
    pageHeat: vm.ex.policy_trace_event_stat(index, 3),
    contextHeat: vm.ex.policy_trace_event_stat(index, 4),
    va: vm.ex.policy_trace_event_stat(index, 5),
    vpage: vm.ex.policy_trace_event_stat(index, 6),
    satp: vm.ex.policy_trace_event_stat(index, 7),
    mode: vm.ex.policy_trace_event_stat(index, 8),
    kind: vm.ex.policy_trace_event_stat(index, 9),
  }));
  return {
    format: "rv64-jit-policy-trace-v2",
    capturedAt: new Date().toISOString(),
    gitCommit,
    hashes,
    context,
    meta,
    pages,
    contexts,
    events,
  };
}

async function saveTrace(mode, phase, vm, context) {
  vm.ex.policy_trace_set_enabled(0);
  const trace = extractTrace(vm, { mode, phase, ...context });
  const file = join(outDir, `${mode}-${phase}.json`);
  await writeFile(
    file,
    JSON.stringify(trace, (_key, value) =>
      typeof value === "bigint" ? value.toString() : value, 2) + "\n",
  );
  const hottest = [...trace.pages].sort((a, b) =>
    Number(b.total - a.total)
  )[0];
  console.log(
    `TRACE ${mode}/${phase}: ${trace.meta.observedInstructions} instructions, ` +
      `${trace.pages.length} pages, ${trace.events.length} events, ` +
      `hottest=${hottest?.total ?? 0} -> ${file}`,
  );
  return trace;
}

for (const mode of modes) {
  const instantiateStart = performance.now();
  const machine = await bootModern({
    RV64,
    wasm,
    images,
    mode,
    jit: false,
    configure: (vm) => vm.ex.policy_trace_set_enabled(1),
  });
  const instantiateMs = performance.now() - instantiateStart;
  const executeStart = performance.now();
  const ready = await waitForAlpine(machine, timeoutMs);
  const executeMs = performance.now() - executeStart;
  assert.ok(ready, `${mode} traced boot timeout: ${machineDiagnostics(machine)}`);
  assert.match(output(machine), /Linux version 6\./);
  if (mode === "opensbi") assert.match(output(machine), /OpenSBI v/);
  await saveTrace(mode, "boot", machine.vm, {
    instantiateMs,
    executeMs,
    endGuestIcount: machine.vm.virtInsnCount(),
  });

  if (!syscompute) continue;
  const transferred = await transferBinary(
    machine,
    syscompute,
    "/tmp/syscompute",
    `POLICY_${mode.toUpperCase()}_BIN`,
  );
  assert.ok(transferred, `${mode} syscompute transfer failed: ${machineDiagnostics(machine)}`);

  for (const workload of workloads) {
    machine.vm.ex.policy_trace_set_enabled(1);
    const startOutput = output(machine).length;
    const startIcount = machine.vm.virtInsnCount();
    const started = performance.now();
    const marker = `POLICY_${workload.toUpperCase()}_DONE`;
    const done = await guestCommand(
      machine,
      `/tmp/syscompute ${workload}; echo POLICY_${workload.toUpperCase()}_'DONE'`,
      marker,
      timeoutMs,
    );
    const executeMs = performance.now() - started;
    assert.ok(done, `${mode}/${workload} timeout: ${machineDiagnostics(machine)}`);
    const phaseOutput = output(machine).slice(startOutput);
    await saveTrace(mode, workload, machine.vm, {
      executeMs,
      startGuestIcount: startIcount,
      endGuestIcount: machine.vm.virtInsnCount(),
      consoleTail: phaseOutput.slice(-500),
    });
  }
}
