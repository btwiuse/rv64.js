#!/usr/bin/env node

// I010: join exact JIT-off multi-row perf samples to V8's native code and
// debug records for the integrated RV64 interpreter. Attribution only: this
// script deliberately reports no benchmark speedup or eligible elapsed time.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { basename, join, resolve } from "node:path";
import { tmpdir } from "node:os";

const root = resolve(import.meta.dirname, "../..");
const base = resolve(root, "target/bench/interpreter-general/i010");
const output = resolve(process.argv.find((value) => value.startsWith("--output="))
  ?.slice("--output=".length) ?? join(base, "native-analysis.json"));
const perf = process.env.PERF ??
  "/nix/store/cavgh13ks5f36c4arsbc6r79rajryblf-perf-linux-7.1.7/bin/perf";
const objdump = process.env.OBJDUMP ?? "objdump";
const rowNames = ["string", "compile", "python"];
const targetNeedle = "run_integrated_scalar_t0";

const sha256Bytes = (bytes) => createHash("sha256").update(bytes).digest("hex");
const sha256File = (path) => sha256Bytes(readFileSync(path));
const add = (map, key, amount) => map.set(key, (map.get(key) ?? 0n) + amount);
const sum = (values) => values.reduce((total, value) => total + value, 0n);
const fraction = (value, total) => total === 0n ? 0 : Number(value) / Number(total);
const serial = (map, denominator) => [...map.entries()]
  .map(([key, period]) => ({ key, period: period.toString(), fraction: fraction(period, denominator) }))
  .sort((left, right) => right.fraction - left.fraction || left.key.localeCompare(right.key));

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
  assert.equal(buffer.readUInt32LE(0), 0x4a695444, "invalid jitdump magic");
  const headerSize = buffer.readUInt32LE(8);
  assert(headerSize >= 40 && headerSize <= buffer.length, "invalid jitdump header size");
  const records = [];
  const resynchronizations = [];
  let offset = headerSize;
  let lastTimestamp = 0n;
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
      if (!best) throw new Error(`cannot resynchronize jitdump at 0x${offset.toString(16)}`);
      resynchronizations.push({ expectedOffset: offset, actualOffset: best.offset });
      offset = best.offset;
      valid = best;
    }
    const record = { offset, ...valid };
    if (record.id === 0) {
      const name = cString(buffer, offset + 56, offset + record.size);
      record.pid = buffer.readUInt32LE(offset + 16);
      record.tid = buffer.readUInt32LE(offset + 20);
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
  assert.equal(offset, buffer.length, "jitdump trailing bytes");
  for (let index = 0; index < records.length; index++) {
    records[index].physicalEnd = records[index + 1]?.offset ?? buffer.length;
    records[index].physicalSize = records[index].physicalEnd - records[index].offset;
  }
  return { records, resynchronizations };
}

function parseDebugPayload(buffer, record) {
  const entries = [];
  let offset = record.offset + 32;
  let previousFilename = null;
  try {
    const count = Number(record.entryCount);
    assert(Number.isSafeInteger(count), "debug entry count is not safe");
    for (let index = 0; index < count; index++) {
      assert(offset + 16 <= record.physicalEnd, `debug entry ${index} is truncated`);
      const address = buffer.readBigUInt64LE(offset);
      const line = buffer.readInt32LE(offset + 8);
      const discriminator = buffer.readInt32LE(offset + 12);
      const filenameStart = offset + 16;
      let filename;
      let filenameEnd;
      if (buffer[filenameStart] === 0xff && buffer[filenameStart + 1] === 0) {
        assert.notEqual(previousFilename, null, "debug filename repeat has no predecessor");
        filename = previousFilename;
        filenameEnd = filenameStart + 1;
      } else {
        const parsed = cString(buffer, filenameStart, record.physicalEnd);
        assert(parsed, `debug entry ${index} has no filename terminator`);
        filename = parsed.value;
        filenameEnd = parsed.end;
        previousFilename = filename;
      }
      entries.push({ address, line, discriminator, filename });
      offset = filenameEnd + 1;
    }
    const padding = record.physicalEnd - offset;
    assert(padding >= 0 && padding <= 7, `unsupported debug padding ${padding}`);
    for (let index = offset; index < record.physicalEnd; index++) {
      assert.equal(buffer[index], 0, "nonzero debug padding");
    }
    for (let index = 1; index < entries.length; index++) {
      assert(entries[index].address >= entries[index - 1].address, "debug addresses decrease");
    }
    return { valid: true, entries, error: null };
  } catch (error) {
    return { valid: false, entries, error: error.message };
  }
}

function associateDebugRecords(debugRecords, loads) {
  const groups = new Map();
  for (const load of loads) {
    const key = load.codeAddress.toString();
    if (!groups.has(key)) groups.set(key, { loads: [], debugs: [] });
    groups.get(key).loads.push(load);
  }
  for (const debug of debugRecords) {
    const key = debug.codeAddress.toString();
    if (!groups.has(key)) groups.set(key, { loads: [], debugs: [] });
    groups.get(key).debugs.push(debug);
  }
  const byLoad = new Map();
  for (const group of groups.values()) {
    group.loads.sort((left, right) => left.offset - right.offset);
    group.debugs.sort((left, right) => left.offset - right.offset);
    let previousLoadOffset = -1;
    for (const load of group.loads) {
      const candidates = group.debugs.filter((debug) =>
        debug.offset > previousLoadOffset && debug.offset < load.offset);
      if (candidates.length === 1) byLoad.set(load.codeIndex.toString(), candidates[0]);
      previousLoadOffset = load.offset;
    }
  }
  return byLoad;
}

function parsePerf(path) {
  const text = execFileSync(perf, [
    "script", "-G", "-i", path,
    "-F", "comm,pid,tid,time,event,period,ip,sym,dso",
  ], { encoding: "utf8", maxBuffer: 512 << 20 });
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
  assert(samples.length > 0, `no perf samples in ${path}`);
  return samples;
}

function containingLoad(sample, loads) {
  let result = null;
  for (const load of loads) {
    if (load.timestamp > sample.timestamp) continue;
    if (sample.ip < load.codeAddress || sample.ip >= load.codeAddress + load.codeSize) continue;
    if (!result || load.timestamp > result.timestamp) result = load;
  }
  return result;
}

function sourcePosition(entries, address, loadEnd) {
  if (!entries.length || address < entries[0].address || address >= loadEnd) return null;
  let low = 0;
  let high = entries.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (entries[middle].address <= address) low = middle + 1;
    else high = middle;
  }
  const index = low - 1;
  const end = entries[index + 1]?.address ?? loadEnd;
  return address < end ? entries[index] : null;
}

function disassemble(buffer, load, directory) {
  const path = join(directory, `${load.codeIndex}.bin`);
  const size = Number(load.codeSize);
  writeFileSync(path, buffer.subarray(load.codeOffset, load.codeOffset + size));
  const text = execFileSync(objdump, [
    "-D", "-b", "binary", "-m", "i386:x86-64", "--insn-width=16",
    `--adjust-vma=0x${load.codeAddress.toString(16)}`, path,
  ], { encoding: "utf8", maxBuffer: Math.max(64 << 20, size * 100) });
  const instructions = [];
  const pattern = /^\s*([0-9a-f]+):\s+((?:[0-9a-f]{2}\s+)+)\s*(\S+)(?:\s+(.*?))?\s*$/;
  for (const line of text.split("\n")) {
    const match = pattern.exec(line);
    if (!match) continue;
    instructions.push({
      address: BigInt(`0x${match[1]}`),
      mnemonic: match[3].toLowerCase(),
      operands: (match[4] || "").toLowerCase(),
    });
  }
  assert(instructions.length > 0, `no disassembly for ${load.name}`);
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

function nativeFamily({ mnemonic, operands }) {
  const memory = operands.includes("(");
  const stack = memory && /%(?:r|e)?(?:sp|bp)\b/.test(operands);
  if (/^(?:push|pop|leave|ret|enter)/.test(mnemonic)) return "prologue-epilogue";
  if (/^call/.test(mnemonic)) return operands.trim().startsWith("*") ? "indirect-call" : "direct-call";
  if (/^jmp/.test(mnemonic)) return operands.trim().startsWith("*") ? "indirect-branch" : "direct-branch";
  if (/^j/.test(mnemonic)) return "conditional-branch";
  if (/^lea/.test(mnemonic)) return "address-generation";
  if (memory) return `${stack ? "native-stack" : "linear-or-host-memory"}`;
  if (/^(?:cmp|test|bt)/.test(mnemonic)) return "compare-test-register";
  if (/^(?:mov|xchg|cmov|set)/.test(mnemonic)) return "register-move";
  if (/^(?:add|sub|adc|sbb|and|or|xor|not|neg|sh|sa|ro|mul|imul|div|idiv|inc|dec)/.test(mnemonic)) {
    return "integer-arithmetic";
  }
  if (operands.includes("%xmm") || /^(?:v|xmm|f)/.test(mnemonic)) return "floating-simd";
  return "other";
}

function rowJitDump(rowDirectory) {
  const paths = readdirSync(join(rowDirectory, "jit"))
    .filter((name) => /^jit-\d+\.dump$/.test(name))
    .map((name) => join(rowDirectory, "jit", name));
  const matches = [];
  for (const path of paths) {
    const buffer = readFileSync(path);
    const parsed = parseRecords(buffer);
    if (parsed.records.some((record) => record.id === 0 && record.name.includes(targetNeedle))) {
      matches.push({ path, buffer, parsed });
    }
  }
  assert.equal(matches.length, 1, `${rowDirectory}: expected one worker jitdump`);
  return matches[0];
}

const temporary = mkdtempSync(join(tmpdir(), "rv64-i010-native-"));
const rows = [];
try {
  for (const rowName of rowNames) {
    const rowDirectory = join(base, rowName);
    const perfData = join(rowDirectory, "perf.data");
    const reportPath = join(rowDirectory, "report",
      readdirSync(join(rowDirectory, "report")).find((name) => name.endsWith(".json")));
    const scorecard = JSON.parse(readFileSync(reportPath, "utf8"));
    const trial = scorecard.trials.find((entry) => entry.side === "rewrite" && entry.row === rowName);
    assert(trial?.result?.runtime?.jitProof?.inactiveProof, `${rowName}: JIT inactivity missing`);
    assert.equal(trial.result.runtime.jitProof.activity.generatedInstructions, "0");
    assert.equal(trial.result.runtime.jitProof.activity.dispatches, "0");

    const { path: jitDump, buffer, parsed } = rowJitDump(rowDirectory);
    const loads = parsed.records.filter((record) => record.id === 0);
    const targetLoads = loads.filter((record) => record.name.includes(targetNeedle));
    assert.equal(targetLoads.length, 2, `${rowName}: expected Liftoff and TurboFan loads`);
    assert(targetLoads.some((load) => load.name.endsWith("-liftoff")));
    assert(targetLoads.some((load) => load.name.endsWith("-turbofan")));

    const debugRecords = parsed.records.filter((record) => record.id === 2);
    for (const record of debugRecords) record.debug = parseDebugPayload(buffer, record);
    const debugByLoad = associateDebugRecords(debugRecords, loads);
    const samples = parsePerf(perfData);
    const targetPeriod = new Map();
    const targetSamples = new Map();
    const nativeFamilies = new Map();
    const nativeBlocks = new Map();
    const positions = new Map();
    const instructionsByLoad = new Map();
    let totalPeriod = 0n;
    let mainPeriod = 0n;
    let integratedPeriod = 0n;
    let integratedSamples = 0;
    for (const sample of samples) {
      totalPeriod += sample.period;
      if (sample.comm === "node-MainThread" && sample.pid === sample.tid) mainPeriod += sample.period;
      const load = containingLoad(sample, targetLoads);
      if (!load) continue;
      const tier = load.name.endsWith("-turbofan") ? "turbofan" : "liftoff";
      integratedPeriod += sample.period;
      integratedSamples++;
      add(targetPeriod, tier, sample.period);
      targetSamples.set(tier, (targetSamples.get(tier) ?? 0) + 1);
      if (!instructionsByLoad.has(load.codeIndex.toString())) {
        instructionsByLoad.set(load.codeIndex.toString(),
          disassemble(buffer, load, temporary));
      }
      const instruction = instructionAt(instructionsByLoad.get(load.codeIndex.toString()), sample.ip);
      add(nativeFamilies, `${tier}:${nativeFamily(instruction)}`, sample.period);
      const offset = sample.ip - load.codeAddress;
      const block = (offset / 64n) * 64n;
      add(nativeBlocks, `${tier}:0x${block.toString(16)}`, sample.period);
      const debug = debugByLoad.get(load.codeIndex.toString());
      if (debug?.debug.valid) {
        const position = sourcePosition(
          debug.debug.entries,
          sample.ip,
          load.codeAddress + load.codeSize,
        );
        if (position) {
          add(positions,
            `${tier}:${position.filename}:${position.line}:${position.discriminator}`,
            sample.period);
        }
      }
    }
    assert(integratedSamples > 0, `${rowName}: no integrated samples`);

    rows.push({
      row: rowName,
      inputs: {
        perfData,
        perfDataSha256: sha256File(perfData),
        jitDump,
        jitDumpSha256: sha256Bytes(buffer),
        scorecardReport: reportPath,
        scorecardReportSha256: sha256File(reportPath),
        runtimeWasmSha256: trial.result.runtime.identity.wasmSha256,
      },
      jitProof: trial.result.runtime.jitProof,
      collection: {
        scorecardMeasurementValid: scorecard.measurementValid,
        profilerElapsedExcluded: true,
        perfSamples: samples.length,
        totalPeriod: totalPeriod.toString(),
        mainThreadPeriod: mainPeriod.toString(),
        integratedSamples,
        integratedPeriod: integratedPeriod.toString(),
        integratedFractionOfAll: fraction(integratedPeriod, totalPeriod),
        integratedFractionOfMainThread: fraction(integratedPeriod, mainPeriod),
      },
      loads: targetLoads.map((load) => {
        const tier = load.name.endsWith("-turbofan") ? "turbofan" : "liftoff";
        const debug = debugByLoad.get(load.codeIndex.toString());
        const filenameCounts = new Map();
        for (const entry of debug?.debug.entries ?? []) add(filenameCounts, entry.filename, 1n);
        return {
          tier,
          name: load.name,
          codeIndex: load.codeIndex.toString(),
          codeSize: Number(load.codeSize),
          codeSha256: sha256Bytes(buffer.subarray(
            load.codeOffset,
            load.codeOffset + Number(load.codeSize),
          )),
          samples: targetSamples.get(tier) ?? 0,
          period: (targetPeriod.get(tier) ?? 0n).toString(),
          fractionOfIntegrated: fraction(targetPeriod.get(tier) ?? 0n, integratedPeriod),
          debugValid: debug?.debug.valid ?? false,
          debugEntries: debug?.debug.entries.length ?? 0,
          debugError: debug?.debug.error ?? null,
          debugFilenames: serial(filenameCounts, sum([...filenameCounts.values()])),
        };
      }),
      nativeFamilies: serial(nativeFamilies, integratedPeriod),
      nativeBlocks64: serial(nativeBlocks, integratedPeriod).slice(0, 256),
      debugPositions: serial(positions, integratedPeriod).slice(0, 256),
      jitdump: {
        records: parsed.records.length,
        codeLoads: loads.length,
        debugRecords: debugRecords.length,
        resynchronizations: parsed.resynchronizations,
      },
    });
  }
} finally {
  rmSync(temporary, { recursive: true, force: true });
}

for (const tier of ["liftoff", "turbofan"]) {
  const sizes = rows.map((row) => row.loads.find((load) => load.tier === tier).codeSize);
  // V8 embeds process-specific absolute addresses, so raw native hashes need
  // not match. Stable size and normalized sample offsets are the cross-row
  // identities used below.
  assert.equal(new Set(sizes).size, 1, `${tier} native code size differs across rows`);
}

const crossRowBlocks = new Map();
for (const row of rows) {
  for (const block of row.nativeBlocks64) {
    if (!crossRowBlocks.has(block.key)) crossRowBlocks.set(block.key, new Map());
    crossRowBlocks.get(block.key).set(row.row, block.fraction);
  }
}
const commonBlocks = [...crossRowBlocks]
  .map(([key, values]) => ({
    key,
    fractions: Object.fromEntries(rowNames.map((row) => [row, values.get(row) ?? 0])),
    minimumFraction: Math.min(...rowNames.map((row) => values.get(row) ?? 0)),
    geometricMeanFraction: Math.exp(
      rowNames.reduce((total, row) => total + Math.log(Math.max(values.get(row) ?? 0, 1e-300)), 0) /
      rowNames.length,
    ),
  }))
  .sort((left, right) => right.minimumFraction - left.minimumFraction ||
    right.geometricMeanFraction - left.geometricMeanFraction || left.key.localeCompare(right.key));

const report = {
  schema: 1,
  experiment: "I010 native multi-row integrated-interpreter attribution",
  performanceEvidence: false,
  elapsedValuesExcluded: true,
  productSourceChanged: false,
  sealedInputsExecuted: false,
  tools: { perf, objdump },
  rows,
  commonNativeBlocks64: commonBlocks.slice(0, 256),
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`I010_NATIVE_ANALYSIS ${JSON.stringify({
  output,
  rows: rows.map((row) => ({
    row: row.row,
    integratedFractionOfMainThread: row.collection.integratedFractionOfMainThread,
    loads: row.loads.map(({ tier, codeSize, samples, fractionOfIntegrated, debugValid }) =>
      ({ tier, codeSize, samples, fractionOfIntegrated, debugValid })),
  })),
  topCommonBlocks: commonBlocks.slice(0, 12),
})}\n`);
