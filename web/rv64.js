// rv64.js — browser/Node loader for the rv64-wasm module.
//
// v86-style: talks to plain extern "C" exports over wasm linear memory.
// No bundler, no wasm-bindgen glue; works as an ES module in browsers and Node.

export const Stop = Object.freeze({
  BUDGET: 0,
  ECALL: 1,
  BREAK: 2,
  TRAP: 3,
  EXITED: 4,
});

export class RV64 {
  /** @param {WebAssembly.Instance} instance */
  constructor(instance) {
    this.ex = instance.exports;
    /** Override to capture guest console output: (fd, Uint8Array) => void */
    this.onWrite = (fd, bytes) => {
      const text = new TextDecoder().decode(bytes);
      (fd === 2 ? console.error : console.log)(text);
    };
    /** Called with each Ethernet frame the guest sends; set by connectNet. */
    this.onNetSend = () => {};
    /** Optional request-level WebSocket fallback configured by connectHttpRelay. */
    this.httpRelay = null;
    /** Origins known to require the relay, avoiding a failed fetch every time. */
    this.httpRelayOrigins = new Set();
  }

  /** Instantiate from wasm bytes (ArrayBuffer/TypedArray/Response). */
  static async create(wasmSource) {
    let vm;
    const imports = {
      env: {
        host_write: (fd, ptr, len) => {
          // Copy out: the view dies if wasm memory grows.
          const bytes = new Uint8Array(vm.ex.memory.buffer, ptr, len).slice();
          vm.onWrite(fd, bytes);
        },
        // One Ethernet frame the guest transmitted. Goes straight out the
        // relay socket — one binary message per frame, websockproxy's protocol.
        host_net_send: (ptr, len) => {
          const frame = new Uint8Array(vm.ex.memory.buffer, ptr, len).slice();
          vm.onNetSend(frame);
        },
        // HTTP egress for the guest's in-process proxy. Performed with fetch(),
        // the browser's only egress primitive — and the reason the proxy design
        // reaches the network with no external infrastructure. An embedder can
        // intercept instead by setting `onHttpRequest`.
        host_http_request: (id, ptr, len) => {
          const bytes = new Uint8Array(vm.ex.memory.buffer, ptr, len).slice();
          if (vm.onHttpRequest) vm.onHttpRequest(id, bytes);
          else vm.performHttp(id, decodeRequest(bytes), bytes);
        },
        host_now_ms: () =>
          typeof performance !== "undefined" ? performance.now() : Date.now(),
        host_unix_ms: () => Date.now(),
        host_random: (ptr, len) => {
          const buf = new Uint8Array(vm.ex.memory.buffer, ptr, len);
          if (!globalThis.crypto?.getRandomValues) {
            throw new Error("cryptographic randomness is unavailable");
          }
          // Web Crypto caps one getRandomValues call at 65536 bytes.
          for (let off = 0; off < len; off += 65536) {
            crypto.getRandomValues(buf.subarray(off, Math.min(off + 65536, len)));
          }
        },
        // JIT: instantiate the module the core just emitted (JIT_OUT),
        // register its `run` function in the core's function table, and
        // return the table index for call_indirect dispatch.
        host_jit_register: () => {
          try {
            const t0 = performance.now();
            const bytes = new Uint8Array(
              vm.ex.memory.buffer,
              vm.ex.jit_out_ptr(),
              vm.ex.jit_out_len(),
            ).slice();
            vm.jitRegCount = (vm.jitRegCount ?? 0) + 1;
            vm.jitRegBytes = (vm.jitRegBytes ?? 0) + bytes.length;
            const mod = new WebAssembly.Module(bytes);
            vm.jitRegMs = (vm.jitRegMs ?? 0) + (performance.now() - t0);
            const inst = new WebAssembly.Instance(mod, {
              // tlb_fill: blocks that probe the guest TLB inline call back
              // into the core to walk the page tables on a miss (wasm->wasm,
              // no JS frame) instead of bailing to the interpreter.
              env: {
                memory: vm.ex.memory,
                tlb_fill: vm.ex.jit_tlb_fill,
                chain_next: vm.ex.chain_next,
                __indirect_function_table: vm.ex.__indirect_function_table,
              },
            });
            const table = vm.ex.__indirect_function_table;
            // Bulk pre-growth: growing a shared table forces V8 to rewire
            // EVERY instance that imports it, so one grow(1) per block was
            // O(instances) each — quadratic across a workload like tcc
            // (7.5k chain-bearing modules), and the reason every chaining
            // configuration measured 2-3x slower there. Grow in 4096-slot
            // steps and hand out indices from a cursor instead.
            vm.tableNext ??= table.length;
            if (vm.tableNext >= table.length) table.grow(4096);
            const idx = vm.tableNext++;
            table.set(idx, inst.exports.run);
            vm.jitRegTotalMs = (vm.jitRegTotalMs ?? 0) + (performance.now() - t0);
            vm.jitBlocks = (vm.jitBlocks ?? 0) + 1;
            return idx;
          } catch (e) {
            console.warn("jit register failed:", e);
            return -1;
          }
        },
        // Batch registration: one module carrying N trace bodies that
        // tail-call each other directly. Its exports go into CONTIGUOUS
        // table slots so emitted links can verify `line.idx == base + j`.
        host_jit_register_batch: (n) => {
          try {
            const bytes = new Uint8Array(
              vm.ex.memory.buffer,
              vm.ex.jit_out_ptr(),
              vm.ex.jit_out_len(),
            ).slice();
            const inst = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
              env: { memory: vm.ex.memory, tlb_fill: vm.ex.jit_tlb_fill },
            });
            const table = vm.ex.__indirect_function_table;
            vm.tableNext ??= table.length;
            if (vm.tableNext + n > table.length) table.grow(Math.max(4096, n));
            const base = vm.tableNext;
            for (let j = 0; j < n; j++) table.set(base + j, inst.exports["r" + j]);
            vm.tableNext += n;
            vm.jitBlocks = (vm.jitBlocks ?? 0) + n;
            vm.jitBatches = (vm.jitBatches ?? 0) + 1;
            return base;
          } catch (e) {
            console.warn("batch jit register failed:", e);
            return -1;
          }
        },
        // Async variant for LARGE modules (page superblocks): compiles on
        // V8's background threads via WebAssembly.compile so guest execution
        // never stalls on a big synchronous Module build. When ready, the
        // function lands in the table and sys_sb_ready(ticket, idx) is
        // invoked — on the JS microtask queue, i.e. strictly BETWEEN
        // runSystem calls, never during wasm execution.
        host_jit_register_async: (ticket) => {
          const bytes = new Uint8Array(
            vm.ex.memory.buffer,
            vm.ex.jit_out_ptr(),
            vm.ex.jit_out_len(),
          ).slice();
          WebAssembly.compile(bytes)
            .then((mod) =>
              WebAssembly.instantiate(mod, {
                env: {
                  memory: vm.ex.memory,
                  tlb_fill: vm.ex.jit_tlb_fill,
                  chain_next: vm.ex.chain_next,
                  __indirect_function_table: vm.ex.__indirect_function_table,
                },
              }),
            )
            .then((inst) => {
              const table = vm.ex.__indirect_function_table;
              vm.tableNext ??= table.length;
              if (vm.tableNext >= table.length) table.grow(4096);
              const idx = vm.tableNext++;
              table.set(idx, inst.exports.run);
              vm.jitBlocks = (vm.jitBlocks ?? 0) + 1;
              vm.ex.sys_sb_ready(ticket, idx);
            })
            .catch((e) => {
              console.warn("async jit register failed:", e);
              vm.ex.sys_sb_ready(ticket, -1);
            });
        },
      },
    };
    const { instance } =
      wasmSource instanceof Response || wasmSource instanceof Promise
        ? await WebAssembly.instantiateStreaming(wasmSource, imports)
        : await WebAssembly.instantiate(wasmSource, imports);
    vm = new RV64(instance);
    // Hardware FMA: use f64x2.relaxed_madd for the guest's FMADD family iff
    // the engine validates it AND it is fused on this hardware (the spec
    // allows unfused; only fused is bit-exact). Probe empirically:
    // a=b=1+2^-52, c=-(1+2^-51) gives 2^-104 fused, 0 unfused.
    try {
      const sec = (id, p) => [id, p.length, ...p];
      const code = [0x00,
        0x20, 0, 0xfd, 0x12, 0x20, 1, 0xfd, 0x12, 0x20, 2, 0xfd, 0x12,
        0xfd, 0x87, 0x02, 0xfd, 0x21, 0x00, 0x0b];
      const mod = new WebAssembly.Module(new Uint8Array([
        0, 0x61, 0x73, 0x6d, 1, 0, 0, 0,
        ...sec(1, [1, 0x60, 3, 0x7e, 0x7e, 0x7e, 1, 0x7c]),
        ...sec(3, [1, 0]),
        ...sec(7, [1, 1, 0x74, 0, 0]),
        ...sec(10, [1, code.length, ...code]),
      ]));
      const bits = (x) => new BigInt64Array(new Float64Array([x]).buffer)[0];
      const r = new WebAssembly.Instance(mod, {}).exports.t(
        bits(1 + 2 ** -52), bits(1 + 2 ** -52), bits(-(1 + 2 ** -51)));
      if (r !== 0 && Math.abs(r - 2 ** -104) < 2 ** -150) {
        vm.ex.jit_set_hw_fma?.(1);
      }
    } catch {
      /* no relaxed SIMD: the exact emulated fma stays in charge */
    }
    // Direct block chaining needs wasm tail calls (return_call_indirect,
    // shipped by default in V8 11.2+). Feature-detect with a 1-function probe
    // so older engines just keep the plain dispatch loop.
    try {
      new WebAssembly.Module(new Uint8Array([
        0, 0x61, 0x73, 0x6d, 1, 0, 0, 0,
        1, 5, 1, 0x60, 1, 0x7f, 0,
        2, 11, 1, 1, 0x65, 3, 0x74, 0x61, 0x62, 0x01, 0x70, 0, 0,
        3, 2, 1, 0,
        10, 11, 1, 9, 0, 0x20, 0, 0x41, 0, 0x13, 0, 0, 0x0b,
      ]));
      // Chaining is DEFAULT OFF after three measured architectures:
      // (1) emitted return_call_indirect — ~2ns/hop on node 20.18.1, but
      // any module importing the shared table makes table.set O(importing
      // instances): quadratic registration for tcc/CPython populations;
      // (2) per-module shared helper — same import, same quadratic;
      // (3) env.chain_next, a host-module Rust dispatch reached as a
      // function import (no table import, no quadratic) — measured SLOWER
      // everywhere (nbench ASSIGNMENT 8.3 -> 6.2, python 4.6 -> 6.2s):
      // the host dispatch loop is already wasm with no JS frame, so the
      // sandwich (block -> host Rust -> block) re-does the loop's own
      // bookkeeping plus two extra call frames per hop. The per-dispatch
      // cost is the bookkeeping itself, not a boundary. RV_TAILCALL=1
      // re-enables for experiments.
      if (process?.env?.RV_TAILCALL === "1") {
        vm.ex.jit_set_tailcall?.(1);
      }
    } catch {
      /* no tail calls: chaining stays off */
    }
    return vm;
  }

  // ---- user-mode Linux API ----

  /** Copy bytes into the wasm staging buffer. */
  #stage(bytes) {
    const ptr = this.ex.staging_alloc(bytes.length);
    new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
  }

  /**
   * Load a static riscv64 Linux ELF (Uint8Array) with the given argv.
   * @returns {boolean} success
   */
  loadElf(elfBytes, argv = ["guest"], memMB = 256) {
    const enc = new TextEncoder();
    for (const arg of argv) {
      this.#stage(enc.encode(arg));
      this.ex.user_arg_push();
    }
    this.#stage(elfBytes);
    return this.ex.user_load(memMB << 20) === 0;
  }

  /** Run the loaded program; returns a Stop.* code (EXITED when done). */
  runUser(budget = 10_000_000_000n) {
    return this.ex.user_run(BigInt(budget));
  }

  userExitCode() {
    return this.ex.user_exit_code();
  }
  userInsnCount() {
    return this.ex.user_insn_count();
  }
  userPc() {
    return this.ex.user_pc();
  }

  /** Allocate guest RAM at guest address `base` and reset the CPU (pc = base). */
  init(base, size) {
    this.ex.init(BigInt(base), size);
  }

  /** Guest RAM as a Uint8Array view into wasm linear memory.
   *  NOTE: invalidated if wasm memory grows — take a fresh view per use. */
  ram() {
    return new Uint8Array(
      this.ex.memory.buffer,
      this.ex.mem_ptr(),
      this.ex.mem_size(),
    );
  }

  /** Copy a program (Uint8Array) into guest RAM at guest address `addr`. */
  load(addr, bytes) {
    const base = Number(this.ex.get_pc()); // pc === base right after init
    this.ram().set(bytes, addr - base);
  }

  /** Run up to `budget` instructions; returns a Stop.* code. */
  run(budget = 1_000_000n) {
    return this.ex.run(BigInt(budget));
  }

  get pc() {
    return this.ex.get_pc();
  }
  set pc(v) {
    this.ex.set_pc(BigInt(v));
  }

  reg(i) {
    return this.ex.get_reg(i);
  }
  setReg(i, v) {
    this.ex.set_reg(i, BigInt(v));
  }

  trapCause() {
    return this.ex.trap_cause();
  }
  insnCount() {
    return this.ex.insn_count();
  }

  /** Dump architectural state (for debugging / differential testing). */
  state() {
    const x = [];
    for (let i = 0; i < 32; i++) x.push(this.reg(i));
    return { pc: this.pc, x };
  }
}

// ---- full-system (boot Linux) API — appended to class via prototype ----

/**
 * Boot Linux.
 *
 * `fsTar`/`fsTag` export a tar archive as a virtio-9p filesystem the guest can
 * mount (`mount -t 9p -o trans=virtio,version=9p2000.L <fsTag> /mnt`); there is
 * no host filesystem in a browser, so the export is an in-memory tree.
 * `net: true` adds a NIC — call `connectNet(url)` to give its frames somewhere
 * to go.
 */
RV64.prototype.bootLinux = function ({
  bios,
  kernel,
  disk,
  cmdline,
  ramMB = 128,
  fsTar,
  fsTag,
  net = false,
  netMac,
  proxy = false,
  proxyUpgradeHttps = true,
}) {
  if (proxy && fsTar && (fsTag || "host") === "rv64-proxy") {
    throw new Error("fsTag 'rv64-proxy' is reserved for the proxy CA");
  }
  const stage = (bytes, fn) => {
    if (!bytes) return;
    const ptr = this.ex.staging_alloc(bytes.length);
    new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
    fn();
  };
  stage(bios, () => this.ex.sys_stage_bios());
  stage(kernel, () => this.ex.sys_stage_kernel());
  stage(disk, () => this.ex.sys_stage_disk());
  if (cmdline) stage(new TextEncoder().encode(cmdline), () => this.ex.sys_stage_cmdline());
  stage(fsTar, () => this.ex.sys_stage_fs_tar());
  if (fsTag) stage(new TextEncoder().encode(fsTag), () => this.ex.sys_stage_fs_tag());
  if (netMac) stage(new Uint8Array(netMac), () => this.ex.sys_stage_net_mac());
  this.ex.sys_net_enable(net ? 1 : 0);
  // The proxy implies a NIC: the guest reaches it over ordinary TCP.
  if (proxy) this.ex.sys_proxy_enable(1, proxyUpgradeHttps ? 1 : 0);
  this.ex.sys_boot(ramMB);
};

/**
 * Attach the guest's NIC to a WebSocket relay: one binary message per Ethernet
 * frame (websockproxy / v86's protocol). Returns the socket.
 *
 * The relay is what makes guest networking possible without privileges — it
 * holds them and does the NAT, so the emulator stays a pure layer-2 device.
 */
RV64.prototype.connectNet = function (url) {
  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  // Frames sent before the socket opens would throw; drop them the way a NIC
  // drops frames on a down link.
  this.onNetSend = (frame) => {
    if (ws.readyState === WebSocket.OPEN) ws.send(frame);
  };
  ws.onmessage = (ev) => {
    if (typeof ev.data === "string") return; // not our protocol
    this.netInput(new Uint8Array(ev.data));
  };
  this.net = ws;
  return ws;
};

/** Deliver one inbound Ethernet frame to the guest's NIC. */
RV64.prototype.netInput = function (frame) {
  const ptr = this.ex.staging_alloc(frame.length);
  new Uint8Array(this.ex.memory.buffer, ptr, frame.length).set(frame);
  this.ex.sys_net_input();
};

// ---- request-level WebSocket relay ---------------------------------------
//
// This is deliberately separate from connectNet's Ethernet-frame relay. Once
// the in-process proxy has terminated a guest connection, its parsed HTTP
// request cannot be handed to a layer-2 websockproxy socket. The protocol here
// preserves the request-shaped boundary and lets a small host relay perform
// requests that browser fetch() cannot read because of CORS.

const HTTP_RELAY_MAGIC = [0x52, 0x48, 0x52, 0x31]; // "RHR1"
const HTTP_RELAY_REQUEST = 1;
const HTTP_RELAY_HEAD = 2;
const HTTP_RELAY_BODY = 3;
const HTTP_RELAY_END = 4;
const HTTP_RELAY_ERROR = 5;
const HTTP_RELAY_SAFE_FALLBACK = new Set(["GET", "HEAD"]);

function httpRelayFrame(type, id, payload = new Uint8Array()) {
  const body =
    payload instanceof Uint8Array
      ? payload
      : new Uint8Array(payload.buffer ?? payload);
  const out = new Uint8Array(16 + body.length);
  out.set(HTTP_RELAY_MAGIC, 0);
  out[4] = type;
  new DataView(out.buffer).setBigUint64(8, BigInt(id), true);
  out.set(body, 16);
  return out;
}

function decodeHttpRelayFrame(bytes) {
  if (
    bytes.length < 16 ||
    HTTP_RELAY_MAGIC.some((byte, index) => bytes[index] !== byte)
  ) {
    throw new Error("malformed HTTP relay frame");
  }
  return {
    type: bytes[4],
    id: new DataView(
      bytes.buffer,
      bytes.byteOffset,
      bytes.byteLength,
    ).getBigUint64(8, true),
    payload: bytes.subarray(16),
  };
}

function requestOrigin(url) {
  try {
    return new URL(url).origin;
  } catch {
    return "";
  }
}

/**
 * Attach an optional request-level WebSocket relay used when fetch() is
 * rejected before a response head (normally CORS or mixed-content policy).
 *
 * Automatic retry is limited to GET/HEAD because a rejected fetch may still
 * have delivered a non-idempotent request. Once a safe request proves an
 * origin needs the relay, later requests to that origin route there directly.
 * Call routeHttpViaRelay(origin) to opt an origin in before its first request.
 */
RV64.prototype.connectHttpRelay = function (url, options = {}) {
  const WebSocketImpl = options.WebSocket ?? globalThis.WebSocket;
  if (!WebSocketImpl) throw new Error("WebSocket is unavailable");
  this.disconnectHttpRelay();

  const ws = new WebSocketImpl(url);
  ws.binaryType = "arraybuffer";
  const pending = new Set();
  let opened = false;
  let resolveReady;
  let rejectReady;
  const ready = new Promise((resolve, reject) => {
    resolveReady = resolve;
    rejectReady = reject;
  });
  // performHttp awaits this, but suppress an unhandled rejection when an
  // embedder connects a relay before it has any proxy traffic.
  ready.catch(() => {});

  const relay = {
    ws,
    pending,
    ready,
    receive: Promise.resolve(),
  };
  this.httpRelay = relay;
  for (const origin of options.origins ?? []) {
    this.routeHttpViaRelay(origin);
  }

  const timeout = setTimeout(() => {
    if (!opened) {
      rejectReady(new Error(`HTTP relay connection timed out: ${url}`));
      try {
        ws.close();
      } catch {
        // A custom WebSocket implementation may already be closed.
      }
    }
  }, options.timeoutMs ?? 10_000);

  const failPending = (reason) => {
    for (const id of pending) {
      this.stageFor(new TextEncoder().encode(reason));
      this.ex.sys_http_fail(id);
    }
    pending.clear();
  };
  ws.onopen = () => {
    opened = true;
    clearTimeout(timeout);
    resolveReady(ws);
  };
  ws.onerror = () => {
    if (!opened) {
      clearTimeout(timeout);
      rejectReady(new Error(`HTTP relay connection failed: ${url}`));
    }
  };
  ws.onclose = () => {
    clearTimeout(timeout);
    if (!opened) rejectReady(new Error(`HTTP relay closed before opening: ${url}`));
    failPending("HTTP relay connection closed");
    if (this.httpRelay === relay) this.httpRelay = null;
  };
  ws.onmessage = (event) => {
    // Preserve WebSocket message order even when a browser supplies Blob data.
    relay.receive = relay.receive
      .then(async () => {
        const data =
          typeof Blob !== "undefined" && event.data instanceof Blob
            ? await event.data.arrayBuffer()
            : event.data;
        const message = decodeHttpRelayFrame(
          data instanceof Uint8Array ? data : new Uint8Array(data),
        );
        if (!pending.has(message.id)) return;
        switch (message.type) {
          case HTTP_RELAY_HEAD:
            this.stageFor(message.payload);
            this.ex.sys_http_head(message.id);
            break;
          case HTTP_RELAY_BODY:
            this.stageFor(message.payload);
            this.ex.sys_http_body(message.id);
            break;
          case HTTP_RELAY_END:
            pending.delete(message.id);
            this.ex.sys_http_end(message.id);
            break;
          case HTTP_RELAY_ERROR:
            pending.delete(message.id);
            this.stageFor(message.payload);
            this.ex.sys_http_fail(message.id);
            break;
          default:
            throw new Error(`unknown HTTP relay message type ${message.type}`);
        }
      })
      .catch((error) => {
        failPending(`HTTP relay protocol error: ${error.message}`);
        try {
          ws.close(1002, "protocol error");
        } catch {
          // Already closed.
        }
      });
  };
  return ws;
};

RV64.prototype.disconnectHttpRelay = function () {
  const relay = this.httpRelay;
  this.httpRelay = null;
  if (relay) {
    try {
      relay.ws.close();
    } catch {
      // Already closed.
    }
  }
};

/** Route an origin directly through the request relay, or remove that choice. */
RV64.prototype.routeHttpViaRelay = function (origin, enabled = true) {
  const normalized = requestOrigin(origin) || origin.replace(/\/+$/, "");
  if (enabled) this.httpRelayOrigins.add(normalized);
  else this.httpRelayOrigins.delete(normalized);
};

RV64.prototype.performHttpViaRelay = async function (id, encodedRequest) {
  const relay = this.httpRelay;
  if (!relay) throw new Error("HTTP relay is not connected");
  await relay.ready;
  if (relay.ws.readyState !== 1) throw new Error("HTTP relay is not open");
  relay.pending.add(BigInt(id));
  try {
    relay.ws.send(httpRelayFrame(HTTP_RELAY_REQUEST, id, encodedRequest));
  } catch (error) {
    relay.pending.delete(BigInt(id));
    throw error;
  }
};

/** Run a slice of the booted system. Returns true when powered off. */
RV64.prototype.runSystem = function (maxInsns = 10_000_000n) {
  return this.ex.sys_run(BigInt(maxInsns)) === 1;
};

/** Send keyboard input to the guest console. */
RV64.prototype.consoleInput = function (bytes) {
  const ptr = this.ex.staging_alloc(bytes.length);
  new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
  this.ex.sys_console_input();
};

RV64.prototype.sysInsnCount = function () {
  return this.ex.sys_insn_count();
};

// ---- in-process HTTP proxy ------------------------------------------------
//
// The guest points http_proxy at the emulated network's gateway and speaks
// ordinary HTTP to it; the Rust side terminates TCP and parses the request, and
// this performs it with fetch(). Nothing external is involved.
//
// Reachability is bounded by CORS unless connectHttpRelay() has attached the
// optional request relay. fetch remains the zero-infrastructure fast path.

/** The http_proxy URL to set in the guest, or "" when the proxy is off. */
RV64.prototype.proxyURL = function () {
  const len = this.ex.sys_proxy_url();
  if (!len) return "";
  // staging_ptr, not staging_alloc: the latter is the write path and clears the
  // buffer, which would wipe the value we are trying to read.
  return new TextDecoder().decode(
    new Uint8Array(this.ex.memory.buffer, this.ex.staging_ptr(), len),
  );
};

/** Headers fetch() refuses to let a page set; forwarding them throws or is ignored. */
const FORBIDDEN_HEADERS = new Set([
  "host", "content-length", "connection", "keep-alive", "transfer-encoding",
  "upgrade", "te", "trailer", "expect", "date", "origin", "referer", "via",
  "cookie", "cookie2", "accept-charset", "accept-encoding", "dnt",
  // Not CORS-safelisted: forwarding the guest's User-Agent would force a
  // preflight on every otherwise-simple request, so drop it and let the browser
  // send its own.
  "user-agent",
]);

/** Perform one guest request, streaming the response back as it arrives. */
RV64.prototype.performHttp = async function (id, req, encodedRequest) {
  const origin = requestOrigin(req.url);
  if (
    encodedRequest &&
    this.httpRelay &&
    this.httpRelayOrigins.has(origin)
  ) {
    try {
      await this.performHttpViaRelay(id, encodedRequest);
    } catch (error) {
      this.stageFor(new TextEncoder().encode(String(error?.message ?? error)));
      this.ex.sys_http_fail(BigInt(id));
    }
    return;
  }

  let headSent = false;
  try {
    const headers = {};
    for (const [name, value] of req.headers) {
      if (!FORBIDDEN_HEADERS.has(name.toLowerCase())) headers[name] = value;
    }
    const init = { method: req.method, headers, redirect: "follow" };
    if (req.body.length) init.body = req.body;
    const resp = await fetch(req.url, init);

    this.stageFor(encodeHead(resp.status, [...resp.headers]));
    this.ex.sys_http_head(BigInt(id));
    headSent = true;

    // Stream rather than await the whole body: an SSE response or a long
    // download must reach the guest as it arrives, and nothing is buffered
    // whole on the way past.
    const reader = resp.body?.getReader();
    if (reader) {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        if (value?.length) {
          this.stageFor(value);
          this.ex.sys_http_body(BigInt(id));
        }
      }
    }
    this.ex.sys_http_end(BigInt(id));
  } catch (e) {
    // A CORS rejection occurs before fetch exposes a response head. GET and
    // HEAD are safe to retry; retrying POST could duplicate a request that the
    // origin received even though the browser hid its response.
    if (
      !headSent &&
      encodedRequest &&
      this.httpRelay &&
      HTTP_RELAY_SAFE_FALLBACK.has(req.method.toUpperCase())
    ) {
      try {
        if (origin) this.httpRelayOrigins.add(origin);
        await this.performHttpViaRelay(id, encodedRequest);
        return;
      } catch (relayError) {
        if (origin) this.httpRelayOrigins.delete(origin);
        e = new Error(
          `fetch failed (${String(e?.message ?? e)}); ` +
            `HTTP relay failed (${String(relayError?.message ?? relayError)})`,
        );
      }
    }
    // The proxy turns this into a 502 the guest can read, rather than a silent
    // hang. A CORS rejection lands here.
    this.stageFor(new TextEncoder().encode(String(e?.message ?? e)));
    this.ex.sys_http_fail(BigInt(id));
  }
};

/** Copy bytes into the wasm staging buffer for the next sys_http_* call. */
RV64.prototype.stageFor = function (bytes) {
  const ptr = this.ex.staging_alloc(bytes.length);
  new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
};

/** Mirror of httpproxy::Request::encode. */
function decodeRequest(bytes) {
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let p = 0;
  const u32 = () => {
    const v = dv.getUint32(p, true);
    p += 4;
    return v;
  };
  const buf = () => {
    const n = u32();
    const b = bytes.subarray(p, p + n);
    p += n;
    return b;
  };
  const str = () => new TextDecoder().decode(buf());
  const method = str();
  const url = str();
  const n = u32();
  const headers = [];
  for (let i = 0; i < n; i++) headers.push([str(), str()]);
  return { method, url, headers, body: buf() };
}

/** Mirror of httpproxy::decode_head. Bodies cross as raw bytes, unframed. */
function encodeHead(status, headers) {
  const enc = new TextEncoder();
  const parts = [];
  const u32 = (v) => {
    const b = new Uint8Array(4);
    new DataView(b.buffer).setUint32(0, v, true);
    return b;
  };
  parts.push(u32(status), u32(headers.length));
  for (const [name, value] of headers) {
    const nb = enc.encode(name);
    const vb = enc.encode(value);
    parts.push(u32(nb.length), nb, u32(vb.length), vb);
  }
  const total = parts.reduce((n, b) => n + b.length, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const b of parts) {
    out.set(b, off);
    off += b.length;
  }
  return out;
}
