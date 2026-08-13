// Differential coverage for the clean-room flat-memory JIT lowering.
//
// Each synthesized RV64 ELF is executed from a fresh VM with the JIT disabled
// and enabled. Full integer state, PC, stop reason, exit code, and retirement
// count must agree. The second case deliberately reaches the end of guest
// memory after tier-up, exercising the compiled precise side exit before the
// interpreter reports the architectural load fault.

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

function appendExit(code) {
  code.push(I(0x13, 0, 17, 0, 93)); // a7 = SYS_exit
  code.push(I(0x13, 0, 10, 0, 0));
  code.push(0x00000073);
  return code;
}

function memoryRoundTripProgram() {
  const code = [
    U(0x37, 20, 0x20), // x20 = 0x20000 data cursor
    U(0x37, 31, 16), // x31 = 65536 iterations
    I(0x13, 0, 1, 0, 1),
  ];
  const loop = code.length;
  code.push(
    S(3, 20, 1, 0), // sd
    I(0x03, 3, 2, 20, 0), // ld
    S(2, 20, 2, 8), // sw
    I(0x03, 6, 3, 20, 8), // lwu
    S(1, 20, 3, 12), // sh
    I(0x03, 5, 4, 20, 12), // lhu
    S(0, 20, 4, 14), // sb
    I(0x03, 4, 5, 20, 14), // lbu
    I(0x03, 0, 6, 20, 14), // lb (sign extension)
    I(0x03, 1, 7, 20, 12), // lh (sign extension)
    I(0x03, 2, 8, 20, 8), // lw (sign extension)
    I(0x13, 0, 1, 1, 13),
    I(0x13, 0, 20, 20, 16),
    I(0x13, 0, 31, 31, -1),
    I(0x13, 0, 0, 0, 0), // keep loop length aligned with 512-insn slices
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  return appendExit(code);
}

function boundsFaultAfterTierUpProgram() {
  const code = [
    U(0x37, 20, 0xfc0), // x20 = 16 MiB - 256 KiB
    U(0x37, 31, 16), // more iterations than fit before the boundary
  ];
  const loop = code.length;
  code.push(
    I(0x03, 3, 5, 20, 0), // ld x5,0(x20)
    I(0x13, 0, 20, 20, 8),
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  return appendExit(code);
}

function synthElf(code) {
  const codeBytes = new Uint8Array(code.length * 4);
  const words = new DataView(codeBytes.buffer);
  code.forEach((word, index) => words.setUint32(index * 4, word >>> 0, true));
  const elf = new Uint8Array(0x1000 + codeBytes.length);
  const header = new DataView(elf.buffer);
  elf.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0], 0);
  header.setUint16(0x10, 2, true); // ET_EXEC
  header.setUint16(0x12, 243, true); // EM_RISCV
  header.setUint32(0x14, 1, true);
  header.setBigUint64(0x18, 0x10000n, true);
  header.setBigUint64(0x20, 64n, true);
  header.setUint16(0x34, 64, true);
  header.setUint16(0x36, 56, true);
  header.setUint16(0x38, 1, true);
  header.setUint32(64, 1, true); // PT_LOAD
  header.setUint32(68, 5, true); // R+X
  header.setBigUint64(72, 0x1000n, true);
  header.setBigUint64(80, 0x10000n, true);
  header.setBigUint64(88, 0x10000n, true);
  header.setBigUint64(96, BigInt(codeBytes.length), true);
  header.setBigUint64(104, BigInt(codeBytes.length), true);
  header.setBigUint64(112, 0x1000n, true);
  elf.set(codeBytes, 0x1000);
  return elf;
}

async function run(code, jit) {
  const vm = await RV64.create(wasmBytes);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.onWrite = () => {};
  if (!vm.loadElf(synthElf(code), ["memory-diff"], 16)) {
    throw new Error("synthetic ELF failed to load");
  }
  const stop = vm.runUser(100_000_000n);
  return {
    stop,
    exit: vm.userExitCode(),
    pc: vm.ex.user_pc(),
    insns: vm.ex.user_insn_count(),
    regs: Array.from({ length: 31 }, (_, index) => vm.ex.user_reg(index + 1)),
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(2) + vm.ex.jit_stat(3),
  };
}

function architecturalDiff(interpreter, jit) {
  const differences = [];
  for (const field of ["stop", "exit", "pc", "insns"]) {
    if (interpreter[field] !== jit[field]) {
      differences.push(`${field}: ${interpreter[field]} != ${jit[field]}`);
    }
  }
  interpreter.regs.forEach((value, index) => {
    if (value !== jit.regs[index]) {
      differences.push(`x${index + 1}: ${value} != ${jit.regs[index]}`);
    }
  });
  return differences;
}

let failures = 0;
for (const [name, program] of [
  ["all integer load/store widths", memoryRoundTripProgram()],
  ["bounds fault after tier-up", boundsFaultAfterTierUpProgram()],
]) {
  const interpreter = await run(program, false);
  const jit = await run(program, true);
  const differences = architecturalDiff(interpreter, jit);
  if (jit.jitBlocks === 0n || jit.jitInsns === 0n) {
    differences.push("test did not execute compiled code");
  }
  if (differences.length) {
    failures++;
    console.log(`FAIL ${name}: ${differences.slice(0, 8).join("; ")}`);
  } else {
    console.log(
      `PASS ${name} — jit-insns=${jit.jitInsns} blocks=${jit.jitBlocks}`,
    );
  }
}

console.log(failures === 0 ? "JIT MEMORY DIFFERENTIAL: ALL PASS" : `${failures} FAILURES`);
process.exit(failures === 0 ? 0 : 1);
