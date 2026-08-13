// Full-system RVV differential. It checks mstatus.VS dirtying, scalar state
// published immediately before a vector store, scalar results written by a
// vector instruction and consumed by following JIT code, plus vector memory.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web", "rv64.js"));
const wasm = await readFile(
  join(root, "target", "wasm32-unknown-unknown", "release", "rv64_wasm.wasm"),
);

const R = (opcode, funct3, funct7, rd, rs1, rs2) =>
  (opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) |
    (rs2 << 20) | (funct7 << 25)) >>> 0;
const I = (opcode, funct3, rd, rs1, immediate) =>
  (opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) |
    ((immediate & 0xfff) << 20)) >>> 0;
const S = (funct3, rs1, rs2, immediate) => {
  const value = immediate & 0xfff;
  return (0x23 | ((value & 0x1f) << 7) | (funct3 << 12) | (rs1 << 15) |
    (rs2 << 20) | ((value >>> 5) << 25)) >>> 0;
};
const U = (opcode, rd, immediate20) =>
  (opcode | (rd << 7) | ((immediate20 & 0xfffff) << 12)) >>> 0;
const B = (funct3, rs1, rs2, offset) => {
  const value = offset & 0x1fff;
  return (0x63 | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) |
    (((value >>> 11) & 1) << 7) | (((value >>> 1) & 0xf) << 8) |
    (((value >>> 5) & 0x3f) << 25) | (((value >>> 12) & 1) << 31)) >>> 0;
};
const CSR = (csr, funct3, rd, rs1) =>
  (0x73 | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (csr << 20)) >>> 0;
const V = (funct6, vm, vs2, source, format, destination) =>
  (0x57 | (destination << 7) | (format << 12) | (source << 15) |
    (vs2 << 20) | ((vm ? 1 : 0) << 25) | (funct6 << 26)) >>> 0;
const VMEM = (load, base, reg, stride = 0) =>
  ((load ? 0x07 : 0x27) | (reg << 7) | (base << 15) | (stride << 20) |
    (1 << 25) | ((stride === 0 ? 0 : 2) << 26)) >>> 0;

function program() {
  const code = [
    U(0x17, 20, 0x2),       // x20 = RAM_BASE + 0x2000
    I(0x13, 0, 4, 0, 0x35), // scalar vector seed
    I(0x13, 0, 17, 0, 0x400), // mstatus.VS = Clean
    CSR(0x300, 1, 0, 17),
    0xcc08_7057,             // vsetivli zero,16,e8,m1,ta,ma
    V(0x17, true, 0, 4, 4, 8), // vmv.v.x v8,x4
    CSR(0x300, 1, 0, 17),    // restore Clean after vector setup dirtied VS
    U(0x37, 31, 0x40),      // 262144 iterations
    I(0x13, 0, 30, 0, 0),
    I(0x13, 0, 28, 0, 1),   // byte stride for generated strided memory
  ];
  const loop = code.length;
  code.push(
    V(0x00, true, 8, 1, 3, 8), // vadd.vi v8,v8,1
    I(0x13, 0, 21, 20, 0),     // scalar value exists only at store boundary
    VMEM(false, 21, 8, 28),     // vsse8.v v8,(x21),x28
    I(0x13, 0, 23, 21, 0),     // direct path must retain the just-published x21
    VMEM(true, 20, 9, 28),      // vlse8.v v9,(x20),x28
    V(0x10, true, 9, 0, 2, 22), // vmv.x.s x22,v9
    R(0x33, 4, 0, 30, 30, 22), // xor x30,x30,x22 after helper result
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(
    CSR(0x300, 2, 24, 0), // capture dirty mstatus
    U(0x37, 25, 0x40008),
    I(0x13, 0, 26, 0, 1),
    S(3, 25, 26, 0),      // HTIF power off
    0x0000_006f,
  );
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word, true));
  return bytes;
}

function fractionalMemoryProgram() {
  const code = [
    U(0x17, 20, 0x2),       // x20 = RAM_BASE + 0x2000
    I(0x13, 0, 4, 0, 0x35), // scalar vector seed
    I(0x13, 0, 17, 0, 0x400), // mstatus.VS = Clean
    CSR(0x300, 1, 0, 17),
    0xcc78_7057,             // vsetivli zero,16,e8,mf2,ta,ma (vl=8)
    V(0x17, true, 0, 4, 4, 8), // vmv.v.x v8,x4
    CSR(0x300, 1, 0, 17),    // restore Clean after setup
    U(0x37, 31, 0x40),       // 262144 iterations cross the system tier gate
    I(0x13, 0, 28, 0, 1),   // byte stride
  ];
  const loop = code.length;
  code.push(
    VMEM(false, 20, 8, 28),  // vsse8.v v8,(x20),x28 at fractional LMUL
    VMEM(true, 20, 9, 28),   // vlse8.v v9,(x20),x28 at fractional LMUL
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(
    CSR(0x300, 2, 24, 0),
    U(0x37, 25, 0x40008),
    I(0x13, 0, 26, 0, 1),
    S(3, 25, 26, 0),
    0x0000_006f,
  );
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word, true));
  return bytes;
}

async function run(jit, bios = program()) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_vector_simd_profile(1);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  let vectorModules = 0;
  vm.onWrite = () => {};
  vm.onJitModule = (bytes) => {
    if (Buffer.from(bytes).includes(Buffer.from("system_vector"))) vectorModules++;
  };
  vm.bootLinux({ bios, ramMB: 32 });
  let poweredOff = 0;
  for (let iteration = 0; iteration < 100 && !poweredOff; iteration++) {
    poweredOff = vm.runSystem(2_000_000n);
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    regs: Array.from({ length: 32 }, (_, index) => vm.ex.sys_reg(index)),
    memory: [
      vm.ex.sys_ram_u64(0x8000_2000n),
      vm.ex.sys_ram_u64(0x8000_2008n),
    ],
    jitInsns: vm.ex.jit_stat(0),
    simdInsns: vm.ex.jit_stat(152),
    jitBlocks: vm.ex.jit_stat(3),
    vectorModules,
  };
}

const interpreter = await run(false);
const jit = await run(true);
assert.ok(interpreter.poweredOff);
assert.ok(jit.poweredOff);
assert.equal(jit.pc, interpreter.pc);
assert.deepEqual(jit.regs, interpreter.regs);
assert.deepEqual(jit.memory, interpreter.memory);
assert.equal(jit.regs[22], 0x35n);
assert.equal(jit.regs[23], 0x8000_2000n);
assert.equal(jit.regs[30], 0n);
assert.equal(jit.regs[24] & 0x600n, 0x600n, "mstatus.VS is not Dirty");
assert.ok(jit.jitInsns > 1000n, `insufficient generated execution: ${jit.jitInsns}`);
assert.ok(jit.simdInsns > 1000n, `direct SIMD path was not hot: ${jit.simdInsns}`);
assert.ok(jit.jitBlocks > 0n, "no system region installed");
assert.ok(jit.vectorModules > 0, "no generated module imported system_vector");
console.log(
  `RVV JIT SYSTEM DIFFERENTIAL: PASS jit-insns=${jit.jitInsns} ` +
  `simd-insns=${jit.simdInsns} blocks=${jit.jitBlocks} vector-modules=${jit.vectorModules}`,
);

// This loop has no hot direct-vector candidate other than its two strided
// memory operations. It catches accidental reuse of the LMUL temporary by the
// system TLB guard: helper fallback remains correct, but would leave the SIMD
// count at zero and fail the coverage assertion.
const fractionalBios = fractionalMemoryProgram();
const fractionalInterpreter = await run(false, fractionalBios);
const fractionalJit = await run(true, fractionalBios);
assert.ok(fractionalInterpreter.poweredOff);
assert.ok(fractionalJit.poweredOff);
assert.equal(fractionalJit.pc, fractionalInterpreter.pc);
assert.deepEqual(fractionalJit.regs, fractionalInterpreter.regs);
assert.deepEqual(fractionalJit.memory, fractionalInterpreter.memory);
assert.equal(fractionalJit.regs[24] & 0x600n, 0x600n, "fractional path did not dirty VS");
assert.ok(
  fractionalJit.simdInsns > 1000n,
  `fractional system memory did not lower directly: ${fractionalJit.simdInsns}`,
);
console.log(
  `RVV JIT SYSTEM FRACTIONAL MEMORY: PASS jit-insns=${fractionalJit.jitInsns} ` +
  `simd-insns=${fractionalJit.simdInsns} vector-modules=${fractionalJit.vectorModules}`,
);
