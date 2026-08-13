#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  existsSync, mkdirSync, readFileSync, rmSync, writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  browserExecutable, r093AssetManifest, runR093BrowserSample, startR093Server,
} from "./vs-v86/r093-browser-host.mjs";

if (process.argv.length !== 3) {
  throw new Error("usage: node tests/run-r093-chrome-boot-pairs.mjs RESULTS_DIR");
}
const resultsDirectory = resolve(process.argv[2]);
assert.ok(!existsSync(resultsDirectory), "results directory already exists; use a new directory");
const hostCpuAffinity = readFileSync(`/proc/${process.pid}/status`, "utf8")
  .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null;
assert.equal(hostCpuAffinity, "8-15", "R093 formal runner must be pinned to CPUs 8-15");

const lockPath = resolve(tmpdir(), "rv64-r093-chrome-boot.lock");
function processIsAlive(pid) {
  try { process.kill(pid, 0); return true; } catch (error) { return error?.code !== "ESRCH"; }
}
for (let attempt = 0; attempt < 3; attempt++) {
  try {
    mkdirSync(lockPath);
    writeFileSync(resolve(lockPath, "pid"), `${process.pid}\n`);
    break;
  } catch (error) {
    if (error?.code !== "EEXIST") throw error;
    let owner = 0;
    try { owner = Number(readFileSync(resolve(lockPath, "pid"), "utf8").trim()); } catch {}
    if (Number.isSafeInteger(owner) && owner > 0 && processIsAlive(owner)) {
      throw new Error(`another R093 browser Boot benchmark is running (pid ${owner})`);
    }
    rmSync(lockPath, { recursive: true, force: true });
    if (attempt === 2) throw new Error("could not acquire the R093 browser Boot lock");
  }
}
let lockHeld = true;
function releaseLock() {
  if (!lockHeld) return;
  rmSync(lockPath, { recursive: true, force: true });
  lockHeld = false;
}
process.on("exit", releaseLock);

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const digestFile = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
const pairs = 7;
const order = Array.from({ length: pairs }, (_, index) => ({
  pair: index + 1,
  sides: index % 2 === 0 ? ["control", "candidate"] : ["candidate", "control"],
}));
const expectedBrowser = {
  protocolVersion: "1.3",
  product: "Chrome/150.0.7871.186",
  revision: "@0fcdce5f4fdec8d442d7df760cb541f1ca6e446d",
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 " +
    "(KHTML, like Gecko) HeadlessChrome/150.0.0.0 Safari/537.36",
  jsVersion: "15.0.245.21",
};
const thresholds = {
  minimumPairedMedianSpeedup: 1 / 1.03,
  minimumPairedBootstrapLower: 0.95,
};
const protocol = {
  schema: 1,
  experiment: "R093 artifact A/B Chrome execution-only modern Boot",
  plannedAt: new Date().toISOString(),
  pairs,
  order,
  hostCpuAffinity,
  browserExecutable: browserExecutable(),
  expectedBrowser,
  configurations: {
    control: { wasm: "R085", policy: "production-page" },
    candidate: { wasm: "R093", policy: "production-page" },
  },
  assetManifest: r093AssetManifest(),
  toolManifest: {
    host: digestFile(resolve(root, "tests/vs-v86/r093-browser-host.mjs")),
    runner: digestFile(fileURLToPath(import.meta.url)),
    analyzer: digestFile(resolve(root, "tests/analyze-r093-chrome-boot-pairs.mjs")),
    selftest: digestFile(resolve(root, "tests/vs-v86/r093-harness-selftest.mjs")),
    worker: digestFile(resolve(root, "tests/jit-modern-boot-r093-browser-worker.mjs")),
    timingLibrary: digestFile(resolve(root, "tests/vs-v86/r076-browser-boot-lib.mjs")),
  },
  timerContract: {
    setupBeforeTimer: [
      "asset-fetch-and-sha256", "RV64.create", "production-policy",
      "bootVirtLinuxDirect", "initial-counters",
    ],
    timed: "runTimedBoot:first-2M-pump-through-SCORECARD_V2_READY",
    quantum: "2000000",
    cadence: "yield-after-pump-1-then-every-fourth",
    marker: "SCORECARD_V2_READY",
  },
  guest: { linux: "6.12.7", alpine: "3.24.1", arch: "riscv64" },
  thresholds,
};
mkdirSync(resultsDirectory, { recursive: true });
writeFileSync(resolve(resultsDirectory, "protocol.json"), `${JSON.stringify(protocol, null, 2)}\n`, {
  flag: "wx",
});

const server = await startR093Server();
try {
  for (const { pair, sides } of order) {
    for (const side of sides) {
      process.stderr.write(`R093 Chrome Boot pair ${pair}/${pairs}: ${side}\n`);
      const sample = await runR093BrowserSample({
        origin: server.origin,
        variant: side,
        chrome: protocol.browserExecutable,
      });
      assert.deepEqual(sample.browser, expectedBrowser, "browser identity changed");
      writeFileSync(
        resolve(resultsDirectory, `pair-${pair}-${side}.json`),
        `${JSON.stringify({ ...sample, hostCpuAffinity }, null, 2)}\n`,
        { flag: "wx" },
      );
    }
  }
} finally {
  await server.close();
  releaseLock();
}
process.stderr.write(`R093 fixed Chrome Boot sample complete: ${resultsDirectory}\n`);

