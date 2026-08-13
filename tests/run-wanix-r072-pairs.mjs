#!/usr/bin/env node

// Execute the frozen R072 browser candidate/control gate. Every leg starts a
// fresh browser process, profile, RV64 Worker, and guest. Both sides load the
// same archive and prepare the same auxiliary module; only its two enable bits
// differ. This runner intentionally has no configuration or sample-count knob.
// The named confirmation mode is a separate, fully fresh protocol that keeps
// the first schema-1 invalid result immutable while restoring the established
// browser validity method (which never imposed a within-side 9P spread cap).

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

const resultsArgument = process.argv[2];
const establishedValidityConfirmation = process.argv[3] ===
  "--established-validity-confirmation";
const sampledOnlyR073 = process.argv[3] === "--r073-sampled-only";
const shortBackoffR074 = process.argv[3] === "--r074-short-backoff";
const prebootR075 = process.argv[3] === "--r075-preboot";
const hasMode = establishedValidityConfirmation || sampledOnlyR073 ||
  shortBackoffR074 || prebootR075;
if (
  !resultsArgument ||
  resultsArgument.startsWith("--") ||
  process.argv.length !== (hasMode ? 4 : 3)
) {
  throw new Error(
    "usage: node tests/run-wanix-r072-pairs.mjs RESULTS_DIR " +
      "[--established-validity-confirmation|--r073-sampled-only|" +
      "--r074-short-backoff|--r075-preboot]",
  );
}

const resultsDirectory = resolve(resultsArgument);
const strengthenedBrowserProtocol = sampledOnlyR073 || shortBackoffR074 || prebootR075;
const pairs = strengthenedBrowserProtocol ? 7 : 5;
const repetitions = strengthenedBrowserProtocol ? 3 : 1;
const maxSlowdown = 0.03;
const cpuAffinity = process.env.WANIX_CPU_AFFINITY;
const url = process.env.WANIX_URL ||
  "http://127.0.0.1:8765/examples/v86-rv64-three-way.html";
const prebootUrls = Object.freeze({
  control:
    "http://127.0.0.1:8765/examples/" +
    "v86-rv64-three-way-r075-control-e0c1971d1ecd4d4f.html",
  candidate:
    "http://127.0.0.1:8765/examples/" +
    "v86-rv64-three-way-r075-candidate-e0c1971d1ecd4d4f.html",
});
const configurations = Object.freeze(shortBackoffR074 || prebootR075 ? {
  control: {
    staticSystemT0: false,
    sampledStaticT0: false,
    sampledStaticT0Backoff: false,
  },
  candidate: {
    staticSystemT0: false,
    sampledStaticT0: true,
    sampledStaticT0Backoff: true,
  },
} : {
  control: { staticSystemT0: false, sampledStaticT0: false },
  candidate: {
    staticSystemT0: sampledOnlyR073 ? false : true,
    sampledStaticT0: true,
  },
});
assert.match(
  cpuAffinity ?? "",
  /^\d+(?:[-,]\d+)*$/,
  "WANIX_CPU_AFFINITY is required (for example, 8-15)",
);
assert.ok(!existsSync(resultsDirectory), "results directory already exists; use a new directory");

const lockPath = resolve(tmpdir(), "rv64-wanix-matched-smoke.lock");
function processIsAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code !== "ESRCH";
  }
}
function acquireLock() {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      mkdirSync(lockPath);
      writeFileSync(resolve(lockPath, "pid"), `${process.pid}\n`);
      return;
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      let owner = 0;
      try {
        owner = Number(readFileSync(resolve(lockPath, "pid"), "utf8").trim());
      } catch {}
      if (Number.isSafeInteger(owner) && owner > 0 && processIsAlive(owner)) {
        throw new Error(`another WANIX benchmark is running (pid ${owner})`);
      }
      rmSync(lockPath, { recursive: true, force: true });
    }
  }
  throw new Error("could not acquire the WANIX benchmark lock");
}
let lockHeld = false;
function releaseLock() {
  if (!lockHeld) return;
  rmSync(lockPath, { recursive: true, force: true });
  lockHeld = false;
}

acquireLock();
lockHeld = true;
process.on("exit", releaseLock);
mkdirSync(resultsDirectory, { recursive: true });

const order = Array.from({ length: pairs }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "candidate"] : ["candidate", "control"],
}));
const protocol = {
  schema: prebootR075 ? 5 : shortBackoffR074 ? 4 : sampledOnlyR073 ? 3 :
    establishedValidityConfirmation ? 2 : 1,
  experiment: prebootR075
    ? "R075 preboot sampled-backoff browser candidate/control"
    : shortBackoffR074
    ? "R074 short-sample backoff browser candidate/control"
    : sampledOnlyR073
      ? "R073 sampled-only browser candidate/control"
    : establishedValidityConfirmation
      ? "R072 independent browser confirmation"
      : "R072 browser candidate/control",
  plannedAt: new Date().toISOString(),
  pairs,
  maxSlowdown,
  ...(prebootR075 ? { urls: prebootUrls } : { url }),
  cpuAffinity,
  browserExecutable: process.env.CHROME ?? null,
  ...(shortBackoffR074 || prebootR075 ? {
    expectedBrowser: {
      protocolVersion: "1.3",
      product: "Chrome/150.0.7871.186",
      revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
      userAgent:
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
        "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
      jsVersion: "15.0.245.21",
    },
  } : {}),
  vm: "rv64-jit",
  phases: ["python", "sha256", "shared9p"],
  repetitions,
  phaseSync: true,
  configurations,
  expectedMainWasmSha256:
    shortBackoffR074 || prebootR075
      ? "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c"
      : "cb7ea81685b3cb9605f6e01b619f3c15005f4ef77ca00c258c20c7a519bb6df6",
  expectedPageSha256:
    prebootR075
      ? {
        control: "a2f43c06f86507c267c36fb3922079d11d44072ab0622526e5577f69448e976f",
        candidate: "7fdf40236c59c27d1b8c7b6f7d45ae5e07784cb31bd04223b8cfc79399fe5413",
      }
      : shortBackoffR074
      ? "bdc4827f2a9b86eee1ce4443a9914eae4ef8e5c4ff8329b81973343feccb1a64"
      : sampledOnlyR073
      ? "1c70b211272fd9a843bfe52aefe804322d7260a144df7195a34363ad9f259aee"
      : "28957e0d5ce381184addb291805ba26a6e64d421a51882c4ae56e0512a82cd3d",
  expectedRv64ArchiveSha256:
    prebootR075
      ? "e0c1971d1ecd4d4f9b7674388b62cd30501fe228e8834166c46abf38034d190c"
      : shortBackoffR074
      ? "917ddcad15a15fa6560c480b9b19ccc2d39ec52ceed65030c94c79f0805df2a9"
      : sampledOnlyR073
      ? "2b52e552d00929fa4c525c5b1aabc7abbce74d7d3ffe571a0e28d7d3b1cf199e"
      : "159fc55c4337345a685252e384d64be39fc50c743b4478e2b864289ad8bb8690",
  ...(shortBackoffR074 || prebootR075 ? {
    expectedHarnessSha256:
      prebootR075
        ? "525b241605244148df71bc52ace6686cfb35935c91a2535d9030e5f7a42bb545"
        : "c3f32eee15012ecc53da541bb3e3b1bda798ae1983d2b3a1e1bcd90dcb4e7495",
  } : {}),
  maximumWithinSideSpread:
    establishedValidityConfirmation || strengthenedBrowserProtocol ? null : 1.25,
  maximumPairedMedianBootstrapUpper: strengthenedBrowserProtocol ? 1.10 : null,
  ...(prebootR075 ? {
    minimumShellSpeedup: 1.10,
    minimumShellMedianBootstrapLower: 1.00,
    jitConfigurationLifecycle: "preboot",
  } : {}),
  ...(establishedValidityConfirmation ? {
    supersedesInvalidReport: {
      path: "target/jit-policy-traces/wanix-r072-cb7ea816-chrome-20260809-config-ab/analysis.json",
      sha256: "91da7722e3289f586c89c5fd01c623c0e82c95c1d15b9a13b989e86ef5b08776",
      reason: "new 1.25 within-side cap was incompatible with established shared-9P variance",
    },
  } : {}),
  minimumGeneratedCoverage: { python: 0.90, sha256: 0.90, shared9p: 0 },
  requiredCandidateProof: [
    "staticT0FastRetired",
    "sampledStaticT0Retired",
    "sampledStaticT0Samples",
    "sampledStaticT0InterruptPolls",
    ...(shortBackoffR074 || prebootR075 ? [
      "sampledStaticT0ShortMarks",
      "sampledStaticT0ShortBypasses",
    ] : []),
  ],
  order,
};
writeFileSync(
  resolve(resultsDirectory, "protocol.json"),
  `${JSON.stringify(protocol, null, 2)}\n`,
  { flag: "wx" },
);

const harness = resolve(
  dirname(fileURLToPath(import.meta.url)),
  prebootR075 ? "wanix-v86-preboot-smoke.mjs" : "wanix-v86-matched-smoke.mjs",
);
if (shortBackoffR074 || prebootR075) {
  assert.equal(
    createHash("sha256").update(readFileSync(harness)).digest("hex"),
    protocol.expectedHarnessSha256,
    "frozen WANIX harness changed",
  );
}
let activeChild = null;

function runLeg(pair, side) {
  return new Promise((resolveRun, rejectRun) => {
    const output = openSync(resolve(resultsDirectory, `pair-${pair}-${side}.log`), "wx");
    const error = openSync(
      resolve(resultsDirectory, `pair-${pair}-${side}.stderr.log`),
      "wx",
    );
    const environment = { ...process.env };
    for (const name of [
      "WANIX_JIT_CONFIG", "WANIX_JIT_POLICY", "WANIX_JIT_PROFILE", "WANIX_JIT_PREBOOT",
    ]) {
      delete environment[name];
    }
    Object.assign(environment, {
      WANIX_URL: prebootR075 ? prebootUrls[side] : url,
      WANIX_VM: "rv64-jit",
      WANIX_JIT_CONFIG: JSON.stringify(configurations[side]),
      WANIX_BENCH_PHASES: protocol.phases.join(","),
      WANIX_BENCH_REPETITIONS: String(protocol.repetitions),
      WANIX_BENCH_PHASE_SYNC: "1",
      WANIX_SUMMARY_ONLY: "1",
      WANIX_ALLOW_PARALLEL: "1",
      ...(prebootR075 ? { WANIX_JIT_PREBOOT: "1" } : {}),
    });
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
      else rejectRun(new Error(`pair ${pair} ${side} failed (code=${code}, signal=${signal})`));
    });
  });
}

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, () => {
    activeChild?.kill(signal);
    process.exitCode = 128;
  });
}

for (const { pair, sides } of order) {
  for (const side of sides) {
    const label = prebootR075 ? "R075" : shortBackoffR074 ? "R074" :
      sampledOnlyR073 ? "R073" : "R072";
    process.stderr.write(`${label} browser pair ${pair}/${pairs}: ${side}\n`);
    await runLeg(pair, side);
  }
}

const label = prebootR075 ? "R075" : shortBackoffR074 ? "R074" :
  sampledOnlyR073 ? "R073" : "R072";
process.stderr.write(
  `${label} fixed browser sample complete: ${resultsDirectory}\n`,
);
