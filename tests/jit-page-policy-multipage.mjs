#!/usr/bin/env node
// The production page policy must honour regionPageCap. Two observed adjacent
// mappings should form one mapping-checked module at cap=2, while cap=1 must
// retain the exact one-page geometry for controlled A/B experiments.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

const J = (rd, off) => {
  const value = off & 0x1f_ffff;
  return 0x6f | (rd << 7) | (((value >> 12) & 0xff) << 12) |
    (((value >> 11) & 1) << 20) | (((value >> 1) & 0x3ff) << 21) |
    (((value >> 20) & 1) << 31);
};

function twoPageLoop() {
  const bytes = new Uint8Array(0x1004);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, J(0, 0xffc) >>> 0, true);       // first page start -> end
  view.setUint32(0xffc, J(0, 4) >>> 0, true);       // cross into page two
  view.setUint32(0x1000, J(0, -0x1000) >>> 0, true); // back to page one
  return bytes;
}

function twoPageCallLoop() {
  const bytes = new Uint8Array(0x1004);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, J(1, 0x1000) >>> 0, true);      // call into page two
  view.setUint32(4, J(0, -4) >>> 0, true);          // repeat after return
  view.setUint32(0x1000, 0x0000_8067, true);        // jalr x0,0(ra)
  return bytes;
}

function straddlingInstructionLoop() {
  const bytes = new Uint8Array(0x1006);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, J(0, 0xffe) >>> 0, true);       // enter at page offset 0xffe
  view.setUint32(0xffe, 0x0010_8093, true);         // addi x1,x1,1 spans pages
  view.setUint32(0x1002, J(0, -4) >>> 0, true);     // return to the straddler
  return bytes;
}

async function run(
  pageCap,
  entryCap = 512,
  kernel = twoPageLoop(),
  crossPageCalls = true,
  controlPermille = 1000,
  controlEntries = false,
  controlProfile = false,
) {
  const vm = await RV64.create(wasm);
  vm.onWrite = () => {};
  vm.ex.jit_set_enabled(1);
  vm.ex.jit_set_page_policy(1);
  vm.ex.jit_set_page_threshold(256);
  // Geometry is the variable under test; privilege-aware tiering has its own
  // smoke case and would move this synthetic S-mode loop's threshold.
  vm.ex.jit_set_privileged_page_threshold_multiplier(1);
  vm.ex.jit_set_page_quantum(1);
  vm.ex.jit_set_region_page_cap(pageCap);
  vm.ex.jit_set_page_multipage_entry_cap(entryCap);
  vm.ex.jit_set_page_multipage_control_permille(controlPermille);
  vm.ex.jit_set_page_control_entries(controlEntries ? 1 : 0);
  vm.ex.jit_set_page_privileged_control_entries(1);
  vm.ex.jit_set_page_control_profile(controlProfile ? 1 : 0);
  vm.ex.jit_set_page_cross_page_calls(crossPageCalls ? 1 : 0);
  vm.ex.jit_set_region_leader_cap(32);
  vm.bootVirtLinuxDirect({ kernel, ramMB: 32 });

  assert.equal(vm.runVirtSystem(4096n), false);
  assert.equal(vm.ex.jit_page_policy_stat(11), 1n, "first module was not issued");
  for (let attempt = 0; attempt < 200 && vm.ex.jit_page_policy_stat(12) === 0n; attempt++) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.ok(vm.ex.jit_page_policy_stat(12) >= 1n, "async region never landed");

  const result = {
    pages: vm.ex.jit_page_policy_stat(19),
    multi: vm.ex.jit_page_policy_stat(20),
  };
  const generatedBefore = vm.ex.jit_stat(0);
  assert.equal(vm.runVirtSystem(100_000n), false);
  assert.ok(vm.ex.jit_stat(0) > generatedBefore, "landed region did not execute");
  return result;
}

assert.deepEqual(await run(1), { pages: 1n, multi: 0n });
assert.deepEqual(await run(2), { pages: 2n, multi: 1n });
assert.deepEqual(await run(2, 0), { pages: 1n, multi: 0n });
assert.deepEqual(await run(2, 512, twoPageCallLoop(), true), { pages: 2n, multi: 1n });
assert.deepEqual(await run(2, 512, twoPageCallLoop(), false), { pages: 1n, multi: 0n });
assert.deepEqual(await run(2, 512, twoPageLoop(), true, 0, true), {
  pages: 1n,
  multi: 0n,
});
// Fetch geometry overrides the optional control-flow expansion gate: without
// page two, the legal 32-bit instruction at offset 0xffe cannot be decoded.
assert.deepEqual(await run(2, 512, straddlingInstructionLoop(), true, 0, true), {
  pages: 2n,
  multi: 1n,
});
assert.deepEqual(await run(2, 512, twoPageLoop(), true, 0, false, true), {
  pages: 1n,
  multi: 0n,
});

console.log(
  "PASS page policy multipage selection — cap, entry/control gates, and call gate execute",
);
