#!/usr/bin/env node

// Attribute the immutable R110 perf collection to exact native control-flow
// bands in optimized Virt run_system_jit. This is diagnostic evidence only;
// perf/JIT logging perturbs the worker, so elapsed time is never reported.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { basename, resolve } from "node:path";
import { readFileSync, writeFileSync } from "node:fs";
import assert from "node:assert/strict";

const PERF_DEFAULT =
  "/nix/store/cavgh13ks5f36c4arsbc6r79rajryblf-perf-linux-7.1.7/bin/perf";

const EXPECTED = Object.freeze({
  perfDataSha256: "6b976f6ebe1b38b6a41b451eff4178cc9313e13a8fc5b29d5588f2bda24d162b",
  schedulerDsoSha256: "5febd1122f8c440958607f119082656b6a0d9ed91fdc9f6564208fccab46a8d6",
  nativeBytes: 66_432,
  symbolAddress: 0x80,
  allSamples: 34_753,
  allPeriod: 71_241_130_879n,
  mainThreadPeriod: 31_684_540_743n,
  schedulerSamples: 381,
  schedulerPeriod: 825_328_724n,
});

const BAND_SPECS = Object.freeze([
  ["pre-dispatch-setup-policy", 0x0000, 0x351c],
  ["dispatch-common-and-line-lookup", 0x351c, 0x3652],
  ["fallback-cache-mapping-verify-refill", 0x3652, 0x3fbf],
  ["ordinary-block-call-entry", 0x3fbf, 0x4072],
  ["ordinary-block-postcall-feedback", 0x4072, 0x51b2],
  ["region-call-entry", 0x51b2, 0x5273],
  ["common-postcall-and-diagnostics", 0x5273, 0x5a73],
  ["dispatch-break-and-exit", 0x5a73, 0x5c0a],
  ["post-dispatch-policy-interpreter", 0x5c0a, 0x10380],
]);

function argument(name, fallback = null) {
  const prefix = `--${name}=`;
  const value = process.argv.find((entry) => entry.startsWith(prefix));
  return value ? value.slice(prefix.length) : fallback;
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function makeBands() {
  return BAND_SPECS.map(([name, begin, end]) => ({
    name,
    begin,
    end,
    samples: 0,
    period: 0n,
  }));
}

function bandForOffset(bands, offset) {
  return bands.find((band) => offset >= band.begin && offset < band.end) ?? null;
}

const samplePattern =
  /^\s*(.*?)\s+(\d+)\/(\d+)\s+([0-9.]+):\s+(\d+)\s+([0-9a-f]+)\s+(.*?)\s+\((.*)\)\s*$/;

function parseSample(line) {
  const match = samplePattern.exec(line);
  if (!match) return null;
  return {
    comm: match[1].trim(),
    pid: Number(match[2]),
    tid: Number(match[3]),
    time: Number(match[4]),
    period: BigInt(match[5]),
    ip: Number.parseInt(match[6], 16),
    symbol: match[7],
    dso: match[8],
  };
}

function symbolOffset(symbol) {
  const match = /\+0x([0-9a-f]+)$/.exec(symbol);
  return match ? Number.parseInt(match[1], 16) : null;
}

function fraction(value, total) {
  return total === 0n ? 0 : Number(value) / Number(total);
}

function selftest() {
  const bands = makeBands();
  assert.equal(bandForOffset(bands, 0)?.name, "pre-dispatch-setup-policy");
  assert.equal(
    bandForOffset(bands, 0x351b)?.name,
    "pre-dispatch-setup-policy",
  );
  assert.equal(
    bandForOffset(bands, 0x351c)?.name,
    "dispatch-common-and-line-lookup",
  );
  assert.equal(
    bandForOffset(bands, 0x3652)?.name,
    "fallback-cache-mapping-verify-refill",
  );
  assert.equal(
    bandForOffset(bands, EXPECTED.nativeBytes - 1)?.name,
    "post-dispatch-policy-interpreter",
  );
  assert.equal(bandForOffset(bands, EXPECTED.nativeBytes), null);
  for (let index = 1; index < bands.length; index++) {
    assert.equal(bands[index - 1].end, bands[index].begin);
  }
  assert.equal(bands[0].begin, 0);
  assert.equal(bands.at(-1).end, EXPECTED.nativeBytes);

  const sample = parseSample(
    " node-MainThread  42/42  123.500000:  999  abc " +
      "JS:run_system_jit-28-turbofan+0x3652 (/tmp/jitted-42.so)",
  );
  assert.equal(sample.comm, "node-MainThread");
  assert.equal(sample.period, 999n);
  assert.equal(symbolOffset(sample.symbol), 0x3652);
  assert.equal(parseSample("not a perf sample"), null);
  process.stdout.write("PASS R121 runtime-dispatch native census selftest\n");
}

if (process.argv.includes("--selftest")) {
  selftest();
  process.exit(0);
}

const perfData = resolve(argument(
  "perf-data",
  "target/bench/r110-optimized-native-census/perf.jitted.data",
));
const schedulerDso = resolve(argument(
  "scheduler-dso",
  "target/bench/r110-optimized-native-census/jitted-292893-2434.so",
));
const output = resolve(argument(
  "output",
  "target/bench/r121-runtime-dispatch/native-census.json",
));
const perf = process.env.PERF || PERF_DEFAULT;
const readelf = process.env.READELF || "readelf";
const objdump = process.env.OBJDUMP || "objdump";

assert.equal(sha256(perfData), EXPECTED.perfDataSha256, "unexpected perf data");
assert.equal(
  sha256(schedulerDso),
  EXPECTED.schedulerDsoSha256,
  "unexpected scheduler DSO",
);

const symbols = execFileSync(readelf, ["-Ws", schedulerDso], {
  encoding: "utf8",
  maxBuffer: 8 << 20,
});
const symbolLine = symbols.split("\n").find((line) =>
  line.includes("run_system_jit") &&
  line.includes("VirtMachine") &&
  line.includes("-turbofan")
);
if (!symbolLine) throw new Error("scheduler DSO lacks optimized Virt run_system_jit");
const symbolMatch =
  /^\s*\d+:\s+([0-9a-f]+)\s+(\d+)\s+FUNC\b/.exec(symbolLine);
if (!symbolMatch) throw new Error("cannot parse scheduler symbol row");
const symbolAddress = Number.parseInt(symbolMatch[1], 16);
const nativeBytes = Number(symbolMatch[2]);
assert.equal(symbolAddress, EXPECTED.symbolAddress, "unexpected symbol address");
assert.equal(nativeBytes, EXPECTED.nativeBytes, "unexpected scheduler size");

const disassembly = execFileSync(objdump, [
  "-d",
  "--no-show-raw-insn",
  schedulerDso,
], { encoding: "utf8", maxBuffer: 32 << 20 });
const instructions = [];
for (const line of disassembly.split("\n")) {
  const match = /^\s*([0-9a-f]+):\s+([a-z][a-z0-9.]*)\s*(.*)$/.exec(line);
  if (!match) continue;
  const address = Number.parseInt(match[1], 16);
  if (address < symbolAddress || address >= symbolAddress + nativeBytes) continue;
  instructions.push({
    address,
    offset: address - symbolAddress,
    mnemonic: match[2],
    operands: match[3].trim(),
  });
}
if (instructions.length === 0) throw new Error("objdump produced no instructions");

function instructionAt(offset) {
  let low = 0;
  let high = instructions.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (instructions[middle].offset <= offset) low = middle + 1;
    else high = middle;
  }
  return low === 0 ? null : instructions[low - 1];
}

const perfText = execFileSync(perf, [
  "script",
  "-G",
  "-i",
  perfData,
  "-F",
  "comm,pid,tid,time,period,ip,sym,symoff,dso",
], { encoding: "utf8", maxBuffer: 256 << 20 });

const samples = [];
for (const line of perfText.split("\n")) {
  const sample = parseSample(line);
  if (sample) samples.push(sample);
}
if (samples.length === 0) throw new Error("perf produced no samples");

const targetDso = basename(schedulerDso);
const bands = makeBands();
const offsetTotals = new Map();
let allPeriod = 0n;
let mainThreadPeriod = 0n;
let schedulerPeriod = 0n;
let schedulerSamples = 0;
let firstTime = Number.POSITIVE_INFINITY;
let lastTime = Number.NEGATIVE_INFINITY;

for (const sample of samples) {
  allPeriod += sample.period;
  firstTime = Math.min(firstTime, sample.time);
  lastTime = Math.max(lastTime, sample.time);
  if (sample.comm === "node-MainThread" && sample.pid === sample.tid) {
    mainThreadPeriod += sample.period;
  }
  if (basename(sample.dso) !== targetDso) continue;
  if (!sample.symbol.includes("run_system_jit") || !sample.symbol.includes("-turbofan")) {
    throw new Error(`unexpected symbol in scheduler DSO: ${sample.symbol}`);
  }
  const offset = symbolOffset(sample.symbol);
  if (offset === null) throw new Error(`scheduler sample lacks offset: ${sample.symbol}`);
  const band = bandForOffset(bands, offset);
  if (!band) throw new Error(`scheduler offset 0x${offset.toString(16)} is outside symbol`);
  band.samples++;
  band.period += sample.period;
  schedulerSamples++;
  schedulerPeriod += sample.period;
  const total = offsetTotals.get(offset) ?? { samples: 0, period: 0n };
  total.samples++;
  total.period += sample.period;
  offsetTotals.set(offset, total);
}

assert.equal(samples.length, EXPECTED.allSamples, "R110 sample-count drift");
assert.equal(allPeriod, EXPECTED.allPeriod, "R110 period drift");
assert.equal(mainThreadPeriod, EXPECTED.mainThreadPeriod, "R110 main-thread drift");
assert.equal(schedulerSamples, EXPECTED.schedulerSamples, "scheduler sample drift");
assert.equal(schedulerPeriod, EXPECTED.schedulerPeriod, "scheduler period drift");
assert.equal(
  bands.reduce((sum, band) => sum + band.samples, 0),
  schedulerSamples,
  "band sample closure failed",
);
assert.equal(
  bands.reduce((sum, band) => sum + band.period, 0n),
  schedulerPeriod,
  "band period closure failed",
);

const binCount = Math.ceil(lastTime - firstTime) + 1;
const temporalBins = Array.from({ length: binCount }, (_, index) => ({
  index,
  beginSeconds: index,
  endSeconds: index + 1,
  allPeriod: 0n,
  mainThreadPeriod: 0n,
  schedulerSamples: 0,
  schedulerPeriod: 0n,
  fallbackPeriod: 0n,
}));
for (const sample of samples) {
  const bin = temporalBins[Math.floor(sample.time - firstTime)];
  bin.allPeriod += sample.period;
  if (sample.comm === "node-MainThread" && sample.pid === sample.tid) {
    bin.mainThreadPeriod += sample.period;
  }
  if (basename(sample.dso) !== targetDso) continue;
  const offset = symbolOffset(sample.symbol);
  bin.schedulerSamples++;
  bin.schedulerPeriod += sample.period;
  if (offset >= 0x3652 && offset < 0x3fbf) bin.fallbackPeriod += sample.period;
}
assert.equal(
  temporalBins.reduce((sum, bin) => sum + bin.allPeriod, 0n),
  allPeriod,
  "temporal all-period closure failed",
);
assert.equal(
  temporalBins.reduce((sum, bin) => sum + bin.schedulerPeriod, 0n),
  schedulerPeriod,
  "temporal scheduler-period closure failed",
);

const hotOffsets = [...offsetTotals.entries()]
  .sort((left, right) => {
    if (left[1].period === right[1].period) return left[0] - right[0];
    return left[1].period > right[1].period ? -1 : 1;
  })
  .slice(0, 32)
  .map(([offset, total]) => {
    const instruction = instructionAt(offset);
    return {
      offset: `0x${offset.toString(16)}`,
      address: `0x${(symbolAddress + offset).toString(16)}`,
      instructionOffset: instruction ? `0x${instruction.offset.toString(16)}` : null,
      mnemonic: instruction?.mnemonic ?? null,
      operands: instruction?.operands ?? null,
      band: bandForOffset(bands, offset).name,
      samples: total.samples,
      period: total.period.toString(),
      fractionOfScheduler: fraction(total.period, schedulerPeriod),
    };
  });

const report = {
  schema: 1,
  experiment: "R121 optimized runtime-dispatch native attribution",
  performanceEvidence: false,
  phaseMarkersAvailable: false,
  inputs: {
    perfData,
    perfDataSha256: sha256(perfData),
    schedulerDso,
    schedulerDsoSha256: sha256(schedulerDso),
    perf,
    readelf,
    objdump,
  },
  symbol: {
    name: symbolLine.trim().split(/\s+/).at(-1),
    address: `0x${symbolAddress.toString(16)}`,
    nativeBytes,
  },
  samples: {
    all: samples.length,
    allPeriod: allPeriod.toString(),
    mainThreadPeriod: mainThreadPeriod.toString(),
    scheduler: schedulerSamples,
    schedulerPeriod: schedulerPeriod.toString(),
    schedulerFractionOfAll: fraction(schedulerPeriod, allPeriod),
    schedulerFractionOfMainThread: fraction(schedulerPeriod, mainThreadPeriod),
  },
  bands: bands.map((band) => ({
    name: band.name,
    begin: `0x${band.begin.toString(16)}`,
    end: `0x${band.end.toString(16)}`,
    samples: band.samples,
    period: band.period.toString(),
    fractionOfScheduler: fraction(band.period, schedulerPeriod),
    fractionOfAll: fraction(band.period, allPeriod),
    fractionOfMainThread: fraction(band.period, mainThreadPeriod),
  })),
  temporalBins: temporalBins.map((bin) => ({
    index: bin.index,
    beginSeconds: bin.beginSeconds,
    endSeconds: bin.endSeconds,
    allPeriod: bin.allPeriod.toString(),
    mainThreadPeriod: bin.mainThreadPeriod.toString(),
    schedulerSamples: bin.schedulerSamples,
    schedulerPeriod: bin.schedulerPeriod.toString(),
    fallbackPeriod: bin.fallbackPeriod.toString(),
    schedulerFractionOfMainThread: fraction(bin.schedulerPeriod, bin.mainThreadPeriod),
    fallbackFractionOfMainThread: fraction(bin.fallbackPeriod, bin.mainThreadPeriod),
  })),
  hotOffsets,
  closure: {
    bandSamples: bands.reduce((sum, band) => sum + band.samples, 0),
    bandPeriod: bands.reduce((sum, band) => sum + band.period, 0n).toString(),
    temporalAllPeriod: temporalBins
      .reduce((sum, bin) => sum + bin.allPeriod, 0n)
      .toString(),
    temporalSchedulerPeriod: temporalBins
      .reduce((sum, bin) => sum + bin.schedulerPeriod, 0n)
      .toString(),
  },
};

writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`R121_NATIVE_CENSUS ${JSON.stringify({
  output,
  samples: report.samples,
  bands: report.bands,
})}\n`);
