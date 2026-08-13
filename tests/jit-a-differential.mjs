// Flat-user RV64A differential for every AMO read/modify/write operation and
// LR/SC in both W and D widths. The LR/SC case also verifies that a second SC
// fails, clears the reservation, and leaves memory untouched.

import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasmBytes = await readFile(
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
const AMO = (width, funct5, aqrl, rd, address, source) =>
  R(0x2f, width, (funct5 << 2) | aqrl, rd, address, source);
const funct5s = [0x01, 0x00, 0x04, 0x0c, 0x08, 0x10, 0x14, 0x18, 0x1c];

function appendExit(code) {
  code.push(I(0x13, 0, 17, 0, 93), I(0x13, 0, 10, 0, 0), 0x0000_0073);
  return code;
}

function allAmoProgram() {
  const code = [
    U(0x37, 20, 0x20), // x20 = 0x20000, D cell
    I(0x13, 0, 21, 20, 8), // x21 = W cell
    I(0x13, 0, 1, 0, 0x175),
    I(0x13, 0, 2, 0, -1),
    S(3, 20, 2, 0),
    S(2, 21, 1, 0),
    I(0x13, 0, 30, 0, 0),
    U(0x37, 31, 16),
  ];
  const loop = code.length;
  funct5s.forEach((funct5, index) => {
    const destination = 3 + index;
    code.push(
      AMO(3, funct5, index & 3, destination, 20, 1),
      R(0x33, 0, 0, 30, 30, destination),
    );
  });
  funct5s.forEach((funct5, index) => {
    // Reuse x3..x11 only after their D results have been accumulated; keep
    // address registers x20/x21 immutable across iterations.
    const destination = 3 + index;
    code.push(
      AMO(2, funct5, 3 - (index & 3), destination, 21, 1),
      R(0x33, 0, 0, 30, 30, destination),
    );
  });
  code.push(
    R(0x33, 4, 0, 1, 1, 30),
    I(0x13, 0, 1, 1, 0x101),
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(I(0x03, 3, 22, 20, 0), I(0x03, 2, 23, 21, 0));
  return appendExit(code);
}

function faultingAmoProgram() {
  const code = [
    U(0x37, 20, 0xfc0), // 16 MiB - 256 KiB
    I(0x13, 0, 1, 0, 7),
    U(0x37, 31, 16),
  ];
  const loop = code.length;
  code.push(
    AMO(3, 0x00, 3, 5, 20, 1),
    I(0x13, 0, 20, 20, 8),
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  return appendExit(code);
}

function lrScProgram() {
  const code = [
    U(0x37, 20, 0x20), // x20 = 0x20000, D cell
    I(0x13, 0, 21, 20, 8), // x21 = 0x20008, W cell
    I(0x13, 0, 1, 0, 1),
    S(3, 20, 0, 0),
    S(2, 21, 0, 0),
    I(0x13, 0, 30, 0, 0), // accumulate SC status codes
    U(0x37, 31, 16), // 65536 iterations
  ];
  const loop = code.length;
  code.push(
    AMO(3, 0x02, 2, 3, 20, 0), // lr.d.aq x3,(x20)
    I(0x13, 0, 4, 3, 1),
    AMO(3, 0x03, 1, 5, 20, 4), // sc.d.rl succeeds: x5 = 0
    AMO(3, 0x03, 3, 6, 20, 1), // second sc.d fails: x6 = 1
    AMO(2, 0x02, 1, 7, 21, 0), // lr.w.rl x7,(x21)
    I(0x13, 0, 8, 7, 3),
    AMO(2, 0x03, 2, 9, 21, 8), // sc.w.aq succeeds: x9 = 0
    AMO(2, 0x03, 0, 10, 21, 1), // second sc.w fails: x10 = 1
    R(0x33, 0, 0, 30, 30, 5),
    R(0x33, 0, 0, 30, 30, 6),
    R(0x33, 0, 0, 30, 30, 9),
    R(0x33, 0, 0, 30, 30, 10),
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(I(0x03, 3, 22, 20, 0), I(0x03, 2, 23, 21, 0));
  return appendExit(code);
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
  header.setUint32(68, 7, true);
  header.setBigUint64(72, 0x1000n, true);
  header.setBigUint64(80, 0x10000n, true);
  header.setBigUint64(88, 0x10000n, true);
  header.setBigUint64(96, BigInt(bytes.length), true);
  header.setBigUint64(104, BigInt(bytes.length), true);
  header.setBigUint64(112, 0x1000n, true);
  elf.set(bytes, 0x1000);
  return elf;
}

async function run(code, jit) {
  const vm = await RV64.create(wasmBytes);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.onWrite = () => {};
  if (!vm.loadElf(synthElf(code), ["a-diff"], 16)) throw new Error("load failed");
  const stop = vm.runUser(100_000_000n);
  return {
    stop,
    exit: vm.userExitCode(),
    pc: vm.ex.user_pc(),
    insns: vm.ex.user_insn_count(),
    regs: Array.from({ length: 31 }, (_, index) => vm.ex.user_reg(index + 1)),
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(2),
  };
}

function compare(interpreter, jit) {
  const differences = [];
  for (const field of ["stop", "exit", "pc", "insns"]) {
    if (interpreter[field] !== jit[field]) differences.push(`${field} differs`);
  }
  interpreter.regs.forEach((value, index) => {
    if (value !== jit.regs[index]) differences.push(`x${index + 1} differs`);
  });
  if (jit.jitInsns === 0n || jit.jitBlocks === 0n) {
    differences.push(`no compiled execution (insns=${jit.jitInsns}, blocks=${jit.jitBlocks})`);
  }
  return differences;
}

let failures = 0;
for (const [name, code] of [
  ["all AMO operations and widths", allAmoProgram()],
  ["LR/SC success, failure, clearing, and both widths", lrScProgram()],
  ["AMO bounds fault after tier-up", faultingAmoProgram()],
]) {
  const interpreter = await run(code, false);
  const jit = await run(code, true);
  const differences = compare(interpreter, jit);
  if (differences.length !== 0) {
    failures++;
    console.log(`FAIL ${name}: ${differences.slice(0, 8).join(", ")}`);
  } else {
    console.log(`PASS ${name} — jit-insns=${jit.jitInsns} blocks=${jit.jitBlocks}`);
  }
}

console.log(
  failures === 0 ? "JIT A DIFFERENTIAL: ALL PASS" : `JIT A DIFFERENTIAL: ${failures} FAILURES`,
);
process.exit(failures === 0 ? 0 : 1);
