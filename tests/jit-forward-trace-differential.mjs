// Differential for guarded forward branches plus followed forward JAL edges.
// The hot loop is a diamond with memory effects; both sides execute thousands
// of times, forcing precise trace exits and re-entry with loop-carried state.

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasmBytes = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

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
  const value = off & 0x1fffff;
  return 0x6f | (rd << 7) | (((value >> 12) & 0xff) << 12) |
    (((value >> 11) & 1) << 20) | (((value >> 1) & 0x3ff) << 21) |
    (((value >> 20) & 1) << 31);
};

function program() {
  const code = [
    U(0x37, 20, 0x20), // data cursor
    U(0x37, 31, 16), // 65536 iterations
    I(0x13, 0, 1, 0, 0),
  ];
  const loop = code.length;
  code.push(
    I(0x13, 7, 3, 31, 3), // x3 = counter & 3
    B(0, 3, 0, 12), // every fourth iteration takes the side exit
    I(0x13, 0, 1, 1, 7),
    J(0, 8), // follow direct edge over else body
    I(0x13, 0, 1, 1, -3),
    S(3, 20, 1, 0),
    I(0x03, 3, 4, 20, 0),
    I(0x13, 0, 20, 20, 8),
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(I(0x13, 0, 17, 0, 93), I(0x13, 0, 10, 0, 0), 0x00000073);
  return code;
}

function synthElf(code) {
  const bytes = new Uint8Array(code.length * 4);
  const words = new DataView(bytes.buffer);
  code.forEach((word, index) => words.setUint32(index * 4, word >>> 0, true));
  const elf = new Uint8Array(0x1000 + bytes.length);
  const header = new DataView(elf.buffer);
  elf.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0], 0);
  header.setUint16(0x10, 2, true);
  header.setUint16(0x12, 243, true);
  header.setUint32(0x14, 1, true);
  header.setBigUint64(0x18, 0x10000n, true);
  header.setBigUint64(0x20, 64n, true);
  header.setUint16(0x34, 64, true);
  header.setUint16(0x36, 56, true);
  header.setUint16(0x38, 1, true);
  header.setUint32(64, 1, true);
  header.setUint32(68, 5, true);
  header.setBigUint64(72, 0x1000n, true);
  header.setBigUint64(80, 0x10000n, true);
  header.setBigUint64(88, 0x10000n, true);
  header.setBigUint64(96, BigInt(bytes.length), true);
  header.setBigUint64(104, BigInt(bytes.length), true);
  header.setBigUint64(112, 0x1000n, true);
  elf.set(bytes, 0x1000);
  return elf;
}

async function run(jit) {
  const vm = await RV64.create(wasmBytes);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.onWrite = () => {};
  if (!vm.loadElf(synthElf(program()), ["trace-diff"], 16)) throw new Error("load failed");
  const stop = vm.runUser(100_000_000n);
  return {
    stop,
    exit: vm.userExitCode(),
    pc: vm.ex.user_pc(),
    insns: vm.ex.user_insn_count(),
    regs: Array.from({ length: 31 }, (_, index) => vm.ex.user_reg(index + 1)),
    jitInsns: vm.ex.jit_stat(0),
    dispatches: vm.ex.jit_stat(1),
  };
}

const interpreter = await run(false);
const jit = await run(true);
const differences = [];
for (const field of ["stop", "exit", "pc", "insns"]) {
  if (interpreter[field] !== jit[field]) differences.push(`${field} differs`);
}
interpreter.regs.forEach((value, index) => {
  if (value !== jit.regs[index]) differences.push(`x${index + 1} differs`);
});
if (jit.jitInsns === 0n) differences.push("no compiled instructions executed");
if (differences.length) {
  console.log(`JIT FORWARD TRACE DIFFERENTIAL: FAIL — ${differences.slice(0, 8).join(", ")}`);
  process.exit(1);
}
console.log(
  `JIT FORWARD TRACE DIFFERENTIAL: ALL PASS — jit-insns=${jit.jitInsns} dispatches=${jit.dispatches}`,
);
