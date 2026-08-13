#!/usr/bin/env node
// Actual-emulator browser A/B. Every interpreter/policy sample uses a fresh
// Chromium process and profile. Asset fetch and primary emulator-Wasm setup
// are reported separately from Linux boot and guest workload timings.

import { createServer } from "node:http";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";
import { summary } from "./statistics.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const page = readFileSync(join(here, "jit-page-policy-browser.html"));
const assets = {
  "/web/rv64.js": resolve(root, "web/rv64.js"),
  "/rv64.wasm": resolve(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
  "/kernel": resolve(root, process.env.RV64_MODERN_KERNEL || "web/images/alpine/Image"),
  "/disk": resolve(root, process.env.RV64_ALPINE_DISK || "web/images/alpine/alpine.ext4"),
  "/syscompute": resolve(
    root,
    "guests/syscompute/target/riscv64gc-unknown-linux-musl/release/syscompute",
  ),
};
const contentTypes = {
  "/web/rv64.js": "text/javascript; charset=utf-8",
  "/rv64.wasm": "application/wasm",
};

function option(name, fallback) {
  const arg = process.argv.find((value) => value.startsWith(`--${name}=`));
  return arg ? arg.slice(name.length + 3) : fallback;
}
const samples = Number(option("samples", "3"));
const threshold = Number(option("threshold", "1048576"));
const quantum = Number(option("quantum", "1024"));
const workloads = option("workloads", "alu1,alu5,mix20");
if (!Number.isInteger(samples) || samples < 1 || samples > 10) {
  throw new Error("--samples must be an integer from 1 through 10");
}

const server = createServer((request, response) => {
  const url = new URL(request.url, "http://127.0.0.1");
  response.setHeader("Cache-Control", "no-store");
  if (url.pathname === "/jit-page-policy-browser.html") {
    response.setHeader("Content-Type", "text/html; charset=utf-8");
    response.end(page);
    return;
  }
  if (assets[url.pathname]) {
    response.setHeader("Content-Type", contentTypes[url.pathname] ?? "application/octet-stream");
    response.end(readFileSync(assets[url.pathname]));
    return;
  }
  response.statusCode = 404;
  response.end("not found");
});
await new Promise((resolveListen, reject) => {
  server.once("error", reject);
  server.listen(0, "127.0.0.1", resolveListen);
});
const origin = `http://127.0.0.1:${server.address().port}`;
const delay = (milliseconds) => new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));

function executable(candidates) {
  for (const candidate of candidates) {
    const found = spawnSync("sh", ["-c", `command -v -- "$1"`, "sh", candidate], {
      encoding: "utf8",
    });
    if (found.status === 0 && found.stdout.trim()) return found.stdout.trim();
  }
  return null;
}
const chrome = process.env.CHROME || executable(["google-chrome", "chromium", "chromium-browser"]);
if (!chrome) throw new Error("Chrome/Chromium executable not found");

class CdpConnection {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (!message.id || !this.pending.has(message.id)) return;
      const pending = this.pending.get(message.id);
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(JSON.stringify(message.error)));
      else pending.resolve(message.result);
    });
  }
  static async open(url) {
    const socket = new WebSocket(url);
    await new Promise((resolveOpen, reject) => {
      socket.addEventListener("open", resolveOpen, { once: true });
      socket.addEventListener("error", reject, { once: true });
    });
    return new CdpConnection(socket);
  }
  send(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolveSend, reject) => {
      this.pending.set(id, { resolve: resolveSend, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  close() { this.socket.close(); }
}

async function waitForProtocol(child, timeoutMs = 20_000) {
  let log = "";
  let resolveUrl;
  let rejectUrl;
  const found = new Promise((resolveFound, rejectFound) => {
    resolveUrl = resolveFound;
    rejectUrl = rejectFound;
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
  for (let attempt = 0; attempt < 400; attempt++) {
    try {
      const targets = await (await fetch(`http://${endpoint.host}/json/list`)).json();
      const target = targets.find((candidate) =>
        candidate.type === "page" && candidate.url.startsWith(expected));
      if (target?.webSocketDebuggerUrl) return target.webSocketDebuggerUrl;
    } catch {
      // The JSON endpoint can trail the DevTools announcement briefly.
    }
    await delay(25);
  }
  throw new Error("benchmark page target did not appear");
}

async function terminate(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([new Promise((resolveExit) => child.once("exit", resolveExit)), delay(500)]);
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
}

async function collect(cdp) {
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
  throw new Error(`browser benchmark timeout: ${lastError ?? "no result"}`);
}

async function runSample(variant, sample) {
  const profile = mkdtempSync(join(tmpdir(), "rv64-page-policy-chrome-"));
  const url = `${origin}/jit-page-policy-browser.html?` + new URLSearchParams({
    variant,
    threshold: String(threshold),
    quantum: String(quantum),
    workloads,
    sample: String(sample),
  });
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
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    url,
  ], { stdio: ["ignore", "pipe", "pipe"] });
  let pageCdp;
  let browserCdp;
  try {
    const browserUrl = await waitForProtocol(child);
    browserCdp = await CdpConnection.open(browserUrl);
    const targetUrl = await pageTarget(browserUrl, `${origin}/jit-page-policy-browser.html`);
    pageCdp = await CdpConnection.open(targetUrl);
    await pageCdp.send("Runtime.enable");
    return await collect(pageCdp);
  } finally {
    pageCdp?.close();
    if (browserCdp) {
      await Promise.race([browserCdp.send("Browser.close").catch(() => {}), delay(250)]);
      browserCdp.close();
    }
    await terminate(child);
    rmSync(profile, { recursive: true, force: true });
  }
}

const raw = { interpreter: [], page: [] };
try {
  for (let sample = 0; sample < samples; sample++) {
    const order = sample & 1 ? ["page", "interpreter"] : ["interpreter", "page"];
    for (const variant of order) raw[variant].push(await runSample(variant, sample));
  }
} finally {
  await new Promise((resolveClose) => server.close(resolveClose));
}

const workloadNames = workloads.split(",");
const variants = {};
for (const variant of ["interpreter", "page"]) {
  variants[variant] = {
    bootMs: summary(raw[variant].map((run) => run.boot.ms)),
    bootMips: summary(raw[variant].map((run) => run.boot.mips)),
    workloads: {},
  };
  for (const workload of workloadNames) {
    variants[variant].workloads[workload] = {
      firstMs: summary(raw[variant].map((run) => run.workloads[workload][0].ms)),
      repeatMs: summary(raw[variant].map((run) => run.workloads[workload][1].ms)),
      firstMips: summary(raw[variant].map((run) => run.workloads[workload][0].mips)),
      repeatMips: summary(raw[variant].map((run) => run.workloads[workload][1].mips)),
    };
  }
}
const speedups = {
  boot: variants.interpreter.bootMs.median / variants.page.bootMs.median,
  workloads: Object.fromEntries(workloadNames.map((workload) => [workload, {
    first: variants.interpreter.workloads[workload].firstMs.median /
      variants.page.workloads[workload].firstMs.median,
    repeat: variants.interpreter.workloads[workload].repeatMs.median /
      variants.page.workloads[workload].repeatMs.median,
  }])),
};
const report = {
  schema: 1,
  methodology: "fresh-chromium-process-and-profile-per-variant/process-order-alternated",
  samples,
  threshold,
  quantum,
  workloads: workloadNames,
  chrome,
  userAgent: raw.interpreter[0].userAgent,
  variants,
  speedups,
  raw,
};
const outputArg = process.argv.find((arg) => arg.startsWith("--output="));
if (outputArg) writeFileSync(outputArg.split("=")[1], JSON.stringify(report, null, 2) + "\n");
if (process.argv.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  console.log(`${samples} fresh Chromium processes/profiles per variant`);
  console.log(
    `boot: interpreter ${variants.interpreter.bootMs.median.toFixed(1)}ms, ` +
      `page ${variants.page.bootMs.median.toFixed(1)}ms (${speedups.boot.toFixed(2)}x)`,
  );
  for (const workload of workloadNames) {
    const interpreter = variants.interpreter.workloads[workload];
    const pageResult = variants.page.workloads[workload];
    const gain = speedups.workloads[workload];
    console.log(
      `${workload}: interpreter ${interpreter.firstMs.median.toFixed(1)}/` +
        `${interpreter.repeatMs.median.toFixed(1)}ms, page ` +
        `${pageResult.firstMs.median.toFixed(1)}/${pageResult.repeatMs.median.toFixed(1)}ms ` +
        `(${gain.first.toFixed(2)}x/${gain.repeat.toFixed(2)}x)`,
    );
  }
}
