#!/usr/bin/env node

// Focused modern-guest A/B for two runtime configurations of the same rewrite
// Wasm artifact. Each leg owns a fresh process and guest; pair order alternates
// and every leg is bracketed by the same native host probe as scorecard v2.
// This is an engineering promotion screen, never an authoritative cross-ISA
// result.

import { spawn } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acquireBenchmarkLock } from "./bench-lock.mjs";
import { alternatingOrder, CPU_PROBE_SPEC, cpuProbe, median } from "./bench-math.mjs";
import {
  MAX_HOST_SPREAD,
  MAX_SAMPLE_SPREAD,
  ROW_BY_KEY,
  sampleSpread,
  sha256,
} from "./scorecard-v2-lib.mjs";
import { summary } from "../statistics.mjs";
import { readFile } from "node:fs/promises";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const worker = join(root, "tests/vs-v86/scorecard-v2-worker.mjs");
const artifactsArg = process.env.ARTIFACTS;
if (!artifactsArg) throw new Error("set ARTIFACTS");
const artifacts = resolve(artifactsArg);
const rows = (process.env.ROWS || "compile")
  .split(",")
  .map((value) => value.trim())
  .filter(Boolean);
if (!rows.length || rows.some((row) => !ROW_BY_KEY.has(row))) {
  throw new Error("ROWS must contain scorecard-v2 row keys");
}
const reps = Number(process.env.REPS || 2);
if (!Number.isSafeInteger(reps) || reps < 2) throw new Error("REPS must be an integer >= 2");

function parseConfig(name) {
  const parsed = JSON.parse(process.env[name] || "{}");
  if (!parsed || Array.isArray(parsed) || typeof parsed !== "object") {
    throw new Error(`${name} must be a JSON object`);
  }
  const out = {};
  for (const [key, value] of Object.entries(parsed)) {
    if (!key.startsWith("SCORECARD_V2_") || typeof value !== "string") {
      throw new Error(`${name}.${key} must be a string SCORECARD_V2_* variable`);
    }
    out[key] = value;
  }
  return out;
}

const controlConfig = parseConfig("CONTROL_CONFIG");
const candidateConfig = parseConfig("CANDIDATE_CONFIG");
if (JSON.stringify(controlConfig) === JSON.stringify(candidateConfig)) {
  throw new Error("CONTROL_CONFIG and CANDIDATE_CONFIG must differ");
}
const configKeys = new Set([...Object.keys(controlConfig), ...Object.keys(candidateConfig)]);
const childBaseEnv = { ...process.env };
for (const key of configKeys) delete childBaseEnv[key];
for (const key of ["CONTROL_CONFIG", "CANDIDATE_CONFIG", "ROWS", "REPS"]) {
  delete childBaseEnv[key];
}

const defaultWasmPath = join(
  root,
  "target/wasm32-unknown-unknown/release/rv64_wasm.wasm",
);
const wasmBySide = Object.fromEntries(await Promise.all(
  [["control", controlConfig], ["candidate", candidateConfig]].map(
    async ([side, config]) => {
      const path = config.SCORECARD_V2_REWRITE_WASM
        ? resolve(config.SCORECARD_V2_REWRITE_WASM)
        : defaultWasmPath;
      return [side, { path, sha256: sha256(await readFile(path)) }];
    },
  ),
));
const outputDir = resolve(
  process.env.SCORECARD_V2_OUTPUT || join(root, "target/bench/scorecard-v2-config-ab"),
);
await mkdir(outputDir, { recursive: true });
const releaseLock = await acquireBenchmarkLock(artifacts);

function runWorker(row, config) {
  return new Promise((resolveRun) => {
    const child = spawn(process.execPath, [worker, "rewrite", row], {
      cwd: root,
      env: { ...childBaseEnv, ARTIFACTS: artifacts, ...config },
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (data) => (stdout += data));
    child.stderr.on("data", (data) => (stderr += data));
    child.on("close", (code) => {
      const line = stdout.split(/\r?\n/).find((value) => value.startsWith("RESULT_JSON "));
      if (code !== 0 || !line) {
        resolveRun({ error: `exit=${code}: ${stderr || stdout.slice(-2000)}` });
        return;
      }
      try {
        resolveRun({ result: JSON.parse(line.slice("RESULT_JSON ".length)), stderr });
      } catch (error) {
        resolveRun({ error: `invalid worker JSON: ${error.message}` });
      }
    });
  });
}

const trials = [];
cpuProbe();
try {
  for (const row of rows) {
    process.stderr.write(`[config A/B ${row}]`);
    for (let rep = 0; rep < reps; rep++) {
      const order = alternatingOrder(rep, "control", "candidate");
      for (const [orderIndex, side] of order.entries()) {
        process.stderr.write(` ${rep + 1}${side === "control" ? "a" : "b"}`);
        const before = cpuProbe();
        const started = new Date().toISOString();
        const outcome = await runWorker(
          row,
          side === "control" ? controlConfig : candidateConfig,
        );
        const after = cpuProbe();
        trials.push({
          row,
          rep: rep + 1,
          side,
          order: orderIndex + 1,
          orderVector: order,
          started,
          hostBeforeMs: before,
          hostAfterMs: after,
          ...outcome,
        });
      }
    }
    process.stderr.write(" ok\n");
  }
} finally {
  await releaseLock();
}

const problems = [];
for (const trial of trials) {
  if (trial.error) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: ${trial.error}`);
    continue;
  }
  const result = trial.result;
  if (result.side !== "rewrite" || result.row !== trial.row) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: worker identity mismatch`);
  }
  if (result.runtime?.identity?.wasmSha256 !== wasmBySide[trial.side].sha256) {
    problems.push(`${trial.side}/${trial.row}/rep${trial.rep}: Wasm hash mismatch`);
  }
}

const aggregates = {};
for (const row of rows) {
  const rowSpec = ROW_BY_KEY.get(row);
  const phase = rowSpec.family === "boot" ? "first" : "steady";
  const bySide = {};
  for (const side of ["control", "candidate"]) {
    const legs = trials.filter((trial) => trial.row === row && trial.side === side && trial.result);
    const values = legs.map((trial) => trial.result.phases?.[phase]?.value).filter(Number.isFinite);
    if (values.length !== reps) problems.push(`${side}/${row}: ${values.length}/${reps} values`);
    const fingerprints = [...new Set(legs.map((trial) =>
      trial.result.phases?.[phase]?.md5 ?? trial.result.phases?.[phase]?.checksum).filter(Boolean))];
    const inputs = [...new Set(legs.map((trial) => JSON.stringify(trial.result.inputSha256)))];
    const spread = sampleSpread(values);
    if (spread !== null && spread > MAX_SAMPLE_SPREAD) {
      problems.push(
        `${side}/${row}: sample spread ${spread.toFixed(3)} exceeds ${MAX_SAMPLE_SPREAD}`,
      );
    }
    bySide[side] = { values, median: median(values), spread, fingerprints, inputs };
  }
  if (JSON.stringify(bySide.control.fingerprints) !== JSON.stringify(bySide.candidate.fingerprints)) {
    problems.push(`${row}: correctness fingerprints differ`);
  }
  if (
    bySide.control.inputs.length !== 1 || bySide.candidate.inputs.length !== 1 ||
    bySide.control.inputs[0] !== bySide.candidate.inputs[0]
  ) {
    problems.push(`${row}: input hashes differ`);
  }
  const speedup = bySide.control.median / bySide.candidate.median;
  const pairedSpeedups = [];
  for (let rep = 1; rep <= reps; rep++) {
    const control = trials.find((trial) =>
      trial.row === row && trial.rep === rep && trial.side === "control" && trial.result);
    const candidate = trials.find((trial) =>
      trial.row === row && trial.rep === rep && trial.side === "candidate" && trial.result);
    const controlValue = control?.result.phases?.[phase]?.value;
    const candidateValue = candidate?.result.phases?.[phase]?.value;
    if (!Number.isFinite(controlValue) || !Number.isFinite(candidateValue)) continue;
    pairedSpeedups.push(
      rowSpec.kind === "throughput"
        ? candidateValue / controlValue
        : controlValue / candidateValue,
    );
  }
  if (pairedSpeedups.length !== reps) {
    problems.push(`${row}: ${pairedSpeedups.length}/${reps} complete pairs`);
  }
  aggregates[row] = {
    phase,
    ...bySide,
    candidateSpeedup: speedup,
    pairedCandidateSpeedup: pairedSpeedups.length ? summary(pairedSpeedups) : null,
  };
}

const probes = trials.flatMap((trial) => [trial.hostBeforeMs, trial.hostAfterMs]);
const hostProbeSpread = Math.max(...probes) / Math.min(...probes);
if (hostProbeSpread > MAX_HOST_SPREAD) {
  problems.push(`host probe spread ${hostProbeSpread.toFixed(3)} exceeds ${MAX_HOST_SPREAD}`);
}

const report = {
  schema: 1,
  created: new Date().toISOString(),
  measurementValid: problems.length === 0,
  authoritative: false,
  purpose: "rewrite runtime-configuration or artifact A/B diagnostic",
  configuration: {
    rows,
    reps,
    controlConfig,
    candidateConfig,
    wasmBySide,
    hostProbe: { ...CPU_PROBE_SPEC, maximumSpread: MAX_HOST_SPREAD },
    maximumSampleSpread: MAX_SAMPLE_SPREAD,
  },
  aggregates,
  trials,
  hostProbeSpread,
  hostCpuAffinity: (await readFile("/proc/self/status", "utf8").catch(() => ""))
    .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null,
  problems,
};
const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const reportPath = join(outputDir, `config-ab-${stamp}.json`);
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);

for (const [row, aggregate] of Object.entries(aggregates)) {
  if (
    aggregate.control.median === null ||
    aggregate.candidate.median === null ||
    aggregate.pairedCandidateSpeedup === null
  ) {
    console.log(`${row} ${aggregate.phase}: INVALID — no complete timing pairs`);
    continue;
  }
  console.log(
    `${row} ${aggregate.phase}: control=${aggregate.control.median.toFixed(2)} ms ` +
      `candidate=${aggregate.candidate.median.toFixed(2)} ms ` +
      `speedup=${aggregate.candidateSpeedup.toFixed(3)}x ` +
      `paired=${aggregate.pairedCandidateSpeedup.median.toFixed(3)}x ` +
      `CI=[${aggregate.pairedCandidateSpeedup.medianConfidence95
        .map((value) => value.toFixed(3)).join(", ")}]`,
  );
}
console.log(
  `host probe spread=${hostProbeSpread.toFixed(3)}; ` +
    `${problems.length ? `INVALID: ${problems.join("; ")}` : "valid"}`,
);
console.log(`saved ${reportPath}`);
if (problems.length) process.exitCode = 1;
