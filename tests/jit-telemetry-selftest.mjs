#!/usr/bin/env node
import assert from "node:assert/strict";
import {
  formatJitBytes,
  formatJitCount,
  formatJitDuration,
  summarizeJitStats,
} from "../web/jit-telemetry.mjs";

assert.equal(formatJitCount("999"), "999");
assert.equal(formatJitCount("1000"), "1.00K");
assert.equal(formatJitCount("12345"), "12.3K");
assert.equal(formatJitCount("1234567"), "1.23M");
assert.equal(formatJitCount("1234567890"), "1.23B");
assert.equal(formatJitBytes(0), "0 B");
assert.equal(formatJitBytes(1536), "1.50 KiB");
assert.equal(formatJitBytes(1572864), "1.50 MiB");
assert.equal(formatJitDuration(0), "0 ms");
assert.equal(formatJitDuration(0.5), "<1 ms");
assert.equal(formatJitDuration(12.34), "12.3 ms");
assert.equal(formatJitDuration(1500), "1.50 s");

const fixture = {
  generated: {
    retired: "9000000000",
    dispatches: "100000000",
    chainHops: "50000000",
    zeroRetireDispatches: "7",
    zeroRetireSuppressions: "6",
    dispatchEmptyMisses: "2",
  },
  interpreter: { retired: "1000000000" },
  instructions: "10000000000",
  generatedCoverage: 0.9,
  generatedInstructionsPerDispatch: 90,
  loader: { modules: 42, bytes: 1572864, compileMs: 12.34 },
  translation: { userNanoseconds: "2000000", systemNanoseconds: "3000000" },
  regions: { issued: "50", landed: "47", pending: "2", translateFailures: "1" },
  pagePolicy: { queued: "3", failed: "2" },
};
const summary = summarizeJitStats(fixture);
assert.deepEqual(summary, {
  state: "Compiling",
  detail: "2 builds in flight",
  coverage: "90.0% generated",
  execution: "9.00B JIT · 1.00B interpreted",
  code: "42 modules · 1.50 MiB",
  codeDetail: "5.0 ms translate · 12.3 ms host compile",
  pipeline: "50 issued · 47 landed · 2 pending",
  pipelineDetail: "3 regions queued · 1 translation failure · 2 build failures",
  dispatch: "100M dispatches · 90.0 insn/dispatch",
  dispatchDetail: "50.0M direct chains",
  fallback: "7 zero-retire · 6 suppressed",
  fallbackDetail: "2 empty-cache misses",
});

assert.equal(summarizeJitStats({}).state, "Cold");
assert.equal(summarizeJitStats({ instructions: "1" }).state, "Profiling");
assert.equal(summarizeJitStats({ loader: { modules: 1 } }).state, "Installed");
assert.equal(summarizeJitStats({ pagePolicy: { queued: "1" } }).state, "Queued");
assert.equal(summarizeJitStats({ generated: { retired: "1" }, instructions: "2" }).state, "Active");

console.log("PASS live JIT telemetry formatting and state reduction");
