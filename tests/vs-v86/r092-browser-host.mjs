import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const externalArtifacts = join(root, "target/bench");
export const R092_ASSETS = Object.freeze({
  page: {
    path: join(root, "tests/jit-modern-boot-r092-browser.html"),
    route: "/jit-modern-boot-r092-browser.html",
  },
  worker: {
    path: join(root, "tests/jit-modern-boot-r092-browser-worker.mjs"),
    route: "/jit-modern-boot-r092-browser-worker.mjs",
  },
  timingLibrary: {
    path: join(root, "tests/vs-v86/r076-browser-boot-lib.mjs"),
    route: "/browser-boot-lib.mjs",
  },
  loader: {
    path: join(root, "web/rv64.js"),
    route: "/web/rv64.js",
    expectedSha256: "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385",
  },
  wasmControl: {
    path: join(root, "target/bench/wasm-candidates/r085-fast-jit-state-efd7830307ef.wasm"),
    route: "/rv64-control.wasm",
    expectedSha256: "efd7830307ef0d36630ea6d64074f438c671be0647f14b87942843dd39196010",
  },
  wasmCandidate: {
    path: join(root, "target/bench/wasm-candidates/r092-member-range-5baeccb5c5fe.wasm"),
    route: "/rv64-candidate.wasm",
    expectedSha256: "5baeccb5c5feaf2d3f7605fd42f741f9cbaa89e566a86c0bbea201a3c6389023",
  },
  kernel: {
    path: join(root, "web/images/alpine/Image"),
    route: "/kernel",
    expectedSha256: "57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2",
  },
  initramfs: {
    path: join(externalArtifacts, "scorecard-v2-modern-riscv64.cpio"),
    route: "/initramfs",
    expectedSha256: "cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808",
  },
});

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function r092AssetManifest() {
  return Object.fromEntries(Object.entries(R092_ASSETS).map(([name, asset]) => {
    const bytes = readFileSync(asset.path);
    const digest = sha256(bytes);
    if (asset.expectedSha256 && digest !== asset.expectedSha256) {
      throw new Error(`${name} SHA-256 ${digest} != ${asset.expectedSha256}`);
    }
    return [name, { path: asset.path, route: asset.route, bytes: bytes.length, sha256: digest }];
  }));
}

const contentTypes = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
};

export async function startR092Server() {
  const byRoute = new Map(Object.values(R092_ASSETS).map((asset) => [asset.route, asset]));
  const server = createServer((request, response) => {
    const url = new URL(request.url, "http://127.0.0.1");
    const asset = byRoute.get(url.pathname);
    response.setHeader("Cache-Control", "no-store");
    if (!asset) {
      response.statusCode = 404;
      response.end("not found");
      return;
    }
    const extension = asset.path.slice(asset.path.lastIndexOf("."));
    response.setHeader("Content-Type", contentTypes[extension] ?? "application/octet-stream");
    response.end(readFileSync(asset.path));
  });
  await new Promise((resolveListen, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolveListen);
  });
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    close: () => new Promise((resolveClose) => server.close(resolveClose)),
  };
}

export function browserExecutable() {
  if (process.env.CHROME) return process.env.CHROME;
  for (const candidate of ["google-chrome", "chromium", "chromium-browser"]) {
    const found = spawnSync("sh", ["-c", `command -v -- "$1"`, "sh", candidate], {
      encoding: "utf8",
    });
    if (found.status === 0 && found.stdout.trim()) return found.stdout.trim();
  }
  throw new Error("Chrome/Chromium executable not found");
}

const delay = (milliseconds) => new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));

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
      // The target list can trail the DevTools announcement briefly.
    }
    await delay(25);
  }
  throw new Error("R092 benchmark page target did not appear");
}

async function terminate(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([new Promise((resolveExit) => child.once("exit", resolveExit)), delay(500)]);
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
}

async function collect(cdp) {
  let lastError;
  for (let attempt = 0; attempt < 24_000; attempt++) {
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
  throw new Error(`R092 browser benchmark timeout: ${lastError ?? "no result"}`);
}

export async function runR092BrowserSample({ origin, variant, chrome }) {
  if (variant !== "control" && variant !== "candidate") {
    throw new Error(`invalid R092 variant ${variant}`);
  }
  const profile = mkdtempSync(join(tmpdir(), `rv64-r092-${variant}-chrome-`));
  const url = `${origin}/jit-modern-boot-r092-browser.html?variant=${variant}`;
  const startedAt = new Date().toISOString();
  const child = spawn(chrome, [
    "--headless=new", "--no-sandbox", "--disable-gpu",
    "--disable-background-networking", "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows", "--disable-component-update",
    "--disable-default-apps", "--disable-extensions", "--disable-renderer-backgrounding",
    "--disable-sync", "--disable-translate", "--metrics-recording-only", "--no-first-run",
    "--remote-debugging-port=0", `--user-data-dir=${profile}`, url,
  ], { stdio: ["ignore", "pipe", "pipe"] });
  let pageCdp;
  let browserCdp;
  try {
    const browserUrl = await waitForProtocol(child);
    browserCdp = await CdpConnection.open(browserUrl);
    const browser = await browserCdp.send("Browser.getVersion");
    pageCdp = await CdpConnection.open(await pageTarget(browserUrl, url));
    await pageCdp.send("Runtime.enable");
    const result = await collect(pageCdp);
    return { startedAt, finishedAt: new Date().toISOString(), browser, result };
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
