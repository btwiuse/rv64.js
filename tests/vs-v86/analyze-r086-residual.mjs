#!/usr/bin/env node

// Closure-aware attribution for R086's exact-R085 V8 CPU profiles.
//
// The generic engine-profile report deliberately groups samples by the first
// child below run_system_jit. That is useful for scheduler ownership, but it
// hides operations reached through policy/translation helpers. This analyzer
// reconstructs every sampled stack, classifies the exclusive leaf operation,
// and records both the first scheduler child and nearest project caller.

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { basename } from "node:path";

const arguments_ = process.argv.slice(2);
const outputPath = arguments_
  .find((argument) => argument.startsWith("--output="))
  ?.slice("--output=".length);
const paths = arguments_.filter((argument) => !argument.startsWith("--"));

if (!paths.length) {
  throw new Error(
    "usage: analyze-r086-residual.mjs [--output=REPORT.json] PROFILE.cpuprofile [...]",
  );
}

const add = (map, key, value) => map.set(key, (map.get(key) || 0) + value);
const descending = (map) =>
  [...map.entries()].sort((left, right) => right[1] - left[1] ||
    left[0].localeCompare(right[0]));
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function broadCategory(frame) {
  const name = frame.functionName || "";
  const url = frame.url || "";
  if (name === "(idle)") return "idle";
  if (name === "(garbage collector)") return "garbage collector";
  if (url.includes("rv64_wasm.wasm-")) return "runtime Wasm";
  if (url.startsWith("wasm://wasm/")) return "generated guest Wasm";
  if (url.startsWith("file:") || url.startsWith("node:")) return "JavaScript";
  return "V8 / native";
}

function isScheduler(frame) {
  return (frame.functionName || "").includes("14run_system_jit");
}

function projectFrame(name) {
  return name.includes("rv64_wasm") || name.includes("rv64_core") ||
    name.includes("rv64_system") || name.includes("rv64_dbt") ||
    name === "virt_run" || name === "sys_run";
}

function shortName(frame) {
  const name = frame?.functionName || "(anonymous)";
  const patterns = [
    ["14run_system_jit", "rv64_wasm::run_system_jit"],
    ["21run_interpreter_until", "rv64_wasm::run_interpreter_until"],
    ["22run_policy_interpreter", "rv64_wasm::run_policy_interpreter"],
    ["23run_slice_sampled_until", "VirtMachine::run_slice_sampled_until"],
    ["17run_slice_sampled", "VirtMachine::run_slice_sampled"],
    ["19page_policy_observe", "rv64_wasm::page_policy_observe"],
    ["17page_policy_issue", "rv64_wasm::page_policy_issue"],
    ["18run_until_observed", "Cpu::run_until_observed"],
    ["9run_until", "Cpu::run_until"],
    ["3Cpu3run", "Cpu::run"],
    ["3Cpu4step", "Cpu::step"],
    ["3Cpu2ld", "Cpu::ld"],
    ["3Cpu2st", "Cpu::st"],
    ["16check_interrupts", "Cpu::check_interrupts"],
    ["14translate_slow", "Cpu::translate_slow"],
    ["13pump_virt_net", "rv64_wasm::pump_virt_net"],
    ["FastBuildHasher", "FastHash table operation"],
    ["std4hash6random11RandomState", "RandomState table operation"],
    ["9hashbrown", "hashbrown table operation"],
    ["12wasm_encoder", "wasm_encoder operation"],
    ["8rv64_dbt", "rv64_dbt operation"],
    ["11rv64_system", "rv64_system operation"],
    ["9rv64_core", "rv64_core operation"],
  ];
  for (const [needle, replacement] of patterns) {
    if (name.includes(needle)) return replacement;
  }
  if (name.startsWith("wasm-function[")) return name;
  if (name.length <= 96) return name;
  return `${name.slice(0, 93)}...`;
}

function nearestProjectCaller(stack) {
  for (let index = stack.length - 1; index >= 0; index--) {
    const frame = stack[index].callFrame;
    if (projectFrame(frame.functionName || "")) return shortName(frame);
  }
  return shortName(stack.at(-1)?.callFrame);
}

function firstBelowScheduler(stack) {
  let schedulerIndex = -1;
  for (let index = 0; index < stack.length; index++) {
    if (isScheduler(stack[index].callFrame)) schedulerIndex = index;
  }
  if (schedulerIndex < 0) return "outside run_system_jit";
  if (schedulerIndex === stack.length - 1) return "run_system_jit self";
  return shortName(stack[schedulerIndex + 1].callFrame);
}

function hasInStack(stack, needle) {
  return stack.some((node) =>
    (node.callFrame.functionName || "").includes(needle));
}

function runtimeFamily(frame, stack) {
  const name = frame.functionName || "";

  // Table families come first: their mangled names often also contain the
  // project type whose map is being manipulated.
  if (name.includes("FastBuildHasher") ||
      (name.includes("hashbrown") && hasInStack(stack, "FastBuildHasher"))) {
    return "R085 fast JIT-state table operations";
  }
  if (name.includes("DefaultHasher") || name.includes("std4hash6random11RandomState") ||
      (name.includes("hashbrown") && hasInStack(stack, "RandomState"))) {
    return "default-hash / non-JIT table operations";
  }
  if (name.includes("hashbrown")) return "other hash-table mechanics";

  if (name.includes("3Cpu4step")) return "interpreter decode and execute";
  if (name.includes("3Cpu2ld") || name.includes("3Cpu2st")) {
    return "interpreter scalar memory helpers";
  }
  if (name.includes("18run_until_observed")) {
    return "interpreter loop with control observation";
  }
  if (name.includes("3Cpu9run_until")) {
    return "interpreter loop with exact generated re-entry";
  }
  if (name.includes("3Cpu3run")) return "ordinary interpreter loop";
  if (name.includes("21run_interpreter_until") || name.includes("17run_slice_until")) {
    return "final-outcome interpreter driver";
  }
  if (name.includes("23run_slice_sampled_until") || name.includes("17run_slice_sampled")) {
    return "page-policy sampling driver";
  }
  if (name.includes("22run_policy_interpreter")) return "page-policy interpreter glue";
  if (name.includes("19page_policy_observe")) return "page-policy observation";
  if (name.includes("17page_policy_issue")) return "DBT translation and issue";
  if (isScheduler(frame)) return "scheduler / generated-dispatch inline work";

  if (name.includes("16check_interrupts") || name.includes("9irq_lines") ||
      name.includes("poll_virtio") || name.includes("sync_devices") ||
      name.includes("pump_virt_net")) {
    return "interrupt and device service";
  }
  if (name.includes("14translate_slow") || name.includes("jit_probe_fetch") ||
      name.includes("probe_fetch")) {
    return "address translation and fetch probes";
  }
  if (name.includes("3Cpu8csr_read") || name.includes("3Cpu9csr_write") ||
      name.includes("jit_system_reservation") || name.includes("bulk_copy")) {
    return "interpreter architectural helpers";
  }
  if (name.includes("10compressed6expand")) return "compressed-instruction decode";

  const translationAncestor = hasInStack(stack, "17page_policy_issue") ||
    hasInStack(stack, "8rv64_dbt") || hasInStack(stack, "12wasm_encoder");
  if (name.includes("8rv64_dbt") || name.includes("12wasm_encoder") ||
      (translationAncestor && (name.includes("alloc") || name.includes("core4iter") ||
        name.includes("collections5btree") || name.includes("drop_glue")))) {
    return "DBT translation and issue";
  }
  if (name.includes("dlmalloc") || name.includes("___rust_alloc") ||
      name.includes("___rust_dealloc") || name.includes("alloc3vec") ||
      name.includes("collections")) {
    return "allocation and general containers";
  }
  if (name.includes("11rv64_system") || name.includes("VirtBus")) {
    return "system memory and device helpers";
  }
  if (name.includes("9rv64_core")) return "other interpreter/runtime helpers";
  if (name.includes("9rv64_wasm")) return "other JIT policy/runtime helpers";
  return "other runtime Wasm";
}

function operationFamily(frame, stack) {
  const broad = broadCategory(frame);
  if (broad === "runtime Wasm") return runtimeFamily(frame, stack);
  if (broad === "generated guest Wasm") return "generated guest execution";
  if (broad === "JavaScript") {
    const name = frame.functionName || "";
    if (name.includes("host_jit") || name.includes("WebAssembly")) {
      return "host module compilation and registration";
    }
    return "JavaScript harness and host services";
  }
  return broad;
}

function ledgerStatus(family) {
  const closed = new Map([
    ["R085 fast JIT-state table operations", "closed by promoted R085"],
    ["default-hash / non-JIT table operations", "closed unless whole-profile evidence contradicts R085"],
    ["interpreter decode and execute", "current Rust/interpreter layout family closed through R083"],
    ["interpreter scalar memory helpers", "fetch/memory helper family closed through R067"],
    ["interpreter loop with control observation", "observer and scalar-driver variants closed"],
    ["interpreter loop with exact generated re-entry", "R040/R041/R056 evidence; old gate requires policy review"],
    ["ordinary interpreter loop", "scalar-driver and static-T0 layouts closed"],
    ["compressed-instruction decode", "decoder/inlining/cache families closed"],
    ["DBT translation and issue", "geometry/threshold/emitter families closed"],
    ["scheduler / generated-dispatch inline work", "scheduler thinning and dispatch-count families closed"],
    ["generated guest execution", "current backend lowering/layout families closed"],
  ]);
  return closed.get(family) || "inventory; requires source audit before admission";
}

function summarize(profile) {
  const nodes = new Map(profile.nodes.map((node) => [node.id, node]));
  const parents = new Map();
  for (const node of profile.nodes) {
    for (const child of node.children || []) parents.set(child, node.id);
  }

  const stacks = new Map();
  function stackFor(id) {
    if (stacks.has(id)) return stacks.get(id);
    const path = [];
    let cursor = id;
    const seen = new Set();
    while (cursor !== undefined && !seen.has(cursor)) {
      seen.add(cursor);
      const node = nodes.get(cursor);
      if (!node) break;
      path.push(node);
      cursor = parents.get(cursor);
    }
    path.reverse();
    stacks.set(id, path);
    return path;
  }

  const broad = new Map();
  const families = new Map();
  const attribution = new Map();
  const leaves = new Map();
  let sampledUs = 0;
  let missingSamples = 0;
  const samples = profile.samples || [];
  const deltas = profile.timeDeltas || [];

  for (let index = 0; index < samples.length; index++) {
    const node = nodes.get(samples[index]);
    if (!node) {
      missingSamples++;
      continue;
    }
    const microseconds = deltas[index] || 0;
    const stack = stackFor(node.id);
    const frame = node.callFrame;
    const category = broadCategory(frame);
    const family = operationFamily(frame, stack);
    const first = firstBelowScheduler(stack);
    const nearest = nearestProjectCaller(stack);
    sampledUs += microseconds;
    add(broad, category, microseconds);
    add(families, family, microseconds);
    add(attribution, `${family}\0${first}\0${nearest}`, microseconds);
    add(leaves, `${family}\0${shortName(frame)}\0${frame.functionName || ""}`, microseconds);
  }

  const sortedFamilies = descending(families);
  let explainedUs = 0;
  const explained95 = [];
  for (const entry of sortedFamilies) {
    if (explainedUs >= sampledUs * 0.95) break;
    explained95.push(entry);
    explainedUs += entry[1];
  }

  return {
    sampledUs,
    samples: samples.length,
    missingSamples,
    broadCategories: descending(broad),
    families: sortedFamilies.map(([family, microseconds]) => ({
      family,
      microseconds,
      fraction: sampledUs ? microseconds / sampledUs : 0,
      optimisticFreeSpeedup: sampledUs > microseconds
        ? sampledUs / (sampledUs - microseconds)
        : null,
      status: ledgerStatus(family),
    })),
    explained95: {
      families: explained95.map(([family]) => family),
      microseconds: explainedUs,
      fraction: sampledUs ? explainedUs / sampledUs : 0,
    },
    attribution: descending(attribution).map(([key, microseconds]) => {
      const [family, firstBelowScheduler_, nearestProjectCaller_] = key.split("\0");
      return {
        family,
        firstBelowScheduler: firstBelowScheduler_,
        nearestProjectCaller: nearestProjectCaller_,
        microseconds,
        fraction: sampledUs ? microseconds / sampledUs : 0,
      };
    }),
    leaves: descending(leaves).map(([key, microseconds]) => {
      const [family, leaf, fullFunctionName] = key.split("\0");
      return {
        family,
        leaf,
        fullFunctionName,
        microseconds,
        fraction: sampledUs ? microseconds / sampledUs : 0,
      };
    }),
  };
}

const reports = [];
for (const path of paths) {
  const bytes = await readFile(path);
  const profile = JSON.parse(bytes);
  reports.push({
    path,
    file: basename(path),
    sha256: sha256(bytes),
    ...summarize(profile),
  });
}

const report = {
  schema: 1,
  experiment: "R086",
  mechanism: "exact-R085-closure-aware-residual-attribution",
  performanceEvidence: false,
  profiles: reports,
};
const serialized = `${JSON.stringify(report, null, 2)}\n`;
if (outputPath) {
  await writeFile(outputPath, serialized, { flag: "wx" });
} else {
  process.stdout.write(serialized);
}
