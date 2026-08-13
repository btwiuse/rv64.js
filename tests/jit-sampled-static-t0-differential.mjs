#!/usr/bin/env node

// R072 semantic gate: the sampled-static decoder must reproduce the ordered
// page-policy callback stream, not merely finish with the same guest output.
// A deterministic supervisor-mode program mixes scalar control/memory with A
// and FENCE slow exits. The maximum threshold prevents asynchronous publish
// timing from becoming part of this observation test.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(
  process.env.RV64_WASM ??
    join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

const R = (op, f3, f7, rd, rs1, rs2) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | (rs2 << 20) | (f7 << 25);
const I = (op, f3, rd, rs1, imm) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | ((imm & 0xfff) << 20);
const S = (f3, rs1, rs2, imm) => {
  const value = imm & 0xfff;
  return 0x23 | ((value & 0x1f) << 7) | (f3 << 12) | (rs1 << 15) |
    (rs2 << 20) | ((value >> 5) << 25);
};
const U = (op, rd, imm20) => op | (rd << 7) | ((imm20 & 0xfffff) << 12);
const B = (f3, rs1, rs2, off) => {
  const value = off & 0x1fff;
  return 0x63 | (f3 << 12) | (rs1 << 15) | (rs2 << 20) |
    (((value >> 11) & 1) << 7) | (((value >> 1) & 0xf) << 8) |
    (((value >> 5) & 0x3f) << 25) | (((value >> 12) & 1) << 31);
};
const J = (rd, off) => {
  const value = off & 0x1f_ffff;
  return 0x6f | (rd << 7) | (((value >> 12) & 0xff) << 12) |
    (((value >> 11) & 1) << 20) | (((value >> 1) & 0x3ff) << 21) |
    (((value >> 20) & 1) << 31);
};
const AMO = (funct5, rd, rs1, rs2) =>
  0x2f | (rd << 7) | (3 << 12) | (rs1 << 15) | (rs2 << 20) |
  (funct5 << 27);

function program() {
  const words = [
    U(0x17, 20, 0x10),       // data = pc + 64 KiB
    I(0x13, 0, 1, 0, 1),    // x1 = running value
    I(0x13, 0, 2, 0, 97),   // x2 = inner loop count
    R(0x33, 0, 1, 3, 1, 2), // loop: mul x3,x1,x2
    S(3, 20, 3, 0),         // sd x3,0(x20)
    I(0x03, 3, 4, 20, 0),   // ld x4,0(x20)
    AMO(0, 5, 20, 1),       // amoadd.d (intentional static slow exit)
    0x0000_000f,             // fence (intentional static slow exit)
    I(0x13, 0, 1, 1, 3),
    I(0x13, 0, 2, 2, -1),
    B(1, 2, 0, -28),         // first mapped non-sequential target
    I(0x13, 0, 2, 0, 97),
    J(0, -36),               // a second control target in later samples
  ];
  const bytes = new Uint8Array(words.length * 4);
  const view = new DataView(bytes.buffer);
  words.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

function selfModifyingPageProgram() {
  const words = [
    U(0x17, 20, 0),          // x20 = current code page
    I(0x13, 0, 20, 20, 1024), // point at unused bytes on the same page
    S(3, 20, 0, 0),         // loop: dirty the marked code page without changing code
    I(0x13, 0, 1, 1, 1),
    J(0, -8),
  ];
  const bytes = new Uint8Array(2048);
  const view = new DataView(bytes.buffer);
  words.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

async function run(
  quantum,
  sampledStatic,
  {
    threshold = 0xffff_ffff,
    slices = [32_768n],
    shortBackoff = false,
    kernel = program(),
  } = {},
) {
  const vm = await RV64.create(wasm);
  vm.onWrite = () => {};
  vm.ex.jit_set_enabled(1);
  vm.ex.jit_set_page_policy(1);
  vm.ex.jit_set_page_threshold(threshold);
  vm.ex.jit_set_page_quantum(quantum);
  vm.ex.jit_set_page_control_entries(1);
  vm.ex.jit_set_page_privileged_control_entries(1);
  vm.ex.jit_set_page_policy_fingerprint(1);
  vm.bootVirtLinuxDirect({ kernel, ramMB: 32 });
  let staticIndex = -1;
  if (sampledStatic) {
    staticIndex = vm.ex.jit_static_t0_system_prepare();
    // R073 exercises the sampled mechanism without enabling R070 residuals.
    vm.ex.jit_set_static_t0_system(0);
    vm.ex.jit_set_static_t0_sampled(1);
    vm.ex.jit_set_static_t0_sampled_backoff(shortBackoff ? 1 : 0);
  }
  for (const slice of slices) {
    vm.runVirtSystem(slice);
    await new Promise((resolve) => setImmediate(resolve));
  }
  return {
    vm,
    pc: vm.ex.sys_pc(),
    insns: vm.ex.sys_insn_count(),
    regs: Array.from({ length: 32 }, (_, index) => vm.ex.sys_reg(index)),
    memory: vm.ex.sys_ram_u64(0x8021_0000n),
    fingerprint: vm.ex.jit_page_policy_fingerprint(0),
    events: vm.ex.jit_page_policy_fingerprint(1),
    policy: Array.from({ length: 52 }, (_, index) => vm.ex.jit_page_policy_stat(index)),
    staticIndex,
    staticFast: vm.ex.jit_static_t0_stat(3),
    staticErrors: vm.ex.jit_static_t0_stat(7),
    sampledRetired: vm.ex.jit_static_t0_stat(8),
    sampledChunks: vm.ex.jit_static_t0_stat(9),
    interruptPolls: vm.ex.jit_static_t0_stat(10),
    shortMarks: vm.ex.jit_static_t0_stat(11),
    shortBypasses: vm.ex.jit_static_t0_stat(12),
    shortClears: vm.ex.jit_static_t0_stat(13),
    jitInstructions: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(3),
  };
}

for (const quantum of [1, 32, 1024]) {
  const control = await run(quantum, false);
  const candidate = await run(quantum, true);
  const adaptive = await run(quantum, true, { shortBackoff: true });
  assert.equal(candidate.pc, control.pc, `q${quantum} PC`);
  assert.equal(candidate.insns, control.insns, `q${quantum} retirement`);
  assert.deepEqual(candidate.regs, control.regs, `q${quantum} registers`);
  assert.equal(candidate.memory, control.memory, `q${quantum} memory`);
  assert.equal(candidate.events, control.events, `q${quantum} observation count`);
  assert.equal(candidate.fingerprint, control.fingerprint, `q${quantum} observation order`);
  assert.deepEqual(candidate.policy, control.policy, `q${quantum} page-policy state`);
  assert.ok(candidate.staticIndex >= 0, `q${quantum} static module registration`);
  assert.ok(candidate.sampledRetired > 0n, `q${quantum} sampled static retirement`);
  assert.ok(candidate.sampledChunks > 0n, `q${quantum} sampled static chunks`);
  assert.equal(candidate.staticErrors, 0n, `q${quantum} static errors`);
  assert.equal(adaptive.pc, control.pc, `q${quantum} adaptive PC`);
  assert.equal(adaptive.insns, control.insns, `q${quantum} adaptive retirement`);
  assert.deepEqual(adaptive.regs, control.regs, `q${quantum} adaptive registers`);
  assert.equal(adaptive.memory, control.memory, `q${quantum} adaptive memory`);
  assert.equal(adaptive.events, control.events, `q${quantum} adaptive observation count`);
  assert.equal(
    adaptive.fingerprint,
    control.fingerprint,
    `q${quantum} adaptive observation order`,
  );
  assert.deepEqual(adaptive.policy, control.policy, `q${quantum} adaptive page-policy state`);
  assert.ok(adaptive.sampledRetired > 0n, `q${quantum} adaptive static retirement`);
  if (quantum < 64) {
    assert.ok(adaptive.shortMarks > 0n, `q${quantum} adaptive short marks`);
    assert.ok(adaptive.shortBypasses > 0n, `q${quantum} adaptive short bypasses`);
  } else {
    assert.equal(adaptive.shortMarks, 0n, `q${quantum} long samples were marked short`);
    assert.equal(adaptive.shortBypasses, 0n, `q${quantum} long samples were bypassed`);
    assert.equal(
      adaptive.sampledRetired,
      candidate.sampledRetired,
      `q${quantum} long samples did not remain static-eligible`,
    );
  }
  assert.equal(adaptive.staticErrors, 0n, `q${quantum} adaptive static errors`);
  console.log(
    `PASS sampled static T0 q${quantum} — events=${candidate.events} ` +
      `hash=0x${candidate.fingerprint.toString(16)} ` +
      `fast=${candidate.sampledRetired} polls=${candidate.interruptPolls} ` +
      `adaptive=${adaptive.sampledRetired} marks=${adaptive.shortMarks} ` +
      `bypasses=${adaptive.shortBypasses}`,
  );
}

// Exact handoff to an asynchronously landed generated entry. Publication time
// is intentionally not fingerprinted; after a fixed retirement budget both
// execution paths must converge on identical architectural state and prove
// that sampled static execution and generated execution both occurred.
const generatedSlices = Array.from({ length: 20 }, () => 32_768n);
const generatedControl = await run(1024, false, {
  threshold: 8192,
  slices: generatedSlices,
});
const generatedCandidate = await run(1024, true, {
  threshold: 8192,
  slices: generatedSlices,
});
const generatedAdaptive = await run(1024, true, {
  threshold: 8192,
  slices: generatedSlices,
  shortBackoff: true,
});
assert.equal(generatedCandidate.pc, generatedControl.pc, "generated-entry PC");
assert.equal(
  generatedCandidate.insns,
  generatedControl.insns,
  "generated-entry retirement",
);
assert.deepEqual(
  generatedCandidate.regs,
  generatedControl.regs,
  "generated-entry registers",
);
assert.equal(
  generatedCandidate.memory,
  generatedControl.memory,
  "generated-entry memory",
);
assert.ok(generatedControl.jitBlocks > 0n, "control generated no blocks");
assert.ok(generatedCandidate.jitBlocks > 0n, "candidate generated no blocks");
assert.ok(generatedCandidate.jitInstructions > 0n, "candidate ran no generated code");
assert.ok(generatedCandidate.sampledRetired > 0n, "candidate ran no sampled static code");
assert.equal(generatedCandidate.staticErrors, 0n, "generated-entry static errors");
assert.equal(generatedAdaptive.pc, generatedControl.pc, "adaptive generated-entry PC");
assert.equal(
  generatedAdaptive.insns,
  generatedControl.insns,
  "adaptive generated-entry retirement",
);
assert.deepEqual(
  generatedAdaptive.regs,
  generatedControl.regs,
  "adaptive generated-entry registers",
);
assert.equal(
  generatedAdaptive.memory,
  generatedControl.memory,
  "adaptive generated-entry memory",
);
assert.ok(generatedAdaptive.jitInstructions > 0n, "adaptive ran no generated code");
assert.ok(generatedAdaptive.sampledRetired > 0n, "adaptive ran no sampled static code");
assert.equal(generatedAdaptive.staticErrors, 0n, "adaptive generated-entry static errors");
console.log(
  `PASS sampled static T0 generated-entry handoff — sampled=` +
    `${generatedCandidate.sampledRetired} generated=${generatedCandidate.jitInstructions} ` +
    `blocks=${generatedCandidate.jitBlocks}`,
);

// A real store to unused bytes on the executing physical page exercises the
// same dirty-page invalidation that protects generated code. Backoff state is
// performance-only, but stale state must not survive a code-generation change.
const dirtyAdaptive = await run(32, true, {
  threshold: 128,
  slices: Array.from({ length: 20 }, () => 4096n),
  shortBackoff: true,
  kernel: selfModifyingPageProgram(),
});
assert.ok(dirtyAdaptive.shortMarks > 0n, "dirty-page case marked no short entries");
assert.ok(dirtyAdaptive.shortBypasses > 0n, "dirty-page case bypassed no short entries");
assert.ok(dirtyAdaptive.shortClears > 0n, "dirty-page case cleared no stale backoff entries");
assert.equal(dirtyAdaptive.staticErrors, 0n, "dirty-page static errors");
console.log(
  `PASS sampled static T0 dirty-page backoff lifecycle — marks=${dirtyAdaptive.shortMarks} ` +
    `bypasses=${dirtyAdaptive.shortBypasses} clears=${dirtyAdaptive.shortClears}`,
);

const resetAdaptive = await run(1, true, {
  slices: [4096n],
  shortBackoff: true,
});
assert.ok(resetAdaptive.shortMarks > 0n, "reset case established no backoff state");
resetAdaptive.vm.bootVirtLinuxDirect({ kernel: program(), ramMB: 32 });
assert.equal(resetAdaptive.vm.ex.jit_static_t0_stat(11), 0n, "reset retained short marks");
assert.equal(resetAdaptive.vm.ex.jit_static_t0_stat(12), 0n, "reset retained bypass count");
assert.equal(resetAdaptive.vm.ex.jit_static_t0_stat(13), 0n, "reset retained clear count");
assert.ok(resetAdaptive.vm.ex.jit_static_t0_system_prepare() >= 0);
resetAdaptive.vm.ex.jit_set_static_t0_system(0);
resetAdaptive.vm.ex.jit_set_static_t0_sampled(1);
resetAdaptive.vm.ex.jit_set_static_t0_sampled_backoff(1);
resetAdaptive.vm.runVirtSystem(4096n);
assert.ok(resetAdaptive.vm.ex.jit_static_t0_stat(11) > 0n, "reset did not relearn entries");
console.log("PASS sampled static T0 reset clears and relearns backoff state");

console.log("JIT SAMPLED STATIC T0 DIFFERENTIAL: ALL PASS");
