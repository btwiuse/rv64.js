#!/usr/bin/env node

// Pre-register and execute a fixed WANIX noninferiority experiment. Every leg
// gets a new browser process/profile through wanix-v86-matched-smoke.mjs. Pair
// order alternates to balance monotonic host drift, and the runner stops on the
// first failed leg instead of extending a sample after observing its result.

import assert from "node:assert/strict";
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const directoryArgument = process.argv.slice(2).find((argument) => !argument.startsWith("--"));
const pairsArgument = process.argv.find((argument) => argument.startsWith("--pairs="));
const slowdownArgument = process.argv.find((argument) => argument.startsWith("--max-slowdown="));
const jitConfigArgument = process.argv.find((argument) => argument.startsWith("--jit-config="));
const expectedWasmArgument = process.argv.find(
  (argument) => argument.startsWith("--expected-wasm-sha256="),
);
const r076CandidateV86 = process.argv.includes("--r076-candidate-v86");
if (
  !directoryArgument ||
  (r076CandidateV86 ? process.argv.length !== 4 : !pairsArgument)
) {
  throw new Error(
    "usage: run-wanix-pairs.mjs RESULTS_DIR --pairs=N [--max-slowdown=0.10] " +
      "[--jit-config=JSON] [--expected-wasm-sha256=HEX] | " +
      "RESULTS_DIR --r076-candidate-v86",
  );
}

const resultsDirectory = resolve(directoryArgument);
const pairs = r076CandidateV86 ? 7 : Number(pairsArgument.split("=", 2)[1]);
const maxSlowdown = r076CandidateV86
  ? 0.10
  : Number(slowdownArgument?.split("=", 2)[1] ?? "0.10");
const cpuAffinity = process.env.WANIX_CPU_AFFINITY;
const url = r076CandidateV86
  ? "http://127.0.0.1:8765/examples/" +
    "v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html"
  : process.env.WANIX_URL ||
    "http://127.0.0.1:8765/examples/v86-rv64-three-way.html";
const jitConfiguration = r076CandidateV86
  ? {
    staticSystemT0: false,
    sampledStaticT0: true,
    sampledStaticT0Backoff: true,
  }
  : jitConfigArgument
  ? JSON.parse(jitConfigArgument.slice("--jit-config=".length))
  : {};
const expectedMainWasmSha256 = r076CandidateV86
  ? "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c"
  : expectedWasmArgument
  ? expectedWasmArgument.slice("--expected-wasm-sha256=".length)
  : null;
const repetitions = r076CandidateV86 ? 3 : 1;
assert.ok(Number.isInteger(pairs) && pairs >= 3 && pairs <= 7, "--pairs must be from 3 through 7");
assert.ok(
  Number.isFinite(maxSlowdown) && maxSlowdown >= 0 && maxSlowdown <= 1,
  "--max-slowdown must be a fraction from 0 through 1",
);
assert.match(
  cpuAffinity ?? "",
  /^\d+(?:[-,]\d+)*$/,
  "WANIX_CPU_AFFINITY is required (for example, 8-15)",
);
if (r076CandidateV86) {
  assert.equal(cpuAffinity, "8-15", "R076 WANIX guard must be pinned to CPUs 8-15");
}
assert.ok(
  jitConfiguration && typeof jitConfiguration === "object" && !Array.isArray(jitConfiguration),
  "--jit-config must be a JSON object",
);
assert.ok(
  expectedMainWasmSha256 === null || /^[0-9a-f]{64}$/.test(expectedMainWasmSha256),
  "--expected-wasm-sha256 must be a lowercase SHA-256 digest",
);

// Reserve the benchmark host for the complete pre-registered experiment, not
// just one browser leg. Otherwise an unrelated harness can slip between two
// paired samples (or make the next leg fail its lock acquisition) after the
// protocol has already been fixed.
const benchmarkLockPath = resolve(tmpdir(), "rv64-wanix-matched-smoke.lock");
function processIsAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code !== "ESRCH";
  }
}
function acquireBenchmarkLock() {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      mkdirSync(benchmarkLockPath);
      writeFileSync(resolve(benchmarkLockPath, "pid"), `${process.pid}\n`);
      return;
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      let owner = 0;
      try {
        owner = Number(readFileSync(resolve(benchmarkLockPath, "pid"), "utf8").trim());
      } catch {}
      if (Number.isSafeInteger(owner) && owner > 0 && processIsAlive(owner)) {
        throw new Error(`another WANIX benchmark is running (pid ${owner})`);
      }
      rmSync(benchmarkLockPath, { recursive: true, force: true });
    }
  }
  throw new Error("could not acquire the WANIX benchmark lock");
}
let benchmarkLockHeld = false;
function releaseBenchmarkLock() {
  if (!benchmarkLockHeld) return;
  rmSync(benchmarkLockPath, { recursive: true, force: true });
  benchmarkLockHeld = false;
}

acquireBenchmarkLock();
benchmarkLockHeld = true;
process.on("exit", releaseBenchmarkLock);

assert.ok(!existsSync(resultsDirectory), "results directory already exists; use a new directory");
mkdirSync(resultsDirectory, { recursive: true });

const order = Array.from({ length: pairs }, (_, index) => ({
  pair: index + 1,
  vms: index % 2 === 0 ? ["rv64-jit", "x86"] : ["x86", "rv64-jit"],
}));
const harness = resolve(
  dirname(fileURLToPath(import.meta.url)),
  r076CandidateV86
    ? "wanix-v86-preboot-smoke.mjs"
    : "wanix-v86-matched-smoke.mjs",
);
const toolManifest = r076CandidateV86 ? (() => {
  const digest = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
  const manifest = {
    runner: digest(fileURLToPath(import.meta.url)),
    harness: digest(harness),
  };
  assert.equal(
    manifest.harness,
    "525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545",
    "frozen R076 WANIX harness changed",
  );
  return manifest;
})() : null;
const protocol = {
  schema: r076CandidateV86 ? 3 : 2,
  ...(r076CandidateV86 ? {
    experiment: "R076 preboot sampled-backoff candidate versus copy/v86",
  } : {}),
  plannedAt: new Date().toISOString(),
  pairs,
  maxSlowdown,
  url,
  cpuAffinity,
  browserExecutable: process.env.CHROME ?? null,
  phases: ["python", "sha256", "shared9p"],
  repetitions,
  phaseSync: true,
  jitPolicy: null,
  jitConfiguration,
  expectedMainWasmSha256,
  ...(r076CandidateV86 ? {
    expectedBrowser: {
      protocolVersion: "1.3",
      product: "Chrome/150.0.7871.186",
      revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
      userAgent:
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
        "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
      jsVersion: "15.0.245.21",
    },
    expectedArtifacts: {
      page: "7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413",
      rv64Jit: "e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c",
      rv64Root: "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb",
      v86: "7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771",
      x86Root: "09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320",
    },
    maximumPairedMedianBootstrapUpper: 1.10,
    jitConfigurationLifecycle: "preboot",
    requiredCandidateProof: [
      "staticT0FastRetired",
      "sampledStaticT0Retired",
      "sampledStaticT0Samples",
      "sampledStaticT0InterruptPolls",
      "sampledStaticT0ShortMarks",
      "sampledStaticT0ShortBypasses",
    ],
    toolManifest,
  } : {}),
  // Compute phases must overwhelmingly execute generated code. shared9p is
  // deliberately a device/syscall-path workload, so its JIT-active gate is
  // chain activity plus nonzero generated retirement rather than an arbitrary
  // compute-coverage percentage.
  minimumGeneratedCoverage: {
    python: 0.90,
    sha256: 0.90,
    shared9p: 0,
  },
  order,
};
writeFileSync(
  resolve(resultsDirectory, "protocol.json"),
  `${JSON.stringify(protocol, null, 2)}\n`,
  { flag: "wx" },
);

let activeChild = null;

function runLeg(pair, vm) {
  return new Promise((resolveRun, rejectRun) => {
    const outputPath = resolve(resultsDirectory, `pair-${pair}-${vm}.log`);
    const errorPath = resolve(resultsDirectory, `pair-${pair}-${vm}.stderr.log`);
    const output = openSync(outputPath, "wx");
    const error = openSync(errorPath, "wx");
    const environment = {
      ...process.env,
      WANIX_URL: url,
      WANIX_VM: vm,
      WANIX_BENCH_PHASES: protocol.phases.join(","),
      WANIX_BENCH_REPETITIONS: String(repetitions),
      WANIX_BENCH_PHASE_SYNC: "1",
      WANIX_SUMMARY_ONLY: "1",
      WANIX_ALLOW_PARALLEL: "1",
    };
    for (const name of [
      "WANIX_JIT_CONFIG",
      "WANIX_JIT_POLICY",
      "WANIX_JIT_PROFILE",
      "WANIX_JIT_PREBOOT",
    ]) {
      delete environment[name];
    }
    if (vm === "rv64-jit" && Object.keys(jitConfiguration).length) {
      environment.WANIX_JIT_CONFIG = JSON.stringify(jitConfiguration);
      if (r076CandidateV86) environment.WANIX_JIT_PREBOOT = "1";
    }
    const child = spawn(
      "taskset",
      ["-c", cpuAffinity, process.execPath, harness],
      { env: environment, stdio: ["ignore", output, error] },
    );
    activeChild = child;
    child.once("error", (spawnError) => {
      activeChild = null;
      closeSync(output);
      closeSync(error);
      rejectRun(spawnError);
    });
    child.once("exit", (code, signal) => {
      activeChild = null;
      closeSync(output);
      closeSync(error);
      if (code === 0) resolveRun();
      else rejectRun(new Error(`pair ${pair} ${vm} failed (code=${code}, signal=${signal})`));
    });
  });
}

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, () => {
    activeChild?.kill(signal);
    process.exitCode = 128;
  });
}

for (const { pair, vms } of order) {
  for (const vm of vms) {
    const label = r076CandidateV86 ? "R076 WANIX-v86" : "WANIX";
    process.stderr.write(`${label} pair ${pair}/${pairs}: ${vm}\n`);
    await runLeg(pair, vm);
  }
}

process.stderr.write(
  `${r076CandidateV86 ? "R076 WANIX-v86" : "WANIX"} fixed sample complete: ` +
    `${resultsDirectory}\n`,
);
