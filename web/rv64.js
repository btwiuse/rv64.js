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
