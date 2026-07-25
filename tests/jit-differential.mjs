// JIT differential fuzzer (ISSUES.md P1): random instruction programs run
// twice in the user-mode machine — JIT enabled and disabled — and the FULL
// architectural state must be bit-identical: pc, exit code, insn_count,
// x1..x31, f0..f31, fcsr.
//
// Programs are synthesized ELFs: registers seeded with edge values (0, -1,
// i64::MIN, small ints), then a LOOP whose body is random ALU/FP
// instructions across every family the JIT compiles (OP, OP-32, OP-IMM,
// OP-IMM-32, LUI/AUIPC, FMV/FP-arith/FMADD/FCVT/FSQRT). 100 iterations
// crosses the tier-up threshold mid-run, so each program exercises
// interpreter execution, tier-up, compiled execution, FP eligibility
// bails, and division edge cases — any retirement or semantic divergence
// shows up as a state mismatch.
//
//   node tests/jit-differential.mjs [n-programs] [seed]
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64, Stop } = await import(join(root, "web/rv64.js"));
const wasmBytes = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

const N_PROGRAMS = +(process.argv[2] || 60);
let seed = BigInt(process.argv[3] || 0x1234_5678);
const rnd = () => {
  seed ^= seed << 13n; seed &= 0xffffffffffffffffn;
  seed ^= seed >> 7n;
  seed ^= seed << 17n; seed &= 0xffffffffffffffffn;
  return Number(seed & 0x7fffffffn);
};

// ---- riscv encoders ----
const R = (op, f3, f7, rd, rs1, rs2) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | (rs2 << 20) | (f7 << 25);
const I = (op, f3, rd, rs1, imm) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | ((imm & 0xfff) << 20);
const U = (op, rd, imm20) => op | (rd << 7) | ((imm20 & 0xfffff) << 12);
const B = (f3, rs1, rs2, off) => {
  const i = off & 0x1fff;
  return 0x63 | (f3 << 12) | (rs1 << 15) | (rs2 << 20) |
    (((i >> 11) & 1) << 7) | (((i >> 1) & 0xf) << 8) |
    (((i >> 5) & 0x3f) << 25) | (((i >> 12) & 1) << 31);
};
const R4 = (op, f3, fmt, rd, rs1, rs2, rs3) =>
  op | (rd << 7) | (f3 << 12) | (rs1 << 15) | (rs2 << 20) | (fmt << 25) | (rs3 << 27);

// random instruction from the JIT-covered families; rd in x1..x30 (x31 is
// the loop counter), sources anywhere in x0..x30 / f0..f31
function randInsn() {
  const rd = 1 + (rnd() % 30);
  const rs1 = rnd() % 31;
  const rs2 = rnd() % 31;
  switch (rnd() % 16) {
    case 0: { // OP: add/sub/sll/slt/sltu/xor/srl/sra/or/and
      const f3 = rnd() % 8;
      const f7 = (f3 === 0 || f3 === 5) && rnd() % 2 ? 0x20 : 0x00;
      return R(0x33, f3, f7, rd, rs1, rs2);
    }
    case 1: { // M: mul/div/divu/rem/remu (skip mulh*)
      const f3 = [0, 4, 5, 6, 7][rnd() % 5];
      return R(0x33, f3, 0x01, rd, rs1, rs2);
    }
    case 2: { // OP-32
      const pick = [[0, 0], [0, 0x20], [1, 0], [5, 0], [5, 0x20], [0, 0x01],
                    [4, 0x01], [5, 0x01], [6, 0x01], [7, 0x01]][rnd() % 10];
      return R(0x3b, pick[0], pick[1], rd, rs1, rs2);
    }
    case 3: { // OP-IMM
      const f3 = rnd() % 8;
      if (f3 === 1) return I(0x13, 1, rd, rs1, rnd() % 64);
      if (f3 === 5) return I(0x13, 5, rd, rs1, (rnd() % 64) | (rnd() % 2 ? 0x400 : 0));
      return I(0x13, f3, rd, rs1, (rnd() % 4096) - 2048);
    }
    case 4: { // OP-IMM-32
      const f3 = [0, 1, 5][rnd() % 3];
      if (f3 === 0) return I(0x1b, 0, rd, rs1, (rnd() % 4096) - 2048);
      return I(0x1b, f3, rd, rs1, (rnd() % 32) | (f3 === 5 && rnd() % 2 ? 0x400 : 0));
    }
    case 5: // LUI / AUIPC
      return U(rnd() % 2 ? 0x37 : 0x17, rd, rnd());
    case 6: // FMV.D.X (seed FP from GPR)
      return R(0x53, 0, 0x79, rnd() % 32, rs1, 0);
    case 7: // FMV.X.D
      return R(0x53, 0, 0x71, rd, rnd() % 32, 0);
    case 8: { // FP arith D (rne)
      const f7 = [0x01, 0x05, 0x09, 0x0d][rnd() % 4]; // fadd/fsub/fmul/fdiv .d
      return R(0x53, 0, f7, rnd() % 32, rnd() % 32, rnd() % 32);
    }
    case 9: // FMADD.D family (rne)
      return R4([0x43, 0x47, 0x4b, 0x4f][rnd() % 4], 0, 1,
                rnd() % 32, rnd() % 32, rnd() % 32, rnd() % 32);
    case 10: { // FCVT: d.w/d.wu/d.l/d.lu (rne) or w.d (rtz)
      if (rnd() % 3 === 0) return R(0x53, 1, 0x61, rd, rnd() % 32, 0); // fcvt.w.d rtz
      return R(0x53, 0, 0x69, rnd() % 32, rs1, rnd() % 4); // fcvt.d.{w,wu,l,lu}
    }
    case 12: // FSGNJ/FSGNJN/FSGNJX.D (copysign / fneg / fabs / fmv.d)
      return R(0x53, rnd() % 3, 0x11, rnd() % 32, rnd() % 32, rnd() % 32);
    case 13: { // F extension: FADD/FSUB/FMUL/FDIV.S, FSGNJ.S, FSQRT.S
      const pick = rnd() % 3;
      if (pick === 0) {
        const f7 = [0x00, 0x04, 0x08, 0x0c][rnd() % 4]; // fmt=0 (single)
        return R(0x53, rnd() % 2 ? 0 : 7, f7, rnd() % 32, rnd() % 32, rnd() % 32);
      }
      if (pick === 1) return R(0x53, rnd() % 3, 0x10, rnd() % 32, rnd() % 32, rnd() % 32);
      return R(0x53, 0, 0x2c, rnd() % 32, rnd() % 32, 0); // fsqrt.s
    }
    case 14: { // F compares / moves / conversions
      const pick = rnd() % 5;
      if (pick === 0) return R(0x53, rnd() % 3, 0x50, rd, rnd() % 32, rnd() % 32); // fle/flt/feq.s
      if (pick === 1) return R(0x53, 0, 0x78, rnd() % 32, rs1, 0); // fmv.w.x
      if (pick === 2) return R(0x53, 0, 0x70, rd, rnd() % 32, 0); // fmv.x.w
      if (pick === 3) return R(0x53, 0, 0x68, rnd() % 32, rs1, rnd() % 4); // fcvt.s.{w,wu,l,lu}
      return R(0x53, 1, 0x60, rd, rnd() % 32, 0); // fcvt.w.s (rtz)
    }
    case 15: // FCVT.S.D / FCVT.D.S (narrowing rounds, widening exact)
      return rnd() % 2
        ? R(0x53, 0, 0x20, rnd() % 32, rnd() % 32, 1) // fcvt.s.d
        : R(0x53, 0, 0x21, rnd() % 32, rnd() % 32, 0); // fcvt.d.s
    default: // FP compare (writes GPR) or fsqrt
      return rnd() % 2
        ? R(0x53, rnd() % 3, 0x51, rd, rnd() % 32, rnd() % 32) // fle/flt/feq.d
        : R(0x53, 0, 0x2d, rnd() % 32, rnd() % 32, 0); // fsqrt.d
  }
}

function buildProgram(bodyLen) {
  const code = [];
  // seed registers with edge values: x1=0? (x0 is 0), x2=-1, x3=i64::MIN,
  // x4..x10 assorted, x31 = loop counter (100)
  code.push(I(0x13, 0, 2, 0, -1)); // x2 = -1
  code.push(I(0x13, 0, 3, 0, 1)); // x3 = 1
  code.push(I(0x13, 1, 3, 3, 63)); // x3 <<= 63  -> i64::MIN
  for (let r = 4; r <= 10; r++) code.push(I(0x13, 0, r, 0, (rnd() % 4096) - 2048));
  code.push(U(0x37, 11, rnd())); // x11 = lui random
  code.push(I(0x13, 0, 31, 0, 100)); // loop counter
  const loopStart = code.length;
  for (let i = 0; i < bodyLen; i++) code.push(randInsn());
  code.push(I(0x13, 0, 31, 31, -1)); // x31--
  const off = -((code.length - loopStart) * 4);
  code.push(B(1, 31, 0, off)); // bnez x31, loop
  // exit(0): a7=93, a0=0, ecall
  code.push(I(0x13, 0, 17, 0, 93));
  code.push(I(0x13, 0, 10, 0, 0));
  code.push(0x00000073);
  return code;
}

function synthElf(code) {
  const codeBytes = new Uint8Array(code.length * 4);
  const dv = new DataView(codeBytes.buffer);
  code.forEach((w, i) => dv.setUint32(i * 4, w >>> 0, true));
  const elf = new Uint8Array(0x1000 + codeBytes.length);
  const e = new DataView(elf.buffer);
  elf.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0], 0); // ident
  e.setUint16(0x10, 2, true); // ET_EXEC
  e.setUint16(0x12, 243, true); // EM_RISCV
  e.setUint32(0x14, 1, true);
  e.setBigUint64(0x18, 0x10000n, true); // entry
  e.setBigUint64(0x20, 64n, true); // phoff
  e.setUint16(0x34, 64, true); // ehsize
  e.setUint16(0x36, 56, true); // phentsize
  e.setUint16(0x38, 1, true); // phnum
  // phdr
  e.setUint32(64 + 0, 1, true); // PT_LOAD
  e.setUint32(64 + 4, 5, true); // R+X
  e.setBigUint64(64 + 8, 0x1000n, true); // offset
  e.setBigUint64(64 + 16, 0x10000n, true); // vaddr
  e.setBigUint64(64 + 24, 0x10000n, true); // paddr
  e.setBigUint64(64 + 32, BigInt(codeBytes.length), true); // filesz
  e.setBigUint64(64 + 40, BigInt(codeBytes.length), true); // memsz
  e.setBigUint64(64 + 48, 0x1000n, true); // align
  elf.set(codeBytes, 0x1000);
  return elf;
}

async function runOne(elf, jit) {
  const vm = await RV64.create(wasmBytes);
  vm.ex.jit_set_enabled(jit);
  vm.onWrite = () => {};
  if (!vm.loadElf(elf, ["diff"])) return null;
  const stop = vm.runUser(200_000_000n);
  const regs = [];
  for (let i = 1; i < 32; i++) regs.push(vm.ex.user_reg(i));
  const fregs = [];
  for (let i = 0; i < 32; i++) fregs.push(vm.ex.user_freg(i));
  return {
    stop,
    exit: vm.userExitCode(),
    pc: vm.ex.user_pc(),
    icount: vm.ex.user_insn_count(),
    fcsr: vm.ex.user_fcsr(),
    regs,
    fregs,
  };
}

let failures = 0;
for (let p = 0; p < N_PROGRAMS; p++) {
  const progSeed = seed;
  const elf = synthElf(buildProgram(12 + (rnd() % 28)));
  const a = await runOne(elf, 0);
  const b = await runOne(elf, 1);
  if (!a || !b) { console.log(`FAIL prog ${p}: load failed`); failures++; continue; }
  const diffs = [];
  if (a.stop !== b.stop) diffs.push(`stop ${a.stop}!=${b.stop}`);
  if (a.exit !== b.exit) diffs.push(`exit ${a.exit}!=${b.exit}`);
  if (a.pc !== b.pc) diffs.push(`pc ${a.pc}!=${b.pc}`);
  if (a.icount !== b.icount) diffs.push(`insn_count ${a.icount}!=${b.icount}`);
  if (a.fcsr !== b.fcsr) diffs.push(`fcsr ${a.fcsr}!=${b.fcsr}`);
  a.regs.forEach((v, i) => { if (v !== b.regs[i]) diffs.push(`x${i + 1} ${v}!=${b.regs[i]}`); });
  a.fregs.forEach((v, i) => { if (v !== b.fregs[i]) diffs.push(`f${i} ${v}!=${b.fregs[i]}`); });
  if (diffs.length) {
    console.log(`FAIL prog ${p} (seed ${progSeed}): ${diffs.slice(0, 6).join("; ")}`);
    failures++;
  }
}
console.log(
  failures === 0
    ? `JIT DIFFERENTIAL: ALL PASS (${N_PROGRAMS} programs, interp==jit full state)`
    : `JIT DIFFERENTIAL: ${failures} FAILURES`,
);
process.exit(failures === 0 ? 0 : 1);
