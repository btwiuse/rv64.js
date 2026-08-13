#!/usr/bin/env node

import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const gate = join(here, "r124-native-gate.mjs");
const temp = await mkdtemp(join(tmpdir(), "r124-native-gate-"));
const controlWasm = "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d";
const candidateWasm = "d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59";
const loader = "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385";
const rows = ["boot", "compile", "python"];

function makeNative({ bootCandidate = 1000, compileCandidate = 980, pythonCandidate = 1000 } = {}) {
  const candidateValue = {
    boot: bootCandidate,
    compile: compileCandidate,
    python: pythonCandidate,
  };
  const trials = [];
  for (const row of rows) {
    const phase = row === "boot" ? "first" : "steady";
    for (let rep = 1; rep <= 15; rep++) {
      for (const side of ["control", "candidate"]) {
        const resultPhase = {
          value: side === "control" ? 1000 : candidateValue[row],
          counters: { guestInstructions: "1000000", generatedInstructions: "500000" },
        };
        if (row === "compile") resultPhase.md5 = "24eedf7e06beffd4d3ba1945585588db";
        if (row === "python") resultPhase.checksum = "832040";
        trials.push({
          row,
          rep,
          side,
          result: {
            measurementEligible: false,
            runtime: {
              identity: {
                wasmSha256: side === "control" ? controlWasm : candidateWasm,
                loaderSha256: loader,
              },
              guest: { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" },
              schedulerCadence: {
                name: "public-one-slice-per-turn",
                rv64SlicesPerEventLoopTurn: 1,
              },
              requestedPolicy: { name: "production-page" },
              policyProblems: [],
              jitProof: { generatedInstructions: "1", dispatches: "1" },
            },
            phases: { [phase]: resultPhase },
          },
        });
      }
    }
  }
  return {
    measurementValid: true,
    problems: [],
    configuration: {
      rows,
      reps: 15,
      controlConfig: { SCORECARD_V2_REWRITE_WASM: "/control.wasm" },
      candidateConfig: { SCORECARD_V2_REWRITE_WASM: "/candidate.wasm" },
      wasmBySide: {
        control: { sha256: controlWasm },
        candidate: { sha256: candidateWasm },
      },
    },
    hostCpuAffinity: "8-15",
    trials,
  };
}

const construction = {
  measurementValid: true,
  problems: [],
  reps: 15,
  control: { cpus: "8-15", wasmSha256: controlWasm, loaderSha256: loader },
  candidate: { cpus: "8-15", wasmSha256: candidateWasm, loaderSha256: loader },
  accounting: { debitMs: 0 },
};

async function evaluate(name, native) {
  const nativePath = join(temp, `${name}-native.json`);
  const constructionPath = join(temp, `${name}-construction.json`);
  const outputPath = join(temp, `${name}-gate.json`);
  await writeFile(nativePath, JSON.stringify(native));
  await writeFile(constructionPath, JSON.stringify(construction));
  const child = spawnSync(process.execPath, [gate, nativePath, constructionPath, outputPath], {
    encoding: "utf8",
  });
  assert.equal(child.status, 0, child.stderr || child.stdout);
  return JSON.parse(await readFile(outputPath, "utf8"));
}

try {
  assert.equal((await evaluate("pass", makeNative())).admitProductGates, true);
  assert.equal((await evaluate("sub-one-percent", makeNative({ compileCandidate: 995 })))
    .admitProductGates, false);
  assert.equal((await evaluate("protected-regression", makeNative({ pythonCandidate: 1015 })))
    .admitProductGates, false);
  console.log("R124 native gate selftest: verified Compile target and protected-row decisions");
} finally {
  await rm(temp, { recursive: true, force: true });
}

