#!/usr/bin/env node

// Summarize phase-local JIT deltas from a WANIX candidate/control result set.
// This is attribution-only: it does not score, validate, or alter a protocol.

import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";

const directoryArgument = process.argv[2];
if (!directoryArgument || process.argv.length !== 3) {
  throw new Error("usage: node tests/vs-v86/analyze-browser-jit-deltas.mjs RESULTS_DIR");
}
const directory = resolve(directoryArgument);
const phases = ["python", "sha256", "shared9p"];
const fields = [
  "instructions",
  "generatedRetired",
  "interpreterRetired",
  "interpreterCalls",
  "staticT0FastRetired",
  "sampledStaticT0Retired",
  "sampledStaticT0Samples",
  "sampledStaticT0InterruptPolls",
  "sampledStaticT0ShortMarks",
  "sampledStaticT0ShortBypasses",
  "sampledStaticT0ShortClears",
  "pagePolicyUserRetired",
  "pagePolicyPrivilegedRetired",
  "candidates",
  "issued",
  "modules",
  "compileMs",
  "p9Requests",
  "p9HostMs",
];

function median(values) {
  const ordered = [...values].sort((left, right) => left - right);
  return ordered[Math.floor(ordered.length / 2)];
}

const names = readdirSync(directory);
const pairNumbers = [...new Set(names.flatMap((name) => {
  const match = /^pair-(\d+)-(?:control|candidate)\.log$/.exec(name);
  return match ? [Number(match[1])] : [];
}))].sort((left, right) => left - right);

function load(pair, side) {
  const lines = readFileSync(join(directory, `pair-${pair}-${side}.log`), "utf8")
    .split("\n");
  return { summary: JSON.parse(lines[0]), jit: JSON.parse(lines[1]).jitPhases };
}

const runs = pairNumbers.map((pair) => ({
  pair,
  control: load(pair, "control"),
  candidate: load(pair, "candidate"),
}));
const result = { pairs: pairNumbers.length, phases: {} };
for (const phase of phases) {
  const phaseResult = {};
  for (const side of ["control", "candidate"]) {
    const legMedians = runs.map((run) => {
      const repetitions = run[side].summary.samples[phase].length;
      const values = {
        elapsed: median(run[side].summary.samples[phase]),
      };
      for (const field of fields) {
        values[field] = median(Array.from({ length: repetitions }, (_, index) => {
          const value = run[side].jit[`${phase}${index + 1}`][field] ?? 0;
          return Number(value);
        }));
      }
      return values;
    });
    phaseResult[side] = Object.fromEntries(
      ["elapsed", ...fields].map((field) => [
        field,
        median(legMedians.map((leg) => leg[field])),
      ]),
    );
  }
  phaseResult.candidateOverControl = Object.fromEntries(
    ["elapsed", ...fields].map((field) => [
      field,
      phaseResult.control[field] === 0
        ? (phaseResult.candidate[field] === 0 ? null : "candidate-only")
        : phaseResult.candidate[field] / phaseResult.control[field],
    ]),
  );
  result.phases[phase] = phaseResult;
}

process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
