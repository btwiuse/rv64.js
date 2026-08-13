#!/usr/bin/env node

// Run and validate the one frozen R122 count-only modern-Boot diagnostic.
// Counter instrumentation changes the hot interpreter body, so every elapsed
// value in the raw worker record is explicitly excluded from performance use.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const outputDir = resolve(
  process.argv.find((value) => value.startsWith("--output-dir="))?.slice(13) ??
    "target/bench/r122-interpreter-body/counter-census",
);
const artifacts = resolve(process.env.ARTIFACTS ?? join(root, "target/bench"));
const wasmPath = join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");

const EXPECTED = Object.freeze({
  wasmSha256: "1dfb3eacb2828ff1b79c45fc75a84b34239efe2c518a400a0fac4d9509e22528",
  coreSha256: "a258ec37210b198c60cfc0a7a7bb790d48ee7e5c23d712ea7b38c0d5ca7a4dbf",
  wasmSourceSha256: "d2d5a82492dfeabe2e33dfef2164fe662f40f231b26522e224755101f188294c",
  harnessLibSha256: "ebeefb5e6d3472002366275c00d7dc532e46b3b12c8b9b5b521cfc7c3273bfb3",
  workerSha256: "3e0a97cfbdc613817328db3e457d5ddb4efff5a081b5325b309cb913c1ee2c36",
  protocolSha256: "b106c12ea2c622171c366cc1b80c0a496788d532eb85d63f9853338180149047",
  nativeAnalyzerSha256: "42485c1fac795d7eb14d45b2f01f2077da2b1adcf92fa4c45e82fbcb9e1033bf",
  loaderSha256: "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385",
  kernelSha256: "57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2",
  initramfsSha256: "cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808",
});

const FROZEN_SOURCES = Object.freeze({
  "cpu.rs": "crates/rv64-core/src/cpu.rs",
  "wasm-lib.rs": "crates/rv64-wasm/src/lib.rs",
  "scorecard-v2-lib.mjs": "tests/vs-v86/scorecard-v2-lib.mjs",
  "scorecard-v2-worker.mjs": "tests/vs-v86/scorecard-v2-worker.mjs",
  "protocol.md": "docs/jit-rewrite/R122_INTERPRETER_BODY_ATTRIBUTION_PROTOCOL.md",
  "native-census.mjs": "tests/vs-v86/r122-interpreter-body-native-census.mjs",
});

const RVC_NAMES = Object.freeze([
  "q0/C.ADDI4SPN", "q0/C.FLD", "q0/C.LW", "q0/C.LD",
  "q0/reserved", "q0/C.FSD", "q0/C.SW", "q0/C.SD",
  "q1/C.NOP-or-ADDI", "q1/C.ADDIW", "q1/C.LI", "q1/C.ADDI16SP-or-LUI",
  "q1/C.MISC-ALU", "q1/C.J", "q1/C.BEQZ", "q1/C.BNEZ",
  "q2/C.SLLI", "q2/C.FLDSP", "q2/C.LWSP", "q2/C.LDSP",
  "q2/C.JR-MV-EBREAK-JALR-ADD", "q2/C.FSDSP", "q2/C.SWSP", "q2/C.SDSP",
]);

const OPCODE_NAMES = new Map([
  [0x03, "LOAD"], [0x07, "LOAD-FP"], [0x0f, "MISC-MEM"],
  [0x13, "OP-IMM"], [0x17, "AUIPC"], [0x1b, "OP-IMM-32"],
  [0x23, "STORE"], [0x27, "STORE-FP"], [0x2f, "AMO"],
  [0x33, "OP"], [0x37, "LUI"], [0x3b, "OP-32"],
  [0x43, "FMADD"], [0x47, "FMSUB"], [0x4b, "FNMSUB"],
  [0x4f, "FNMADD"], [0x53, "OP-FP"], [0x63, "BRANCH"],
  [0x67, "JALR"], [0x6f, "JAL"], [0x73, "SYSTEM"],
]);

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function big(value) {
  return BigInt(value);
}

function ratio(value, total) {
  return total === 0n ? 0 : Number(value) / Number(total);
}

function ranked(values, names) {
  const total = values.reduce((sum, value) => sum + value, 0n);
  return values
    .map((value, index) => ({
      index,
      name: names(index),
      attempts: value.toString(),
      fraction: ratio(value, total),
    }))
    .filter((row) => big(row.attempts) !== 0n)
    .sort((left, right) => {
      const a = big(left.attempts);
      const b = big(right.attempts);
      return a === b ? left.index - right.index : a > b ? -1 : 1;
    });
}

const sourceChecks = {
  coreSha256: sha256(join(root, FROZEN_SOURCES["cpu.rs"])),
  wasmSourceSha256: sha256(join(root, FROZEN_SOURCES["wasm-lib.rs"])),
  harnessLibSha256: sha256(join(root, FROZEN_SOURCES["scorecard-v2-lib.mjs"])),
  workerSha256: sha256(join(root, FROZEN_SOURCES["scorecard-v2-worker.mjs"])),
  protocolSha256: sha256(join(root, FROZEN_SOURCES["protocol.md"])),
  nativeAnalyzerSha256: sha256(join(root, FROZEN_SOURCES["native-census.mjs"])),
};
for (const [name, value] of Object.entries(sourceChecks)) {
  assert.equal(value, EXPECTED[name], `${name} mismatch`);
}
assert.equal(sha256(wasmPath), EXPECTED.wasmSha256, "instrumented Wasm mismatch");

mkdirSync(outputDir, { recursive: true });
const command = [
  "taskset", "-c", "8-15", process.execPath,
  "tests/vs-v86/scorecard-v2-worker.mjs", "rewrite", "boot",
];
const environment = {
  ARTIFACTS: artifacts,
  SCORECARD_V2_TIMEOUT_MS: process.env.SCORECARD_V2_TIMEOUT_MS ?? "900000",
  SCORECARD_V2_INTERP_DIAG: "1",
};
const child = spawnSync(command[0], command.slice(1), {
  cwd: root,
  env: { ...process.env, ...environment },
  encoding: "utf8",
  maxBuffer: 64 << 20,
});
if (child.status !== 0) {
  throw new Error(
    `R122 diagnostic worker failed (${child.status})\n${child.stderr}\n${child.stdout}`,
  );
}
const resultLine = child.stdout.split("\n").find((line) => line.startsWith("RESULT_JSON "));
if (!resultLine) throw new Error("R122 diagnostic worker emitted no RESULT_JSON");
const result = JSON.parse(resultLine.slice("RESULT_JSON ".length));

assert.equal(result.side, "rewrite");
assert.equal(result.row, "boot");
assert.equal(result.measurementEligible, false, "instrumented worker must be ineligible");
assert.equal(result.runtime.diagnostic?.r122InterpreterBodyCounters, true);
assert.equal(result.runtime.diagnostic?.elapsedValuesExcluded, true);
assert.equal(result.runtime.identity.wasmSha256, EXPECTED.wasmSha256);
assert.equal(result.runtime.identity.loaderSha256, EXPECTED.loaderSha256);
assert.equal(result.runtime.guest.linux, "6.12.7");
assert.equal(result.runtime.guest.alpine, "3.24.1");
assert.equal(result.runtime.guest.arch, "riscv64");
assert.equal(result.runtime.schedulerCadence.name, "public-one-slice-per-turn");
assert.equal(result.inputSha256.kernel, EXPECTED.kernelSha256);
assert.equal(result.inputSha256.initramfs, EXPECTED.initramfsSha256);
assert.equal(result.runtime.policyProblems.length, 0, "production policy mismatch");
assert(result.settle.every((entry) => entry.complete), "JIT did not settle");

const counters = result.phases.first.counters;
const diagnostic = counters.r122InterpreterBodyDiagnostic;
assert(diagnostic, "missing R122 interpreter-body counters");
assert.equal(diagnostic.rvcAttempts.length, 24);
assert.equal(diagnostic.rv32Attempts.length, 128);

const rvcAttempts = diagnostic.rvcAttempts.map(big);
const rv32Attempts = diagnostic.rv32Attempts.map(big);
const rvcAttemptTotal = rvcAttempts.reduce((sum, value) => sum + value, 0n);
const rv32AttemptTotal = rv32Attempts.reduce((sum, value) => sum + value, 0n);
const wrCalls = big(diagnostic.wrCalls);
const wrZero = big(diagnostic.wrZero);
const rvcRetired = big(diagnostic.rvcRetired);
const rv32Retired = big(diagnostic.rv32Retired);
const sequential = big(diagnostic.sequential);
const nonsequential = big(diagnostic.nonsequential);
const retired = rvcRetired + rv32Retired;
const interpreterInstructions = big(counters.interpreterInstructions);
const generatedInstructions = big(counters.generatedInstructions);
const guestInstructions = big(counters.guestInstructions);

assert(rvcAttemptTotal >= rvcRetired, "RVC attempts below RVC retirement");
assert(rv32AttemptTotal >= rv32Retired, "RV32 attempts below RV32 retirement");
assert.equal(retired, interpreterInstructions, "interpreter retirement closure");
assert.equal(sequential + nonsequential, retired, "successor classification closure");
assert.equal(
  interpreterInstructions + generatedInstructions,
  guestInstructions,
  "generated plus interpreted guest-retirement closure",
);
assert(wrZero <= wrCalls, "discarded GPR writes exceed helper calls");
assert(interpreterInstructions > 0n, "no interpreter execution observed");
assert(generatedInstructions > 0n, "no generated execution observed");

const summary = {
  guestInstructions: guestInstructions.toString(),
  generatedInstructions: generatedInstructions.toString(),
  generatedFraction: ratio(generatedInstructions, guestInstructions),
  interpreterInstructions: interpreterInstructions.toString(),
  interpreterFraction: ratio(interpreterInstructions, guestInstructions),
  rvc: {
    attempts: rvcAttemptTotal.toString(),
    retired: rvcRetired.toString(),
    retirementFraction: ratio(rvcRetired, retired),
    decodedNonretiringAttempts: (rvcAttemptTotal - rvcRetired).toString(),
    families: ranked(rvcAttempts, (index) => RVC_NAMES[index]),
  },
  rv32: {
    attempts: rv32AttemptTotal.toString(),
    retired: rv32Retired.toString(),
    retirementFraction: ratio(rv32Retired, retired),
    decodedNonretiringAttempts: (rv32AttemptTotal - rv32Retired).toString(),
    opcodes: ranked(
      rv32Attempts,
      (index) => OPCODE_NAMES.get(index) ?? `opcode-0x${index.toString(16).padStart(2, "0")}`,
    ),
  },
  gprWriteHelper: {
    calls: wrCalls.toString(),
    discardedX0: wrZero.toString(),
    discardedFraction: ratio(wrZero, wrCalls),
    callsPerInterpreterInstruction: ratio(wrCalls, interpreterInstructions),
  },
  successor: {
    sequential: sequential.toString(),
    nonsequential: nonsequential.toString(),
    sequentialFraction: ratio(sequential, retired),
    nonsequentialFraction: ratio(nonsequential, retired),
  },
};

const report = {
  schema: 1,
  experiment: "R122 exact current-product interpreter-body counter census",
  performanceEvidence: false,
  elapsedValuesExcluded: true,
  candidateSelectionUse: "architecture-wide operation evidence only; opcode frequencies are descriptive",
  command,
  environment,
  identities: {
    ...sourceChecks,
    wasmSha256: EXPECTED.wasmSha256,
    loaderSha256: EXPECTED.loaderSha256,
    kernelSha256: EXPECTED.kernelSha256,
    initramfsSha256: EXPECTED.initramfsSha256,
  },
  workload: {
    measurementEligible: result.measurementEligible,
    guest: result.runtime.guest,
    cadence: result.runtime.schedulerCadence,
    requestedPolicy: result.runtime.requestedPolicy,
    effectivePolicy: result.runtime.effectivePolicy,
    generatedExecutionProved: generatedInstructions > 0n,
    settleComplete: result.settle.every((entry) => entry.complete),
  },
  closure: {
    interpreterRetirement: retired === interpreterInstructions,
    successorClassification: sequential + nonsequential === retired,
    totalGuestRetirement:
      interpreterInstructions + generatedInstructions === guestInstructions,
    rvcAttemptsCoverRetirement: rvcAttemptTotal >= rvcRetired,
    rv32AttemptsCoverRetirement: rv32AttemptTotal >= rv32Retired,
  },
  summary,
  rawResult: result,
};

const sourceDir = join(outputDir, "instrumented-source");
mkdirSync(sourceDir);
for (const [name, path] of Object.entries(FROZEN_SOURCES)) {
  copyFileSync(join(root, path), join(sourceDir, name));
}
copyFileSync(wasmPath, join(outputDir, "rv64_wasm.instrumented.wasm"));
writeFileSync(join(outputDir, "worker.stdout.log"), child.stdout, { flag: "wx" });
writeFileSync(join(outputDir, "worker.stderr.log"), child.stderr, { flag: "wx" });
writeFileSync(join(outputDir, "counter-census.json"), `${JSON.stringify(report, null, 2)}\n`, {
  flag: "wx",
});
process.stdout.write(`R122_COUNTER_CENSUS ${JSON.stringify({ outputDir, summary })}\n`);
