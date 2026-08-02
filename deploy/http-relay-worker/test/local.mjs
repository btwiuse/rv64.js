import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { WebSocket } from "ws";

const port = 8791;
const base = `http://127.0.0.1:${port}`;
const relay = `ws://127.0.0.1:${port}/relay`;
const executable = process.platform === "win32"
  ? "node_modules\\.bin\\wrangler.cmd"
  : "node_modules/.bin/wrangler";
const wrangler = spawn(executable, [
  "dev", "--port", String(port), "--show-interactive-dev-session=false",
], { stdio: ["ignore", "pipe", "pipe"] });
let logs = "";
wrangler.stdout.on("data", bytes => { logs += bytes; });
wrangler.stderr.on("data", bytes => { logs += bytes; });

async function waitForWorker() {
  for (let attempt = 0; attempt < 100; attempt++) {
    try {
      const response = await fetch(`${base}/health`);
      if (response.ok && await response.text() === "ok\n") return;
    } catch { /* still starting */ }
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw new Error(`wrangler did not become ready\n${logs}`);
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();
function field(value) {
  const bytes = typeof value === "string" ? encoder.encode(value) : value;
  const out = new Uint8Array(4 + bytes.length);
  new DataView(out.buffer).setUint32(0, bytes.length, true);
  out.set(bytes, 4);
  return out;
}
function requestPayload(method, url) {
  const parts = [field(method), field(url), new Uint8Array(4), field(new Uint8Array())];
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}
function frame(type, id, payload) {
  const out = new Uint8Array(16 + payload.length);
  out.set(encoder.encode("RHR1"));
  out[4] = type;
  new DataView(out.buffer).setBigUint64(8, id, true);
  out.set(payload, 16);
  return out;
}
async function open(origin) {
  const socket = new WebSocket(relay, { origin });
  await once(socket, "open");
  return socket;
}

try {
  await waitForWorker();

  const rejected = new WebSocket(relay, { origin: "https://attacker.example" });
  const [request, response] = await once(rejected, "unexpected-response");
  assert.equal(response.statusCode, 403);
  request.destroy();

  const socket = await open("https://ibuildthecloud.github.io");
  const id = 1n;
  const result = new Promise((resolve, reject) => {
    let status;
    socket.on("message", data => {
      try {
        const bytes = new Uint8Array(data);
        assert.equal(decoder.decode(bytes.subarray(0, 4)), "RHR1");
        assert.equal(new DataView(bytes.buffer, bytes.byteOffset).getBigUint64(8, true), id);
        if (bytes[4] === 2) {
          status = new DataView(bytes.buffer, bytes.byteOffset + 16).getUint32(0, true);
        } else if (bytes[4] === 4) {
          resolve(status);
        } else if (bytes[4] === 5) {
          reject(new Error(decoder.decode(bytes.subarray(16))));
        }
      } catch (error) {
        reject(error);
      }
    });
    socket.on("error", reject);
  });
  socket.send(frame(1, id, requestPayload("HEAD", "https://dl-cdn.alpinelinux.org/alpine/")));
  assert.equal(await result, 200);
  socket.close();

  const local = await open("http://localhost:4321");
  local.close();
  console.log("PASS local Wrangler relay — health, origin policy, Alpine fetch, localhost");
} finally {
  wrangler.kill("SIGTERM");
  await Promise.race([
    once(wrangler, "exit"),
    new Promise(resolve => setTimeout(resolve, 2000)),
  ]);
}
