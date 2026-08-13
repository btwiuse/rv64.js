#!/usr/bin/env node

// R113: classify every preserved R110 TurboFan guest native-stack sample by
// architecture-neutral x86 native form and static control context.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const PERF_DEFAULT =
  "/nix/store/cavgh13ks5f36c4arsbc6r79rajryblf-perf-linux-7.1.7/bin/perf";
const R088_GENERATED_SHARE = 0.40684;
const FORM_ORDER = [
  "immediate-compare-test",
  "register-reload",
  "register-spill",
  "immediate-spill",
  "stack-rmw",
  "other-stack-read",
  "other-stack-write",
];
const CONTEXT_ORDER = [
  "entry-prefix",
  "call-neighborhood",
  "control-neighborhood",
  "general-body",
];

function argument(name, fallback = null) {
  const prefix = `--${name}=`;
  const value = process.argv.find((entry) => entry.startsWith(prefix));
  return value ? value.slice(prefix.length) : fallback;
}

function cString(buffer, start, limit) {
  let end = start;
  while (end < limit && buffer[end] !== 0) end++;
  if (end === limit) return null;
  return { value: buffer.toString("utf8", start, end), end };
}

function validateRecord(buffer, headerSize, offset, lastTimestamp) {
  if (offset < headerSize || offset + 16 > buffer.length) return null;
  const id = buffer.readUInt32LE(offset);
  const size = buffer.readUInt32LE(offset + 4);
  if (id > 4 || size < 16 || offset + size > buffer.length) return null;
  const timestamp = buffer.readBigUInt64LE(offset + 8);
  if (timestamp < lastTimestamp) return null;
  if (id === 0) {
    if (size < 57) return null;
    const codeSize = buffer.readBigUInt64LE(offset + 40);
    const name = cString(buffer, offset + 56, offset + size);
    if (!name || codeSize > BigInt(size)) return null;
    if (BigInt(offset + size - name.end - 1) < codeSize) return null;
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

function parseLoads(buffer) {
  if (buffer.length < 40 || buffer.readUInt32LE(0) !== 0x4a695444) {
    throw new Error("invalid jitdump header");
  }
  const headerSize = buffer.readUInt32LE(8);
  let offset = headerSize;
  let lastTimestamp = 0n;
  const records = [];
  const resynchronizations = [];
  while (offset + 16 <= buffer.length) {
    let valid = validateRecord(buffer, headerSize, offset, lastTimestamp);
    if (!valid) {
      let best = null;
      const low = Math.max(headerSize, offset - 4096);
      const high = Math.min(buffer.length - 16, offset + 65536);
      for (let candidate = low; candidate <= high; candidate++) {
        const next = validateRecord(buffer, headerSize, candidate, lastTimestamp);
        if (!next) continue;
        if (!best || Math.abs(candidate - offset) < Math.abs(best.offset - offset)) {
          best = { offset: candidate, ...next };
        }
      }
      if (!best) throw new Error(`cannot resynchronize jitdump at ${offset}`);
      resynchronizations.push({
        expectedOffset: offset,
        actualOffset: best.offset,
        delta: best.offset - offset,
      });
      offset = best.offset;
      valid = best;
    }
    const record = { offset, ...valid };
    if (record.id === 0) {
      const name = cString(buffer, offset + 56, offset + record.size);
      record.codeAddress = buffer.readBigUInt64LE(offset + 32);
      record.codeSize = buffer.readBigUInt64LE(offset + 40);
      record.codeIndex = buffer.readBigUInt64LE(offset + 48);
      record.name = name.value;
      record.codeOffset = name.end + 1;
    }
    records.push(record);
    lastTimestamp = record.timestamp;
    offset += record.size;
  }
  if (offset !== buffer.length) {
    throw new Error(`jitdump ended at ${offset}, expected ${buffer.length}`);
  }
  return {
    records,
    loads: records.filter((record) => record.id === 0),
    resynchronizations,
  };
}

function parsePerf(perf, perfData) {
  const text = execFileSync(perf, [
    "script",
    "-G",
    "-i",
    perfData,
    "-F",
    "comm,pid,tid,time,event,period,ip,sym,dso",
  ], { encoding: "utf8", maxBuffer: 256 << 20 });
  const pattern = /^\s*(.*?)\s+(\d+)\/(\d+)\s+([0-9.]+):\s+(\d+)\s+cycles:u:\s+([0-9a-f]+)\s+(.*?)\s+\((.*)\)\s*$/;
  const samples = [];
  for (const line of text.split("\n")) {
    const match = pattern.exec(line);
    if (!match) continue;
    const timeParts = match[4].split(".");
    samples.push({
      comm: match[1].trim(),
      pid: Number(match[2]),
      tid: Number(match[3]),
      timestamp: BigInt(timeParts[0]) * 1_000_000_000n +
        BigInt((timeParts[1] || "").padEnd(9, "0").slice(0, 9)),
      period: BigInt(match[5]),
      ip: BigInt(`0x${match[6]}`),
    });
  }
  if (samples.length === 0) throw new Error("perf script produced no samples");
  return samples;
}

function containingLoad(sample, loads) {
  let match = null;
  for (const load of loads) {
    if (load.timestamp > sample.timestamp) continue;
    if (sample.ip < load.codeAddress || sample.ip >= load.codeAddress + load.codeSize) continue;
    if (!match || load.timestamp > match.timestamp) match = load;
  }
  return match;
}

function disassemble(buffer, load, directory, objdump) {
  const rawPath = join(directory, `${load.codeIndex}.bin`);
  const codeSize = Number(load.codeSize);
  writeFileSync(rawPath, buffer.subarray(load.codeOffset, load.codeOffset + codeSize));
  const text = execFileSync(objdump, [
    "-D",
    "-b",
    "binary",
    "-m",
    "i386:x86-64",
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
      mnemonic: match[3],
      operands: match[4] || "",
    });
  }
  if (instructions.length === 0) {
    throw new Error(`objdump produced no instructions for code ${load.codeIndex}`);
  }
  return instructions;
}

function instructionIndexAt(instructions, address) {
  let low = 0;
  let high = instructions.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (instructions[middle].address <= address) low = middle + 1;
    else high = middle;
  }
  return Math.max(0, low - 1);
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
  if (/^lea/.test(mnemonic)) return "address-generation";
  if (hasMemory) {
    const list = splitOperands(operands);
    const destination = list.at(-1) || "";
    const destinationIsMemory = destination.includes("(");
    const readOnly = /^(?:cmp|test|prefetch|ucom|comis)/.test(mnemonic) ||
      mnemonic === "bt";
    const pureStore = /^(?:v?mov|mov[sz]|set|stos|fst)/.test(mnemonic) ||
      (/^rep/.test(mnemonic) && operands.includes("stos"));
    let access = "read";
    if (!readOnly && destinationIsMemory) {
      access = pureStore ? "write" : "read-modify-write";
    }
    return `${stackMemory ? "native-stack" : "nonstack-memory"}-${access}`;
  }
  if (/^(?:cmp|test|bt)/.test(mnemonic)) return "compare-test-register";
  if (/^(?:mov|xchg|cmov|set)/.test(mnemonic)) return "register-move";
  if (/^(?:add|sub|adc|sbb|and|or|xor|not|neg|sh|sa|ro|mul|imul|div|idiv|inc|dec)/.test(mnemonic)) {
    return "integer-arithmetic";
  }
  if (/^(?:v|xmm|f)/.test(mnemonic) || operands.includes("%xmm")) {
    return "floating-simd";
  }
  if (/^(?:nop|int3|ud2|endbr)/.test(mnemonic)) return "padding-trap";
  return "other";
}

function signedDisplacement(text) {
  if (text === "" || text === "+") return 0;
  if (/^-0x[0-9a-f]+$/i.test(text)) return -Number.parseInt(text.slice(3), 16);
  if (/^\+?0x[0-9a-f]+$/i.test(text)) return Number.parseInt(text.replace("+", "").slice(2), 16);
  if (/^[+-]?\d+$/.test(text)) return Number.parseInt(text, 10);
  throw new Error(`unsupported stack displacement ${text}`);
}

function memoryWidth(instruction, operands) {
  const mnemonic = instruction.mnemonic.toLowerCase();
  const suffix = /(?:^|[^a-z])([bwlq])$/.exec(mnemonic)?.[1] ??
    /(?:mov|cmp|test|add|sub|and|or|xor|shl|shr|sar|inc|dec)([bwlq])$/.exec(mnemonic)?.[1];
  if (suffix) return { b: 1, w: 2, l: 4, q: 8 }[suffix];
  for (const operand of operands) {
    if (/%(?:al|bl|cl|dl|sil|dil|spl|bpl|r\d+b)\b/i.test(operand)) return 1;
    if (/%(?:ax|bx|cx|dx|si|di|sp|bp|r\d+w)\b/i.test(operand)) return 2;
    if (/%(?:eax|ebx|ecx|edx|esi|edi|esp|ebp|r\d+d)\b/i.test(operand)) return 4;
    if (/%(?:rax|rbx|rcx|rdx|rsi|rdi|rsp|rbp|r\d+)\b/i.test(operand)) return 8;
    if (/%xmm\d+\b/i.test(operand)) return 16;
  }
  return 0;
}

function stackAccess(instruction) {
  const operands = splitOperands(instruction.operands.toLowerCase());
  const stackPattern = /^([+-]?(?:0x[0-9a-f]+|\d+)?)\(%(rbp|rsp)\)$/i;
  const stackOperands = operands
    .map((operand, index) => ({ operand, index, match: stackPattern.exec(operand) }))
    .filter((entry) => entry.match);
  if (stackOperands.length !== 1) return null;
  const stack = stackOperands[0];
  const destinationIndex = operands.length - 1;
  const source = operands[0] || "";
  const destination = operands[destinationIndex] || "";
  const mnemonic = instruction.mnemonic.toLowerCase();
  const pureMove = /^(?:v?mov|mov[sz])/.test(mnemonic);
  const stackIsSource = stack.index < destinationIndex;
  const stackIsDestination = stack.index === destinationIndex;
  const sourceIsImmediate = source.startsWith("$");
  const sourceIsRegister = source.startsWith("%") && !source.includes("(");
  const destinationIsRegister = destination.startsWith("%") && !destination.includes("(");
  let form;
  if (/^(?:cmp|test|bt)/.test(mnemonic) && operands.some((operand) => operand.startsWith("$"))) {
    form = "immediate-compare-test";
  } else if (pureMove && stackIsSource && destinationIsRegister) {
    form = "register-reload";
  } else if (pureMove && stackIsDestination && sourceIsRegister) {
    form = "register-spill";
  } else if (pureMove && stackIsDestination && sourceIsImmediate) {
    form = "immediate-spill";
  } else {
    const family = nativeFamily(instruction);
    if (family === "native-stack-read-modify-write") form = "stack-rmw";
    else if (family === "native-stack-read") form = "other-stack-read";
    else form = "other-stack-write";
  }
  const width = memoryWidth(instruction, operands);
  return {
    form,
    base: stack.match[2].toLowerCase(),
    displacement: signedDisplacement(stack.match[1]),
    width,
    slotKey: `${stack.match[2].toLowerCase()}:${signedDisplacement(stack.match[1])}:${width}`,
  };
}

function isCall(instruction) {
  return /^call/.test(instruction.mnemonic.toLowerCase());
}

function isBranch(instruction) {
  return /^j/.test(instruction.mnemonic.toLowerCase());
}

function isReturnOrTrap(instruction) {
  return /^(?:ret|leave|ud2|int3)/.test(instruction.mnemonic.toLowerCase());
}

function nearby(instructions, index, limit, target, barrier) {
  const candidates = [];
  for (const direction of [-1, 1]) {
    for (let distance = 1; distance <= limit; distance++) {
      const instruction = instructions[index + direction * distance];
      if (!instruction) break;
      if (target(instruction)) {
        candidates.push({ distance, direction: direction < 0 ? "before" : "after" });
        break;
      }
      if (barrier(instruction)) break;
    }
  }
  candidates.sort((left, right) =>
    left.distance - right.distance || left.direction.localeCompare(right.direction)
  );
  return candidates[0] ?? null;
}

function nativeContext(instructions, index, loadAddress) {
  const offset = Number(instructions[index].address - loadAddress);
  if (offset < 512) return { context: "entry-prefix", distance: null, direction: null };
  const call = nearby(
    instructions,
    index,
    8,
    isCall,
    (instruction) => isCall(instruction) || isBranch(instruction) || isReturnOrTrap(instruction),
  );
  if (call) return { context: "call-neighborhood", ...call };
  const control = nearby(
    instructions,
    index,
    2,
    isBranch,
    (instruction) => isCall(instruction) || isReturnOrTrap(instruction),
  );
  if (control) return { context: "control-neighborhood", ...control };
  return { context: "general-body", distance: null, direction: null };
}

function addPeriod(map, key, period, samples = 1) {
  let row = map.get(key);
  if (!row) {
    row = { period: 0n, samples: 0 };
    map.set(key, row);
  }
  row.period += period;
  row.samples += samples;
}

function addFamily(families, form, codeIndex, period) {
  let row = families.get(form);
  if (!row) {
    row = { period: 0n, samples: 0, loads: new Map() };
    families.set(form, row);
  }
  row.period += period;
  row.samples++;
  addPeriod(row.loads, codeIndex, period);
}

function sum(values) {
  return values.reduce((total, value) => total + value, 0n);
}

function fraction(value, total) {
  return total === 0n ? 0 : Number(value) / Number(total);
}

function serializeSimple(map, denominator) {
  return [...map.entries()]
    .map(([name, row]) => ({
      name,
      period: row.period.toString(),
      samples: row.samples,
      fraction: fraction(row.period, denominator),
    }))
    .sort((left, right) => right.fraction - left.fraction || left.name.localeCompare(right.name));
}

function concentration(loads, total) {
  const ordered = [...loads.entries()]
    .map(([codeIndex, row]) => ({ codeIndex, period: row.period, samples: row.samples }))
    .sort((left, right) => left.period === right.period
      ? left.codeIndex.localeCompare(right.codeIndex)
      : left.period > right.period ? -1 : 1);
  const prefix = (count) => fraction(sum(ordered.slice(0, count).map((row) => row.period)), total);
  return {
    distinctLoads: ordered.length,
    topOneFraction: prefix(1),
    topFiveFraction: prefix(5),
    topEightFraction: prefix(8),
    topLoads: ordered.slice(0, 32).map((row) => ({
      codeIndex: row.codeIndex,
      period: row.period.toString(),
      samples: row.samples,
      fraction: fraction(row.period, total),
    })),
  };
}

function assertPeriodRows(actual, expectedRows, name) {
  const expected = new Map(expectedRows.map((row) => [row.name, BigInt(row.period)]));
  const normalized = new Map([...actual].map(([key, value]) => [key, value.period]));
  assert.deepEqual(
    [...normalized].sort(([left], [right]) => left.localeCompare(right)),
    [...expected].sort(([left], [right]) => left.localeCompare(right)),
    `${name} differs from R110`,
  );
}

function runSelftest() {
  const instruction = (mnemonic, operands, address = 0x1000n) => ({ mnemonic, operands, address });
  assert.equal(signedDisplacement("-0x50"), -80);
  assert.equal(signedDisplacement("0x20"), 32);
  assert.equal(signedDisplacement(""), 0);
  assert.equal(stackAccess(instruction("cmpl", "$0x3,-0x50(%rbp)")).form, "immediate-compare-test");
  assert.equal(stackAccess(instruction("mov", "-0x48(%rbp),%r11")).form, "register-reload");
  assert.equal(stackAccess(instruction("mov", "%r8,-0xf0(%rbp)")).form, "register-spill");
  assert.equal(stackAccess(instruction("movq", "$0x7,-0x88(%rbp)")).form, "immediate-spill");
  assert.equal(stackAccess(instruction("addq", "$0x1,-0x20(%rbp)")).form, "stack-rmw");
  assert.equal(stackAccess(instruction("cmp", "%rax,-0x20(%rbp)")).form, "other-stack-read");
  assert.equal(stackAccess(instruction("setne", "-0x20(%rbp)")).form, "other-stack-write");
  assert.equal(stackAccess(instruction("movq", "-0x18(%rsp),%rax")).slotKey, "rsp:-24:8");

  const entry = [instruction("mov", "%rax,-0x20(%rbp)", 0x1100n)];
  assert.equal(nativeContext(entry, 0, 0x1000n).context, "entry-prefix");
  const call = [
    instruction("call", "0x2000", 0x1200n),
    instruction("mov", "-0x20(%rbp),%rax", 0x1205n),
  ];
  assert.equal(nativeContext(call, 1, 0x1000n).context, "call-neighborhood");
  const blocked = [
    instruction("call", "0x2000", 0x1200n),
    instruction("jne", "0x1300", 0x1205n),
    instruction("mov", "-0x20(%rbp),%rax", 0x1207n),
  ];
  assert.equal(nativeContext(blocked, 2, 0x1000n).context, "control-neighborhood");
  const general = [
    instruction("add", "%rax,%rbx", 0x1200n),
    instruction("mov", "-0x20(%rbp),%rax", 0x1204n),
  ];
  assert.equal(nativeContext(general, 1, 0x1000n).context, "general-body");

  const loads = new Map([
    ["a", { period: 50n, samples: 5 }],
    ["b", { period: 30n, samples: 3 }],
    ["c", { period: 20n, samples: 2 }],
  ]);
  assert.equal(concentration(loads, 100n).topOneFraction, 0.5);
  assert.equal(concentration(loads, 100n).topFiveFraction, 1);
  process.stdout.write("PASS R113 native stack-context selftest\n");
}

if (process.argv.includes("--selftest")) {
  runSelftest();
  process.exit(0);
}

const perfData = resolve(argument(
  "perf-data",
  "target/bench/r110-optimized-native-census/perf.data",
));
const jitDump = resolve(argument(
  "jit-dump",
  "target/bench/r110-optimized-native-census/jit-292893.dump",
));
const r110ReportPath = resolve(argument(
  "r110-report",
  "target/bench/r110-optimized-native-census/native-census-v3.json",
));
const output = resolve(argument(
  "output",
  "target/bench/r113-native-stack-context/stack-context.json",
));
const perf = process.env.PERF || PERF_DEFAULT;
const objdump = process.env.OBJDUMP || "objdump";

const buffer = readFileSync(jitDump);
const parsed = parseLoads(buffer);
const generatedPattern = /^JS:wasm-function\[([0-5])\]-\1-(liftoff|turbofan)$/;
const generatedLoads = parsed.loads.filter((load) => generatedPattern.test(load.name));
const samples = parsePerf(perf, perfData);
const r110 = JSON.parse(readFileSync(r110ReportPath, "utf8"));
const r110Rows = new Map(r110.codeLoads.map((row) => [row.codeIndex, row]));

let totalPeriod = 0n;
let mainPeriod = 0n;
let generatedPeriod = 0n;
let generatedSamples = 0;
const byLoad = new Map();
for (const sample of samples) {
  totalPeriod += sample.period;
  if (sample.pid === sample.tid && sample.comm === "node-MainThread") mainPeriod += sample.period;
  const load = containingLoad(sample, generatedLoads);
  if (!load) continue;
  generatedPeriod += sample.period;
  generatedSamples++;
  const key = load.codeIndex.toString();
  if (!byLoad.has(key)) byLoad.set(key, { load, samples: [] });
  byLoad.get(key).samples.push(sample);
}

assert.equal(samples.length, r110.samples.total);
assert.equal(totalPeriod.toString(), r110.samples.totalPeriod);
assert.equal(mainPeriod.toString(), r110.samples.mainThreadPeriod);
assert.equal(generatedSamples, r110.samples.generatedSamples);
assert.equal(generatedPeriod.toString(), r110.samples.generatedPeriod);

const tierPeriods = new Map();
const rolePeriods = new Map();
const familyPeriods = new Map();
const guestFamilyPeriods = new Map();
const forms = new Map();
const contexts = new Map();
const formContexts = new Map();
const slots = new Map();
const perLoadSlots = new Map();
const frameBuckets = new Map();
const stackByLoad = new Map();
let turboGuestPeriod = 0n;
let turboGuestStackPeriod = 0n;
let turboGuestStackSamples = 0;

const frameBucket = (bytes) => {
  if (bytes === 0) return "0";
  if (bytes <= 128) return "1-128";
  if (bytes <= 256) return "129-256";
  if (bytes <= 384) return "257-384";
  if (bytes <= 512) return "385-512";
  return ">512";
};

const temporary = mkdtempSync(join(tmpdir(), "rv64-r113-stack-"));
try {
  for (const [codeIndex, row] of byLoad) {
    const expected = r110Rows.get(codeIndex);
    if (!expected) throw new Error(`R110 report lacks sampled load ${codeIndex}`);
    assert.equal(expected.name, row.load.name);
    assert.equal(expected.codeAddress, `0x${row.load.codeAddress.toString(16)}`);
    assert.equal(expected.codeSize, Number(row.load.codeSize));
    const rowPeriod = sum(row.samples.map((sample) => sample.period));
    assert.equal(rowPeriod.toString(), expected.period);
    assert.equal(row.samples.length, expected.samples);
    const instructions = disassemble(buffer, row.load, temporary, objdump);
    for (const sample of row.samples) {
      const index = instructionIndexAt(instructions, sample.ip);
      const instruction = instructions[index];
      const family = nativeFamily(instruction);
      addPeriod(tierPeriods, expected.tier, sample.period);
      addPeriod(rolePeriods, expected.role, sample.period);
      addPeriod(familyPeriods, family, sample.period);
      if (expected.role === "guest-body") addPeriod(guestFamilyPeriods, family, sample.period);
      if (expected.role !== "guest-body" || expected.tier !== "turbofan") continue;
      turboGuestPeriod += sample.period;
      if (!family.startsWith("native-stack-")) continue;
      const access = stackAccess(instruction);
      if (!access) {
        throw new Error(`cannot decode sampled stack operand ${instruction.mnemonic} ${instruction.operands}`);
      }
      const context = nativeContext(instructions, index, row.load.codeAddress);
      turboGuestStackPeriod += sample.period;
      turboGuestStackSamples++;
      addFamily(forms, access.form, codeIndex, sample.period);
      addPeriod(contexts, context.context, sample.period);
      addPeriod(formContexts, `${access.form}/${context.context}`, sample.period);
      addPeriod(slots, access.slotKey, sample.period);
      addPeriod(perLoadSlots, `${codeIndex}/${access.slotKey}`, sample.period);
      addPeriod(frameBuckets, frameBucket(expected.frameBytes), sample.period);
      addPeriod(stackByLoad, codeIndex, sample.period);
    }
  }
} finally {
  rmSync(temporary, { recursive: true, force: true });
}

assertPeriodRows(tierPeriods, r110.tiers, "tier periods");
assertPeriodRows(rolePeriods, r110.roles, "role periods");
assertPeriodRows(familyPeriods, r110.nativeFamilies, "native family periods");
assertPeriodRows(guestFamilyPeriods, r110.guestBodyNativeFamilies, "guest family periods");
assert.equal(turboGuestPeriod.toString(), r110.nativeAttribution.turboFanGuest.period);
assert.equal(turboGuestStackPeriod.toString(), r110.nativeAttribution.turboFanGuest.explicitStackPeriod);
for (const partition of [forms, contexts, formContexts, slots, perLoadSlots, frameBuckets, stackByLoad]) {
  assert.equal(sum([...partition.values()].map((row) => row.period)), turboGuestStackPeriod);
}

const formRows = FORM_ORDER.map((name) => {
  const row = forms.get(name) ?? { period: 0n, samples: 0, loads: new Map() };
  const spread = concentration(row.loads, row.period);
  const wholeCompileExposure = fraction(row.period, generatedPeriod) * R088_GENERATED_SHARE;
  const gates = {
    exposure: wholeCompileExposure >= 0.02,
    distinctLoads: spread.distinctLoads >= 10,
    samples: row.samples >= 500,
    topOne: spread.topOneFraction <= 0.35,
    topFive: spread.topFiveFraction <= 0.70,
  };
  return {
    name,
    period: row.period.toString(),
    samples: row.samples,
    fractionOfTurboFanGuestStack: fraction(row.period, turboGuestStackPeriod),
    fractionOfCompleteJitPath: fraction(row.period, generatedPeriod),
    wholeCompileExposure,
    spread,
    gates,
    pass: Object.values(gates).every(Boolean),
  };
});
const passingForms = formRows.filter((row) => row.pass).sort((left, right) =>
  right.wholeCompileExposure - left.wholeCompileExposure ||
  FORM_ORDER.indexOf(left.name) - FORM_ORDER.indexOf(right.name)
);

const report = {
  schema: 1,
  experiment: "R113 native stack-slot and control-context attribution",
  performanceEvidence: false,
  inputs: { perfData, jitDump, r110Report: r110ReportPath, perf, objdump },
  rule: {
    entryPrefixBytes: 512,
    callNeighborhoodInstructions: 8,
    controlNeighborhoodInstructions: 2,
    r088GeneratedShare: R088_GENERATED_SHARE,
    minimumWholeCompileExposure: 0.02,
    minimumLoads: 10,
    minimumSamples: 500,
    maximumTopOneLoadFraction: 0.35,
    maximumTopFiveLoadFraction: 0.70,
    formOrder: FORM_ORDER,
    contextOrder: CONTEXT_ORDER,
  },
  closure: {
    records: parsed.records.length,
    resynchronizations: parsed.resynchronizations,
    totalSamples: samples.length,
    totalPeriod: totalPeriod.toString(),
    mainPeriod: mainPeriod.toString(),
    generatedSamples,
    generatedPeriod: generatedPeriod.toString(),
    turboFanGuestPeriod: turboGuestPeriod.toString(),
    turboFanGuestStackSamples: turboGuestStackSamples,
    turboFanGuestStackPeriod: turboGuestStackPeriod.toString(),
    tiers: serializeSimple(tierPeriods, generatedPeriod),
    roles: serializeSimple(rolePeriods, generatedPeriod),
    nativeFamilies: serializeSimple(familyPeriods, generatedPeriod),
  },
  forms: formRows,
  contexts: CONTEXT_ORDER.map((name) => {
    const row = contexts.get(name) ?? { period: 0n, samples: 0 };
    return {
      name,
      period: row.period.toString(),
      samples: row.samples,
      fraction: fraction(row.period, turboGuestStackPeriod),
      wholeCompileExposure: fraction(row.period, generatedPeriod) * R088_GENERATED_SHARE,
    };
  }),
  formContexts: serializeSimple(formContexts, turboGuestStackPeriod),
  slots: serializeSimple(slots, turboGuestStackPeriod),
  perLoadSlots: serializeSimple(perLoadSlots, turboGuestStackPeriod),
  frameBuckets: serializeSimple(frameBuckets, turboGuestStackPeriod),
  stackByLoad: serializeSimple(stackByLoad, turboGuestStackPeriod),
  passingForms: passingForms.map((row) => row.name),
  admittedForm: passingForms[0]?.name ?? null,
  pass: passingForms.length > 0,
};

writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`R113_NATIVE_STACK_CONTEXT ${JSON.stringify({
  output,
  closure: report.closure,
  forms: report.forms.map((row) => ({
    name: row.name,
    samples: row.samples,
    wholeCompileExposure: row.wholeCompileExposure,
    distinctLoads: row.spread.distinctLoads,
    topOneFraction: row.spread.topOneFraction,
    topFiveFraction: row.spread.topFiveFraction,
    pass: row.pass,
  })),
  contexts: report.contexts,
  admittedForm: report.admittedForm,
})}\n`);
