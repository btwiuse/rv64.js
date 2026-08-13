// Sv39 + MPRV full-system JIT differential. Bare-metal setup creates a
// three-level page table with initially clear A/D bits, redirects M-mode data
// accesses through S-mode translation, then runs a hot load/store loop through
// the virtual mapping. This exercises page walking, permission-class-specific
// rows, A/D publication, and generated refill/re-probe behavior.

import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(
  process.env.RV64_WASM ??
    join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);
const interpreterFusedMemory =
  process.env.RV64_INTERPRETER_FUSED_MEMORY === "1";

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
const CSR = (csr, f3, rd, rs1) =>
  0x73 | (rd << 7) | (f3 << 12) | (rs1 << 15) | (csr << 20);
const NOP = I(0x13, 0, 0, 0, 0);

function program() {
  const code = [
    U(0x17, 10, 0x4), // root = RAM_BASE + 0x4000
    U(0x37, 9, 0x1),
    R(0x33, 0, 0, 11, 10, 9), // level 1 = +0x5000
    R(0x33, 0, 0, 12, 11, 9), // level 0 = +0x6000
    U(0x37, 8, 0x2),
    R(0x33, 0, 0x20, 13, 10, 8), // physical data = +0x2000
    U(0x37, 20, 0x400), // virtual data = 0x00400000

    I(0x13, 5, 14, 11, 2), // root[0] -> level 1
    I(0x13, 6, 14, 14, 1),
    S(3, 10, 14, 0),
    I(0x13, 5, 14, 12, 2), // level1[2] -> level 0
    I(0x13, 6, 14, 14, 1),
    S(3, 11, 14, 16),
    I(0x13, 5, 14, 13, 2), // level0[0] -> data, V|R|W; A/D clear
    I(0x13, 6, 14, 14, 7),
    S(3, 12, 14, 0),

    I(0x13, 0, 1, 0, 0x135),
    S(3, 13, 1, 0),
    I(0x13, 0, 15, 0, 8),
    I(0x13, 1, 15, 15, 60), // satp.MODE = Sv39
    I(0x13, 5, 16, 10, 12),
    R(0x33, 6, 0, 15, 15, 16),
    CSR(0x180, 1, 0, 15), // csrw satp,x15
    U(0x37, 17, 0x21),
    I(0x13, 0, 17, 17, -2048), // MPRV | MPP=S
    CSR(0x300, 1, 0, 17), // csrw mstatus,x17
    U(0x37, 31, 0x10),
  ];
  // Five trampoline instructions + 27 setup instructions = 32, exactly
  // divisible by the eight-instruction loop and the cold 4096-insn slice.
  const loop = code.length;
  code.push(
    I(0x03, 3, 3, 20, 0),
    I(0x13, 0, 3, 3, 1),
    S(3, 20, 3, 0),
    R(0x33, 4, 0, 30, 30, 3),
    NOP,
    NOP,
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(
    I(0x03, 3, 22, 20, 0),
    CSR(0x300, 1, 0, 0), // clear MPRV before physical/HTIF accesses
    I(0x03, 3, 23, 12, 0), // final leaf PTE including hardware A/D
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

async function run(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_interpreter_fused_memory(interpreterFusedMemory ? 1 : 0);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.jit_set_tlb_fill(1);
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
    data: vm.ex.sys_ram_u64(0x8000_2000n),
    leaf: vm.ex.sys_ram_u64(0x8000_6000n),
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(3),
    tlbFills: vm.ex.jit_stat(31),
  };
}

const interpreter = await run(false);
const jit = await run(true);
const found = [];
for (const [name, actual] of [["ordinary", jit]]) {
  for (const field of ["poweredOff", "pc", "data", "leaf"]) {
    if (interpreter[field] !== actual[field]) found.push(`${name} ${field} differs`);
  }
  interpreter.regs.forEach((value, index) => {
    if (value !== actual.regs[index]) found.push(`${name} x${index} differs`);
  });
  if ((actual.leaf & 0xc0n) !== 0xc0n) found.push(`${name} leaf A/D bits missing`);
  if (actual.jitBlocks === 0n || actual.jitInsns === 0n || actual.tlbFills === 0n) {
    found.push(`${name} Sv39 compiled refill path did not execute`);
  }
}
if (found.length) {
  console.log(`FAIL Sv39 differential: ${found.join(", ")}`);
  console.log("interpreter", interpreter);
  console.log("jit", jit);
  process.exit(1);
}
console.log(
  `PASS Sv39 MPRV mapping — jit-insns=${jit.jitInsns} fills=${jit.tlbFills} ` +
    `leaf=0x${jit.leaf.toString(16)}`,
);
console.log("JIT SYSTEM SV39 DIFFERENTIAL: ALL PASS");
