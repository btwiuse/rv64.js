#!/usr/bin/env node

// Parse a V8 jitdump directly and join it to Linux perf samples. Node/V8 14.6
// emits two malformed JIT_CODE_DEBUG_INFO lengths in the R110 run, causing
// `perf inject --jit` to stop before generated modules. This reader validates
// every record and resynchronizes only to a nearby, timestamp-monotonic record
// header. It never uses elapsed time as performance evidence.

import { execFileSync } from "node:child_process";
import {
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

const PERF_DEFAULT =
  "/nix/store/cavgh13ks5f36c4arsbc6r79rajryblf-perf-linux-7.1.7/bin/perf";

function argument(name, fallback = null) {
  const prefix = `--${name}=`;
  const value = process.argv.find((entry) => entry.startsWith(prefix));
  return value ? value.slice(prefix.length) : fallback;
}

const perfData = resolve(argument(
  "perf-data",
  "target/bench/r110-optimized-native-census/perf.data",
));
const jitDump = resolve(argument(
  "jit-dump",
  "target/bench/r110-optimized-native-census/jit-292893.dump",
));
const output = resolve(argument(
  "output",
  "target/bench/r110-optimized-native-census/native-census.json",
));
const perf = process.env.PERF || PERF_DEFAULT;
const objdump = process.env.OBJDUMP || "objdump";

const buffer = readFileSync(jitDump);
const headerSize = buffer.readUInt32LE(8);
if (buffer.readUInt32LE(0) !== 0x4a695444 || headerSize < 40) {
  throw new Error("invalid jitdump header");
}

function cString(start, limit) {
  let end = start;
  while (end < limit && buffer[end] !== 0) end++;
  if (end === limit) return null;
  return { value: buffer.toString("utf8", start, end), end };
}

function validateRecord(offset, lastTimestamp) {
  if (offset < headerSize || offset + 16 > buffer.length) return null;
  const id = buffer.readUInt32LE(offset);
  const size = buffer.readUInt32LE(offset + 4);
  if (id > 4 || size < 16 || offset + size > buffer.length) return null;
  const timestamp = buffer.readBigUInt64LE(offset + 8);
  if (timestamp < lastTimestamp) return null;
  if (id === 0) {
    if (size < 57) return null;
    const codeSize = buffer.readBigUInt64LE(offset + 40);
    if (codeSize > BigInt(size)) return null;
    const name = cString(offset + 56, offset + size);
    if (!name || BigInt(offset + size - name.end - 1) < codeSize) return null;
  } else if (id === 1 && size < 56) {
    return null;
  } else if (id === 2 && size < 32) {
    return null;
  } else if (id === 3 && size !== 16) {
    return null;
  } else if (id === 4 && size < 32) {
    return null;
  }
  return { id, size, timestamp };
}

function parseJitDump() {
  let offset = headerSize;
  let lastTimestamp = 0n;
  const records = [];
  const resynchronizations = [];
  while (offset + 16 <= buffer.length) {
    let valid = validateRecord(offset, lastTimestamp);
    if (!valid) {
      let best = null;
      const low = Math.max(headerSize, offset - 4096);
      const high = Math.min(buffer.length - 16, offset + 65536);
      for (let candidate = low; candidate <= high; candidate++) {
        const next = validateRecord(candidate, lastTimestamp);
        if (!next) continue;
        if (!best || Math.abs(candidate - offset) < Math.abs(best.offset - offset)) {
          best = { offset: candidate, ...next };
        }
      }
      if (!best) {
        throw new Error(`cannot resynchronize jitdump at 0x${offset.toString(16)}`);
      }
      resynchronizations.push({
        expectedOffset: offset,
        actualOffset: best.offset,
        delta: best.offset - offset,
      });
      offset = best.offset;
      valid = best;
    }
    lastTimestamp = valid.timestamp;
    const record = { offset, ...valid };
    if (valid.id === 0) {
      const name = cString(offset + 56, offset + valid.size);
      record.pid = buffer.readUInt32LE(offset + 16);
      record.tid = buffer.readUInt32LE(offset + 20);
      record.vma = buffer.readBigUInt64LE(offset + 24);
      record.codeAddress = buffer.readBigUInt64LE(offset + 32);
      record.codeSize = buffer.readBigUInt64LE(offset + 40);
      record.codeIndex = buffer.readBigUInt64LE(offset + 48);
      record.name = name.value;
      record.codeOffset = name.end + 1;
    }
    records.push(record);
    offset += valid.size;
  }
  if (offset !== buffer.length) {
    throw new Error(`jitdump ended at ${offset}, expected ${buffer.length}`);
  }
  return { records, resynchronizations };
}

const parsed = parseJitDump();
const loads = parsed.records.filter((record) => record.id === 0);
const generatedName = /^JS:wasm-function\[([0-5])\]-\1-(liftoff|turbofan)$/;
const generatedLoads = loads.filter((record) => generatedName.test(record.name));
if (generatedLoads.length === 0) throw new Error("jitdump has no generated functions");

const perfText = execFileSync(perf, [
  "script",
  "-G",
  "-i",
  perfData,
  "-F",
  "comm,pid,tid,time,event,period,ip,sym,dso",
], { encoding: "utf8", maxBuffer: 256 << 20 });

const samplePattern = /^\s*(.*?)\s+(\d+)\/(\d+)\s+([0-9.]+):\s+(\d+)\s+cycles:u:\s+([0-9a-f]+)\s+(.*?)\s+\((.*)\)\s*$/;
const samples = [];
for (const line of perfText.split("\n")) {
  const match = samplePattern.exec(line);
  if (!match) continue;
  const timeParts = match[4].split(".");
  const timestamp = BigInt(timeParts[0]) * 1_000_000_000n +
    BigInt((timeParts[1] || "").padEnd(9, "0").slice(0, 9));
  samples.push({
    comm: match[1].trim(),
    pid: Number(match[2]),
    tid: Number(match[3]),
    timestamp,
    period: BigInt(match[5]),
    ip: BigInt(`0x${match[6]}`),
    perfSymbol: match[7],
    perfDso: match[8],
  });
}
if (samples.length === 0) throw new Error("perf script produced no cycle samples");

// Code pages can be reused. Select the newest load that both precedes the
// sample and contains its instruction pointer.
function containingGenerated(sample) {
  let match = null;
  for (const load of generatedLoads) {
    if (load.timestamp > sample.timestamp) continue;
    if (
      sample.ip < load.codeAddress ||
      sample.ip >= load.codeAddress + load.codeSize
    ) continue;
    if (!match || load.timestamp > match.timestamp) match = load;
  }
  return match;
}

let allPeriod = 0n;
let mainPeriod = 0n;
let generatedPeriod = 0n;
let generatedSamples = 0;
const byLoad = new Map();
for (const sample of samples) {
  allPeriod += sample.period;
  if (sample.pid === sample.tid && sample.comm === "node-MainThread") {
    mainPeriod += sample.period;
  }
  const load = containingGenerated(sample);
  if (!load) continue;
  generatedSamples++;
  generatedPeriod += sample.period;
  let row = byLoad.get(load.codeIndex.toString());
  if (!row) {
    row = { load, period: 0n, samples: [] };
    byLoad.set(load.codeIndex.toString(), row);
  }
  row.period += sample.period;
  row.samples.push(sample);
}

function disassemble(row, directory) {
  const load = row.load;
  const rawPath = join(directory, `${load.codeIndex}.bin`);
  const codeSize = Number(load.codeSize);
  writeFileSync(rawPath, buffer.subarray(load.codeOffset, load.codeOffset + codeSize));
  const text = execFileSync(objdump, [
    "-D",
    "-b",
    "binary",
    "-m",
    "i386:x86-64",
    // Keep long x86 instructions (notably movabs) on one line. Without this,
    // GNU objdump emits a continuation line that looks like a second opcode
    // to the deliberately small parser below.
    "--insn-width=16",
    `--adjust-vma=0x${load.codeAddress.toString(16)}`,
    rawPath,
  ], { encoding: "utf8", maxBuffer: Math.max(64 << 20, codeSize * 80) });
  const instructions = [];
  const pattern = /^\s*([0-9a-f]+):\s+((?:[0-9a-f]{2}\s+)+)\s*(\S+)(?:\s+(.*?))?\s*$/;
  for (const line of text.split("\n")) {
    const match = pattern.exec(line);
    if (!match) continue;
    instructions.push({
      address: BigInt(`0x${match[1]}`),
      bytes: match[2].trim().replace(/\s+/g, " "),
      mnemonic: match[3],
      operands: match[4] || "",
      period: 0n,
      samples: 0,
    });
  }
  if (instructions.length === 0) {
    throw new Error(`objdump produced no instructions for code ${load.codeIndex}`);
  }
  for (const sample of row.samples) {
    let low = 0;
    let high = instructions.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (instructions[middle].address <= sample.ip) low = middle + 1;
      else high = middle;
    }
    const instruction = instructions[Math.max(0, low - 1)];
    instruction.period += sample.period;
    instruction.samples++;
  }
  return instructions;
}

function splitOperands(operands) {
  const parts = [];
  let start = 0;
  let depth = 0;
  for (let index = 0; index < operands.length; index++) {
    if (operands[index] === "(") depth++;
    else if (operands[index] === ")") depth--;
    else if (operands[index] === "," && depth === 0) {
      parts.push(operands.slice(start, index).trim());
      start = index + 1;
    }
  }
  parts.push(operands.slice(start).trim());
  return parts;
}

function nativeFamily(instruction) {
  const mnemonic = instruction.mnemonic.toLowerCase();
  const operands = instruction.operands.toLowerCase();
  const hasMemory = operands.includes("(");
  const stackMemory = hasMemory && /%(?:r|e)?(?:sp|bp)\b/.test(operands);
  if (/^(?:push|pop|leave|ret|enter)/.test(mnemonic)) return "prologue-epilogue";
  if (/^(?:call|jmp)/.test(mnemonic) && operands.trim().startsWith("*")) {
    return "indirect-transfer";
  }
  if (/^call/.test(mnemonic)) return "direct-call";
  if (/^j/.test(mnemonic) && !/^jmp/.test(mnemonic)) return "conditional-branch";
  if (/^jmp/.test(mnemonic)) return "direct-branch";
  // LEA performs arithmetic only; its parenthesized syntax is not a memory
  // access and must be classified before the memory-form checks.
  if (/^lea/.test(mnemonic)) return "address-generation";
  if (hasMemory) {
    const operandsList = splitOperands(operands);
    const destination = operandsList.at(-1) || "";
    const destinationIsMemory = destination.includes("(");
    // AT&T syntax puts the destination last, but compare/test never writes
    // that operand. Arithmetic and atomic operations with a memory
    // destination both read and write it, so keep them distinct from stores
    // instead of overstating write-only traffic.
    const readOnly = /^(?:cmp|test|prefetch|ucom|comis)/.test(mnemonic) ||
      mnemonic === "bt";
    const pureStore = /^(?:v?mov|mov[sz]|set|stos|fst)/.test(mnemonic) ||
      (/^rep/.test(mnemonic) && operands.includes("stos"));
    let access = "read";
    if (!readOnly && destinationIsMemory) {
      access = pureStore ? "write" : "read-modify-write";
    }
    const prefix = stackMemory ? "native-stack" : "nonstack-memory";
    return `${prefix}-${access}`;
  }
  if (/^(?:cmp|test|bt)/.test(mnemonic)) return "compare-test-register";
  if (/^(?:mov|xchg|cmov|set)/.test(mnemonic)) return "register-move";
  if (/^(?:add|sub|adc|sbb|and|or|xor|not|neg|sh|sa|ro|mul|imul|div|idiv|inc|dec)/.test(mnemonic)) {
    return "integer-arithmetic";
  }
  if (/^(?:v|xmm|f)/.test(mnemonic) || operands.includes("%xmm")) return "floating-simd";
  if (/^(?:nop|int3|ud2|endbr)/.test(mnemonic)) return "padding-trap";
  return "other";
}

const temporary = mkdtempSync(join(tmpdir(), "rv64-r110-native-"));
const familyPeriods = new Map();
const guestBodyFamilyPeriods = new Map();
const rolePeriods = new Map();
const mnemonicPeriods = new Map();
const hotInstructions = [];
const loadRows = [];
const smallDisassemblies = [];
try {
  for (const row of byLoad.values()) {
    const instructions = disassemble(row, temporary);
    const role = (
      row.load.name === "JS:wasm-function[0]-0-turbofan" &&
      Number(row.load.codeSize) <= 256 &&
      instructions.some((instruction) =>
        instruction.mnemonic === "jmp" && instruction.operands.trim().startsWith("*")
      )
    ) ? "shared-tail-trampoline" : "guest-body";
    const rowFamilies = new Map();
    let frameBytes = 0;
    let staticStackMemoryInstructions = 0;
    for (const instruction of instructions) {
      const family = nativeFamily(instruction);
      if (family.startsWith("native-stack-")) staticStackMemoryInstructions++;
      if (
        instruction.address - row.load.codeAddress < 256n &&
        /^sub/.test(instruction.mnemonic) &&
        /\$0x([0-9a-f]+),%rsp/i.test(instruction.operands)
      ) {
        frameBytes = Math.max(
          frameBytes,
          Number.parseInt(/\$0x([0-9a-f]+),%rsp/i.exec(instruction.operands)[1], 16),
        );
      }
    }
    for (const instruction of instructions) {
      if (instruction.period === 0n) continue;
      const family = nativeFamily(instruction);
      familyPeriods.set(family, (familyPeriods.get(family) || 0n) + instruction.period);
      if (role === "guest-body") {
        guestBodyFamilyPeriods.set(
          family,
          (guestBodyFamilyPeriods.get(family) || 0n) + instruction.period,
        );
      }
      rowFamilies.set(family, (rowFamilies.get(family) || 0n) + instruction.period);
      mnemonicPeriods.set(
        instruction.mnemonic,
        (mnemonicPeriods.get(instruction.mnemonic) || 0n) + instruction.period,
      );
      hotInstructions.push({
        codeIndex: row.load.codeIndex.toString(),
        tier: generatedName.exec(row.load.name)[2],
        codeAddress: `0x${row.load.codeAddress.toString(16)}`,
        offset: Number(instruction.address - row.load.codeAddress),
        address: `0x${instruction.address.toString(16)}`,
        mnemonic: instruction.mnemonic,
        operands: instruction.operands,
        family,
        period: instruction.period,
        samples: instruction.samples,
      });
    }
    if (Number(row.load.codeSize) <= 512) {
      smallDisassemblies.push({
        codeIndex: row.load.codeIndex.toString(),
        name: row.load.name,
        codeAddress: `0x${row.load.codeAddress.toString(16)}`,
        codeSize: Number(row.load.codeSize),
        instructions: instructions.map((instruction) => ({
          offset: Number(instruction.address - row.load.codeAddress),
          mnemonic: instruction.mnemonic,
          operands: instruction.operands,
          family: nativeFamily(instruction),
          period: instruction.period.toString(),
          samples: instruction.samples,
        })),
      });
    }
    loadRows.push({
      codeIndex: row.load.codeIndex.toString(),
      name: row.load.name,
      role,
      tier: generatedName.exec(row.load.name)[2],
      codeAddress: `0x${row.load.codeAddress.toString(16)}`,
      codeSize: Number(row.load.codeSize),
      loadTimestampNs: row.load.timestamp.toString(),
      period: row.period,
      samples: row.samples.length,
      frameBytes,
      staticInstructions: instructions.length,
      staticStackMemoryInstructions,
      nativeFamilies: rowFamilies,
    });
    rolePeriods.set(role, (rolePeriods.get(role) || 0n) + row.period);
  }
} finally {
  rmSync(temporary, { recursive: true, force: true });
}

const fraction = (value, total) => total === 0n ? 0 : Number(value) / Number(total);
const sumPeriods = (values) => values.reduce((sum, value) => sum + value, 0n);
const mapPeriod = (map) => sumPeriods([...map.values()]);
const stackPeriod = (map) => sumPeriods(
  [...map.entries()]
    .filter(([name]) => name.startsWith("native-stack-"))
    .map(([, period]) => period),
);
const serializePeriodRows = (map, denominator = generatedPeriod) => [...map.entries()]
  .map(([name, period]) => ({
    name,
    period: period.toString(),
    fraction: fraction(period, denominator),
  }))
  .sort((left, right) => right.fraction - left.fraction || left.name.localeCompare(right.name));

const tierPeriods = new Map();
const guestBodyPeriod = rolePeriods.get("guest-body") || 0n;
const rowPeriod = sumPeriods(loadRows.map((row) => row.period));
if (rowPeriod !== generatedPeriod) {
  throw new Error(`mapped row period ${rowPeriod} != generated period ${generatedPeriod}`);
}
if (mapPeriod(familyPeriods) !== generatedPeriod) {
  throw new Error("native family periods do not partition generated period");
}
if (mapPeriod(rolePeriods) !== generatedPeriod) {
  throw new Error("role periods do not partition generated period");
}
if (mapPeriod(guestBodyFamilyPeriods) !== guestBodyPeriod) {
  throw new Error("guest-body family periods do not partition guest-body period");
}
const trampolineRows = loadRows.filter((row) => row.role === "shared-tail-trampoline");
if (trampolineRows.length !== 1) {
  throw new Error(`expected one sampled shared tail trampoline, found ${trampolineRows.length}`);
}

const turboFanGuestRows = loadRows.filter((row) =>
  row.role === "guest-body" && row.tier === "turbofan"
);
const turboFanGuestPeriod = sumPeriods(turboFanGuestRows.map((row) => row.period));
const rowStackPeriod = (row) => stackPeriod(row.nativeFamilies);
const turboFanGuestStackPeriod = sumPeriods(turboFanGuestRows.map(rowStackPeriod));
const weightedFrameBytes = turboFanGuestRows.reduce(
  (sum, row) => sum + row.frameBytes * Number(row.period),
  0,
) / Number(turboFanGuestPeriod);
const weightedStackFraction = fraction(turboFanGuestStackPeriod, turboFanGuestPeriod);
let covariance = 0;
let frameVariance = 0;
let stackVariance = 0;
for (const row of turboFanGuestRows) {
  const weight = Number(row.period);
  const localStackFraction = fraction(rowStackPeriod(row), row.period);
  covariance += weight *
    (row.frameBytes - weightedFrameBytes) *
    (localStackFraction - weightedStackFraction);
  frameVariance += weight * (row.frameBytes - weightedFrameBytes) ** 2;
  stackVariance += weight * (localStackFraction - weightedStackFraction) ** 2;
}
const frameBuckets = [
  ["0", (bytes) => bytes === 0],
  ["1-128", (bytes) => bytes > 0 && bytes <= 128],
  ["129-256", (bytes) => bytes > 128 && bytes <= 256],
  ["257-384", (bytes) => bytes > 256 && bytes <= 384],
  ["385-512", (bytes) => bytes > 384 && bytes <= 512],
  [">512", (bytes) => bytes > 512],
].map(([name, includes]) => {
  const rows = turboFanGuestRows.filter((row) => includes(row.frameBytes));
  const period = sumPeriods(rows.map((row) => row.period));
  const explicitStackPeriod = sumPeriods(rows.map(rowStackPeriod));
  return {
    name,
    codeLoads: rows.length,
    period: period.toString(),
    fractionOfTurboFanGuest: fraction(period, turboFanGuestPeriod),
    explicitStackPeriod: explicitStackPeriod.toString(),
    explicitStackFractionWithinBucket: fraction(explicitStackPeriod, period),
  };
});
const R088_GENERATED_STEADY_SHARE = 0.40684;
const guestBodyExplicitStackPeriod = stackPeriod(guestBodyFamilyPeriods);
const nativeAttribution = {
  guestBodyPeriod: guestBodyPeriod.toString(),
  guestBodyFractionOfJitPath: fraction(guestBodyPeriod, generatedPeriod),
  guestBodyExplicitStackPeriod: guestBodyExplicitStackPeriod.toString(),
  guestBodyExplicitStackFraction: fraction(guestBodyExplicitStackPeriod, guestBodyPeriod),
  explicitGuestStackFractionOfJitPath: fraction(
    guestBodyExplicitStackPeriod,
    generatedPeriod,
  ),
  r088GeneratedSteadyShare: R088_GENERATED_STEADY_SHARE,
  wholeCompileExposureUpperBound: fraction(
    guestBodyExplicitStackPeriod,
    generatedPeriod,
  ) * R088_GENERATED_STEADY_SHARE,
  turboFanGuest: {
    codeLoads: turboFanGuestRows.length,
    period: turboFanGuestPeriod.toString(),
    explicitStackPeriod: turboFanGuestStackPeriod.toString(),
    explicitStackFraction: weightedStackFraction,
    weightedFrameBytes,
    weightedCorrelationFrameBytesVsExplicitStackFraction:
      covariance / Math.sqrt(frameVariance * stackVariance),
    frameBuckets,
  },
};
for (const row of loadRows) {
  tierPeriods.set(row.tier, (tierPeriods.get(row.tier) || 0n) + row.period);
  row.nativeFamilies = serializePeriodRows(row.nativeFamilies);
  row.period = row.period.toString();
  row.fraction = fraction(BigInt(row.period), generatedPeriod);
}
loadRows.sort((left, right) => right.fraction - left.fraction);

hotInstructions.sort((left, right) => {
  if (left.period === right.period) return left.address.localeCompare(right.address);
  return left.period > right.period ? -1 : 1;
});
for (const row of hotInstructions) {
  row.period = row.period.toString();
  row.fraction = fraction(BigInt(row.period), generatedPeriod);
}

const report = {
  schema: 1,
  experiment: "R110 optimized native hotspot census",
  performanceEvidence: false,
  inputs: {
    perfData,
    jitDump,
    perf,
    objdump,
  },
  jitdump: {
    bytes: buffer.length,
    records: parsed.records.length,
    codeLoads: loads.length,
    generatedCodeLoads: generatedLoads.length,
    resynchronizations: parsed.resynchronizations,
    generatedByName: Object.fromEntries(
      [...new Set(generatedLoads.map((load) => load.name))].sort().map((name) => [
        name,
        generatedLoads.filter((load) => load.name === name).length,
      ]),
    ),
  },
  samples: {
    total: samples.length,
    totalPeriod: allPeriod.toString(),
    mainThreadPeriod: mainPeriod.toString(),
    generatedSamples,
    generatedPeriod: generatedPeriod.toString(),
    generatedFractionOfMainThread: fraction(generatedPeriod, mainPeriod),
  },
  tiers: serializePeriodRows(tierPeriods),
  nativeFamilies: serializePeriodRows(familyPeriods),
  guestBodyNativeFamilies: serializePeriodRows(guestBodyFamilyPeriods, guestBodyPeriod),
  roles: serializePeriodRows(rolePeriods),
  nativeAttribution,
  mnemonics: serializePeriodRows(mnemonicPeriods),
  codeLoads: loadRows,
  smallDisassemblies,
  hotInstructions: hotInstructions.slice(0, 512),
};

writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`R110_NATIVE_CENSUS ${JSON.stringify({
  output,
  generatedLoads: generatedLoads.length,
  generatedSamples,
  generatedFractionOfMainThread: report.samples.generatedFractionOfMainThread,
  tiers: report.tiers,
  nativeFamilies: report.nativeFamilies,
})}\n`);
