#!/usr/bin/env node
// A synchronous system slice must return as soon as WFI makes no architectural
// progress. Host-side devices (including WANIX's asynchronous 9P bridge) cannot
// deliver their reply while JavaScript is trapped inside runSystem/virt_run.

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
const U = (op, rd, imm20) => op | (rd << 7) | ((imm20 & 0xfffff) << 12);
const B = (f3, rs1, rs2, off) => {
  const value = off & 0x1fff;
  return 0x63 | (f3 << 12) | (rs1 << 15) | (rs2 << 20) |
    (((value >> 11) & 1) << 7) | (((value >> 1) & 0xf) << 8) |
    (((value >> 5) & 0x3f) << 25) | (((value >> 12) & 1) << 31);
};
const WFI = 0x1050_0073;

function words(...code) {
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

async function run(name, bios, warmupBudget = 0n) {
  const vm = await RV64.create(wasm);
  vm.onWrite = () => {};
  vm.bootLinux({ bios, ramMB: 32 });

  if (warmupBudget) {
    vm.runSystem(warmupBudget);
    // Let any asynchronously compiled region publish before the measured call.
    await new Promise((resolve) => setImmediate(resolve));
    if (vm.ex.jit_stat(3) === 0n) {
      throw new Error(`${name}: setup loop did not exercise the warm JIT path`);
    }
  }

  const callsBefore = vm.ex.jit_stat(4);
  const insnsBefore = vm.ex.sys_insn_count();
  const started = performance.now();
  vm.runSystem(2_000_000n);
  const elapsed = performance.now() - started;
  const calls = vm.ex.jit_stat(4) - callsBefore;
  const insns = vm.ex.sys_insn_count() - insnsBefore;
  const blocks = vm.ex.jit_stat(3);

  // The exact number varies by whether WFI ends the current interpreter call
  // or the next one. It must stay constant-sized, never scale with the caller's
  // two-million-instruction budget.
  if (calls > 10_000n) {
    console.error(`${name} diagnostics`, {
      pc: vm.ex.sys_pc(),
      insns,
      calls,
      blocks,
    });
    throw new Error(`${name}: WFI spun for ${calls} interpreter slices`);
  }
  console.log(
    `PASS ${name} — ${insns} retired, ${calls} slices, ` +
      `${blocks} blocks, ${elapsed.toFixed(2)} ms`,
  );
}

await run("cold WFI", words(WFI));
await run(
  "warm WFI",
  words(
    U(0x37, 1, 0x40),       // lui x1,0x40: 262,144 loop iterations
    I(0x13, 0, 1, 1, -1),  // addi x1,x1,-1
    B(1, 1, 0, -4),        // bne x1,x0,loop
    WFI,
  ),
  500_000n,
);
console.log("JIT SYSTEM WFI YIELD: ALL PASS");
