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
        // HTTP egress: the core hands an encoded request; the page performs
        // it (fetch in a browser, vm.onHttpRequest override elsewhere) and
        // answers via sys_http_response. Default: hand to the hook if the
        // embedder installed one, otherwise report the request as failed so
        // the guest sees a clean error instead of a hang.
        host_http_request: (id, ptr, len) => {
          const req = new Uint8Array(vm.ex.memory.buffer, ptr, len).slice();
          if (vm.onHttpRequest) {
            vm.onHttpRequest(id, req);
          } else {
            queueMicrotask(() => vm.ex.sys_http_response?.(id));
          }
        },
        host_now_ms: () =>
          typeof performance !== "undefined" ? performance.now() : Date.now(),
        host_random: (ptr, len) => {
          const buf = new Uint8Array(vm.ex.memory.buffer, ptr, len);
          if (globalThis.crypto?.getRandomValues && len <= 65536) {
            crypto.getRandomValues(buf);
          } else {
            for (let i = 0; i < len; i++) buf[i] = (Math.random() * 256) | 0;
          }
        },
        // JIT: instantiate the module the core just emitted (JIT_OUT),
        // register its `run` function in the core's function table, and
        // return the table index for call_indirect dispatch.
        host_jit_register: () => {
          try {
            const bytes = new Uint8Array(
              vm.ex.memory.buffer,
              vm.ex.jit_out_ptr(),
              vm.ex.jit_out_len(),
            ).slice();
            const mod = new WebAssembly.Module(bytes);
            const inst = new WebAssembly.Instance(mod, {
              // tlb_fill: blocks that probe the guest TLB inline call back
              // into the core to walk the page tables on a miss (wasm->wasm,
              // no JS frame) instead of bailing to the interpreter.
              env: {
                memory: vm.ex.memory,
                tlb_fill: vm.ex.jit_tlb_fill,
                __indirect_function_table: vm.ex.__indirect_function_table,
              },
            });
            const table = vm.ex.__indirect_function_table;
            const idx = table.grow(1);
            table.set(idx, inst.exports.run);
            vm.jitBlocks = (vm.jitBlocks ?? 0) + 1;
            return idx;
          } catch (e) {
            console.warn("jit register failed:", e);
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
                  __indirect_function_table: vm.ex.__indirect_function_table,
                },
              }),
            )
            .then((inst) => {
              const table = vm.ex.__indirect_function_table;
              const idx = table.grow(1);
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
      // Available, but DEFAULT OFF: measured on V8 11.3, a cross-instance
      // return_call_indirect costs ~1.2us per hop (each block is its own
      // instance, and the tail call goes through an instance-switching
      // trampoline), which is 20-50x the plain dispatch loop's cost per
      // block. Opt in for engines where cross-instance tail calls are cheap.
      if (globalThis.RV64_TAILCALL || process?.env?.RV_TAILCALL === "1") {
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
}) {
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
