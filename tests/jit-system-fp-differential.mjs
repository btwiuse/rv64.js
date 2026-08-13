// Full-system FP differential for mstatus.FS checking/dirtying plus exact
// helper and translated-memory composition. The loop enters with FS=Clean;
// its first FLD must mark Dirty before continuing, matching the interpreter.

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
const CSR = (csr, f3, rd, rs1) =>
  0x73 | (rd << 7) | (f3 << 12) | (rs1 << 15) | (csr << 20);
const NOP = I(0x13, 0, 0, 0, 0);

function program() {
  const code = [
    U(0x17, 20, 0x2), // data = RAM_BASE + 0x2000
    U(0x37, 17, 0x4), // mstatus.FS = Clean
    CSR(0x300, 1, 0, 17),
    I(0x13, 0, 1, 0, 0x3ff),
    I(0x13, 1, 1, 1, 52), // raw f64 1.0
    0xf200_80d3, // fmv.d.x f1,x1
    I(0x13, 0, 2, 0, 1),
    CSR(0x001, 1, 0, 2), // set sticky NX, enabling proven native RNE subset
    I(0x13, 0, 4, 0, 0x3fe),
    I(0x13, 1, 4, 4, 52), // raw f64 0.5
    S(3, 20, 4, 0),
    U(0x37, 31, 0x10),
    I(0x13, 0, 30, 0, 0),
    CSR(0x300, 1, 0, 17), // restore FS=Clean immediately before hot loop
    NOP, NOP, NOP, NOP, // trampoline + setup = 24 instructions
  ];
  const loop = code.length;
  code.push(
    0x000a_3187, // fld f3,0(x20)
    0x0211_81d3, // fadd.d f3,f3,f1,rne
    0x003a_3027, // fsd f3,0(x20)
    0xe201_81d3, // fmv.x.d x3,f3
    R(0x33, 4, 0, 30, 30, 3),
    NOP,
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(
    0xe201_8b53, // fmv.x.d x22,f3
    CSR(0x003, 2, 23, 0), // csrr x23,fcsr
    CSR(0x300, 2, 24, 0), // csrr x24,mstatus (includes SD summary)
    U(0x37, 25, 0x40008),
    I(0x13, 0, 26, 0, 1),
    S(3, 25, 26, 0),
    0x0000_006f,
  );
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

function fsOffTrapProgram() {
  const code = [
    U(0x17, 10, 0),
    I(0x13, 0, 10, 10, 0x100), // trap handler at RAM_BASE + 0x100
    CSR(0x305, 1, 0, 10), // csrw mtvec,x10
    I(0x13, 0, 1, 0, 1),
    I(0x13, 0, 30, 0, 0),
    U(0x37, 31, 0x10),
    NOP, NOP, NOP, NOP, NOP, // trampoline + setup = 16 instructions
  ];
  const loop = code.length;
  code.push(
    0xf200_80d3, // fmv.d.x f1,x1: illegal while FS=Off
    I(0x13, 0, 30, 30, 1),
    NOP, NOP, NOP, NOP, NOP, NOP, NOP, NOP,
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(
    U(0x37, 25, 0x40008),
    I(0x13, 0, 26, 0, 1),
    S(3, 25, 26, 0),
    0x0000_006f,
  );
  while (code.length < 64) code.push(NOP);
  code.push(
    CSR(0x341, 2, 5, 0), // csrr x5,mepc
    I(0x13, 0, 5, 5, 4),
    CSR(0x341, 1, 0, 5), // csrw mepc,x5
    0x3020_0073, // mret
  );
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

async function run(jit, bios = program()) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.jit_set_tlb_fill(1);
  vm.onWrite = () => {};
  vm.bootLinux({ bios, ramMB: 32 });
  let poweredOff = 0;
  for (let iteration = 0; iteration < 100 && !poweredOff; iteration++) {
    poweredOff = vm.runSystem(2_000_000n);
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    regs: Array.from({ length: 32 }, (_, index) => vm.ex.sys_reg(index)),
    memory: vm.ex.sys_ram_u64(0x8000_2000n),
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(3),
    zeroRetire: vm.ex.jit_stat(15),
    tlbFills: vm.ex.jit_stat(31),
  };
}

const interpreter = await run(false);
const jit = await run(true);
const found = [];
for (const [name, actual] of [["ordinary", jit]]) {
  for (const field of ["poweredOff", "pc", "memory"]) {
    if (interpreter[field] !== actual[field]) found.push(`${name} ${field} differs`);
  }
  interpreter.regs.forEach((value, index) => {
    if (value !== actual.regs[index]) found.push(`${name} x${index} differs`);
  });
  if ((actual.regs[24] & 0x6000n) !== 0x6000n) found.push(`${name} mstatus.FS not Dirty`);
  if (actual.regs[23] !== 1n) found.push(`${name} fcsr sticky NX changed`);
  if (actual.jitInsns === 0n || actual.jitBlocks === 0n ||
      actual.tlbFills === 0n) {
    found.push(`${name} compiled system FP path did not execute`);
  }
}

if (found.length) {
  console.log(`FAIL system FP differential: ${found.join(", ")}`);
  console.log("interpreter", interpreter);
  console.log("jit", jit);
  process.exit(1);
}
console.log(
  `PASS system FP FS/helper/memory — jit-insns=${jit.jitInsns} ` +
    `fills=${jit.tlbFills} result=0x${jit.memory.toString(16)}`,
);
const offInterpreter = await run(false, fsOffTrapProgram());
const offJit = await run(true, fsOffTrapProgram());
const offFound = [];
for (const [name, actual] of [["ordinary", offJit]]) {
  for (const field of ["poweredOff", "pc", "memory"]) {
    if (offInterpreter[field] !== actual[field]) offFound.push(`${name} ${field} differs`);
  }
  offInterpreter.regs.forEach((value, index) => {
    if (value !== actual.regs[index]) offFound.push(`${name} x${index} differs`);
  });
  if (actual.regs[30] !== 65536n) offFound.push(`${name} trap handler failed`);
  if (actual.jitBlocks === 0n) offFound.push(`${name} FS-Off block did not compile`);
}
if (offJit.zeroRetire === 0n) offFound.push("ordinary FS-Off guard did not take a precise exit");
if (offFound.length) {
  console.log(`FAIL system FS-Off differential: ${offFound.join(", ")}`);
  console.log("interpreter", offInterpreter);
  console.log("jit", offJit);
  process.exit(1);
}
console.log(
  `PASS system FP FS-Off precise exits — blocks=${offJit.jitBlocks} ` +
    `jit-insns=${offJit.jitInsns}`,
);
console.log("JIT SYSTEM FP DIFFERENTIAL: ALL PASS");
