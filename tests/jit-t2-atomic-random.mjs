#!/usr/bin/env node
// Deterministic randomized full-system atomic/T2 differential. Each case forms
// six hot indirect entries, mixes LR/SC with AMOs, takes one rare handled fault
// after tier-up, and finishes with additional misaligned/unmapped AMO faults.
// Interpreter and generated execution must match every register, RAM cell,
// retirement count, and terminal state.

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
const CSR = (f3, rd, rs1, csr) =>
  0x73 | (rd << 7) | (f3 << 12) | (rs1 << 15) | (csr << 20);
const AMO = (width, funct5, aqrl, rd, address, source) =>
  R(0x2f, width, (funct5 << 2) | aqrl, rd, address, source);
const MRET = 0x3020_0073;
const WFI = 0x1050_0073;
const AMO_OPS = [0x01, 0x00, 0x04, 0x0c, 0x08, 0x10, 0x14, 0x18, 0x1c];

function random(seed) {
  let state = seed >>> 0;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return state >>> 0;
  };
}

function program(seed) {
  const next = random(seed);
  const code = [];
  const labels = new Map();
  const addresses = [];
  const branches = [];
  const label = (name) => labels.set(name, code.length * 4);
  const addressOf = (register, name) => {
    const pc = code.length * 4;
    code.push(U(0x17, register, 0), 0);
    addresses.push({ index: code.length - 1, register, name, pc });
  };
  const branchTo = (f3, rs1, rs2, name) => {
    const pc = code.length * 4;
    code.push(0);
    branches.push({ index: code.length - 1, f3, rs1, rs2, name, pc });
  };

  const width = next() & 1 ? 3 : 2;
  const address = width === 3 ? 20 : 21;
  const otherAddress = width === 3 ? 21 : 20;
  const storeWidth = width;
  const interfere = (next() & 1) !== 0;
  // At least 262k cycles are required to defeat the cold interpreter's 4096-
  // instruction sampling aliases for both the 25- and 26-instruction loop
  // shapes. Shorter cases can finish before every rotating entry reaches the
  // production tier-up threshold, which tests luck rather than T2 semantics.
  const loops = next() & 1 ? 393_216 : 262_144;
  const loopUpper = loops >> 12;

  code.push(
    U(0x17, 20, 0x10), // RAM_BASE + 64 KiB: aligned D cell
    I(0x13, 0, 21, 20, 8), // aligned W cell
    I(0x13, 0, 22, 20, 1), // misaligned address
    U(0x37, 23, 0x90000), // unmapped sign-extended physical address
    I(0x13, 0, 24, 20, -4), // D access crosses a page and is misaligned
    I(0x13, 0, 4, 0, 1 + (next() & 31)),
    S(3, 20, 0, 0),
    S(2, 21, 0, 0),
    U(0x37, 31, loopUpper),
    I(0x13, 0, 2, 0, 0), // SC status accumulator
    I(0x13, 0, 3, 0, 0), // AMO result accumulator
    I(0x13, 0, 27, 0, 0), // trap count
    I(0x13, 0, 28, 0, 0), // mcause checksum
    I(0x13, 0, 29, 0, 17), // take rare fault with 17 iterations left
  );
  addressOf(5, "trap");
  code.push(CSR(1, 0, 5, 0x305)); // csrw mtvec,x5
  for (const [register, name] of [
    [11, "b1"], [12, "b2"], [13, "b3"],
    [14, "b4"], [15, "b5"], [16, "b0"],
  ]) addressOf(register, name);
  code.push(I(0x67, 0, 0, 16, 0));

  label("b0");
  code.push(
    AMO(width, 0x02, next() & 3, 6, address, 0), // LR.W/D
    I(0x13, 0, 7, 6, 1 + (next() & 7)),
  );
  if (interfere) code.push(S(storeWidth, otherAddress, 4, 0));
  code.push(
    AMO(width, 0x03, next() & 3, 8, address, 7), // first SC
    AMO(width, 0x03, next() & 3, 9, address, 4), // second SC must fail
    R(0x33, 0, 0, 2, 2, 8),
    R(0x33, 0, 0, 2, 2, 9),
    AMO(width, AMO_OPS[next() % AMO_OPS.length], next() & 3, 10, address, 4),
    R(0x33, 4, 0, 3, 3, 10),
  );
  branchTo(1, 31, 29, "b0_after_fault");
  code.push(AMO(3, 0x00, next() & 3, 10, 22, 4)); // rare misaligned AMO.D
  label("b0_after_fault");
  code.push(I(0x67, 0, 0, 11, 0));

  for (let member = 1; member <= 4; member++) {
    label(`b${member}`);
    const memberWidth = next() & 1 ? 3 : 2;
    const memberAddress = memberWidth === 3 ? 20 : 21;
    code.push(
      AMO(
        memberWidth,
        AMO_OPS[next() % AMO_OPS.length],
        next() & 3,
        6 + member,
        memberAddress,
        4,
      ),
      R(0x33, 4, 0, 3, 3, 6 + member),
      I(0x67, 0, 0, 11 + member, 0),
    );
  }

  label("b5");
  code.push(I(0x13, 0, 31, 31, -1));
  branchTo(0, 31, 0, "exit_faults");
  code.push(I(0x67, 0, 0, 16, 0));

  label("exit_faults");
  code.push(
    AMO(3, 0x00, next() & 3, 10, 24, 4), // crossing/misaligned
    AMO(3, 0x00, next() & 3, 10, 23, 4), // unmapped AMO
    AMO(3, 0x02, next() & 3, 10, 23, 0), // unmapped LR
  );
  branchTo(0, 0, 0, "exit");

  label("trap");
  code.push(
    CSR(2, 6, 0, 0x342), // csrr x6,mcause
    I(0x13, 0, 27, 27, 1),
    R(0x33, 4, 0, 28, 28, 6),
    CSR(2, 7, 0, 0x341), // csrr x7,mepc
    I(0x13, 0, 7, 7, 4),
    CSR(1, 0, 7, 0x341), // csrw mepc,x7
    MRET,
  );

  label("exit");
  code.push(
    U(0x37, 24, 0x40008),
    I(0x13, 0, 25, 0, 1),
    S(3, 24, 25, 0),
    // Stop the CPU immediately after the power-off MMIO write. An infinite
    // JAL here lets a coarse interpreter slice retire arbitrary post-shutdown
    // padding while the warm JIT fallback stops at a different boundary.
    WFI,
  );

  for (const { index, register, name, pc } of addresses) {
    const delta = labels.get(name) - pc;
    if (delta < -2048 || delta > 2047) throw new Error(`${name} address is too far`);
    code[index] = I(0x13, 0, register, register, delta);
  }
  for (const { index, f3, rs1, rs2, name, pc } of branches) {
    const delta = labels.get(name) - pc;
    if (delta < -4096 || delta > 4094) throw new Error(`${name} branch is too far`);
    code[index] = B(f3, rs1, rs2, delta);
  }
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return { bytes, width, interfere, loops };
}

const yieldToCompiler = () => new Promise((done) => setImmediate(done));

async function runCase(caseInfo, config) {
  const vm = await RV64.create(wasm);
  let reservationT2Modules = 0;
  vm.onJitModule = (bytes, metadata) => {
    if (metadata.kind !== "batch" && metadata.kind !== "async-region") return;
    const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
    if (imports.some((entry) => entry.name === "system_reservation")) {
      reservationT2Modules++;
    }
  };
  vm.ex.jit_set_enabled(config.jit ? 1 : 0);
  vm.ex.jit_set_tlb_fill(config.jit && config.refill ? 1 : 0);
  vm.ex.jit_set_batch(config.jit && config.t2 === "batch" ? 1 : 0);
  vm.ex.sys_set_superblock(config.jit && config.t2 === "region" ? 1 : 0);
  vm.ex.jit_set_region_lazy_state(
    config.jit && config.t2 === "region" && config.lazy ? 1 : 0,
  );
  if (config.jit && config.t2 === "region") vm.ex.jit_set_sb_spacing(0);
  if (config.jit) vm.ex.jit_set_ic_trigger(64);
  vm.onWrite = () => {};
  vm.bootLinux({ bios: caseInfo.bytes, ramMB: 32 });
  let poweredOff = 0;
  let maxSlice = 0n;
  const sliceBudget = BigInt(process.env.RV64_ATOMIC_SLICE ?? 100_000);
  for (let slice = 0; slice < 1200 && !poweredOff; slice++) {
    const before = vm.ex.sys_insn_count();
    poweredOff = vm.runSystem(sliceBudget);
    const retired = vm.ex.sys_insn_count() - before;
    if (retired > maxSlice) maxSlice = retired;
    if (process.env.RV64_ATOMIC_TRACE) {
      console.log(
        `${config.jit ? "jit" : "int"} slice=${slice} retired=${retired} ` +
          `total=${vm.ex.sys_insn_count()} pc=0x${vm.ex.sys_pc().toString(16)} ` +
          `remaining=${vm.ex.sys_reg(31)} generated=${vm.ex.jit_stat(0)}`,
      );
    }
    if (config.jit) await yieldToCompiler();
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    insns: vm.ex.sys_insn_count(),
    regs: Array.from({ length: 32 }, (_, index) => vm.ex.sys_reg(index)),
    data0: vm.ex.sys_ram_u64(0x8001_0000n),
    data1: vm.ex.sys_ram_u64(0x8001_0008n),
    jitInsns: vm.ex.jit_stat(0),
    dispatches: vm.ex.jit_stat(1),
    tlbFills: vm.ex.jit_stat(31),
    batches: vm.ex.jit_stat(43),
    landed: vm.ex.jit_stat(13),
    reservationT2Modules,
    maxSlice,
  };
}

function compare(expected, actual) {
  const differences = [];
  for (const field of ["poweredOff", "pc", "insns", "data0", "data1"]) {
    if (expected[field] !== actual[field]) differences.push(`${field} differs`);
  }
  expected.regs.forEach((value, index) => {
    if (value !== actual.regs[index]) differences.push(`x${index} differs`);
  });
  return differences;
}

const caseCount = Number(process.env.RV64_ATOMIC_CASES ?? 12);
const seeds = Array.from({ length: caseCount }, (_, index) =>
  0x6d2b_79f5 ^ (index * 0x9e37_79b1));
let failures = 0;
let totalJit = 0n;
let totalT2 = 0;
for (let index = 0; index < seeds.length; index++) {
  const seed = seeds[index] >>> 0;
  const caseInfo = program(seed);
  const t2 = index & 1 ? "region" : "batch";
  const refill = (index & 2) !== 0;
  const lazy = t2 === "region";
  const interpreter = await runCase(caseInfo, { jit: false, refill: false, t2, lazy });
  const compiled = await runCase(caseInfo, { jit: true, refill, t2, lazy });
  const differences = compare(interpreter, compiled);
  if (!compiled.poweredOff) differences.push("did not power off");
  if (compiled.regs[31] !== 0n) differences.push(`remaining=${compiled.regs[31]}`);
  if (compiled.regs[27] < 2n) differences.push(`handled only ${compiled.regs[27]} faults`);
  if (compiled.jitInsns === 0n) differences.push("no generated execution");
  // Batch members deliberately include the LR/SC entry and therefore prove
  // the typed reservation ABI. A page region is free to select only the AMO
  // leaders; requiring its opportunistic leader set to contain LR/SC makes
  // this a sampling lottery rather than a correctness condition.
  if (t2 === "batch" && compiled.reservationT2Modules === 0) {
    differences.push("no reservation-bearing T2 module");
  }
  const sliceLimit = BigInt(process.env.RV64_ATOMIC_SLICE ?? 100_000) + 100n;
  if (compiled.maxSlice > sliceLimit) differences.push(`slice overshoot=${compiled.maxSlice}`);
  if (!refill && compiled.tlbFills !== 0n) {
    differences.push(`no-refill called helper ${compiled.tlbFills} times`);
  }
  if (refill && compiled.tlbFills === 0n) differences.push("refill helper was unused");
  if (t2 === "batch" && compiled.batches === 0n) differences.push("no batch formed");
  if (t2 === "region" && compiled.landed === 0n) differences.push("no region landed");
  if (differences.length) {
    failures++;
    console.log(
      `FAIL seed=0x${seed.toString(16)} t2=${t2} refill=${refill}: ` +
        differences.slice(0, 12).join(", "),
    );
    console.log("interpreter", interpreter);
    console.log("compiled", compiled);
  } else {
    totalJit += compiled.jitInsns;
    totalT2 += compiled.reservationT2Modules;
    console.log(
      `PASS seed=0x${seed.toString(16)} width=${caseInfo.width === 3 ? "D" : "W"} ` +
      `interfere=${caseInfo.interfere} t2=${t2} lazy=${lazy} refill=${refill} ` +
        `faults=${compiled.regs[27]} jit=${compiled.jitInsns}`,
    );
  }
}

if (failures) {
  console.log(`JIT T2 ATOMIC RANDOM: ${failures}/${seeds.length} FAILURES`);
  process.exit(1);
}
console.log(
  `JIT T2 ATOMIC RANDOM: ALL ${seeds.length} PASS — ` +
    `jit-insns=${totalJit} reservation-t2-modules=${totalT2}`,
);
