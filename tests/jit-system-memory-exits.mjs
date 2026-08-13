// Precise full-system slow-path differentials: cross-page accesses, MMIO, and
// a store to the physical page holding compiled code. Each loop is aligned so
// the cold 4096-instruction scheduler repeatedly samples its header and tiers
// it up deterministically.

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
const NOP = I(0x13, 0, 0, 0, 0);

function finish(code) {
  code.push(
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

function crossPageProgram() {
  const code = [
    U(0x17, 20, 0x2),
    I(0x13, 0, 20, 20, -3), // RAM_BASE + 0x1ffd
    I(0x13, 0, 1, 0, 0x135),
    S(3, 20, 1, 0),
    I(0x13, 0, 30, 0, 0),
    U(0x37, 31, 0x10),
    NOP, NOP, NOP, NOP, NOP, // trampoline + setup = 16 instructions
  ];
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
  code.push(I(0x03, 3, 22, 20, 0));
  return finish(code);
}

function mmioProgram() {
  const code = [
    U(0x37, 20, 0x2000), // CLINT base 0x02000000; msip reads as zero
    U(0x37, 31, 0x10),
    I(0x13, 0, 30, 0, 0),
    NOP, NOP, NOP, NOP, NOP, NOP, NOP, NOP,
  ];
  const loop = code.length;
  code.push(
    I(0x03, 3, 3, 20, 0),
    R(0x33, 0, 0, 30, 30, 3),
    I(0x13, 0, 30, 30, 1),
    NOP,
    NOP,
    NOP,
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  return finish(code);
}

function codePageStoreProgram() {
  const code = [
    U(0x17, 20, 0x2), // ordinary data at RAM_BASE + 0x2000
    U(0x17, 21, 0x0),
    I(0x13, 0, 21, 21, 0x7fc), // current code page + 0x800
    I(0x13, 0, 1, 0, 7),
    S(3, 20, 1, 0),
    U(0x37, 31, 0x10),
    U(0x37, 29, 0x7), // select code-page address once at x31 == 0x7000
    I(0x13, 0, 30, 0, 0),
    NOP, NOP, NOP,
  ];
  const loop = code.length;
  code.push(
    R(0x33, 4, 0, 2, 31, 29), // t = counter ^ target
    I(0x13, 3, 3, 2, 1), // condition = (t < 1)
    R(0x33, 0, 0x20, 4, 0, 3), // mask = -condition
    R(0x33, 4, 0, 5, 20, 21),
    R(0x33, 7, 0, 5, 5, 4),
    R(0x33, 4, 0, 6, 20, 5), // x6 = data or code-page address
    I(0x03, 3, 7, 20, 0),
    R(0x33, 0, 0, 7, 7, 1),
    S(3, 6, 7, 0),
    R(0x33, 4, 0, 30, 30, 7),
    NOP,
    NOP,
    NOP,
    NOP,
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(I(0x03, 3, 22, 20, 0), I(0x03, 3, 23, 21, 0));
  return finish(code);
}

async function run(bios, jit, memoryAddresses) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.jit_set_tlb_fill(1);
  vm.onWrite = () => {};
  vm.bootLinux({ bios, ramMB: 32 });
  let poweredOff = 0;
  for (let iteration = 0; iteration < 20 && !poweredOff; iteration++) {
    poweredOff = vm.runSystem(2_000_000n);
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    regs: Array.from({ length: 32 }, (_, index) => vm.ex.sys_reg(index)),
    memory: memoryAddresses.map((address) => vm.ex.sys_ram_u64(address)),
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(3),
    zeroRetire: vm.ex.jit_stat(15),
    tlbFills: vm.ex.jit_stat(31),
    dirtyEvents: vm.ex.jit_stat(23),
    dirtyDropped: vm.ex.jit_stat(24),
  };
}

function compare(reference, candidate) {
  const found = [];
  for (const field of ["poweredOff", "pc"]) {
    if (reference[field] !== candidate[field]) found.push(`${field} differs`);
  }
  reference.regs.forEach((value, index) => {
    if (value !== candidate.regs[index]) found.push(`x${index} differs`);
  });
  reference.memory.forEach((value, index) => {
    if (value !== candidate.memory[index]) found.push(`memory${index} differs`);
  });
  if (candidate.jitBlocks === 0n) found.push("no system block compiled");
  return found;
}

const cases = [
  ["cross-page load/store", crossPageProgram(), [0x8000_1ffdn], (state) =>
    state.zeroRetire > 0n],
  ["MMIO load", mmioProgram(), [], (state) =>
    state.zeroRetire > 0n && state.tlbFills > 0n],
  ["compiled-code-page store", codePageStoreProgram(), [0x8000_2000n, 0x8000_0800n],
    (state) => state.dirtyEvents > 0n && state.dirtyDropped > 0n],
];

let failures = 0;
for (const [name, bios, memoryAddresses, exercised] of cases) {
  const interpreter = await run(bios, false, memoryAddresses);
  const jit = await run(bios, true, memoryAddresses);
  const found = compare(interpreter, jit);
  if (!exercised(jit)) found.push("expected slow-path counters did not move");
  if (found.length) {
    failures++;
    console.log(`FAIL ${name}: ${found.join(", ")}`);
    console.log("interpreter", interpreter);
    console.log("jit", jit);
  } else {
    console.log(
      `PASS ${name} — jit-insns=${jit.jitInsns} zero=${jit.zeroRetire} ` +
        `fills=${jit.tlbFills} dirty=${jit.dirtyEvents}/${jit.dirtyDropped}`,
    );
  }
}

console.log(
  failures === 0
    ? "JIT SYSTEM MEMORY EXITS: ALL PASS"
    : `JIT SYSTEM MEMORY EXITS: ${failures} FAILURES`,
);
process.exit(failures === 0 ? 0 : 1);
