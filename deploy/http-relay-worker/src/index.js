const MAGIC = new Uint8Array([0x52, 0x48, 0x52, 0x31]); // RHR1
const REQUEST = 1;
const HEAD = 2;
const BODY = 3;
const END = 4;
const ERROR = 5;
const MAX_REQUEST_BYTES = 2 * 1024 * 1024;
const MAX_REDIRECTS = 5;

const FORBIDDEN_HEADERS = new Set([
  "host", "content-length", "connection", "keep-alive", "transfer-encoding",
  "upgrade", "te", "trailer", "expect", "proxy-connection",
  "proxy-authorization", "proxy-authenticate", "forwarded", "via",
  "x-forwarded-for", "x-forwarded-host", "x-forwarded-proto", "cf-connecting-ip",
]);

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function csv(value) {
  return new Set(String(value ?? "").split(",").map(value => value.trim()).filter(Boolean));
}

export function originAllowed(origin, configured) {
  if (!origin) return false;
  let url;
  try { url = new URL(origin); } catch { return false; }
  if (url.origin !== origin) return false;
  for (const entry of csv(configured)) {
    if (entry === origin) return true;
    // A port-less localhost entry permits arbitrary development-server ports.
    try {
      const allowed = new URL(entry);
      if (!allowed.port && allowed.protocol === url.protocol && allowed.hostname === url.hostname) {
        return true;
      }
    } catch { /* invalid configuration entries never match */ }
  }
  return false;
}

function upstreamAllowed(url, configured) {
  return url.protocol === "https:" && !url.username && !url.password && csv(configured).has(url.hostname);
}

function protocolFrame(type, id, payload = new Uint8Array()) {
  const body = payload instanceof Uint8Array ? payload : new Uint8Array(payload);
  const out = new Uint8Array(16 + body.length);
  out.set(MAGIC);
  out[4] = type;
  new DataView(out.buffer).setBigUint64(8, BigInt(id), true);
  out.set(body, 16);
  return out;
}

function decodeProtocolFrame(bytes) {
  if (bytes.length < 16 || MAGIC.some((byte, index) => bytes[index] !== byte)) {
    throw new Error("malformed HTTP relay frame");
  }
  return {
    type: bytes[4],
    id: new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getBigUint64(8, true),
    payload: bytes.subarray(16),
  };
}

function decodeRequest(bytes) {
  let pos = 0;
  const u32 = () => {
    if (pos + 4 > bytes.length) throw new Error("truncated request");
    const value = new DataView(bytes.buffer, bytes.byteOffset + pos, 4).getUint32(0, true);
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
  const text = () => decoder.decode(field());
  const method = text();
  const url = text();
  const count = u32();
  if (count > 256) throw new Error("too many request headers");
  const headers = [];
  for (let index = 0; index < count; index++) headers.push([text(), text()]);
  const body = field();
  if (pos !== bytes.length) throw new Error("trailing request data");
  return { method, url, headers, body };
}

function encodeHead(status, headers) {
  const fields = headers.map(([name, value]) => [encoder.encode(name), encoder.encode(value)]);
  const length = 8 + fields.reduce((sum, [name, value]) => sum + 8 + name.length + value.length, 0);
  const out = new Uint8Array(length);
  const view = new DataView(out.buffer);
  let pos = 0;
  view.setUint32(pos, status, true); pos += 4;
  view.setUint32(pos, fields.length, true); pos += 4;
  for (const [name, value] of fields) {
    view.setUint32(pos, name.length, true); pos += 4;
    out.set(name, pos); pos += name.length;
    view.setUint32(pos, value.length, true); pos += 4;
    out.set(value, pos); pos += value.length;
  }
  return out;
}

async function messageBytes(data) {
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  if (typeof data?.arrayBuffer === "function") return new Uint8Array(await data.arrayBuffer());
  throw new Error("binary WebSocket messages required");
}

async function allowedFetch(request, env, signal) {
  let url = new URL(request.url);
  const headers = new Headers();
  for (const [name, value] of request.headers) {
    if (!FORBIDDEN_HEADERS.has(name.toLowerCase())) headers.append(name, value);
  }
  for (let redirects = 0; ; redirects++) {
    if (!upstreamAllowed(url, env.ALLOWED_HOSTS)) throw new Error(`upstream is not allowed: ${url.hostname}`);
    const response = await fetch(url, {
      method: request.method,
      headers,
      body: request.body.length ? request.body : undefined,
      redirect: "manual",
      signal,
    });
    if (![301, 302, 303, 307, 308].includes(response.status)) return response;
    if (redirects >= MAX_REDIRECTS) throw new Error("too many redirects");
    const location = response.headers.get("location");
    if (!location) return response;
    await response.body?.cancel();
    url = new URL(location, url);
  }
}

async function perform(socket, frame, env, controllers) {
  const controller = new AbortController();
  controllers.set(frame.id, controller);
  let headSent = false;
  try {
    const request = decodeRequest(frame.payload);
    if (!new Set(["GET", "HEAD"]).has(request.method.toUpperCase())) {
      throw new Error("only GET and HEAD are allowed");
    }
    const response = await allowedFetch(request, env, controller.signal);
    socket.send(protocolFrame(HEAD, frame.id, encodeHead(response.status, [...response.headers])));
    headSent = true;
    const reader = response.body?.getReader();
    if (reader) {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        if (value?.length) socket.send(protocolFrame(BODY, frame.id, value));
      }
    }
    socket.send(protocolFrame(END, frame.id));
  } catch (cause) {
    const message = String(cause?.message ?? cause);
    try {
      socket.send(protocolFrame(ERROR, frame.id, encoder.encode(headSent ? `upstream response interrupted: ${message}` : message)));
    } catch { /* client disconnected */ }
  } finally {
    controllers.delete(frame.id);
  }
}

export function attachRelay(socket, env) {
  const controllers = new Map();
  const maximum = Math.max(1, Math.min(128, Number(env.MAX_CONCURRENT) || 16));
  socket.accept();
  socket.addEventListener("message", async event => {
    try {
      const bytes = await messageBytes(event.data);
      if (bytes.length > MAX_REQUEST_BYTES) throw new Error("request frame is too large");
      const frame = decodeProtocolFrame(bytes);
      if (frame.type !== REQUEST) throw new Error("expected a request frame");
      if (controllers.has(frame.id)) throw new Error("duplicate request id");
      if (controllers.size >= maximum) throw new Error("too many concurrent requests");
      void perform(socket, frame, env, controllers);
    } catch (error) {
      socket.close(1002, String(error?.message ?? error).slice(0, 120));
    }
  });
  const abort = () => {
    for (const controller of controllers.values()) controller.abort();
    controllers.clear();
  };
  socket.addEventListener("close", abort);
  socket.addEventListener("error", abort);
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/health") return new Response("ok\n");
    if (url.pathname !== "/relay") return new Response("not found\n", { status: 404 });
    if (!originAllowed(request.headers.get("origin"), env.ALLOWED_ORIGINS)) {
      return new Response("forbidden origin\n", { status: 403 });
    }
    if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") {
      return new Response("WebSocket upgrade required\n", { status: 426 });
    }
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    attachRelay(server, env);
    return new Response(null, { status: 101, webSocket: client });
  },
};
