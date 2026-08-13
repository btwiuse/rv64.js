// Full-system fused-TLB differential using a synthetic bare-metal BIOS. Two
// data pages intentionally alias in the direct-mapped translation rows. The
// loop has two same-width load-to-store pairs so the dense-copy invocation
// cache is exercised without enabling that cache for general memory regions.

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

function program() {
  const code = [
    U(0x17, 20, 0x10), // x20 = RAM_BASE + 64 KiB
    U(0x37, 19, 0x1000), // x19 = 16 MiB (one 4096-row alias period)
    R(0x33, 0, 0, 21, 20, 19), // x21 aliases x20's fused-TLB slot
    I(0x13, 0, 1, 0, 0x135),
    S(3, 20, 1, 0),
    S(3, 21, 1, 0),
    U(0x37, 31, 0x10), // 65536 iterations
    I(0x13, 0, 0, 0, 0),
    I(0x13, 0, 0, 0, 0),
    I(0x13, 0, 0, 0, 0),
    I(0x13, 0, 0, 0, 0), // trampoline + setup = 16 insns; align 4096 slices
  ];
  const loop = code.length;
  code.push(
    I(0x03, 3, 3, 20, 0),
    I(0x03, 3, 4, 21, 0),
    S(3, 20, 3, 8),
    S(3, 21, 4, 8),
    I(0x13, 0, 0, 0, 0),
    I(0x13, 0, 0, 0, 0),
    I(0x13, 0, 31, 31, -1),
  );
  code.push(B(1, 31, 0, (loop - code.length) * 4));
  code.push(
    I(0x03, 3, 22, 20, 8),
    I(0x03, 3, 23, 21, 8),
    U(0x37, 24, 0x40008), // HTIF tohost = 0x40008000
    I(0x13, 0, 25, 0, 1),
    S(3, 24, 25, 0), // odd tohost value powers the machine off
    0x0000_006f,
  );
  const bytes = new Uint8Array(code.length * 4);
  const view = new DataView(bytes.buffer);
  code.forEach((word, index) => view.setUint32(index * 4, word >>> 0, true));
  return bytes;
}

async function run(
  jit,
  refill,
  selectiveCache = false,
  tlbHash = false,
) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_interpreter_fused_memory(interpreterFusedMemory ? 1 : 0);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.jit_set_tlb_fill(refill ? 1 : 0);
  vm.ex.jit_set_tlb_hash(tlbHash ? 1 : 0);
  vm.ex.jit_set_region_tlb_cache(selectiveCache ? 1 : 0);
  vm.ex.jit_set_region_tlb_cache_min_accesses(2);
  vm.onWrite = () => {};
  vm.bootLinux({ bios: program(), ramMB: 32 });
  let poweredOff = 0;
  for (let iteration = 0; iteration < 100 && !poweredOff; iteration++) {
    poweredOff = vm.runSystem(2_000_000n);
  }
  return {
    poweredOff,
    pc: vm.ex.sys_pc(),
    insns: vm.ex.sys_insn_count(),
    regs: Array.from({ length: 32 }, (_, index) => vm.ex.sys_reg(index)),
    memory: [vm.ex.sys_ram_u64(0x8001_0008n), vm.ex.sys_ram_u64(0x8101_0008n)],
    jitInsns: vm.ex.jit_stat(0),
    jitBlocks: vm.ex.jit_stat(3),
    zeroRetire: vm.ex.jit_stat(15),
    tlbFills: vm.ex.jit_stat(31),
  };
}

function differences(reference, candidate) {
  const found = [];
  // The SystemBus power-off flag is sampled only after the current host slice;
  // cold and compiled schedulers therefore retire different numbers of the
  // terminal `jal 0` after the architecturally visible HTIF write. Compare the
  // completion state and useful-work result, not that host-slice padding.
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

const interpreter = await run(false, false);
const sideExit = await run(true, false);
const refill = await run(true, true);
const selectiveCache = await run(true, true, true);
const hashed = await run(true, true, false, true);
const sideDifferences = differences(interpreter, sideExit);
const refillDifferences = differences(interpreter, refill);
const selectiveDifferences = differences(interpreter, selectiveCache);
const hashedDifferences = differences(interpreter, hashed);
if (sideExit.zeroRetire === 0n) sideDifferences.push("TLB conflict did not side-exit");
if (refill.tlbFills === 0n || refill.jitInsns === 0n) {
  refillDifferences.push("typed refill path did not execute compiled memory");
}
if (selectiveCache.tlbFills === 0n || selectiveCache.jitInsns === 0n) {
  selectiveDifferences.push("selective translation cache did not execute compiled memory");
}
if (selectiveCache.zeroRetire > refill.zeroRetire + 1024n) {
  selectiveDifferences.push(
    `selective uncached access caused excess zero-retire exits ` +
      `(${selectiveCache.zeroRetire} versus ${refill.zeroRetire})`,
  );
}
if (hashed.tlbFills === 0n || hashed.jitInsns === 0n) {
  hashedDifferences.push("hashed TLB path did not execute compiled memory");
}
if (
  sideDifferences.length ||
  refillDifferences.length ||
  selectiveDifferences.length ||
  hashedDifferences.length
) {
  console.log("interpreter state", interpreter);
  console.log("side-exit state", sideExit);
  console.log("refill state", refill);
  console.log("selective-cache state", selectiveCache);
  console.log("hashed-TLB state", hashed);
  console.log(`FAIL side-exit: ${sideDifferences.join(", ") || "none"}`);
  console.log(`FAIL refill: ${refillDifferences.join(", ") || "none"}`);
  console.log(`FAIL selective cache: ${selectiveDifferences.join(", ") || "none"}`);
  console.log(`FAIL hashed TLB: ${hashedDifferences.join(", ") || "none"}`);
  process.exit(1);
}
console.log(
  `PASS system memory side-exit — blocks=${sideExit.jitBlocks} zero-retire=${sideExit.zeroRetire}`,
);
console.log(
  `PASS system memory refill — jit-insns=${refill.jitInsns} tlb-fills=${refill.tlbFills}`,
);
console.log(
  `PASS selective system-memory cache — jit-insns=${selectiveCache.jitInsns} ` +
    `tlb-fills=${selectiveCache.tlbFills} zero-retire=${selectiveCache.zeroRetire}`,
);
console.log(
  `PASS hashed system-memory TLB — jit-insns=${hashed.jitInsns} ` +
    `tlb-fills=${hashed.tlbFills} zero-retire=${hashed.zeroRetire}`,
);
console.log("JIT SYSTEM MEMORY DIFFERENTIAL: ALL PASS");
