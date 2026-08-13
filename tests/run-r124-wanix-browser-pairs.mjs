#!/usr/bin/env node

// R124's integration guard. Every leg gets a fresh Chrome process, profile,
// WANIX Worker, RV64 instance, and modern Linux guest. The two pages differ
// only in the immutable rv64-jit archive bound into #vm/rv64-jit.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  closeSync, existsSync, mkdirSync, openSync, readFileSync, rmSync, writeFileSync,
} from "node:fs";
import { execFileSync, spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

if (process.argv.length !== 3) {
  throw new Error("usage: node tests/run-r124-wanix-browser-pairs.mjs RESULTS_DIR");
}
const resultsDirectory = resolve(process.argv[2]);
assert.ok(!existsSync(resultsDirectory), "results directory already exists; use a new directory");
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const hostCpuAffinity = readFileSync(`/proc/${process.pid}/status`, "utf8")
  .match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null;
assert.equal(hostCpuAffinity, "8-15", "R124 formal runner must be pinned to CPUs 8-15");

const lockPath = resolve(tmpdir(), "rv64-r124-wanix-browser.lock");
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
      throw new Error(`another R124 WANIX benchmark is running (pid ${owner})`);
    }
    rmSync(lockPath, { recursive: true, force: true });
    if (attempt === 2) throw new Error("could not acquire the R124 WANIX lock");
  }
}
let lockHeld = true;
function releaseLock() {
  if (!lockHeld) return;
  rmSync(lockPath, { recursive: true, force: true });
  lockHeld = false;
}
process.on("exit", releaseLock);

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const digestFile = (path) => sha256(readFileSync(path));
async function fetchedIdentity(url) {
  const response = await fetch(url, {
    cache: "no-store",
    headers: { "cache-control": "no-cache", pragma: "no-cache" },
  });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  return { url: response.url, bytes: bytes.length, sha256: sha256(bytes) };
}
function archiveMemberIdentity(path, member) {
  const bytes = execFileSync("tar", ["-xOf", path, member], { maxBuffer: 16 * 1024 * 1024 });
  return { bytes: bytes.length, sha256: sha256(bytes) };
}

const pages = {
  control: {
    url: "http://127.0.0.1:8765/examples/" +
      "v86-rv64-three-way-r124-control.html",
    path: resolve(root,
      "integrations/wanix/v86-rv64-three-way-r124-control.html"),
    sha256: "ac3b9c63e67b0a46b95a0abe7b79b44868a0be4d23bdd51078ba024c805bf5da",
  },
  candidate: {
    url: "http://127.0.0.1:8765/examples/" +
      "v86-rv64-three-way-r124-candidate.html",
    path: resolve(root,
      "integrations/wanix/v86-rv64-three-way-r124-candidate.html"),
    sha256: "f5086330565fd781ab7f5929af9fad896b74d3cc9e39c65e9aaa5013b5bcf525",
  },
};
const archives = {
  control: {
    path: resolve(root, "target/bench/r124-rvc-bank-hybrid/wanix-assets/" +
      "rv64-jit-r124-control-9d0bf45cdbcffcc0.tgz"),
    url: "http://127.0.0.1:8765/rv64/rv64-jit-r124-control-9d0bf45cdbcffcc0.tgz",
    sha256: "9d0bf45cdbcffcc06f68ac48a5e5692e548c5c9a4b310a236dc4bcbb8086a98d",
    wasmSha256: "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
  },
  candidate: {
    path: resolve(root, "target/bench/r124-rvc-bank-hybrid/wanix-assets/" +
      "rv64-jit-r124-candidate-76c7139ba38c2f65.tgz"),
    url: "http://127.0.0.1:8765/rv64/rv64-jit-r124-candidate-76c7139ba38c2f65.tgz",
    sha256: "76c7139ba38c2f658d981ebd24bbeeb0308e1acf0a5b593a9b0784d32f9127d8",
    wasmSha256: "d017a10f00a8813df7c1af5750c8d4cb3a74b0dccea548eea31850a4ac56da59",
  },
};
for (const side of ["control", "candidate"]) {
  assert.equal(digestFile(pages[side].path), pages[side].sha256, `${side} page changed`);
  assert.equal(digestFile(archives[side].path), archives[side].sha256, `${side} archive changed`);
  assert.equal(
    archiveMemberIdentity(archives[side].path, "rv64_wasm.wasm").sha256,
    archives[side].wasmSha256,
    `${side} inner Wasm changed`,
  );
}
const deployed = {};
for (const side of ["control", "candidate"]) {
  deployed[side] = {
    page: await fetchedIdentity(pages[side].url),
    archive: await fetchedIdentity(archives[side].url),
  };
  assert.equal(deployed[side].page.sha256, pages[side].sha256, `${side} deployed page`);
  assert.equal(deployed[side].archive.sha256, archives[side].sha256, `${side} deployed archive`);
}

const pairs = 7;
const repetitions = 3;
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
  minimumPairedMedianSpeedup: 0.99,
  minimumPairedBootstrapUpper: 1.00,
  minimumPhaseBootstrapLower: 1 / 1.10,
  maximumWithinBrowserSpread: 1.25,
  minimumShared9pSampleSeconds: 2.0,
  exactP9WriteBytes: 33_554_432,
  minimumP9ReadBytes: 33_554_432,
  maximumP9TransferBytes: 4_096,
  minimumGeneratedCoverage: { python: 0.90, sha256: 0.90, shared9p: 0 },
};
const harness = resolve(root, "tests/wanix-v86-preboot-smoke.mjs");
const protocol = {
  schema: 1,
  experiment: "R124 fixed-bank immutable-artifact WANIX browser guard",
  plannedAt: new Date().toISOString(),
  pairs,
  repetitions,
  order,
  hostCpuAffinity,
  browserExecutable: process.env.CHROME ?? "/usr/bin/google-chrome",
  expectedBrowser,
  vm: "rv64-jit",
  phases: ["python", "sha256", "shared9p"],
  phaseSync: true,
  configurations: { control: {}, candidate: {} },
  pages,
  archives: Object.fromEntries(Object.entries(archives).map(([side, value]) => [side, {
    ...value,
    members: {
      rv64Wasm: archiveMemberIdentity(value.path, "rv64_wasm.wasm"),
      loader: archiveMemberIdentity(value.path, "rv64.js"),
      adapter: archiveMemberIdentity(value.path, "rv64-jit-vm.wasm"),
    },
  }])),
  deployed,
  commonArchiveSha256: {
    v86: "7b2c1986bed238ce1a7a7c23cb68b274dd2e09cea8dcc7b3aa5b56543b4ba771",
    rv64Root: "274a1e4766464cfeea9a5f7cce1d5f6569447e01df57bfc7a534bc8bb2aa06cb",
    x86Root: "09735e00b02b013964410e9f50a3536b8845fa7bfd31e739bb6b2c4df5e4b320",
  },
  integrationGuest: {
    linux: "6.12.7",
    alpine: "3.22.5",
    python: "3.12.13",
    arch: "riscv64",
    note: "public WANIX /shared/bench.py guard; authoritative scorecard guest remains Alpine 3.24.1",
  },
  thresholds,
  toolManifest: {
    runner: digestFile(fileURLToPath(import.meta.url)),
    analyzer: digestFile(resolve(root, "tests/analyze-r124-wanix-browser-pairs.mjs")),
    analyzerSelftest: digestFile(resolve(root, "tests/analyze-r124-wanix-selftest.mjs")),
    harness: digestFile(harness),
    selftest: digestFile(resolve(root, "tests/vs-v86/r124-wanix-harness-selftest.mjs")),
  },
};
assert.equal(protocol.archives.control.members.loader.sha256,
  protocol.archives.candidate.members.loader.sha256, "loader differs");
assert.equal(protocol.archives.control.members.adapter.sha256,
  protocol.archives.candidate.members.adapter.sha256, "adapter differs");
assert.notEqual(protocol.archives.control.members.rv64Wasm.sha256,
  protocol.archives.candidate.members.rv64Wasm.sha256, "inner Wasm must differ");
const normalizedControlPage = readFileSync(pages.control.path, "utf8").replace(
  "rv64-jit-r124-control-9d0bf45cdbcffcc0.tgz?v=9d0bf45cdbcffcc0",
  "RV64_ARCHIVE",
);
const normalizedCandidatePage = readFileSync(pages.candidate.path, "utf8").replace(
  "rv64-jit-r124-candidate-76c7139ba38c2f65.tgz?v=76c7139ba38c2f65",
  "RV64_ARCHIVE",
);
assert.equal(normalizedCandidatePage, normalizedControlPage,
  "pages differ beyond the immutable archive identity");
mkdirSync(resultsDirectory, { recursive: true });
writeFileSync(resolve(resultsDirectory, "protocol.json"), `${JSON.stringify(protocol, null, 2)}\n`, {
  flag: "wx",
});

let activeChild = null;
function runLeg(pair, side) {
  return new Promise((resolveRun, rejectRun) => {
    const output = openSync(resolve(resultsDirectory, `pair-${pair}-${side}.log`), "wx");
    const error = openSync(resolve(resultsDirectory, `pair-${pair}-${side}.stderr.log`), "wx");
    const environment = { ...process.env };
    for (const name of [
      "WANIX_JIT_CONFIG", "WANIX_JIT_POLICY", "WANIX_JIT_PROFILE", "WANIX_JIT_PREBOOT",
    ]) delete environment[name];
    Object.assign(environment, {
      CHROME: protocol.browserExecutable,
      WANIX_URL: pages[side].url,
      WANIX_VM: "rv64-jit",
      WANIX_JIT_CONFIG: "{}",
      WANIX_BENCH_PHASES: protocol.phases.join(","),
      WANIX_BENCH_REPETITIONS: String(repetitions),
      WANIX_BENCH_PHASE_SYNC: "1",
      WANIX_SUMMARY_ONLY: "1",
      WANIX_ALLOW_PARALLEL: "1",
    });
    const child = spawn(process.execPath, [harness], {
      env: environment,
      stdio: ["ignore", output, error],
    });
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
try {
  for (const { pair, sides } of order) {
    for (const side of sides) {
      process.stderr.write(`R124 WANIX pair ${pair}/${pairs}: ${side}\n`);
      await runLeg(pair, side);
    }
  }
} finally {
  releaseLock();
}
process.stderr.write(`R124 fixed WANIX sample complete: ${resultsDirectory}\n`);
