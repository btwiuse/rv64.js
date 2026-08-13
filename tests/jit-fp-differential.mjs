// Exact scalar F/D differential. Every helper-backed arithmetic, sqrt,
// min/max, comparison, classification, conversion, and fused multiply-add
// family executes in a hot loop under all five legal rounding modes. Operand
// sets cover normal values, quiet/signaling NaNs and infinities, plus signed
// zero/subnormal/max-finite boundaries.

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
const R4 = (op, f3, fmt, rd, rs1, rs2, rs3) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | (rs2 << 20) |
  (fmt << 25) | (rs3 << 27);
const I = (op, f3, rd, rs1, imm) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | ((imm & 0xfff) << 20);
const U = (op, rd, imm20) => op | (rd << 7) | ((imm20 & 0xfffff) << 12);
const B = (f3, rs1, rs2, off) => {
  const value = off & 0x1fff;
  return 0x63 | (f3 << 12) | (rs1 << 15) | (rs2 << 20) |
    (((value >> 11) & 1) << 7) | (((value >> 1) & 0xf) << 8) |
    (((value >> 5) & 0x3f) << 25) | (((value >> 12) & 1) << 31);
};
const CSR = (csr, f3, rd, rs1) =>
  0x73 | (rd << 7) | (f3 << 12) | (rs1 << 15) | (csr << 20);

const boxed32 = (bits) => 0xffff_ffff_0000_0000n | BigInt(bits);
const operandSets = [
  [
    0x3ff8_0000_0000_0000n,
    0xc002_0000_0000_0000n,
    0x4008_0000_0000_0000n,
    boxed32(0x3fc0_0000),
    boxed32(0xc010_0000),
    boxed32(0x4040_0000),
  ],
  [
    0x7ff0_0000_0000_0001n,
    0x7ff8_0000_0000_1234n,
    0x7ff0_0000_0000_0000n,
    boxed32(0x7f80_0001),
    boxed32(0x7fc0_1234),
    boxed32(0x7f80_0000),
  ],
  [
    0x0000_0000_0000_0001n,
    0x7fef_ffff_ffff_ffffn,
    0x8000_0000_0000_0000n,
    boxed32(0x0000_0001),
    boxed32(0x7f7f_ffff),
    boxed32(0x8000_0000),
  ],
];

function prelude(roundingMode) {
  return [
    U(0x37, 20, 0x20), // x20 = fixed data address 0x20000
    I(0x07, 3, 0, 20, 0),
    I(0x07, 3, 1, 20, 8),
    I(0x07, 3, 2, 20, 16),
    I(0x07, 3, 3, 20, 24),
    I(0x07, 3, 4, 20, 32),
    I(0x07, 3, 5, 20, 40),
    I(0x13, 0, 6, 0, roundingMode),
    CSR(2, 1, 0, 6), // csrw frm,x6
    I(0x13, 0, 7, 0, -1),
    U(0x37, 31, 16), // 65536 iterations; enough for every loop PC to tier
  ];
}

function finishLoop(code, loop) {
  code.push(I(0x13, 0, 31, 31, -1));
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(I(0x13, 0, 17, 0, 93), I(0x13, 0, 10, 0, 0), 0x0000_0073);
  return code;
}

function arithmeticProgram(roundingMode) {
  const code = prelude(roundingMode);
  const loop = code.length;
  code.push(CSR(1, 1, 0, 0)); // clear sticky fflags each iteration
  for (const [f7, rd] of [[0x00, 6], [0x04, 7], [0x08, 8], [0x0c, 9]]) {
    code.push(R(0x53, 7, f7, rd, 3, 4));
  }
  code.push(
    R(0x53, 7, 0x2c, 10, 3, 0),
    R(0x53, 0, 0x14, 11, 3, 4),
    R(0x53, 1, 0x14, 12, 3, 4),
  );
  for (const [f7, rd] of [[0x01, 13], [0x05, 14], [0x09, 15], [0x0d, 16]]) {
    code.push(R(0x53, 7, f7, rd, 0, 1));
  }
  code.push(
    R(0x53, 7, 0x2d, 17, 0, 0),
    R(0x53, 0, 0x15, 18, 0, 1),
    R(0x53, 1, 0x15, 19, 0, 1),
    R(0x53, 2, 0x50, 8, 3, 4),
    R(0x53, 1, 0x50, 9, 3, 4),
    R(0x53, 0, 0x50, 10, 3, 4),
    R(0x53, 2, 0x51, 11, 0, 1),
    R(0x53, 1, 0x51, 12, 0, 1),
    R(0x53, 0, 0x51, 13, 0, 1),
    R(0x53, 1, 0x70, 14, 3, 0),
    R(0x53, 1, 0x71, 15, 0, 0),
  );
  for (let family = 0; family < 4; family++) {
    code.push(R4([0x43, 0x47, 0x4b, 0x4f][family], 7, 0, 20 + family, 3, 4, 5));
    code.push(R4([0x43, 0x47, 0x4b, 0x4f][family], 7, 1, 24 + family, 0, 1, 2));
  }
  return finishLoop(code, loop);
}

function conversionProgram(roundingMode) {
  const code = prelude(roundingMode);
  const loop = code.length;
  code.push(
    CSR(1, 1, 0, 0),
    R(0x53, 7, 0x20, 6, 0, 1), // fcvt.s.d
    R(0x53, 7, 0x21, 7, 3, 0), // fcvt.d.s (exact)
  );
  for (let kind = 0; kind < 4; kind++) {
    code.push(R(0x53, 7, 0x60, 8 + kind, 3, kind));
    code.push(R(0x53, 7, 0x61, 12 + kind, 0, kind));
    code.push(R(0x53, 7, 0x68, 8 + kind, 7, kind));
    code.push(R(0x53, 7, 0x69, 12 + kind, 7, kind));
  }
  return finishLoop(code, loop);
}

function synthElf(code, operands) {
  const dataAddress = 0x20000;
  const segmentBytes = dataAddress - 0x10000 + operands.length * 8;
  const elf = new Uint8Array(0x1000 + segmentBytes);
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
  header.setUint32(68, 7, true); // RWX test segment
  header.setBigUint64(72, 0x1000n, true);
  header.setBigUint64(80, 0x10000n, true);
  header.setBigUint64(88, 0x10000n, true);
  header.setBigUint64(96, BigInt(segmentBytes), true);
  header.setBigUint64(104, BigInt(segmentBytes), true);
  header.setBigUint64(112, 0x1000n, true);
  const words = new DataView(elf.buffer, 0x1000);
  code.forEach((word, index) => words.setUint32(index * 4, word >>> 0, true));
  operands.forEach((value, index) =>
    header.setBigUint64(0x1000 + dataAddress - 0x10000 + index * 8, value, true));
  return elf;
}

async function run(program, operands, jit) {
  const vm = await RV64.create(wasmBytes);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.onWrite = () => {};
  if (!vm.loadElf(synthElf(program, operands), ["fp-diff"], 16)) {
    throw new Error("synthetic FP ELF failed to load");
  }
  const stop = vm.runUser(100_000_000n);
  return {
    stop,
    exit: vm.userExitCode(),
    pc: vm.ex.user_pc(),
    insns: vm.ex.user_insn_count(),
    fcsr: vm.ex.user_fcsr(),
    regs: Array.from({ length: 31 }, (_, index) => vm.ex.user_reg(index + 1)),
    fregs: Array.from({ length: 32 }, (_, index) => vm.ex.user_freg(index)),
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(2),
  };
}

function compare(interpreter, jit) {
  const differences = [];
  for (const field of ["stop", "exit", "pc", "insns", "fcsr"]) {
    if (interpreter[field] !== jit[field]) differences.push(`${field} differs`);
  }
  interpreter.regs.forEach((value, index) => {
    if (value !== jit.regs[index]) differences.push(`x${index + 1} differs`);
  });
  interpreter.fregs.forEach((value, index) => {
    if (value !== jit.fregs[index]) differences.push(`f${index} differs`);
  });
  if (jit.jitInsns < 10_000n || jit.jitBlocks === 0n) {
    differences.push("insufficient compiled execution");
  }
  return differences;
}

let failures = 0;
for (let rm = 0; rm <= 4; rm++) {
  const operands = operandSets[rm % operandSets.length];
  for (const [name, makeProgram] of [
    ["arithmetic", arithmeticProgram],
    ["conversion", conversionProgram],
  ]) {
    const program = makeProgram(rm);
    const interpreter = await run(program, operands, false);
    const jit = await run(program, operands, true);
    const differences = compare(interpreter, jit);
    if (differences.length !== 0) {
      failures++;
      console.log(`FAIL ${name} rm=${rm}: ${differences.slice(0, 8).join(", ")}`);
    } else {
      console.log(`PASS ${name} rm=${rm} — jit-insns=${jit.jitInsns}`);
    }
  }
}

console.log(
  failures === 0
    ? "JIT FP DIFFERENTIAL: ALL PASS"
    : `JIT FP DIFFERENTIAL: ${failures} FAILURES`,
);
process.exit(failures === 0 ? 0 : 1);
