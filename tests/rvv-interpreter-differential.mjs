// Ratified RVV 1.0 interpreter differential against QEMU.
//
// This builds a deterministic synthetic RV64GCV Linux ELF directly, executes
// every base-V arithmetic/permutation encoding on both interpreters, and
// compares the resulting vector register file, scalar result registers, and
// vector/FP CSRs.  Tail-undisturbed/mask-undisturbed configurations make all
// ordinary result bytes deterministic; architecturally agnostic tail bits of
// mask destinations are excluded from comparison.

import { spawnSync } from "node:child_process";
import { chmod, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outputDirectory = join(
  root,
  "target",
  process.env.RVV_JIT_DIFFERENTIAL === "1" ? "rvv-jit-differential" : "rvv-differential",
);
const elfPath = join(outputDirectory, "rvv-interpreter-differential.rv64");
const qemu = process.env.QEMU_RISCV64 || "qemu-riscv64";
const rewrite = process.env.RV64_RUN || join(root, "target", "release", "rv64-run");
const jitDifferential = process.env.RVV_JIT_DIFFERENTIAL === "1";
const skipQemu = process.env.RVV_SKIP_QEMU === "1";
const qemuVersion = skipQemu
  ? ""
  : spawnSync(qemu, ["--version"], { encoding: "utf8" }).stdout.match(/version ([0-9.]+)/)?.[1] ?? "unknown";
// QEMU 8.2 fails to WARL-mask reserved bits on direct vxsat/vxrm writes. Keep
// its useful target-field oracle while newer QEMU versions retain the full
// VCSR alias comparison. Core unit tests independently prove that our writes
// preserve the other defined field and discard every reserved bit.
const qemu82VectorCsrCompatibility = qemuVersion.startsWith("8.2.");
const caseRepetitions = Number(process.env.RVV_CASE_REPETITIONS || (jitDifferential ? 128 : 1));
if (!Number.isSafeInteger(caseRepetitions) || caseRepetitions < 1 || caseRepetitions > 2047) {
  throw new Error("RVV_CASE_REPETITIONS must be an integer from 1 through 2047");
}

const BASE = 0x10000;
const SEED = 0x80000;
const MEMORY = 0x82000;
const INDICES = 0x82100;
const OUTPUT = 0x90000;
const HEADER_BYTES = 64;
const VECTOR_BYTES = 32 * 16;
const RECORD_BYTES = HEADER_BYTES + VECTOR_BYTES;

const X_ZERO = 0;
const X_SEED = 20;
const X_ADDRESS = 21;
const X_DUMP = 22;
const X_MEMORY = 23;
const X_SCALAR = 24;
const X_AVL = 25;
const X_RESULT = 26;
const X_OUTPUT = 27;
const X_CASE = 28;
const X_TMP = 29;

const CSR_FFLAGS = 0x001;
const CSR_FRM = 0x002;
const CSR_FCSR = 0x003;
const CSR_VSTART = 0x008;
const CSR_VCSR = 0x00f;
const CSR_VL = 0xc20;
const CSR_VTYPE = 0xc21;

const I = (opcode, funct3, rd, rs1, immediate) =>
  (opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) |
    ((immediate & 0xfff) << 20)) >>> 0;
const U = (opcode, rd, immediate20) =>
  (opcode | (rd << 7) | ((immediate20 & 0xfffff) << 12)) >>> 0;
const S = (opcode, funct3, rs1, rs2, immediate) => {
  const imm = immediate & 0xfff;
  return (opcode | ((imm & 0x1f) << 7) | (funct3 << 12) | (rs1 << 15) |
    (rs2 << 20) | ((imm >>> 5) << 25)) >>> 0;
};
const B = (funct3, rs1, rs2, offset) => {
  const immediate = offset & 0x1fff;
  return (0x63 | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) |
    (((immediate >>> 11) & 1) << 7) | (((immediate >>> 1) & 0xf) << 8) |
    (((immediate >>> 5) & 0x3f) << 25) | (((immediate >>> 12) & 1) << 31)) >>> 0;
};
const CSR = (csr, funct3, rd, rs1) =>
  (0x73 | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (csr << 20)) >>> 0;
const V = (funct6, vm, vs2, source, format, vd) =>
  (0x57 | (vd << 7) | (format << 12) | (source << 15) | (vs2 << 20) |
    ((vm ? 1 : 0) << 25) | (funct6 << 26)) >>> 0;
const VMEM = ({ load, width, nf = 1, mop = 0, vm = true, aux = 0, base, reg }) =>
  ((load ? 0x07 : 0x27) | (reg << 7) | (width << 12) | (base << 15) |
    (aux << 20) | ((vm ? 1 : 0) << 25) | (mop << 26) | ((nf - 1) << 29)) >>> 0;

const ADDI = (rd, rs1, immediate) => I(0x13, 0, rd, rs1, immediate);
const LD = (rd, rs1, immediate = 0) => I(0x03, 3, rd, rs1, immediate);
const FLW = (rd, rs1, immediate = 0) => I(0x07, 2, rd, rs1, immediate);
const FLD = (rd, rs1, immediate = 0) => I(0x07, 3, rd, rs1, immediate);
const SD = (rs2, rs1, immediate = 0) => S(0x23, 3, rs1, rs2, immediate);
const FSD = (rs2, rs1, immediate = 0) => S(0x27, 3, rs1, rs2, immediate);
const ECALL = 0x00000073;
const WIDTH_CODE = new Map([[8, 0], [16, 5], [32, 6], [64, 7]]);
const INDEX_ADDRESS = new Map([
  [8, INDICES], [16, INDICES + 0x40], [32, INDICES + 0x80], [64, INDICES + 0x100],
]);

const U64_MASK = 0xffff_ffff_ffff_ffffn;
const SEED_PROFILES = [
  {
    name: "mixed-special",
    random: 0x6d2b79f5,
    scalar: 0xffff_ffff_ffff_fffdn,
    fp32: 0x3fc0_0000,
    fp64: 0xc002_0000_0000_0000n,
    patch(seed, view) {
      const special = [
        0n, 1n, U64_MASK, 0x8000_0000_0000_0000n,
        0x7fff_ffff_ffff_ffffn, 0x3ff8_0000_0000_0000n,
        0xc002_0000_0000_0000n, 0x7ff0_0000_0000_0000n,
        0x7ff8_0000_0000_1234n, 0x7ff0_0000_0000_0001n,
      ];
      special.forEach((value, index) => view.setBigUint64(128 + index * 8, value, true));
    },
  },
  {
    name: "integer-boundaries",
    random: 0x243f6a88,
    scalar: 0x8000_0000_0000_0001n,
    fp32: 0x7f7f_ffff,
    fp64: 0x0010_0000_0000_0000n,
    patch(seed) {
      seed.set([
        0x00, 0x01, 0x02, 0x3f, 0x40, 0x7e, 0x7f, 0x80,
        0x81, 0xaa, 0xcc, 0xfe, 0xff, 0x55, 0x10, 0xf0,
      ], 128);
      seed.set([
        0xff, 0xfe, 0x80, 0x7f, 0x01, 0x00, 0x55, 0xaa,
        0x7e, 0x81, 0x03, 0xfd, 0x20, 0xe0, 0x40, 0xc0,
      ], 192);
    },
  },
  {
    name: "fp32-special",
    random: 0x13198a2e,
    scalar: 0xaaaa_5555_aaaa_5555n,
    fp32: 0xc020_0000,
    fp64: 0x3ff0_0000_0000_0000n,
    patch(_seed, view) {
      [0x0000_0000, 0x8000_0000, 0x3fc0_0000, 0x0000_0001]
        .forEach((value, index) => view.setUint32(128 + index * 4, value, true));
      [0x7f80_0000, 0xff7f_ffff, 0x7fc0_1234, 0x7f80_0001]
        .forEach((value, index) => view.setUint32(192 + index * 4, value, true));
    },
  },
  {
    name: "fp64-special",
    random: 0xa4093822,
    scalar: 0x5555_aaaa_5555_aaaan,
    fp32: 0x3f00_0000,
    fp64: 0xc004_0000_0000_0000n,
    patch(_seed, view) {
      [0x0000_0000_0000_0001n, 0x7fef_ffff_ffff_ffffn]
        .forEach((value, index) => view.setBigUint64(128 + index * 8, value, true));
      [0xfff0_0000_0000_0000n, 0x7ff0_0000_0000_0001n]
        .forEach((value, index) => view.setBigUint64(192 + index * 8, value, true));
    },
  },
  {
    name: "random-a",
    random: 0x082efa98,
    scalar: 0x0123_4567_89ab_cdefn,
    fp32: 0x3eaa_aaab,
    fp64: 0x3fd5_5555_5555_5555n,
  },
  {
    name: "random-b",
    random: 0x452821e6,
    scalar: 0xfedc_ba98_7654_3210n,
    fp32: 0xbeaa_aaab,
    fp64: 0xbfd5_5555_5555_5555n,
  },
];

function vtype(sew, lmul = 1) {
  const vsew = Math.log2(sew) - 3;
  const vlmul = new Map([[1 / 8, 5], [1 / 4, 6], [1 / 2, 7], [1, 0], [2, 1], [4, 2], [8, 3]]).get(lmul);
  if (vlmul === undefined || !Number.isInteger(vsew)) throw new Error("bad vtype");
  return (vsew << 3) | vlmul; // tu,mu
}

function vlmax(sew, lmul = 1) {
  return 128 * lmul / sew;
}

function loadImmediate(code, rd, value) {
  const high = Math.floor((value + 0x800) / 0x1000);
  const low = value - high * 0x1000;
  code.push(U(0x37, rd, high), ADDI(rd, rd, low));
}

const cases = [];
function addCase(name, instruction, options = {}) {
  const sew = options.sew ?? 32;
  const lmul = options.lmul ?? 1;
  const vl = options.vl ?? vlmax(sew, lmul);
  const vd = options.vd ?? 16;
  cases.push({ name, instruction, sew, lmul, vl, vd, ...options });
}

const widths = [8, 16, 32, 64];
const eachWidth = (name, make, options = {}) => {
  for (const sew of widths) addCase(`${name}/e${sew}`, make(sew), { sew, ...options });
};
const maskResult = { maskDestination: true };

// OPIVV/OPIVX/OPIVI single-width integer families.
for (const [name, funct6] of [
  ["vadd", 0x00], ["vand", 0x09], ["vor", 0x0a], ["vxor", 0x0b],
  ["vsll", 0x25], ["vsrl", 0x28], ["vsra", 0x29],
]) {
  eachWidth(`${name}.vv`, () => V(funct6, true, 8, 12, 0, 16));
  eachWidth(`${name}.vx`, () => V(funct6, true, 8, X_SCALAR, 4, 16));
  eachWidth(`${name}.vi`, () => V(funct6, true, 8, 7, 3, 16));
}
for (const [name, funct6] of [
  ["vsub", 0x02], ["vminu", 0x04], ["vmin", 0x05],
  ["vmaxu", 0x06], ["vmax", 0x07],
]) {
  eachWidth(`${name}.vv`, () => V(funct6, true, 8, 12, 0, 16));
  eachWidth(`${name}.vx`, () => V(funct6, true, 8, X_SCALAR, 4, 16));
}
eachWidth("vrsub.vx", () => V(0x03, true, 8, X_SCALAR, 4, 16));
eachWidth("vrsub.vi", () => V(0x03, true, 8, 7, 3, 16));

for (const [name, funct6, forms] of [
  ["vmseq", 0x18, [0, 4, 3]], ["vmsne", 0x19, [0, 4, 3]],
  ["vmsltu", 0x1a, [0, 4]], ["vmslt", 0x1b, [0, 4]],
  ["vmsleu", 0x1c, [0, 4, 3]], ["vmsle", 0x1d, [0, 4, 3]],
  ["vmsgtu", 0x1e, [4, 3]], ["vmsgt", 0x1f, [4, 3]],
]) {
  for (const format of forms) {
    const suffix = format === 0 ? "vv" : format === 4 ? "vx" : "vi";
    const source = format === 0 ? 12 : format === 4 ? X_SCALAR : 7;
    eachWidth(`${name}.${suffix}`, () => V(funct6, true, 8, source, format, 16), maskResult);
  }
}

// Carry and borrow. vm=0 consumes v0; mask-result vm=1 variants omit it.
for (const [suffix, format, source] of [["vvm", 0, 12], ["vxm", 4, X_SCALAR], ["vim", 3, 7]]) {
  eachWidth(`vadc.${suffix}`, () => V(0x10, false, 8, source, format, 16));
  eachWidth(`vmadc.${suffix}`, () => V(0x11, false, 8, source, format, 16), maskResult);
  eachWidth(`vmadc.${suffix.slice(0, 2)}`, () => V(0x11, true, 8, source, format, 16), maskResult);
}
for (const [suffix, format, source] of [["vvm", 0, 12], ["vxm", 4, X_SCALAR]]) {
  eachWidth(`vsbc.${suffix}`, () => V(0x12, false, 8, source, format, 16));
  eachWidth(`vmsbc.${suffix}`, () => V(0x13, false, 8, source, format, 16), maskResult);
  eachWidth(`vmsbc.${suffix.slice(0, 2)}`, () => V(0x13, true, 8, source, format, 16), maskResult);
}

// Saturating and fixed-point arithmetic.
for (const [name, funct6, immediate] of [
  ["vsaddu", 0x20, true], ["vsadd", 0x21, true],
  ["vssubu", 0x22, false], ["vssub", 0x23, false],
]) {
  eachWidth(`${name}.vv`, () => V(funct6, true, 8, 12, 0, 16));
  eachWidth(`${name}.vx`, () => V(funct6, true, 8, X_SCALAR, 4, 16));
  if (immediate) eachWidth(`${name}.vi`, () => V(funct6, true, 8, 7, 3, 16));
}
for (const [name, funct6] of [
  ["vaaddu", 0x08], ["vaadd", 0x09], ["vasubu", 0x0a], ["vasub", 0x0b],
]) {
  eachWidth(`${name}.vv`, () => V(funct6, true, 8, 12, 2, 16));
  eachWidth(`${name}.vx`, () => V(funct6, true, 8, X_SCALAR, 6, 16));
}
eachWidth("vsmul.vv", () => V(0x27, true, 8, 12, 0, 16));
eachWidth("vsmul.vx", () => V(0x27, true, 8, X_SCALAR, 4, 16));
for (const [name, funct6] of [["vssrl", 0x2a], ["vssra", 0x2b]]) {
  eachWidth(`${name}.vv`, () => V(funct6, true, 8, 12, 0, 16));
  eachWidth(`${name}.vx`, () => V(funct6, true, 8, X_SCALAR, 4, 16));
  eachWidth(`${name}.vi`, () => V(funct6, true, 8, 7, 3, 16));
}
for (const [name, funct6] of [
  ["vnsrl", 0x2c], ["vnsra", 0x2d], ["vnclipu", 0x2e], ["vnclip", 0x2f],
]) {
  for (const sew of [8, 16, 32]) {
    addCase(`${name}.wv/e${sew}`, V(funct6, true, 8, 12, 0, 16), { sew });
    addCase(`${name}.wx/e${sew}`, V(funct6, true, 8, X_SCALAR, 4, 16), { sew });
    addCase(`${name}.wi/e${sew}`, V(funct6, true, 8, 7, 3, 16), { sew });
  }
}

// Integer multiply/divide, multiply-add, and widening arithmetic.
for (const [name, funct6] of [
  ["vdivu", 0x20], ["vdiv", 0x21], ["vremu", 0x22], ["vrem", 0x23],
  ["vmulhu", 0x24], ["vmul", 0x25], ["vmulhsu", 0x26], ["vmulh", 0x27],
  ["vmadd", 0x29], ["vnmsub", 0x2b], ["vmacc", 0x2d], ["vnmsac", 0x2f],
]) {
  eachWidth(`${name}.vv`, () => V(funct6, true, 8, 12, 2, 16));
  eachWidth(`${name}.vx`, () => V(funct6, true, 8, X_SCALAR, 6, 16));
}
for (const [name, funct6] of [
  ["vwaddu", 0x30], ["vwadd", 0x31], ["vwsubu", 0x32], ["vwsub", 0x33],
  ["vwaddu.w", 0x34], ["vwadd.w", 0x35], ["vwsubu.w", 0x36], ["vwsub.w", 0x37],
  ["vwmulu", 0x38], ["vwmulsu", 0x3a], ["vwmul", 0x3b],
  ["vwmaccu", 0x3c], ["vwmacc", 0x3d], ["vwmaccsu", 0x3f],
]) {
  for (const sew of [8, 16, 32]) {
    const vectorSuffix = name.endsWith(".w") ? "v" : ".vv";
    const scalarSuffix = name.endsWith(".w") ? "x" : ".vx";
    addCase(`${name}${vectorSuffix}/e${sew}`, V(funct6, true, 8, 12, 2, 16), { sew });
    addCase(`${name}${scalarSuffix}/e${sew}`, V(funct6, true, 8, X_SCALAR, 6, 16), { sew });
  }
}
for (const sew of [8, 16, 32]) {
  addCase(`vwmaccus.vx/e${sew}`, V(0x3e, true, 8, X_SCALAR, 6, 16), { sew });
}

// Moves, extensions, reductions, masks, and permutations.
eachWidth("vmv.v.v", () => V(0x17, true, 0, 12, 0, 16));
eachWidth("vmv.v.x", () => V(0x17, true, 0, X_SCALAR, 4, 16));
eachWidth("vmv.v.i", () => V(0x17, true, 0, 7, 3, 16));
eachWidth("vmerge.vvm", () => V(0x17, false, 8, 12, 0, 16));
eachWidth("vmerge.vxm", () => V(0x17, false, 8, X_SCALAR, 4, 16));
eachWidth("vmerge.vim", () => V(0x17, false, 8, 7, 3, 16));
eachWidth("vmv.x.s", () => V(0x10, true, 8, 0, 2, X_RESULT), { vd: 0 });
eachWidth("vmv.s.x", () => V(0x10, true, 0, X_SCALAR, 6, 16));
for (const [name, selector, divisor] of [
  ["vzext.vf2", 6, 2], ["vsext.vf2", 7, 2],
  ["vzext.vf4", 4, 4], ["vsext.vf4", 5, 4],
  ["vzext.vf8", 2, 8], ["vsext.vf8", 3, 8],
]) {
  for (const sew of widths.filter((width) => width / divisor >= 8)) {
    addCase(`${name}/e${sew}`, V(0x12, true, 8, selector, 2, 16), { sew });
  }
}
for (const [name, funct6] of [
  ["vredsum", 0x00], ["vredand", 0x01], ["vredor", 0x02], ["vredxor", 0x03],
  ["vredminu", 0x04], ["vredmin", 0x05], ["vredmaxu", 0x06], ["vredmax", 0x07],
]) eachWidth(`${name}.vs`, () => V(funct6, true, 8, 12, 2, 16));
for (const sew of [8, 16, 32]) {
  addCase(`vwredsumu.vs/e${sew}`, V(0x30, true, 8, 12, 0, 16), { sew });
  addCase(`vwredsum.vs/e${sew}`, V(0x31, true, 8, 12, 0, 16), { sew });
}
for (const [name, funct6] of [
  ["vmand", 0x19], ["vmandn", 0x18], ["vmor", 0x1a], ["vmxor", 0x1b],
  ["vmorn", 0x1c], ["vmnand", 0x1d], ["vmnor", 0x1e], ["vmxnor", 0x1f],
]) addCase(`${name}.mm`, V(funct6, true, 8, 12, 2, 16), { sew: 8, maskDestination: true });
addCase("vcpop.m", V(0x10, true, 8, 0x10, 2, X_RESULT), { sew: 8, vd: 0 });
addCase("vcpop.m/masked", V(0x10, false, 8, 0x10, 2, X_RESULT), { sew: 8, vd: 0 });
addCase("vfirst.m", V(0x10, true, 8, 0x11, 2, X_RESULT), { sew: 8, vd: 0 });
addCase("vfirst.m/masked", V(0x10, false, 8, 0x11, 2, X_RESULT), { sew: 8, vd: 0 });
for (const [name, selector] of [["vmsbf", 1], ["vmsof", 2], ["vmsif", 3]]) {
  addCase(`${name}.m`, V(0x14, true, 8, selector, 2, 16), { sew: 8, maskDestination: true });
  addCase(`${name}.m/masked`, V(0x14, false, 8, selector, 2, 16), { sew: 8, maskDestination: true });
}
addCase("viota.m", V(0x14, true, 8, 0x10, 2, 16), { sew: 8 });
addCase("viota.m/masked", V(0x14, false, 8, 0x10, 2, 16), { sew: 8 });
eachWidth("vid.v", () => V(0x14, true, 0, 0x11, 2, 16));

for (const sew of widths) {
  addCase(`vrgather.vv/e${sew}`, V(0x0c, true, 8, 12, 0, 16), { sew });
  addCase(`vrgather.vx/e${sew}`, V(0x0c, true, 8, X_SCALAR, 4, 16), { sew, scalar: 1 });
  addCase(`vrgather.vi/e${sew}`, V(0x0c, true, 8, 1, 3, 16), { sew });
  addCase(`vslideup.vx/e${sew}`, V(0x0e, true, 8, X_SCALAR, 4, 16), { sew, scalar: 1 });
  addCase(`vslideup.vi/e${sew}`, V(0x0e, true, 8, 1, 3, 16), { sew });
  addCase(`vslidedown.vx/e${sew}`, V(0x0f, true, 8, X_SCALAR, 4, 16), { sew, scalar: 1 });
  addCase(`vslidedown.vi/e${sew}`, V(0x0f, true, 8, 1, 3, 16), { sew });
  addCase(`vslide1up.vx/e${sew}`, V(0x0e, true, 8, X_SCALAR, 6, 16), { sew });
  addCase(`vslide1down.vx/e${sew}`, V(0x0f, true, 8, X_SCALAR, 6, 16), { sew });
  addCase(`vcompress.vm/e${sew}`, V(0x17, true, 8, 12, 2, 16), { sew });
}
for (const sew of widths) {
  addCase(`vrgatherei16.vv/e${sew}`, V(0x0e, true, 8, 12, 0, 16), { sew });
}
for (const [count, selector] of [[1, 0], [2, 1], [4, 3], [8, 7]]) {
  addCase(`vmv${count}r.v`, V(0x27, true, 8, selector, 3, 16), { sew: 8 });
}

// Single-width and widening floating-point families supplied by G (f32/f64).
for (const [name, funct6] of [
  ["vfadd", 0x00], ["vfsub", 0x02], ["vfmin", 0x04], ["vfmax", 0x06],
  ["vfsgnj", 0x08], ["vfsgnjn", 0x09], ["vfsgnjx", 0x0a],
  ["vfdiv", 0x20], ["vfmul", 0x24],
]) {
  for (const sew of [32, 64]) {
    addCase(`${name}.vv/e${sew}`, V(funct6, true, 8, 12, 1, 16), { sew, fp: true });
    addCase(`${name}.vf/e${sew}`, V(funct6, true, 8, 4, 5, 16), { sew, fp: true });
  }
}
for (const [name, funct6] of [["vfrdiv", 0x21], ["vfrsub", 0x27]]) {
  for (const sew of [32, 64]) addCase(`${name}.vf/e${sew}`, V(funct6, true, 8, 4, 5, 16), { sew, fp: true });
}
addCase("vfadd.vf/unboxed-f32-scalar", V(0x00, true, 8, 4, 5, 16), {
  sew: 32, fp: true, unboxedScalar: true,
});
for (const [name, funct6] of [
  ["vmfeq", 0x18], ["vmfle", 0x19], ["vmflt", 0x1b], ["vmfne", 0x1c],
]) {
  for (const sew of [32, 64]) {
    addCase(`${name}.vv/e${sew}`, V(funct6, true, 8, 12, 1, 16), { sew, fp: true, maskDestination: true });
    addCase(`${name}.vf/e${sew}`, V(funct6, true, 8, 4, 5, 16), { sew, fp: true, maskDestination: true });
  }
}
for (const [name, funct6] of [["vmfgt", 0x1d], ["vmfge", 0x1f]]) {
  for (const sew of [32, 64]) addCase(`${name}.vf/e${sew}`, V(funct6, true, 8, 4, 5, 16), { sew, fp: true, maskDestination: true });
}
for (const [name, funct6] of [
  ["vfmadd", 0x28], ["vfnmadd", 0x29], ["vfmsub", 0x2a], ["vfnmsub", 0x2b],
  ["vfmacc", 0x2c], ["vfnmacc", 0x2d], ["vfmsac", 0x2e], ["vfnmsac", 0x2f],
]) {
  for (const sew of [32, 64]) {
    addCase(`${name}.vv/e${sew}`, V(funct6, true, 8, 12, 1, 16), { sew, fp: true });
    addCase(`${name}.vf/e${sew}`, V(funct6, true, 8, 4, 5, 16), { sew, fp: true });
  }
}
for (const [name, selector] of [["vfsqrt", 0], ["vfrsqrt7", 4], ["vfrec7", 5], ["vfclass", 0x10]]) {
  for (const sew of [32, 64]) addCase(`${name}.v/e${sew}`, V(0x13, true, 8, selector, 1, 16), { sew, fp: true });
}
for (const [name, funct6] of [
  ["vfredusum", 0x01], ["vfredosum", 0x03], ["vfredmin", 0x05], ["vfredmax", 0x07],
]) {
  for (const sew of [32, 64]) addCase(`${name}.vs/e${sew}`, V(funct6, true, 8, 12, 1, 16), { sew, fp: true });
}
for (const sew of [32, 64]) {
  addCase(`vfmerge.vfm/e${sew}`, V(0x17, false, 8, 4, 5, 16), { sew, fp: true });
  addCase(`vfmv.v.f/e${sew}`, V(0x17, true, 0, 4, 5, 16), { sew, fp: true });
  addCase(`vfmv.f.s/e${sew}`, V(0x10, true, 8, 0, 1, 5), { sew, fp: true, vd: 0 });
  addCase(`vfmv.s.f/e${sew}`, V(0x10, true, 0, 4, 5, 16), { sew, fp: true });
  addCase(`vfslide1up.vf/e${sew}`, V(0x0e, true, 8, 4, 5, 16), { sew, fp: true });
  addCase(`vfslide1down.vf/e${sew}`, V(0x0f, true, 8, 4, 5, 16), { sew, fp: true });
}

for (const [name, selector] of [
  ["vfcvt.xu.f.v", 0x00], ["vfcvt.x.f.v", 0x01],
  ["vfcvt.f.xu.v", 0x02], ["vfcvt.f.x.v", 0x03],
  ["vfcvt.rtz.xu.f.v", 0x06], ["vfcvt.rtz.x.f.v", 0x07],
]) {
  for (const sew of [32, 64]) addCase(`${name}/e${sew}`, V(0x12, true, 8, selector, 1, 16), { sew, fp: true });
}
for (const [name, selector, sews] of [
  ["vfwcvt.xu.f.v", 0x08, [32]], ["vfwcvt.x.f.v", 0x09, [32]],
  ["vfwcvt.f.xu.v", 0x0a, [16, 32]], ["vfwcvt.f.x.v", 0x0b, [16, 32]],
  ["vfwcvt.f.f.v", 0x0c, [32]], ["vfwcvt.rtz.xu.f.v", 0x0e, [32]],
  ["vfwcvt.rtz.x.f.v", 0x0f, [32]],
  ["vfncvt.xu.f.w", 0x10, [16, 32]], ["vfncvt.x.f.w", 0x11, [16, 32]],
  ["vfncvt.f.xu.w", 0x12, [32]], ["vfncvt.f.x.w", 0x13, [32]],
  ["vfncvt.f.f.w", 0x14, [32]], ["vfncvt.rod.f.f.w", 0x15, [32]],
  ["vfncvt.rtz.xu.f.w", 0x16, [16, 32]], ["vfncvt.rtz.x.f.w", 0x17, [16, 32]],
]) {
  for (const sew of sews) addCase(`${name}/e${sew}`, V(0x12, true, 8, selector, 1, 16), { sew, fp: true });
}
for (const [name, funct6] of [
  ["vfwadd", 0x30], ["vfwsub", 0x32], ["vfwadd.w", 0x34],
  ["vfwsub.w", 0x36], ["vfwmul", 0x38],
]) {
  const vectorSuffix = name.endsWith(".w") ? "v" : ".vv";
  const scalarSuffix = name.endsWith(".w") ? "f" : ".vf";
  addCase(`${name}${vectorSuffix}`, V(funct6, true, 8, 12, 1, 16), { sew: 32, fp: true });
  addCase(`${name}${scalarSuffix}`, V(funct6, true, 8, 4, 5, 16), { sew: 32, fp: true });
}
for (const [name, funct6] of [
  ["vfwmacc", 0x3c], ["vfwnmacc", 0x3d], ["vfwmsac", 0x3e], ["vfwnmsac", 0x3f],
]) {
  addCase(`${name}.vv`, V(funct6, true, 8, 12, 1, 16), { sew: 32, fp: true });
  addCase(`${name}.vf`, V(funct6, true, 8, 4, 5, 16), { sew: 32, fp: true });
}
addCase("vfwredusum.vs", V(0x31, true, 8, 12, 1, 16), { sew: 32, fp: true });
addCase("vfwredosum.vs", V(0x33, true, 8, 12, 1, 16), { sew: 32, fp: true });

// Every RVV memory addressing family. Segment fields are part of the same
// instruction encoding, so exercise both nf=2 and each width's largest legal
// field count in addition to the single-field forms.
function addMemoryCase(name, options) {
  const {
    load,
    memoryEew,
    sew = 32,
    lmul = 1,
    vl = Math.min(4, vlmax(sew, lmul)),
    nf = 1,
    mop = 0,
    vm = true,
    aux = 0,
    reg = 16,
    indexEew,
    scalar,
    baseOffset = 0,
    vstart,
  } = options;
  addCase(
    name,
    VMEM({ load, width: WIDTH_CODE.get(memoryEew), nf, mop, vm, aux, base: X_MEMORY, reg }),
    {
      sew, lmul, vl, memory: true, captureMemory: !load, indexEew,
      scalar, baseOffset, vstart,
    },
  );
}

for (const eew of widths) {
  addMemoryCase(`vle${eew}.v`, { load: true, memoryEew: eew });
  addMemoryCase(`vse${eew}.v`, { load: false, memoryEew: eew });
  addMemoryCase(`vle${eew}ff.v/nonfault`, { load: true, memoryEew: eew, aux: 0x10 });
  addMemoryCase(`vlse${eew}.v`, { load: true, memoryEew: eew, mop: 2, aux: X_SCALAR, scalar: 16 });
  addMemoryCase(`vsse${eew}.v`, { load: false, memoryEew: eew, mop: 2, aux: X_SCALAR, scalar: 16 });

  const maxFields = eew === 64 ? 4 : 8;
  for (const nf of [2, maxFields].filter((value, index, array) => array.indexOf(value) === index)) {
    addMemoryCase(`vlseg${nf}e${eew}.v`, { load: true, memoryEew: eew, nf });
    addMemoryCase(`vlseg${nf}e${eew}ff.v/nonfault`, {
      load: true, memoryEew: eew, nf, aux: 0x10,
    });
    addMemoryCase(`vsseg${nf}e${eew}.v`, { load: false, memoryEew: eew, nf });
  }
}

// Predicate consumption, negative strides, and segmented strided transfers.
addMemoryCase("vle32.v/masked", { load: true, memoryEew: 32, vm: false });
addMemoryCase("vse32.v/masked", { load: false, memoryEew: 32, vm: false });
addMemoryCase("vlse16.v/masked", { load: true, memoryEew: 16, mop: 2, aux: X_SCALAR, scalar: 16, vm: false });
addMemoryCase("vsse16.v/masked", { load: false, memoryEew: 16, mop: 2, aux: X_SCALAR, scalar: 16, vm: false });
addMemoryCase("vlse32.v/negative", { load: true, memoryEew: 32, mop: 2, aux: X_SCALAR, scalar: -8, baseOffset: 64 });
addMemoryCase("vsse32.v/negative", { load: false, memoryEew: 32, mop: 2, aux: X_SCALAR, scalar: -8, baseOffset: 64 });
for (const eew of widths) {
  const maxFields = eew === 64 ? 4 : 8;
  for (const nf of [2, maxFields].filter((value, index, array) => array.indexOf(value) === index)) {
    addMemoryCase(`vlsseg${nf}e${eew}.v`, {
      load: true, memoryEew: eew, nf, mop: 2, aux: X_SCALAR, scalar: 32,
    });
    addMemoryCase(`vssseg${nf}e${eew}.v`, {
      load: false, memoryEew: eew, nf, mop: 2, aux: X_SCALAR, scalar: 32,
    });
  }
}

for (const indexEew of widths) {
  for (const [order, mop] of [["unordered", 1], ["ordered", 3]]) {
    addMemoryCase(`vluxei${indexEew}.v/${order}`, {
      load: true, memoryEew: indexEew, mop, aux: 12, indexEew,
    });
    addMemoryCase(`vsuxei${indexEew}.v/${order}`, {
      load: false, memoryEew: indexEew, mop, aux: 12, indexEew,
    });
    for (const nf of [2, 8]) {
      addMemoryCase(`vl${order === "ordered" ? "ox" : "ux"}seg${nf}ei${indexEew}.v`, {
        load: true, memoryEew: indexEew, nf, mop, aux: 12, indexEew,
      });
      addMemoryCase(`vs${order === "ordered" ? "ox" : "ux"}seg${nf}ei${indexEew}.v`, {
        load: false, memoryEew: indexEew, nf, mop, aux: 12, indexEew,
      });
    }
  }
}
addMemoryCase("vluxei16.v/masked", { load: true, memoryEew: 16, mop: 1, aux: 12, indexEew: 16, vm: false });
addMemoryCase("vsuxei16.v/masked", { load: false, memoryEew: 16, mop: 1, aux: 12, indexEew: 16, vm: false });

addMemoryCase("vlm.v", { load: true, memoryEew: 8, sew: 8, vl: 13, aux: 0x0b });
addMemoryCase("vsm.v", { load: false, memoryEew: 8, sew: 8, vl: 13, aux: 0x0b });
for (const count of [1, 2, 4, 8]) {
  for (const eew of widths) {
    addMemoryCase(`vl${count}re${eew}.v`, {
      load: true, memoryEew: eew, nf: count, aux: 8,
    });
  }
  addMemoryCase(`vs${count}r.v`, {
    load: false, memoryEew: 8, nf: count, aux: 8,
  });
}

const VSETVLI = (d, s1, rawType) =>
  (0x57 | (d << 7) | (7 << 12) | (s1 << 15) | (rawType << 20)) >>> 0;
const VSETIVLI = (d, immediate, rawType) =>
  (0xc0000057 | (d << 7) | (7 << 12) | (immediate << 15) | (rawType << 20)) >>> 0;
const VSETVL = (d, s1, s2) =>
  (0x80000057 | (d << 7) | (7 << 12) | (s1 << 15) | (s2 << 20)) >>> 0;

// Exercise every legal SEW/LMUL configuration at VLEN=128. The opcode
// inventory above uses LMUL=1 for compactness; this matrix verifies register
// grouping, fractional groups, maximum-length groups, and widening/narrowing
// at both EMUL boundaries.
const lmulNames = new Map([
  [1 / 8, "mf8"], [1 / 4, "mf4"], [1 / 2, "mf2"],
  [1, "m1"], [2, "m2"], [4, "m4"], [8, "m8"],
]);
const legalConfigurations = [];
for (const sew of widths) {
  for (const lmul of lmulNames.keys()) {
    const maximum = vlmax(sew, lmul);
    if (Number.isInteger(maximum) && maximum >= 1 && sew <= 64 * lmul) {
      legalConfigurations.push({ sew, lmul, maximum, label: `e${sew},${lmulNames.get(lmul)}` });
    }
  }
}
for (const { sew, lmul, maximum, label } of legalConfigurations) {
  addCase(`vsetivli/${label}`, VSETIVLI(X_RESULT, 31, vtype(sew, lmul)), {
    sew: 8, vl: 1,
  });
  addCase(`vadd.vv/vtype-${label}`, V(0x00, true, 0, 8, 0, 16), {
    sew, lmul, vl: maximum,
  });
  addCase(`vredsum.vs/vtype-${label}`, V(0x00, true, 0, 12, 2, 16), {
    sew, lmul, vl: maximum,
  });
  addCase(`vslidedown.vi/vtype-${label}`, V(0x0f, true, 0, 1, 3, 16), {
    sew, lmul, vl: maximum,
  });
  addMemoryCase(`vle${sew}.v/vtype-${label}`, {
    load: true, memoryEew: sew, sew, lmul, vl: maximum,
  });
  addMemoryCase(`vse${sew}.v/vtype-${label}`, {
    load: false, memoryEew: sew, sew, lmul, vl: maximum,
  });
  if (sew === 32 || sew === 64) {
    addCase(`vfadd.vv/vtype-${label}`, V(0x00, true, 0, 8, 1, 16), {
      sew, lmul, vl: maximum, fp: true,
    });
  }
  if (sew < 64 && lmul <= 4) {
    addCase(`vwaddu.vv/vtype-${label}`, V(0x30, true, 0, 8, 2, 16), {
      sew, lmul, vl: maximum,
    });
    addCase(`vnsrl.wi/vtype-${label}`, V(0x2c, true, 0, 3, 3, 16), {
      sew, lmul, vl: maximum,
    });
    if (sew === 32) {
      addCase(`vfwadd.vv/vtype-${label}`, V(0x30, true, 0, 8, 1, 16), {
        sew, lmul, vl: maximum, fp: true,
      });
    }
  }
}

for (const [name, csr] of [
  ["vstart", CSR_VSTART], ["vxsat", 0x009], ["vxrm", 0x00a], ["vcsr", CSR_VCSR],
]) {
  addCase(`csrrw/${name}`, CSR(csr, 1, X_RESULT, X_SCALAR), { sew: 8, vl: 16 });
}
for (const [name, csr] of [
  ["vstart", CSR_VSTART], ["vxsat", 0x009], ["vxrm", 0x00a], ["vcsr", CSR_VCSR],
  ["vl", CSR_VL], ["vtype", CSR_VTYPE], ["vlenb", 0xc22],
]) {
  addCase(`csrr/${name}`, CSR(csr, 2, X_RESULT, X_ZERO), { sew: 16, lmul: 2, vl: 7 });
}

addCase("vsetvli/normal", VSETVLI(X_RESULT, X_AVL, vtype(16, 2)), { sew: 32, vl: 4 });
addCase("vsetvli/vlmax", VSETVLI(X_RESULT, X_ZERO, vtype(8, 2)), { sew: 32, vl: 4 });
addCase("vsetivli", VSETIVLI(X_RESULT, 31, vtype(8, 2)), { sew: 32, vl: 4 });
addCase("vsetvl", VSETVL(X_RESULT, X_AVL, X_SCALAR), { sew: 32, vl: 4, scalar: vtype(64, 1) });
addCase("vsetvli/retain-vl", VSETVLI(X_ZERO, X_ZERO, vtype(16, 2)), { sew: 32, lmul: 4, vl: 7 });
addCase("vsetvli/illegal-vtype", VSETVLI(X_RESULT, X_AVL, 4), { sew: 32, vl: 4 });

// Architecturally legal mixed-width in-place forms. These are deliberately
// full-length and unmasked because mixed-width overlap makes inactive and
// tail elements agnostic even under tu,mu.
addCase("vwaddu.vv/high-overlap", V(0x30, true, 17, 12, 2, 16), { sew: 8, vl: 16 });
addCase("vwadd.wv/in-place", V(0x35, true, 16, 12, 2, 16), { sew: 8, vl: 16 });
addCase("vzext.vf2/high-overlap", V(0x12, true, 17, 6, 2, 16), { sew: 16, lmul: 2, vl: 16 });
addCase("vnsrl.wi/low-overlap", V(0x2c, true, 16, 3, 3, 16), { sew: 8, vl: 16 });
addCase("vfwadd.vv/high-overlap", V(0x30, true, 17, 12, 1, 16), { sew: 32, fp: true, vl: 4 });
addCase("vfwadd.wv/in-place", V(0x34, true, 16, 12, 1, 16), { sew: 32, fp: true, vl: 4 });
addCase("vfwcvt.f.f.v/high-overlap", V(0x12, true, 17, 0x0c, 1, 16), { sew: 32, fp: true, vl: 4 });
addCase("vfncvt.f.f.w/low-overlap", V(0x12, true, 16, 0x14, 1, 16), { sew: 32, fp: true, vl: 4 });

// Equal-width operands can overlap freely. Include the accumulator,
// permutation, merge, and scalar-reduction shapes whose read-before-write
// behavior differs from a simple binary operation.
for (const [name, instruction, options] of [
  ["vadd.vv/dest-vs2", V(0x00, true, 8, 12, 0, 8), { sew: 16, vd: 8 }],
  ["vadd.vv/dest-vs1", V(0x00, true, 8, 12, 0, 12), { sew: 16, vd: 12 }],
  ["vand.vv/dest-vs2-fastpath", V(0x09, true, 8, 12, 0, 8), { sew: 8, vd: 8 }],
  ["vmacc.vv/dest-vs2", V(0x2d, true, 8, 12, 2, 8), { sew: 16, vd: 8 }],
  ["vmacc.vv/dest-vs1", V(0x2d, true, 8, 12, 2, 12), { sew: 16, vd: 12 }],
  ["vmerge.vvm/dest-vs2", V(0x17, false, 8, 12, 0, 8), { sew: 16, vd: 8 }],
  ["vmerge.vvm/dest-vs1", V(0x17, false, 8, 12, 0, 12), { sew: 16, vd: 12 }],
  ["vslidedown.vi/in-place", V(0x0f, true, 8, 1, 3, 8), { sew: 16, vd: 8 }],
  ["vslide1down.vx/in-place", V(0x0f, true, 8, X_SCALAR, 6, 8), { sew: 16, vd: 8 }],
  ["vredsum.vs/dest-vs2", V(0x00, true, 8, 12, 2, 8), { sew: 16, vd: 8 }],
  ["vredsum.vs/dest-v0-masked", V(0x00, false, 8, 12, 2, 0), { sew: 16, vd: 0 }],
  ["vwredsumu.vs/dest-vs2", V(0x30, true, 8, 12, 0, 8), { sew: 16, vd: 8 }],
  ["vfadd.vv/dest-vs2", V(0x00, true, 8, 12, 1, 8), { sew: 32, vd: 8, fp: true }],
  ["vfmacc.vv/dest-vs2", V(0x2c, true, 8, 12, 1, 8), { sew: 32, vd: 8, fp: true }],
  ["vfmacc.vv/dest-vs1", V(0x2c, true, 8, 12, 1, 12), { sew: 32, vd: 12, fp: true }],
  ["vfslide1down.vf/in-place", V(0x0f, true, 8, 4, 5, 8), { sew: 32, vd: 8, fp: true }],
  ["vfredosum.vs/dest-vs2", V(0x03, true, 8, 12, 1, 8), { sew: 32, vd: 8, fp: true }],
]) addCase(name, instruction, options);

// Mask destinations may legally share the low register of a data source, and
// a masked compare may write v0 while consuming the old v0 predicate bits.
addCase("vmseq.vv/dest-source-overlap", V(0x18, true, 8, 12, 0, 8), {
  sew: 8, vd: 8, maskDestination: true,
});
addCase("vmseq.vv/dest-v0-masked", V(0x18, false, 8, 12, 0, 0), {
  sew: 8, vd: 0, maskDestination: true,
});
addCase("vmadc.vvm/dest-source-overlap", V(0x11, true, 8, 12, 0, 8), {
  sew: 8, vd: 8, maskDestination: true,
});

// All four fixed-point rounding modes across every rounded family.
for (let vxrm = 0; vxrm < 4; vxrm++) {
  for (const [name, instruction, sew] of [
    ["vaaddu.vv", V(0x08, true, 8, 12, 2, 16), 16],
    ["vaadd.vv", V(0x09, true, 8, 12, 2, 16), 16],
    ["vasubu.vv", V(0x0a, true, 8, 12, 2, 16), 16],
    ["vasub.vv", V(0x0b, true, 8, 12, 2, 16), 16],
    ["vssrl.vv", V(0x2a, true, 8, 12, 0, 16), 16],
    ["vssra.vv", V(0x2b, true, 8, 12, 0, 16), 16],
    ["vsmul.vv", V(0x27, true, 8, 12, 0, 16), 16],
    ["vnclipu.wi", V(0x2e, true, 8, 3, 3, 16), 16],
    ["vnclip.wi", V(0x2f, true, 8, 3, 3, 16), 16],
  ]) addCase(`${name}/vxrm${vxrm}`, instruction, { sew, vxrm });
}

// IEEE rounding-mode coverage for arithmetic, FMA, square root, conversion,
// and widening/narrowing. RMM is valid for arithmetic even though compiler
// output most often leaves frm at RNE.
for (let roundingMode = 0; roundingMode <= 4; roundingMode++) {
  for (const [name, instruction, sew] of [
    ["vfadd.vv", V(0x00, true, 8, 12, 1, 16), 32],
    ["vfmul.vv", V(0x24, true, 8, 12, 1, 16), 32],
    ["vfdiv.vv", V(0x20, true, 8, 12, 1, 16), 32],
    ["vfmacc.vv", V(0x2c, true, 8, 12, 1, 16), 32],
    ["vfsqrt.v", V(0x13, true, 8, 0, 1, 16), 32],
    ["vfcvt.x.f.v", V(0x12, true, 8, 1, 1, 16), 32],
    ["vfwadd.vv", V(0x30, true, 8, 12, 1, 16), 32],
    ["vfncvt.f.f.w", V(0x12, true, 8, 0x14, 1, 16), 32],
  ]) addCase(`${name}/frm${roundingMode}`, instruction, { sew, fp: true, roundingMode });
}

// Masked execution and non-zero restart points traverse the custom paths that
// cannot be inferred solely from the unmasked opcode inventory.
for (const [name, instruction, options] of [
  ["vadd.vv/masked", V(0x00, false, 8, 12, 0, 16), { sew: 16 }],
  ["vwaddu.vv/masked", V(0x30, false, 8, 12, 2, 16), { sew: 16 }],
  ["vnsrl.wv/masked", V(0x2c, false, 8, 12, 0, 16), { sew: 16 }],
  ["vsmul.vv/masked", V(0x27, false, 8, 12, 0, 16), { sew: 16 }],
  ["vrgather.vv/masked", V(0x0c, false, 8, 12, 0, 16), { sew: 16 }],
  ["vslide1down.vx/masked", V(0x0f, false, 8, X_SCALAR, 6, 16), { sew: 16 }],
  ["vfadd.vv/masked", V(0x00, false, 8, 12, 1, 16), { sew: 32, fp: true }],
  ["vfsqrt.v/masked", V(0x13, false, 8, 0, 1, 16), { sew: 32, fp: true }],
  ["vfcvt.x.f.v/masked", V(0x12, false, 8, 1, 1, 16), { sew: 32, fp: true }],
  ["vfwadd.vv/masked", V(0x30, false, 8, 12, 1, 16), { sew: 32, fp: true }],
]) addCase(name, instruction, options);

for (const [name, instruction, options] of [
  ["vadd.vv/vstart", V(0x00, true, 8, 12, 0, 16), { sew: 16 }],
  ["vwaddu.vv/vstart", V(0x30, true, 8, 12, 2, 16), { sew: 16 }],
  ["vnsrl.wv/vstart", V(0x2c, true, 8, 12, 0, 16), { sew: 16 }],
  ["vmacc.vv/vstart", V(0x2d, true, 8, 12, 2, 16), { sew: 16 }],
  ["vmand.mm/vstart", V(0x19, true, 8, 12, 2, 16), { sew: 8, maskDestination: true }],
  ["vrgather.vv/vstart", V(0x0c, true, 8, 12, 0, 16), { sew: 16 }],
  ["vslide1down.vx/vstart", V(0x0f, true, 8, X_SCALAR, 6, 16), { sew: 16 }],
  ["vfadd.vv/vstart", V(0x00, true, 8, 12, 1, 16), { sew: 32, fp: true }],
  ["vfsqrt.v/vstart", V(0x13, true, 8, 0, 1, 16), { sew: 32, fp: true }],
  ["vfcvt.x.f.v/vstart", V(0x12, true, 8, 1, 1, 16), { sew: 32, fp: true }],
  ["vfwadd.vv/vstart", V(0x30, true, 8, 12, 1, 16), { sew: 32, fp: true }],
]) addCase(name, instruction, { ...options, vstart: 1 });

addMemoryCase("vle32.v/vstart", { load: true, memoryEew: 32, vstart: 1 });
addMemoryCase("vse32.v/vstart", { load: false, memoryEew: 32, vstart: 1 });
addMemoryCase("vlse16.v/vstart", { load: true, memoryEew: 16, mop: 2, aux: X_SCALAR, scalar: 16, vstart: 1 });
addMemoryCase("vluxei16.v/vstart", { load: true, memoryEew: 16, mop: 1, aux: 12, indexEew: 16, vstart: 1 });
addMemoryCase("vlseg2e32.v/vstart", { load: true, memoryEew: 32, nf: 2, vstart: 1 });
addMemoryCase("vlm.v/vstart", { load: true, memoryEew: 8, sew: 8, vl: 13, aux: 0x0b, vstart: 1 });
addMemoryCase("vl4re32.v/vstart", { load: true, memoryEew: 32, nf: 4, aux: 8, vstart: 1 });

function emitWholeTransfer(code, load, first, addressRegister) {
  code.push(VMEM({ load, width: 0, nf: 8, aux: 8, base: addressRegister, reg: first }));
}

function emitSetup(code, test) {
  code.push(
    CSR(CSR_VSTART, 1, 0, X_ZERO),
    CSR(CSR_VCSR, 1, 0, X_ZERO),
    CSR(CSR_FCSR, 1, 0, X_ZERO),
  );
  loadImmediate(code, X_ADDRESS, SEED);
  for (const first of [0, 8, 16, 24]) {
    emitWholeTransfer(code, true, first, X_ADDRESS);
    if (first !== 24) code.push(ADDI(X_ADDRESS, X_ADDRESS, 128));
  }
  code.push(LD(X_SCALAR, X_SEED, 0), LD(X_RESULT, X_SEED, 8));
  if (test.scalar !== undefined) code.push(ADDI(X_SCALAR, X_ZERO, test.scalar));
  code.push(test.sew === 32 && !test.unboxedScalar ? FLW(4, X_SEED, 16) : FLD(4, X_SEED, 16));
  code.push(FLD(5, X_SEED, 24));

  if (test.memory) {
    // Restore a pristine 128-byte memory window before every memory case.
    // v24..v31 still contain the final 128 bytes of the seed image.
    loadImmediate(code, X_MEMORY, MEMORY);
    code.push(ADDI(X_AVL, X_ZERO, 128));
    code.push((0x57 | (7 << 12) | (X_AVL << 15) | (vtype(8, 8) << 20)) >>> 0);
    code.push(VMEM({ load: false, width: 0, base: X_MEMORY, reg: 24 }));
    if (test.baseOffset !== 0) code.push(ADDI(X_MEMORY, X_MEMORY, test.baseOffset));
  }

  code.push(ADDI(X_AVL, X_ZERO, test.vl));
  const rawType = vtype(test.sew, test.lmul);
  code.push((0x57 | (7 << 12) | (X_AVL << 15) | (rawType << 20)) >>> 0);
  if (test.indexEew !== undefined) {
    loadImmediate(code, X_ADDRESS, INDEX_ADDRESS.get(test.indexEew));
    code.push(VMEM({
      load: true,
      width: WIDTH_CODE.get(test.indexEew),
      base: X_ADDRESS,
      reg: 12,
    }));
  }
  if (test.roundingMode !== undefined) {
    code.push(CSR(CSR_FRM, 5, 0, test.roundingMode));
  }
  if (test.vxrm !== undefined) {
    code.push(ADDI(X_TMP, X_ZERO, test.vxrm << 1));
    code.push(CSR(CSR_VCSR, 1, 0, X_TMP));
  }
  if (test.vstart !== undefined) {
    code.push(ADDI(X_TMP, X_ZERO, test.vstart));
    code.push(CSR(CSR_VSTART, 1, 0, X_TMP));
  }
}

function emitDump(code, test, caseIndex) {
  code.push(ADDI(X_CASE, X_ZERO, caseIndex));
  code.push(
    SD(X_CASE, X_OUTPUT, 0), SD(X_RESULT, X_OUTPUT, 8), FSD(5, X_OUTPUT, 16),
    CSR(CSR_FCSR, 2, X_TMP, X_ZERO), SD(X_TMP, X_OUTPUT, 24),
    CSR(CSR_VCSR, 2, X_TMP, X_ZERO), SD(X_TMP, X_OUTPUT, 32),
    CSR(CSR_VL, 2, X_TMP, X_ZERO), SD(X_TMP, X_OUTPUT, 40),
    CSR(CSR_VTYPE, 2, X_TMP, X_ZERO), SD(X_TMP, X_OUTPUT, 48),
    CSR(CSR_VSTART, 2, X_TMP, X_ZERO), SD(X_TMP, X_OUTPUT, 56),
  );
  if (test.captureMemory) {
    loadImmediate(code, X_ADDRESS, MEMORY);
    code.push(ADDI(X_AVL, X_ZERO, 128));
    code.push((0x57 | (7 << 12) | (X_AVL << 15) | (vtype(8, 8) << 20)) >>> 0);
    code.push(VMEM({ load: true, width: 0, base: X_ADDRESS, reg: 24 }));
  }
  code.push(ADDI(X_AVL, X_ZERO, 128));
  code.push((0x57 | (7 << 12) | (X_AVL << 15) | (vtype(8, 8) << 20)) >>> 0);
  code.push(ADDI(X_DUMP, X_OUTPUT, HEADER_BYTES));
  for (const first of [0, 8, 16, 24]) {
    code.push(VMEM({ load: false, width: 0, base: X_DUMP, reg: first }));
    if (first !== 24) code.push(ADDI(X_DUMP, X_DUMP, 128));
  }
  code.push(ADDI(X_OUTPUT, X_OUTPUT, RECORD_BYTES));
}

function buildProgram() {
  const code = [];
  loadImmediate(code, X_SEED, SEED);
  loadImmediate(code, X_OUTPUT, OUTPUT);
  cases.forEach((test, index) => {
    emitSetup(code, test);
    if (caseRepetitions === 1) {
      test.pc = BASE + code.length * 4;
      code.push(test.instruction);
    } else {
      // Keep the instruction under test at one stable hot PC. The first 64
      // iterations cross the production adaptive tier threshold; remaining
      // iterations therefore exercise the generated vector boundary.
      code.push(ADDI(31, X_ZERO, caseRepetitions));
      const loop = code.length;
      test.pc = BASE + loop * 4;
      code.push(test.instruction, ADDI(31, 31, -1));
      const branch = code.length;
      code.push(B(1, 31, X_ZERO, (loop - branch) * 4));
    }
    emitDump(code, test, index);
  });
  loadImmediate(code, 10, 1); // stdout
  loadImmediate(code, 11, OUTPUT);
  loadImmediate(code, 12, cases.length * RECORD_BYTES);
  loadImmediate(code, 17, 64); // write
  code.push(ECALL);
  loadImmediate(code, 10, 0);
  loadImmediate(code, 17, 93); // exit
  code.push(ECALL);
  return code;
}

function synthElf(code, profile) {
  const fileEnd = INDICES + 0x200;
  const fileBytes = fileEnd - BASE;
  const memoryBytes = OUTPUT + cases.length * RECORD_BYTES - BASE;
  const elf = new Uint8Array(0x1000 + fileBytes);
  const view = new DataView(elf.buffer);
  elf.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0], 0);
  view.setUint16(0x10, 2, true); // ET_EXEC
  view.setUint16(0x12, 243, true); // EM_RISCV
  view.setUint32(0x14, 1, true);
  view.setBigUint64(0x18, BigInt(BASE), true);
  view.setBigUint64(0x20, 64n, true);
  view.setUint32(0x30, 5, true); // EF_RISCV_RVC | EF_RISCV_FLOAT_ABI_DOUBLE
  view.setUint16(0x34, 64, true);
  view.setUint16(0x36, 56, true);
  view.setUint16(0x38, 1, true);
  view.setUint32(64, 1, true); // PT_LOAD
  view.setUint32(68, 7, true); // RWX synthetic differential segment
  view.setBigUint64(72, 0x1000n, true);
  view.setBigUint64(80, BigInt(BASE), true);
  view.setBigUint64(88, BigInt(BASE), true);
  view.setBigUint64(96, BigInt(fileBytes), true);
  view.setBigUint64(104, BigInt(memoryBytes), true);
  view.setBigUint64(112, 0x1000n, true);
  const words = new DataView(elf.buffer, 0x1000);
  code.forEach((word, index) => words.setUint32(index * 4, word, true));

  const seedOffset = 0x1000 + SEED - BASE;
  const seed = new Uint8Array(512);
  let random = profile.random;
  for (let index = 0; index < seed.length; index++) {
    random = Math.imul(random ^ (random >>> 15), 1 | random);
    random ^= random + Math.imul(random ^ (random >>> 7), 61 | random);
    seed[index] = ((random ^ (random >>> 14)) + index * 37) & 0xff;
  }
  const seedView = new DataView(seed.buffer);
  seedView.setBigUint64(0, profile.scalar & U64_MASK, true);
  seedView.setBigUint64(8, BigInt(profile.random >>> 0), true);
  seedView.setUint32(16, profile.fp32, true);
  seedView.setBigUint64(24, profile.fp64, true);
  profile.patch?.(seed, seedView);
  elf.set(seed, seedOffset);

  const memoryOffset = 0x1000 + MEMORY - BASE;
  for (let index = 0; index < 0x200; index++) elf[memoryOffset + index] = (index * 29 + 7) & 0xff;
  for (const [eew, address] of INDEX_ADDRESS) {
    const table = new DataView(elf.buffer, 0x1000 + address - BASE);
    for (let index = 0; index < 8; index++) {
      const value = index * 4;
      if (eew === 8) table.setUint8(index, value);
      if (eew === 16) table.setUint16(index * 2, value, true);
      if (eew === 32) table.setUint32(index * 4, value, true);
      if (eew === 64) table.setBigUint64(index * 8, BigInt(value), true);
    }
  }
  return elf;
}

function run(command, args) {
  return spawnSync(command, args, { encoding: null, maxBuffer: 64 * 1024 * 1024 });
}

let jitRuntime;
async function runWasmJit(elf) {
  if (!jitRuntime) {
    jitRuntime = Promise.all([
      import(join(root, "web", "rv64.js")),
      readFile(join(root, "target", "wasm32-unknown-unknown", "release", "rv64_wasm.wasm")),
    ]);
  }
  const [{ RV64Debug: RV64 }, wasm] = await jitRuntime;
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(1);
  vm.ex.jit_set_user_block_threshold(1);
  const chunks = [];
  let vectorModules = 0;
  const vectorImport = Buffer.from("user_vector");
  vm.onWrite = (fd, bytes) => {
    if (fd === 1) chunks.push(Buffer.from(bytes));
  };
  vm.onJitModule = (bytes) => {
    if (Buffer.from(bytes).includes(vectorImport)) vectorModules++;
  };
  if (!vm.loadElf(elf, ["rvv-jit-differential"], 16)) {
    throw new Error("Wasm JIT failed to load the synthetic RVV ELF");
  }
  const stop = vm.runUser(200_000_000n);
  return {
    output: Buffer.concat(chunks),
    stop,
    exit: vm.userExitCode(),
    jitInsns: vm.ex.jit_stat(0),
    vectorModules,
  };
}

function byteMask(test, qemuCompatibility = false) {
  const mask = new Uint8Array(RECORD_BYTES).fill(0xff);
  if (test.maskDestination) {
    const registerOffset = HEADER_BYTES + test.vd * 16;
    for (let bit = test.vl; bit < 128; bit++) {
      mask[registerOffset + (bit >>> 3)] &= ~(1 << (bit & 7));
    }
  }
  if (qemuCompatibility && test.name === "csrrw/vxsat") {
    mask.fill(0, 32, 40);
    mask[32] = 0x01;
  }
  if (qemuCompatibility && test.name === "csrrw/vxrm") {
    mask.fill(0, 32, 40);
    mask[32] = 0x06;
  }
  return mask;
}

function compare(qemuOutput, rewriteOutput, qemuCompatibility = false) {
  const expectedLength = cases.length * RECORD_BYTES;
  if (qemuOutput.length !== expectedLength || rewriteOutput.length !== expectedLength) {
    return [`output length qemu=${qemuOutput.length} rewrite=${rewriteOutput.length} expected=${expectedLength}`];
  }
  const differences = [];
  for (let caseIndex = 0; caseIndex < cases.length; caseIndex++) {
    const mask = byteMask(cases[caseIndex], qemuCompatibility);
    const base = caseIndex * RECORD_BYTES;
    for (let offset = 0; offset < RECORD_BYTES; offset++) {
      if (((qemuOutput[base + offset] ^ rewriteOutput[base + offset]) & mask[offset]) !== 0) {
        differences.push(
          `${cases[caseIndex].name}: byte ${offset}, qemu=0x${qemuOutput[base + offset].toString(16).padStart(2, "0")}, rewrite=0x${rewriteOutput[base + offset].toString(16).padStart(2, "0")}, pc=0x${cases[caseIndex].pc.toString(16)}`,
        );
        break;
      }
    }
  }
  return differences;
}

if (process.env.RVV_LIST_CASES === "1") {
  for (const test of cases) console.log(test.name);
  process.exit(0);
}

const code = buildProgram();
if (BASE + code.length * 4 >= SEED) throw new Error("differential code overlaps data");
await mkdir(outputDirectory, { recursive: true });
for (const profile of SEED_PROFILES) {
  await writeFile(elfPath, synthElf(code, profile));
  await chmod(elfPath, 0o755);

  const qemuRun = skipQemu
    ? null
    : run(qemu, ["-cpu", "rv64,v=true,vlen=128,elen=64", elfPath]);
  const rewriteRun = run(rewrite, [elfPath]);
  if (qemuRun?.error) throw qemuRun.error;
  if (rewriteRun.error) throw rewriteRun.error;
  if ((qemuRun && qemuRun.status !== 0) || rewriteRun.status !== 0) {
    if (qemuRun) {
      console.error(`${profile.name}: QEMU status=${qemuRun.status}: ${qemuRun.stderr.toString()}${qemuRun.stdout.toString()}`);
    }
    console.error(`${profile.name}: rewrite status=${rewriteRun.status}: ${rewriteRun.stderr.toString()}${rewriteRun.stdout.toString()}`);
    process.exit(1);
  }
  const reference = qemuRun?.stdout ?? rewriteRun.stdout;
  const differences = compare(reference, rewriteRun.stdout, qemu82VectorCsrCompatibility);
  if (differences.length !== 0) {
    for (const difference of differences.slice(0, 30)) {
      console.error(`FAIL ${profile.name}: ${difference}`);
    }
    console.error(`RVV DIFFERENTIAL: ${profile.name}: ${differences.length} / ${cases.length} CASES DIFFER`);
    process.exit(1);
  }
  if (jitDifferential) {
    const jit = await runWasmJit(await readFile(elfPath));
    const jitDifferences = compare(reference, jit.output);
    const eligibleCases = cases.filter((test) => {
      const opcode = test.instruction & 0x7f;
      const funct3 = (test.instruction >>> 12) & 7;
      return opcode === 0x57 ||
        ((opcode === 0x07 || opcode === 0x27) && [0, 5, 6, 7].includes(funct3));
    }).length;
    const minimumGenerated = BigInt(eligibleCases * Math.max(1, caseRepetitions - 80));
    if (jit.stop !== 4 || jit.exit !== 0 || jit.vectorModules === 0 ||
        jit.jitInsns < minimumGenerated || jitDifferences.length !== 0) {
      for (const difference of jitDifferences.slice(0, 30)) {
        console.error(`FAIL JIT ${profile.name}: ${difference}`);
      }
      console.error(
        `RVV JIT DIFFERENTIAL: ${profile.name} failed: stop=${jit.stop} exit=${jit.exit} ` +
        `vector-modules=${jit.vectorModules} jit-insns=${jit.jitInsns} ` +
        `minimum-generated=${minimumGenerated} differences=${jitDifferences.length}`,
      );
      process.exit(1);
    }
    console.log(
      `RVV JIT ${profile.name}: PASS vector-modules=${jit.vectorModules} ` +
      `jit-insns=${jit.jitInsns}`,
    );
  }
}
if (jitDifferential) {
  console.log(
    `RVV JIT DIFFERENTIAL: ALL ${cases.length * SEED_PROFILES.length} EXECUTIONS PASS ` +
    `(${cases.length} encodings x ${SEED_PROFILES.length} data profiles x ` +
    `${caseRepetitions} hot repetitions; interpreter==JIT full output)`,
  );
} else {
  console.log(
    `RVV DIFFERENTIAL: ALL ${cases.length * SEED_PROFILES.length} EXECUTIONS PASS ` +
    `(${cases.length} encodings x ${SEED_PROFILES.length} data profiles; ${code.length} guest instructions/profile)`,
  );
}
