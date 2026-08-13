#!/usr/bin/env node

// Three-way modern-guest scorecard: rewrite rv64 JIT, historical rv64 JIT
// through its isolated VirtMachine adapter, and copy/v86. Every measured leg
// gets a fresh process and guest. FIRST is cold, PRIME is an unscored tiering
// pass, and STEADY is the comparison value for non-boot rows.

import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import { cpus, loadavg, platform, release } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acquireBenchmarkLock } from "./bench-lock.mjs";
import { CPU_PROBE_SPEC, cpuProbe } from "./bench-math.mjs";
import { cadenceRecord, parsePumpCadence } from "./scorecard-v2-cadence.mjs";
import {
  MATCH_FLOOR,
  MAX_HOST_SPREAD,
  MAX_SAMPLE_SPREAD,
  INTERPRETER_SIDES,
  NBENCH_WORKLOAD_CONTRACT,
  PHASES,
  ROWS,
  ROW_BY_KEY,
  SCHEMA,
  SIDES,
  balancedOrder,
  median,
  parseExecutionMode,
  phasesFor,
  sampleSpread,
  speedRatio,
  verdict,
} from "./scorecard-v2-lib.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const worker = join(root, "tests/vs-v86/scorecard-v2-worker.mjs");
const pumpCadence = parsePumpCadence();

function envFlag(name, fallback = false) {
  const raw = process.env[name];
  if (raw === undefined) return fallback;
  if (raw !== "0" && raw !== "1") throw new Error(`${name} must be 0 or 1`);
  return raw === "1";
}

function envInteger(name, fallback, minimum = 1) {
  const value = process.env[name] === undefined ? fallback : Number(process.env[name]);
  if (!Number.isSafeInteger(value) || value < minimum) {
    throw new Error(`${name} must be an integer >= ${minimum}`);
  }
  return value;
}

function selectedList(raw, allowed, label) {
  if (!raw) return [...allowed];
  const selected = raw.split(",").map((value) => value.trim()).filter(Boolean);
  const unknown = selected.filter((value) => !allowed.includes(value));
  if (unknown.length) throw new Error(`unknown ${label}: ${unknown.join(", ")}`);
  return [...new Set(selected)];
}

const artifactsArg = process.env.ARTIFACTS;
if (!artifactsArg) throw new Error("set ARTIFACTS");
const artifacts = resolve(artifactsArg);
const executionMode = parseExecutionMode(process.env.SCORECARD_V2_EXECUTION_MODE);
const inputPopulation = process.env.SCORECARD_V2_INPUT_POPULATION || "scorecard-v2-modern";
const allowedPopulations = [
  "scorecard-v2-modern",
  "scorecard-v2-rv64gcv-v1",
  "stock-musl-v1",
  "stock-musl-rv64gcv-v1",
];
const RV64GCV_KERNEL_SHA256 = "6029e2d5f0c24da911052be961cb7b3c1150206cff76666c8c8eebd8270a78d9";
if (!allowedPopulations.includes(inputPopulation)) {
  throw new Error(`SCORECARD_V2_INPUT_POPULATION must be one of ${allowedPopulations.join(", ")}`);
}
const rv64gcvJitPopulation = inputPopulation === "scorecard-v2-rv64gcv-v1";
if (inputPopulation !== "scorecard-v2-modern" && !rv64gcvJitPopulation && executionMode !== "interpreter") {
  throw new Error("alternate input populations are interpreter-only");
}
if (rv64gcvJitPopulation && executionMode !== "jit") {
  throw new Error("scorecard-v2-rv64gcv-v1 is JIT-only");
}
const defaultSides = executionMode === "interpreter" || rv64gcvJitPopulation
  ? INTERPRETER_SIDES
  : SIDES;
const selectedSides = process.env.SIDES
  ? selectedList(process.env.SIDES, SIDES, "sides")
  : [...defaultSides];
const selectedRows = selectedList(process.env.ROWS, ROWS.map((row) => row.key), "rows");
const reps = envInteger("REPS", 1);
const authoritative = envFlag("AUTHORITATIVE");
const v86ProofEnabled = envFlag("V86_EXECUTION_PREFLIGHT", executionMode === "jit");
const timeoutMs = envInteger("SCORECARD_V2_TIMEOUT_MS", 900_000, 60_000);
const rewritePolicy = process.env.SCORECARD_V2_REWRITE_POLICY || "production";
if (!["production", "compat"].includes(rewritePolicy)) {
  throw new Error("SCORECARD_V2_REWRITE_POLICY must be production or compat");
}
if (executionMode === "interpreter" && v86ProofEnabled) {
  throw new Error("V86_EXECUTION_PREFLIGHT must be 0 in interpreter mode");
}
const outputDir = resolve(process.env.SCORECARD_V2_OUTPUT || join(artifacts, "scorecard-v2"));
const legacyRoot = resolve(process.env.LEGACY_ROOT || join(root, "target/scorecard-v2-legacy"));
const v86dir = resolve(process.env.V86DIR || join(artifacts, "v86"));

const stockMuslPopulation = inputPopulation.startsWith("stock-musl-");
const rv64gcvPopulation = stockMuslPopulation || rv64gcvJitPopulation;
const rv64gcvRv64Initramfs = resolve(
  process.env.SCORECARD_V2_RV64GCV_RV64_INITRAMFS ||
  process.env.INTERPRETER_AUDIT_RV64_INITRAMFS ||
  join(artifacts, "interpreter-stock-musl-rv64gcv-v1/interpreter-stock-musl-riscv64.cpio"),
);
const rv64gcvX86Initramfs = resolve(
  process.env.SCORECARD_V2_RV64GCV_X86_INITRAMFS ||
  process.env.INTERPRETER_AUDIT_X86_INITRAMFS ||
  join(artifacts, "interpreter-stock-musl-rv64gcv-v1/interpreter-stock-musl-x86.cpio"),
);
const rv64KernelImage = resolve(
  process.env.RV64_KERNEL_IMAGE ||
  (rv64gcvJitPopulation
    ? join(artifacts, "interpreter-stock-musl-rv64gcv-v1/rv64gcv-linux-Image")
    : join(root, "web/images/alpine/Image")),
);
const requiredPaths = rv64gcvPopulation
  ? [
      rv64gcvRv64Initramfs,
      rv64gcvX86Initramfs,
    ]
  : [
      join(artifacts, "scorecard-v2-modern-riscv64.cpio"),
      join(artifacts, "scorecard-v2-modern-x86.cpio"),
    ];
if (stockMuslPopulation &&
    (!process.env.INTERPRETER_AUDIT_RV64_INITRAMFS ||
     !process.env.INTERPRETER_AUDIT_X86_INITRAMFS)) {
  throw new Error("stock-musl-v1 requires both interpreter audit initramfs paths");
}
if (selectedSides.includes("rewrite") || selectedSides.includes("legacy")) {
  requiredPaths.push(rv64KernelImage);
}
if (selectedSides.includes("rewrite")) {
  requiredPaths.push(
    join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
    join(root, "web/rv64.js"),
  );
}
if (selectedSides.includes("legacy")) {
  requiredPaths.push(
    join(legacyRoot, "web/rv64.js"),
    join(legacyRoot, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  );
}
if (selectedSides.includes("v86")) {
  requiredPaths.push(
    join(artifacts, "matched-linux-x86-bzImage"),
    join(v86dir, "src/main.js"),
    join(v86dir, "build/v86.wasm"),
    join(v86dir, "bios/seabios.bin"),
    join(v86dir, "bios/vgabios.bin"),
  );
}
for (const path of requiredPaths) {
  await access(path).catch(() => {
    throw new Error(`required scorecard input missing: ${path}`);
  });
}
if (rv64gcvJitPopulation) {
  const kernelSha256 = createHash("sha256")
    .update(await readFile(rv64KernelImage))
    .digest("hex");
  if (kernelSha256 !== RV64GCV_KERNEL_SHA256) {
    throw new Error(
      `RV64GCV kernel identity mismatch: ${kernelSha256} ` +
      `(expected ${RV64GCV_KERNEL_SHA256})`,
    );
  }
}

if (authoritative) {
  const problems = [];
  if (selectedSides.join(",") !== defaultSides.join(",")) {
    problems.push(`SIDES=${defaultSides.join(",")}`);
  }
  if (selectedRows.length !== ROWS.length || ROWS.some((row) => !selectedRows.includes(row.key))) {
    problems.push("all 13 rows");
  }
  if (reps < 3 || !(reps & 1)) problems.push("odd REPS>=3");
  if (executionMode === "jit" && !v86ProofEnabled) {
    problems.push("V86_EXECUTION_PREFLIGHT=1");
  }
  if (problems.length) throw new Error(`authoritative preflight requires ${problems.join(", ")}`);
}

const releaseLock = await acquireBenchmarkLock(artifacts);
const trials = [];
const hostProbes = [];
const problems = [];
let v86ExecutionPreflight = null;

function hostProbe(label) {
  const sample = {
    label,
    at: new Date().toISOString(),
    cpuMs: cpuProbe(),
    loadAverage: loadavg(),
  };
  hostProbes.push(sample);
  return sample;
}

function workerEnvironment(extra = {}) {
  return {
    ...process.env,
    ARTIFACTS: artifacts,
    LEGACY_ROOT: legacyRoot,
    V86DIR: v86dir,
    SCORECARD_V2_EXECUTION_MODE: executionMode,
    SCORECARD_V2_TIMEOUT_MS: String(timeoutMs),
    ...extra,
  };
}

function runWorker(side, row, extraEnv = {}) {
  return new Promise((resolveResult) => {
    const child = spawn(
      process.execPath,
      ["--max-old-space-size=4096", worker, side, row],
      { cwd: root, env: workerEnvironment(extraEnv) },
    );
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    let timedOut = false;
    const guard = setTimeout(() => {
      timedOut = true;
      child.kill("SIGTERM");
    }, timeoutMs * 4 + 180_000);
    child.on("close", (code, signal) => {
      clearTimeout(guard);
      const line = [...stdout.matchAll(/^RESULT_JSON (.+)$/gm)].at(-1);
      if (code !== 0 || !line) {
        resolveResult({
          error: timedOut
            ? `worker timeout after ${timeoutMs * 4 + 180_000}ms`
            : `worker exit=${code} signal=${signal ?? "none"}`,
          stdout: stdout.slice(-4000),
          stderr: stderr.slice(-4000),
        });
        return;
      }
      try {
        resolveResult({ result: JSON.parse(line[1]) });
      } catch (error) {
        resolveResult({
          error: `invalid worker JSON: ${error.message}`,
          stdout: stdout.slice(-4000),
          stderr: stderr.slice(-4000),
        });
      }
    });
  });
}

function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) =>
      `${JSON.stringify(key)}:${canonical(value[key])}`
    ).join(",")}}`;
  }
  return JSON.stringify(value);
}

function effectivePhase(row, result) {
  return row.family === "boot" ? result?.phases?.first : result?.phases?.steady;
}

function consistency(values) {
  const present = values.filter((value) => value != null);
  const unique = [...new Set(present)];
  return { complete: present.length === values.length, unique, value: unique.length === 1 ? unique[0] : null };
}

function addProblem(message) {
  problems.push(message);
}

function validateWorkerResult(trial) {
  const { side, row: rowKey, result } = trial;
  const row = ROW_BY_KEY.get(rowKey);
  if (!result) return;
  if (result.schema !== SCHEMA) addProblem(`${side}/${rowKey}: schema=${result.schema}`);
  if (result.side !== side || result.row !== rowKey) addProblem(`${side}/${rowKey}: mislabeled result`);
  if (result.measurementEligible === false) addProblem(`${side}/${rowKey}: proof-only run entered measurements`);
  if (result.runtime?.executionMode !== executionMode) {
    addProblem(`${side}/${rowKey}: execution mode proof missing or mismatched`);
  }
  if (result.runtime?.inputPopulation !== inputPopulation) {
    addProblem(`${side}/${rowKey}: input population proof missing or mismatched`);
  }
  if (result.runtime?.policyProblems?.length) {
    addProblem(`${side}/${rowKey}: ${result.runtime.policyProblems.join("; ")}`);
  }
  const jitProof = result.runtime?.jitProof;
  if (executionMode === "jit") {
    if (!jitProof?.enabledRequested) addProblem(`${side}/${rowKey}: JIT not requested`);
  } else {
    if (jitProof?.enabledRequested !== false) {
      addProblem(`${side}/${rowKey}: interpreter mode did not request JIT-off`);
    }
    if (jitProof?.inactiveProof !== true) {
      addProblem(`${side}/${rowKey}: interpreter mode lacks negative JIT-activity proof`);
    }
    const active = Object.entries(jitProof?.activity ?? {})
      .filter(([, value]) => value !== 0 && value !== "0");
    if (!jitProof?.activity || active.length) {
      addProblem(
        `${side}/${rowKey}: interpreter mode observed JIT activity` +
        (active.length ? ` (${active.map(([name, value]) => `${name}=${value}`).join(" ")})` : ""),
      );
    }
    if (side === "v86" && jitProof?.disabled !== 1) {
      addProblem(`${side}/${rowKey}: v86 disable_jit proof is not 1`);
    }
  }
  if (!result.runtime?.identity) addProblem(`${side}/${rowKey}: runtime identity missing`);
  if (!result.inputSha256 || Object.values(result.inputSha256).some((value) => !value)) {
    addProblem(`${side}/${rowKey}: input hashes missing`);
  }
  if (!Array.isArray(result.settle) || result.settle.some((entry) => !entry.complete)) {
    addProblem(`${side}/${rowKey}: generated compilation did not settle`);
  }
  if (row.family === "nbench") {
    if (
      result.runtime?.workload?.variant !== NBENCH_WORKLOAD_CONTRACT.variant ||
      result.runtime?.workload?.crossIsaComparable !== true
    ) {
      addProblem(`${side}/${rowKey}: scored BYTEmark workload is not fixed-work-data32`);
    }
    if (
      !result.inputSha256?.workloadContract ||
      !result.inputSha256?.workloadTransforms ||
      !result.inputSha256?.implementationSources
    ) {
      addProblem(`${side}/${rowKey}: BYTEmark workload proof hashes missing`);
    }
    for (const phase of phasesFor(row)) {
      const fixed = result.phases?.[phase]?.fixedWork;
      if (fixed?.fid !== row.nbenchId || !Array.isArray(fixed?.parameters)) {
        addProblem(`${side}/${rowKey}/${phase}: fixed-work execution proof missing`);
      }
    }
  }
  const expectedPhases = phasesFor(row);
  for (const phase of expectedPhases) {
    const sample = result.phases?.[phase];
    if (!(sample?.value > 0)) addProblem(`${side}/${rowKey}/${phase}: value missing`);
    if (row.family !== "boot" && !(sample?.hostMs > 0)) {
      addProblem(`${side}/${rowKey}/${phase}: host duration missing`);
    }
  }
  if (row.family !== "boot") {
    const warmValues = [result.phases?.prime?.value, result.phases?.steady?.value];
    const warmSpread = sampleSpread(warmValues);
    trial.warmSpread = warmSpread;
    const hostSpread = sampleSpread([
      result.phases?.prime?.hostMs,
      result.phases?.steady?.hostMs,
    ]);
    trial.hostWarmSpread = hostSpread;
    // PRIME is deliberately the tiering pass. Generated Wasm compilation may
    // occur while it runs, so a PRIME/STEADY difference is useful diagnostic
    // evidence but is not measurement instability. Repetition spread below is
    // computed from fresh-process STEADY samples and is the validity gate.
  }
  if (row.family === "nbench" && result.phases?.steady?.internal?.confidencePassed === false) {
    addProblem(`${side}/${rowKey}: nbench steady phase failed its internal confidence test`);
  }
}

function aggregateTrials() {
  const rows = [];
  for (const rowKey of selectedRows) {
    const row = ROW_BY_KEY.get(rowKey);
    const sides = {};
    for (const side of selectedSides) {
      const matching = trials.filter((trial) =>
        trial.side === side && trial.row === rowKey && trial.result
      );
      const samples = matching.map((trial) => effectivePhase(row, trial.result)?.value);
      sides[side] = {
        samples,
        median: median(samples),
        spread: sampleSpread(samples),
        coldSamples: matching.map((trial) => trial.result.phases?.first?.value),
        primeSamples: matching.map((trial) => trial.result.phases?.prime?.value).filter(Number.isFinite),
      };
      if (matching.length !== reps || samples.some((value) => !(value > 0))) {
        addProblem(`${side}/${rowKey}: expected ${reps} complete measured trials`);
      }
      if (sides[side].spread != null && sides[side].spread > MAX_SAMPLE_SPREAD) {
        addProblem(`${side}/${rowKey}: repetition spread ${sides[side].spread.toFixed(2)}x`);
      }
    }
    const rewrite = sides.rewrite?.median;
    const legacy = sides.legacy?.median;
    const v86 = sides.v86?.median;
    rows.push({
      key: rowKey,
      name: row.name,
      kind: row.kind,
      family: row.family,
      sides,
      rewriteVsLegacy: speedRatio(row.kind, rewrite, legacy),
      rewriteVsV86: speedRatio(row.kind, rewrite, v86),
      legacyVsV86: speedRatio(row.kind, legacy, v86),
    });
  }
  return rows;
}

function checkStableFingerprints() {
  const cadence = canonical(cadenceRecord(pumpCadence));
  for (const trial of trials.filter((trial) => trial.result)) {
    if (canonical(trial.result.runtime?.schedulerCadence) !== cadence) {
      addProblem(`${trial.side}/${trial.row}: scheduler cadence mismatch`);
    }
  }
  for (const side of selectedSides) {
    const identities = trials
      .filter((trial) => trial.side === side && trial.result)
      .map((trial) => canonical(trial.result.runtime.identity));
    if (identities.length && new Set(identities).size !== 1) {
      addProblem(`${side}: runtime identity changed across trials`);
    }
  }
  for (const rowKey of selectedRows) {
    for (const side of selectedSides) {
      const fingerprints = trials
        .filter((trial) => trial.side === side && trial.row === rowKey && trial.result)
        .map((trial) => canonical(trial.result.inputSha256));
      if (fingerprints.length && new Set(fingerprints).size !== 1) {
        addProblem(`${side}/${rowKey}: input hashes changed across repetitions`);
      }
    }
    if (selectedSides.includes("rewrite") && selectedSides.includes("legacy")) {
      const rewrite = trials.find((trial) => trial.side === "rewrite" && trial.row === rowKey)?.result;
      const legacy = trials.find((trial) => trial.side === "legacy" && trial.row === rowKey)?.result;
      if (rewrite && legacy && canonical(rewrite.inputSha256) !== canonical(legacy.inputSha256)) {
        addProblem(`${rowKey}: rewrite and legacy did not load identical RV64 inputs`);
      }
    }
    const row = ROW_BY_KEY.get(rowKey);
    if (row.family === "nbench") {
      for (const field of ["workloadContract", "workloadTransforms", "implementationSources"]) {
        const hashes = trials
          .filter((trial) => trial.row === rowKey && trial.result)
          .map((trial) => trial.result.inputSha256?.[field]);
        if (hashes.length && (hashes.some((hash) => !hash) || new Set(hashes).size !== 1)) {
          addProblem(`${rowKey}: cross-ISA ${field} hash mismatch`);
        }
      }
    }
  }
}

function checkCorrectness() {
  const resultSamples = trials.filter((trial) => trial.result);
  for (const trial of resultSamples) {
    const row = ROW_BY_KEY.get(trial.row);
    if (row.family === "boot" || row.family === "nbench") continue;
    const field = row.family === "compile" ? "md5" : "checksum";
    const values = PHASES.map((phase) => trial.result.phases?.[phase]?.[field]);
    const check = consistency(values);
    if (!check.complete || check.unique.length !== 1) {
      addProblem(`${trial.side}/${trial.row}: ${field} changed across phases`);
    }
  }
  for (const rowKey of selectedRows) {
    const row = ROW_BY_KEY.get(rowKey);
    if (!["compute", "python", "compile"].includes(row.family)) continue;
    const field = row.family === "compile" ? "md5" : "checksum";
    for (const side of selectedSides) {
      const values = resultSamples
        .filter((trial) => trial.side === side && trial.row === rowKey)
        .map((trial) => trial.result.phases?.steady?.[field]);
      const check = consistency(values);
      if (!check.complete || check.unique.length !== 1) {
        addProblem(`${side}/${rowKey}: ${field} changed across repetitions`);
      }
    }
  }
  if (selectedRows.includes("alu")) {
    const values = resultSamples
      .filter((trial) => trial.row === "alu")
      .map((trial) => trial.result.phases?.steady?.checksum);
    if (values.some((value) => value !== "0xf858aba6")) {
      addProblem(`ALU checksum mismatch: ${[...new Set(values)].join(",")}`);
    }
  }
  if (selectedRows.includes("mixed")) {
    const low32 = resultSamples
      .filter((trial) => trial.row === "mixed")
      .map((trial) => trial.result.phases?.steady?.checksum?.slice(-8));
    if (new Set(low32).size !== 1) addProblem(`Mixed low-32 checksum mismatch: ${[...new Set(low32)].join(",")}`);
  }
  if (selectedRows.includes("python")) {
    const values = resultSamples
      .filter((trial) => trial.row === "python")
      .map((trial) => trial.result.phases?.steady?.checksum);
    if (values.some((value) => value !== "832040")) addProblem("Python fib(30) checksum mismatch");
  }
  if (selectedRows.includes("compile") && selectedSides.includes("rewrite") && selectedSides.includes("legacy")) {
    const rvHashes = resultSamples
      .filter((trial) => trial.row === "compile" && trial.side !== "v86")
      .map((trial) => trial.result.phases?.steady?.md5);
    if (new Set(rvHashes).size !== 1) addProblem("rewrite/legacy compile object mismatch");
  }
}

function checkHostDrift() {
  const values = hostProbes.map((sample) => sample.cpuMs);
  const globalSpread = sampleSpread(values);
  if (globalSpread != null && globalSpread > MAX_HOST_SPREAD) {
    addProblem(`host CPU probe spread ${globalSpread.toFixed(2)}x over the run`);
  }
  for (const trial of trials) {
    const spread = sampleSpread([trial.hostBefore?.cpuMs, trial.hostAfter?.cpuMs]);
    trial.hostProbeSpread = spread;
    if (spread != null && spread > MAX_HOST_SPREAD) {
      addProblem(`${trial.side}/${trial.row}/rep${trial.rep}: host probe spread ${spread.toFixed(2)}x`);
    }
  }
  for (const rowKey of selectedRows) {
    for (let rep = 1; rep <= reps; rep++) {
      const paired = trials.filter((trial) => trial.row === rowKey && trial.rep === rep);
      const pairValues = paired.flatMap((trial) => [trial.hostBefore?.cpuMs, trial.hostAfter?.cpuMs]);
      const spread = sampleSpread(pairValues);
      if (spread != null && spread > MAX_HOST_SPREAD) {
        addProblem(`${rowKey}/rep${rep}: three-way host probe spread ${spread.toFixed(2)}x`);
      }
    }
  }
}

function formatValue(value, kind) {
  if (!(value > 0)) return "—";
  if (kind === "duration") return `${value.toFixed(value < 100 ? 2 : 1)} ms`;
  if (value >= 1_000_000) return value.toExponential(3);
  return value.toFixed(value < 100 ? 3 : 1);
}

function formatVerdict(ratio) {
  return ratio == null ? "—" : verdict(ratio);
}

function git(...args) {
  try {
    return execFileSync("git", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).trim();
  } catch {
    return "unknown";
  }
}

try {
  // Initialize the native probe before its values become evidence.
  cpuProbe();
  if (selectedSides.includes("v86") && v86ProofEnabled) {
    process.stderr.write("[preflight v86 generated dispatch]");
    const before = hostProbe("v86-proof:before");
    const proof = await runWorker("v86", "alu", { SCORECARD_V2_V86_EXEC_PROOF: "1" });
    const after = hostProbe("v86-proof:after");
    v86ExecutionPreflight = { ...proof, hostBefore: before, hostAfter: after };
    const probe = proof.result?.runtime?.jitProof?.executionProbe;
    if (
      !proof.result ||
      proof.result.measurementEligible !== false ||
      !(probe?.hits > 0) ||
      !(probe?.distinctHitIndexes > 0)
    ) {
      addProblem(`v86 generated-dispatch preflight failed: ${proof.error ?? canonical(probe)}`);
    }
    process.stderr.write(" ok\n");
  }

  for (const [rowIndex, rowKey] of selectedRows.entries()) {
    process.stderr.write(`[${rowKey}]`);
    for (let rep = 0; rep < reps; rep++) {
      const order = balancedOrder(rowIndex + rep, selectedSides);
      for (const [orderIndex, side] of order.entries()) {
        process.stderr.write(` ${rep + 1}${side[0]}`);
        const label = `${rowKey}:rep${rep + 1}:${side}`;
        const hostBefore = hostProbe(`${label}:before`);
        const started = new Date().toISOString();
        const workerResult = await runWorker(side, rowKey);
        const hostAfter = hostProbe(`${label}:after`);
        const trial = {
          row: rowKey,
          side,
          rep: rep + 1,
          order: orderIndex + 1,
          orderVector: order,
          started,
          hostBefore,
          hostAfter,
          ...workerResult,
        };
        trials.push(trial);
        if (trial.error) addProblem(`${side}/${rowKey}/rep${rep + 1}: ${trial.error}`);
        validateWorkerResult(trial);
      }
    }
    process.stderr.write(" ok\n");
  }
} finally {
  await releaseLock();
}

checkStableFingerprints();
checkCorrectness();
checkHostDrift();
const aggregates = aggregateTrials();

const rewriteComparisons = aggregates.flatMap((row) => [
  row.rewriteVsLegacy,
  row.rewriteVsV86,
]).filter((ratio) => ratio != null);
const goalMet = rewriteComparisons.length > 0 && rewriteComparisons.every((ratio) => ratio >= MATCH_FLOOR);

const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
const procStatus = await readFile("/proc/self/status", "utf8").catch(() => "");
const gitDiff = `${git("-C", root, "diff", "--binary", "HEAD")}\0${git("-C", root, "diff", "--binary", "--cached", "HEAD")}`;
const report = {
  schema: SCHEMA,
  created: new Date().toISOString(),
  authoritative,
  measurementValid: problems.length === 0,
  goalMet,
  configuration: {
    executionMode,
    inputPopulation,
    sides: selectedSides,
    rows: selectedRows,
    reps,
    phases: PHASES,
    scoredPhase: "steady (first for boot)",
    v86ExecutionPreflight: v86ProofEnabled,
    timeoutMs,
    rewritePolicy,
    schedulerCadence: cadenceRecord(pumpCadence),
    hostProbe: {
      ...CPU_PROBE_SPEC,
      maximumSpread: MAX_HOST_SPREAD,
    },
  },
  aggregates,
  trials,
  v86ExecutionPreflight,
  hostProbes,
  problems,
  provenance: {
    git: git("-C", root, "rev-parse", "HEAD"),
    gitStatus: git("-C", root, "status", "--short"),
    gitDiffSha256: createHash("sha256").update(gitDiff).digest("hex"),
    node: process.version,
    execArgv: process.execArgv,
    platform: platform(),
    release: release(),
    cpu: cpus()[0]?.model ?? null,
    cpuCount: cpus().length,
    cpuAffinity: procStatus.match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
  },
};

let markdown = `# rv64.js modern-guest ${executionMode === "interpreter" ? "interpreter" : "JIT"} scorecard v2\n\n`;
markdown += `_${report.created}; ${authoritative ? "authoritative" : "exploratory"}; `;
markdown += `${reps} fresh-process repetition(s); score = STEADY (FIRST for boot)._\n\n`;
if (executionMode === "interpreter") {
  markdown += `| Benchmark | Rewrite interpreter | copy/v86 interpreter | Rewrite vs v86 |\n`;
  markdown += `|---|---:|---:|---:|\n`;
  for (const row of aggregates) {
    markdown += `| ${row.name} | ${formatValue(row.sides.rewrite?.median, row.kind)} `;
    markdown += `| ${formatValue(row.sides.v86?.median, row.kind)} `;
    markdown += `| ${formatVerdict(row.rewriteVsV86)} |\n`;
  }
} else if (rv64gcvJitPopulation) {
  markdown += `| Benchmark | Rewrite RV64GCV JIT | copy/v86 JIT | Rewrite vs v86 |\n`;
  markdown += `|---|---:|---:|---:|\n`;
  for (const row of aggregates) {
    markdown += `| ${row.name} | ${formatValue(row.sides.rewrite?.median, row.kind)} `;
    markdown += `| ${formatValue(row.sides.v86?.median, row.kind)} `;
    markdown += `| ${formatVerdict(row.rewriteVsV86)} |\n`;
  }
} else {
  markdown += `| Benchmark | Rewrite | Legacy JIT | copy/v86 | Rewrite vs legacy | Rewrite vs v86 |\n`;
  markdown += `|---|---:|---:|---:|---:|---:|\n`;
  for (const row of aggregates) {
    markdown += `| ${row.name} | ${formatValue(row.sides.rewrite?.median, row.kind)} `;
    markdown += `| ${formatValue(row.sides.legacy?.median, row.kind)} `;
    markdown += `| ${formatValue(row.sides.v86?.median, row.kind)} `;
    markdown += `| ${formatVerdict(row.rewriteVsLegacy)} | ${formatVerdict(row.rewriteVsV86)} |\n`;
  }
}
markdown += rv64gcvJitPopulation
  ? `\n- RV64GCV guest contract: the frozen stock-musl RV64GCV binary population is compared with its pinned i686 counterpart.\n`
  : `\n- Modern guest contract: Linux 6.12.7, Alpine 3.24.1; riscv64 for both RV64 engines and i686 for v86.\n`;
markdown += `- Input population: ${inputPopulation}.\n`;
if (executionMode === "interpreter") {
  markdown += `- Both runtimes explicitly disable JIT before guest boot; every measured worker proves zero generated modules, cache growth, translation, and dispatch activity.\n`;
  markdown += `- FIRST, PRIME, and STEADY preserve the JIT scorecard workload order; PRIME is an unscored warm interpreter pass.\n`;
  markdown += `- Fresh-process STEADY repetition spread is the stability gate.\n`;
} else {
  markdown += `- Runtime main-module creation is outside workload timing; generated Wasm compilation and execution remain inside it.\n`;
  markdown += `- PRIME and all asynchronous compilation drains are unscored. Proof-only v86 instrumentation runs in a separate process.\n`;
  markdown += `- PRIME/STEADY deltas are recorded as tiering diagnostics, not rejected as noise; fresh-process STEADY repetition spread is the stability gate.\n`;
}
markdown += `- BYTEmark rows use ${NBENCH_WORKLOAD_CONTRACT.variant}: identical payload widths and explicit work counts on i686/RV64, timed by host serial markers rather than emulator-dependent guest clocks.\n`;
markdown += executionMode === "interpreter"
  ? `- FIRST is cold, PRIME warms the interpreter, and STEADY is scored. Self-timed and ABI-native-long BYTEmark binaries are retained only for unscored diagnostics.\n`
  : `- FIRST is cold, PRIME tiers the engine, and STEADY is scored. Self-timed and ABI-native-long BYTEmark binaries are retained only for unscored diagnostics.\n`;
markdown += `- Every worker records exact runtime, kernel, initramfs, and benchmark hashes in the JSON.\n`;
markdown += `\n**Measurement ${report.measurementValid ? "VALID" : "INVALID"}. `;
markdown += `Rewrite goal ${goalMet ? "MET" : "NOT MET"}.**\n`;
if (problems.length) markdown += `\nProblems:\n${problems.map((problem) => `- ${problem}`).join("\n")}\n`;

await mkdir(outputDir, { recursive: true });
const stem = `${executionMode === "interpreter" ? "interpreter-scorecard-v2" : "scorecard-v2"}-${timestamp}`;
const jsonPath = join(outputDir, `${stem}.json`);
const markdownPath = join(outputDir, `${stem}.md`);
await Promise.all([
  writeFile(jsonPath, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" }),
  writeFile(markdownPath, markdown, { flag: "wx" }),
]);

console.log(markdown);
console.log(`raw report: ${jsonPath}`);
console.log(`markdown:   ${markdownPath}`);
if (problems.length || (authoritative && !goalMet)) process.exitCode = 1;
