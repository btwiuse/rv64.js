#!/usr/bin/env node

// Boot the copy/v86 pane from the three-way WANIX page, verify that the
// architecture-matched namespace reached a shell, and run the shared workload.
// The WANIX site is intentionally external to this repository; point WANIX_URL
// at an installed page or use the local comparison server's default URL.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn, spawnSync } from "node:child_process";

const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const url = process.env.WANIX_URL ||
  "http://127.0.0.1:8765/examples/v86-rv64-three-way.html";
const vmId = process.env.WANIX_VM || "x86";
const jitPolicy = process.env.WANIX_JIT_POLICY;
const jitConfiguration = process.env.WANIX_JIT_CONFIG
  ? JSON.parse(process.env.WANIX_JIT_CONFIG)
  : {};
const prebootJitConfiguration = process.env.WANIX_JIT_PREBOOT === "1";
const jitProfile = process.env.WANIX_JIT_PROFILE === "1";
const summaryOnly = process.env.WANIX_SUMMARY_ONLY === "1";
const externalP9MetricsEnabled = process.env.WANIX_EXTERNAL_P9_METRICS !== "0";
const hostCpuAffinity = (() => {
  try {
    const status = readFileSync(`/proc/${process.pid}/status`, "utf8");
    return status.match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null;
  } catch {
    return null;
  }
})();
const requestedPhases = (process.env.WANIX_BENCH_PHASES || "python,sha256,shared9p")
  .split(",")
  .filter(Boolean);
const benchRepetitions = Number(process.env.WANIX_BENCH_REPETITIONS || 1);
const phaseSync = process.env.WANIX_BENCH_PHASE_SYNC === "1";
const phaseDefinitions = {
  python: {
    marker: "pure Python CPU",
    argument: "python",
    expected: "checksum=38460b78",
  },
  sha256: {
    marker: "SHA-256 (32 MiB)",
    argument: "sha256",
    expected: "e09320c5b00b34bb",
  },
  shared9p: { marker: "shared 9P I/O", argument: "shared9p" },
};
if (!new Set(["x86", "rv64-legacy", "rv64-jit"]).has(vmId)) {
  throw new Error("WANIX_VM must be x86, rv64-legacy, or rv64-jit");
}
if (jitPolicy !== undefined && !new Set(["page", "adaptive"]).has(jitPolicy)) {
  throw new Error("WANIX_JIT_POLICY must be page or adaptive");
}
if (requestedPhases.length === 0 || requestedPhases.some((phase) => !(phase in phaseDefinitions))) {
  throw new Error("WANIX_BENCH_PHASES must contain python, sha256, and/or shared9p");
}
if (!jitConfiguration || typeof jitConfiguration !== "object" || Array.isArray(jitConfiguration)) {
  throw new Error("WANIX_JIT_CONFIG must be a JSON object");
}
const prebootStaticT0Value = (() => {
  if (!prebootJitConfiguration) return null;
  if (vmId !== "rv64-jit") {
    throw new Error("WANIX_JIT_PREBOOT requires rv64-jit");
  }
  const keys = Object.keys(jitConfiguration).sort();
  if (JSON.stringify(keys) !== JSON.stringify([
    "sampledStaticT0", "sampledStaticT0Backoff", "staticSystemT0",
  ])) {
    throw new Error("pre-boot configuration must contain exactly the three static-T0 fields");
  }
  if (jitConfiguration.staticSystemT0 !== false) {
    throw new Error("pre-boot residual static T0 must be disabled");
  }
  if (jitConfiguration.sampledStaticT0 === false &&
      jitConfiguration.sampledStaticT0Backoff === false) {
    return "control";
  }
  if (jitConfiguration.sampledStaticT0 === true &&
      jitConfiguration.sampledStaticT0Backoff === true) {
    return "sampled-backoff";
  }
  throw new Error("invalid pre-boot sampled static-T0 configuration");
})();
if (!Number.isInteger(benchRepetitions) || benchRepetitions < 1 || benchRepetitions > 10) {
  throw new Error("WANIX_BENCH_REPETITIONS must be an integer from 1 through 10");
}
const terminalPath = `#vm/${vmId}/term`;
const statusSelector = `#${vmId}-status`;
const expectedMachine = vmId === "x86" ? "i686" : "riscv64";
const bootTimeoutMs = Number(process.env.WANIX_BOOT_TIMEOUT_MS ||
  (vmId === "x86" ? 180_000 : 600_000));
const benchTimeoutMs = Number(process.env.WANIX_BENCH_TIMEOUT_MS || 600_000);

// Benchmarking two browser VMs concurrently makes both samples meaningless.
// This harness is often driven by independent shells, so guard the whole
// browser lifetime with a host-wide atomic directory lock. A dead owner's
// lock is reclaimed; WANIX_ALLOW_PARALLEL=1 is available for non-timing use.
const benchmarkLockPath = join(tmpdir(), "rv64-wanix-matched-smoke.lock");
let benchmarkLockHeld = false;

function processIsAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code !== "ESRCH";
  }
}

function acquireBenchmarkLock() {
  if (process.env.WANIX_ALLOW_PARALLEL === "1") return;
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      mkdirSync(benchmarkLockPath);
      writeFileSync(join(benchmarkLockPath, "pid"), `${process.pid}\n`);
      benchmarkLockHeld = true;
      return;
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      let owner = 0;
      try {
        owner = Number(readFileSync(join(benchmarkLockPath, "pid"), "utf8").trim());
      } catch {}
      if (Number.isSafeInteger(owner) && owner > 0 && processIsAlive(owner)) {
        throw new Error(`another WANIX benchmark is running (pid ${owner})`);
      }
      rmSync(benchmarkLockPath, { recursive: true, force: true });
    }
  }
  throw new Error("could not acquire the WANIX benchmark lock");
}

function releaseBenchmarkLock() {
  if (!benchmarkLockHeld) return;
  rmSync(benchmarkLockPath, { recursive: true, force: true });
  benchmarkLockHeld = false;
}

acquireBenchmarkLock();
process.on("exit", releaseBenchmarkLock);

const startedAt = new Date().toISOString();

async function fetchedDigest(resourceUrl) {
  const response = await fetch(resourceUrl, {
    cache: "no-store",
    headers: {
      "cache-control": "no-cache",
      pragma: "no-cache",
    },
  });
  if (!response.ok) {
    throw new Error(`artifact fetch failed (${response.status}) for ${resourceUrl}`);
  }
  const hash = createHash("sha256");
  let bytes = 0;
  for await (const chunk of response.body) {
    hash.update(chunk);
    bytes += chunk.byteLength;
  }
  return {
    url: response.url,
    bytes,
    sha256: hash.digest("hex"),
  };
}

function attribute(tag, name) {
  const match = tag.match(new RegExp(`\\b${name}\\s*=\\s*(["'])(.*?)\\1`, "i"));
  return match?.[2] ?? null;
}

async function artifactSnapshot() {
  const response = await fetch(url, {
    cache: "no-store",
    headers: {
      "cache-control": "no-cache",
      pragma: "no-cache",
    },
  });
  if (!response.ok) {
    throw new Error(`comparison page fetch failed (${response.status}) for ${url}`);
  }
  const htmlBytes = Buffer.from(await response.arrayBuffer());
  const html = htmlBytes.toString("utf8");
  const page = {
    url: response.url,
    bytes: htmlBytes.byteLength,
    sha256: createHash("sha256").update(htmlBytes).digest("hex"),
  };
  const tags = [...html.matchAll(/<wanix-bind\b[^>]*>/gi)].map((match) => match[0]);
  const sourceForDestination = (destination) => {
    const tag = tags.find((candidate) => attribute(candidate, "dst") === destination);
    const source = tag && attribute(tag, "src");
    if (!source) throw new Error(`comparison page has no ${destination} archive binding`);
    return source;
  };
  const sourceEndingWith = (suffix) => {
    const sources = tags.map((tag) => attribute(tag, "src")).filter(Boolean);
    const source = sources.find((candidate) => new URL(candidate, response.url).pathname.endsWith(suffix));
    if (!source) throw new Error(`comparison page has no archive ending in ${suffix}`);
    return source;
  };
  const sources = {
    rv64Jit: sourceForDestination("#vm/rv64-jit"),
    v86: sourceForDestination("#vm/v86"),
    rv64Root: sourceEndingWith("/wanix-linux-rv64.tgz"),
    x86Root: sourceEndingWith("/wanix-linux-x86.tgz"),
  };
  const archives = Object.fromEntries(await Promise.all(Object.entries(sources).map(
    async ([name, source]) => [name, {
      source,
      ...await fetchedDigest(new URL(source, response.url).href),
    }],
  )));
  return { page, archives };
}

// These reads happen outside the measured guest interval. They make every log
// self-identifying and catch a deployment being replaced during a run. Both
// candidates and both matched roots are covered, including when this process
// starts only one of the two browser VMs.
const artifactsBefore = await artifactSnapshot();

function executable(candidates) {
  for (const candidate of candidates) {
    const found = spawnSync("sh", ["-c", `command -v -- "$1"`, "sh", candidate], {
      encoding: "utf8",
    });
    if (found.status === 0 && found.stdout.trim()) return found.stdout.trim();
  }
  return null;
}

const chrome = process.env.CHROME || executable([
  "google-chrome",
  "chromium",
  "chromium-browser",
]);
if (!chrome) throw new Error("Chrome/Chromium executable not found");

class CdpConnection {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    this.events = [];
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (!message.id) {
        this.events.push(message);
        return;
      }
      if (!this.pending.has(message.id)) return;
      const pending = this.pending.get(message.id);
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(JSON.stringify(message.error)));
      else pending.resolve(message.result);
    });
  }

  static async open(socketUrl) {
    const socket = new WebSocket(socketUrl);
    await new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", reject, { once: true });
    });
    return new CdpConnection(socket);
  }

  send(method, params = {}, sessionId = undefined) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }

  close() {
    this.socket.close();
  }
}

async function waitForProtocol(child, timeoutMs = 20_000) {
  let log = "";
  let resolveUrl;
  let rejectUrl;
  const found = new Promise((resolve, reject) => {
    resolveUrl = resolve;
    rejectUrl = reject;
  });
  const consume = (chunk) => {
    log = (log + chunk.toString()).slice(-64 * 1024);
    const match = log.match(/DevTools listening on (ws:\/\/\S+)/);
    if (match) resolveUrl(match[1]);
  };
  child.stdout.on("data", consume);
  child.stderr.on("data", consume);
  child.once("exit", (code, signal) => {
    rejectUrl(new Error(`browser exited (${code ?? signal}) before DevTools:\n${log}`));
  });
  const timer = setTimeout(() => rejectUrl(new Error(`DevTools timeout:\n${log}`)), timeoutMs);
  try {
    return await found;
  } finally {
    clearTimeout(timer);
  }
}

async function pageTarget(browserUrl, expected) {
  const endpoint = new URL(browserUrl);
  // WANIX instantiates its own 20 MiB Wasm kernel before registering elements;
  // a cold browser can legitimately stay on the initial target document for
  // longer than a normal static page load.
  for (let attempt = 0; attempt < 2_400; attempt++) {
    try {
      const targets = await (await fetch(`http://${endpoint.host}/json/list`)).json();
      const target = targets.find((candidate) =>
        candidate.type === "page" && candidate.url.startsWith(expected));
      if (target?.webSocketDebuggerUrl) return target.webSocketDebuggerUrl;
    } catch {
      // Chrome's target endpoint can trail the DevTools announcement briefly.
    }
    await delay(25);
  }
  throw new Error("WANIX page target did not appear");
}

async function terminate(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    delay(500),
  ]);
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
}

async function evaluate(cdp, expression) {
  const response = await cdp.send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (response.exceptionDetails) {
    throw new Error(response.exceptionDetails.exception?.description ||
      response.exceptionDetails.text || "browser evaluation failed");
  }
  return response.result?.value;
}

function missingDefaultExecutionContext(error) {
  return error instanceof Error &&
    error.message.includes("Cannot find default execution context");
}

async function evaluateSession(cdp, sessionId, expression) {
  const response = await cdp.send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  }, sessionId);
  if (response.exceptionDetails) {
    throw new Error(response.exceptionDetails.exception?.description ||
      response.exceptionDetails.text || "Worker evaluation failed");
  }
  return response.result?.value;
}

async function findBenchmarkWorker(cdp, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const sessions = [...new Set(cdp.events
      .filter((event) => event.method === "Target.attachedToTarget" &&
        event.params?.targetInfo?.type === "worker")
      .map((event) => event.params.sessionId))];
    for (const session of sessions) {
      try {
        if (await evaluateSession(cdp, session,
          "Boolean(globalThis.__rv64BenchmarkVM?.jitStats)")) return session;
      } catch {
        // A short-lived worker may disappear while candidates are inspected.
      }
    }
    await delay(50);
  }
  throw new Error("rv64 benchmark Worker did not expose JIT counters");
}

async function findP9Worker(cdp, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const sessions = [...new Set(cdp.events
      .filter((event) => event.method === "Target.attachedToTarget" &&
        event.params?.targetInfo?.type === "worker")
      .map((event) => event.params.sessionId))];
    for (const session of sessions) {
      try {
        if (await evaluateSession(cdp, session,
          "Boolean(globalThis.worker?.p9?.postMessage)")) return session;
      } catch {
        // A short-lived worker may disappear while candidates are inspected.
      }
    }
    await delay(50);
  }
  throw new Error("VM worker did not expose its external 9P port");
}

async function installExternalP9Metrics(cdp, sessionId) {
  return evaluateSession(cdp, sessionId, `(() => {
    if (globalThis.__wanixP9Metrics) return true;
    const port = globalThis.worker?.p9;
    if (!port?.postMessage) return false;
    const metrics = {
      requests: 0, replies: 0, requestBytes: 0, replyBytes: 0,
      reads: 0, writes: 0, readBytes: 0, writeBytes: 0,
      maximumRead: 0, maximumWrite: 0, pending: 0, maximumPending: 0,
      tagCollisions: 0, hostMs: 0,
    };
    const started = new Map();
    const tagOf = (bytes) => bytes[5] | (bytes[6] << 8);
    const original = port.postMessage.bind(port);
    port.postMessage = (message, ...rest) => {
      const bytes = message instanceof Uint8Array
        ? message
        : new Uint8Array(message?.buffer ?? message);
      metrics.requests++;
      metrics.requestBytes += bytes.byteLength;
      const tag = tagOf(bytes);
      if (started.has(tag)) metrics.tagCollisions++;
      started.set(tag, {
        at: performance.now(),
        type: bytes[4],
        size: bytes.byteLength,
      });
      metrics.pending++;
      metrics.maximumPending = Math.max(metrics.maximumPending, metrics.pending);
      if (bytes.byteLength >= 23 && (bytes[4] === 116 || bytes[4] === 118)) {
        const count = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
          .getUint32(19, true);
        if (bytes[4] === 116) {
          metrics.reads++;
          metrics.readBytes += count;
          metrics.maximumRead = Math.max(metrics.maximumRead, count);
        } else {
          metrics.writes++;
          metrics.writeBytes += count;
          metrics.maximumWrite = Math.max(metrics.maximumWrite, count);
        }
      }
      return original(message, ...rest);
    };
    port.addEventListener("message", (event) => {
      const bytes = event.data instanceof Uint8Array
        ? event.data
        : new Uint8Array(event.data?.buffer ?? event.data);
      metrics.replies++;
      metrics.replyBytes += bytes.byteLength;
      const tag = tagOf(bytes);
      const began = started.get(tag);
      if (began !== undefined) {
        metrics.hostMs += performance.now() - began.at;
        started.delete(tag);
        metrics.pending--;
      }
    });
    globalThis.__wanixP9Metrics = metrics;
    globalThis.__wanixP9Started = started;
    return true;
  })()`);
}

async function readExternalP9Metrics(cdp, sessionId) {
  return evaluateSession(cdp, sessionId,
    "structuredClone(globalThis.__wanixP9Metrics)");
}

async function readExternalP9Diagnostic(cdp, sessionId) {
  return evaluateSession(cdp, sessionId, `(() => ({
    metrics: structuredClone(globalThis.__wanixP9Metrics),
    pendingRequests: [...(globalThis.__wanixP9Started ?? [])].map(([tag, request]) => ({
      tag,
      type: request.type,
      size: request.size,
      ageMs: performance.now() - request.at,
    })),
  }))()`);
}

function numericDelta(before, after) {
  return Object.fromEntries(Object.keys(after).map((key) => [key, after[key] - before[key]]));
}

function externalP9Delta(before, after) {
  const delta = numericDelta(before, after);
  delta.pending = after.pending;
  delta.maximumPending = after.maximumPending;
  delta.maximumRead = after.maximumRead;
  delta.maximumWrite = after.maximumWrite;
  return delta;
}

async function readJitStats(cdp, sessionId) {
  return evaluateSession(cdp, sessionId, "globalThis.__rv64BenchmarkVM.jitStats()");
}

function jitDelta(before, after) {
  const delta = (path) => {
    let a = before;
    let b = after;
    for (const part of path.split(".")) {
      a = a[part];
      b = b[part];
    }
    return BigInt(b) - BigInt(a);
  };
  const instructions = delta("instructions");
  const generated = delta("generated.retired");
  const dispatches = delta("generated.dispatches");
  return {
    instructions: instructions.toString(),
    generatedRetired: generated.toString(),
    interpreterRetired: delta("interpreter.retired").toString(),
    interpreterCalls: delta("interpreter.calls").toString(),
    generatedDispatches: dispatches.toString(),
    staticSystemT0Enabled: after.staticT0.systemEnabled,
    sampledStaticT0Enabled: after.staticT0.sampledEnabled,
    sampledStaticT0BackoffEnabled: after.staticT0.sampledBackoffEnabled,
    staticSystemT0ModuleIndex: after.staticT0.moduleIndex,
    staticT0FastRetired: delta("staticT0.systemFastRetired").toString(),
    staticT0SlowRetired: delta("staticT0.systemSlowRetired").toString(),
    staticT0SlowBatches: delta("staticT0.systemSlowBatches").toString(),
    staticT0FetchFills: delta("staticT0.systemFetchFills").toString(),
    staticT0Errors: delta("staticT0.systemErrors").toString(),
    sampledStaticT0Retired: delta("staticT0.sampledRetired").toString(),
    sampledStaticT0Samples: delta("staticT0.samples").toString(),
    sampledStaticT0InterruptPolls: delta("staticT0.interruptPolls").toString(),
    sampledStaticT0ShortMarks: delta("staticT0.shortSampleMarks").toString(),
    sampledStaticT0ShortBypasses: delta("staticT0.shortSampleBypasses").toString(),
    sampledStaticT0ShortClears: delta("staticT0.shortSampleClears").toString(),
    zeroRetireDispatches: delta("generated.zeroRetireDispatches").toString(),
    zeroRetireSuppressions: delta("generated.zeroRetireSuppressions").toString(),
    dispatchEmptyMisses: delta("generated.dispatchEmptyMisses").toString(),
    dispatchTagCollisions: delta("generated.dispatchTagCollisions").toString(),
    dispatchReverifications: delta("generated.dispatchReverifications").toString(),
    zeroRetireTracked: delta("generated.zeroRetireTracked").toString(),
    zeroRetireUntracked: delta("generated.zeroRetireUntracked").toString(),
    zeroRetireProfileResets: delta("generated.zeroRetireProfileResets").toString(),
    zeroRetireProfiles: after.generated.zeroRetireProfiles,
    zeroRetireSuppressedEntries: after.generated.zeroRetireSuppressedEntries,
    tlbFills: delta("generated.tlbFills").toString(),
    tlbFillKinds: Object.fromEntries([
      "loadHit", "loadEmpty", "loadContext", "loadCollision",
      "storeHit", "storeEmpty", "storeContext", "storeCollision",
    ].map((kind) => [kind, delta(`generated.tlbFillKinds.${kind}`).toString()])),
    chainHops: delta("generated.chainHops").toString(),
    mappingInvalidations: delta("mmu.mappingInvalidations").toString(),
    changedSatp: delta("mmu.changedSatp").toString(),
    fullTlbClears: delta("mmu.fullTlbClears").toString(),
    storeJitTlbClears: delta("mmu.storeJitTlbClears").toString(),
    sfenceAll: delta("mmu.sfenceAll").toString(),
    sfencePage: delta("mmu.sfencePage").toString(),
    sfenceForeignAsid: delta("mmu.sfenceForeignAsid").toString(),
    generatedCoverage: instructions === 0n
      ? 0
      : Number(generated * 1_000_000n / instructions) / 1_000_000,
    generatedInstructionsPerDispatch: dispatches === 0n
      ? 0
      : Number(generated) / Number(dispatches),
    pagePolicyEnabled: after.pagePolicy.enabled,
    pageThreshold: after.pagePolicy.threshold,
    privilegedPageThresholdMultiplier: after.pagePolicy.privilegedThresholdMultiplier,
    pageQuantum: after.pagePolicy.quantum,
    controlEntriesEnabled: after.pagePolicy.controlEntriesEnabled,
    privilegedControlEntriesEnabled: after.pagePolicy.privilegedControlEntriesEnabled,
    stablePageChainEnabled: after.pagePolicy.stableChainEnabled,
    pageInflightLimit: after.pagePolicy.inflightLimit,
    pagePolicyUserRetired: delta("pagePolicy.userRetired").toString(),
    pagePolicyPrivilegedRetired: delta("pagePolicy.privilegedRetired").toString(),
    pagePolicyUserCandidates: delta("pagePolicy.userCandidates").toString(),
    pagePolicyPrivilegedCandidates: delta("pagePolicy.privilegedCandidates").toString(),
    candidates: delta("pagePolicy.candidates").toString(),
    issued: delta("pagePolicy.issued").toString(),
    landed: delta("pagePolicy.landed").toString(),
    issuedPages: delta("pagePolicy.issuedPages").toString(),
    multiPageIssued: delta("pagePolicy.multiPageIssued").toString(),
    multiPageEntryEligible: delta("pagePolicy.multiPageEntryEligible").toString(),
    multiPageEntryBlocked: delta("pagePolicy.multiPageEntryBlocked").toString(),
    multiPageEntryCap: after.pagePolicy.multiPageEntryCap,
    multiPageControlPermille: after.pagePolicy.multiPageControlPermille,
    multiPageControlEligible: delta("pagePolicy.multiPageControlEligible").toString(),
    multiPageControlBlocked: delta("pagePolicy.multiPageControlBlocked").toString(),
    controlProfileEnabled: after.pagePolicy.controlProfileEnabled,
    regionPageCap: after.pagePolicy.regionPageCap,
    regionLeaderCap: after.pagePolicy.regionLeaderCap,
    regionTailChainEnabled: after.pagePolicy.regionTailChainEnabled,
    regionTlbCacheEnabled: after.pagePolicy.regionTlbCacheEnabled,
    regionTlbCacheMinAccesses: after.pagePolicy.regionTlbCacheMinAccesses,
    regionExtensions: delta("regions.extensions").toString(),
    regionExitSamples: delta("regions.exitSamples").toString(),
    regionExtensionQueued: delta("regions.extensionQueued").toString(),
    regionExtensionDrainVisits: delta("regions.extensionDrainVisits").toString(),
    regionDemoted: delta("regions.demoted").toString(),
    regionExtensionShortBlocked: delta("pagePolicy.extensionShortBlocked").toString(),
    translateAttempts: delta("translation.systemAttempts").toString(),
    emittedBytes: delta("translation.systemEmittedBytes").toString(),
    modules: after.loader.modules - before.loader.modules,
    compileMs: after.loader.compileMs - before.loader.compileMs,
    p9Requests: after.p9.requests - before.p9.requests,
    p9MaximumPending: after.p9.maximumPending,
    p9HostMs: after.p9.hostMs - before.p9.hostMs,
    p9RequestBytes: after.p9.requestBytes - before.p9.requestBytes,
    p9ReplyBytes: after.p9.replyBytes - before.p9.replyBytes,
    p9ReadRequests: after.p9.readRequests - before.p9.readRequests,
    p9WriteRequests: after.p9.writeRequests - before.p9.writeRequests,
    p9ReadBytes: after.p9.readBytes - before.p9.readBytes,
    p9WriteBytes: after.p9.writeBytes - before.p9.writeBytes,
    p9MaximumRead: after.p9.maximumRead,
    p9MaximumWrite: after.p9.maximumWrite,
  };
}

const terminalExpression = `(() => {
  const element = [...document.querySelectorAll("wanix-term")]
    .find((candidate) => candidate.getAttribute("path") === ${JSON.stringify(terminalPath)});
  const buffer = element?._term?.buffer?.active;
  if (!buffer) return "";
  const lines = [];
  for (let index = 0; index < buffer.length; index++) {
    const line = buffer.getLine(index);
    const text = line?.translateToString(true) ?? "";
    // xterm stores visual wraps as separate buffer rows. Rejoin them so
    // benchmark values remain parseable in the narrow comparison page.
    if (line?.isWrapped && lines.length) lines[lines.length - 1] += text;
    else lines.push(text);
  }
  return lines.join("\\n");
})()`;

function benchmarkMatches(terminal, phase) {
  const definition = phaseDefinitions[phase];
  const marker = definition.marker.replace(/[.*+?^$(){}|[\]\\]/g, "\\$&");
  const pattern = new RegExp(
    marker + "\\s+([0-9]+(?:\\.[0-9]+)?)\\s+s",
    "g",
  );
  return [...terminal.matchAll(pattern)];
}

function latestBenchmarkSample(terminal, phase) {
  const definition = phaseDefinitions[phase];
  const matches = benchmarkMatches(terminal, phase);
  if (!matches.length) {
    throw new Error(
      `completed ${phase} phase has no parseable timing\n` +
        terminal.split("\n").slice(-80).join("\n"),
    );
  }
  const line = terminal.split("\n").filter((entry) => entry.includes(definition.marker)).at(-1);
  if (definition.expected && !line?.includes(definition.expected)) {
    throw new Error(
      `${phase} result failed its correctness marker: ${JSON.stringify(line)}`,
    );
  }
  return Number(matches.at(-1)[1]);
}

async function waitForText(cdp, marker, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let terminal = "";
  while (Date.now() < deadline) {
    terminal = await evaluate(cdp, terminalExpression);
    if (terminal.includes(marker)) return terminal;
    const state = await evaluate(cdp, `(() => {
      const status = document.querySelector(${JSON.stringify(statusSelector)});
      return { state: status?.dataset.state ?? "", text: status?.textContent ?? "" };
    })()`);
    if (state.state === "error") {
      const events = cdp.events.filter((event) =>
        event.method === "Runtime.exceptionThrown" ||
        event.method === "Log.entryAdded" ||
        event.method === "Network.loadingFailed");
      throw new Error(`${vmId} VM start failed: ${state.text}; ` +
        `browser events: ${JSON.stringify(events.slice(-20), null, 2)}; ` +
        `terminal tail:\n${terminal.split("\n").slice(-40).join("\n")}`);
    }
    await delay(250);
  }
  const state = await evaluate(cdp, `(() => {
    const status = document.querySelector(${JSON.stringify(statusSelector)});
    return {
      href: location.href,
      state: status?.dataset.state ?? "",
      status: status?.textContent ?? "",
    };
  })()`);
  const events = cdp.events.filter((event) =>
    event.method === "Runtime.exceptionThrown" ||
    event.method === "Log.entryAdded" ||
    event.method === "Network.loadingFailed");
  throw new Error(`timed out waiting for ${JSON.stringify(marker)}; ` +
    `page state: ${JSON.stringify(state)}; browser events: ` +
    `${JSON.stringify(events.slice(-20), null, 2)}; terminal tail:\n` +
    terminal.split("\n").slice(-40).join("\n"));
}

async function waitForTextCount(cdp, marker, count, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let terminal = "";
  while (Date.now() < deadline) {
    terminal = await evaluate(cdp, terminalExpression);
    if (terminal.split(marker).length - 1 >= count) return terminal;
    await delay(250);
  }
  throw new Error(`timed out waiting for occurrence ${count} of ${JSON.stringify(marker)}; ` +
    `terminal tail:\n${terminal.split("\n").slice(-60).join("\n")}`);
}

// Serial output can split one formatted benchmark line at any byte. Waiting
// merely for its human-readable prefix races with the numeric field (and with
// the correctness suffix). Count only complete, parseable records so phase
// snapshots and timings always describe the same finished guest operation.
async function waitForBenchmarkCount(cdp, phase, count, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  const definition = phaseDefinitions[phase];
  let terminal = "";
  while (Date.now() < deadline) {
    terminal = await evaluate(cdp, terminalExpression);
    const complete = benchmarkMatches(terminal, phase).length;
    const correct = definition.expected
      ? terminal.split("\n").filter((line) =>
        line.includes(definition.marker) && line.includes(definition.expected)).length
      : complete;
    if (complete >= count && correct >= count) return terminal;
    await delay(250);
  }
  throw new Error(`timed out waiting for complete ${phase} benchmark ${count}; ` +
    `terminal tail:\n${terminal.split("\n").slice(-60).join("\n")}`);
}

async function send(cdp, command) {
  await evaluate(cdp, `(() => {
    const element = [...document.querySelectorAll("wanix-term")]
      .find((candidate) => candidate.getAttribute("path") === ${JSON.stringify(terminalPath)});
    if (!element?._term) throw new Error(${JSON.stringify(`${vmId} terminal is unavailable`)});
    element._term.input(${JSON.stringify(`${command}\r`)}, true);
    return true;
  })()`);
}

const profile = mkdtempSync(join(tmpdir(), `wanix-${vmId}-smoke-chrome-`));
const debugPort = process.env.CHROME_DEBUG_PORT || "0";
const child = spawn(chrome, [
  "--headless=new",
  "--no-sandbox",
  "--disable-gpu",
  "--disable-background-networking",
  "--disable-background-timer-throttling",
  "--disable-backgrounding-occluded-windows",
  "--disable-component-update",
  "--disable-default-apps",
  "--disable-extensions",
  "--disable-renderer-backgrounding",
  "--disable-sync",
  "--disable-translate",
  "--metrics-recording-only",
  "--no-first-run",
  `--remote-debugging-port=${debugPort}`,
  `--user-data-dir=${profile}`,
  url,
], { stdio: ["ignore", "pipe", "pipe"] });

let pageCdp;
let browserCdp;
let browserVersion;
try {
  const bootStart = performance.now();
  const browserUrl = await waitForProtocol(child);
  browserCdp = await CdpConnection.open(browserUrl);
  browserVersion = await browserCdp.send("Browser.getVersion");
  pageCdp = await CdpConnection.open(await pageTarget(browserUrl, url));
  await pageCdp.send("Runtime.enable");
  await pageCdp.send("Log.enable");
  await pageCdp.send("Network.enable");
  // Dedicated Workers are child targets of the page. Flattened auto-attach
  // lets the benchmark read the explicitly exposed RV64 counters without
  // heap scanning or debugger-side object discovery.
  await pageCdp.send("Target.setAutoAttach", {
    autoAttach: true,
    waitForDebuggerOnStart: false,
    flatten: true,
  });

  // WANIX instantiates its own 20 MiB Wasm kernel before registering elements;
  // a cold browser can legitimately stay on the initial target document for
  // longer than a normal static page load. Check and click atomically because
  // the initial about:blank execution context is replaced during navigation.
  let missingExecutionContextRetries = 0;
  for (let attempt = 0; attempt < 2_400; attempt++) {
    let started = false;
    try {
      started = await evaluate(pageCdp, `(() => {
      const button = document.querySelector(${JSON.stringify(`button[data-vm="${vmId}"]`)});
      if (!button || !customElements.get("wanix-vm")) return false;
      const vm = document.querySelector(${JSON.stringify(`wanix-vm#${vmId}`)});
      if (!vm) return false;
      const prebootValue = ${JSON.stringify(prebootStaticT0Value)};
      if (prebootValue !== null) {
        const append = vm.getAttribute("append") ?? "";
        const expected = "rv64.static-t0=" + prebootValue;
        const values = append.split(/\\s+/).filter((value) =>
          value.startsWith("rv64.static-t0="));
        if (values.length !== 1 || values[0] !== expected) {
          throw new Error("comparison page does not select " + expected);
        }
      }
      button.click();
      return true;
      })()`);
    } catch (error) {
      // `/json/list` can expose the navigated page target a few milliseconds
      // before Chrome creates its default JavaScript world. This is a browser
      // attachment race before any guest or benchmark work exists; retry only
      // that exact CDP condition. Navigation, renderer loss, guest failures,
      // and measured-phase evaluation errors still fail immediately.
      if (!missingDefaultExecutionContext(error)) throw error;
      missingExecutionContextRetries++;
    }
    if (started) break;
    if (attempt === 2_399) {
      const state = await evaluate(pageCdp, `(() => ({
        href: location.href,
        readyState: document.readyState,
        button: Boolean(document.querySelector(${JSON.stringify(`button[data-vm="${vmId}"]`)})),
        wanixVm: Boolean(customElements.get("wanix-vm")),
        scripts: [...document.scripts].map((script) => script.src),
        html: document.documentElement.outerHTML.slice(0, 500),
      }))()`);
      const events = pageCdp.events.filter((event) =>
        event.method === "Runtime.exceptionThrown" ||
        event.method === "Log.entryAdded" ||
        event.method === "Network.loadingFailed");
      throw new Error(`WANIX controls did not initialize: ${JSON.stringify(state)}\n` +
        JSON.stringify(events.slice(-20), null, 2));
    }
    await delay(50);
  }
  await waitForText(pageCdp, "WANIX shell ready", bootTimeoutMs);
  const shellMs = performance.now() - bootStart;

  // Keep commands shorter than the narrow three-column terminal. Waiting on
  // command output (rather than an echoed marker) also proves each process ran.
  await send(pageCdp, "uname -m");
  await waitForText(pageCdp, expectedMachine, 30_000);
  await send(pageCdp, "cat /etc/alpine-release");
  await waitForText(pageCdp, "3.22.5", 30_000);
  await send(pageCdp, "python3 -V");
  let terminal = await waitForText(pageCdp, "Python 3.12.13", 30_000);

  const jitWorker = vmId === "rv64-jit" ? await findBenchmarkWorker(pageCdp) : null;
  const p9Worker = vmId === "x86" && externalP9MetricsEnabled
    ? await findP9Worker(pageCdp)
    : null;
  if (p9Worker && !(await installExternalP9Metrics(pageCdp, p9Worker))) {
    throw new Error("failed to install external 9P metrics in the v86 worker");
  }
  if (jitWorker && !prebootJitConfiguration &&
      (jitPolicy || jitProfile || Object.keys(jitConfiguration).length)) {
    const configuration = {
      ...jitConfiguration,
      ...(jitPolicy ? { policy: jitPolicy } : {}),
      ...(jitProfile ? { profile: true, profileSampleShift: 6 } : {}),
    };
    await evaluateSession(pageCdp, jitWorker,
      `globalThis.__rv64BenchmarkVM.configureJit(${JSON.stringify(configuration)})`);
  }
  const jitAtShell = jitWorker ? await readJitStats(pageCdp, jitWorker) : null;
  const jitSnapshots = jitWorker ? { before: jitAtShell } : null;
  const externalP9Snapshots = p9Worker
    ? { before: await readExternalP9Metrics(pageCdp, p9Worker) }
    : null;

  const benchStart = performance.now();
  const phaseArguments = requestedPhases.map((phase) => phaseDefinitions[phase].argument).join(" ");
  const phaseIntervals = [];
  const samples = Object.fromEntries(requestedPhases.map((phase) => [phase, []]));
  let previousSnapshotKey = "before";
  for (let repetition = 0; repetition < benchRepetitions; repetition++) {
    const doneMarker = `__RV64_BENCH_DONE_${repetition + 1}__`;
    const phaseEnvironment = phaseSync ? "WANIX_BENCH_PHASE_SYNC=1 " : "";
    await send(pageCdp,
      `${phaseEnvironment}python3 /shared/bench.py ${phaseArguments};echo ${doneMarker}`);
    for (const phase of requestedPhases) {
      let startKey = previousSnapshotKey;
      if (phaseSync) {
        const readyMarker = `__WANIX_PHASE_READY_${phase}__`;
        terminal = await waitForTextCount(pageCdp, readyMarker, repetition + 1, benchTimeoutMs);
        startKey = `${phase}${repetition + 1}Start`;
        if (jitSnapshots) jitSnapshots[startKey] = await readJitStats(pageCdp, jitWorker);
        if (externalP9Snapshots) {
          externalP9Snapshots[startKey] = await readExternalP9Metrics(pageCdp, p9Worker);
        }
        await send(pageCdp, "");
      }
      try {
        terminal = await waitForBenchmarkCount(
          pageCdp, phase, repetition + 1, benchTimeoutMs,
        );
      } catch (error) {
        if (p9Worker) {
          const p9Diagnostic = await readExternalP9Diagnostic(pageCdp, p9Worker);
          error.message += `\nexternal 9P diagnostic: ${JSON.stringify(p9Diagnostic)}`;
        }
        throw error;
      }
      samples[phase].push(latestBenchmarkSample(terminal, phase));
      const key = benchRepetitions === 1 ? phase : `${phase}${repetition + 1}`;
      const endKey = `${key}End`;
      if (jitSnapshots) jitSnapshots[endKey] = await readJitStats(pageCdp, jitWorker);
      if (externalP9Snapshots) {
        externalP9Snapshots[endKey] = await readExternalP9Metrics(pageCdp, p9Worker);
      }
      phaseIntervals.push({ key, startKey, endKey });
      previousSnapshotKey = endKey;
    }
    terminal = await waitForText(pageCdp, doneMarker, benchTimeoutMs);
  }
  const benchMs = performance.now() - benchStart;
  if (requestedPhases.some((phase) => samples[phase].length !== benchRepetitions)) {
    throw new Error(`benchmark sample capture is incomplete: ${JSON.stringify(samples)}`);
  }
  const artifactsAfter = await artifactSnapshot();
  assert.deepEqual(
    artifactsAfter,
    artifactsBefore,
    "comparison page or benchmark artifact changed while the sample was running",
  );
  const finishedAt = new Date().toISOString();
  const correctness = Object.fromEntries(requestedPhases.map((phase) => {
    const definition = phaseDefinitions[phase];
    const complete = benchmarkMatches(terminal, phase).length;
    const correct = definition.expected
      ? terminal.split("\n").filter((line) =>
        line.includes(definition.marker) && line.includes(definition.expected)).length
      : complete;
    return [phase, { complete, correct }];
  }));
  console.log(JSON.stringify({
    vm: vmId,
    startedAt,
    finishedAt,
    hostCpuAffinity,
    browser: browserVersion,
    browserStartup: { missingExecutionContextRetries },
    artifactStable: true,
    artifacts: artifactsBefore,
    guest: {
      machine: expectedMachine,
      alpineRelease: "3.22.5",
      pythonVersion: "3.12.13",
    },
    phases: requestedPhases,
    repetitions: benchRepetitions,
    phaseSync,
    jitPolicy: jitPolicy ?? null,
    jitConfiguration,
    jitConfigurationLifecycle: prebootJitConfiguration ? "preboot" : "shell",
    shellMs,
    benchMs,
    samples,
    correctness,
    ...(externalP9Snapshots ? {
      externalP9: externalP9Delta(
        externalP9Snapshots.before,
        externalP9Snapshots[previousSnapshotKey],
      ),
      externalP9Phases: Object.fromEntries(phaseIntervals.map(({ key, startKey, endKey }) => [
        key,
        externalP9Delta(externalP9Snapshots[startKey], externalP9Snapshots[endKey]),
      ])),
    } : {}),
  }));
  if (jitSnapshots) {
    const jitPhases = {};
    for (const { key, startKey, endKey } of phaseIntervals) {
      jitPhases[key] = jitDelta(jitSnapshots[startKey], jitSnapshots[endKey]);
    }
    jitPhases.total = jitDelta(jitSnapshots.before, jitSnapshots[previousSnapshotKey]);
    console.log(JSON.stringify(summaryOnly
      ? { jitAtShell, jitPhases }
      : { jitAtShell, jitSnapshots, jitPhases }));
    if (jitProfile) {
      console.log(JSON.stringify({
        jitProfile: await evaluateSession(
          pageCdp,
          jitWorker,
          "globalThis.__rv64BenchmarkVM.jitProfile(30)",
        ),
      }));
    }
  }
  console.log(terminal.split("\n").slice(-35).join("\n"));
  const holdMs = Number(process.env.WANIX_HOLD_MS || 0);
  if (holdMs > 0) await delay(holdMs);
} finally {
  pageCdp?.close();
  if (browserCdp) {
    await Promise.race([browserCdp.send("Browser.close").catch(() => {}), delay(250)]);
    browserCdp.close();
  }
  await terminate(child);
  rmSync(profile, { recursive: true, force: true });
  releaseBenchmarkLock();
}
