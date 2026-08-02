import { RV64 } from "./rv64.js";
import { defaultRelayURL } from "./site-config.js";

export const PUBLISHED_ASSETS = "./assets/demo-images-v2";

export const PRESETS = Object.freeze({
  fast: {
    label: "Fast Linux",
    ramMB: 128,
    local: ["images/bbl64.bin", "images/kernel-riscv64.bin", "images/root-riscv64.bin"],
    release: ["fast-bbl64.bin", "fast-kernel-riscv64.bin", "fast-root-riscv64.bin"],
  },
  modern: {
    label: "Alpine Linux",
    ramMB: 512,
    local: ["images/modern/Image", "images/modern/alpine.ext4"],
    release: ["modern-Image", "modern-alpine.ext4"],
  },
});

const term = document.querySelector("#terminal");
const status = document.querySelector("#status");
const boot = document.querySelector("#boot");
const title = document.querySelector("#terminal-title");
const decoder = new TextDecoder();
const encoder = new TextEncoder();
let selected = "fast";
let active = null;
let generation = 0;

function write(data) {
  let text = typeof data === "string" ? data : decoder.decode(data, { stream: true });
  text = text.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "").replace(/\r/g, "");
  term.textContent += text;
  term.scrollTop = term.scrollHeight;
}

function localAssetCandidates(local, release) {
  const override = new URLSearchParams(location.search).get("assets");
  if (override) return [{ url: `${override.replace(/\/$/, "")}/${release}` }];
  const candidates = (Array.isArray(local) ? local : [local]).map((url) => ({ url }));
  candidates.push({ url: `${PUBLISHED_ASSETS}/${release}` });
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

async function loadWasm() {
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
  term.textContent = "";
  write(`[host] loading ${preset.label}…\n`);
  try {
    if (active) await active.destroy();
    active = null;
    const wasm = await loadWasm();
    const images = [];
    for (let i = 0; i < preset.local.length; i++) {
      const name = preset.release[i];
      images.push(await fetchAsset(preset.local[i], name, (loaded, total) => {
        const amount = total ? `${(loaded / total * 100).toFixed(0)}%` : `${(loaded / 1048576).toFixed(1)} MiB`;
        status.textContent = `Downloading ${name}: ${amount}`;
      }));
    }
    if (myGeneration !== generation) return;
    const bootConfig = presetName === "fast"
      ? { mode: "firmware", firmware: images[0], kernel: images[1], disk: images[2] }
      : {
          mode: "linux-direct",
          kernel: images[0],
          disk: images[1],
          cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
        };
    const vm = await RV64.create({
      wasm,
      memoryMB: preset.ramMB,
      boot: bootConfig,
      network: presetName === "modern"
        ? {
            mode: "fetch",
            relayURL: new URLSearchParams(location.search).get("relay") || defaultRelayURL || undefined,
          }
        : undefined,
      events: {
        console: (data) => write(data),
        stop: ({ reason }) => {
          if (reason === "powered-off") write("\n[host] guest powered off\n");
          status.textContent = reason === "powered-off" ? "Powered off" : "Stopped";
        },
      },
    });
    active = vm;
    const started = performance.now();
    let lastStatus = 0;
    const frame = () => {
      if (myGeneration !== generation || !active) return;
      if (!active.running) return;
      const now = performance.now();
      if (now - lastStatus > 500) {
        lastStatus = now;
        const insns = Number(active.instructions);
        status.textContent = `${(insns / 1e6).toFixed(0)} Minsns · ${(insns / (now - started) / 1000).toFixed(1)} Minsn/s`;
      }
      setTimeout(frame, 0);
    };
    title.textContent = `${preset.label} console`;
    term.focus();
    await vm.start();
    frame();
  } catch (error) {
    write(`\n[host] unable to boot: ${error.message}\n\n`);
    write("For local setup, follow the commands below the terminal.\n");
    status.textContent = "Boot failed";
    console.error(error);
  } finally {
    boot.disabled = false;
  }
}

document.querySelectorAll(".preset").forEach((button) => {
  button.addEventListener("click", () => {
    selected = button.dataset.preset;
    document.querySelectorAll(".preset").forEach((item) => item.classList.toggle("selected", item === button));
    boot.textContent = selected === "fast" ? "Boot Fast Linux" : "Boot Alpine Linux";
  });
});
boot.addEventListener("click", () => start(selected));
document.querySelector("#clear").addEventListener("click", () => { term.textContent = ""; });
term.addEventListener("keydown", (event) => {
  if (!active) return;
  if (event.ctrlKey && event.key.length === 1) {
    active.console.send(new Uint8Array([event.key.toUpperCase().charCodeAt(0) - 64]));
    event.preventDefault();
    return;
  }
  const keys = { Enter: "\r", Backspace: "\x7f", Tab: "\t", Escape: "\x1b", ArrowUp: "\x1b[A", ArrowDown: "\x1b[B", ArrowRight: "\x1b[C", ArrowLeft: "\x1b[D" };
  const text = keys[event.key] ?? (event.key.length === 1 ? event.key : null);
  if (text) { active.console.send(encoder.encode(text)); event.preventDefault(); }
});
