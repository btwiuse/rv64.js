#!/usr/bin/env node
// The sampled policy driver must preserve a WFI stop that occurs after useful
// retirement. Otherwise one public call runs through repeated idle wakeups and
// consumes its entire instruction budget before JavaScript can service work.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

const I = (op, f3, rd, rs1, imm) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | ((imm & 0xfff) << 20);
const J = (rd, off) => {
  const value = off & 0x1f_ffff;
  return 0x6f | (rd << 7) | (((value >> 12) & 0xff) << 12) |
    (((value >> 11) & 1) << 20) | (((value >> 1) & 0x3ff) << 21) |
    (((value >> 20) & 1) << 31);
};
function words(...code) {
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

const vm = await RV64.create(wasm);
vm.onWrite = () => {};
vm.ex.jit_set_enabled(1);
vm.ex.jit_set_page_policy(1);
vm.ex.jit_set_page_threshold(0xffff_ffff);
vm.ex.jit_set_page_quantum(1024);
vm.bootVirtLinuxDirect({
  kernel: words(
    I(0x13, 0, 1, 1, 1), // addi x1,x1,1: useful work before sleep
    0x1050_0073,          // wfi
    J(0, -8),             // repeat on the next host wakeup
  ),
  ramMB: 32,
});

const before = vm.virtInsnCount();
assert.equal(vm.runVirtSystem(2_000_000n), false);
const retired = vm.virtInsnCount() - before;
assert.ok(retired >= 2n && retired <= 4n, `partial-progress WFI retired ${retired}`);
assert.equal(vm.ex.jit_stat(0), 0n);
assert.equal(vm.ex.jit_stat(77), 0n);

console.log(`PASS page-policy partial-progress WFI yield — ${retired} instructions`);
