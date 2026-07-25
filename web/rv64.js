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
              env: { memory: vm.ex.memory, tlb_fill: vm.ex.jit_tlb_fill },
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
                env: { memory: vm.ex.memory, tlb_fill: vm.ex.jit_tlb_fill },
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

RV64.prototype.bootLinux = function ({ bios, kernel, disk, cmdline, ramMB = 128 }) {
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
  this.ex.sys_boot(ramMB);
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
