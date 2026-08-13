#!/usr/bin/env node

// Deterministically attribute the always-enabled R054 diagnostic guard in the
// immutable R119 optimized Cpu::ld/st native bodies. This is exposure only;
// perf/JIT-logging elapsed values are excluded.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { basename, resolve } from "node:path";
import { readFileSync, writeFileSync } from "node:fs";

const PERF_DEFAULT =
  "/nix/store/cavgh13ks5f36c4arsbc6r79rajryblf-perf-linux-7.1.7/bin/perf";

function argument(name, fallback) {
  const prefix = `--${name}=`;
  return process.argv.find((value) => value.startsWith(prefix))?.slice(prefix.length) ?? fallback;
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
    symbol: match[6],
    dso: match[7],
  };
}

function selftest() {
  const instruction = parseInstruction(
    " 3e7:\t42 80 bc 27 d6 0a 05 00 01 \tcmp BYTE PTR [rdi+r12*1+0x50ad6],0x1",
  );
  assert.equal(instruction.address, 0x3e7);
  assert.equal(instruction.mnemonic, "cmp");
  assert.equal(parseInstruction("  3ee:\t00 01 "), null);
  const sample = parseSample(
    " node-MainThread  7/7  1234  abc JS:Cpu2ld-turbofan+0x35e (/tmp/jitted.so)",
  );
  assert.equal(sample.period, 1234n);
  assert.equal(sample.symbol, "JS:Cpu2ld-turbofan+0x35e");
  process.stdout.write("R123 fused-memory guard census selftest: PASS\n");
}

if (process.argv.includes("--selftest")) {
  selftest();
  process.exit(0);
}

const root = resolve(argument("root", "."));
const perfData = resolve(root, argument(
  "perf-data",
  "target/bench/r119-existing-probe-opportunity/perf.jitted.data",
));
const output = resolve(root, argument(
  "output",
  "target/bench/r123-fused-memory-static-guard/native-census.json",
));
const perf = process.env.PERF ?? PERF_DEFAULT;
const readelf = process.env.READELF ?? "readelf";
const objdump = process.env.OBJDUMP ?? "objdump";
const base = "target/bench/r119-existing-probe-opportunity";

const bodies = [
  {
    name: "ld1", file: "jitted-384587-2428.so",
    sha256: "6bb9919ffc99c73452c4f58ce781a29f7dea27dbf823f6a7f4fe8f1e3996f7ad",
    symbol: /Cpu2ld.*VirtBusKm1_.*-turbofan$/, guardBegin: 0x0033, guardEnd: 0x004f,
  },
  {
    name: "ld4", file: "jitted-384587-2427.so",
    sha256: "475f06161e1b73159727cc0f52684a209cccf71284e879c62d8f42dbd47a2869",
    symbol: /Cpu2ld.*VirtBusKm4_.*-turbofan$/, guardBegin: 0x0700, guardEnd: 0x0718,
  },
  {
    name: "ld8", file: "jitted-384587-2429.so",
    sha256: "5096d4c3a9e9603e7f3d17f37f00dd02ee29e9b4812930699f85bf1927727c55",
    symbol: /Cpu2ld.*VirtBusKm8_.*-turbofan$/, guardBegin: 0x035e, guardEnd: 0x0376,
  },
  {
    name: "st4", file: "jitted-384587-2433.so",
    sha256: "0e8f127f7d2031d9c9067392ee09c65a79bcb8bf5c439e9bc182586e26de4d1c",
    symbol: /Cpu2st.*VirtBusKm4_.*-turbofan$/, guardBegin: 0x04ee, guardEnd: 0x0506,
  },
  {
    name: "st8", file: "jitted-384587-2412.so",
    sha256: "109df5d60641e4100f288d79702fba1a1e489c35efee34eb1ebbdc98b97525cb",
    symbol: /Cpu2st.*VirtBusKm8_.*-turbofan$/, guardBegin: 0x02b1, guardEnd: 0x02c9,
  },
].map((body) => ({
  ...body,
  path: resolve(root, base, body.file),
  samples: 0,
  period: 0n,
  guardSamples: 0,
  guardPeriod: 0n,
  guardPcs: new Map(),
}));

assert.equal(
  sha256(perfData),
  "24028310d77f5f8c5be6e5cb560e20d8c051b010a5b9fe078cc41fa73a691a2c",
  "R119 perf data changed",
);

for (const body of bodies) {
  assert.equal(sha256(body.path), body.sha256, `${body.name} DSO changed`);
  const symbols = execFileSync(readelf, ["-Ws", body.path], {
    encoding: "utf8",
    maxBuffer: 4 << 20,
  });
  const symbolLine = symbols.split("\n").find((line) => body.symbol.test(line.trim().split(/\s+/).at(-1)));
  assert(symbolLine, `${body.name}: optimized symbol absent`);
  const symbolMatch = /^\s*\d+:\s+([0-9a-f]+)\s+(\d+)\s+FUNC\b/.exec(symbolLine);
  assert(symbolMatch, `${body.name}: malformed symbol row`);
  body.symbolAddress = Number.parseInt(symbolMatch[1], 16);
  body.nativeBytes = Number(symbolMatch[2]);

  const instructions = execFileSync(objdump, ["-d", "-Mintel", body.path], {
    encoding: "utf8",
    maxBuffer: 8 << 20,
  }).split("\n").map(parseInstruction).filter((instruction) => instruction &&
    instruction.address >= body.symbolAddress + body.guardBegin &&
    instruction.address < body.symbolAddress + body.guardEnd);
  const removable = instructions.filter((instruction) =>
    instruction.operands.includes("0x50ad6") || /^j(?:e|ne)$/.test(instruction.mnemonic)
  );
  assert.equal(removable.length, 3, `${body.name}: guard must have three removable instructions`);
  assert.equal(removable[0].mnemonic, "movzx", `${body.name}: flag load changed`);
  assert(removable[0].operands.includes("0x50ad6"), `${body.name}: flag cell changed`);
  assert.equal(removable[1].mnemonic, "cmp", `${body.name}: flag compare changed`);
  assert(removable[1].operands.includes("0x50ad6"), `${body.name}: compared cell changed`);
  assert(removable[1].operands.endsWith(",0x1"), `${body.name}: enabled value changed`);
  assert(/^j(?:e|ne)$/.test(removable[2].mnemonic), `${body.name}: guard branch changed`);
  body.guardInstructions = removable.map((instruction) =>
    `0x${(instruction.address - body.symbolAddress).toString(16)} ` +
    `${instruction.mnemonic} ${instruction.operands}`
  );
}

const perfText = execFileSync(perf, [
  "script", "-G", "-i", perfData,
  "-F", "comm,pid,tid,period,ip,sym,symoff,dso",
], { encoding: "utf8", maxBuffer: 256 << 20 });

let allPeriod = 0n;
let mainThreadPeriod = 0n;
let memorySamples = 0;
let memoryPeriod = 0n;
for (const line of perfText.split("\n")) {
  const sample = parseSample(line);
  if (!sample) continue;
  allPeriod += sample.period;
  if (sample.comm === "node-MainThread" && sample.pid === sample.tid) {
    mainThreadPeriod += sample.period;
  }
  if (!sample.symbol.includes("Cpu2ld") && !sample.symbol.includes("Cpu2st")) continue;
  if (!sample.symbol.includes("-turbofan")) {
    throw new Error(`unexpected non-optimized memory sample: ${sample.symbol}`);
  }
  const body = bodies.find((entry) => basename(sample.dso) === entry.file &&
    entry.symbol.test(sample.symbol.replace(/\+0x[0-9a-f]+$/, "")));
  if (!body) throw new Error(`unclassified optimized memory sample: ${sample.symbol}`);
  const offsetMatch = /\+0x([0-9a-f]+)$/.exec(sample.symbol);
  assert(offsetMatch, `${body.name}: sample lacks symbol offset`);
  const offset = Number.parseInt(offsetMatch[1], 16);
  assert(offset < body.nativeBytes, `${body.name}: sample outside symbol`);
  body.samples++;
  body.period += sample.period;
  memorySamples++;
  memoryPeriod += sample.period;
  if (offset >= body.guardBegin && offset < body.guardEnd) {
    body.guardSamples++;
    body.guardPeriod += sample.period;
    body.guardPcs.set(offset, (body.guardPcs.get(offset) ?? 0n) + sample.period);
  }
}

const guardPeriod = bodies.reduce((sum, body) => sum + body.guardPeriod, 0n);
assert(memoryPeriod > 0n && mainThreadPeriod > 0n, "missing native samples");
assert.equal(
  bodies.reduce((sum, body) => sum + body.period, 0n),
  memoryPeriod,
  "memory body period does not close",
);

const report = {
  schema: 1,
  experiment: "R123 immutable fused-memory runtime-guard native census",
  performanceEvidence: false,
  elapsedValuesExcluded: true,
  inputs: {
    perfData,
    perfDataSha256: sha256(perfData),
    perf,
    readelf,
    objdump,
  },
  samples: {
    allPeriod: allPeriod.toString(),
    mainThreadPeriod: mainThreadPeriod.toString(),
    optimizedMemoryBodies: memorySamples,
    optimizedMemoryPeriod: memoryPeriod.toString(),
    optimizedMemoryFractionOfMainThread: fraction(memoryPeriod, mainThreadPeriod),
    guardPeriod: guardPeriod.toString(),
    guardFractionOfMemoryBodies: fraction(guardPeriod, memoryPeriod),
    guardFractionOfMainThread: fraction(guardPeriod, mainThreadPeriod),
    optimisticWholeBootSpeedup: 1 / (1 - fraction(guardPeriod, mainThreadPeriod)),
  },
  bodies: bodies.map((body) => ({
    name: body.name,
    dso: body.path,
    dsoSha256: body.sha256,
    nativeBytes: body.nativeBytes,
    guardBegin: `0x${body.guardBegin.toString(16)}`,
    guardEnd: `0x${body.guardEnd.toString(16)}`,
    guardInstructions: body.guardInstructions,
    samples: body.samples,
    period: body.period.toString(),
    fractionOfMainThread: fraction(body.period, mainThreadPeriod),
    guardSamples: body.guardSamples,
    guardPeriod: body.guardPeriod.toString(),
    guardFractionOfMainThread: fraction(body.guardPeriod, mainThreadPeriod),
    guardPcs: [...body.guardPcs.entries()].map(([offset, period]) => ({
      offset: `0x${offset.toString(16)}`,
      period: period.toString(),
    })),
  })),
  admission: {
    requiredMainThreadFraction: 0.0125,
    observedMainThreadFraction: fraction(guardPeriod, mainThreadPeriod),
    passesExposure: fraction(guardPeriod, mainThreadPeriod) >= 0.0125,
    candidateAdmitted: false,
  },
};

writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
process.stdout.write(`R123_FUSED_GUARD_CENSUS ${JSON.stringify({
  output,
  samples: report.samples,
  admission: report.admission,
})}\n`);
