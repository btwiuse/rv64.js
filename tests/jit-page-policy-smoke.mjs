#!/usr/bin/env node
// The experimental policy must tier a hot page without ever invoking the
// synchronous one-block compiler, keep one async build in flight, and execute
// the landed page function.

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
const modules = [];
vm.onWrite = () => {};
vm.onJitModule = (_bytes, metadata) => modules.push(metadata);
vm.ex.jit_set_enabled(1);
vm.ex.jit_set_page_policy(1);
vm.ex.jit_set_page_threshold(200_000);
// This first lifecycle case isolates asynchronous policy mechanics. Mode-aware
// threshold behavior is exercised independently below.
vm.ex.jit_set_privileged_page_threshold_multiplier(1);
vm.bootVirtLinuxDirect({
  kernel: new Uint8Array([0x6f, 0x00, 0x00, 0x00]), // jal x0, 0
  ramMB: 32,
});

assert.equal(vm.runVirtSystem(500_000n), false);
assert.equal(vm.ex.jit_page_policy_stat(0), 1n);
assert.equal(vm.ex.jit_page_policy_stat(1), 200_000n);
assert.equal(vm.ex.jit_page_policy_stat(6), 1n, "hot page did not become a candidate");
assert.equal(vm.ex.jit_page_policy_stat(11), 1n, "page build was not issued");
assert.equal(vm.ex.jit_page_policy_stat(8), 1n, "build should still be pending synchronously");
assert.equal(vm.ex.jit_stat(77), 1n, "expected one page translation");

for (let attempt = 0; attempt < 200 && vm.ex.jit_page_policy_stat(12) === 0n; attempt++) {
  await new Promise((resolve) => setImmediate(resolve));
}
assert.equal(vm.ex.jit_page_policy_stat(12), 1n, "async page function never landed");
assert.equal(vm.ex.jit_page_policy_stat(8), 0n, "pending set did not drain");
assert.equal(vm.ex.jit_page_policy_stat(14), 1n, "mapping not marked compiled");
assert.deepEqual(modules.map((module) => module.kind), ["async-region"]);

const generatedBefore = vm.ex.jit_stat(0);
assert.equal(vm.runVirtSystem(1_000_000n), false);
assert.ok(vm.ex.jit_stat(0) > generatedBefore, "landed page function did not execute");
assert.equal(vm.ex.jit_stat(77), 1n, "synchronous per-PC translation leaked in");
assert.equal(vm.jitRegCount, 1, "unexpected extra Wasm modules");

// The production mode-aware threshold must not spend the user-code compile
// budget on transient supervisor code. An S-mode loop crosses the base
// threshold without becoming a candidate, then crosses base*multiplier and
// does. This catches both a discarded privilege sample and a multiplier that
// is reported but not used by the selector.
const privilegedVm = await RV64.create(wasm);
privilegedVm.onWrite = () => {};
privilegedVm.ex.jit_set_enabled(1);
privilegedVm.ex.jit_set_page_policy(1);
privilegedVm.ex.jit_set_page_threshold(1_024);
privilegedVm.ex.jit_set_privileged_page_threshold_multiplier(4);
privilegedVm.ex.jit_set_page_quantum(64);
privilegedVm.bootVirtLinuxDirect({
  kernel: new Uint8Array([0x6f, 0x00, 0x00, 0x00]), // jal x0, 0
  ramMB: 32,
});
assert.equal(privilegedVm.runVirtSystem(3_000n), false);
assert.equal(privilegedVm.ex.jit_page_policy_stat(45), 4n);
assert.equal(privilegedVm.ex.jit_page_policy_stat(46), 0n, "S-mode heat counted as user");
assert.ok(privilegedVm.ex.jit_page_policy_stat(47) >= 3_000n, "S-mode heat not recorded");
assert.equal(
  privilegedVm.ex.jit_page_policy_stat(49),
  0n,
  "privileged page compiled at the user threshold",
);
assert.equal(privilegedVm.runVirtSystem(2_000n), false);
assert.equal(privilegedVm.ex.jit_page_policy_stat(49), 1n, "privileged threshold never fired");

// A deliberate generated-code side-exit can remain hot forever after policy
// has already tried it.  Linux sstatus CSR paths exposed this shape in the 9P
// workload.  Once the exact mapping/PC is in policy_attempted, subsequent
// fallbacks must retain exact interpreter retirement and stop-at-compiled
// behavior without paying the control-entry sampler again.
const fallbackVm = await RV64.create(wasm);
fallbackVm.onWrite = () => {};
fallbackVm.ex.jit_set_enabled(1);
fallbackVm.ex.jit_set_page_policy(1);
fallbackVm.ex.jit_set_page_threshold(1_024);
fallbackVm.ex.jit_set_page_quantum(64);
fallbackVm.ex.jit_set_page_control_entries(1);
const fallbackKernel = new Uint8Array(4096);
const fallbackCode = new DataView(fallbackKernel.buffer);
fallbackCode.setUint32(0, 0x10002073, true); // csrrs x0,sstatus,x0
fallbackCode.setUint32(4, 0xffdff06f, true); // jal x0,-4
fallbackVm.bootVirtLinuxDirect({ kernel: fallbackKernel, ramMB: 32 });

assert.equal(fallbackVm.runVirtSystem(100_000n), false);
assert.equal(fallbackVm.ex.jit_stat(77), 1n, "known-fallback translation attempt");
assert.equal(fallbackVm.ex.jit_page_policy_stat(13), 1n, "known-fallback rejection");
const samplesAfterAttempt = fallbackVm.ex.jit_page_policy_stat(3);
const sampledRetiredAfterAttempt = fallbackVm.ex.jit_page_policy_stat(4);
const controlsAfterAttempt = fallbackVm.ex.jit_page_policy_stat(24);
assert.ok(samplesAfterAttempt > 0n, "known fallback was never sampled initially");

assert.equal(fallbackVm.runVirtSystem(200_000n), false);
assert.equal(
  fallbackVm.ex.jit_page_policy_stat(3),
  samplesAfterAttempt,
  "known-final entry was sampled again",
);
assert.equal(fallbackVm.ex.jit_page_policy_stat(4), sampledRetiredAfterAttempt);
assert.equal(fallbackVm.ex.jit_page_policy_stat(24), controlsAfterAttempt);
assert.equal(fallbackVm.ex.jit_stat(5), 300_000n, "known-fallback interpreter retirement");
assert.equal(fallbackVm.ex.jit_stat(0), 0n, "known-fallback loop unexpectedly generated");
assert.equal(fallbackVm.ex.virt_pc(), 0x8020_0000n, "known-fallback architectural PC");

console.log(
  "PASS page policy smoke — one async page module; known-final fallback stops sampling",
);
