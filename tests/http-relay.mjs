// End-to-end test for web/http-relay.mjs using Node's real WebSocket and fetch.
//
// Node exposes a global WebSocket unflagged only from v22; on v20 (the dev
// shell's node) it needs --experimental-websocket. Without this the test
// crashed with `ReferenceError: WebSocket is not defined` and took the
// whole suite down with it — so the relay path was never actually being
// exercised here. Re-exec once with the flag, and SKIP explicitly if the
// runtime still has no WebSocket (rather than failing the suite for a
// missing runtime feature).
if (typeof WebSocket === "undefined") {
  const { spawnSync } = await import("node:child_process");
  if (!process.env.HTTP_RELAY_REEXEC) {
    const r = spawnSync(
      process.execPath,
      ["--experimental-websocket", ...process.argv.slice(1)],
      { stdio: "inherit", env: { ...process.env, HTTP_RELAY_REEXEC: "1" } },
    );
    process.exit(r.status ?? 1);
  }
  console.log("SKIP http-relay (no WebSocket in this runtime; needs node >= 22 or --experimental-websocket)");
  process.exit(0);
}

import { createServer } from "node:http";
import { startHttpRelay } from "../web/http-relay.mjs";
import { RV64 } from "../web/rv64.js";

const encoder = new TextEncoder();
const MAGIC = encoder.encode("RHR1");

function putField(parts, value) {
  const bytes =
    typeof value === "string" ? encoder.encode(value) : new Uint8Array(value);
  const length = new Uint8Array(4);
  new DataView(length.buffer).setUint32(0, bytes.length, true);
  parts.push(length, bytes);
}

function encodeRequest({ method, url, headers = [], body = new Uint8Array() }) {
  const parts = [];
  putField(parts, method);
  putField(parts, url);
  const count = new Uint8Array(4);
  new DataView(count.buffer).setUint32(0, headers.length, true);
  parts.push(count);
  for (const [name, value] of headers) {
    putField(parts, name);
    putField(parts, value);
  }
  putField(parts, body);
  const length = parts.reduce((total, part) => total + part.length, 0);
  const out = new Uint8Array(length);
  let pos = 0;
  for (const part of parts) {
    out.set(part, pos);
    pos += part.length;
  }
  return out;
}

function protocolFrame(type, id, payload) {
  const out = new Uint8Array(16 + payload.length);
  out.set(MAGIC);
  out[4] = type;
  new DataView(out.buffer).setBigUint64(8, id, true);
  out.set(payload, 16);
  return out;
}

function decodeFrame(data) {
  const bytes = new Uint8Array(data);
  if (bytes.length < 16 || MAGIC.some((byte, index) => bytes[index] !== byte)) {
    throw new Error("bad relay frame");
  }
  return {
    type: bytes[4],
    id: new DataView(bytes.buffer, bytes.byteOffset).getBigUint64(8, true),
    payload: bytes.subarray(16),
  };
}

const origin = createServer((request, response) => {
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    const body = Buffer.concat(chunks).toString();
    if (request.method !== "POST" || body !== "through the relay") {
      response.writeHead(400);
      response.end("bad request");
      return;
    }
    response.writeHead(201, { "Content-Type": "text/plain", "X-Relay": "yes" });
    response.write("RELAY-");
    setImmediate(() => response.end("OK"));
  });
});
await new Promise((resolve) => origin.listen(0, "127.0.0.1", resolve));
const originUrl = `http://127.0.0.1:${origin.address().port}/upload`;
const relay = await startHttpRelay();
const socket = new WebSocket(relay.url);
socket.binaryType = "arraybuffer";
await new Promise((resolve, reject) => {
  socket.onopen = resolve;
  socket.onerror = () => reject(new Error("WebSocket open failed"));
});

const id = 7n;
const request = encodeRequest({
  method: "POST",
  url: originUrl,
  headers: [["Content-Type", "text/plain"]],
  body: encoder.encode("through the relay"),
});
const received = [];
let status = 0;
const complete = new Promise((resolve, reject) => {
  socket.onmessage = (event) => {
    try {
      const message = decodeFrame(event.data);
      if (message.id !== id) throw new Error("response id mismatch");
      if (message.type === 2) {
        status = new DataView(
          message.payload.buffer,
          message.payload.byteOffset,
        ).getUint32(0, true);
      } else if (message.type === 3) {
        received.push(message.payload.slice());
      } else if (message.type === 4) {
        resolve();
      } else if (message.type === 5) {
        reject(new Error(new TextDecoder().decode(message.payload)));
      }
    } catch (error) {
      reject(error);
    }
  };
});
socket.send(protocolFrame(1, id, request));
await complete;

const body = Buffer.concat(received.map((bytes) => Buffer.from(bytes))).toString();
const ok = status === 201 && body === "RELAY-OK";
console.log(
  `${ok ? "PASS" : "FAIL"} HTTP WebSocket relay — status=${status} body=${JSON.stringify(body)}`,
);

// A failed fetch is ambiguous for non-idempotent methods: the origin may have
// acted before CORS hid its response. Verify POST is not retried unless its
// origin was explicitly selected for direct relay routing.
let fetchCalls = 0;
let relayCalls = 0;
let failures = 0;
const fakeVm = {
  httpRelay: {},
  httpRelayOrigins: new Set(),
  stageFor() {},
  ex: {
    sys_http_fail() {
      failures++;
    },
  },
  async performHttpViaRelay() {
    relayCalls++;
  },
};
const realFetch = globalThis.fetch;
globalThis.fetch = async () => {
  fetchCalls++;
  throw new TypeError("Failed to fetch");
};
try {
  const request = {
    method: "POST",
    url: "https://unsafe.example/upload",
    headers: [],
    body: encoder.encode("once"),
  };
  await RV64.prototype.performHttp.call(fakeVm, 8n, request, new Uint8Array([1]));
  fakeVm.httpRelayOrigins.add("https://unsafe.example");
  await RV64.prototype.performHttp.call(fakeVm, 9n, request, new Uint8Array([1]));
} finally {
  globalThis.fetch = realFetch;
}
const safeRouting = fetchCalls === 1 && relayCalls === 1 && failures === 1;
console.log(
  `${safeRouting ? "PASS" : "FAIL"} HTTP relay unsafe-method routing — ` +
    `fetch=${fetchCalls} relay=${relayCalls} failures=${failures}`,
);

const closed = new Promise((resolve) => {
  socket.onclose = resolve;
});
socket.close();
await closed;
await relay.close();
await new Promise((resolve) => origin.close(resolve));
process.exit(ok && safeRouting ? 0 : 1);
