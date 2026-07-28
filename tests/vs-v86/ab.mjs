// Fast, serial, fresh-process rv64 candidate A/B.
//
// This is the promotion screen between a code/config change and the complete
// cross-emulator scorecard. It deliberately does not reuse an old v86 number:
// it answers only "did candidate B improve over control A on identical rv64
// inputs?" Selected nbench kernels run alone for a much shorter cycle.
//
//   BASE_WASM=target/bench/wasm-candidates/head-....wasm \
//   CANDIDATE_WASM=target/.../rv64_wasm.wasm \
//   ROWS=compile,numeric,assignment REPS=3 ARTIFACTS=target/bench \
//     node tests/vs-v86/ab.mjs
//
// Runtime config A/B without rebuilding:
//   BASE_CONFIG='{"TRACELVL":3}' CANDIDATE_CONFIG='{"TRACELVL":0}' ...
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { cpus } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acquireBenchmarkLock } from "./bench-lock.mjs";
import {
  alternatingOrder,
  candidateVerdict,
  cpuProbe,
  median,
} from "./bench-math.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const artifactsArg = process.env.ARTIFACTS || process.env.SC;
const BASE_WASM = process.env.BASE_WASM ? resolve(process.env.BASE_WASM) : null;
const CANDIDATE_WASM =
  (process.env.CANDIDATE_WASM ? resolve(process.env.CANDIDATE_WASM) : null) ||
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
if (!artifactsArg || !BASE_WASM) {
  console.error("set ARTIFACTS=<dir> and BASE_WASM=<control.wasm>");
  process.exit(2);
}
const ARTIFACTS = resolve(artifactsArg);
const releaseBenchmarkLock = await acquireBenchmarkLock(ARTIFACTS);

const ROWS = {
  alu: { lower: true },
  mixed: { lower: true },
  boot: { lower: true },
  python: { lower: true },
  compile: { lower: true },
  numeric: { lower: false },
  string: { lower: false },
  bitfield: { lower: false },
  fpemul: { lower: false },
  fourier: { lower: false },
  assignment: { lower: false },
  idea: { lower: false },
  huffman: { lower: false },
};
const selected = (process.env.ROWS || "compile,numeric,assignment")
  .split(",")
  .map((row) => row.trim().toLowerCase())
  .filter(Boolean);
if (!selected.length || new Set(selected).size !== selected.length) {
  throw new Error("ROWS must contain one or more distinct benchmark names");
}
for (const row of selected) {
  if (!ROWS[row]) {
    console.error(`unknown row "${row}"; choose: ${Object.keys(ROWS).join(",")}`);
    process.exit(2);
  }
}
const REPS = Number(process.env.REPS ?? 3);
if (!Number.isSafeInteger(REPS) || REPS < 1) {
  throw new Error("REPS must be a positive integer");
}
const PROFILE = process.env.PROFILE === "1";
const DRIFT_TOL = 1.25;
const KNOBS = new Set([
  "SB",
  "TLBFILL",
  "TRACELVL",
  "KEEPMIN",
  "DEMOTE",
  "BATCH",
  "ICTRIG",
  "DEFTRACK",
  "ROTNEST",
  "BCAP",
  "BPAGE",
  "SBSPACE",
]);

function parseConfig(name) {
  let config;
  try {
    config = JSON.parse(process.env[name] || "{}");
  } catch (error) {
    throw new Error(`${name} is not valid JSON: ${error.message}`);
  }
  if (!config || Array.isArray(config) || typeof config !== "object") {
    throw new Error(`${name} must be a JSON object`);
  }
  for (const key of Object.keys(config)) {
    if (!KNOBS.has(key)) throw new Error(`${name} contains unsupported knob ${key}`);
  }
  return Object.fromEntries(
    Object.entries(config)
      .map(([key, value]) => {
        if (!Number.isFinite(Number(value))) {
          throw new Error(`${name}.${key} must be numeric`);
        }
        return [key, String(value)];
      })
      .sort(([a], [b]) => a.localeCompare(b)),
  );
}
const withDefaults = (config) =>
  Object.fromEntries(
    Object.entries({ SB: "1", ...config }).sort(([a], [b]) =>
      a.localeCompare(b)
    ),
  );
const baseConfig = withDefaults(parseConfig("BASE_CONFIG"));
const candidateConfig = withDefaults(parseConfig("CANDIDATE_CONFIG"));
const cleanChildEnv = { ...process.env };
for (const knob of KNOBS) delete cleanChildEnv[knob];
const baseBytes = await readFile(BASE_WASM);
const candidateBytes = await readFile(CANDIDATE_WASM);
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const baseSha = sha(baseBytes);
const candidateSha = sha(candidateBytes);
if (
  baseSha === candidateSha &&
  JSON.stringify(baseConfig) === JSON.stringify(candidateConfig) &&
  process.env.ALLOW_IDENTICAL !== "1"
) {
  throw new Error(
    "control and candidate wasm/config are identical; set ALLOW_IDENTICAL=1 only for a repeatability audit",
  );
}

function runWorker(row, wasm, config) {
  return new Promise((resolve) => {
    const child = spawn(
      process.execPath,
      [join(root, "tests/vs-v86/rv64-scorecard-worker.mjs"), row],
      {
        cwd: root,
        env: {
          ...cleanChildEnv,
          ARTIFACTS,
          WASM: wasm,
          PROFILE: PROFILE ? "1" : "0",
          DISABLE_JIT: "0",
          ...config,
        },
      },
    );
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (data) => (stdout += data));
    child.stderr.on("data", (data) => (stderr += data));
    child.on("close", (code) => {
      const match = stdout.match(/^RESULT_JSON (.+)$/m);
      if (code !== 0 || !match) {
        resolve({
          error: `exit=${code}: ${stderr || stdout.slice(-1000)}`,
        });
        return;
      }
      try {
        resolve(JSON.parse(match[1]));
      } catch (error) {
        resolve({ error: `invalid worker JSON: ${error.message}` });
      }
    });
  });
}

cpuProbe();

const trials = [];
for (const row of selected) {
  process.stderr.write(`[A/B ${row}]`);
  for (let rep = 0; rep < REPS; rep++) {
    const order = alternatingOrder(rep, "base", "candidate");
    for (const side of order) {
      process.stderr.write(` ${rep + 1}${side === "base" ? "a" : "b"}`);
      const probeBefore = cpuProbe();
      const result = await runWorker(
        row,
        side === "base" ? BASE_WASM : CANDIDATE_WASM,
        side === "base" ? baseConfig : candidateConfig,
      );
      const probeAfter = cpuProbe();
      trials.push({
        row,
        rep: rep + 1,
        side,
        order: order.indexOf(side) + 1,
        probe_before_ms: probeBefore,
        probe_after_ms: probeAfter,
        result,
      });
    }
  }
  process.stderr.write(" ok\n");
}

const madPct = (values) => {
  const center = median(values);
  return center
    ? (median(values.map((value) => Math.abs(value - center))) / center) * 100
    : null;
};
const fmt = (value) =>
  value >= 100_000 ? value.toExponential(3) : value.toFixed(2);
const issues = [];
const results = {};

for (const row of selected) {
  const samples = Object.fromEntries(
    ["base", "candidate"].map((side) => [
      side,
      trials.filter((trial) => trial.row === row && trial.side === side),
    ]),
  );
  for (const side of ["base", "candidate"]) {
    const complete = samples[side].filter(
      (trial) =>
        !trial.result?.error &&
        Number.isFinite(trial.result?.value),
    );
    if (complete.length !== REPS) {
      issues.push(`${row}: ${side} has ${complete.length}/${REPS} complete trials`);
    }
    const expectedSha = side === "base" ? baseSha : candidateSha;
    for (const trial of complete) {
      if (trial.result.wasm_sha256 !== expectedSha) {
        issues.push(
          `${row} rep ${trial.rep}: ${side} used wasm ${trial.result.wasm_sha256}, expected ${expectedSha}`,
        );
      }
    }
  }
  const baseValues = samples.base
    .map((trial) => trial.result?.value)
    .filter(Number.isFinite);
  const candidateValues = samples.candidate
    .map((trial) => trial.result?.value)
    .filter(Number.isFinite);
  if (!baseValues.length || !candidateValues.length) continue;

  const baseMedian = median(baseValues);
  const candidateMedian = median(candidateValues);
  const speed = ROWS[row].lower
    ? baseMedian / candidateMedian
    : candidateMedian / baseMedian;
  const verdict = PROFILE ? "DIAGNOSTIC" : candidateVerdict(speed);
  const fingerprints = {};
  for (const side of ["base", "candidate"]) {
    fingerprints[side] = [
      ...new Set(
        samples[side]
          .map(
            (trial) =>
              trial.result?.md5 ?? trial.result?.checksum ?? null,
          )
          .filter(Boolean),
      ),
    ];
    if (fingerprints[side].length > 1) {
      issues.push(`${row}: ${side} correctness fingerprint varied`);
    }
  }
  if (
    fingerprints.base.length &&
    fingerprints.candidate.length &&
    fingerprints.base[0] !== fingerprints.candidate[0]
  ) {
    issues.push(`${row}: control/candidate correctness fingerprints differ`);
  }
  const inputFingerprints = [
    ...new Set(
      [...samples.base, ...samples.candidate]
        .map((trial) =>
          trial.result?.input_sha256
            ? JSON.stringify(trial.result.input_sha256)
            : null
        )
        .filter(Boolean),
    ),
  ];
  if (
    inputFingerprints.length !== 1 ||
    [...samples.base, ...samples.candidate].some(
      (trial) => !trial.result?.error && !trial.result?.input_sha256,
    )
  ) {
    issues.push(`${row}: workload input hashes are missing or inconsistent`);
  }
  const unstableReps = Object.fromEntries(
    ["base", "candidate"].map((side) => [
      side,
      samples[side].filter((trial) => (trial.result?.unstable ?? 0) > 0)
        .length,
    ]),
  );
  if (!ROWS[row].lower) {
    for (const side of ["base", "candidate"]) {
      if (unstableReps[side] > REPS / 2) {
        issues.push(
          `${row}: ${side} reported nbench instability in ${unstableReps[side]}/${REPS} trials`,
        );
      }
    }
  }

  const statMedian = (side, field) => {
    const values = samples[side]
      .map((trial) => trial.result?.jit?.[field])
      .filter(Number.isFinite);
    return values.length ? median(values) : null;
  };
  results[row] = {
    lower_is_better: ROWS[row].lower,
    base: {
      median: baseMedian,
      mad_pct: madPct(baseValues),
      samples: baseValues,
      dispatches: statMedian("base", "dispatches"),
      insns_per_dispatch: statMedian("base", "insns_per_dispatch"),
      jit_retired: statMedian("base", "jit_retired"),
      unstable_reps: unstableReps.base,
    },
    candidate: {
      median: candidateMedian,
      mad_pct: madPct(candidateValues),
      samples: candidateValues,
      dispatches: statMedian("candidate", "dispatches"),
      insns_per_dispatch: statMedian("candidate", "insns_per_dispatch"),
      jit_retired: statMedian("candidate", "jit_retired"),
      unstable_reps: unstableReps.candidate,
    },
    candidate_speedup: speed,
    verdict,
    fingerprints,
    input_sha256:
      inputFingerprints.length === 1
        ? JSON.parse(inputFingerprints[0])
        : null,
  };
}

for (const trial of trials) {
  const spread =
    Math.max(trial.probe_before_ms, trial.probe_after_ms) /
    Math.min(trial.probe_before_ms, trial.probe_after_ms);
  if (spread > DRIFT_TOL) {
    issues.push(
      `${trial.row} rep ${trial.rep} ${trial.side}: host probe drifted ${spread.toFixed(2)}x`,
    );
  }
}
const allProbes = trials.flatMap((trial) => [
  trial.probe_before_ms,
  trial.probe_after_ms,
]);
const globalDrift = Math.max(...allProbes) / Math.min(...allProbes);
if (globalDrift > DRIFT_TOL) {
  issues.push(`host probe spread ${globalDrift.toFixed(2)}x exceeds ${DRIFT_TOL}x`);
}

console.log(
  `\nSERIAL FRESH-PROCESS A/B — ${REPS} paired repetition(s), alternating order`,
);
console.log(
  "row".padEnd(12) +
    "control".padStart(12) +
    "candidate".padStart(12) +
    "speed".padStart(10) +
    "  result".padEnd(15) +
    "  MAD a/b",
);
for (const [row, result] of Object.entries(results)) {
  console.log(
    row.padEnd(12) +
      fmt(result.base.median).padStart(12) +
      fmt(result.candidate.median).padStart(12) +
      `${result.candidate_speedup.toFixed(3)}x`.padStart(10) +
      `  ${result.verdict}`.padEnd(15) +
      `  ${result.base.mad_pct.toFixed(1)}%/${result.candidate.mad_pct.toFixed(1)}%`,
  );
  console.log(
    `  raw control=[${result.base.samples.map(fmt).join(", ")}] candidate=[${result.candidate.samples.map(fmt).join(", ")}]`,
  );
  if (result.base.dispatches != null && result.candidate.dispatches != null) {
    console.log(
      `  dispatches ${fmt(result.base.dispatches)} -> ${fmt(result.candidate.dispatches)}; ` +
        `insns/dispatch ${fmt(result.base.insns_per_dispatch)} -> ${fmt(result.candidate.insns_per_dispatch)}`,
    );
  }
}
console.log(
  `host probe spread ${globalDrift.toFixed(2)}x; ${issues.length ? "INVALID" : "valid"}`,
);
console.log(
  "SCREEN ONLY: promote a >=10% improvement through the authoritative 13-row scorecard.",
);
if (issues.length) {
  console.log(`issues: ${issues.join("; ")}`);
  process.exitCode = 1;
}

const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const reportPath = join(ARTIFACTS, `ab-${ts}.json`);
await writeFile(
  reportPath,
  JSON.stringify(
    {
      schema: 1,
      ts,
      valid: issues.length === 0,
      rows: selected,
      reps: REPS,
      profile: PROFILE,
      host_logical_cpus: cpus().length,
      control: {
        wasm: BASE_WASM,
        wasm_sha256: baseSha,
        config: baseConfig,
      },
      candidate: {
        wasm: CANDIDATE_WASM,
        wasm_sha256: candidateSha,
        config: candidateConfig,
      },
      results,
      trials,
      host_probe_spread: globalDrift,
      issues,
    },
    null,
    2,
  ),
);
console.log(`saved ${reportPath}`);
await releaseBenchmarkLock();
