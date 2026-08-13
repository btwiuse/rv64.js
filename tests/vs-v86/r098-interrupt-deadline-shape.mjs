#!/usr/bin/env node

// Prove that R098 changes each concrete interpreter driver's hot interrupt
// state from a decrement/store countdown to one rare deadline store, without
// relying on source-level intent.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const [controlPath, candidatePath, outputPath] = process.argv.slice(2);
if (!controlPath || !candidatePath || !outputPath) {
  throw new Error(
    "usage: r098-interrupt-deadline-shape.mjs CONTROL.wasm CANDIDATE.wasm OUTPUT.json",
  );
}

const CPU_INTERRUPT_CELL_OFFSET = 330448;
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const occurrences = (text, pattern) => [...text.matchAll(pattern)].length;

function disassemble(path) {
  const child = spawnSync("llvm-objdump", ["-d", path], {
    encoding: "utf8",
    maxBuffer: 128 << 20,
  });
  if (child.status !== 0) {
    throw new Error(child.stderr || child.stdout || `cannot disassemble ${path}`);
  }
  return child.stdout;
}

function functionBodies(disassembly) {
  const headers = [...disassembly.matchAll(/^([0-9a-f]+) <([^>]+)>:$/gm)];
  return headers.map((header, index) => ({
    name: header[2],
    body: disassembly.slice(
      header.index,
      index + 1 < headers.length ? headers[index + 1].index : disassembly.length,
    ),
  }));
}

const methods = [
  ["run", "3Cpu3run"],
  ["run_until", "3Cpu9run_until"],
  ["run_until_observed", "3Cpu18run_until_observed"],
  ["run_traced", "3Cpu10run_traced"],
];

function inspect(path) {
  const bytes = readFileSync(path);
  if (!WebAssembly.validate(bytes)) throw new Error(`${path}: invalid Wasm`);
  const bodies = functionBodies(disassemble(path));
  const selected = Object.fromEntries(
    methods.map(([label, marker]) => [
      label,
      bodies
        .filter(({ name }) => name.includes(marker))
        .map(({ name, body }) => ({
          name,
          interruptCellLoads: occurrences(
            body,
            new RegExp(`\\bi32\\.load\\s+${CPU_INTERRUPT_CELL_OFFSET}\\b`, "g"),
          ),
          interruptCellStores: occurrences(
            body,
            new RegExp(`\\bi32\\.store\\s+${CPU_INTERRUPT_CELL_OFFSET}\\b`, "g"),
          ),
          signedDueComparisons: occurrences(body, /\bi32\.(?:le_s|lt_s)\b/g),
          directCalls: occurrences(body, /\bcall\s+\d+/g),
          indirectCalls: occurrences(body, /\bcall_indirect\b/g),
        })),
    ]),
  );
  return {
    path,
    sha256: sha256(bytes),
    bytes: bytes.length,
    methods: selected,
  };
}

const control = inspect(controlPath);
const candidate = inspect(candidatePath);
const problems = [];
const check = (condition, message) => {
  if (!condition) problems.push(message);
};

check(
  control.sha256 === "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
  "control is not the exact R085-equivalent source build",
);
check(
  candidate.sha256 === "b2e2831bb7851f6ce0c2cd58fba6a9f6f78e77a9e7c428192ed3270968553453",
  "candidate is not the frozen R098 build",
);

for (const [label] of methods) {
  const before = control.methods[label];
  const after = candidate.methods[label];
  check(before.length > 0, `${label}: no control body found`);
  check(after.length === before.length, `${label}: concrete body count changed`);
  for (let index = 0; index < Math.min(before.length, after.length); index += 1) {
    const a = before[index];
    const b = after[index];
    check(a.interruptCellLoads === 1, `${label}[${index}]: control cell load count is not one`);
    check(a.interruptCellStores === 2, `${label}[${index}]: control cell store count is not two`);
    check(b.interruptCellLoads === 1, `${label}[${index}]: candidate cell load count is not one`);
    check(b.interruptCellStores === 1, `${label}[${index}]: candidate cell store count is not one`);
    check(
      b.signedDueComparisons >= a.signedDueComparisons + 1,
      `${label}[${index}]: candidate deadline comparison is absent`,
    );
    check(a.directCalls === b.directCalls, `${label}[${index}]: direct-call count changed`);
    check(a.indirectCalls === b.indirectCalls, `${label}[${index}]: indirect-call count changed`);
  }
}

const report = {
  schema: 1,
  experiment: "R098",
  mechanism: "absolute-interrupt-poll-deadline",
  cpuInterruptCellOffset: CPU_INTERRUPT_CELL_OFFSET,
  control,
  candidate,
  pass: problems.length === 0,
  problems,
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
if (problems.length) process.exitCode = 1;
