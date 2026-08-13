#!/usr/bin/env node
// The compile-policy tracer is opt-in and interpreter-only. A one-instruction
// physical-page loop gives exact, deterministic expectations for every core
// counter without requiring Linux images.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

const vm = await RV64.create(wasm);
vm.onWrite = () => {};
vm.ex.jit_set_enabled(0);
vm.ex.policy_trace_set_enabled(1);
vm.bootVirtLinuxDirect({
  // jal x0, 0 at the direct-kernel load address (RAM + 2 MiB).
  kernel: new Uint8Array([0x6f, 0x00, 0x00, 0x00]),
  ramMB: 32,
});

const budget = 100_000n;
assert.equal(vm.runVirtSystem(budget), false);
assert.equal(vm.virtInsnCount(), budget);

assert.equal(vm.ex.policy_trace_meta(0), 2n, "unexpected trace schema");
assert.equal(vm.ex.policy_trace_meta(1), 1n, "trace not enabled");
assert.equal(vm.ex.policy_trace_meta(4), budget, "wrong observed count");
assert.equal(vm.ex.policy_trace_meta(5), 1n, "loop should touch one page");
assert.equal(vm.ex.policy_trace_meta(6), 6n, "wrong heat-event count");
assert.equal(vm.ex.policy_trace_meta(7), 0n, "events unexpectedly dropped");
assert.equal(vm.ex.policy_trace_meta(8), 0n, "executed outside RAM");
assert.equal(vm.ex.policy_trace_meta(9), 16_384n, "wrong trace quantum");
assert.equal(vm.ex.policy_trace_meta(13), 1n, "loop should have one context");

assert.equal(vm.ex.policy_trace_page_stat(0, 0), 0x8020_0000n);
assert.equal(vm.ex.policy_trace_page_stat(0, 2), budget);
assert.equal(vm.ex.policy_trace_page_stat(0, 5), 1n, "wrong unique-PC count");
assert.equal(vm.ex.policy_trace_page_stat(0, 6), 1n, "wrong entry count");
assert.equal(vm.ex.policy_trace_page_stat(0, 7), budget, "wrong transfer count");
assert.equal(vm.ex.policy_trace_page_stat(0, 8), budget, "wrong backedge count");
assert.equal(vm.ex.policy_trace_page_stat(0, 9), 0n, "unexpected cross-page exit");
assert.equal(vm.ex.policy_trace_event_stat(0, 0), 16_384n);
assert.equal(vm.ex.policy_trace_event_stat(0, 1), 0x8020_0000n);
assert.equal(vm.ex.policy_trace_event_stat(0, 3), 16_384n);
assert.equal(vm.ex.policy_trace_event_stat(0, 4), 16_384n);
assert.equal(vm.ex.policy_trace_event_stat(0, 9), 3n);

assert.equal(vm.ex.policy_trace_context_stat(0, 0), 0x8020_0000n);
assert.equal(vm.ex.policy_trace_context_stat(0, 2), 0x8020_0000n);
assert.equal(vm.ex.policy_trace_context_stat(0, 5), budget);
assert.equal(vm.ex.policy_trace_context_stat(0, 8), 1n);
assert.equal(vm.ex.policy_trace_context_stat(0, 9), 1n);

// Disabling collection must retain the completed capture and return subsequent
// slices to the exact no-trace interpreter path.
vm.ex.policy_trace_set_enabled(0);
assert.equal(vm.runVirtSystem(10_000n), false);
assert.equal(vm.ex.policy_trace_meta(4), budget);
assert.equal(vm.ex.jit_stat(0), 0n);
assert.equal(vm.ex.jit_stat(1), 0n);

console.log("PASS policy trace smoke — exact one-page heat and retained capture");
