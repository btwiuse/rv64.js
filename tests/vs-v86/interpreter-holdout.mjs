#!/usr/bin/env node

// Sealed pure-interpreter transfer suite. The inputs are built before tuning
// and this runner is invoked only after a candidate has been frozen.

import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import { cpus, loadavg, platform, release } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acquireBenchmarkLock } from "./bench-lock.mjs";
import { cpuProbe } from "./bench-math.mjs";
import {
  INTERPRETER_HOLDOUT_ROWS,
  MATCH_FLOOR,
  MAX_HOST_SPREAD,
  MAX_SAMPLE_SPREAD,
  balancedOrder,
  median,
  phasesFor,
  sampleSpread,
  speedRatio,
  verdict,
} from "./scorecard-v2-lib.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const worker = join(root, "tests/vs-v86/scorecard-v2-worker.mjs");
const artifacts = resolve(process.env.ARTIFACTS || "");
if (!process.env.ARTIFACTS) throw new Error("set ARTIFACTS");
const holdoutRoot = resolve(
  process.env.INTERPRETER_HOLDOUT_ROOT || join(artifacts, "interpreter-holdouts-v1"),
);
const rv64Initramfs = join(holdoutRoot, "interpreter-holdout-riscv64.cpio");
const x86Initramfs = join(holdoutRoot, "interpreter-holdout-x86.cpio");
const contractPath = join(holdoutRoot, "contract.json");
const sumsPath = join(holdoutRoot, "SHA256SUMS");
const outputDir = resolve(
  process.env.INTERPRETER_HOLDOUT_OUTPUT || join(holdoutRoot, "results"),
);
const v86dir = resolve(process.env.V86DIR || join(artifacts, "v86"));
const reps = Number(process.env.REPS || 3);
const authoritative = process.env.AUTHORITATIVE === "1";
const timeoutMs = Number(process.env.SCORECARD_V2_TIMEOUT_MS || 900_000);
if (!Number.isSafeInteger(reps) || reps < 1 || authoritative && (reps < 3 || !(reps & 1))) {
  throw new Error("REPS must be positive; authoritative runs require odd REPS>=3");
}

for (const path of [rv64Initramfs, x86Initramfs, contractPath, sumsPath, worker]) {
  await access(path).catch(() => { throw new Error(`missing sealed holdout input: ${path}`); });
}
const contract = JSON.parse(await readFile(contractPath, "utf8"));
if (contract.population !== "sealed-interpreter-holdouts-v1" ||
    contract.evaluationRule !== "run only after the candidate is frozen") {
  throw new Error("holdout contract is not the frozen v1 population");
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const [rv64Bytes, x86Bytes, sumsBytes, runnerBytes, workerBytes] = await Promise.all([
  readFile(rv64Initramfs),
  readFile(x86Initramfs),
  readFile(sumsPath),
  readFile(import.meta.filename),
  readFile(worker),
]);
const identities = {
  rv64InitramfsSha256: sha256(rv64Bytes),
  x86InitramfsSha256: sha256(x86Bytes),
  contractSha256: sha256(await readFile(contractPath)),
  sumsSha256: sha256(sumsBytes),
  runnerSha256: sha256(runnerBytes),
  workerSha256: sha256(workerBytes),
};

const releaseLock = await acquireBenchmarkLock(artifacts);
const trials = [];
const hostProbes = [];
const problems = [];

function probe(label) {
  const value = { label, at: new Date().toISOString(), cpuMs: cpuProbe(), loadAverage: loadavg() };
  hostProbes.push(value);
  return value;
}

function runWorker(side, row) {
  return new Promise((resolveResult) => {
    const child = spawn(process.execPath, ["--max-old-space-size=4096", worker, side, row.key], {
      cwd: root,
      env: {
        ...process.env,
        ARTIFACTS: artifacts,
        V86DIR: v86dir,
        SCORECARD_V2_EXECUTION_MODE: "interpreter",
        INTERPRETER_HOLDOUT_RV64_INITRAMFS: rv64Initramfs,
        INTERPRETER_HOLDOUT_X86_INITRAMFS: x86Initramfs,
      },
    });
    let stdout = "";
    let stderr = "";
    const guard = setTimeout(() => child.kill("SIGTERM"), timeoutMs * 4 + 180_000);
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.on("close", (code, signal) => {
      clearTimeout(guard);
      const match = [...stdout.matchAll(/^RESULT_JSON (.+)$/gm)].at(-1);
      if (code !== 0 || !match) {
        resolveResult({ error: `exit=${code} signal=${signal ?? "none"}`, stdout, stderr });
        return;
      }
      try {
        resolveResult({ result: JSON.parse(match[1]), stderr });
      } catch (error) {
        resolveResult({ error: `invalid worker JSON: ${error.message}`, stdout, stderr });
      }
    });
  });
}

function validate(trial) {
  const { side, row, result } = trial;
  if (!result) return;
  if (result.side !== side || result.row !== row.key) problems.push(`${side}/${row.key}: mislabeled`);
  if (result.measurementEligible !== true) problems.push(`${side}/${row.key}: proof-only result`);
  if (result.runtime?.executionMode !== "interpreter") problems.push(`${side}/${row.key}: not interpreter`);
  if (result.runtime?.jitProof?.inactiveProof !== true) problems.push(`${side}/${row.key}: JIT not inactive`);
  const active = Object.entries(result.runtime?.jitProof?.activity ?? {})
    .filter(([, value]) => Number(value) !== 0);
  if (active.length) problems.push(`${side}/${row.key}: JIT activity ${JSON.stringify(active)}`);
  if (side === "v86" && result.runtime?.jitProof?.disabled !== 1) {
    problems.push(`${side}/${row.key}: disable_jit proof missing`);
  }
  const expectedInitramfs = side === "rewrite"
    ? identities.rv64InitramfsSha256
    : identities.x86InitramfsSha256;
  if (result.inputSha256?.initramfs !== expectedInitramfs) {
    problems.push(`${side}/${row.key}: initramfs identity mismatch`);
  }
  const expected = `0x${contract.expected[row.key].slice(0, 16)}`;
  for (const phase of phasesFor(row)) {
    if (result.phases?.[phase]?.checksum !== expected) {
      problems.push(`${side}/${row.key}/${phase}: checksum mismatch`);
    }
  }
}

try {
  probe("suite-start");
  for (let rep = 0; rep < reps; rep++) {
    for (const row of INTERPRETER_HOLDOUT_ROWS) {
      process.stdout.write(`[${row.key}] ${rep + 1}`);
      for (const side of balancedOrder(rep, ["rewrite", "v86"])) {
        const hostBefore = probe(`${row.key}/${rep}/${side}/before`);
        process.stdout.write(side === "rewrite" ? "r" : "v");
        const outcome = await runWorker(side, row);
        const hostAfter = probe(`${row.key}/${rep}/${side}/after`);
        const trial = { rep, side, row, hostBefore, hostAfter, ...outcome };
        if (outcome.error) {
          problems.push(`${side}/${row.key}: ${outcome.error}`);
        } else {
          validate(trial);
        }
        trials.push(trial);
      }
      process.stdout.write(" ok\n");
    }
  }
  probe("suite-end");
} finally {
  await releaseLock();
}

const aggregates = INTERPRETER_HOLDOUT_ROWS.map((row) => {
  const sides = Object.fromEntries(["rewrite", "v86"].map((side) => {
    const samples = trials
      .filter((trial) => trial.side === side && trial.row.key === row.key && trial.result)
      .map((trial) => trial.result.phases.steady.value);
    return [side, { samples, median: median(samples), spread: sampleSpread(samples) }];
  }));
  const ratio = speedRatio(row.kind, sides.rewrite.median, sides.v86.median);
  if (sides.rewrite.samples.length !== reps || sides.v86.samples.length !== reps) {
    problems.push(`${row.key}: incomplete population`);
  }
  for (const [side, summary] of Object.entries(sides)) {
    if (summary.spread > MAX_SAMPLE_SPREAD) problems.push(`${side}/${row.key}: unstable samples`);
  }
  return { ...row, sides, rewriteVsV86: ratio };
});
const hostSpread = sampleSpread(hostProbes.map((sample) => sample.cpuMs));
if (hostSpread > MAX_HOST_SPREAD) problems.push(`host probe spread ${hostSpread.toFixed(4)}x`);
const measurementValid = problems.length === 0;
const goalMet = measurementValid && aggregates.every((row) => row.rewriteVsV86 >= MATCH_FLOOR);
const created = new Date().toISOString();
const report = {
  schema: 1,
  created,
  authoritative,
  measurementValid,
  goalMet,
  problems,
  configuration: { executionMode: "interpreter", reps, sides: ["rewrite", "v86"], hostSpread },
  contract,
  identities,
  provenance: { node: process.version, platform: platform(), release: release(), cpu: cpus()[0]?.model },
  hostProbes,
  trials,
  aggregates,
};

let markdown = `# Sealed pure-interpreter holdouts\n\n`;
markdown += `_${created}; ${authoritative ? "authoritative" : "exploratory"}; ${reps} repetition(s)._\n\n`;
markdown += `| Workload | Rewrite | copy/v86 | Rewrite vs v86 |\n|---|---:|---:|---:|\n`;
for (const row of aggregates) {
  markdown += `| ${row.name} | ${row.sides.rewrite.median?.toFixed(1) ?? "N/A"} ms | ` +
    `${row.sides.v86.median?.toFixed(1) ?? "N/A"} ms | ${verdict(row.rewriteVsV86)} |\n`;
}
markdown += `\n**Measurement ${measurementValid ? "VALID" : "INVALID"}. ` +
  `Holdout goal ${goalMet ? "MET" : "NOT MET"}.**\n`;
if (problems.length) markdown += `\n${problems.map((problem) => `- ${problem}`).join("\n")}\n`;

await mkdir(outputDir, { recursive: true });
const stamp = created.replaceAll(":", "-");
const jsonPath = join(outputDir, `interpreter-holdout-${stamp}.json`);
const markdownPath = join(outputDir, `interpreter-holdout-${stamp}.md`);
await Promise.all([
  writeFile(jsonPath, `${JSON.stringify(report, null, 2)}\n`),
  writeFile(markdownPath, markdown),
]);
process.stdout.write(`\n${markdown}\nraw report: ${jsonPath}\nmarkdown:   ${markdownPath}\n`);
if (!measurementValid) process.exitCode = 1;
