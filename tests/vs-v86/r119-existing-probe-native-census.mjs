#!/usr/bin/env node

// Attribute an immutable perf/JIT collection to native PC bands in the
// optimized full-system Cpu::step. This is diagnostic evidence only: perf and
// V8 JIT logging perturb execution, so this script never reports elapsed time.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { basename, resolve } from "node:path";
import { readFileSync, writeFileSync } from "node:fs";

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
  "target/bench/r119-existing-probe-opportunity/opportunity.json",
));
const perf = process.env.PERF || PERF_DEFAULT;
const readelf = process.env.READELF || "readelf";

const symbolTable = execFileSync(readelf, ["-Ws", stepDso], {
  encoding: "utf8",
  maxBuffer: 8 << 20,
});
const stepSymbolRow = symbolTable.split("\n").find((line) =>
  line.includes("Cpu4step") && line.includes("-turbofan")
);
if (!stepSymbolRow) throw new Error("step DSO has no optimized Cpu::step symbol");
const symbolMatch = /^\s*\d+:\s+[0-9a-f]+\s+(\d+)\s+FUNC\b/.exec(stepSymbolRow);
if (!symbolMatch) throw new Error("cannot parse Cpu::step symbol size");
const nativeBytes = Number(symbolMatch[1]);

const perfText = execFileSync(perf, [
  "script",
  "-G",
  "-i",
  perfData,
  "-F",
  "comm,pid,tid,period,ip,sym,symoff,dso",
], { encoding: "utf8", maxBuffer: 256 << 20 });

const samplePattern = /^\s*(\S+)\s+(\d+)\/(\d+)\s+(\d+)\s+([0-9a-f]+)\s+(.*?)\s+\((.*)\)\s*$/;
const targetDso = basename(stepDso);
const bands = [
  { name: "entry-and-execute-tlb", begin: 0x0000, end: 0x0091, period: 0n, samples: 0 },
  { name: "physical-bus-fetch", begin: 0x0091, end: 0x018b, period: 0n, samples: 0 },
  { name: "length-and-rvc-dispatch", begin: 0x018b, end: 0x0250, period: 0n, samples: 0 },
  { name: "compressed-body", begin: 0x0250, end: 0x1698, period: 0n, samples: 0 },
  { name: "rv32-body-and-exit", begin: 0x1698, end: nativeBytes, period: 0n, samples: 0 },
];

let allPeriod = 0n;
let mainThreadPeriod = 0n;
let allSamples = 0;
let stepPeriod = 0n;
let stepSamples = 0;
for (const line of perfText.split("\n")) {
  const match = samplePattern.exec(line);
  if (!match) continue;
  const comm = match[1];
  const pid = Number(match[2]);
  const tid = Number(match[3]);
  const period = BigInt(match[4]);
  const symbol = match[6];
  const dso = basename(match[7]);
  allSamples++;
  allPeriod += period;
  if (comm === "node-MainThread" && pid === tid) mainThreadPeriod += period;
  if (dso !== targetDso || !symbol.includes("Cpu4step") || !symbol.includes("-turbofan")) {
    continue;
  }
  const offsetMatch = /\+0x([0-9a-f]+)$/.exec(symbol);
  const offset = offsetMatch ? Number.parseInt(offsetMatch[1], 16) : 0;
  const band = bands.find((entry) => offset >= entry.begin && offset < entry.end);
  if (!band) throw new Error(`step sample offset 0x${offset.toString(16)} is outside symbol`);
  band.period += period;
  band.samples++;
  stepPeriod += period;
  stepSamples++;
}
if (allSamples === 0 || stepSamples === 0) throw new Error("perf produced no eligible samples");
if (bands.reduce((sum, band) => sum + band.period, 0n) !== stepPeriod) {
  throw new Error("native PC bands do not partition Cpu::step period");
}

const fraction = (value, total) => Number(value) / Number(total);
const report = {
  schema: 1,
  experiment: "R119 existing-probe fused execute-TLB opportunity census",
  performanceEvidence: false,
  inputs: {
    perfData,
    perfDataSha256: sha256(perfData),
    stepDso,
    stepDsoSha256: sha256(stepDso),
    perf,
    readelf,
  },
  nativeBytes,
  samples: {
    all: allSamples,
    allPeriod: allPeriod.toString(),
    mainThreadPeriod: mainThreadPeriod.toString(),
    step: stepSamples,
    stepPeriod: stepPeriod.toString(),
    stepFractionOfAll: fraction(stepPeriod, allPeriod),
    stepFractionOfMainThread: fraction(stepPeriod, mainThreadPeriod),
  },
  bands: bands.map((band) => ({
    name: band.name,
    begin: `0x${band.begin.toString(16)}`,
    end: `0x${band.end.toString(16)}`,
    samples: band.samples,
    period: band.period.toString(),
    fractionOfStep: fraction(band.period, stepPeriod),
    fractionOfAll: fraction(band.period, allPeriod),
    fractionOfMainThread: fraction(band.period, mainThreadPeriod),
  })),
};

writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`R119_NATIVE_CENSUS ${JSON.stringify({
  output,
  nativeBytes,
  samples: report.samples,
  bands: report.bands,
})}\n`);
