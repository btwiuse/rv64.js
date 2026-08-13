// Modern riscv-virt differential for the whole-loop memcpy/memmove lowering.
// The synthetic S-mode image deliberately uses the exact eight-ld/eight-sd,
// 64-byte induction form emitted by the scorecard's RV64 compiler.  Both the
// forward non-overlap and backward overlap cases must match the interpreter,
// and the JIT run must prove that the Wasm-to-Wasm bulk helper actually ran.

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

const ADDI = (rd, rs1, imm) => I(0x13, 0, rd, rs1, imm);
const LD = (rd, rs1, imm) => I(0x03, 3, rd, rs1, imm);
const SD = (rs2, rs1, imm) => S(3, rs1, rs2, imm);
const add = (rd, rs1, rs2) => R(0x33, 0, 0, rd, rs1, rs2);
const xor = (rd, rs1, rs2) => R(0x33, 4, 0, rd, rs1, rs2);
const CSR = (csr, f3, rd, rs1) =>
  0x73 | (rd << 7) | (f3 << 12) | (rs1 << 15) | (csr << 20);

function li32(code, rd, value) {
  const low = (value << 20) >> 20;
  code.push(U(0x37, rd, (value - low) >> 12), ADDI(rd, rd, low));
}

function words(code) {
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

function program({ backward, sourceOffset, destinationOffset }) {
  const code = [
    U(0x17, 18, 0x100), // x18 = 0x80300000 source base
  ];
  if (sourceOffset) code.push(ADDI(18, 18, sourceOffset));
  if (backward) {
    code.push(ADDI(19, 18, destinationOffset));
  } else {
    const pcOffset = code.length * 4;
    code.push(
      U(0x17, 19, 0x200),
      ADDI(19, 19, destinationOffset - pcOffset),
    );
  }

  // Initialize 4096 source bytes with an observable sequence of u64 values.
  code.push(ADDI(20, 18, 0), ADDI(21, 0, 512), ADDI(22, 0, 1));
  const initialize = code.length;
  code.push(
    SD(22, 20, 0),
    ADDI(20, 20, 8),
    ADDI(22, 22, 1),
    ADDI(21, 21, -1),
  );
  code.push(B(1, 21, 0, (initialize - code.length) * 4));

  code.push(U(0x37, 31, 1), ADDI(11, 0, 63)); // 4096 outer copies
  const outer = code.length;
  code.push(ADDI(13, 18, 0), ADDI(14, 19, 0), U(0x37, 12, 1));
  if (backward) {
    code.push(U(0x37, 10, 1), add(13, 13, 10), add(14, 14, 10));
  }

  const copy = code.length;
  const offsets = backward
    ? [-8, -16, -24, -32, -40, -48, -56, -64]
    : [0, 8, 16, 24, 32, 40, 48, 56];
  for (const offset of offsets) code.push(LD(15, 13, offset), SD(15, 14, offset));
  code.push(
    ADDI(14, 14, backward ? -64 : 64),
    ADDI(12, 12, -64),
    ADDI(13, 13, backward ? -64 : 64),
  );
  code.push(B(6, 11, 12, (copy - code.length) * 4)); // bltu 63,count,copy
  code.push(ADDI(31, 31, -1));
  code.push(B(1, 31, 0, (outer - code.length) * 4));

  // Checksum and edge samples make corruption visible without host mutation.
  code.push(ADDI(23, 19, 0), ADDI(24, 0, 512), ADDI(26, 0, 0));
  const checksum = code.length;
  code.push(
    LD(25, 23, 0),
    xor(26, 26, 25),
    ADDI(23, 23, 8),
    ADDI(24, 24, -1),
  );
  code.push(B(1, 24, 0, (checksum - code.length) * 4));
  code.push(U(0x37, 8, 1), add(8, 19, 8));
  code.push(LD(27, 19, 0), LD(28, 8, -16), LD(9, 8, -8));

  // SiFive test device: low 16 bits 0x5555 power the virt machine off.
  code.push(U(0x37, 29, 0x100), U(0x37, 30, 5), ADDI(30, 30, 0x555));
  code.push(S(2, 29, 30, 0), 0x0000_006f);

  return words(code);
}

// S-mode program with identity-mapped code and deliberately non-contiguous
// physical pages behind two-page source and destination virtual ranges. Both
// ranges begin four bytes into a page, forcing the helper to replay an
// unaligned 64-bit word across each independent physical boundary.
function sv39Program() {
  const code = [
    U(0x17, 10, 0x4), // root = 0x80204000
    U(0x37, 9, 0x1),
    add(11, 10, 9), // l1 = 0x80205000
    add(12, 11, 9), // source l0 = 0x80206000
    add(13, 12, 9), // destination l0 = 0x80207000
  ];
  const ptes = [
    [10, 0, 0x2008_1401], // root[0] -> l1
    [10, 16, 0x2000_00cf], // root[2] -> identity 0x80000000 1-GiB leaf
    [11, 16, 0x2008_1801], // l1[2] -> source l0
    [11, 24, 0x2008_1c01], // l1[3] -> destination l0
    [12, 0, 0x200c_00c7], // source VA page 0 -> PA 0x80300000
    [12, 8, 0x2014_00c7], // source VA page 1 -> PA 0x80500000
    [13, 0, 0x2010_00c7], // destination VA page 0 -> PA 0x80400000
    [13, 8, 0x2018_00c7], // destination VA page 1 -> PA 0x80600000
  ];
  for (const [base, offset, pte] of ptes) {
    li32(code, 14, pte);
    code.push(SD(14, base, offset));
  }
  code.push(
    ADDI(15, 0, 8),
    I(0x13, 1, 15, 15, 60), // satp.MODE=Sv39
    I(0x13, 5, 16, 10, 12), // root PPN
    R(0x33, 6, 0, 15, 15, 16),
    CSR(0x180, 1, 0, 15),
    0x1200_0073, // sfence.vma x0,x0
    U(0x37, 18, 0x400),
    ADDI(18, 18, 4), // source VA = 0x00400004
    U(0x37, 19, 0x600),
    ADDI(19, 19, 4), // destination VA = 0x00600004
    ADDI(20, 18, 0),
    ADDI(21, 0, 512),
    ADDI(22, 0, 1),
  );
  const initialize = code.length;
  code.push(
    SD(22, 20, 0),
    ADDI(20, 20, 8),
    ADDI(22, 22, 1),
    ADDI(21, 21, -1),
    B(1, 21, 0, (initialize - (code.length + 4)) * 4),
    U(0x37, 31, 1),
    ADDI(11, 0, 63),
  );
  const outer = code.length;
  code.push(ADDI(13, 18, 0), ADDI(14, 19, 0), U(0x37, 12, 1));
  const copy = code.length;
  for (const offset of [0, 8, 16, 24, 32, 40, 48, 56]) {
    code.push(LD(15, 13, offset), SD(15, 14, offset));
  }
  code.push(
    ADDI(14, 14, 64),
    ADDI(12, 12, -64),
    ADDI(13, 13, 64),
    B(6, 11, 12, (copy - (code.length + 3)) * 4),
    ADDI(31, 31, -1),
    B(1, 31, 0, (outer - (code.length + 5)) * 4),
    ADDI(23, 19, 0),
    ADDI(24, 0, 512),
    ADDI(26, 0, 0),
  );
  const checksum = code.length;
  code.push(
    LD(25, 23, 0),
    xor(26, 26, 25),
    ADDI(23, 23, 8),
    ADDI(24, 24, -1),
    B(1, 24, 0, (checksum - (code.length + 4)) * 4),
  );
  li32(code, 17, 0x5352_5354); // SBI SRST extension
  code.push(ADDI(16, 0, 0), ADDI(10, 0, 0), ADDI(11, 0, 0), 0x0000_0073, 0x0000_006f);
  return words(code);
}

async function run(spec, jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.jit_set_page_threshold(4096);
  vm.ex.jit_set_tlb_fill(1);
  vm.onWrite = () => {};
  vm.bootVirtLinuxDirect({ kernel: program(spec), ramMB: 32 });
  let poweredOff = false;
  for (let slice = 0; slice < 50 && !poweredOff; slice++) {
    poweredOff = vm.runVirtSystem(2_000_000n);
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  const source = 0x8030_0000n + BigInt(spec.sourceOffset);
  const destination = spec.backward
    ? source + BigInt(spec.destinationOffset)
    : 0x8040_0000n + BigInt(spec.destinationOffset);
  return {
    poweredOff,
    pc: vm.ex.virt_pc(),
    insns: vm.ex.virt_insn_count(),
    regs: [9, 12, 13, 14, 15, 18, 19, 26, 27, 28, 31]
      .map((index) => [index, vm.ex.virt_reg(index)]),
    memory: [0n, 64n, 2048n, 4088n]
      .map((offset) => vm.ex.virt_ram_u64(destination + offset)),
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(3),
    copyChunks: vm.ex.jit_stat(8),
  };
}

async function runSv39(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.jit_set_page_threshold(4096);
  vm.ex.jit_set_tlb_fill(1);
  vm.onWrite = () => {};
  vm.bootVirtLinuxDirect({ kernel: sv39Program(), ramMB: 32 });
  let poweredOff = false;
  for (let slice = 0; slice < 50 && !poweredOff; slice++) {
    poweredOff = vm.runVirtSystem(2_000_000n);
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  return {
    poweredOff,
    pc: vm.ex.virt_pc(),
    insns: vm.ex.virt_insn_count(),
    regs: [12, 13, 14, 15, 18, 19, 26, 31]
      .map((index) => [index, vm.ex.virt_reg(index)]),
    memory: [
      vm.ex.virt_ram_u64(0x8040_0004n),
      vm.ex.virt_ram_u64(0x8040_0044n),
      vm.ex.virt_ram_u64(0x8040_0804n),
      vm.ex.virt_ram_u64(0x8040_0ff4n),
      vm.ex.virt_ram_u64(0x8060_0000n),
    ],
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(3),
    copyChunks: vm.ex.jit_stat(8),
  };
}

function compare(reference, candidate) {
  const errors = [];
  if (!reference.poweredOff || !candidate.poweredOff) errors.push("guest did not power off");
  if (candidate.pc !== reference.pc) errors.push("final PC differs");
  reference.regs.forEach(([reg, value], index) => {
    if (candidate.regs[index][0] !== reg || candidate.regs[index][1] !== value) {
      errors.push(`x${reg} differs`);
    }
  });
  reference.memory.forEach((value, index) => {
    if (candidate.memory[index] !== value) errors.push(`memory sample ${index} differs`);
  });
  if (candidate.jitBlocks === 0n || candidate.jitInsns === 0n) {
    errors.push("no generated system code executed");
  }
  if (candidate.copyChunks === 0n) errors.push("whole-loop copy helper did not execute");
  return errors;
}

const cases = [
  { name: "forward-aligned", backward: false, sourceOffset: 0, destinationOffset: 0 },
  { name: "forward-cross-page-word", backward: false, sourceOffset: 4, destinationOffset: 0 },
  { name: "backward-overlap-aligned", backward: true, sourceOffset: 0, destinationOffset: 64 },
  { name: "backward-cross-page-store", backward: true, sourceOffset: 0, destinationOffset: 68 },
];

for (const spec of cases) {
  const reference = await run(spec, false);
  const candidate = await run(spec, true);
  const errors = compare(reference, candidate);
  if (errors.length) {
    console.log("interpreter", reference);
    console.log("jit", candidate);
    throw new Error(`${spec.name} bulk copy: ${errors.join(", ")}`);
  }
  console.log(
    `PASS ${spec.name} bulk copy — ` +
      `chunks=${candidate.copyChunks} jit-insns=${candidate.jitInsns}`,
  );
}

{
  const reference = await runSv39(false);
  const candidate = await runSv39(true);
  const errors = compare(reference, candidate);
  if (errors.length) {
    console.log("interpreter", reference);
    console.log("jit", candidate);
    throw new Error(`Sv39 non-contiguous bulk copy: ${errors.join(", ")}`);
  }
  console.log(
    `PASS Sv39 non-contiguous cross-page copy — ` +
      `chunks=${candidate.copyChunks} jit-insns=${candidate.jitInsns}`,
  );
}
console.log("JIT SYSTEM BULK COPY DIFFERENTIAL: ALL PASS");
