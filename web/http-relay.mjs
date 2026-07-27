#!/usr/bin/env node
// Request-level WebSocket relay for rv64.js's in-browser HTTP proxy.
//
// Browser fetch() is still the first choice. This relay is the optional path
// for responses hidden by CORS. It intentionally does not carry Ethernet
// frames: connectNet() and websockproxy use a separate layer-2 protocol.

import { createHash } from "node:crypto";
import { once } from "node:events";
import { createServer } from "node:http";
import { pathToFileURL } from "node:url";

const MAGIC = Buffer.from("RHR1");
const REQUEST = 1;
const HEAD = 2;
const BODY = 3;
const END = 4;
const ERROR = 5;
const MAX_MESSAGE = 33 * 1024 * 1024;
const MAX_CONCURRENT = 128;

const FORBIDDEN_HEADERS = new Set([
  "host",
  "content-length",
  "connection",
  "keep-alive",
  "transfer-encoding",
  "upgrade",
  "te",
  "trailer",
  "expect",
  "proxy-connection",
  "proxy-authorization",
  "proxy-authenticate",
]);

function protocolFrame(type, id, payload = Buffer.alloc(0)) {
  const body = Buffer.from(payload);
  const out = Buffer.allocUnsafe(16 + body.length);
  MAGIC.copy(out, 0);
  out[4] = type;
  out.fill(0, 5, 8);
  out.writeBigUInt64LE(BigInt(id), 8);
  body.copy(out, 16);
  return out;
}

function decodeProtocolFrame(bytes) {
  if (bytes.length < 16 || !bytes.subarray(0, 4).equals(MAGIC)) {
    throw new Error("malformed HTTP relay frame");
  }
  return {
    type: bytes[4],
    id: bytes.readBigUInt64LE(8),
    payload: bytes.subarray(16),
  };
}

function decodeRequest(bytes) {
  let pos = 0;
  const u32 = () => {
    if (pos + 4 > bytes.length) throw new Error("truncated request");
    const value = bytes.readUInt32LE(pos);
    pos += 4;
    return value;
  };
  const field = () => {
    const length = u32();
    if (pos + length > bytes.length) throw new Error("truncated request field");
    const value = bytes.subarray(pos, pos + length);
    pos += length;
    return value;
  };
  const text = () => new TextDecoder().decode(field());

  const method = text();
  const url = text();
  const count = u32();
  if (count > 4096) throw new Error("too many request headers");
  const headers = [];
  for (let index = 0; index < count; index++) {
    headers.push([text(), text()]);
  }
  const body = field();
  if (pos !== bytes.length) throw new Error("trailing request data");
  return { method, url, headers, body };
}

function encodeHead(status, headers) {
  const encoder = new TextEncoder();
  const fields = headers.map(([name, value]) => [
    encoder.encode(name),
    encoder.encode(value),
  ]);
  const length =
    8 +
    fields.reduce(
      (total, [name, value]) => total + 8 + name.length + value.length,
      0,
    );
  const out = Buffer.allocUnsafe(length);
  let pos = 0;
  out.writeUInt32LE(status, pos);
  pos += 4;
  out.writeUInt32LE(fields.length, pos);
  pos += 4;
  for (const [name, value] of fields) {
    out.writeUInt32LE(name.length, pos);
    pos += 4;
    Buffer.from(name).copy(out, pos);
    pos += name.length;
    out.writeUInt32LE(value.length, pos);
    pos += 4;
    Buffer.from(value).copy(out, pos);
    pos += value.length;
  }
  return out;
}

function webSocketFrame(payload, opcode = 2) {
  const body = Buffer.from(payload);
  let header;
  if (body.length < 126) {
    header = Buffer.from([0x80 | opcode, body.length]);
  } else if (body.length <= 0xffff) {
    header = Buffer.allocUnsafe(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(body.length, 2);
  } else {
    header = Buffer.allocUnsafe(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(body.length), 2);
  }
  return Buffer.concat([header, body]);
}

async function writeSocket(socket, bytes) {
  if (socket.destroyed) throw new Error("relay client disconnected");
  if (socket.write(bytes)) return;
  await Promise.race([
    once(socket, "drain"),
    once(socket, "close").then(() => {
      throw new Error("relay client disconnected");
    }),
  ]);
}

async function sendMessage(socket, type, id, payload) {
  await writeSocket(socket, webSocketFrame(protocolFrame(type, id, payload)));
}

function localOrigin(origin) {
  if (!origin) return true; // non-browser clients used by diagnostics/tests
  try {
    const url = new URL(origin);
    return (
      (url.protocol === "http:" || url.protocol === "https:") &&
      ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)
    );
  } catch {
    return false;
  }
}

async function performRequest(socket, id, bytes, fetchImpl, controllers) {
  const controller = new AbortController();
  controllers.set(id, controller);
  let headSent = false;
  try {
    const request = decodeRequest(bytes);
    const url = new URL(request.url);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      throw new Error(`unsupported URL scheme: ${url.protocol}`);
    }
    const headers = new Headers();
    for (const [name, value] of request.headers) {
      if (!FORBIDDEN_HEADERS.has(name.toLowerCase())) headers.append(name, value);
    }
    const init = {
      method: request.method,
      headers,
      redirect: "follow",
      signal: controller.signal,
    };
    if (request.body.length) init.body = request.body;

    const response = await fetchImpl(url, init);
    await sendMessage(
      socket,
      HEAD,
      id,
      encodeHead(response.status, [...response.headers]),
    );
    headSent = true;
    const reader = response.body?.getReader();
    if (reader) {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        if (value?.length) await sendMessage(socket, BODY, id, value);
      }
    }
    await sendMessage(socket, END, id);
  } catch (cause) {
    const message = String(cause?.message ?? cause);
    try {
      await sendMessage(
        socket,
        ERROR,
        id,
        new TextEncoder().encode(
          headSent ? `upstream response interrupted: ${message}` : message,
        ),
      );
    } catch {
      // The client is already gone.
    }
  } finally {
    controllers.delete(id);
  }
}

function attachWebSocket(socket, initial, fetchImpl) {
  let pending = Buffer.from(initial);
  const controllers = new Map();

  const close = () => {
    for (const controller of controllers.values()) controller.abort();
    controllers.clear();
  };
  socket.on("close", close);
  socket.on("error", close);

  const drain = () => {
    for (;;) {
      if (pending.length < 2) return;
      const first = pending[0];
      const second = pending[1];
      const final = (first & 0x80) !== 0;
      const opcode = first & 0x0f;
      const masked = (second & 0x80) !== 0;
      let length = second & 0x7f;
      let pos = 2;
      if (length === 126) {
        if (pending.length < 4) return;
        length = pending.readUInt16BE(2);
        pos = 4;
      } else if (length === 127) {
        if (pending.length < 10) return;
        const wide = pending.readBigUInt64BE(2);
        if (wide > BigInt(MAX_MESSAGE)) {
          socket.destroy();
          return;
        }
        length = Number(wide);
        pos = 10;
      }
      if (!masked || length > MAX_MESSAGE) {
        socket.destroy();
        return;
      }
      if (pending.length < pos + 4 + length) return;
      const mask = pending.subarray(pos, pos + 4);
      pos += 4;
      const payload = Buffer.from(pending.subarray(pos, pos + length));
      for (let index = 0; index < payload.length; index++) {
        payload[index] ^= mask[index & 3];
      }
      pending = pending.subarray(pos + length);

      if (opcode === 8) {
        socket.end(webSocketFrame(Buffer.alloc(0), 8));
        return;
      }
      if (opcode === 9) {
        socket.write(webSocketFrame(payload.subarray(0, 125), 10));
        continue;
      }
      if (opcode !== 2 || !final) {
        socket.destroy();
        return;
      }
      let message;
      try {
        message = decodeProtocolFrame(payload);
        if (message.type !== REQUEST) throw new Error("expected a request frame");
        if (controllers.has(message.id)) throw new Error("duplicate request id");
        if (controllers.size >= MAX_CONCURRENT) throw new Error("too many requests");
      } catch (error) {
        socket.destroy(error);
        return;
      }
      void performRequest(
        socket,
        message.id,
        message.payload,
        fetchImpl,
        controllers,
      );
    }
  };

  socket.on("data", (bytes) => {
    pending = Buffer.concat([pending, bytes]);
    drain();
  });
  drain();
}

/**
 * Start a loopback HTTP relay. Remote browser origins must be explicitly
 * allowed because WebSockets are not protected by browser CORS.
 */
export async function startHttpRelay({
  host = "127.0.0.1",
  port = 0,
  allowedOrigins = [],
  fetchImpl = globalThis.fetch,
} = {}) {
  if (!fetchImpl) throw new Error("fetch is unavailable");
  const allowed = new Set(allowedOrigins);
  const server = createServer((_request, response) => {
    response.writeHead(426, { "Content-Type": "text/plain" });
    response.end("rv64.js HTTP relay: WebSocket upgrade required\n");
  });
  server.on("upgrade", (request, socket, head) => {
    const origin = request.headers.origin ?? "";
    if (!localOrigin(origin) && !allowed.has(origin)) {
      socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n");
      return;
    }
    const key = request.headers["sec-websocket-key"];
    if (
      request.headers.upgrade?.toLowerCase() !== "websocket" ||
      typeof key !== "string"
    ) {
      socket.end("HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
      return;
    }
    const accept = createHash("sha1")
      .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
      .digest("base64");
    socket.write(
      "HTTP/1.1 101 Switching Protocols\r\n" +
        "Upgrade: websocket\r\n" +
        "Connection: Upgrade\r\n" +
        `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
    );
    attachWebSocket(socket, head, fetchImpl);
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, resolve);
  });
  const address = server.address();
  const displayHost = address.address.includes(":")
    ? `[${address.address}]`
    : address.address;
  return {
    server,
    url: `ws://${displayHost}:${address.port}`,
    close: () =>
      new Promise((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      ),
  };
}

function parseCli(argv) {
  const options = { allowedOrigins: [] };
  for (let index = 0; index < argv.length; index++) {
    switch (argv[index]) {
      case "--host":
        options.host = argv[++index];
        break;
      case "--port":
        options.port = Number(argv[++index]);
        break;
      case "--allow-origin":
        options.allowedOrigins.push(argv[++index]);
        break;
      default:
        throw new Error(
          "usage: node web/http-relay.mjs [--host ADDRESS] [--port PORT] " +
            "[--allow-origin ORIGIN]",
        );
    }
  }
  return options;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const relay = await startHttpRelay(parseCli(process.argv.slice(2)));
    console.log(`rv64.js HTTP relay listening on ${relay.url}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 2;
  }
}
