import { RV64 } from "./rv64.js?v=3";
import { resolveAssetOverride } from "./asset-source.mjs";
import { defaultAssetURL, defaultRelayURL } from "./site-config.js?v=2";
import { summarizeJitStats } from "./jit-telemetry.mjs?v=1";
import { Terminal } from "https://esm.sh/@xterm/xterm@6.0.0";
import { FitAddon } from "https://esm.sh/@xterm/addon-fit@0.11.0";

export const PUBLISHED_ASSETS = defaultAssetURL;

export const PRESETS = Object.freeze({
  alpine: {
    label: "Alpine Linux",
    ramMB: 512,
    local: ["images/alpine/Image", "images/alpine/alpine.ext4"],
    release: ["modern-Image", "modern-alpine.ext4"],
  },
});

const terminalElement = document.querySelector("#terminal");
const terminal = new Terminal({
  convertEol: true,
  cursorBlink: true,
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
  fontSize: 14,
  lineHeight: 1.2,
  scrollback: 5000,
  theme: {
    background: "#030504",
    foreground: "#b9f6ce",
    cursor: "#6ee7a8",
    selectionBackground: "#355b48",
  },
});
const fitAddon = new FitAddon();
terminal.loadAddon(fitAddon);
terminal.open(terminalElement);
fitAddon.fit();
terminal.write("Press Boot Alpine Linux to start.\r\n");
new ResizeObserver(() => fitAddon.fit()).observe(terminalElement);
const status = document.querySelector("#status");
const networkStatus = document.querySelector("#network-status");
const jitTelemetry = {
  state: document.querySelector("#jit-state"),
  detail: document.querySelector("#jit-state-detail"),
  coverage: document.querySelector("#jit-coverage"),
  execution: document.querySelector("#jit-execution"),
  code: document.querySelector("#jit-code"),
  codeDetail: document.querySelector("#jit-code-detail"),
  pipeline: document.querySelector("#jit-pipeline"),
  pipelineDetail: document.querySelector("#jit-pipeline-detail"),
  dispatch: document.querySelector("#jit-dispatch"),
  dispatchDetail: document.querySelector("#jit-dispatch-detail"),
  fallback: document.querySelector("#jit-fallback"),
  fallbackDetail: document.querySelector("#jit-fallback-detail"),
};
const boot = document.querySelector("#boot");
const title = document.querySelector("#terminal-title");
const decoder = new TextDecoder();
const encoder = new TextEncoder();
let active = null;
let generation = 0;
const requestedExecution = new URLSearchParams(location.search).get("execution");
const executionMode = requestedExecution === "local" ? "local" : "worker";

function cpuStatus(text) {
  status.textContent = `${text} · ${executionMode}`;
}
cpuStatus("Ready");

function setJitState(state, detail) {
  jitTelemetry.state.textContent = state;
  jitTelemetry.detail.textContent = detail;
  jitTelemetry.detail.title = detail;
}

function resetJitTelemetry(state = "Waiting to boot", detail = "Statistics sampled once per second") {
  setJitState(state, detail);
  for (const field of [
    "coverage",
    "execution",
    "code",
    "codeDetail",
    "pipeline",
    "pipelineDetail",
    "dispatch",
    "dispatchDetail",
    "fallback",
    "fallbackDetail",
  ]) {
    jitTelemetry[field].textContent = "—";
    jitTelemetry[field].title = "";
  }
}

function renderJitTelemetry(stats) {
  const summary = summarizeJitStats(stats);
  setJitState(summary.state, summary.detail);
  for (const field of [
    "coverage",
    "execution",
    "code",
    "codeDetail",
    "pipeline",
    "pipelineDetail",
    "dispatch",
    "dispatchDetail",
    "fallback",
    "fallbackDetail",
  ]) {
    jitTelemetry[field].textContent = summary[field];
    jitTelemetry[field].title = summary[field];
  }
}

function write(data) {
  terminal.write(typeof data === "string" ? data : decoder.decode(data, { stream: true }));
}

function localAssetCandidates(local, release) {
  const override = resolveAssetOverride(
    new URLSearchParams(location.search).get("assets"),
    location.href,
  );
  if (override) return [{ url: `${override}/${release}` }];
  const candidates = [];
  if (PUBLISHED_ASSETS) {
    candidates.push({ url: `${PUBLISHED_ASSETS.replace(/\/$/, "")}/${release}` });
  }
  candidates.push(...(Array.isArray(local) ? local : [local]).map((url) => ({ url })));
  return candidates;
}

async function downloadAsset(candidate, progress) {
  const response = await fetch(candidate.url, { headers: candidate.headers });
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  const total = response.headers.has("content-encoding")
    ? 0
    : Number(response.headers.get("content-length")) || 0;
  if (!response.body) return new Uint8Array(await response.arrayBuffer());
  const reader = response.body.getReader();
  const chunks = [];
  let loaded = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    loaded += value.length;
    progress(loaded, total);
  }
  const result = new Uint8Array(loaded);
  let offset = 0;
  for (const chunk of chunks) { result.set(chunk, offset); offset += chunk.length; }
  return result;
}

async function fetchAsset(local, release, progress) {
  let last;
  const candidates = localAssetCandidates(local, release);
  for (const candidate of candidates) {
    try {
      return await downloadAsset(candidate, progress);
    } catch (error) {
      last = new Error(`${candidate.url}: ${error.message}`);
    }
  }

  throw last;
}

async function loadRuntime() {
  return fetchAsset(
    ["./rv64_wasm.wasm", "../target/wasm32-unknown-unknown/release/rv64_wasm.wasm"],
    "rv64_wasm.wasm",
    () => {},
  );
}

async function start(presetName) {
  const myGeneration = ++generation;
  const preset = PRESETS[presetName];
  boot.disabled = true;
  terminal.clear();
  networkStatus.textContent = "Network idle";
  networkStatus.title = "";
  resetJitTelemetry("Loading", "Preparing the runtime and guest images");
  write(`[host] loading ${preset.label}…\n`);
  try {
    if (active) await active.destroy();
    active = null;
    const wasm = await loadRuntime();
    const images = [];
    for (let i = 0; i < preset.local.length; i++) {
      const name = preset.release[i];
      images.push(await fetchAsset(preset.local[i], name, (loaded, total) => {
        const amount = total ? `${(loaded / total * 100).toFixed(0)}%` : `${(loaded / 1048576).toFixed(1)} MiB`;
        cpuStatus(`Downloading ${name}: ${amount}`);
      }));
    }
    if (myGeneration !== generation) return;
    const bootConfig = {
      mode: "linux-direct",
      kernel: images[0],
      disk: images[1],
      cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
    };
    const vm = await RV64.create({
      wasm,
      memoryMB: preset.ramMB,
      execution: { mode: executionMode },
      boot: bootConfig,
      network: {
        mode: "fetch",
        relayURL: new URLSearchParams(location.search).get("relay") || defaultRelayURL || undefined,
      },
      events: {
        console: (data) => write(data),
        networkTraffic: (() => {
          let requests = 0;
          let uploaded = 0;
          let downloaded = 0;
          let last = "";
          const render = () => {
            const received = downloaded < 1048576
              ? `${(downloaded / 1024).toFixed(0)} KiB`
              : `${(downloaded / 1048576).toFixed(1)} MiB`;
            const sent = uploaded < 1024
              ? `${uploaded} B`
              : `${(uploaded / 1024).toFixed(1)} KiB`;
            networkStatus.textContent = `${requests} requests · ${sent} sent · ${received} received${last ? ` · ${last}` : ""}`;
          };
          return (detail) => {
            if (detail.type === "request") {
              requests++;
              uploaded += detail.bytes;
              last = `${detail.method} ${new URL(detail.url).hostname}`;
            } else if (detail.type === "download") {
              downloaded += detail.bytes;
            } else if (detail.type === "response") {
              last = `HTTP ${detail.status}`;
            } else if (detail.type === "end") {
              last = "complete";
            } else if (detail.type === "error") {
              last = "network error";
              networkStatus.title = detail.message;
              write(`\n[network] ${detail.message}\n`);
            }
            render();
          };
        })(),
        stop: ({ reason }) => {
          if (reason === "powered-off") write("\n[host] guest powered off\n");
          cpuStatus(reason === "powered-off" ? "Powered off" : "Stopped");
          if (myGeneration === generation) {
            setJitState("Stopped", "Latest sampled counters retained below");
          }
        },
      },
    });
    active = vm;
    setJitState("Starting", "Waiting for the first JIT counter snapshot");
    const started = performance.now();
    let lastStatus = 0;
    let lastJitSample = 0;
    let jitSampleInFlight = false;

    const sampleJit = () => {
      if (jitSampleInFlight) return;
      jitSampleInFlight = true;
      Promise.resolve()
        .then(() => vm.jitStats())
        .then((stats) => {
          if (myGeneration === generation && active === vm && vm.running) {
            renderJitTelemetry(stats);
          }
        })
        .catch((error) => {
          if (myGeneration === generation && active === vm && vm.running) {
            setJitState("Metrics unavailable", error.message);
          }
        })
        .finally(() => { jitSampleInFlight = false; });
    };

    const frame = () => {
      if (myGeneration !== generation || !active) return;
      if (!active.running) return;
      const now = performance.now();
      if (now - lastStatus > 500) {
        lastStatus = now;
        const insns = Number(active.instructions);
        cpuStatus(`${(insns / 1e6).toFixed(0)} Minsns · ${(insns / (now - started) / 1000).toFixed(1)} Minsn/s`);
      }
      if (now - lastJitSample >= 1_000 && document.visibilityState !== "hidden") {
        lastJitSample = now;
        sampleJit();
      }
      setTimeout(frame, 500);
    };
    title.textContent = `${preset.label} console`;
    terminal.focus();
    await vm.start();
    frame();
  } catch (error) {
    write(`\n[host] unable to boot: ${error.message}\n\n`);
    write("For local setup, follow the commands below the terminal.\n");
    cpuStatus("Boot failed");
    resetJitTelemetry("Unavailable", error.message);
    console.error(error);
  } finally {
    boot.disabled = false;
  }
}

boot.addEventListener("click", () => start("alpine"));
document.querySelector("#clear").addEventListener("click", () => terminal.clear());
terminal.onData((data) => {
  if (active) active.console.send(encoder.encode(data));
});
