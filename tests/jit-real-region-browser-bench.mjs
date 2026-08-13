// Fresh-profile Chrome/Firefox frontend replay of exact multi-entry modules
// captured from real compiler-produced RV64 ELF code. No guest translation or
// generated execution occurs inside timed compile/instantiate spans.

import { createHash } from "node:crypto";
import { createServer } from "node:http";
import {
  mkdtempSync, readFileSync, rmSync, writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";
import { summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const pageBytes = readFileSync(join(here, "jit-real-region-browser.html"));
const corpusArg = process.argv.find((argument) => argument.startsWith("--corpus="));
const corpusDir = corpusArg
  ? corpusArg.split("=")[1]
  : join(root, "target/jit-real-region-corpus");

if (!process.argv.includes("--skip-generate")) {
  const generated = spawnSync(
    "cargo",
    [
      "run", "--release", "-q", "-p", "rv64-dbt", "--example",
      "emit_real_region_corpus", "--", corpusDir,
    ],
    { cwd: root, encoding: "utf8" },
  );
  if (generated.status !== 0) {
    throw new Error(generated.stderr || generated.stdout || "real-region generation failed");
  }
}

function manifestRows() {
  const lines = readFileSync(join(corpusDir, "manifest.tsv"), "utf8").trim().split("\n");
  const header = lines.shift().split("\t");
  return lines.map((line) => {
    const values = line.split("\t");
    const row = Object.fromEntries(header.map((name, index) => [name, values[index]]));
    for (const name of [
      "pages", "leader_cap", "entries", "read_x", "write_x", "read_f", "write_f",
      "bytes", "uses_fp",
    ]) row[name] = Number(row[name]);
    return row;
  });
}
const manifest = manifestRows();
const moduleFiles = new Set(manifest.map((record) => record.wasm));
const moduleMetadata = Object.fromEntries(manifest.map((record) => {
  const bytes = readFileSync(join(corpusDir, record.wasm));
  return [record.wasm, {
    bytes: bytes.length,
    hash: createHash("sha256").update(bytes).digest("hex"),
  }];
}));
const manifestJson = Buffer.from(JSON.stringify(manifest));

const server = createServer((request, response) => {
  const url = new URL(request.url, "http://127.0.0.1");
  response.setHeader("Cache-Control", "no-store");
  if (url.pathname === "/jit-real-region-browser.html") {
    response.setHeader("Content-Type", "text/html; charset=utf-8");
    response.end(pageBytes);
    return;
  }
  if (url.pathname === "/manifest.json") {
    response.setHeader("Content-Type", "application/json");
    response.end(manifestJson);
    return;
  }
  if (url.pathname.startsWith("/corpus/")) {
    const name = basename(url.pathname);
    if (moduleFiles.has(name) && url.pathname === `/corpus/${name}`) {
      response.setHeader("Content-Type", "application/wasm");
      response.end(readFileSync(join(corpusDir, name)));
      return;
    }
  }
  response.statusCode = 404;
  response.end("not found");
});
await new Promise((resolve, reject) => {
  server.once("error", reject);
  server.listen(0, "127.0.0.1", resolve);
});
const origin = `http://127.0.0.1:${server.address().port}`;
const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

function executable(candidates) {
  for (const candidate of candidates) {
    const found = spawnSync("sh", ["-c", `command -v -- "$1"`, "sh", candidate], {
      encoding: "utf8",
    });
    if (found.status === 0 && found.stdout.trim()) return found.stdout.trim();
  }
  return null;
}

const browserDefinitions = {
  chrome: {
    command: process.env.CHROME || executable(["google-chrome", "chromium", "chromium-browser"]),
    args: (profile, url) => [
      "--headless=new", "--no-sandbox", "--disable-gpu", "--disable-background-networking",
      "--disable-component-update", "--disable-default-apps", "--disable-extensions",
      "--disable-sync", "--disable-translate", "--metrics-recording-only", "--no-first-run",
      "--remote-debugging-port=0", `--user-data-dir=${profile}`, url,
    ],
  },
  firefox: {
    command: process.env.FIREFOX || executable(["firefox"]),
    args: (profile, url) => [
      "--headless", "--remote-debugging-port", "0", "--profile", profile, url,
    ],
  },
};

class CdpConnection {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id && this.pending.has(message.id)) {
        const pending = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error || message.type === "error") {
          pending.reject(new Error(JSON.stringify(message.error ?? message)));
        }
        else pending.resolve(message.result);
      }
    });
  }
  static async open(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", reject, { once: true });
    });
    return new CdpConnection(socket);
  }
  send(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  close() { this.socket.close(); }
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
    const cdp = log.match(/DevTools listening on (ws:\/\/\S+)/);
    if (cdp) resolveUrl({ kind: "cdp", url: cdp[1] });
    const bidi = log.match(/WebDriver BiDi listening on (ws:\/\/\S+)/);
    if (bidi) resolveUrl({ kind: "bidi", url: `${bidi[1]}/session` });
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

async function pageTarget(browserWsUrl, expectedUrl) {
  const endpoint = new URL(browserWsUrl);
  const listUrl = `http://${endpoint.host}/json/list`;
  for (let attempt = 0; attempt < 400; attempt++) {
    try {
      const targets = await (await fetch(listUrl)).json();
      const target = targets.find((candidate) =>
        candidate.type === "page" && candidate.url.startsWith(expectedUrl));
      if (target?.webSocketDebuggerUrl) return target.webSocketDebuggerUrl;
    } catch {
      // The remote endpoint can announce itself just before /json/list exists.
    }
    await delay(25);
  }
  throw new Error(`benchmark page did not appear at ${listUrl}`);
}

async function terminate(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), delay(500)]);
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
}

async function collectPageResult(cdp, engine) {
  let lastError;
  for (let attempt = 0; attempt < 12_000; attempt++) {
    try {
      const evaluation = await cdp.send("Runtime.evaluate", {
        expression: `(() => ({
          done: document.documentElement.dataset.done === "1",
          text: document.querySelector("#result")?.textContent ?? ""
        }))()`,
        returnByValue: true,
      });
      const value = evaluation.result?.value;
      if (value?.done) {
        const result = JSON.parse(value.text);
        if (result.error) throw new Error(result.error);
        return result;
      }
    } catch (error) {
      lastError = error;
    }
    await delay(25);
  }
  throw new Error(`${engine} benchmark timeout: ${lastError ?? "no result"}`);
}

async function collectBidiResult(bidi, context, engine) {
  let lastError;
  for (let attempt = 0; attempt < 12_000; attempt++) {
    try {
      const evaluation = await bidi.send("script.evaluate", {
        expression: `JSON.stringify({
          done: document.documentElement.dataset.done === "1",
          text: document.querySelector("#result")?.textContent ?? ""
        })`,
        target: { context },
        awaitPromise: false,
      });
      if (evaluation.type === "exception") {
        throw new Error(evaluation.exceptionDetails?.text ?? "BiDi script exception");
      }
      const value = JSON.parse(evaluation.result.value);
      if (value.done) {
        const result = JSON.parse(value.text);
        if (result.error) throw new Error(result.error);
        return result;
      }
    } catch (error) {
      lastError = error;
    }
    await delay(25);
  }
  throw new Error(`${engine} BiDi benchmark timeout: ${lastError ?? "no result"}`);
}

async function runSample(engine, sample) {
  const definition = browserDefinitions[engine];
  const profile = mkdtempSync(join(tmpdir(), `rv64-real-${engine}-`));
  const url = `${origin}/jit-real-region-browser.html?reverse=${sample & 1 ? 1 : 0}`;
  const child = spawn(definition.command, definition.args(profile, url), {
    stdio: ["ignore", "pipe", "pipe"],
  });
  let cdp;
  let browserCdp;
  let bidi;
  try {
    const protocol = await waitForProtocol(child);
    if (protocol.kind === "bidi") {
      bidi = await CdpConnection.open(protocol.url);
      await bidi.send("session.new", { capabilities: {} });
      const tree = await bidi.send("browsingContext.getTree");
      const context = tree.contexts.find((candidate) => candidate.url.startsWith(origin))?.context ??
        tree.contexts[0]?.context;
      if (!context) throw new Error("Firefox BiDi returned no browsing context");
      return await collectBidiResult(bidi, context, engine);
    }
    browserCdp = await CdpConnection.open(protocol.url);
    const pageUrl = await pageTarget(protocol.url, `${origin}/jit-real-region-browser.html`);
    cdp = await CdpConnection.open(pageUrl);
    await cdp.send("Runtime.enable");
    return await collectPageResult(cdp, engine);
  } finally {
    cdp?.close();
    if (bidi) {
      await Promise.race([bidi.send("session.end").catch(() => {}), delay(250)]);
      bidi.close();
    }
    if (browserCdp) {
      await Promise.race([browserCdp.send("Browser.close").catch(() => {}), delay(250)]);
      browserCdp.close();
    }
    await terminate(child);
    rmSync(profile, { recursive: true, force: true });
  }
}

const samplesArg = process.argv.find((argument) => argument.startsWith("--samples="));
const samples = samplesArg ? Number(samplesArg.split("=")[1]) : 3;
if (!Number.isInteger(samples) || samples < 1 || samples > 10) {
  throw new Error("--samples must be an integer from 1 through 10");
}
const enginesArg = process.argv.find((argument) => argument.startsWith("--engines="));
const requested = enginesArg ? enginesArg.split("=")[1].split(",") : ["chrome", "firefox"];
const engines = requested.filter((engine) => browserDefinitions[engine]?.command);
if (engines.length === 0) throw new Error("no requested browser executable was found");

const raw = {};
try {
  for (const engine of engines) {
    raw[engine] = [];
    for (let sample = 0; sample < samples; sample++) {
      raw[engine].push(await runSample(engine, sample));
    }
  }
} finally {
  await new Promise((resolve) => server.close(resolve));
}

const manifestByWasm = new Map(manifest.map((record) => [record.wasm, record]));
const groupKey = (record) =>
  `${record.workload}/p${record.pages}/l${record.leader_cap}/${record.mode}`;

const report = {
  schema: 1,
  methodology: "frozen-real-elf-regions/fresh-browser-and-profile-per-sample/alternating-order",
  samples,
  moduleCount: manifest.length,
  regionCount: new Set(manifest.map((record) => record.id)).size,
  modules: moduleMetadata,
  engines: {},
};

for (const engine of engines) {
  const byWasm = new Map(manifest.map((record) => [record.wasm, []]));
  for (const run of raw[engine]) {
    for (const result of run.results) byWasm.get(result.wasm)?.push(result);
  }
  for (const [wasm, results] of byWasm) {
    if (results.length !== samples) throw new Error(`${engine}/${wasm}: incomplete samples`);
  }
  const groups = new Map();
  for (const [wasm, results] of byWasm) {
    const record = manifestByWasm.get(wasm);
    const key = groupKey(record);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push({ record, results });
  }
  const cohorts = {};
  for (const [key, entries] of groups) {
    const groupRuns = raw[engine].map((run) =>
      run.groupResults.find((result) => result.key === key));
    if (groupRuns.some((run) => !run)) throw new Error(`${engine}/${key}: missing group timing`);
    const unsupported = entries.some((entry) =>
      entry.results.some((result) => result.unsupported));
    if (unsupported) {
      cohorts[key] = { supported: false, modules: entries.length };
      continue;
    }
    cohorts[key] = {
      supported: true,
      modules: entries.length,
      bytes: entries.reduce((total, entry) => total + entry.record.bytes, 0),
      compileTotalMs: summary(groupRuns.map((run) => run.compileTotalMs)),
      compilePerModuleMs: summary(entries.flatMap((entry) =>
        entry.results.map((result) => result.compileMs))),
      instantiateTotalMs: summary(groupRuns.map((run) => run.instantiateTotalMs)),
      instantiatePerModuleMs: summary(entries.flatMap((entry) =>
        entry.results.map((result) => result.instantiateMs))),
    };
  }
  const comparisons = {};
  const workloads = [...new Set(manifest.map((record) => record.workload))];
  const geometries = [...new Map(manifest.map((record) => [
    `${record.pages}/${record.leader_cap}`,
    { pages: record.pages, leaderCap: record.leader_cap },
  ])).values()];
  for (const workload of workloads) {
    for (const { pages, leaderCap } of geometries) {
      const prefix = `${workload}/p${pages}/l${leaderCap}`;
      comparisons[prefix] = {};
      for (const [name, numerator, denominator] of [
        ["lazyVsEager", "lazy", "eager"],
        ["directVsEager", "direct", "eager"],
        ["eagerVsMemory", "eager", "memory"],
        ["tailCallVsMemory", "tailcall", "memory"],
      ]) {
        const left = cohorts[`${prefix}/${numerator}`];
        const right = cohorts[`${prefix}/${denominator}`];
        comparisons[prefix][name] = !left?.supported || !right?.supported ? null : {
          bytesRatio: left.bytes / right.bytes,
          compileRatio: summary(left.compileTotalMs.raw.map((value, index) =>
            value / right.compileTotalMs.raw[index])),
          instantiateRatio: summary(left.instantiateTotalMs.raw.map((value, index) =>
            value / right.instantiateTotalMs.raw[index])),
        };
      }
    }
  }
  report.engines[engine] = {
    userAgent: raw[engine][0].userAgent,
    cohorts,
    comparisons,
    rawRuns: raw[engine],
  };
}

const outputArg = process.argv.find((argument) => argument.startsWith("--output="));
if (outputArg) writeFileSync(outputArg.split("=")[1], `${JSON.stringify(report, null, 2)}\n`);
if (process.argv.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const f = (value) => value.toFixed(3);
  console.log(`${samples} samples; ${report.regionCount} real regions; ${report.moduleCount} modules`);
  for (const engine of engines) {
    console.log(`${engine}: ${report.engines[engine].userAgent}`);
    for (const [key, values] of Object.entries(report.engines[engine].comparisons)) {
      const show = (entry) => entry === null ? "unsupported" :
        `${f(entry.bytesRatio)} bytes/${f(entry.compileRatio.median)} compile`;
      console.log(
        `  ${key}: lazy/eager ${show(values.lazyVsEager)}; ` +
          `direct/eager ${show(values.directVsEager)}; ` +
          `eager/memory ${show(values.eagerVsMemory)}; ` +
          `tail/memory ${show(values.tailCallVsMemory)}`,
      );
    }
  }
}
