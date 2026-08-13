// Cross-engine frozen backend benchmark. Each sample gets a fresh browser
// process and fresh profile; variants run as fresh documents in alternating
// order within that process. A tiny local HTTP server supplies exact generated
// modules. DevTools Protocol only awaits/collects results outside timed spans.

import { createHash } from "node:crypto";
import { createServer } from "node:http";
import {
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";
import { median, summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const pageBytes = readFileSync(join(here, "jit-backend-browser.html"));
const corpusDir = join(root, "target/jit-backend-corpus");
const variants = [
  "cached",
  "lazy",
  "direct",
  "materialized",
  "tailcall",
  "cached-memory",
  "cached-memory-no-tlb",
  "lazy-memory",
  "direct-memory",
  "materialized-memory",
  "tailcall-memory",
];

const generated = spawnSync(
  "cargo",
  ["run", "--release", "-q", "-p", "rv64-dbt", "--example", "emit_backend_corpus", "--", corpusDir],
  { cwd: root, encoding: "utf8" },
);
if (generated.status !== 0) {
  throw new Error(generated.stderr || generated.stdout || "corpus generation failed");
}

const corpus = Object.fromEntries(variants.map((variant) => {
  const bytes = readFileSync(join(corpusDir, `${variant}.wasm`));
  return [variant, {
    bytes,
    hash: createHash("sha256").update(bytes).digest("hex"),
  }];
}));

const server = createServer((request, response) => {
  const url = new URL(request.url, "http://127.0.0.1");
  response.setHeader("Cache-Control", "no-store");
  if (url.pathname === "/jit-backend-browser.html") {
    response.setHeader("Content-Type", "text/html; charset=utf-8");
    response.end(pageBytes);
    return;
  }
  if (url.pathname.startsWith("/corpus/")) {
    const name = basename(url.pathname, ".wasm");
    if (variants.includes(name) && url.pathname === `/corpus/${name}.wasm`) {
      response.setHeader("Content-Type", "application/wasm");
      response.end(corpus[name].bytes);
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
const address = server.address();
const origin = `http://127.0.0.1:${address.port}`;

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
      "--headless=new",
      "--no-sandbox",
      "--disable-gpu",
      "--disable-background-networking",
      "--disable-component-update",
      "--disable-default-apps",
      "--disable-extensions",
      "--disable-sync",
      "--disable-translate",
      "--metrics-recording-only",
      "--no-first-run",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      url,
    ],
  },
  firefox: {
    command: process.env.FIREFOX || executable(["firefox"]),
    args: (profile, url) => [
      "--headless",
      "--remote-debugging-port",
      "0",
      "--profile",
      profile,
      url,
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
        const { resolve, reject } = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error || message.type === "error") {
          reject(new Error(JSON.stringify(message.error ?? message)));
        }
        else resolve(message.result);
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
  const timer = setTimeout(() => {
    rejectUrl(new Error(`timed out waiting for DevTools:\n${log}`));
  }, timeoutMs);
  try {
    return await found;
  } finally {
    clearTimeout(timer);
  }
}

async function pageTarget(browserWsUrl, expectedUrl) {
  const endpoint = new URL(browserWsUrl);
  const listUrl = `http://${endpoint.host}/json/list`;
  for (let attempt = 0; attempt < 200; attempt++) {
    try {
      const targets = await (await fetch(listUrl)).json();
      const target = targets.find((candidate) =>
        candidate.type === "page" && candidate.url.startsWith(expectedUrl));
      if (target?.webSocketDebuggerUrl) return target.webSocketDebuggerUrl;
    } catch {
      // The remote agent can announce its port just before /json/list exists.
    }
    await delay(25);
  }
  throw new Error(`benchmark page did not appear at ${listUrl}`);
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

async function collectPageResult(cdp, engine, variant) {
  for (let attempt = 0; attempt < 1_200; attempt++) {
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
      // Navigation destroys the prior execution context before the new one is
      // visible. Other errors are retried only inside this bounded window and
      // become the timeout's cause if the page never reports completion.
      if (attempt === 1_199) throw error;
    }
    await delay(25);
  }
  throw new Error(`timed out waiting for ${engine}/${variant}`);
}

async function collectBidiResult(bidi, context, engine, variant) {
  let lastError;
  for (let attempt = 0; attempt < 1_200; attempt++) {
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
  throw new Error(`timed out waiting for ${engine}/${variant}: ${lastError ?? "no result"}`);
}

async function runBrowserSample(engine, order, sample) {
  const definition = browserDefinitions[engine];
  const profile = mkdtempSync(join(tmpdir(), `rv64-dbt-${engine}-`));
  const pageUrl = (variant, position) =>
    `${origin}/jit-backend-browser.html?variant=${variant}&sample=${sample}&position=${position}`;
  const child = spawn(definition.command, definition.args(profile, pageUrl(order[0], 0)), {
    stdio: ["ignore", "pipe", "pipe"],
  });
  let cdp;
  let browserCdp;
  let bidi;
  try {
    const protocol = await waitForProtocol(child);
    const results = {};
    if (protocol.kind === "bidi") {
      bidi = await CdpConnection.open(protocol.url);
      await bidi.send("session.new", { capabilities: {} });
      const tree = await bidi.send("browsingContext.getTree");
      const context = tree.contexts.find((candidate) => candidate.url.startsWith(origin))?.context ??
        tree.contexts[0]?.context;
      if (!context) throw new Error("Firefox BiDi returned no browsing context");
      for (let position = 0; position < order.length; position++) {
        const variant = order[position];
        if (position !== 0) {
          await bidi.send("browsingContext.navigate", {
            context,
            url: pageUrl(variant, position),
            wait: "complete",
          });
        }
        results[variant] = await collectBidiResult(bidi, context, engine, variant);
      }
      return results;
    }
    browserCdp = await CdpConnection.open(protocol.url);
    const targetUrl = await pageTarget(protocol.url, `${origin}/jit-backend-browser.html`);
    cdp = await CdpConnection.open(targetUrl);
    await cdp.send("Runtime.enable");
    await cdp.send("Page.enable");
    for (let position = 0; position < order.length; position++) {
      const variant = order[position];
      if (position !== 0) {
        await cdp.send("Page.navigate", { url: pageUrl(variant, position) });
      }
      results[variant] = await collectPageResult(cdp, engine, variant);
    }
    return results;
  } finally {
    cdp?.close();
    if (bidi) {
      await Promise.race([bidi.send("session.end").catch(() => {}), delay(250)]);
      bidi.close();
    }
    if (browserCdp) {
      await Promise.race([
        browserCdp.send("Browser.close").catch(() => {}),
        delay(250),
      ]);
      browserCdp.close();
    }
    await terminate(child);
    rmSync(profile, { recursive: true, force: true });
  }
}

const sampleArg = process.argv.find((arg) => arg.startsWith("--samples="));
const samples = sampleArg ? Number(sampleArg.split("=")[1]) : 3;
if (!Number.isInteger(samples) || samples < 1 || samples > 10) {
  throw new Error("--samples must be an integer from 1 through 10");
}
const engineArg = process.argv.find((arg) => arg.startsWith("--engines="));
const requestedEngines = engineArg
  ? engineArg.split("=")[1].split(",")
  : ["chrome", "firefox"];
const engines = requestedEngines.filter((engine) => browserDefinitions[engine]?.command);
if (engines.length === 0) throw new Error("no requested browser executable was found");

const raw = Object.fromEntries(engines.map((engine) => [
  engine,
  Object.fromEntries(variants.map((variant) => [variant, []])),
]));

try {
  for (const engine of engines) {
    for (let sample = 0; sample < samples; sample++) {
      const order = sample % 2 === 0 ? variants : [...variants].reverse();
      const sampleResults = await runBrowserSample(engine, order, sample);
      for (const variant of order) raw[engine][variant].push(sampleResults[variant]);
    }
  }
} finally {
  await new Promise((resolve) => server.close(resolve));
}

const report = {
  schema: 1,
  methodology: "frozen-bytes/fresh-browser-per-sample/fresh-profile/fresh-documents/alternating-order",
  samples,
  corpus: Object.fromEntries(variants.map((variant) => [variant, {
    bytes: corpus[variant].bytes.length,
    hash: corpus[variant].hash,
  }])),
  engines: {},
};

for (const engine of engines) {
  const variantsReport = {};
  for (const variant of variants) {
    const runs = raw[engine][variant];
    const unsupported = runs.filter((run) => run.unsupported);
    if (unsupported.length !== 0) {
      if (unsupported.length !== runs.length) {
        throw new Error(`${engine}/${variant}: support changed between samples`);
      }
      variantsReport[variant] = {
        supported: false,
        userAgent: runs[0].userAgent,
        reason: runs[0].unsupported,
        rawRuns: runs,
      };
      continue;
    }
    variantsReport[variant] = {
      supported: true,
      userAgent: runs[0].userAgent,
      compileMs: summary(runs.map((run) => run.compileMs)),
      instantiateMs: summary(runs.map((run) => run.instantiateMs)),
      firstMs: summary(runs.map((run) => run.firstMs)),
      steadyMs: summary(runs.map((run) => median(run.steadyMs))),
      steadyMInsnPerSec: summary(runs.map((run) =>
        Number(run.steadyInsns) / median(run.steadyMs) / 1000)),
      rawRuns: runs,
    };
  }
  const speedup = (faster, baseline) => {
    if (raw[engine][faster].some((run) => run.unsupported) ||
        raw[engine][baseline].some((run) => run.unsupported)) {
      return null;
    }
    return summary(raw[engine][faster].map((run, index) =>
      median(raw[engine][baseline][index].steadyMs) / median(run.steadyMs)));
  };
  report.engines[engine] = {
    variants: variantsReport,
    pairedSteadySpeedup: {
      cachedState: speedup("cached", "materialized"),
      lazyVsEager: speedup("lazy", "cached"),
      directVsBalanced: speedup("direct", "cached"),
      tailCallVsMaterialized: speedup("tailcall", "materialized"),
      cachedStateWithMemory: speedup("cached-memory", "materialized-memory"),
      lazyVsEagerWithMemory: speedup("lazy-memory", "cached-memory-no-tlb"),
      directVsBalancedWithMemory: speedup("direct-memory", "cached-memory-no-tlb"),
      tailCallVsMaterializedWithMemory: speedup("tailcall-memory", "materialized-memory"),
      translationCache: speedup("cached-memory", "cached-memory-no-tlb"),
    },
  };
}

const outputArg = process.argv.find((arg) => arg.startsWith("--output="));
if (outputArg) {
  writeFileSync(outputArg.split("=")[1], `${JSON.stringify(report, null, 2)}\n`);
}

if (process.argv.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const f = (value) => value.toFixed(3);
  console.log(`${samples} samples; ${report.methodology}`);
  for (const engine of engines) {
    const result = report.engines[engine];
    console.log(`${engine}: ${result.variants.cached.userAgent}`);
    for (const variant of variants) {
      const row = result.variants[variant];
      if (!row.supported) {
        console.log(`  ${variant}: unsupported (${row.reason})`);
        continue;
      }
      console.log(
        `  ${variant}: compile ${f(row.compileMs.median)} ms; first ` +
        `${f(row.firstMs.median)} ms; steady ${f(row.steadyMInsnPerSec.median)} Minsn/s`,
      );
    }
    const ratio = (entry) => entry === null ? "unsupported" : `${f(entry.median)}x`;
    console.log(
      `  paired speedups: state=${ratio(result.pairedSteadySpeedup.cachedState)}; ` +
      `lazy/eager=${ratio(result.pairedSteadySpeedup.lazyVsEager)}; ` +
      `direct/balanced=${ratio(result.pairedSteadySpeedup.directVsBalanced)}; ` +
      `tail/materialized=${ratio(result.pairedSteadySpeedup.tailCallVsMaterialized)}; ` +
      `state+memory=${ratio(result.pairedSteadySpeedup.cachedStateWithMemory)}; ` +
      `lazy/eager+memory=${ratio(result.pairedSteadySpeedup.lazyVsEagerWithMemory)}; ` +
      `direct/balanced+memory=${ratio(result.pairedSteadySpeedup.directVsBalancedWithMemory)}; ` +
      `tail/materialized+memory=${ratio(result.pairedSteadySpeedup.tailCallVsMaterializedWithMemory)}; ` +
      `translation-cache=${ratio(result.pairedSteadySpeedup.translationCache)}`,
    );
  }
}
