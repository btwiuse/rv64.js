// A hot vector load walks to the end of the 16 MiB user address space and
// faults after completing 15 of 16 elements. The JIT helper must return at the
// unretired instruction, and interpreter replay must retain vstart=15 without
// repeating earlier elements or perturbing scalar retirement.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64, Stop } = await import(join(root, "web", "rv64.js"));
const wasm = await readFile(
  join(root, "target", "wasm32-unknown-unknown", "release", "rv64_wasm.wasm"),
);

const I = (opcode, funct3, rd, rs1, immediate) =>
  (opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) |
    ((immediate & 0xfff) << 20)) >>> 0;
const U = (opcode, rd, immediate20) =>
  (opcode | (rd << 7) | ((immediate20 & 0xfffff) << 12)) >>> 0;
const B = (funct3, rs1, rs2, offset) => {
  const immediate = offset & 0x1fff;
  return (0x63 | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) |
    (((immediate >>> 11) & 1) << 7) | (((immediate >>> 1) & 0xf) << 8) |
    (((immediate >>> 5) & 0x3f) << 25) | (((immediate >>> 12) & 1) << 31)) >>> 0;
};

function program() {
  const code = [
    0xcc08_7057,       // vsetivli zero,16,e8,m1,ta,ma
    U(0x37, 20, 0xff0), // x20 = 16 MiB - 64 KiB
    U(0x37, 31, 0x10),  // x31 = 65536
  ];
  const loop = code.length;
  code.push(
    0x020a_0407,             // vle8.v v8,(x20)
    I(0x13, 0, 20, 20, 1),  // x20++
    I(0x13, 0, 31, 31, -1), // x31--
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  return { code, faultPc: 0x10000n + BigInt(loop * 4) };
}

function synthElf(words) {
  const code = new Uint8Array(words.length * 4);
  const codeView = new DataView(code.buffer);
  words.forEach((word, index) => codeView.setUint32(index * 4, word, true));
  const elf = new Uint8Array(0x1000 + code.length);
  const view = new DataView(elf.buffer);
  elf.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0], 0);
  view.setUint16(0x10, 2, true);
  view.setUint16(0x12, 243, true);
  view.setUint32(0x14, 1, true);
  view.setBigUint64(0x18, 0x10000n, true);
  view.setBigUint64(0x20, 64n, true);
  view.setUint32(0x30, 5, true);
  view.setUint16(0x34, 64, true);
  view.setUint16(0x36, 56, true);
  view.setUint16(0x38, 1, true);
  view.setUint32(64, 1, true);
  view.setUint32(68, 5, true);
  view.setBigUint64(72, 0x1000n, true);
  view.setBigUint64(80, 0x10000n, true);
  view.setBigUint64(88, 0x10000n, true);
  view.setBigUint64(96, BigInt(code.length), true);
  view.setBigUint64(104, BigInt(code.length), true);
  view.setBigUint64(112, 0x1000n, true);
  elf.set(code, 0x1000);
  return elf;
}

async function run(elf, jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  let vectorModules = 0;
  vm.onWrite = () => {};
  vm.onJitModule = (bytes) => {
    if (Buffer.from(bytes).includes(Buffer.from("user_vector"))) vectorModules++;
  };
  assert.ok(vm.loadElf(elf, ["rvv-fault"], 16));
  const stop = vm.runUser(10_000_000n);
  return {
    stop,
    pc: vm.ex.user_pc(),
    insns: vm.ex.user_insn_count(),
    regs: Array.from({ length: 31 }, (_, index) => vm.ex.user_reg(index + 1)),
    vl: vm.ex.user_vl(),
    vtype: vm.ex.user_vtype(),
    vstart: vm.ex.user_vstart(),
    vcsr: vm.ex.user_vcsr(),
    vregs: Uint8Array.from(
      { length: 32 * 16 },
      (_, index) => vm.ex.user_vreg_byte(index >>> 4, index & 15),
    ),
    jitInsns: vm.ex.jit_stat(0),
    vectorModules,
  };
}

const { code, faultPc } = program();
const elf = synthElf(code);
const interpreter = await run(elf, false);
const jit = await run(elf, true);
assert.equal(interpreter.stop, Stop.TRAP);
assert.equal(jit.stop, Stop.TRAP);
assert.equal(interpreter.pc, faultPc);
assert.equal(jit.pc, faultPc);
assert.equal(interpreter.vstart, 15n);
assert.equal(jit.vstart, 15n);
for (const field of ["pc", "insns", "vl", "vtype", "vstart", "vcsr"]) {
  assert.equal(jit[field], interpreter[field], `${field} differs`);
}
assert.deepEqual(jit.regs, interpreter.regs);
assert.deepEqual(jit.vregs, interpreter.vregs);
assert.ok(jit.jitInsns > 1000n, `insufficient generated execution: ${jit.jitInsns}`);
assert.ok(jit.vectorModules > 0, "no generated module imported user_vector");
console.log(
  `RVV JIT FAULT DIFFERENTIAL: PASS pc=0x${jit.pc.toString(16)} ` +
  `vstart=${jit.vstart} jit-insns=${jit.jitInsns} vector-modules=${jit.vectorModules}`,
);
