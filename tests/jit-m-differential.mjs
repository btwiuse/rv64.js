// RV64M differential: every 64-bit and RV64 word multiply/divide/remainder
// form executes in a hot single-latch loop. The initial iteration includes
// signed overflow and zero-divisor cases; later iterations vary the operands.

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasmBytes = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

const R = (op, f3, f7, rd, rs1, rs2) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | (rs2 << 20) | (f7 << 25);
const I = (op, f3, rd, rs1, imm) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | ((imm & 0xfff) << 20);
const U = (op, rd, imm20) => op | (rd << 7) | ((imm20 & 0xfffff) << 12);
const B = (f3, rs1, rs2, off) => {
  const value = off & 0x1fff;
  return 0x63 | (f3 << 12) | (rs1 << 15) | (rs2 << 20) |
    (((value >> 11) & 1) << 7) | (((value >> 1) & 0xf) << 8) |
    (((value >> 5) & 0x3f) << 25) | (((value >> 12) & 1) << 31);
};

function program() {
  const code = [
    I(0x13, 0, 1, 0, -1), // x1 = -1
    I(0x13, 0, 2, 0, 1),
    I(0x13, 1, 2, 2, 63), // x2 = i64::MIN
    U(0x37, 3, 0x80000), // low word of x3 = i32::MIN
    I(0x13, 0, 4, 0, 0), // x4 = zero divisor
    U(0x37, 31, 16), // 65536 iterations
  ];
  const loop = code.length;
  code.push(
    R(0x33, 0, 1, 5, 2, 1), // mul
    R(0x33, 1, 1, 6, 2, 1), // mulh
    R(0x33, 2, 1, 7, 2, 1), // mulhsu
    R(0x33, 3, 1, 8, 2, 1), // mulhu
    R(0x33, 4, 1, 9, 2, 1), // div (MIN/-1 first)
    R(0x33, 5, 1, 10, 2, 4), // divu (zero first)
    R(0x33, 6, 1, 11, 2, 1), // rem (MIN/-1 first)
    R(0x33, 7, 1, 12, 2, 4), // remu (zero first)
    R(0x3b, 0, 1, 13, 3, 1), // mulw
    R(0x3b, 4, 1, 14, 3, 1), // divw (MIN/-1 first)
    R(0x3b, 5, 1, 15, 3, 4), // divuw (zero first)
    R(0x3b, 6, 1, 16, 3, 1), // remw
    R(0x3b, 7, 1, 17, 3, 4), // remuw
    R(0x33, 4, 0, 2, 2, 5), // vary dividend: xor x2,x2,x5
    I(0x13, 0, 1, 1, 1), // vary signed divisor, including zero
    I(0x13, 7, 4, 31, 15), // x4 = counter & 15 (periodic zero)
    I(0x13, 0, 3, 3, 0x101),
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
  if (!vm.loadElf(synthElf(program()), ["m-diff"], 16)) throw new Error("load failed");
  const stop = vm.runUser(100_000_000n);
  return {
    stop,
    exit: vm.userExitCode(),
    pc: vm.ex.user_pc(),
    insns: vm.ex.user_insn_count(),
    regs: Array.from({ length: 31 }, (_, index) => vm.ex.user_reg(index + 1)),
    jitInsns: vm.ex.jit_stat(0),
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
  console.log(`JIT M DIFFERENTIAL: FAIL — ${differences.slice(0, 8).join(", ")}`);
  process.exit(1);
}
console.log(`JIT M DIFFERENTIAL: ALL PASS — jit-insns=${jit.jitInsns}`);
