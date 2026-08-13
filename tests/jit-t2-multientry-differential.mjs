// T2 multi-entry execution differential. It covers a landed six-state region,
// observed batches, monomorphic misses, same/cross-page two-way indirect
// targets, cumulative fuel, and invalidation when a target page self-modifies.
// All public entries share one register-resident in-module PC dispatcher; the
// test also executes the small standalone sbtest used by the raw Wasm ABI.

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
const J = (rd, off) => {
  const value = off & 0x1fffff;
  return 0x6f | (rd << 7) | (((value >> 12) & 0xff) << 12) |
    (((value >> 11) & 1) << 20) | (((value >> 1) & 0x3ff) << 21) |
    (((value >> 20) & 1) << 31);
};
const AMO = (funct5, aqrl, rd, address, source) =>
  R(0x2f, 3, (funct5 << 2) | aqrl, rd, address, source);
const WFI = 0x1050_0073;

function program() {
  const code = [
    U(0x37, 31, 0x20), // 131072 cycles
    I(0x13, 0, 1, 0, 0),
    U(0x17, 10, 0), // x10 = pc(0x08) + 0x18 = block 0
    I(0x13, 0, 10, 10, 0x18),
    I(0x67, 0, 0, 10, 0),
    I(0x13, 0, 0, 0, 0),
    I(0x13, 0, 0, 0, 0),
    I(0x13, 0, 0, 0, 0),
  ];

  // Five fixed-size indirect blocks. AUIPC is at block+4 and the next block
  // is at block+0x10, hence the +0x0c target displacement.
  for (let increment = 1; increment <= 5; increment++) {
    code.push(
      I(0x13, 0, 1, 1, increment),
      U(0x17, 10, 0),
      I(0x13, 0, 10, 10, 0x0c),
      I(0x67, 0, 0, 10, 0),
    );
  }

  // Sixth block returns to block 0 through a stable indirect edge until the
  // counter expires, then stops at WFI. Unlike HTIF power-off, WFI ends the
  // current slice at one exact architectural point, so every backend must
  // report the same retired-instruction count rather than differing by a
  // slice-sized amount of terminal-loop padding.
  code.push(
    I(0x13, 0, 1, 1, 6),
    I(0x13, 0, 31, 31, -1),
    B(0, 31, 0, 16), // beq x31,x0,exit
    U(0x17, 10, 0), // pc=0x7c; block 0 is 0x20
    I(0x13, 0, 10, 10, -0x5c),
    I(0x67, 0, 0, 10, 0),
    WFI,
  );

  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

function changingTargetProgram() {
  const code = [
    U(0x17, 20, 0x10), // data slot = RAM_BASE + 64 KiB
    U(0x17, 10, 0), // address of A: pc 0x04 + 0x3c = 0x40
    I(0x13, 0, 10, 10, 0x3c),
    S(3, 20, 10, 0),
    U(0x37, 31, 0x20), // 131072 stable D -> A edges before switching
    J(0, 0x0c), // D at 0x20
    I(0x13, 0, 0, 0, 0),
    I(0x13, 0, 0, 0, 0),
    I(0x03, 3, 10, 20, 0), // 0x20 D: dynamic target from data
    I(0x67, 0, 0, 10, 0),
  ];
  while (code.length < 16) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x13, 0, 1, 1, 1), // 0x40 A
    I(0x13, 0, 31, 31, -1),
    B(0, 31, 0, 8), // switch target at 0x50
    J(0, -0x2c), // back to D
    U(0x17, 10, 0), // 0x50 + 0x10 = B at 0x60
    I(0x13, 0, 10, 10, 0x10),
    S(3, 20, 10, 0),
    J(0, -0x3c), // execute D once with its new target
    I(0x13, 0, 2, 0, 0x55), // 0x60 B
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

function polymorphicTargetProgram() {
  const code = [
    U(0x17, 20, 0x10), // x20 = data slot at RAM_BASE + 64 KiB
    U(0x17, 11, 0),
    I(0x13, 0, 11, 11, 0x5c), // x11 = target A at 0x60
    U(0x17, 12, 0),
    I(0x13, 0, 12, 12, 0x74), // x12 = target B at 0x80
    U(0x37, 31, 0x40), // 262144 alternating target executions
    S(3, 20, 11, 0),
    J(0, 0x24), // enter the dynamic dispatcher at 0x40
  ];

  while (code.length < 16) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x03, 3, 10, 20, 0), // 0x40 D: load the current dynamic target
    I(0x67, 0, 0, 10, 0),
  );

  while (code.length < 24) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x13, 0, 1, 1, 1), // 0x60 A
    I(0x13, 0, 31, 31, -1),
    B(0, 31, 0, 0x38), // final iteration exits at 0xa0
    S(3, 20, 12, 0), // next target is B
    J(0, -0x30), // return to D
  );

  while (code.length < 32) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x13, 0, 2, 2, 1), // 0x80 B
    I(0x13, 0, 31, 31, -1),
    B(0, 31, 0, 0x18), // final iteration exits at 0xa0
    S(3, 20, 11, 0), // next target is A
    J(0, -0x50), // return to D
  );

  while (code.length < 40) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    U(0x37, 24, 0x40008), // 0xa0: HTIF tohost power-off
    I(0x13, 0, 25, 0, 1),
    S(3, 24, 25, 0),
    0x0000_006f,
  );

  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

function crossPagePolymorphicTargetProgram() {
  const code = [
    U(0x17, 20, 0x10), // x20 = data slot at RAM_BASE + 64 KiB
    U(0x17, 11, 0),
    I(0x13, 0, 11, 11, 0x5c), // x11 = target A at 0x60
    U(0x17, 12, 1),
    I(0x13, 0, 12, 12, -0x0c), // x12 = target B at 0x1000
    U(0x37, 31, 0x40), // 262144 alternating target executions
    S(3, 20, 11, 0),
    J(0, 0x24), // enter the dynamic dispatcher at 0x40
  ];

  while (code.length < 16) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x03, 3, 10, 20, 0), // 0x40 D: load the current dynamic target
    I(0x67, 0, 0, 10, 0),
  );

  while (code.length < 24) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x13, 0, 1, 1, 1), // 0x60 A
    I(0x13, 0, 31, 31, -1),
    B(0, 31, 0, 0xfb8), // exit at 0x1020
    S(3, 20, 12, 0),
    J(0, -0x30), // return to D
  );

  while (code.length < 1024) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x13, 0, 2, 2, 1), // 0x1000 B
    I(0x13, 0, 31, 31, -1),
    B(0, 31, 0, 0x18), // exit at 0x1020
    S(3, 20, 11, 0),
    J(0, -0xfd0), // return to D at 0x40
  );

  while (code.length < 1032) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    U(0x37, 24, 0x40008), // 0x1020: HTIF tohost power-off
    I(0x13, 0, 25, 0, 1),
    S(3, 24, 25, 0),
    0x0000_006f,
  );

  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

function crossPagePolymorphicSmcProgram() {
  const replacement = I(0x13, 0, 2, 2, 2); // addi x2,x2,2
  const code = [
    U(0x17, 20, 0x10), // x20 = dynamic-target data slot
    U(0x17, 11, 0),
    I(0x13, 0, 11, 11, 0x5c), // x11 = A at 0x60
    U(0x17, 12, 1),
    I(0x13, 0, 12, 12, -0x0c), // x12 = B at 0x1000
    I(0x13, 0, 21, 12, 0), // x21 = B instruction address
    U(0x37, 22, 0x210),
    I(0x13, 0, 22, 22, 0x113), // x22 = replacement encoding
    U(0x37, 30, 0x1), // modify after the two-page PIC is resident
    U(0x37, 31, 0x40), // 262144 total target executions
    S(3, 20, 11, 0),
    J(0, 0x14), // dynamic dispatcher at 0x40
  ];
  if ((replacement >>> 0) !== 0x0021_0113) {
    throw new Error("unexpected SMC replacement encoding");
  }

  while (code.length < 16) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x03, 3, 10, 20, 0), // 0x40 D
    I(0x67, 0, 0, 10, 0),
  );

  while (code.length < 24) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x13, 0, 1, 1, 1), // 0x60 A
    I(0x13, 0, 31, 31, -1),
    B(0, 31, 0, 0xfd8), // exit at 0x1040
    S(3, 20, 12, 0),
    J(0, -0x30),
  );

  while (code.length < 1024) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    I(0x13, 0, 2, 2, 1), // 0x1000 B, replaced halfway through
    I(0x13, 0, 31, 31, -1),
    B(0, 31, 0, 0x38), // exit at 0x1040
    B(1, 31, 30, 8), // skip the one-time code store
    S(2, 21, 22, 0), // overwrite B's first instruction
    S(3, 20, 11, 0),
    J(0, -0xfd8), // return to D at 0x40
  );

  while (code.length < 1040) code.push(I(0x13, 0, 0, 0, 0));
  code.push(
    U(0x37, 24, 0x40008), // 0x1040: HTIF tohost power-off
    I(0x13, 0, 25, 0, 1),
    S(3, 24, 25, 0),
    0x0000_006f,
  );

  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

// Six hot indirect members force the register-resident batch backend to carry
// a real LR/SC body. Publishing generated code clears the store JTLB, so the
// first compiled SC reaches the default no-refill side exit. Its reservation
// must survive exact T0 re-execution; x2 accumulates every failed SC and makes
// even one lost attempt visible.
function atomicMultiEntryProgram() {
  const code = [];
  const labels = new Map();
  const addressPatches = [];
  const branchPatches = [];
  const label = (name) => labels.set(name, code.length * 4);
  const addressOf = (register, name) => {
    const pc = code.length * 4;
    code.push(U(0x17, register, 0), 0);
    addressPatches.push({ index: code.length - 1, register, name, pc });
  };

  code.push(
    U(0x17, 20, 0x10), // x20 = RAM_BASE + 64 KiB, initially zero
    I(0x13, 0, 4, 0, 1),
    U(0x37, 31, 0x20), // 131072 complete six-member cycles
    I(0x13, 0, 1, 0, 0),
    I(0x13, 0, 2, 0, 0),
  );
  for (const [register, name] of [
    [11, "b1"], [12, "b2"], [13, "b3"],
    [14, "b4"], [15, "b5"], [16, "b0"],
  ]) addressOf(register, name);
  code.push(I(0x67, 0, 0, 16, 0));

  label("b0");
  code.push(
    AMO(0x02, 3, 3, 20, 0), // lr.d.aqrl x3,(x20)
    AMO(0x03, 3, 5, 20, 4), // sc.d.aqrl x5,x4,(x20)
    R(0x33, 0, 0, 2, 2, 5), // accumulate failures
    S(3, 20, 0, 0), // release for the next cycle
    I(0x13, 0, 1, 1, 1),
    I(0x67, 0, 0, 11, 0),
  );
  for (const [name, increment, next] of [
    ["b1", 2, 12], ["b2", 3, 13], ["b3", 4, 14], ["b4", 5, 15],
  ]) {
    label(name);
    code.push(I(0x13, 0, 1, 1, increment), I(0x67, 0, 0, next, 0));
  }
  label("b5");
  code.push(I(0x13, 0, 1, 1, 6), I(0x13, 0, 31, 31, -1));
  const finalBranch = code.length;
  code.push(0, I(0x67, 0, 0, 16, 0));
  branchPatches.push({ index: finalBranch, name: "exit", pc: finalBranch * 4 });
  label("exit");
  code.push(
    U(0x37, 24, 0x40008),
    I(0x13, 0, 25, 0, 1),
    S(3, 24, 25, 0),
    0x0000_006f,
  );

  for (const { index, register, name, pc } of addressPatches) {
    const delta = labels.get(name) - pc;
    if (delta < -2048 || delta > 2047) throw new Error(`target ${name} is too far`);
    code[index] = I(0x13, 0, register, register, delta);
  }
  for (const { index, name, pc } of branchPatches) {
    code[index] = B(0, 31, 0, labels.get(name) - pc);
  }
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

const yieldToCompiler = () => new Promise((resolve) => setImmediate(resolve));

// A region is deliberately compiled with WebAssembly.compile(), so publication
// happens on the host event loop between runSystem calls.  Fast engines usually
// finish within the next 100K-instruction slice, but that is not an API
// guarantee (and older V8 builds can take longer for the direct-dispatch
// variant).  Once this execution-focused test has caused a region build, keep
// the guest paused until the build lands or a bounded diagnostic timeout
// expires.  That guarantees there is still guest work left with which to prove
// the generated region actually executes, independent of host/engine speed.
async function settlePendingRegions(vm, timeoutMs = 10_000) {
  if (vm.ex.sys_pending_builds() === 0) return;
  const deadline = performance.now() + timeoutMs;
  while (vm.ex.sys_pending_builds() !== 0 && performance.now() < deadline) {
    await yieldToCompiler();
  }
}

async function run(mode) {
  const jit = mode !== "interpreter";
  const superblock =
    mode === "superblock" ||
    mode === "superblock-direct" ||
    mode === "superblock-cfg" ||
    mode === "superblock-structured" ||
    mode === "superblock-structured-chain" ||
    mode === "superblock-structured-tail";
  const vm = await RV64.create(wasm);
  const moduleKinds = [];
  let tailFunctionImports = 0;
  let unexpectedTailTableImports = 0;
  vm.onJitModule = (bytes, metadata) => {
    moduleKinds.push(metadata.kind);
    const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
    if (imports.some((entry) => entry.name === "tail_chain" && entry.kind === "function")) {
      tailFunctionImports++;
    }
    if (imports.some((entry) => entry.kind === "table")) unexpectedTailTableImports++;
  };
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.sys_set_superblock(superblock ? 1 : 0);
  if (superblock) vm.ex.jit_set_sb_spacing(0);
  vm.ex.jit_set_region_direct_dispatch(mode === "superblock-direct" ? 1 : 0);
  vm.ex.jit_set_region_cfg_blocks(mode === "superblock-cfg" ? 1 : 0);
  vm.ex.jit_set_region_structured_cfg(mode.startsWith("superblock-structured") ? 1 : 0);
  vm.ex.jit_set_region_chain(mode === "superblock-structured-chain" ? 1 : 0);
  vm.ex.jit_set_region_tail_chain(mode === "superblock-structured-tail" ? 1 : 0);
  if (mode === "batch") {
    vm.ex.jit_set_batch(1);
    vm.ex.jit_set_ic_trigger(64);
  }
  if (mode === "ic") vm.ex.jit_set_ic_trigger(64);
  vm.ex.dprof_set(jit ? 1 : 0);
  vm.onWrite = () => {};
  vm.bootLinux({ bios: program(), ramMB: 32 });
  let completed = false;
  let maxSlice = 0n;
  for (let slice = 0; slice < 100 && !completed; slice++) {
    const before = vm.ex.sys_insn_count();
    vm.runSystem(100_000n);
    const retired = vm.ex.sys_insn_count() - before;
    if (retired > maxSlice) maxSlice = retired;
    completed = vm.ex.sys_reg(31) === 0n && vm.ex.sys_pc() === 0x8000_008cn;
    if (jit) {
      await yieldToCompiler();
      await settlePendingRegions(vm);
    }
  }
  return {
    completed,
    pc: vm.ex.sys_pc(),
    x1: vm.ex.sys_reg(1),
    x31: vm.ex.sys_reg(31),
    instructions: vm.ex.sys_insn_count(),
    issued: vm.ex.jit_stat(12),
    landed: vm.ex.jit_stat(13),
    pending: vm.ex.sys_pending_builds(),
    regionCalls: vm.ex.jit_stat(48),
    regionInsns: vm.ex.jit_stat(49),
    jitInsns: vm.ex.jit_stat(0),
    dispatches: vm.ex.jit_stat(1),
    chainHops: vm.ex.jit_stat(80),
    maxSlice,
    batches: vm.ex.jit_stat(43),
    batchMembers: vm.ex.jit_stat(44),
    icExtends: vm.ex.jit_stat(45),
    moduleKinds,
    tailFunctionImports,
    unexpectedTailTableImports,
    lifecycle: vm.jitLifecycleStats(),
  };
}

async function runChangingTarget(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  if (jit) vm.ex.jit_set_ic_trigger(64);
  vm.onWrite = () => {};
  vm.bootLinux({ bios: changingTargetProgram(), ramMB: 32 });
  let poweredOff = 0;
  for (let slice = 0; slice < 20 && !poweredOff; slice++) {
    poweredOff = vm.runSystem(100_000n);
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    x1: vm.ex.sys_reg(1),
    x2: vm.ex.sys_reg(2),
    x31: vm.ex.sys_reg(31),
    target: vm.ex.sys_ram_u64(0x8001_0000n),
    icExtends: vm.ex.jit_stat(45),
    jitInsns: vm.ex.jit_stat(0),
    dispatches: vm.ex.jit_stat(1),
  };
}

async function runPolymorphicTarget(jit, crossPage = false) {
  const vm = await RV64.create(wasm);
  const moduleBytes = [];
  vm.onJitModule = (bytes) => moduleBytes.push(bytes.length);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  if (jit) vm.ex.jit_set_ic_trigger(64);
  vm.onWrite = () => {};
  vm.bootLinux({
    bios: crossPage
      ? crossPagePolymorphicTargetProgram()
      : polymorphicTargetProgram(),
    ramMB: 32,
  });
  let poweredOff = 0;
  let maxSlice = 0n;
  for (let slice = 0; slice < 40 && !poweredOff; slice++) {
    const before = vm.ex.sys_insn_count();
    poweredOff = vm.runSystem(100_000n);
    const retired = vm.ex.sys_insn_count() - before;
    if (retired > maxSlice) maxSlice = retired;
    if (jit) await yieldToCompiler();
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    x1: vm.ex.sys_reg(1),
    x2: vm.ex.sys_reg(2),
    x31: vm.ex.sys_reg(31),
    target: vm.ex.sys_ram_u64(0x8001_0000n),
    icExtends: vm.ex.jit_stat(45),
    picExtends: vm.ex.jit_stat(79),
    jitInsns: vm.ex.jit_stat(0),
    dispatches: vm.ex.jit_stat(1),
    maxSlice,
    moduleBytes,
  };
}

async function runCrossPageSmc(jit) {
  const vm = await RV64.create(wasm);
  const moduleEvents = [];
  vm.onJitModule = (bytes) => moduleEvents.push({
    bytes: bytes.length,
    remaining: vm.ex.sys_reg(31),
  });
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.jit_set_tlb_fill(1);
  if (jit) vm.ex.jit_set_ic_trigger(64);
  vm.onWrite = () => {};
  vm.bootLinux({ bios: crossPagePolymorphicSmcProgram(), ramMB: 32 });
  let poweredOff = 0;
  let maxSlice = 0n;
  for (let slice = 0; slice < 40 && !poweredOff; slice++) {
    const before = vm.ex.sys_insn_count();
    poweredOff = vm.runSystem(100_000n);
    const retired = vm.ex.sys_insn_count() - before;
    if (retired > maxSlice) maxSlice = retired;
    if (jit) await yieldToCompiler();
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    x1: vm.ex.sys_reg(1),
    x2: vm.ex.sys_reg(2),
    x31: vm.ex.sys_reg(31),
    target: vm.ex.sys_ram_u64(0x8001_0000n),
    code: vm.ex.sys_ram_u64(0x8000_1000n),
    picExtends: vm.ex.jit_stat(79),
    dirtyEvents: vm.ex.jit_stat(23),
    dirtyDropped: vm.ex.jit_stat(24),
    jitInsns: vm.ex.jit_stat(0),
    dispatches: vm.ex.jit_stat(1),
    maxSlice,
    moduleEvents,
  };
}

async function runAtomicMultiEntry(jit) {
  const vm = await RV64.create(wasm);
  let reservationBatches = 0;
  vm.onJitModule = (bytes, metadata) => {
    if (metadata.kind !== "batch") return;
    const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
    if (imports.some((entry) => entry.name === "system_reservation")) {
      reservationBatches++;
    }
  };
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.jit_set_tlb_fill(0);
  if (jit) {
    vm.ex.jit_set_batch(1);
    vm.ex.jit_set_ic_trigger(64);
  }
  vm.onWrite = () => {};
  vm.bootLinux({ bios: atomicMultiEntryProgram(), ramMB: 32 });
  let poweredOff = 0;
  let maxSlice = 0n;
  for (let slice = 0; slice < 100 && !poweredOff; slice++) {
    const before = vm.ex.sys_insn_count();
    poweredOff = vm.runSystem(100_000n);
    const retired = vm.ex.sys_insn_count() - before;
    if (retired > maxSlice) maxSlice = retired;
    if (jit) await yieldToCompiler();
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    x1: vm.ex.sys_reg(1),
    failures: vm.ex.sys_reg(2),
    remaining: vm.ex.sys_reg(31),
    memory: vm.ex.sys_ram_u64(0x8001_0000n),
    jitInsns: vm.ex.jit_stat(0),
    batches: vm.ex.jit_stat(43),
    batchMembers: vm.ex.jit_stat(44),
    tlbFills: vm.ex.jit_stat(31),
    reservationBatches,
    maxSlice,
  };
}

{
  const vm = await RV64.create(wasm);
  const result = vm.ex.sbtest();
  if (result !== 55n) {
    console.log(`JIT T2 MULTI-ENTRY: FAIL — sbtest returned ${result}`);
    process.exit(1);
  }
  const structuredResult = vm.ex.sbtest_structured();
  if (structuredResult !== 55n) {
    console.log(
      `JIT T2 MULTI-ENTRY: FAIL — sbtest_structured returned ${structuredResult}`,
    );
    process.exit(1);
  }
  const crossModuleModules = [];
  vm.onJitModule = (bytes, metadata) => {
    crossModuleModules.push({ bytes, metadata });
  };
  const crossModuleTail = vm.ex.sbtest_structured_tail_chain();
  if (crossModuleTail !== 0x000a_000a_0028_0013n) {
    console.log(
      `JIT T2 MULTI-ENTRY: FAIL — sbtest_structured_tail_chain returned 0x` +
        `${BigInt.asUintN(64, crossModuleTail).toString(16)}`,
    );
    process.exit(1);
  }
  if (
    vm.ex.jit_stat(80) !== 19n ||
    crossModuleModules.length !== 2 ||
    crossModuleModules.some(({ bytes }) =>
      !WebAssembly.Module.imports(new WebAssembly.Module(bytes)).some(
        (entry) => entry.name === "tail_chain" && entry.kind === "function",
      ) || WebAssembly.Module.imports(new WebAssembly.Module(bytes)).some(
        (entry) => entry.kind === "table",
      )
    )
  ) {
    console.log("JIT T2 MULTI-ENTRY: FAIL — cross-module tail lifecycle was not exercised");
    process.exit(1);
  }
  const tailResult = vm.ex.sbtest_tail_call();
  if (tailResult !== 0x0000_000a_0000_000an) {
    console.log(
      `JIT T2 MULTI-ENTRY: FAIL — sbtest_tail_call returned 0x` +
        `${BigInt.asUintN(64, tailResult).toString(16)}`,
    );
    process.exit(1);
  }
  for (const name of ["sbtest_fp", "sbtest_fp_lazy"]) {
    const fpResult = vm.ex[name]();
    if (fpResult !== 0x4018_0000_0000_0000n) {
      console.log(
        `JIT T2 MULTI-ENTRY: FAIL — ${name} returned 0x` +
          `${BigInt.asUintN(64, fpResult).toString(16)}`,
      );
      process.exit(1);
    }
  }
}

const interpreter = await run("interpreter");
const compiled = await run("superblock");
const direct = await run("superblock-direct");
const cfg = await run("superblock-cfg");
const structured = await run("superblock-structured");
const structuredChain = await run("superblock-structured-chain");
const structuredTail = await run("superblock-structured-tail");
const batch = await run("batch");
const ic = await run("ic");
const changingInterpreter = await runChangingTarget(false);
const changingIc = await runChangingTarget(true);
const polymorphicInterpreter = await runPolymorphicTarget(false);
const polymorphicIc = await runPolymorphicTarget(true);
const crossPageInterpreter = await runPolymorphicTarget(false, true);
const crossPageIc = await runPolymorphicTarget(true, true);
const smcInterpreter = await runCrossPageSmc(false);
const smcIc = await runCrossPageSmc(true);
const atomicInterpreter = await runAtomicMultiEntry(false);
const atomicBatch = await runAtomicMultiEntry(true);
const failures = [];
for (const field of ["completed", "pc", "x1", "x31", "instructions"]) {
  if (interpreter[field] !== compiled[field]) failures.push(`${field} differs`);
  if (interpreter[field] !== direct[field]) failures.push(`direct ${field} differs`);
  if (interpreter[field] !== cfg[field]) failures.push(`CFG ${field} differs`);
  if (interpreter[field] !== structured[field]) {
    failures.push(`structured CFG ${field} differs`);
  }
  if (interpreter[field] !== structuredChain[field]) {
    failures.push(`chained structured CFG ${field} differs`);
  }
  if (interpreter[field] !== structuredTail[field]) {
    failures.push(`tail-chained structured CFG ${field} differs`);
  }
  if (interpreter[field] !== batch[field]) failures.push(`batch ${field} differs`);
  if (interpreter[field] !== ic[field]) failures.push(`IC ${field} differs`);
}
if (!interpreter.completed) failures.push("multi-entry WFI program did not complete");
if (compiled.x1 !== 2_752_512n) failures.push(`unexpected checksum ${compiled.x1}`);
if (compiled.issued === 0n || compiled.landed === 0n) {
  failures.push("multi-entry region did not issue and land");
}
if (compiled.regionCalls === 0n || compiled.regionInsns === 0n) {
  failures.push("landed region did not execute");
}
for (const [name, state] of [
  ["region", compiled], ["direct region", direct], ["CFG region", cfg],
  ["structured CFG region", structured],
  ["chained structured CFG region", structuredChain],
  ["tail-chained structured CFG region", structuredTail],
  ["batch", batch], ["IC", ic],
]) {
  if (state.maxSlice > 100_100n) {
    failures.push(`${name} exceeded the cumulative slice-fuel bound`);
  }
}
if (
  !compiled.moduleKinds.includes("async-region") ||
  compiled.lifecycle.copiedBytes === 0 ||
  compiled.lifecycle.systemTranslateAttempts === 0
) {
  failures.push("async region lifecycle/capture was not accounted");
}
if (
  direct.issued === 0n || direct.landed === 0n ||
  direct.regionCalls === 0n || direct.regionInsns === 0n ||
  !direct.moduleKinds.includes("async-region")
) {
  failures.push("direct structured region did not land and execute");
}
if (
  cfg.issued === 0n || cfg.landed === 0n ||
  cfg.regionCalls === 0n || cfg.regionInsns === 0n ||
  !cfg.moduleKinds.includes("async-region")
) {
  failures.push("CFG basic-block region did not land and execute");
}
if (
  structured.issued === 0n || structured.landed === 0n ||
  structured.regionCalls === 0n || structured.regionInsns === 0n ||
  !structured.moduleKinds.includes("async-region")
) {
  failures.push("structured CFG region did not land and execute");
}
if (
  structuredChain.issued === 0n || structuredChain.landed === 0n ||
  structuredChain.regionCalls === 0n || structuredChain.regionInsns === 0n ||
  !structuredChain.moduleKinds.includes("async-region")
) {
  failures.push("chained structured CFG region did not land and execute");
}
if (
  structuredTail.issued === 0n || structuredTail.landed === 0n ||
  structuredTail.regionCalls === 0n || structuredTail.regionInsns === 0n ||
  !structuredTail.moduleKinds.includes("async-region") ||
  structuredTail.tailFunctionImports === 0 ||
  structuredTail.unexpectedTailTableImports !== 0
) {
  failures.push("tail-chained structured CFG region did not import, land, and execute");
}
if (batch.batches === 0n || batch.batchMembers < 2n) {
  failures.push("observed multi-entry batch did not execute");
}
if (
  !batch.moduleKinds.includes("batch") ||
  batch.lifecycle.copiedBytes === 0 ||
  batch.lifecycle.systemTranslateAttempts === 0
) {
  failures.push("batch lifecycle/capture was not accounted");
}
if (ic.icExtends === 0n) failures.push("monomorphic target extension did not compile");
for (const field of ["poweredOff", "pc", "x1", "x2", "x31", "target"]) {
  if (changingInterpreter[field] !== changingIc[field]) {
    failures.push(`changing-target ${field} differs`);
  }
}
if (changingIc.icExtends === 0n || changingIc.x1 !== 131_072n || changingIc.x2 !== 0x55n) {
  failures.push("dynamic indirect-target miss path did not complete precisely");
}
for (const field of ["poweredOff", "pc", "x1", "x2", "x31", "target"]) {
  if (polymorphicInterpreter[field] !== polymorphicIc[field]) {
    failures.push(`polymorphic-target ${field} differs`);
  }
}
if (
  polymorphicIc.picExtends !== 1n ||
  polymorphicIc.icExtends > 2n ||
  polymorphicIc.x1 !== 131_072n ||
  polymorphicIc.x2 !== 131_072n ||
  polymorphicIc.maxSlice > 100_100n
) {
  failures.push("two-way polymorphic target cache did not execute both arms");
}
for (const field of ["poweredOff", "pc", "x1", "x2", "x31", "target"]) {
  if (crossPageInterpreter[field] !== crossPageIc[field]) {
    failures.push(`cross-page polymorphic-target ${field} differs`);
  }
}
if (
  crossPageIc.picExtends !== 1n ||
  crossPageIc.icExtends > 2n ||
  crossPageIc.x1 !== 131_072n ||
  crossPageIc.x2 !== 131_072n ||
  crossPageIc.dispatches >= 1_000n ||
  crossPageIc.maxSlice > 100_100n
) {
  failures.push("cross-page polymorphic target cache did not retain both arms");
}
for (const field of ["poweredOff", "pc", "x1", "x2", "x31", "target", "code"]) {
  if (smcInterpreter[field] !== smcIc[field]) {
    failures.push(`cross-page PIC invalidation ${field} differs`);
  }
}
if (
  smcIc.picExtends !== 1n ||
  smcIc.dirtyEvents === 0n ||
  smcIc.dirtyDropped === 0n ||
  smcIc.x1 !== 131_072n ||
  smcIc.x2 !== 133_120n ||
  (smcIc.code & 0xffff_ffffn) !== 0x0021_0113n ||
  !smcIc.moduleEvents.some((event) => event.remaining <= 4_096n) ||
  smcIc.maxSlice > 100_100n
) {
  failures.push("cross-page PIC code-page invalidation was not precise and resident");
}
for (const field of ["poweredOff", "pc", "x1", "failures", "remaining", "memory"]) {
  if (atomicInterpreter[field] !== atomicBatch[field]) {
    failures.push(`atomic multi-entry ${field} differs`);
  }
}
if (
  atomicBatch.x1 !== 2_752_512n ||
  atomicBatch.failures !== 0n ||
  atomicBatch.remaining !== 0n ||
  atomicBatch.memory !== 0n ||
  atomicBatch.batches === 0n ||
  atomicBatch.batchMembers < 2n ||
  atomicBatch.reservationBatches === 0 ||
  atomicBatch.tlbFills !== 0n ||
  atomicBatch.jitInsns === 0n ||
  atomicBatch.maxSlice > 100_100n
) {
  failures.push("no-refill LR/SC did not execute precisely inside a T2 batch");
}

if (failures.length) {
  console.log("interpreter", interpreter);
  console.log("compiled", compiled);
  console.log("direct", direct);
  console.log("structured chained", structuredChain);
  console.log("structured tail-chained", structuredTail);
  console.log("batch", batch);
  console.log("ic", ic);
  console.log("changing interpreter", changingInterpreter);
  console.log("changing IC", changingIc);
  console.log("polymorphic interpreter", polymorphicInterpreter);
  console.log("polymorphic IC", polymorphicIc);
  console.log("cross-page interpreter", crossPageInterpreter);
  console.log("cross-page IC", crossPageIc);
  console.log("cross-page SMC interpreter", smcInterpreter);
  console.log("cross-page SMC IC", smcIc);
  console.log("atomic multi-entry interpreter", atomicInterpreter);
  console.log("atomic multi-entry batch", atomicBatch);
  console.log(`JIT T2 MULTI-ENTRY: FAIL — ${failures.join(", ")}`);
  process.exit(1);
}
console.log(
  `PASS T2 multi-entry region — issued=${compiled.issued} landed=${compiled.landed} ` +
    `region-insns=${compiled.regionInsns} jit-insns=${compiled.jitInsns}`,
);
console.log(
  `PASS T2 direct structured region — issued=${direct.issued} landed=${direct.landed} ` +
    `region-insns=${direct.regionInsns} jit-insns=${direct.jitInsns}`,
);
console.log(
  `PASS T2 structured region chaining — issued=${structuredChain.issued} ` +
    `landed=${structuredChain.landed} chain-hops=${structuredChain.chainHops} ` +
    `jit-insns=${structuredChain.jitInsns}`,
);
console.log(
  `PASS T2 structured tail chaining — issued=${structuredTail.issued} ` +
    `landed=${structuredTail.landed} chain-hops=${structuredTail.chainHops} ` +
    `tail-function-imports=${structuredTail.tailFunctionImports} ` +
      `jit-insns=${structuredTail.jitInsns}`,
);
console.log(
  `PASS T2 observed batch — batches=${batch.batches} members=${batch.batchMembers} ` +
    `jit-insns=${batch.jitInsns}`,
);
console.log(
  `PASS T2 monomorphic target extension — extensions=${ic.icExtends} ` +
    `jit-insns=${ic.jitInsns} dispatches=${ic.dispatches}`,
);
console.log(
  `PASS T2 changing-target precise exit — extensions=${changingIc.icExtends} ` +
    `jit-insns=${changingIc.jitInsns} dispatches=${changingIc.dispatches}`,
);
console.log(
  `PASS T2 two-way polymorphic target cache — extensions=${polymorphicIc.icExtends} ` +
    `pic-extensions=${polymorphicIc.picExtends} jit-insns=${polymorphicIc.jitInsns} ` +
    `dispatches=${polymorphicIc.dispatches}`,
);
console.log(
  `PASS T2 cross-page polymorphic cache — extensions=${crossPageIc.icExtends} ` +
    `pic-extensions=${crossPageIc.picExtends} jit-insns=${crossPageIc.jitInsns} ` +
    `dispatches=${crossPageIc.dispatches}`,
);
console.log(
  `PASS T2 cross-page PIC invalidation — dirty=${smcIc.dirtyEvents}/` +
    `${smcIc.dirtyDropped} jit-insns=${smcIc.jitInsns} ` +
    `dispatches=${smcIc.dispatches}`,
);
console.log(
  `PASS T2 no-refill LR/SC batch — batches=${atomicBatch.batches} ` +
    `members=${atomicBatch.batchMembers} reservation-batches=` +
    `${atomicBatch.reservationBatches} jit-insns=${atomicBatch.jitInsns}`,
);
console.log("JIT T2 MULTI-ENTRY DIFFERENTIAL: ALL PASS");
