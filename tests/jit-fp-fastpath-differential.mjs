// Randomized differential for generated native-f64 and fused-FMA fast paths.
// Sticky NX makes RNE arithmetic eligible, while edge-biased operands force
// the generated predicates back through the exact helper for NaN, infinity,
// overflow, underflow, division-by-zero, and non-normal fused results.

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

function hashRegister(code, hash, register) {
  // Keep the loop inside the translator's bounded trace length. Distinct
  // result families plus final FP/register state make an accidental XOR
  // collision extraordinarily unlikely; the architectural fcsr is compared
  // independently.
  code.push(R(0x33, 4, 0, hash, hash, register));
}

function countRegister(code, count, register) {
  // Comparison results are 0/1. A sum records the number of true outcomes;
  // XOR would erase every even count and made the original end state weak.
  code.push(R(0x33, 0, 0, count, count, register));
}

function program(kind, iterations) {
  const code = [
    U(0x37, 20, 0x20),               // input = 0x20000
    U(0x37, 22, iterations >>> 12),   // exact multiple-of-4096 iteration count
    I(0x13, 0, 23, 0, 0),
    I(0x13, 0, 25, 0, 0),
    I(0x13, 0, 26, 0, 0),
    I(0x13, 0, 27, 0, 0),            // independent family hashes
    I(0x13, 0, 6, 0, 1),
    CSR(1, 1, 0, 6),                  // csrw fflags,1: sticky NX
  ];
  const loop = code.length;
  code.push(I(0x07, 3, 1, 20, 0), I(0x07, 3, 2, 20, 8));
  if (kind === "arithmetic") {
    code.push(
      R(0x53, 0, 0x01, 4, 1, 2),     // fadd.d
      R(0x53, 0, 0x05, 5, 1, 2),     // fsub.d
      R(0x53, 0, 0x09, 6, 1, 2),     // fmul.d
      R(0x53, 0, 0x0d, 7, 1, 2),     // fdiv.d
    );
    for (const [fp, hash] of [[4, 23], [5, 25], [6, 26], [7, 27]]) {
      code.push(R(0x53, 0, 0x71, 24, fp, 0)); // fmv.x.d x24,fp
      hashRegister(code, hash, 24);
    }
  } else if (kind === "comparison") {
    code.push(
      R(0x53, 2, 0x51, 8, 1, 2),     // feq.d
      R(0x53, 1, 0x51, 9, 1, 2),     // flt.d
      R(0x53, 0, 0x51, 10, 1, 2),    // fle.d
    );
    for (const [integer, hash] of [[8, 23], [9, 25], [10, 26]]) {
      countRegister(code, hash, integer);
    }
  } else if (kind === "fma") {
    code.push(
      I(0x07, 3, 3, 20, 16),
      R4(0x43, 0, 1, 11, 1, 2, 3),   // fmadd.d
      R(0x53, 0, 0x71, 24, 11, 0),   // fmv.x.d x24,f11
    );
    hashRegister(code, 23, 24);
  } else {
    throw new Error(`unknown FP fast-path corpus ${kind}`);
  }
  code.push(
    I(0x13, 0, 20, 20, 24),
    I(0x13, 0, 22, 22, -1),
  );
  // user_run() enters the interpreter in 512-instruction slices and counts
  // heat at each slice's entry PC. Keep the loop length a divisor of 512 so
  // the same header accumulates 64 entries and the test proves generated
  // execution instead of distributing heat around the loop.
  const lengthWithBranch = code.length - loop + 1;
  const paddedLength = lengthWithBranch <= 16 ? 16 : 32;
  while (code.length - loop + 1 < paddedLength) {
    code.push(I(0x13, 0, 0, 0, 0));
  }
  code.push(B(1, 22, 0, (loop - code.length) * 4));
  code.push(I(0x13, 0, 17, 0, 93), I(0x13, 0, 10, 0, 0), 0x0000_0073);
  return { code, preludeInstructions: loop, loopInstructions: paddedLength };
}

function makeOperands(count) {
  let state = 0x9e37_79b9_7f4a_7c15n;
  const next = () => {
    state ^= state << 13n;
    state ^= state >> 7n;
    state ^= state << 17n;
    return BigInt.asUintN(64, state);
  };
  const edges = [
    0n, 0x8000_0000_0000_0000n, 1n, 0x8000_0000_0000_0001n,
    0x0010_0000_0000_0000n, 0x7fef_ffff_ffff_ffffn,
    0xffef_ffff_ffff_ffffn, 0x7ff0_0000_0000_0000n,
    0xfff0_0000_0000_0000n, 0x7ff8_0000_0000_1234n,
    0x7ff0_0000_0000_0001n, 0x3ff0_0000_0000_0000n,
  ];
  const values = [];
  for (let i = 0; i < count * 3; i++) {
    if (i < edges.length * 8) {
      values.push(edges[(i * 7 + (i >> 3)) % edges.length]);
      continue;
    }
    let bits = next();
    if (i % 4 !== 0) {
      // Bias toward normal arithmetic so the inline arm receives broad data,
      // while every fourth sample keeps arbitrary edge encodings.
      const exponent = 0x100n + next() % 0x600n;
      bits = (bits & ~(0x7ffn << 52n)) | (exponent << 52n);
    }
    values.push(bits);
  }
  return values;
}

function synthElf(code, operands) {
  const codeAddress = 0x10000;
  const dataAddress = 0x20000;
  const segmentBytes = dataAddress - codeAddress + operands.length * 8;
  const elf = new Uint8Array(0x1000 + segmentBytes);
  const view = new DataView(elf.buffer);
  elf.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0], 0);
  view.setUint16(0x10, 2, true);
  view.setUint16(0x12, 243, true);
  view.setUint32(0x14, 1, true);
  view.setBigUint64(0x18, BigInt(codeAddress), true);
  view.setBigUint64(0x20, 64n, true);
  view.setUint16(0x34, 64, true);
  view.setUint16(0x36, 56, true);
  view.setUint16(0x38, 1, true);
  view.setUint32(64, 1, true);
  view.setUint32(68, 7, true);
  view.setBigUint64(72, 0x1000n, true);
  view.setBigUint64(80, BigInt(codeAddress), true);
  view.setBigUint64(88, BigInt(codeAddress), true);
  view.setBigUint64(96, BigInt(segmentBytes), true);
  view.setBigUint64(104, BigInt(segmentBytes), true);
  view.setBigUint64(112, 0x1000n, true);
  code.forEach((word, index) => view.setUint32(0x1000 + index * 4, word >>> 0, true));
  const dataOffset = 0x1000 + dataAddress - codeAddress;
  operands.forEach((value, index) => view.setBigUint64(dataOffset + index * 8, value, true));
  return elf;
}

function containsRelaxedFma(bytes) {
  for (let i = 0; i + 2 < bytes.length; i++) {
    if (bytes[i] === 0xfd && bytes[i + 1] === 0x87 && bytes[i + 2] === 0x02) return true;
  }
  return false;
}

async function run(name, elf, jit, schedule) {
  const vm = await RV64.create(wasm);
  let relaxedModules = 0;
  vm.onJitModule = (bytes) => { if (containsRelaxedFma(bytes)) relaxedModules++; };
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.onWrite = () => {};
  if (!vm.loadElf(elf, [`fp-fast-diff-${name}`], 16)) {
    throw new Error(`failed to load ${name} FP corpus`);
  }
  let stop;
  if (jit) {
    // Enter once per loop iteration. This is deliberately a scheduler-level
    // test: user-mode heat is counted at runUser entry, while ordinary runs
    // use 512-instruction slices. Exact loop-sized budgets make the loop
    // header hot deterministically and ensure every post-tier operand reaches
    // the generated block.
    stop = vm.runUser(BigInt(schedule.preludeInstructions));
    let calls = 0;
    while (stop === 0 && calls++ < iterations + 4) {
      stop = vm.runUser(BigInt(schedule.loopInstructions));
    }
  } else {
    stop = vm.runUser(100_000_000n);
  }
  return {
    stop,
    exit: vm.userExitCode(),
    pc: vm.ex.user_pc(),
    insns: vm.ex.user_insn_count(),
    fcsr: vm.ex.user_fcsr(),
    regs: Array.from({ length: 31 }, (_, index) => vm.ex.user_reg(index + 1)),
    fregs: Array.from({ length: 32 }, (_, index) => vm.ex.user_freg(index)),
    generated: vm.ex.jit_stat(0),
    blocks: vm.ex.jit_stat(2),
    relaxedModules,
    hardwareFmaSupported: vm.hardwareFmaSupported === true,
  };
}

const iterations = 4096;
const operands = makeOperands(iterations);
let failures = 0;
for (const name of ["arithmetic", "comparison", "fma"]) {
  const corpus = program(name, iterations);
  const elf = synthElf(corpus.code, operands);
  const interpreter = await run(name, elf, false, corpus);
  const jit = await run(name, elf, true, corpus);
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
  if (jit.generated < 10_000n || jit.blocks === 0n) {
    differences.push("generated path did not run");
  }
  if (name === "fma" && jit.hardwareFmaSupported && jit.relaxedModules === 0) {
    differences.push("fused relaxed-SIMD path was not emitted");
  }
  if (differences.length) {
    failures++;
    console.log(`FAIL ${name}: ${differences.slice(0, 12).join(", ")}`);
    console.log({ interpreter, jit });
  } else {
    console.log(
      `PASS ${name} — cases=${iterations} generated=${jit.generated} ` +
      `hash=0x${BigInt.asUintN(64, jit.regs[22]).toString(16)} ` +
      `relaxed-modules=${jit.relaxedModules}`,
    );
  }
}
console.log(
  failures === 0
    ? "JIT FP FAST-PATH DIFFERENTIAL: ALL PASS"
    : `JIT FP FAST-PATH DIFFERENTIAL: ${failures} FAILURES`,
);
process.exit(failures === 0 ? 0 : 1);
