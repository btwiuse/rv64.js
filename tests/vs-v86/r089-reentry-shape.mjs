#!/usr/bin/env node

// Prove that R089 changes the concrete Wasm call shape it was designed to
// change while preserving the direct calls in Cpu::run_until.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const [controlPath, candidatePath, outputPath] = process.argv.slice(2);
if (!controlPath || !candidatePath || !outputPath) {
  throw new Error("usage: r089-reentry-shape.mjs CONTROL.wasm CANDIDATE.wasm OUTPUT.json");
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

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

function functionBody(disassembly, marker) {
  const headers = [...disassembly.matchAll(/^([0-9a-f]+) <([^>]+)>:$/gm)];
  const matches = headers.filter((header) => header[2].includes(marker));
  if (matches.length !== 1) {
    throw new Error(`${marker}: expected one function, found ${matches.length}`);
  }
  const header = matches[0];
  const position = headers.indexOf(header);
  const end = position + 1 < headers.length ? headers[position + 1].index : disassembly.length;
  return disassembly.slice(header.index, end);
}

function count(body, pattern) {
  return [...body.matchAll(pattern)].length;
}

function inspect(path) {
  const bytes = readFileSync(path);
  if (!WebAssembly.validate(bytes)) throw new Error(`${path}: invalid Wasm`);
  const disassembly = disassemble(path);
  const runUntil = functionBody(disassembly, "3Cpu9run_until");
  const observed = functionBody(disassembly, "3Cpu18run_until_observed");
  return {
    path,
    sha256: sha256(bytes),
    bytes: bytes.length,
    imports: WebAssembly.Module.imports(new WebAssembly.Module(bytes)).length,
    exports: WebAssembly.Module.exports(new WebAssembly.Module(bytes)).length,
    runUntil: {
      callIndirect: count(runUntil, /\bcall_indirect\b/g),
      directCalls: count(runUntil, /\bcall\s+\d+/g),
      i64Loads: count(runUntil, /\bi64\.load\b/g),
      i64Comparisons: count(runUntil, /\bi64\.(?:eq|ne)\b/g),
      wrapIndexes: count(runUntil, /\bi32\.wrap_i64\b/g),
    },
    runUntilObserved: {
      callIndirect: count(observed, /\bcall_indirect\b/g),
      directCalls: count(observed, /\bcall\s+\d+/g),
    },
  };
}

const control = inspect(controlPath);
const candidate = inspect(candidatePath);
const problems = [];
const check = (condition, message) => {
  if (!condition) problems.push(message);
};

check(control.sha256 ===
  "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010",
"control is not exact R085");
check(control.runUntil.callIndirect === 2,
  `control run_until has ${control.runUntil.callIndirect}, not 2, call_indirect operations`);
check(control.runUntilObserved.callIndirect === 1,
  "control observed path does not have its expected indirect predicate");
check(candidate.runUntil.callIndirect === 0,
  "candidate run_until still contains call_indirect");
check(candidate.runUntilObserved.callIndirect === 0,
  "candidate observed path still contains call_indirect");
check(candidate.runUntil.directCalls === control.runUntil.directCalls,
  "candidate changed run_until direct-call count");
check(candidate.runUntil.i64Loads >= control.runUntil.i64Loads + 2,
  "candidate does not contain both inlined dispatch-tag loads");
check(candidate.runUntil.i64Comparisons >= control.runUntil.i64Comparisons + 2,
  "candidate does not contain both inlined dispatch-tag comparisons");
check(candidate.runUntil.wrapIndexes >= 2,
  "candidate does not contain both inlined dispatch indexes");
check(candidate.imports === control.imports && candidate.exports === control.exports,
  "candidate import/export counts changed");

const report = {
  schema: 1,
  experiment: "R089",
  mechanism: "monomorphic-exact-generated-entry-reentry",
  control,
  candidate,
  pass: problems.length === 0,
  problems,
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
if (problems.length) process.exitCode = 1;
