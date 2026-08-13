#!/usr/bin/env node

// Deterministically refine the immutable R119 optimized Cpu::step profile.
// This is attribution only: perf/JIT logging perturbs execution, so elapsed
// time is deliberately absent and performanceEvidence is always false.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { basename, resolve } from "node:path";
import { readFileSync, writeFileSync } from "node:fs";
import assert from "node:assert/strict";

const PERF_DEFAULT =
  "/nix/store/cavgh13ks5f36c4arsbc6r79rajryblf-perf-linux-7.1.7/bin/perf";

function argument(name, fallback = null) {
  const prefix = `--${name}=`;
  const value = process.argv.find((entry) => entry.startsWith(prefix));
  return value ? value.slice(prefix.length) : fallback;
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function fraction(value, total) {
  return total === 0n ? 0 : Number(value) / Number(total);
}

export function parseInstruction(line) {
  const match = /^\s*([0-9a-f]+):\s+((?:[0-9a-f]{2}(?:\s+|$))+)(\S+)(?:\s+(.*?))?\s*$/.exec(line);
  if (!match) return null;
  // objdump wraps instructions longer than its byte column onto address-tagged
  // continuation lines containing bytes only. The permissive byte regex can
  // otherwise mistake the final byte for a mnemonic.
  if (/^[0-9a-f]{2}$/.test(match[3])) return null;
  return {
    address: Number.parseInt(match[1], 16),
    bytes: match[2].trim().split(/\s+/),
    mnemonic: match[3],
    operands: match[4] ?? "",
  };
}

export function parseSample(line) {
  const pattern = /^\s*(\S+)\s+(\d+)\/(\d+)\s+(\d+)\s+([0-9a-f]+)\s+(.*?)\s+\((.*)\)\s*$/;
  const match = pattern.exec(line);
  if (!match) return null;
  return {
    comm: match[1],
    pid: Number(match[2]),
    tid: Number(match[3]),
    period: BigInt(match[4]),
    ip: Number.parseInt(match[5], 16),
    symbol: match[6],
    dso: match[7],
  };
}

function selftest() {
  assert.deepEqual(
    parseInstruction("  16ed:\t4a c7 04 1f 00 00 00 \tmov QWORD PTR [rdi+r11*1],0x0"),
    {
      address: 0x16ed,
      bytes: ["4a", "c7", "04", "1f", "00", "00", "00"],
      mnemonic: "mov",
      operands: "QWORD PTR [rdi+r11*1],0x0",
    },
  );
  assert.equal(parseInstruction("  16f4:\t00 00 "), null);
  const sample = parseSample(
    " node-MainThread  7/7  1234  abc JS:Cpu4step-turbofan+0x166d (/tmp/jitted.so)",
  );
  assert.equal(sample.period, 1234n);
  assert.equal(sample.symbol, "JS:Cpu4step-turbofan+0x166d");
  assert.equal(sample.dso, "/tmp/jitted.so");
  process.stdout.write("R122 native-census selftest: PASS\n");
}

if (process.argv.includes("--selftest")) {
  selftest();
  process.exit(0);
}

const perfData = resolve(argument(
  "perf-data",
  "target/bench/r119-existing-probe-opportunity/perf.jitted.data",
));
const stepDso = resolve(argument(
  "step-dso",
  "target/bench/r119-existing-probe-opportunity/jitted-384587-2419.so",
));
const output = resolve(argument(
  "output",
  "target/bench/r122-interpreter-body/native-census.json",
));
const perf = process.env.PERF || PERF_DEFAULT;
const readelf = process.env.READELF || "readelf";
const objdump = process.env.OBJDUMP || "objdump";

const expected = Object.freeze({
  perfData: "24028310d77f5f8c5be6e5cb560e20d8c051b010a5b9fe078cc41fa73a691a2c",
  stepDso: "5c712b022f0808b58a7b366209a951a5e1bfdd3922ab625f6f5a5d11a62b90b6",
});
assert.equal(sha256(perfData), expected.perfData, "R119 perf input changed");
assert.equal(sha256(stepDso), expected.stepDso, "R119 Cpu::step DSO changed");

const symbolTable = execFileSync(readelf, ["-Ws", stepDso], {
  encoding: "utf8",
  maxBuffer: 8 << 20,
});
const symbolRow = symbolTable.split("\n").find((line) =>
  line.includes("Cpu4step") && line.includes("-turbofan")
);
if (!symbolRow) throw new Error("optimized Cpu::step symbol is absent");
const symbolMatch = /^\s*\d+:\s+([0-9a-f]+)\s+(\d+)\s+FUNC\b/.exec(symbolRow);
if (!symbolMatch) throw new Error("cannot parse optimized Cpu::step symbol");
const symbolAddress = Number.parseInt(symbolMatch[1], 16);
const nativeBytes = Number(symbolMatch[2]);

const disassembly = execFileSync(objdump, ["-d", "-Mintel", stepDso], {
  encoding: "utf8",
  maxBuffer: 32 << 20,
});
const instructions = disassembly.split("\n")
  .map(parseInstruction)
  .filter((instruction) => instruction &&
    instruction.address >= symbolAddress &&
    instruction.address < symbolAddress + nativeBytes)
  .sort((left, right) => left.address - right.address);
if (!instructions.length) throw new Error("objdump produced no Cpu::step instructions");

for (let index = 0; index < instructions.length; index++) {
  const instruction = instructions[index];
  instruction.offset = instruction.address - symbolAddress;
  instruction.end = index + 1 < instructions.length
    ? instructions[index + 1].address
    : symbolAddress + nativeBytes;
}

const targetPattern = /\b([0-9a-f]+)\s+<[^>]+>/;
const boundaries = new Set([symbolAddress]);
for (let index = 0; index < instructions.length; index++) {
  const instruction = instructions[index];
  const target = targetPattern.exec(instruction.operands);
  if (target) {
    const address = Number.parseInt(target[1], 16);
    if (address >= symbolAddress && address < symbolAddress + nativeBytes) {
      boundaries.add(address);
    }
  }
  if (/^(?:j\S*|ret|int3|ud2)$/.test(instruction.mnemonic) &&
      index + 1 < instructions.length) {
    boundaries.add(instructions[index + 1].address);
  }
}
const orderedBoundaries = [...boundaries].toSorted((a, b) => a - b);

const nativeBands = [
  ["entry-and-execute-tlb", 0x0000, 0x0091],
  ["physical-bus-fetch", 0x0091, 0x018b],
  ["length-and-rvc-dispatch", 0x018b, 0x0250],
  ["compressed-semantic-body", 0x0250, 0x165c],
  ["compressed-pc-store", 0x165c, 0x1664],
  ["compressed-common-outcome", 0x1664, 0x1675],
  ["compressed-retirement", 0x1675, 0x1689],
  ["compressed-return", 0x1689, 0x1698],
  ["rv32-semantic-body", 0x1698, 0x3e17],
  ["rv32-pc-store", 0x3e17, 0x3e22],
  ["rv32-common-outcome", 0x3e22, 0x3e33],
  ["rv32-retirement", 0x3e33, 0x3e47],
  ["rv32-return", 0x3e47, 0x3e57],
  ["uncommon-exits-and-tables", 0x3e57, nativeBytes],
].map(([name, begin, end]) => ({ name, begin, end, samples: 0, period: 0n }));
assert.equal(nativeBands[0].begin, 0);
assert.equal(nativeBands.at(-1).end, nativeBytes);
for (let index = 1; index < nativeBands.length; index++) {
  assert.equal(nativeBands[index - 1].end, nativeBands[index].begin, "band gap/overlap");
}

function containingInstruction(offset) {
  const address = symbolAddress + offset;
  let low = 0;
  let high = instructions.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const instruction = instructions[middle];
    if (address < instruction.address) high = middle - 1;
    else if (address >= instruction.end) low = middle + 1;
    else return instruction;
  }
  return null;
}

function containingBlock(address) {
  let low = 0;
  let high = orderedBoundaries.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (orderedBoundaries[middle] <= address) low = middle + 1;
    else high = middle - 1;
  }
  return orderedBoundaries[Math.max(0, high)];
}

const perfText = execFileSync(perf, [
  "script", "-G", "-i", perfData,
  "-F", "comm,pid,tid,period,ip,sym,symoff,dso",
], { encoding: "utf8", maxBuffer: 256 << 20 });

const targetDso = basename(stepDso);
let allSamples = 0;
let allPeriod = 0n;
let mainThreadPeriod = 0n;
let stepSamples = 0;
let stepPeriod = 0n;
const pcRows = new Map();
const blockRows = new Map();
for (const line of perfText.split("\n")) {
  const sample = parseSample(line);
  if (!sample) continue;
  allSamples++;
  allPeriod += sample.period;
  if (sample.comm === "node-MainThread" && sample.pid === sample.tid) {
    mainThreadPeriod += sample.period;
  }
  if (basename(sample.dso) !== targetDso ||
      !sample.symbol.includes("Cpu4step") ||
      !sample.symbol.includes("-turbofan")) continue;

  const offsetMatch = /\+0x([0-9a-f]+)$/.exec(sample.symbol);
  const offset = offsetMatch ? Number.parseInt(offsetMatch[1], 16) : 0;
  const band = nativeBands.find((entry) => offset >= entry.begin && offset < entry.end);
  if (!band) throw new Error(`sample offset 0x${offset.toString(16)} outside Cpu::step`);
  band.samples++;
  band.period += sample.period;
  stepSamples++;
  stepPeriod += sample.period;

  const instruction = containingInstruction(offset);
  if (!instruction) throw new Error(`no instruction contains sample 0x${offset.toString(16)}`);
  const pc = pcRows.get(instruction.offset) ?? {
    offset: instruction.offset,
    address: instruction.address,
    bytes: instruction.bytes.join(" "),
    mnemonic: instruction.mnemonic,
    operands: instruction.operands,
    samples: 0,
    period: 0n,
  };
  pc.samples++;
  pc.period += sample.period;
  pcRows.set(instruction.offset, pc);

  const blockAddress = containingBlock(instruction.address);
  const blockOffset = blockAddress - symbolAddress;
  const block = blockRows.get(blockOffset) ?? {
    offset: blockOffset,
    address: blockAddress,
    samples: 0,
    period: 0n,
  };
  block.samples++;
  block.period += sample.period;
  blockRows.set(blockOffset, block);
}

if (!allSamples || !stepSamples) throw new Error("perf contains no eligible samples");
assert.equal(
  nativeBands.reduce((sum, band) => sum + band.period, 0n),
  stepPeriod,
  "native bands do not close Cpu::step period",
);

function inSemanticBody(offset) {
  return (offset >= 0x0250 && offset < 0x165c) ||
    (offset >= 0x1698 && offset < 0x3e17);
}

function serializeHot(row) {
  return {
    offset: `0x${row.offset.toString(16)}`,
    samples: row.samples,
    period: row.period.toString(),
    fractionOfStep: fraction(row.period, stepPeriod),
    fractionOfAll: fraction(row.period, allPeriod),
    fractionOfMainThread: fraction(row.period, mainThreadPeriod),
    ...(row.mnemonic ? {
      instruction: `${row.mnemonic}${row.operands ? ` ${row.operands}` : ""}`,
      bytes: row.bytes,
    } : {}),
  };
}

const hotInstructions = [...pcRows.values()]
  .filter((row) => inSemanticBody(row.offset))
  .toSorted((left, right) => Number(right.period - left.period))
  .slice(0, 64)
  .map(serializeHot);
const hotBlocks = [...blockRows.values()]
  .filter((row) => inSemanticBody(row.offset))
  .toSorted((left, right) => Number(right.period - left.period))
  .slice(0, 48)
  .map((row) => {
    const nextBoundary = orderedBoundaries.find((address) => address > row.address) ??
      symbolAddress + nativeBytes;
    const blockInstructions = instructions.filter((instruction) =>
      instruction.address >= row.address && instruction.address < nextBoundary
    );
    return {
      ...serializeHot(row),
      end: `0x${(nextBoundary - symbolAddress).toString(16)}`,
      instructions: blockInstructions.map((instruction) =>
        `0x${instruction.offset.toString(16)} ${instruction.mnemonic}` +
        (instruction.operands ? ` ${instruction.operands}` : "")
      ),
    };
  });

const report = {
  schema: 1,
  experiment: "R122 immutable current-product interpreter-body native census",
  performanceEvidence: false,
  elapsedValuesExcluded: true,
  inputs: {
    perfData,
    perfDataSha256: sha256(perfData),
    stepDso,
    stepDsoSha256: sha256(stepDso),
    perf,
    readelf,
    objdump,
  },
  symbol: {
    address: `0x${symbolAddress.toString(16)}`,
    nativeBytes,
    decodedInstructions: instructions.length,
    basicBlockBoundaries: orderedBoundaries.length,
  },
  samples: {
    all: allSamples,
    allPeriod: allPeriod.toString(),
    mainThreadPeriod: mainThreadPeriod.toString(),
    step: stepSamples,
    stepPeriod: stepPeriod.toString(),
    stepFractionOfAll: fraction(stepPeriod, allPeriod),
    stepFractionOfMainThread: fraction(stepPeriod, mainThreadPeriod),
  },
  bands: nativeBands.map((band) => ({
    name: band.name,
    begin: `0x${band.begin.toString(16)}`,
    end: `0x${band.end.toString(16)}`,
    samples: band.samples,
    period: band.period.toString(),
    fractionOfStep: fraction(band.period, stepPeriod),
    fractionOfAll: fraction(band.period, allPeriod),
    fractionOfMainThread: fraction(band.period, mainThreadPeriod),
  })),
  hotInstructions,
  hotBlocks,
};

writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`R122_NATIVE_CENSUS ${JSON.stringify({
  output,
  samples: report.samples,
  bands: report.bands,
  topInstructions: hotInstructions.slice(0, 12),
  topBlocks: hotBlocks.slice(0, 12).map(({ instructions: _instructions, ...row }) => row),
})}\n`);
