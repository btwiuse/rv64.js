#!/usr/bin/env node

// R112: parse the already-preserved R110 JIT_CODE_DEBUG_INFO records and join
// them to the exact R110 native samples. This is attribution only; elapsed
// time is never evidence.

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
    if (codeSize > BigInt(size)) return null;
    const name = cString(buffer, offset + 56, offset + size);
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

function parseRecords(buffer) {
  if (buffer.length < 40 || buffer.readUInt32LE(0) !== 0x4a695444) {
    throw new Error("invalid jitdump header");
  }
  const headerSize = buffer.readUInt32LE(8);
  if (headerSize < 40 || headerSize > buffer.length) {
    throw new Error("invalid jitdump header size");
  }
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
    const record = { offset, ...valid };
    if (record.id === 0) {
      const name = cString(buffer, offset + 56, offset + record.size);
      record.pid = buffer.readUInt32LE(offset + 16);
      record.tid = buffer.readUInt32LE(offset + 20);
      record.vma = buffer.readBigUInt64LE(offset + 24);
      record.codeAddress = buffer.readBigUInt64LE(offset + 32);
      record.codeSize = buffer.readBigUInt64LE(offset + 40);
      record.codeIndex = buffer.readBigUInt64LE(offset + 48);
      record.name = name.value;
      record.codeOffset = name.end + 1;
    } else if (record.id === 2) {
      record.codeAddress = buffer.readBigUInt64LE(offset + 16);
      record.entryCount = buffer.readBigUInt64LE(offset + 24);
    }
    records.push(record);
    lastTimestamp = record.timestamp;
    offset += record.size;
  }
  if (offset !== buffer.length) {
    throw new Error(`jitdump ended at ${offset}, expected ${buffer.length}`);
  }
  for (let index = 0; index < records.length; index++) {
    records[index].physicalEnd = records[index + 1]?.offset ?? buffer.length;
    records[index].physicalSize = records[index].physicalEnd - records[index].offset;
  }
  return { headerSize, records, resynchronizations };
}

function parseDebugPayload(buffer, record) {
  const result = {
    valid: false,
    declaredSize: record.size,
    physicalSize: record.physicalSize,
    declaredEntryCount: record.entryCount.toString(),
    entries: [],
    alignmentPadding: 0,
    error: null,
  };
  try {
    if (record.entryCount > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new Error("entry count exceeds safe integer range");
    }
    const count = Number(record.entryCount);
    let offset = record.offset + 32;
    let previousFilename = null;
    for (let index = 0; index < count; index++) {
      if (offset + 16 > record.physicalEnd) {
        throw new Error(`entry ${index} fixed fields exceed physical payload`);
      }
      const address = buffer.readBigUInt64LE(offset);
      const line = buffer.readInt32LE(offset + 8);
      const discriminator = buffer.readInt32LE(offset + 12);
      const filenameStart = offset + 16;
      let filename;
      let filenameEnd;
      let repeatedFilename = false;
      if (
        filenameStart + 2 <= record.physicalEnd &&
        buffer[filenameStart] === 0xff &&
        buffer[filenameStart + 1] === 0
      ) {
        if (previousFilename === null) {
          throw new Error(`entry ${index} repeats a missing filename`);
        }
        filename = previousFilename;
        filenameEnd = filenameStart + 1;
        repeatedFilename = true;
      } else {
        const parsed = cString(buffer, filenameStart, record.physicalEnd);
        if (!parsed) throw new Error(`entry ${index} has no filename terminator`);
        filename = parsed.value;
        filenameEnd = parsed.end;
        previousFilename = filename;
      }
      result.entries.push({
        address,
        line,
        discriminator,
        filename,
        repeatedFilename,
      });
      offset = filenameEnd + 1;
    }
    const alignmentPadding = record.physicalEnd - offset;
    if (alignmentPadding < 0 || alignmentPadding > 7) {
      throw new Error(`${alignmentPadding} unsupported trailing physical bytes`);
    }
    for (let index = offset; index < record.physicalEnd; index++) {
      if (buffer[index] !== 0) {
        throw new Error(`nonzero alignment byte at physical offset ${index - record.offset}`);
      }
    }
    result.alignmentPadding = alignmentPadding;
    for (let index = 1; index < result.entries.length; index++) {
      if (result.entries[index].address < result.entries[index - 1].address) {
        throw new Error(`entry addresses decrease at ${index}`);
      }
    }
    result.valid = true;
  } catch (error) {
    result.error = error.message;
  }
  return result;
}

function associateDebugRecords(debugRecords, loads) {
  const byAddress = new Map();
  for (const load of loads) {
    const key = load.codeAddress.toString();
    if (!byAddress.has(key)) byAddress.set(key, { loads: [], debugs: [] });
    byAddress.get(key).loads.push(load);
  }
  for (const debug of debugRecords) {
    const key = debug.codeAddress.toString();
    if (!byAddress.has(key)) byAddress.set(key, { loads: [], debugs: [] });
    byAddress.get(key).debugs.push(debug);
  }

  const byLoad = new Map();
  const debugStatus = new Map();
  for (const group of byAddress.values()) {
    group.loads.sort((left, right) => left.offset - right.offset);
    group.debugs.sort((left, right) => left.offset - right.offset);
    let previousLoadOffset = -1;
    for (const load of group.loads) {
      const candidates = group.debugs.filter((debug) =>
        debug.offset > previousLoadOffset && debug.offset < load.offset
      );
      if (candidates.length === 1) {
        byLoad.set(load.codeIndex.toString(), candidates[0]);
        debugStatus.set(candidates[0].offset, {
          status: "associated",
          codeIndex: load.codeIndex.toString(),
        });
      } else if (candidates.length > 1) {
        for (const debug of candidates) {
          debugStatus.set(debug.offset, {
            status: "ambiguous",
            candidateCodeIndex: load.codeIndex.toString(),
          });
        }
      }
      previousLoadOffset = load.offset;
    }
  }
  for (const debug of debugRecords) {
    if (!debugStatus.has(debug.offset)) {
      debugStatus.set(debug.offset, { status: "unassociated" });
    }
  }
  return { byLoad, debugStatus };
}

function sourcePosition(entries, address, loadEnd) {
  if (address >= loadEnd || entries.length === 0 || address < entries[0].address) {
    return null;
  }
  let low = 0;
  let high = entries.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (entries[middle].address <= address) low = middle + 1;
    else high = middle;
  }
  const index = low - 1;
  const end = entries[index + 1]?.address ?? loadEnd;
  if (address >= end) return null;
  return entries[index];
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
    const operandsList = splitOperands(operands);
    const destination = operandsList.at(-1) || "";
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

function disassemble(buffer, row, directory, objdump) {
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

function instructionAt(instructions, address) {
  let low = 0;
  let high = instructions.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (instructions[middle].address <= address) low = middle + 1;
    else high = middle;
  }
  return instructions[Math.max(0, low - 1)];
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

function containingLoad(sample, generatedLoads) {
  let match = null;
  for (const load of generatedLoads) {
    if (load.timestamp > sample.timestamp) continue;
    if (sample.ip < load.codeAddress || sample.ip >= load.codeAddress + load.codeSize) continue;
    if (!match || load.timestamp > match.timestamp) match = load;
  }
  return match;
}

function addPeriod(map, key, period) {
  map.set(key, (map.get(key) || 0n) + period);
}

function sum(values) {
  return values.reduce((total, value) => total + value, 0n);
}

function fraction(value, total) {
  return total === 0n ? 0 : Number(value) / Number(total);
}

function serialMap(map, denominator) {
  return [...map.entries()]
    .map(([key, period]) => ({ key, period: period.toString(), fraction: fraction(period, denominator) }))
    .sort((left, right) => right.fraction - left.fraction || left.key.localeCompare(right.key));
}

function periodMap(rows) {
  return new Map(rows.map((row) => [row.name, BigInt(row.period)]));
}

function assertPeriodMap(actual, expectedRows, name) {
  const expected = periodMap(expectedRows);
  assert.deepEqual(
    [...actual.entries()].sort(([left], [right]) => left.localeCompare(right)),
    [...expected.entries()].sort(([left], [right]) => left.localeCompare(right)),
    `${name} differs from R110`,
  );
}

function runSelftest() {
  const filename = Buffer.from("wasm://unit\0");
  const semanticBytes = 32 + 16 + filename.length + 16 + 2;
  const physicalBytes = semanticBytes + 2;
  const buffer = Buffer.alloc(physicalBytes);
  buffer.writeUInt32LE(2, 0);
  buffer.writeUInt32LE(physicalBytes + 6, 4);
  buffer.writeBigUInt64LE(1n, 8);
  buffer.writeBigUInt64LE(0x1000n, 16);
  buffer.writeBigUInt64LE(2n, 24);
  let offset = 32;
  buffer.writeBigUInt64LE(0x1004n, offset);
  buffer.writeInt32LE(7, offset + 8);
  buffer.writeInt32LE(0, offset + 12);
  filename.copy(buffer, offset + 16);
  offset += 16 + filename.length;
  buffer.writeBigUInt64LE(0x1010n, offset);
  buffer.writeInt32LE(9, offset + 8);
  buffer.writeInt32LE(3, offset + 12);
  buffer[offset + 16] = 0xff;
  buffer[offset + 17] = 0;
  const record = {
    offset: 0,
    size: physicalBytes + 6,
    physicalEnd: physicalBytes,
    physicalSize: physicalBytes,
    entryCount: 2n,
  };
  const parsed = parseDebugPayload(buffer, record);
  assert.equal(parsed.valid, true);
  assert.equal(parsed.declaredSize - parsed.physicalSize, 6);
  assert.equal(parsed.alignmentPadding, 2);
  assert.deepEqual(parsed.entries.map((entry) => entry.filename), ["wasm://unit", "wasm://unit"]);
  assert.equal(sourcePosition(parsed.entries, 0x1003n, 0x1020n), null);
  assert.equal(sourcePosition(parsed.entries, 0x1004n, 0x1020n).line, 7);
  assert.equal(sourcePosition(parsed.entries, 0x100fn, 0x1020n).line, 7);
  assert.equal(sourcePosition(parsed.entries, 0x1010n, 0x1020n).line, 9);
  assert.equal(sourcePosition(parsed.entries, 0x1020n, 0x1020n), null);

  const loads = [
    { offset: 100, codeAddress: 0x2000n, codeIndex: 1n },
    { offset: 300, codeAddress: 0x2000n, codeIndex: 2n },
  ];
  const debugs = [
    { offset: 50, codeAddress: 0x2000n },
    { offset: 250, codeAddress: 0x2000n },
  ];
  const associated = associateDebugRecords(debugs, loads);
  assert.equal(associated.byLoad.get("1"), debugs[0]);
  assert.equal(associated.byLoad.get("2"), debugs[1]);
  const ambiguousDebugs = [...debugs, { offset: 240, codeAddress: 0x2000n }];
  const ambiguous = associateDebugRecords(ambiguousDebugs, loads);
  assert.equal(ambiguous.byLoad.has("2"), false);
  assert.equal(ambiguous.debugStatus.get(240).status, "ambiguous");
  assert.equal(ambiguous.debugStatus.get(250).status, "ambiguous");
  process.stdout.write("PASS R112 JIT debug source-attribution selftest\n");
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
  "target/bench/r112-jit-debug-source/source-attribution.json",
));
const perf = process.env.PERF || PERF_DEFAULT;
const objdump = process.env.OBJDUMP || "objdump";

const buffer = readFileSync(jitDump);
const parsed = parseRecords(buffer);
const loads = parsed.records.filter((record) => record.id === 0);
const debugRecords = parsed.records.filter((record) => record.id === 2);
for (const record of debugRecords) record.debug = parseDebugPayload(buffer, record);
const debugAssociation = associateDebugRecords(debugRecords, loads);
const generatedName = /^JS:wasm-function\[([0-5])\]-\1-(liftoff|turbofan)$/;
const generatedLoads = loads.filter((record) => generatedName.test(record.name));
const loadByCodeIndex = new Map(loads.map((load) => [load.codeIndex.toString(), load]));
const r110 = JSON.parse(readFileSync(r110ReportPath, "utf8"));
const r110Rows = new Map(r110.codeLoads.map((row) => [row.codeIndex, row]));
const samples = parsePerf(perf, perfData);

const byLoad = new Map();
let allPeriod = 0n;
let mainPeriod = 0n;
let generatedPeriod = 0n;
let generatedSamples = 0;
for (const sample of samples) {
  allPeriod += sample.period;
  if (sample.pid === sample.tid && sample.comm === "node-MainThread") {
    mainPeriod += sample.period;
  }
  const load = containingLoad(sample, generatedLoads);
  if (!load) continue;
  generatedPeriod += sample.period;
  generatedSamples++;
  const key = load.codeIndex.toString();
  if (!byLoad.has(key)) byLoad.set(key, { load, samples: [] });
  byLoad.get(key).samples.push(sample);
}

assert.equal(generatedPeriod.toString(), r110.samples.generatedPeriod);
assert.equal(generatedSamples, r110.samples.generatedSamples);
assert.equal(allPeriod.toString(), r110.samples.totalPeriod);
assert.equal(mainPeriod.toString(), r110.samples.mainThreadPeriod);

const tierPeriods = new Map();
const rolePeriods = new Map();
const familyPeriods = new Map();
const guestFamilyPeriods = new Map();
const positionPeriods = new Map();
const stackPositionPeriods = new Map();
let turboGuestPeriod = 0n;
let turboGuestAssociatedLoadPeriod = 0n;
let turboGuestMappedPeriod = 0n;
let turboGuestNonSentinelPeriod = 0n;
let turboGuestStackPeriod = 0n;
let turboGuestStackMappedPeriod = 0n;
let turboGuestStackNonSentinelPeriod = 0n;

const temporary = mkdtempSync(join(tmpdir(), "rv64-r112-source-"));
try {
  for (const [codeIndex, row] of byLoad) {
    const r110Row = r110Rows.get(codeIndex);
    if (!r110Row) throw new Error(`R110 report lacks sampled load ${codeIndex}`);
    assert.equal(r110Row.name, row.load.name);
    assert.equal(r110Row.codeAddress, `0x${row.load.codeAddress.toString(16)}`);
    assert.equal(r110Row.codeSize, Number(row.load.codeSize));
    const instructions = disassemble(buffer, row, temporary, objdump);
    const debugRecord = debugAssociation.byLoad.get(codeIndex);
    const entries = debugRecord?.debug.valid ? debugRecord.debug.entries : [];
    for (const sample of row.samples) {
      const family = nativeFamily(instructionAt(instructions, sample.ip));
      addPeriod(tierPeriods, r110Row.tier, sample.period);
      addPeriod(rolePeriods, r110Row.role, sample.period);
      addPeriod(familyPeriods, family, sample.period);
      if (r110Row.role === "guest-body") addPeriod(guestFamilyPeriods, family, sample.period);
      if (r110Row.role !== "guest-body" || r110Row.tier !== "turbofan") continue;

      turboGuestPeriod += sample.period;
      const isStack = family.startsWith("native-stack-");
      if (isStack) turboGuestStackPeriod += sample.period;
      if (debugRecord) turboGuestAssociatedLoadPeriod += sample.period;
      const position = sourcePosition(
        entries,
        sample.ip,
        row.load.codeAddress + row.load.codeSize,
      );
      if (!position) continue;
      turboGuestMappedPeriod += sample.period;
      if (isStack) turboGuestStackMappedPeriod += sample.period;
      const nonSentinel = position.filename.length > 0 && position.line > 0;
      if (!nonSentinel) continue;
      const key = `${position.filename}\u0000${position.line}\u0000${position.discriminator}`;
      turboGuestNonSentinelPeriod += sample.period;
      addPeriod(positionPeriods, key, sample.period);
      if (isStack) {
        turboGuestStackNonSentinelPeriod += sample.period;
        addPeriod(stackPositionPeriods, key, sample.period);
      }
    }
  }
} finally {
  rmSync(temporary, { recursive: true, force: true });
}

assertPeriodMap(tierPeriods, r110.tiers, "tier periods");
assertPeriodMap(rolePeriods, r110.roles, "role periods");
assertPeriodMap(familyPeriods, r110.nativeFamilies, "native family periods");
assertPeriodMap(guestFamilyPeriods, r110.guestBodyNativeFamilies, "guest family periods");

const debugRows = debugRecords.map((record) => ({
  offset: record.offset,
  timestampNs: record.timestamp.toString(),
  codeAddress: `0x${record.codeAddress.toString(16)}`,
  declaredSize: record.size,
  physicalSize: record.physicalSize,
  declaredPhysicalDelta: record.size - record.physicalSize,
  declaredEntryCount: record.entryCount.toString(),
  parsedEntryCount: record.debug.entries.length,
  alignmentPadding: record.debug.alignmentPadding,
  valid: record.debug.valid,
  error: record.debug.error,
  association: debugAssociation.debugStatus.get(record.offset),
  associatedCodeLoadName: loadByCodeIndex.get(
    debugAssociation.debugStatus.get(record.offset)?.codeIndex,
  )?.name ?? null,
  firstEntries: record.debug.entries.slice(0, 8).map((entry) => ({
    address: `0x${entry.address.toString(16)}`,
    line: entry.line,
    discriminator: entry.discriminator,
    filename: entry.filename,
    repeatedFilename: entry.repeatedFilename,
  })),
}));
const debugFilenameCounts = new Map();
for (const record of debugRecords) {
  if (!record.debug.valid) continue;
  for (const entry of record.debug.entries) addPeriod(debugFilenameCounts, entry.filename, 1n);
}

const gates = {
  exactDebugPayloads: debugRows.every((row) => row.valid),
  associatedTurboGuestPeriod:
    fraction(turboGuestAssociatedLoadPeriod, turboGuestPeriod) >= 0.95,
  mappedTurboGuestNonSentinelPeriod:
    fraction(turboGuestNonSentinelPeriod, turboGuestPeriod) >= 0.90,
  mappedTurboGuestStackNonSentinelPeriod:
    fraction(turboGuestStackNonSentinelPeriod, turboGuestStackPeriod) >= 0.90,
  distinctStackPositions: stackPositionPeriods.size >= 2,
};
const problems = Object.entries(gates)
  .filter(([, passed]) => !passed)
  .map(([name]) => name);

const report = {
  schema: 1,
  experiment: "R112 JIT debug source attribution",
  performanceEvidence: false,
  inputs: { perfData, jitDump, r110Report: r110ReportPath, perf, objdump },
  jitdump: {
    bytes: buffer.length,
    records: parsed.records.length,
    codeLoads: loads.length,
    generatedCodeLoads: generatedLoads.length,
    debugRecords: debugRows.length,
    parsedDebugEntries: sum(debugRows.map((row) => BigInt(row.parsedEntryCount))).toString(),
    associatedGeneratedDebugRecords: debugRows.filter((row) =>
      row.associatedCodeLoadName && generatedName.test(row.associatedCodeLoadName)
    ).length,
    filenames: serialMap(
      debugFilenameCounts,
      sum([...debugFilenameCounts.values()]),
    ),
    resynchronizations: parsed.resynchronizations,
    debugRows,
  },
  closure: {
    totalSamples: samples.length,
    totalPeriod: allPeriod.toString(),
    mainThreadPeriod: mainPeriod.toString(),
    generatedSamples,
    generatedPeriod: generatedPeriod.toString(),
    tiers: serialMap(tierPeriods, generatedPeriod),
    roles: serialMap(rolePeriods, generatedPeriod),
    nativeFamilies: serialMap(familyPeriods, generatedPeriod),
  },
  attribution: {
    turboFanGuestPeriod: turboGuestPeriod.toString(),
    associatedLoadPeriod: turboGuestAssociatedLoadPeriod.toString(),
    associatedLoadFraction: fraction(turboGuestAssociatedLoadPeriod, turboGuestPeriod),
    mappedPositionPeriod: turboGuestMappedPeriod.toString(),
    mappedPositionFraction: fraction(turboGuestMappedPeriod, turboGuestPeriod),
    nonSentinelPositionPeriod: turboGuestNonSentinelPeriod.toString(),
    nonSentinelPositionFraction: fraction(turboGuestNonSentinelPeriod, turboGuestPeriod),
    explicitStackPeriod: turboGuestStackPeriod.toString(),
    explicitStackMappedPeriod: turboGuestStackMappedPeriod.toString(),
    explicitStackMappedFraction: fraction(turboGuestStackMappedPeriod, turboGuestStackPeriod),
    explicitStackNonSentinelPeriod: turboGuestStackNonSentinelPeriod.toString(),
    explicitStackNonSentinelFraction:
      fraction(turboGuestStackNonSentinelPeriod, turboGuestStackPeriod),
    distinctPositions: positionPeriods.size,
    distinctStackPositions: stackPositionPeriods.size,
    topPositions: serialMap(positionPeriods, turboGuestNonSentinelPeriod).slice(0, 256),
    topStackPositions:
      serialMap(stackPositionPeriods, turboGuestStackNonSentinelPeriod).slice(0, 256),
  },
  gates,
  problems,
  pass: problems.length === 0,
};

writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`R112_SOURCE_ATTRIBUTION ${JSON.stringify({
  output,
  debugRecords: report.jitdump.debugRecords,
  parsedDebugEntries: report.jitdump.parsedDebugEntries,
  attribution: report.attribution,
  gates,
  problems,
})}\n`);
