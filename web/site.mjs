import { RV64 } from "./rv64.js";

export const RELEASE_ASSETS =
  "https://github.com/ibuildthecloud/rv64.js/releases/download/demo-images-v1";

export const PRESETS = Object.freeze({
  fast: {
    label: "Fast Linux",
    ramMB: 128,
    local: ["images/bbl64.bin", "images/kernel-riscv64.bin", "images/root-riscv64.bin"],
    release: ["fast-bbl64.bin", "fast-kernel-riscv64.bin", "fast-root-riscv64.bin"],
  },
  modern: {
    label: "Modern Debian",
    ramMB: 512,
    local: ["images/modern/opensbi.bin", "images/modern/Image", "images/modern/debian.ext4"],
    release: ["modern-opensbi.bin", "modern-Image", "modern-debian.ext4"],
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

function assetCandidates(local, release) {
  const override = new URLSearchParams(location.search).get("assets");
  if (override) return [`${override.replace(/\/$/, "")}/${release}`];
  return [local, `${RELEASE_ASSETS}/${release}`];
}

async function fetchAsset(local, release, progress) {
  let last;
  for (const url of assetCandidates(local, release)) {
    try {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      const total = Number(response.headers.get("content-length")) || 0;
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
    } catch (error) {
      last = new Error(`${url}: ${error.message}`);
    }
  }
  throw last;
}

async function loadWasm() {
  return fetchAsset(
    "../target/wasm32-unknown-unknown/release/rv64_wasm.wasm",
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
    const wasm = await loadWasm();
    const vm = await RV64.create(wasm);
    vm.onWrite = (_fd, data) => write(data);
    const images = [];
    for (let i = 0; i < preset.local.length; i++) {
      const name = preset.release[i];
      images.push(await fetchAsset(preset.local[i], name, (loaded, total) => {
        const amount = total ? `${(loaded / total * 100).toFixed(0)}%` : `${(loaded / 1048576).toFixed(1)} MiB`;
        status.textContent = `Downloading ${name}: ${amount}`;
      }));
    }
    if (myGeneration !== generation) return;
    if (presetName === "fast") {
      vm.bootLinux({ bios: images[0], kernel: images[1], disk: images[2], ramMB: preset.ramMB });
      active = { vm, run: () => vm.runSystem(3_000_000n), input: (b) => vm.consoleInput(b), count: () => vm.sysInsnCount() };
    } else {
      vm.bootVirtLinux({
        opensbi: images[0], kernel: images[1], disk: images[2], ramMB: preset.ramMB,
        cmdline: "console=ttyS0 root=/dev/vda rw init=/binit.sh",
      });
      active = { vm, run: () => vm.runVirtSystem(2_000_000n), input: (b) => vm.virtConsoleInput(b), count: () => vm.virtInsnCount() };
    }
    const started = performance.now();
    let lastStatus = 0;
    const frame = () => {
      if (myGeneration !== generation || !active) return;
      if (active.run()) { write("\n[host] guest powered off\n"); status.textContent = "Powered off"; return; }
      const now = performance.now();
      if (now - lastStatus > 500) {
        lastStatus = now;
        const insns = Number(active.count());
        status.textContent = `${(insns / 1e6).toFixed(0)} Minsns · ${(insns / (now - started) / 1000).toFixed(1)} Minsn/s`;
      }
      setTimeout(frame, 0);
    };
    title.textContent = `${preset.label} console`;
    term.focus();
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
    boot.textContent = selected === "fast" ? "Boot Fast Linux" : "Boot Modern Debian";
  });
});
boot.addEventListener("click", () => start(selected));
document.querySelector("#clear").addEventListener("click", () => { term.textContent = ""; });
term.addEventListener("keydown", (event) => {
  if (!active) return;
  if (event.ctrlKey && event.key.length === 1) {
    active.input(new Uint8Array([event.key.toUpperCase().charCodeAt(0) - 64]));
    event.preventDefault();
    return;
  }
  const keys = { Enter: "\r", Backspace: "\x7f", Tab: "\t", Escape: "\x1b", ArrowUp: "\x1b[A", ArrowDown: "\x1b[B", ArrowRight: "\x1b[C", ArrowLeft: "\x1b[D" };
  const text = keys[event.key] ?? (event.key.length === 1 ? event.key : null);
  if (text) { active.input(encoder.encode(text)); event.preventDefault(); }
});
