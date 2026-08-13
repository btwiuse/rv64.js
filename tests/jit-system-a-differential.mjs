// Full-system RV64A differential for the typed system reservation capability.
// A hot bare-metal loop combines LR.D/SC.D success, a second failing SC, and
// AMOADD.D through the fused system-memory path.

import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(
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
const AMO = (funct5, aqrl, rd, address, source) =>
  R(0x2f, 3, (funct5 << 2) | aqrl, rd, address, source);
const NOP = I(0x13, 0, 0, 0, 0);

function program() {
  const code = [
    U(0x17, 20, 0x2), // data = RAM_BASE + 0x2000
    I(0x13, 0, 1, 0, 1),
    S(3, 20, 0, 0),
    I(0x13, 0, 30, 0, 0),
    U(0x37, 31, 0x10),
    NOP, NOP, NOP, NOP, NOP, NOP, // trampoline + setup = 16 instructions
  ];
  const loop = code.length;
  code.push(
    AMO(0x02, 2, 3, 20, 0), // lr.d.aq
    I(0x13, 0, 4, 3, 1),
    AMO(0x03, 1, 5, 20, 4), // sc.d.rl succeeds
    AMO(0x03, 3, 6, 20, 1), // no reservation: must fail/no store
    AMO(0x00, 3, 7, 20, 1), // amoadd.d.aqrl
    R(0x33, 4, 0, 30, 30, 7),
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(
    I(0x03, 3, 22, 20, 0),
    U(0x37, 24, 0x40008),
    I(0x13, 0, 25, 0, 1),
    S(3, 24, 25, 0),
    0x0000_006f,
  );
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

async function run(jit, tlbRefill = false) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.jit_set_tlb_fill(jit && tlbRefill ? 1 : 0);
  vm.onWrite = () => {};
  vm.bootLinux({ bios: program(), ramMB: 32 });
  let poweredOff = 0;
  for (let iteration = 0; iteration < 100 && !poweredOff; iteration++) {
    poweredOff = vm.runSystem(2_000_000n);
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    regs: Array.from({ length: 32 }, (_, index) => vm.ex.sys_reg(index)),
    memory: vm.ex.sys_ram_u64(0x8000_2000n),
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(3),
    tlbFills: vm.ex.jit_stat(31),
  };
}

const interpreter = await run(false);
const jit = await run(true, false);
const jitRefill = await run(true, true);
const found = [];
for (const [name, actual] of [
  ["no-refill", jit],
  ["refill", jitRefill],
]) {
  for (const field of ["poweredOff", "pc", "memory"]) {
    if (interpreter[field] !== actual[field]) found.push(`${name} ${field} differs`);
  }
  interpreter.regs.forEach((value, index) => {
    if (value !== actual.regs[index]) found.push(`${name} x${index} differs`);
  });
  if (actual.regs[5] !== 0n || actual.regs[6] !== 1n) {
    found.push(`${name} SC status contract failed`);
  }
  if (actual.jitInsns === 0n || actual.jitBlocks === 0n) {
    found.push(`${name} compiled system A path did not execute`);
  }
}
if (jit.tlbFills !== 0n) {
  found.push(`no-refill unexpectedly called the TLB helper ${jit.tlbFills} times`);
}
if (jitRefill.tlbFills === 0n) {
  found.push("refill path did not call the TLB helper");
}
if (found.length) {
  console.log(`FAIL system A differential: ${found.join(", ")}`);
  console.log("interpreter", interpreter);
  console.log("jit no-refill", jit);
  console.log("jit refill", jitRefill);
  process.exit(1);
}
console.log(
  `PASS system LR/SC + AMO with and without refill — ` +
    `jit-insns=${jit.jitInsns} fills=${jitRefill.tlbFills} ` +
    `memory=${jit.memory}`,
);
console.log("JIT SYSTEM A DIFFERENTIAL: ALL PASS");
