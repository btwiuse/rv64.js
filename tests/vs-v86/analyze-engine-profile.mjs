#!/usr/bin/env node

// Summarize phase-isolated Node/V8 CPU profiles emitted by scorecard-v2-worker.
// Self time is reconstructed from sample IDs and their time deltas. Runtime
// Wasm and dynamically generated guest-code Wasm are deliberately separated;
// the latter uses anonymous per-module wasm:// URLs in both rv64.js and v86.

import { readFile, writeFile } from "node:fs/promises";
import { basename } from "node:path";

const arguments_ = process.argv.slice(2);
const jsonMode = arguments_.includes("--json") || arguments_.some((argument) =>
  argument.startsWith("--output="));
const outputPath = arguments_.find((argument) => argument.startsWith("--output="))
  ?.slice("--output=".length);
const paths = arguments_.filter((argument) => !argument.startsWith("--"));
if (!paths.length) {
  throw new Error(
    "usage: analyze-engine-profile.mjs [--json] [--output=REPORT.json] " +
    "PROFILE.cpuprofile [...]",
  );
}

function category(frame) {
  const { functionName = "", url = "" } = frame;
  if (functionName === "(idle)") return "idle";
  if (functionName === "(garbage collector)") return "gc";
  if (functionName === "(program)" || functionName === "(root)") return "v8/native";
  if (url.includes("rv64_wasm.wasm-") || url.includes("v86.wasm-")) {
    return "runtime-wasm";
  }
  if (url.startsWith("wasm://wasm/")) return "generated-wasm";
  if (url.startsWith("file:") || url.startsWith("node:")) return "javascript";
  return "v8/native";
}

function compactName(frame) {
  const name = frame.functionName || "(anonymous)";
  const patterns = [
    ["run_system_jit", "rv64::run_system_jit"],
    ["run_interpreter_until", "rv64::run_interpreter_until"],
    ["page_policy_issue", "rv64::page_policy_issue"],
    ["3Cpu4step", "rv64::Cpu::step"],
    ["3Cpu9run_until", "rv64::Cpu::run_until"],
    ["3Cpu18run_until_observed", "rv64::Cpu::run_until_observed"],
    ["3Cpu3run", "rv64::Cpu::run"],
    ["3Cpu2ld", "rv64::Cpu::ld"],
    ["3Cpu2st", "rv64::Cpu::st"],
    ["10compressed6expand", "rv64::compressed::expand"],
    ["11interpreter3run", "v86::interpreter::run"],
    ["13interpreter0f3run", "v86::interpreter::run_fragment"],
    ["3cpu13modrm_resolve", "v86::modrm_resolve"],
    ["18___rdl_alloc_zeroed", "rust::alloc_zeroed"],
  ];
  for (const [needle, replacement] of patterns) {
    if (name.includes(needle)) return replacement;
  }
  if (name.startsWith("wasm-function[")) return name;
  if (name.length <= 72) return name;
  return `${name.slice(0, 69)}...`;
}

function add(map, key, microseconds) {
  map.set(key, (map.get(key) || 0) + microseconds);
}

function isRvScheduler(frame) {
  return (frame.functionName || "").includes("14run_system_jit");
}

// Attribute the complete subtree below run_system_jit, rather than only the
// leaf frame's self time. This keeps helper calls made by generated modules
// with generated execution and separates the two interpreter entry paths from
// scheduler and translation work.
function schedulerComponent(frame, self) {
  if (self) return "scheduler self";
  const kind = category(frame);
  const name = frame.functionName || "";
  if (kind === "generated-wasm") return "generated module subtree";
  if (name.includes("run_slice_sampled")) return "policy-sampled interpreter subtree";
  if (name.includes("run_interpreter_until")) return "final-outcome interpreter subtree";
  if (name.includes("run_slice_until")) return "final-outcome interpreter subtree";
  if (name.includes("page_policy_issue")) return "translation / issue subtree";
  if (name.includes("BuildHasher8hash_one") || name.includes("DefaultHasher")) {
    return "scheduler cache hashing";
  }
  if (name.includes("pump_virt_net")) return "host I/O pump";
  if (name.includes("run_policy_interpreter")) return "policy interpreter glue";
  return `other: ${compactName(frame)}`;
}

function summarize(profile) {
  const nodes = new Map(profile.nodes.map((node) => [node.id, node]));
  const parents = new Map();
  for (const node of profile.nodes) {
    for (const child of node.children || []) parents.set(child, node.id);
  }
  const self = new Map();
  const categories = new Map();
  const generatedModules = new Map();
  const scheduler = new Map();
  let sampledUs = 0;
  let schedulerUs = 0;
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
    sampledUs += microseconds;
    const frame = node.callFrame;
    const kind = category(frame);
    add(categories, kind, microseconds);
    add(self, `${kind}\0${compactName(frame)}\0${frame.url || ""}`, microseconds);
    if (kind === "generated-wasm") add(generatedModules, frame.url, microseconds);

    let cursor = node.id;
    let child = cursor;
    while (cursor !== undefined && !isRvScheduler(nodes.get(cursor).callFrame)) {
      child = cursor;
      cursor = parents.get(cursor);
    }
    if (cursor !== undefined) {
      schedulerUs += microseconds;
      add(
        scheduler,
        schedulerComponent(nodes.get(child).callFrame, child === cursor),
        microseconds,
      );
    }
  }
  const byTime = (map) => [...map.entries()].sort((left, right) => right[1] - left[1]);
  return {
    sampledUs,
    samples: samples.length,
    missingSamples,
    categories: byTime(categories),
    self: byTime(self),
    generatedModules: byTime(generatedModules),
    schedulerUs,
    scheduler: byTime(scheduler),
  };
}

const percent = (value, total) => total ? (100 * value / total).toFixed(2) : "0.00";
const milliseconds = (microseconds) => (microseconds / 1000).toFixed(2);

const reports = [];
for (const path of paths) {
  const profile = JSON.parse(await readFile(path, "utf8"));
  const result = summarize(profile);
  reports.push({ path, file: basename(path), ...result });
  if (jsonMode) continue;
  console.log(`\n${basename(path)}: ${milliseconds(result.sampledUs)} ms, ` +
    `${result.samples} samples, ${result.missingSamples} missing`);
  console.log("category             self_ms   percent");
  for (const [name, time] of result.categories) {
    console.log(`${name.padEnd(20)} ${milliseconds(time).padStart(9)} ` +
      `${percent(time, result.sampledUs).padStart(8)}%`);
  }
  console.log("\ntop self-time frames");
  for (const [key, time] of result.self.slice(0, 20)) {
    const [kind, name] = key.split("\0");
    console.log(`${milliseconds(time).padStart(9)} ms ` +
      `${percent(time, result.sampledUs).padStart(6)}% ` +
      `${kind.padEnd(15)} ${name}`);
  }
  if (result.schedulerUs) {
    console.log(`\nrun_system_jit subtree: ${milliseconds(result.schedulerUs)} ms ` +
      `(${percent(result.schedulerUs, result.sampledUs)}% of all samples)`);
    console.log("component                                  self_ms   of_subtree   of_total");
    for (const [name, time] of result.scheduler) {
      console.log(`${name.padEnd(42)} ${milliseconds(time).padStart(9)} ` +
        `${percent(time, result.schedulerUs).padStart(10)}% ` +
        `${percent(time, result.sampledUs).padStart(9)}%`);
    }
  }
  const generated = result.categories.find(([name]) => name === "generated-wasm")?.[1] || 0;
  console.log(`\ngenerated modules: ${result.generatedModules.length}; ` +
    `top 10 cover ${percent(
      result.generatedModules.slice(0, 10).reduce((sum, entry) => sum + entry[1], 0),
      generated,
    )}% of generated-Wasm self time`);
}

if (jsonMode) {
  const serialized = `${JSON.stringify({
    format: "rv64-engine-profile-analysis-v2",
    generatedAt: new Date().toISOString(),
    profiles: reports,
  }, null, 2)}\n`;
  if (outputPath) {
    await writeFile(outputPath, serialized, { flag: "wx" });
  } else {
    process.stdout.write(serialized);
  }
}
