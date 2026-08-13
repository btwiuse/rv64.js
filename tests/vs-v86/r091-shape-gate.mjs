#!/usr/bin/env node

// Verify R091's static Wasm outline and the naturally observed V8 tier-up.

import { createHash } from "node:crypto";
import { createReadStream, readFileSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { spawnSync } from "node:child_process";

const [controlPath, candidatePath, nativeLogPath, outputPath] = process.argv.slice(2);
if (!controlPath || !candidatePath || !nativeLogPath || !outputPath) {
  throw new Error(
    "usage: r091-shape-gate.mjs CONTROL.wasm CANDIDATE.wasm NATIVE.log OUTPUT.json",
  );
}

const CONTROL = "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010";
const CANDIDATE = "a8f14136e7d217f4e71aec2c52020f749c476ee2531268f0bab7adfff2e42c75";
const COMPILE_MD5 = "24eedf7e06beffd4d3ba1945585588db";
const MAX_VIRT_BODY_RATIO = 0.90;
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function command(name, args, maxBuffer = 192 << 20) {
  const child = spawnSync(name, args, { encoding: "utf8", maxBuffer });
  if (child.status !== 0) {
    throw new Error(child.stderr || child.stdout || `${name} failed`);
  }
  return child.stdout;
}

function count(body, pattern) {
  return [...body.matchAll(pattern)].length;
}

function functionsFrom(disassembly) {
  const headers = [...disassembly.matchAll(/^([0-9a-f]+) <([^>]+)>:$/gm)];
  return headers.map((header, index) => {
    const start = Number.parseInt(header[1], 16);
    const end = index + 1 < headers.length
      ? Number.parseInt(headers[index + 1][1], 16)
      : null;
    const textEnd = index + 1 < headers.length ? headers[index + 1].index : disassembly.length;
    return {
      name: header[2],
      start,
      end,
      // llvm-objdump's next symbol address includes the following body's
      // three-byte size prefix. This is the executable body size used by the
      // frozen R091 protocol (R085 Virt = 33,230 bytes).
      instructionBytes: end === null ? null : end - start - 3,
      text: disassembly.slice(header.index, textEnd),
    };
  });
}

function exactFunction(functions, marker, suffix) {
  const matches = functions.filter((fn) => fn.name.startsWith("_RINv") &&
    fn.name.includes(marker) && fn.name.endsWith(suffix));
  if (matches.length !== 1) {
    throw new Error(`${marker}/${suffix}: expected one function, found ${matches.length}`);
  }
  return matches[0];
}

function functionIndex(xray, name) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const matches = [...xray.matchAll(new RegExp(
    `^ - func\\[(\\d+)\\] sig=\\d+ <${escaped}>$`,
    "gm",
  ))];
  if (matches.length !== 1) {
    throw new Error(`${name}: expected one function index, found ${matches.length}`);
  }
  return Number(matches[0][1]);
}

function bodyRecord(fn, index) {
  const calls = [...fn.text.matchAll(/\bcall\s+(\d+)/g)].map((match) => Number(match[1]));
  return {
    name: fn.name,
    index,
    instructionBytes: fn.instructionBytes,
    directCalls: calls.length,
    callTargets: Object.fromEntries(
      [...new Set(calls)].sort((a, b) => a - b)
        .map((target) => [target, calls.filter((value) => value === target).length]),
    ),
    callIndirect: count(fn.text, /\bcall_indirect\b/g),
    i32Loads: count(fn.text, /\bi32\.load(?:8_[su]|16_[su])?\b/g),
    i64Loads: count(fn.text, /\bi64\.load(?:8_[su]|16_[su]|32_[su])?\b/g),
    i64Stores: count(fn.text, /\bi64\.store(?:8|16|32)?\b/g),
    i32Comparisons: count(fn.text, /\bi32\.(?:eq|ne|eqz)\b/g),
    i64Eqz: count(fn.text, /\bi64\.eqz\b/g),
    conditionalBranches: count(fn.text, /\bbr_if\b/g),
  };
}

function inspect(path, expectHelpers) {
  const bytes = readFileSync(path);
  if (!WebAssembly.validate(bytes)) throw new Error(`${path}: invalid Wasm`);
  const module = new WebAssembly.Module(bytes);
  const disassembly = command("llvm-objdump", ["-d", path]);
  const xray = command("wasm-objdump", ["-x", path]);
  const functions = functionsFrom(disassembly);
  const suffixes = {
    legacy: "11rv64_system7MachineEB2_",
    virt: "4virt11VirtMachineEB2_",
  };
  const out = {
    path,
    sha256: sha256(bytes),
    bytes: bytes.length,
    imports: WebAssembly.Module.imports(module),
    exports: WebAssembly.Module.exports(module),
    main: {},
    helper: {},
  };
  for (const [kind, suffix] of Object.entries(suffixes)) {
    const main = exactFunction(functions, "14run_system_jit", suffix);
    out.main[kind] = bodyRecord(main, functionIndex(xray, main.name));
    const helperMatches = functions.filter((fn) => fn.name.startsWith("_RINv") &&
      fn.name.includes("26run_system_generated_chain") && fn.name.endsWith(suffix));
    if (expectHelpers) {
      if (helperMatches.length !== 1) {
        throw new Error(`${kind}: expected one generated-chain helper, found ${helperMatches.length}`);
      }
      const helper = helperMatches[0];
      out.helper[kind] = bodyRecord(helper, functionIndex(xray, helper.name));
    } else if (helperMatches.length !== 0) {
      throw new Error(`${kind}: frozen control unexpectedly contains the R091 helper`);
    }
  }
  return out;
}

async function inspectNativeLog(path) {
  const stream = createReadStream(path);
  const digest = createHash("sha256");
  stream.on("data", (chunk) => digest.update(chunk));
  const lines = createInterface({ input: stream, crlfDelay: Infinity });
  const code = [];
  let current = null;
  let result = null;
  for await (const line of lines) {
    if (line.startsWith("name: ")) {
      const name = line.slice("name: ".length);
      current = name.includes("26run_system_generated_chain") && name.includes("VirtMachine")
        ? { name }
        : null;
    } else if (current && line.startsWith("index: ")) {
      current.index = Number(line.slice("index: ".length));
    } else if (current && line.startsWith("kind: ")) {
      current.kind = line.slice("kind: ".length);
    } else if (current && line.startsWith("compiler: ")) {
      current.compiler = line.slice("compiler: ".length);
    } else if (current && line.startsWith("Body (size = ")) {
      current.bodyBytes = Number(line.match(/^Body \(size = (\d+)/)?.[1]);
    } else if (current && line.startsWith("Instructions (size = ")) {
      current.instructionBytes = Number(line.match(/^Instructions \(size = (\d+)/)?.[1]);
    } else if (current && line === "--- End code ---") {
      code.push(current);
      current = null;
    } else if (line.startsWith("RESULT_JSON ")) {
      result = JSON.parse(line.slice("RESULT_JSON ".length));
    }
  }
  return { path, sha256: digest.digest("hex"), bytes: stream.bytesRead, code, result };
}

const control = inspect(controlPath, false);
const candidate = inspect(candidatePath, true);
const native = await inspectNativeLog(nativeLogPath);
const problems = [];
const check = (condition, message) => {
  if (!condition) problems.push(message);
};

check(control.sha256 === CONTROL, "control is not exact R085");
check(candidate.sha256 === CANDIDATE, "candidate artifact changed");
check(JSON.stringify(control.imports) === JSON.stringify(candidate.imports),
  "candidate imports changed");
check(JSON.stringify(control.exports) === JSON.stringify(candidate.exports),
  "candidate exports changed");

for (const kind of ["legacy", "virt"]) {
  const before = control.main[kind];
  const after = candidate.main[kind];
  const helper = candidate.helper[kind];
  check(before.callIndirect === 2, `${kind}: control does not contain two generated calls`);
  check(after.callIndirect === 0, `${kind}: generated indirect call remains in scheduler`);
  check(helper.callIndirect === 2, `${kind}: helper does not own both generated calls`);
  check(after.callTargets[helper.index] === 1,
    `${kind}: scheduler does not call helper exactly once`);
  check(helper.i32Loads >= 2 && helper.i64Loads >= 1 && helper.i64Stores >= 1,
    `${kind}: helper is missing dispatch/fuel/retirement memory operations`);
  check(helper.i32Comparisons >= 2 && helper.i64Eqz >= 1 && helper.conditionalBranches >= 1,
    `${kind}: helper is missing mapping/zero-retirement control operations`);
}
const virtBodyRatio = candidate.main.virt.instructionBytes /
  control.main.virt.instructionBytes;
check(control.main.virt.instructionBytes === 33_230,
  `control Virt scheduler is ${control.main.virt.instructionBytes}, not 33230 bytes`);
check(virtBodyRatio <= MAX_VIRT_BODY_RATIO,
  `candidate Virt scheduler ratio ${virtBodyRatio} exceeds ${MAX_VIRT_BODY_RATIO}`);

const nativeHelperIndex = candidate.helper.virt.index;
const nativeBodies = native.code.filter((entry) => entry.index === nativeHelperIndex &&
  entry.kind === "wasm function");
check(nativeBodies.some((entry) => entry.compiler === "Liftoff"),
  "native capture has no Liftoff Virt helper body");
check(nativeBodies.some((entry) => entry.compiler === "TurboFan"),
  "native capture has no TurboFan Virt helper body");
check(native.result?.side === "rewrite" && native.result?.row === "compile",
  "native capture is not a rewrite/compile run");
check(native.result?.runtime?.identity?.wasmSha256 === CANDIDATE,
  "native capture runtime artifact changed");
check(native.result?.runtime?.guest?.linux === "6.12.7" &&
  native.result?.runtime?.guest?.alpine === "3.24.1" &&
  native.result?.runtime?.guest?.arch === "riscv64", "native capture guest changed");
check(native.result?.runtime?.schedulerCadence?.name === "public-one-slice-per-turn" &&
  native.result?.runtime?.schedulerCadence?.rv64SlicesPerEventLoopTurn === 1,
"native capture scheduler cadence changed");
check(native.result?.runtime?.requestedPolicy?.name === "production-page" &&
  native.result?.runtime?.policyProblems?.length === 0,
"native capture production policy changed or failed validation");
check(BigInt(native.result?.runtime?.jitProof?.generatedInstructions ?? 0) > 0n &&
  BigInt(native.result?.runtime?.jitProof?.dispatches ?? 0) > 0n,
"native capture does not prove generated execution");
const compileHashes = Object.values(native.result?.phases ?? {}).map((phase) => phase.md5);
check(compileHashes.length === 3 && compileHashes.every((hash) => hash === COMPILE_MD5),
  "native capture Compile fingerprints changed");

const report = {
  schema: 1,
  experiment: "R091",
  mechanism: "hot-generated-chain-scheduler-outline",
  requirements: {
    maximumVirtSchedulerBodyRatio: MAX_VIRT_BODY_RATIO,
    requiredNativeCompilers: ["Liftoff", "TurboFan"],
  },
  control,
  candidate,
  observed: {
    virtSchedulerBodyRatio: virtBodyRatio,
    virtSchedulerReduction: 1 - virtBodyRatio,
    nativeLog: {
      path: native.path,
      sha256: native.sha256,
      bytes: native.bytes,
      helperBodies: nativeBodies,
      runtime: native.result && {
        side: native.result.side,
        row: native.result.row,
        identity: native.result.runtime?.identity,
        guest: native.result.runtime?.guest,
        schedulerCadence: native.result.runtime?.schedulerCadence,
        requestedPolicy: native.result.runtime?.requestedPolicy,
        policyProblems: native.result.runtime?.policyProblems,
        jitProof: native.result.runtime?.jitProof,
        compileHashes,
      },
    },
  },
  pass: problems.length === 0,
  problems,
};
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
if (problems.length) process.exitCode = 1;
