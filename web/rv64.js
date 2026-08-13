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

// Low-level bindings used by rv64.js itself and the repository's architecture
// and differential tests. This is intentionally not the supported embedding
// API; applications should use RV64 below.
export class RV64Debug {
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
    this.onWispOpen = () => {};
    this.onWispData = () => {};
    this.onWispClose = () => {};
    this.onWispDatagram = () => {};
    /** Request/response byte and error diagnostics for the stable API. */
    this.onNetworkTraffic = () => {};
    /** Optional async external virtio-9P handler: request => reply bytes. */
    this.onP9Request = null;
    /** Number of external 9P requests currently executing in the host. */
    this.p9Pending = 0;
    this.p9Requests = 0;
    this.p9MaxPending = 0;
    this.p9HostMs = 0;
    this.p9RequestBytes = 0;
    this.p9ReplyBytes = 0;
    this.p9ReadRequests = 0;
    this.p9WriteRequests = 0;
    this.p9ReadBytes = 0;
    this.p9WriteBytes = 0;
    this.p9MaxRead = 0;
    this.p9MaxWrite = 0;
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
        host_wisp_open: (id, ptr, port) => {
          const address = new Uint8Array(vm.ex.memory.buffer, ptr, 4).slice();
          vm.onWispOpen(id, address, port);
        },
        host_wisp_data: (id, ptr, len) => {
          const bytes = new Uint8Array(vm.ex.memory.buffer, ptr, len).slice();
          vm.onWispData(id, bytes);
        },
        host_wisp_close: (id) => vm.onWispClose(id),
        host_wisp_datagram: (id, addressPtr, port, dataPtr, len) => {
          const address = new Uint8Array(vm.ex.memory.buffer, addressPtr, 4).slice();
          const bytes = new Uint8Array(vm.ex.memory.buffer, dataPtr, len).slice();
          vm.onWispDatagram(id, address, port, bytes);
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
            const copied = performance.now();
            vm.jitRegCount = (vm.jitRegCount ?? 0) + 1;
            vm.jitRegBytes = (vm.jitRegBytes ?? 0) + bytes.length;
            const mod = new WebAssembly.Module(bytes);
            const compiled = performance.now();
            vm.jitRegMs = (vm.jitRegMs ?? 0) + (compiled - t0);
            const inst = new WebAssembly.Instance(mod, {
              // tlb_fill: blocks that probe the guest TLB inline call back
              // into the core to walk the page tables on a miss (wasm->wasm,
              // no JS frame) instead of bailing to the interpreter.
              env: {
                memory: vm.ex.memory,
                tlb_fill: vm.ex.jit_tlb_fill,
                system_bulk_copy: vm.ex.jit_system_bulk_copy,
                fp_exec: vm.ex.jit_fp_exec,
                user_reservation: vm.ex.jit_user_reservation,
                system_reservation: vm.ex.jit_system_reservation,
                user_vector: vm.ex.jit_user_vector,
                system_vector: vm.ex.jit_system_vector,
                chain_next: vm.ex.chain_next,
                tail_chain: vm.tailChain,
                __indirect_function_table: vm.ex.__indirect_function_table,
              },
            });
            const instantiated = performance.now();
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
            const published = performance.now();
            vm.jitCopyMs = (vm.jitCopyMs ?? 0) + (copied - t0);
            vm.jitCompileMs = (vm.jitCompileMs ?? 0) + (compiled - copied);
            vm.jitInstantiateMs = (vm.jitInstantiateMs ?? 0) +
              (instantiated - compiled);
            vm.jitPublishMs = (vm.jitPublishMs ?? 0) + (published - instantiated);
            vm.jitRegTotalMs = (vm.jitRegTotalMs ?? 0) + (published - t0);
            vm.jitBlocks = (vm.jitBlocks ?? 0) + 1;
            // Capture is opt-in and intentionally happens after lifecycle
            // timing. A capture run is not a workload timing run; frozen
            // replay consumes the retained immutable bytes separately.
            vm.onJitModule?.(bytes, { kind: "single", index: idx });
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
            const t0 = performance.now();
            const bytes = new Uint8Array(
              vm.ex.memory.buffer,
              vm.ex.jit_out_ptr(),
              vm.ex.jit_out_len(),
            ).slice();
            const copied = performance.now();
            vm.jitRegCount = (vm.jitRegCount ?? 0) + 1;
            vm.jitRegBytes = (vm.jitRegBytes ?? 0) + bytes.length;
            const mod = new WebAssembly.Module(bytes);
            const compiled = performance.now();
            const inst = new WebAssembly.Instance(mod, {
              env: {
                memory: vm.ex.memory,
                tlb_fill: vm.ex.jit_tlb_fill,
                system_bulk_copy: vm.ex.jit_system_bulk_copy,
                fp_exec: vm.ex.jit_fp_exec,
                user_reservation: vm.ex.jit_user_reservation,
                system_reservation: vm.ex.jit_system_reservation,
                user_vector: vm.ex.jit_user_vector,
                system_vector: vm.ex.jit_system_vector,
              },
            });
            const instantiated = performance.now();
            const table = vm.ex.__indirect_function_table;
            vm.tableNext ??= table.length;
            if (vm.tableNext + n > table.length) table.grow(Math.max(4096, n));
            const base = vm.tableNext;
            for (let j = 0; j < n; j++) table.set(base + j, inst.exports["r" + j]);
            vm.tableNext += n;
            const published = performance.now();
            vm.jitCopyMs = (vm.jitCopyMs ?? 0) + (copied - t0);
            vm.jitCompileMs = (vm.jitCompileMs ?? 0) + (compiled - copied);
            vm.jitInstantiateMs = (vm.jitInstantiateMs ?? 0) +
              (instantiated - compiled);
            vm.jitPublishMs = (vm.jitPublishMs ?? 0) + (published - instantiated);
            vm.jitRegMs = (vm.jitRegMs ?? 0) + (compiled - t0);
            vm.jitRegTotalMs = (vm.jitRegTotalMs ?? 0) + (published - t0);
            vm.jitBlocks = (vm.jitBlocks ?? 0) + n;
            vm.jitBatches = (vm.jitBatches ?? 0) + 1;
            vm.onJitModule?.(bytes, { kind: "batch", base, members: n });
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
          const t0 = performance.now();
          const bytes = new Uint8Array(
            vm.ex.memory.buffer,
            vm.ex.jit_out_ptr(),
            vm.ex.jit_out_len(),
          ).slice();
          const copied = performance.now();
          const templateCacheAction = Number(
            vm.ex.jit_out_template_cache?.(0) ?? 0n,
          );
          const templateCacheId = BigInt.asUintN(
            64,
            vm.ex.jit_out_template_cache?.(1) ?? 0n,
          ).toString();
          const guestBase = BigInt.asIntN(
            64,
            vm.ex.jit_out_template_cache?.(2) ?? 0n,
          );
          const guestBaseGlobal = templateCacheAction === 0
            ? null
            : new WebAssembly.Global(
                { value: "i64", mutable: false },
                guestBase,
              );
          const pageTemplateClass = Number(
            vm.ex.jit_out_page_template_diag?.(0) ?? 0n,
          );
          const pageTemplateValue = (field) =>
            BigInt.asUintN(64, vm.ex.jit_out_page_template_diag(field));
          const pageTemplateDiagnostic = pageTemplateClass === 0
            ? null
            : {
                ticket: ticket.toString(),
                phase: vm.scorecardPhase ?? "unattributed",
                matchClass: pageTemplateClass,
                currentVirtualPage:
                  `0x${pageTemplateValue(1).toString(16)}`,
                currentPhysicalPage:
                  `0x${pageTemplateValue(2).toString(16)}`,
                templateVirtualPage:
                  `0x${pageTemplateValue(3).toString(16)}`,
                templatePhysicalPage:
                  `0x${pageTemplateValue(4).toString(16)}`,
                requestedEntries:
                  pageTemplateValue(5).toString(),
                coveredEntries:
                  pageTemplateValue(6).toString(),
                emittedWasmBytes:
                  pageTemplateValue(7).toString(),
                copiedBytes: bytes.length,
                templateCacheAction,
                templateCacheId,
                issuedAtMs: t0,
              };
          vm.jitRegCount = (vm.jitRegCount ?? 0) + 1;
          vm.jitRegBytes = (vm.jitRegBytes ?? 0) + bytes.length;
          let compiledModule;
          if (templateCacheAction === 2) {
            compiledModule = vm.jitPageTemplateModules?.get(templateCacheId);
            if (compiledModule) {
              vm.jitTemplateCacheHits = (vm.jitTemplateCacheHits ?? 0) + 1;
              compiledModule = Promise.resolve(compiledModule);
            } else {
              vm.jitTemplateCacheMisses = (vm.jitTemplateCacheMisses ?? 0) + 1;
              compiledModule = Promise.reject(
                new Error(`missing JIT page template ${templateCacheId}`),
              );
            }
          } else {
            compiledModule = WebAssembly.compile(bytes).then((mod) => {
              if (templateCacheAction === 1) {
                (vm.jitPageTemplateModules ??= new Map()).set(templateCacheId, mod);
                vm.jitTemplateCacheStores = (vm.jitTemplateCacheStores ?? 0) + 1;
              }
              return mod;
            });
          }
          compiledModule
            .then((mod) => {
              const compiled = performance.now();
              const guestBaseImport = guestBaseGlobal
                ? { guest_base: guestBaseGlobal }
                : {};
              const inst = new WebAssembly.Instance(mod, {
                env: {
                  memory: vm.ex.memory,
                  ...guestBaseImport,
                  tlb_fill: vm.ex.jit_tlb_fill,
                  system_bulk_copy: vm.ex.jit_system_bulk_copy,
                  fp_exec: vm.ex.jit_fp_exec,
                  user_reservation: vm.ex.jit_user_reservation,
                  system_reservation: vm.ex.jit_system_reservation,
                  user_vector: vm.ex.jit_user_vector,
                  system_vector: vm.ex.jit_system_vector,
                  chain_next: vm.ex.chain_next,
                  tail_chain: vm.tailChain,
                  __indirect_function_table: vm.ex.__indirect_function_table,
                },
              });
              return { inst, compiled, instantiated: performance.now() };
            })
            .then(({ inst, compiled, instantiated }) => {
              const table = vm.ex.__indirect_function_table;
              vm.tableNext ??= table.length;
              if (vm.tableNext >= table.length) table.grow(4096);
              const idx = vm.tableNext++;
              table.set(idx, inst.exports.run);
              const published = performance.now();
              vm.jitCopyMs = (vm.jitCopyMs ?? 0) + (copied - t0);
              vm.jitCompileMs = (vm.jitCompileMs ?? 0) + (compiled - copied);
              vm.jitInstantiateMs = (vm.jitInstantiateMs ?? 0) +
                (instantiated - compiled);
              vm.jitPublishMs = (vm.jitPublishMs ?? 0) +
                (published - instantiated);
              vm.jitRegMs = (vm.jitRegMs ?? 0) + (compiled - t0);
              vm.jitRegTotalMs = (vm.jitRegTotalMs ?? 0) + (published - t0);
              vm.jitBlocks = (vm.jitBlocks ?? 0) + 1;
              vm.ex.sys_sb_ready(ticket, idx);
              if (pageTemplateDiagnostic) {
                (vm.jitAsyncModuleDiagnostics ??= []).push({
                  ...pageTemplateDiagnostic,
                  failed: false,
                  copyMs: copied - t0,
                  compileLatencyMs: compiled - copied,
                  instantiateMs: instantiated - compiled,
                  publishMs: published - instantiated,
                  totalLatencyMs: published - t0,
                });
              }
              vm.onJitModule?.(bytes, { kind: "async-region", ticket, index: idx });
            })
            .catch((e) => {
              if (pageTemplateDiagnostic) {
                (vm.jitAsyncModuleDiagnostics ??= []).push({
                  ...pageTemplateDiagnostic,
                  failed: true,
                  error: String(e),
                  copyMs: copied - t0,
                  totalLatencyMs: performance.now() - t0,
                });
              }
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
    vm = new RV64Debug(instance);
    vm.hardwareFmaSupported = false;
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
        vm.hardwareFmaSupported = true;
      }
    } catch {
      /* no relaxed SIMD: the exact emulated fma stays in charge */
    }
    // Frame-free region chaining needs Wasm tail calls. A single helper owns
    // the shared table and tail-calls its selected entry; generated modules
    // import this function rather than the table itself. That preserves the
    // fast transfer while avoiding V8's O(table-importing instances) table.set
    // publication behavior.
    vm.tailCallsSupported = false;
    vm.tailChain = undefined;
    try {
      const tailChainModule = new WebAssembly.Module(new Uint8Array([
        0, 0x61, 0x73, 0x6d, 1, 0, 0, 0,
        // type 0: generated entry (i32 state) -> (); type 1: trampoline
        // (i32 state, i32 table index) -> ().
        1, 10, 2, 0x60, 1, 0x7f, 0, 0x60, 2, 0x7f, 0x7f, 0,
        // One table import, owned by this helper instance only.
        2, 15, 1, 3, 0x65, 0x6e, 0x76, 5, 0x74, 0x61, 0x62, 0x6c, 0x65,
        1, 0x70, 0, 0,
        3, 2, 1, 1,
        7, 14, 1, 10, 0x74, 0x61, 0x69, 0x6c, 0x5f, 0x63, 0x68, 0x61, 0x69,
        0x6e, 0, 0,
        // local.get state; local.get index; return_call_indirect type 0 table 0.
        10, 11, 1, 9, 0, 0x20, 0, 0x20, 1, 0x13, 0, 0, 0x0b,
      ]));
      vm.tailChain = new WebAssembly.Instance(tailChainModule, {
        env: { table: vm.ex.__indirect_function_table },
      }).exports.tail_chain;
      vm.tailCallsSupported = true;
      // Legacy trace tail-calls retain their explicit environment switch;
      // the stable system API enables only structured-region transfers below.
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

  /** Independent JIT lifecycle phases for benchmark accounting. */
  jitLifecycleStats() {
    const userTranslateNs = Number(this.ex.jit_stat(73));
    const systemTranslateNs = Number(this.ex.jit_stat(76));
    const userTranslateAttempts = Number(this.ex.jit_stat(74));
    const systemTranslateAttempts = Number(this.ex.jit_stat(77));
    const userEmittedBytes = Number(this.ex.jit_stat(75));
    const systemEmittedBytes = Number(this.ex.jit_stat(78));
    return {
      translateMs: (userTranslateNs + systemTranslateNs) / 1_000_000,
      translateAttempts: userTranslateAttempts + systemTranslateAttempts,
      emittedBytes: userEmittedBytes + systemEmittedBytes,
      userTranslateMs: userTranslateNs / 1_000_000,
      userTranslateAttempts,
      userEmittedBytes,
      systemTranslateMs: systemTranslateNs / 1_000_000,
      systemTranslateAttempts,
      systemEmittedBytes,
      copiedBytes: this.jitRegBytes ?? 0,
      modules: this.jitRegCount ?? 0,
      copyMs: this.jitCopyMs ?? 0,
      compileMs: this.jitCompileMs ?? 0,
      instantiateMs: this.jitInstantiateMs ?? 0,
      publishMs: this.jitPublishMs ?? 0,
    };
  }

  /**
   * Exact cumulative JIT counters for diagnostics and benchmark attribution.
   *
   * u64 counters are returned as decimal strings so the result can cross a
   * Worker boundary without losing precision. Timing fields are ordinary
   * JavaScript numbers because their source is performance.now().
   */
  jitStats() {
    const stat = (index) => this.ex.jit_stat?.(index)?.toString() ?? "0";
    const policy = (index) =>
      this.ex.jit_page_policy_stat?.(index)?.toString() ?? "0";
    return {
      generated: {
        retired: stat(0),
        dispatches: stat(1),
        userEntries: stat(2),
        systemEntries: stat(3),
        zeroRetireDispatches: stat(15),
        tlbFills: stat(31),
        chainHops: stat(80),
        tlbFillKinds: {
          loadHit: stat(81),
          loadEmpty: stat(82),
          loadContext: stat(83),
          loadCollision: stat(84),
          storeHit: stat(85),
          storeEmpty: stat(86),
          storeContext: stat(87),
          storeCollision: stat(88),
        },
      },
      mmu: {
        mappingInvalidations: stat(89),
        changedSatp: stat(90),
        fullTlbClears: stat(91),
        storeJitTlbClears: stat(92),
        sfenceAll: stat(93),
        sfencePage: stat(94),
        sfenceForeignAsid: stat(95),
      },
      interpreter: {
        calls: stat(4),
        retired: stat(5),
      },
      // Read-only compatibility for reports produced while the rejected
      // R070-R077 experiment existed. The production runtime no longer
      // exports or executes static-T0 code; old counter readers see an
      // explicit unsupported/all-zero lifecycle instead of failing.
      staticT0: {
        supported: false,
        userFastRetired: "0",
        userSlowRetired: "0",
        userSlowBatches: "0",
        systemFastRetired: "0",
        systemSlowRetired: "0",
        systemSlowBatches: "0",
        systemFetchFills: "0",
        systemErrors: "0",
        sampledRetired: "0",
        samples: "0",
        interruptPolls: "0",
        shortSampleMarks: "0",
        shortSampleBypasses: "0",
        shortSampleClears: "0",
        systemEnabled: false,
        sampledEnabled: false,
        sampledBackoffEnabled: false,
        moduleIndex: -1,
      },
      regions: {
        triggered: stat(10),
        translateFailures: stat(11),
        issued: stat(12),
        landed: stat(13),
        stale: stat(14),
        pending: stat(17),
        entriesInstalled: stat(25),
        entriesReplaced: stat(26),
        batches: stat(43),
        batchMembers: stat(44),
        calls: stat(48),
        retired: stat(49),
        extensions: stat(32),
        exitSamples: stat(33),
        extensionQueued: stat(39),
        extensionDrainVisits: stat(40),
        demoted: stat(42),
      },
      translation: {
        userNanoseconds: stat(73),
        userAttempts: stat(74),
        userEmittedBytes: stat(75),
        systemNanoseconds: stat(76),
        systemAttempts: stat(77),
        systemEmittedBytes: stat(78),
      },
      pagePolicy: {
        enabled: policy(0),
        threshold: policy(1),
        quantum: policy(2),
        samples: policy(3),
        sampledRetired: policy(4),
        liveMappings: policy(5),
        candidates: policy(6),
        queued: policy(7),
        pending: policy(8),
        queueDrops: policy(9),
        queueMaximum: policy(10),
        issued: policy(11),
        landed: policy(12),
        failed: policy(13),
        compiledMappings: policy(14),
        queueCapacity: policy(15),
        rejected: policy(16),
        suppressed: policy(17),
        staleDrops: policy(18),
        issuedPages: policy(19),
        multiPageIssued: policy(20),
        rebuilds: policy(21),
        rebuildEnabled: policy(22),
        controlEntriesEnabled: policy(23),
        controlEntrySamples: policy(24),
        inflightLimit: policy(25),
        multiPageEntryCap: policy(26),
        multiPageEntryEligible: policy(27),
        multiPageEntryBlocked: policy(28),
        crossPageCallsEnabled: policy(29),
        measuredRegionsEnabled: policy(30),
        extensionPageCap: policy(31),
        extensionMinStay: policy(32),
        extensionShortBlocked: policy(33),
        multiPageControlPermille: policy(34),
        multiPageControlEligible: policy(35),
        multiPageControlBlocked: policy(36),
        controlProfileEnabled: policy(37),
        regionPageCap: policy(38),
        regionLeaderCap: policy(39),
        regionTailChainEnabled: policy(40),
        regionTlbCacheEnabled: policy(43),
        regionTlbCacheMinAccesses: policy(44),
        privilegedThresholdMultiplier: policy(45),
        userRetired: policy(46),
        privilegedRetired: policy(47),
        userCandidates: policy(48),
        privilegedCandidates: policy(49),
        privilegedControlEntriesEnabled: policy(50),
        stableChainEnabled: policy(51),
      },
      loader: {
        modules: this.jitRegCount ?? 0,
        bytes: this.jitRegBytes ?? 0,
        copyMs: this.jitCopyMs ?? 0,
        compileMs: this.jitCompileMs ?? 0,
        instantiateMs: this.jitInstantiateMs ?? 0,
        publishMs: this.jitPublishMs ?? 0,
        totalMs: this.jitRegTotalMs ?? 0,
      },
      p9: {
        requests: this.p9Requests,
        pending: this.p9Pending,
        maximumPending: this.p9MaxPending,
        hostMs: this.p9HostMs,
        requestBytes: this.p9RequestBytes,
        replyBytes: this.p9ReplyBytes,
        readRequests: this.p9ReadRequests,
        writeRequests: this.p9WriteRequests,
        readBytes: this.p9ReadBytes,
        writeBytes: this.p9WriteBytes,
        maximumRead: this.p9MaxRead,
        maximumWrite: this.p9MaxWrite,
      },
    };
  }

  /** Snapshot opt-in dispatch/fallback profiling for benchmark diagnostics. */
  jitProfile(limit = 20) {
    const dispatch = [];
    const edges = [];
    const fallback = [];
    for (let index = 0; index < 8192; index++) {
      const calls = Number(this.ex.dprof_get?.(1, index) ?? 0n);
      if (calls) {
        const retired = Number(this.ex.dprof_get(2, index));
        dispatch.push({
          pc: `0x${this.ex.dprof_get(0, index).toString(16)}`,
          calls,
          retired,
          instructionsPerCall: retired / calls,
        });
      }
      const transitions = Number(this.ex.eprof_get?.(2, index) ?? 0n);
      if (transitions) {
        const retired = Number(this.ex.eprof_get(3, index));
        edges.push({
          source: `0x${this.ex.eprof_get(0, index).toString(16)}`,
          target: `0x${this.ex.eprof_get(1, index).toString(16)}`,
          transitions,
          retired,
          instructionsPerTransition: retired / transitions,
        });
      }
    }
    for (let index = 0; index < 1024; index++) {
      const stretches = Number(this.ex.ihist_get?.(1, index) ?? 0n);
      if (stretches) {
        fallback.push({
          key: `0x${this.ex.ihist_get(0, index).toString(16)}`,
          stretches,
          interpretedInstructions: Number(this.ex.ihist_get(2, index)),
        });
      }
    }
    dispatch.sort((a, b) => b.calls - a.calls);
    edges.sort((a, b) => b.transitions - a.transitions);
    fallback.sort((a, b) => b.interpretedInstructions - a.interpretedInstructions);
    return {
      generatedBlockCalls: this.ex.jit_stat(46).toString(),
      generatedBlockInstructions: this.ex.jit_stat(47).toString(),
      generatedRegionCalls: this.ex.jit_stat(48).toString(),
      generatedRegionInstructions: this.ex.jit_stat(49).toString(),
      dispatch: dispatch.slice(0, limit),
      edges: edges.slice(0, limit),
      fallback: fallback.slice(0, limit),
    };
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
RV64Debug.prototype.bootLinux = function ({
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
RV64Debug.prototype.connectNet = function (url) {
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
RV64Debug.prototype.netInput = function (frame) {
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
RV64Debug.prototype.connectHttpRelay = function (url, options = {}) {
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
            this.onNetworkTraffic?.({
              type: "response",
              status: new DataView(
                message.payload.buffer,
                message.payload.byteOffset,
                message.payload.byteLength,
              ).getUint32(0, true),
            });
            this.stageFor(message.payload);
            this.ex.sys_http_head(message.id);
            break;
          case HTTP_RELAY_BODY:
            this.onNetworkTraffic?.({
              type: "download",
              bytes: message.payload.length,
            });
            this.stageFor(message.payload);
            this.ex.sys_http_body(message.id);
            break;
          case HTTP_RELAY_END:
            pending.delete(message.id);
            this.onNetworkTraffic?.({ type: "end" });
            this.ex.sys_http_end(message.id);
            break;
          case HTTP_RELAY_ERROR:
            pending.delete(message.id);
            this.onNetworkTraffic?.({
              type: "error",
              message: new TextDecoder().decode(message.payload),
            });
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

RV64Debug.prototype.disconnectHttpRelay = function () {
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
RV64Debug.prototype.routeHttpViaRelay = function (origin, enabled = true) {
  const normalized = requestOrigin(origin) || origin.replace(/\/+$/, "");
  if (enabled) this.httpRelayOrigins.add(normalized);
  else this.httpRelayOrigins.delete(normalized);
};

RV64Debug.prototype.performHttpViaRelay = async function (id, encodedRequest) {
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
RV64Debug.prototype.runSystem = function (maxInsns = 10_000_000n) {
  return this.ex.sys_run(BigInt(maxInsns)) === 1;
};

/** Send keyboard input to the guest console. */
RV64Debug.prototype.consoleInput = function (bytes) {
  const ptr = this.ex.staging_alloc(bytes.length);
  new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
  this.ex.sys_console_input();
};

RV64Debug.prototype.sysInsnCount = function () {
  return this.ex.sys_insn_count();
};

/** Boot the modern OpenSBI/Linux virt machine. */
RV64Debug.prototype.bootVirtLinux = function ({
  opensbi,
  kernel,
  initrd,
  disk,
  cmdline,
  ramMB = 512,
  net = false,
  netMac,
  proxy = false,
  proxyUpgradeHttps = true,
  p9,
  virtioConsole = false,
}) {
  const stage = (bytes, fn) => {
    if (!bytes) return;
    const ptr = this.ex.staging_alloc(bytes.length);
    new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
    fn();
  };
  stage(opensbi, () => this.ex.virt_stage_opensbi());
  stage(kernel, () => this.ex.virt_stage_kernel());
  stage(initrd, () => this.ex.virt_stage_initrd());
  stage(disk, () => this.ex.virt_stage_disk());
  if (cmdline) stage(new TextEncoder().encode(cmdline), () => this.ex.virt_stage_cmdline());
  if (netMac) stage(new Uint8Array(netMac), () => this.ex.virt_stage_net_mac());
  if (p9?.tag) stage(new TextEncoder().encode(p9.tag), () => this.ex.virt_stage_fs_external_tag());
  this.onP9Request = p9?.handle ?? null;
  this.ex.virt_console_enable(virtioConsole ? 1 : 0);
  this.ex.virt_net_enable(net || proxy ? 1 : 0);
  this.ex.sys_proxy_enable(proxy ? 1 : 0, proxyUpgradeHttps ? 1 : 0);
  this.ex.virt_boot(ramMB);
};

/** Assemble riscv-virt and enter Linux directly in S-mode. */
RV64Debug.prototype.bootVirtLinuxDirect = function ({
  kernel,
  initrd,
  disk,
  cmdline,
  ramMB = 512,
  net = false,
  netMac,
  proxy = false,
  proxyUpgradeHttps = true,
  p9,
  virtioConsole = false,
}) {
  const stage = (bytes, fn) => {
    if (!bytes) return;
    const ptr = this.ex.staging_alloc(bytes.length);
    new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
    fn();
  };
  stage(kernel, () => this.ex.virt_stage_kernel());
  stage(initrd, () => this.ex.virt_stage_initrd());
  stage(disk, () => this.ex.virt_stage_disk());
  if (cmdline) stage(new TextEncoder().encode(cmdline), () => this.ex.virt_stage_cmdline());
  if (netMac) stage(new Uint8Array(netMac), () => this.ex.virt_stage_net_mac());
  if (p9?.tag) stage(new TextEncoder().encode(p9.tag), () => this.ex.virt_stage_fs_external_tag());
  this.onP9Request = p9?.handle ?? null;
  this.ex.virt_console_enable(virtioConsole ? 1 : 0);
  this.ex.virt_net_enable(net || proxy ? 1 : 0);
  this.ex.sys_proxy_enable(proxy ? 1 : 0, proxyUpgradeHttps ? 1 : 0);
  this.ex.virt_boot_direct(ramMB);
};

/** Run a slice of the modern virt machine. Returns true when powered off. */
RV64Debug.prototype.runVirtSystem = function (maxInsns = 2_000_000n) {
  const stopped = this.ex.virt_run(BigInt(maxInsns)) === 1;
  this.pumpP9();
  return stopped;
};

RV64Debug.prototype.pumpP9 = function () {
  if (!this.onP9Request) return;
  for (;;) {
    const len = this.ex.virt_p9_take_request();
    if (!len) break;
    const request = new Uint8Array(this.ex.memory.buffer, this.ex.staging_ptr(), len).slice();
    this.p9Pending++;
    this.p9Requests++;
    this.p9RequestBytes += request.length;
    if (request.length >= 23 && (request[4] === 116 || request[4] === 118)) {
      const count = new DataView(request.buffer, request.byteOffset, request.byteLength)
        .getUint32(19, true);
      if (request[4] === 116) {
        this.p9ReadRequests++;
        this.p9ReadBytes += count;
        this.p9MaxRead = Math.max(this.p9MaxRead, count);
      } else {
        this.p9WriteRequests++;
        this.p9WriteBytes += count;
        this.p9MaxWrite = Math.max(this.p9MaxWrite, count);
      }
    }
    this.p9MaxPending = Math.max(this.p9MaxPending, this.p9Pending);
    const started = performance.now();
    Promise.resolve()
      .then(() => this.onP9Request(request))
      .then((reply) => {
        if (!(reply instanceof Uint8Array)) reply = new Uint8Array(reply);
        if (reply.length < 7) throw new Error("external 9P handler returned an invalid reply");
        this.p9ReplyBytes += reply.length;
        const ptr = this.ex.staging_alloc(reply.length);
        new Uint8Array(this.ex.memory.buffer, ptr, reply.length).set(reply);
        if (this.ex.virt_p9_reply() !== 1) throw new Error("unexpected external 9P reply");
      })
      .catch((error) => {
        console.error("external 9P request failed", error);
        // Rlerror(size=11, type=7, original tag, errno=EIO) keeps the guest
        // queue moving even when the host handler rejects.
        const reply = new Uint8Array(11);
        new DataView(reply.buffer).setUint32(0, 11, true);
        reply[4] = 7;
        reply[5] = request[5];
        reply[6] = request[6];
        new DataView(reply.buffer).setUint32(7, 5, true);
        const ptr = this.ex.staging_alloc(reply.length);
        new Uint8Array(this.ex.memory.buffer, ptr, reply.length).set(reply);
        this.ex.virt_p9_reply();
      })
      .finally(() => {
        this.p9HostMs += performance.now() - started;
        this.p9Pending--;
        // A completion may have let Linux submit another batch in a run slice.
        this.pumpP9();
      });
  }
};

/** Send keyboard input to the modern machine's 8250 UART. */
RV64Debug.prototype.virtConsoleInput = function (bytes) {
  const ptr = this.ex.staging_alloc(bytes.length);
  new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
  this.ex.virt_console_input();
};

RV64Debug.prototype.virtExportInput = function (bytes) {
  const ptr = this.ex.staging_alloc(bytes.length);
  new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
  this.ex.virt_export_input();
};

RV64Debug.prototype.virtNetInput = function (frame) {
  const ptr = this.ex.staging_alloc(frame.length);
  new Uint8Array(this.ex.memory.buffer, ptr, frame.length).set(frame);
  this.ex.virt_net_input();
};

RV64Debug.prototype.wispEnable = function (enabled) {
  this.ex.sys_wisp_enable(enabled ? 1 : 0);
};

RV64Debug.prototype.wispData = function (id, bytes) {
  this.stageFor(bytes);
  this.ex.sys_wisp_data(BigInt(id));
};

RV64Debug.prototype.wispClose = function (id) {
  this.ex.sys_wisp_close(BigInt(id));
};

RV64Debug.prototype.wispDatagram = function (id, bytes) {
  this.stageFor(bytes);
  this.ex.sys_wisp_datagram(BigInt(id));
};

RV64Debug.prototype.virtInsnCount = function () {
  return this.ex.virt_insn_count();
};

/** Direct-SBI call counts for diagnostics and profiling. */
RV64Debug.prototype.virtSbiCallCounts = function () {
  const names = ["total", "base", "time", "ipi", "rfence", "hsm", "srst", "other"];
  return Object.fromEntries(
    names.map((name, index) => [name, this.ex.virt_sbi_call_count(index)]),
  );
};

/** Current modern-machine PC. Diagnostic API; not part of the stable facade. */
RV64Debug.prototype.virtPc = function () {
  return this.ex.virt_pc();
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
RV64Debug.prototype.proxyURL = function () {
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
RV64Debug.prototype.performHttp = async function (id, req, encodedRequest) {
  const origin = requestOrigin(req.url);
  this.onNetworkTraffic?.({
    type: "request",
    bytes: encodedRequest?.length ?? req.body.length,
    method: req.method,
    url: req.url,
  });
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

    this.onNetworkTraffic?.({ type: "response", status: resp.status, url: req.url });
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
          this.onNetworkTraffic?.({ type: "download", bytes: value.length });
          this.stageFor(value);
          this.ex.sys_http_body(BigInt(id));
        }
      }
    }
    this.ex.sys_http_end(BigInt(id));
    this.onNetworkTraffic?.({ type: "end", url: req.url });
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
    this.onNetworkTraffic?.({
      type: "error",
      message: String(e?.message ?? e),
      url: req.url,
    });
    this.stageFor(new TextEncoder().encode(String(e?.message ?? e)));
    this.ex.sys_http_fail(BigInt(id));
  }
};

/** Copy bytes into the wasm staging buffer for the next sys_http_* call. */
RV64Debug.prototype.stageFor = function (bytes) {
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

const PUBLIC_EVENTS = new Set([
  "ready",
  "start",
  "stop",
  "error",
  "console",
  "export",
  "networkTransmit",
  "networkTraffic",
  "downloadProgress",
]);

async function imageBytes(source, name, emit) {
  if (source instanceof Uint8Array) return source;
  if (ArrayBuffer.isView(source)) {
    return new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
  }
  if (source instanceof ArrayBuffer) return new Uint8Array(source);

  let response;
  if (source instanceof Response) response = source;
  else if (source && typeof source === "object" && typeof source.url === "string") {
    response = await fetch(source.url);
  } else {
    throw new TypeError(`${name} must be an ImageSource`);
  }
  if (!response.ok) throw new Error(`${name}: ${response.status} ${response.statusText}`);
  const total = response.headers.has("content-encoding")
    ? undefined
    : Number(response.headers.get("content-length")) || undefined;
  if (!response.body) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    emit("downloadProgress", { image: name, loaded: bytes.length, total });
    return bytes;
  }
  const reader = response.body.getReader();
  const chunks = [];
  let loaded = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    loaded += value.length;
    emit("downloadProgress", { image: name, loaded, total });
  }
  const bytes = new Uint8Array(loaded);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  return bytes;
}

const hostYieldQueue = [];
let hostYieldChannel;

function hostYield(callback) {
  if (typeof setImmediate === "function") return setImmediate(callback);
  if (typeof MessageChannel === "function") {
    if (!hostYieldChannel) {
      hostYieldChannel = new MessageChannel();
      hostYieldChannel.port1.onmessage = () => hostYieldQueue.shift()?.();
    }
    hostYieldQueue.push(callback);
    hostYieldChannel.port2.postMessage(0);
    return;
  }
  return setTimeout(callback, 0);
}

function isOpenSBI(bytes) {
  const needle = new TextEncoder().encode("OpenSBI");
  outer: for (let i = 0; i <= bytes.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (bytes[i + j] !== needle[j]) continue outer;
    }
    return true;
  }
  return false;
}

// WISP v1 transports stream payloads in binary WebSocket messages. The guest
// side TCP state machine lives in Rust; this class is deliberately only the
// WISP wire protocol and flow control.
class WispClient {
  constructor(url, protocols, core) {
    this.core = core;
    this.nextStream = 1;
    this.credit = 0;
    this.byGuest = new Map();
    this.byStream = new Map();
    this.queue = [];
    const transportURL = url.replace(/^wisp:/, "ws:").replace(/^wisps:/, "wss:");
    this.socket = new WebSocket(transportURL, protocols);
    this.socket.binaryType = "arraybuffer";
    this.socket.onopen = () => this.#flush();
    this.socket.onmessage = ({ data }) => {
      if (typeof data !== "string") this.#receive(new Uint8Array(data));
    };
    this.socket.onclose = () => {
      for (const guest of this.byGuest.keys()) core.wispClose(guest);
      this.byGuest.clear();
      this.byStream.clear();
    };
  }

  open(guest, address, port, transport = 1) {
    const stream = this.nextStream++ >>> 0;
    const host = Array.from(address).join(".");
    const hostname = new TextEncoder().encode(host);
    const packet = new Uint8Array(8 + hostname.length);
    const view = new DataView(packet.buffer);
    packet[0] = 1; // CONNECT
    view.setUint32(1, stream, true);
    packet[5] = transport;
    view.setUint16(6, port, true);
    packet.set(hostname, 8);
    const state = { guest, stream, transport, credit: this.credit, pending: [] };
    this.byGuest.set(guest, state);
    this.byStream.set(stream, state);
    this.#send(packet);
  }

  data(guest, bytes) {
    const state = this.byGuest.get(guest);
    if (!state) return;
    const packet = new Uint8Array(5 + bytes.length);
    packet[0] = 2; // DATA
    new DataView(packet.buffer).setUint32(1, state.stream, true);
    packet.set(bytes, 5);
    if (state.credit > 0) {
      state.credit--;
      this.#send(packet);
    } else {
      state.pending.push(packet);
    }
  }

  datagram(guest, address, port, bytes) {
    if (!this.byGuest.has(guest)) this.open(guest, address, port, 2);
    this.data(guest, bytes);
  }

  closeGuest(guest) {
    const state = this.byGuest.get(guest);
    if (!state) return;
    const packet = new Uint8Array(6);
    packet[0] = 4; // CLOSE
    new DataView(packet.buffer).setUint32(1, state.stream, true);
    packet[5] = 2; // voluntary closure
    this.#send(packet);
    this.#forget(state);
  }

  close() {
    this.socket.onclose = null;
    this.socket.close();
    this.byGuest.clear();
    this.byStream.clear();
  }

  #receive(packet) {
    if (packet.length < 5) return;
    const view = new DataView(packet.buffer, packet.byteOffset, packet.byteLength);
    const type = packet[0];
    const stream = view.getUint32(1, true);
    const state = this.byStream.get(stream);
    if (type === 2 && state) {
      if (state.transport === 2) this.core.wispDatagram(state.guest, packet.subarray(5));
      else this.core.wispData(state.guest, packet.subarray(5));
    } else if (type === 3 && packet.length >= 9) {
      if (stream === 0) {
        this.credit = view.getUint32(5, true);
        for (const connection of this.byStream.values()) {
          if (connection.credit === 0) connection.credit = this.credit;
          this.#drain(connection);
        }
        return;
      }
      if (!state) return;
      state.credit = view.getUint32(5, true);
      this.#drain(state);
    } else if (type === 4 && state) {
      this.core.wispClose(state.guest);
      this.#forget(state);
    }
  }

  #forget(state) {
    this.byGuest.delete(state.guest);
    this.byStream.delete(state.stream);
  }

  #drain(state) {
    while (state.credit > 0 && state.pending.length) {
      state.credit--;
      this.#send(state.pending.shift());
    }
  }

  #send(packet) {
    if (this.socket.readyState === WebSocket.OPEN) this.socket.send(packet);
    else this.queue.push(packet);
  }

  #flush() {
    for (const packet of this.queue) this.socket.send(packet);
    this.queue.length = 0;
  }
}

function normalizeNetwork(network, bootMode) {
  const value = network ?? { mode: bootMode === "bare-metal" ? "none" : "fetch" };
  if (!value || typeof value !== "object") throw new TypeError("network must be an object");
  if (!["none", "fetch", "wsproxy", "wisp", "inbrowser", "external"].includes(value.mode)) {
    throw new TypeError(`unknown network mode: ${value.mode}`);
  }
  if (bootMode === "bare-metal" && value.mode !== "none") {
    throw new Error("bare-metal networking is not implemented");
  }
  if (["wsproxy", "wisp"].includes(value.mode) && typeof value.url !== "string") {
    throw new TypeError(`${value.mode} networking requires url`);
  }
  if (value.mode === "inbrowser" && value.channel !== undefined && typeof value.channel !== "string") {
    throw new TypeError("inbrowser network.channel must be a string");
  }
  if (value.mac !== undefined && (!(value.mac instanceof Uint8Array) || value.mac.length !== 6)) {
    throw new TypeError("network.mac must be a 6-byte Uint8Array");
  }
  return { ...value };
}

/** Stable embedding API. Raw Wasm and instruction slicing stay private. */
export class RV64 {
  #core;
  #boot;
  #listeners = new Map();
  #running = false;
  #destroyed = false;
  #generation = 0;
  #runSlice;
  #input;
  #instructions;
  #networkConfig;
  #networkInput;
  #networkSocket;
  #networkChannel;
  #wisp;

  constructor(core, boot, network, listeners) {
    this.#core = core;
    this.#boot = boot;
    this.#networkConfig = network;
    for (const [event, listener] of Object.entries(listeners ?? {})) {
      this.on(event, listener);
    }
    this.console = Object.freeze({ send: (data) => this.#sendConsole(data) });
    this.export = Object.freeze({ send: (data) => this.#sendExport(data) });
    this.network = Object.freeze({
      mode: network.mode,
      get proxyURL() { return network.mode === "fetch" ? core.proxyURL() : undefined; },
      receive: (frame) => this.#receiveNetwork(frame),
    });
  }

  /** Resolve images, instantiate Wasm, and assemble a stopped machine. */
  static async create(options) {
    if (!options || typeof options !== "object") {
      throw new TypeError("RV64.create expects an options object");
    }
    if (options.execution?.mode === "worker") {
      if (options.boot?.p9) {
        throw new TypeError("external 9P handlers require local execution mode");
      }
      return RV64WorkerProxy.create(options);
    }
    if (options.execution?.mode !== undefined && options.execution.mode !== "local") {
      throw new TypeError(`unknown execution mode: ${options.execution.mode}`);
    }
    const { wasm, boot, memoryMB, events } = options;
    if (!wasm) throw new TypeError("RV64.create requires wasm");
    if (!boot?.mode) throw new TypeError("RV64.create requires boot.mode");

    // Register creation-time listeners before fetching any image.
    const pending = new Map();
    for (const [event, listener] of Object.entries(events ?? {})) {
      if (!PUBLIC_EVENTS.has(event) || typeof listener !== "function") {
        throw new TypeError(`invalid ${event} event listener`);
      }
      pending.set(event, new Set([listener]));
    }
    const emit = (event, detail) => {
      for (const listener of pending.get(event) ?? []) listener(detail);
    };

    const wasmBytes = await imageBytes(wasm, "wasm", emit);
    const resolved = { ...boot };
    for (const key of ["firmware", "kernel", "initrd", "disk", "image"]) {
      const source = boot[key];
      if (source !== undefined && source !== "default") {
        resolved[key] = await imageBytes(source, key, emit);
      }
    }
    const network = normalizeNetwork(options.network, boot.mode);
    const core = await RV64Debug.create(wasmBytes);
    // System-mode production policy: approximate heat per safe VA→PA mapping,
    // then emit one bounded region at a time and compile it asynchronously.
    // The Wasm defaults are trace- and browser-benchmarked; low-level tests can
    // still select either policy directly through RV64Debug exports.
    core.ex.jit_set_page_policy?.(1);
    if (core.tailCallsSupported) core.ex.jit_set_region_tail_chain?.(1);
    const vm = new RV64(core, { ...resolved, memoryMB }, network, events);
    vm.#assemble();
    vm.#emit("ready", undefined);
    return vm;
  }

  get running() {
    return this.#running;
  }

  get instructions() {
    this.#assertLive();
    return this.#instructions();
  }

  /** Enable or disable generated-code tiering without stopping the machine. */
  setJitEnabled(enabled) {
    this.#assertLive();
    if (typeof enabled !== "boolean") throw new TypeError("enabled must be a boolean");
    this.#core.ex.jit_set_enabled(enabled ? 1 : 0);
  }

  /**
   * Select measured JIT policy knobs for controlled experiments.
   * Omitted fields retain their current value.
   */
  configureJit(options) {
    this.#assertLive();
    if (!options || typeof options !== "object") {
      throw new TypeError("JIT options must be an object");
    }
    for (const removed of ["staticSystemT0", "sampledStaticT0", "sampledStaticT0Backoff"]) {
      if (Object.hasOwn(options, removed)) {
        throw new TypeError(`${removed} was removed after the static-T0 experiment was rejected`);
      }
    }
    if (options.policy !== undefined) {
      if (options.policy !== "page" && options.policy !== "adaptive") {
        throw new TypeError("JIT policy must be page or adaptive");
      }
      this.#core.ex.jit_set_page_policy(options.policy === "page" ? 1 : 0);
    }
    if (options.regionTailChain !== undefined) {
      if (typeof options.regionTailChain !== "boolean") {
        throw new TypeError("regionTailChain must be a boolean");
      }
      if (options.regionTailChain && !this.#core.tailCallsSupported) {
        throw new Error("regionTailChain requires WebAssembly tail-call support");
      }
      this.#core.ex.jit_set_region_tail_chain?.(options.regionTailChain ? 1 : 0);
    }
    for (const [name, setter, minimum, maximum] of [
      ["pageThreshold", "jit_set_page_threshold", 1, 0xffff_ffff],
      ["privilegedPageThresholdMultiplier", "jit_set_privileged_page_threshold_multiplier", 1, 1024],
      ["pageQuantum", "jit_set_page_quantum", 1, 4096],
      ["regionLeaderCap", "jit_set_region_leader_cap", 2, 512],
      ["regionPageCap", "jit_set_region_page_cap", 1, 3],
      ["regionExtensionPageCap", "jit_set_region_extension_page_cap", 1, 16],
      ["measuredRegionMinStay", "jit_set_page_extension_min_stay", 1, 4096],
      ["pageInflightLimit", "jit_set_page_inflight_limit", 1, 8],
      ["multiPageEntryCap", "jit_set_page_multipage_entry_cap", 0, 512],
      ["multiPageControlPermille", "jit_set_page_multipage_control_permille", 0, 1000],
      ["regionTlbCacheMinAccesses", "jit_set_region_tlb_cache_min_accesses", 1, 64],
      ["profileSampleShift", "dprof_set_sample_shift", 0, 20],
    ]) {
      if (options[name] === undefined) continue;
      const value = options[name];
      if (!Number.isInteger(value) || value < minimum || value > maximum) {
        throw new RangeError(`${name} must be an integer from ${minimum} through ${maximum}`);
      }
      this.#core.ex[setter]?.(value);
    }
    for (const [name, setter] of [
      ["directDispatch", "jit_set_region_direct_dispatch"],
      ["lazyState", "jit_set_region_lazy_state"],
      ["pageRebuild", "jit_set_page_rebuild"],
      ["controlEntries", "jit_set_page_control_entries"],
      ["privilegedControlEntries", "jit_set_page_privileged_control_entries"],
      ["stablePageChain", "jit_set_page_stable_chain"],
      ["controlProfile", "jit_set_page_control_profile"],
      ["crossPageCalls", "jit_set_page_cross_page_calls"],
      ["measuredRegions", "jit_set_page_measured_regions"],
      ["demoteRegions", "jit_set_demote"],
      ["cfgBlocks", "jit_set_region_cfg_blocks"],
      ["structuredCfg", "jit_set_region_structured_cfg"],
      ["tlbFill", "jit_set_tlb_fill"],
      ["tlbHash", "jit_set_tlb_hash"],
      ["privilegeTlbRetention", "jit_set_privilege_tlb_retention"],
      ["regionTlbCache", "jit_set_region_tlb_cache"],
      ["regionChain", "jit_set_region_chain"],
      ["profile", "dprof_set"],
    ]) {
      if (options[name] === undefined) continue;
      if (typeof options[name] !== "boolean") {
        throw new TypeError(`${name} must be a boolean`);
      }
      this.#core.ex[setter]?.(options[name] ? 1 : 0);
    }
  }

  /** Cumulative execution and JIT counters for workload attribution. */
  jitStats() {
    this.#assertLive();
    const stats = this.#core.jitStats();
    const instructions = this.#instructions();
    const generated = BigInt(stats.generated.retired);
    const interpreter = BigInt(stats.interpreter.retired);
    return {
      ...stats,
      instructions: instructions.toString(),
      accountedInstructions: (generated + interpreter).toString(),
      generatedCoverage: instructions === 0n
        ? 0
        : Number(generated * 1_000_000n / instructions) / 1_000_000,
      generatedInstructionsPerDispatch:
        stats.generated.dispatches === "0"
          ? 0
          : Number(generated) / Number(stats.generated.dispatches),
    };
  }

  /** Return opt-in JIT diagnostics configured with `profile: true`. */
  jitProfile(limit = 20) {
    this.#assertLive();
    if (!Number.isInteger(limit) || limit < 1 || limit > 100) {
      throw new RangeError("profile limit must be an integer from 1 through 100");
    }
    return this.#core.jitProfile(limit);
  }

  on(event, listener) {
    if (!PUBLIC_EVENTS.has(event)) throw new TypeError(`unknown event: ${event}`);
    if (typeof listener !== "function") throw new TypeError("listener must be a function");
    let listeners = this.#listeners.get(event);
    if (!listeners) this.#listeners.set(event, (listeners = new Set()));
    listeners.add(listener);
    return () => listeners.delete(listener);
  }

  async start() {
    this.#assertLive();
    if (this.#running) return;
    this.#running = true;
    const generation = ++this.#generation;
    this.#emit("start", undefined);
    hostYield(() => this.#tick(generation));
  }

  async stop() {
    this.#assertLive();
    if (!this.#running) return;
    this.#running = false;
    ++this.#generation;
    this.#emit("stop", { reason: "requested" });
  }

  async reset() {
    this.#assertLive();
    const restart = this.#running;
    if (restart) await this.stop();
    this.#assemble();
    this.#emit("ready", undefined);
    if (restart) await this.start();
  }

  async destroy() {
    if (this.#destroyed) return;
    if (this.#running) await this.stop();
    this.#destroyed = true;
    ++this.#generation;
    this.#networkSocket?.close();
    this.#networkSocket = null;
    this.#networkChannel?.close();
    this.#networkChannel = null;
    this.#wisp?.close();
    this.#wisp = null;
    this.#core.disconnectHttpRelay();
    this.#listeners.clear();
    this.#core = null;
  }

  #assemble() {
    const boot = this.#boot;
    const memoryMB = boot.memoryMB;
    const network = this.#networkConfig;
    const net = network.mode !== "none";
    const proxy = network.mode === "fetch";
    const networkOptions = {
      net,
      netMac: network.mac,
      proxy,
      proxyUpgradeHttps: network.upgradeHttps ?? true,
    };
    const modernCmdline = `${boot.cmdline ?? "console=ttyS0 root=/dev/vda rw"} rv64.network=${network.mode}`;
    const legacyCmdline = `${boot.cmdline ?? "console=hvc0 root=/dev/vda rw"} rv64.network=${network.mode}`;
    if (boot.mode === "firmware") {
      if (boot.firmware === "default") {
        throw new Error("packaged default firmware is not available yet");
      }
      if (!(boot.firmware instanceof Uint8Array)) {
        throw new TypeError("firmware mode requires a firmware image");
      }
      if (isOpenSBI(boot.firmware)) {
        this.#core.bootVirtLinux({
          opensbi: boot.firmware,
          kernel: boot.kernel,
          initrd: boot.initrd,
          disk: boot.disk,
          cmdline: modernCmdline,
          ramMB: memoryMB ?? 512,
          ...networkOptions,
        });
        this.#runSlice = () => this.#core.runVirtSystem(2_000_000n);
        this.#input = (bytes) => this.#core.virtConsoleInput(bytes);
        this.#networkInput = (frame) => this.#core.virtNetInput(frame);
        this.#instructions = () => this.#core.virtInsnCount();
      } else {
        if (boot.initrd) throw new Error("this firmware does not support a separate initrd");
        this.#core.bootLinux({
          bios: boot.firmware,
          kernel: boot.kernel,
          disk: boot.disk,
          cmdline: legacyCmdline,
          ramMB: memoryMB ?? 128,
          ...networkOptions,
        });
        this.#runSlice = () => this.#core.runSystem(3_000_000n);
        this.#input = (bytes) => this.#core.consoleInput(bytes);
        this.#networkInput = (frame) => this.#core.netInput(frame);
        this.#instructions = () => this.#core.sysInsnCount();
      }
    } else if (boot.mode === "bare-metal") {
      if (!(boot.image instanceof Uint8Array)) {
        throw new TypeError("bare-metal mode requires an image");
      }
      if (boot.privilege && boot.privilege !== "machine") {
        throw new Error("bare-metal supervisor entry is not implemented");
      }
      const loadAddress = BigInt(boot.loadAddress);
      this.#core.init(loadAddress, (memoryMB ?? 16) << 20);
      this.#core.load(Number(loadAddress), boot.image);
      this.#core.pc = boot.entry === undefined ? loadAddress : BigInt(boot.entry);
      this.#runSlice = () => {
        const stop = this.#core.run(250_000n);
        return stop !== Stop.BUDGET;
      };
      this.#input = null;
      this.#instructions = () => this.#core.insnCount();
    } else if (boot.mode === "linux-direct") {
      this.#core.bootVirtLinuxDirect({
        kernel: boot.kernel,
        initrd: boot.initrd,
        disk: boot.disk,
        cmdline: modernCmdline,
        ramMB: memoryMB ?? 512,
        p9: boot.p9,
        virtioConsole: boot.virtioConsole,
        ...networkOptions,
      });
      this.#runSlice = () => {
        const poweredOff = this.#core.runVirtSystem(2_000_000n);
        const ext = this.#core.ex.virt_unsupported_sbi_ext();
        if (ext !== 0n) {
          const fn = this.#core.ex.virt_unsupported_sbi_function();
          throw new Error(`unsupported SBI call extension=${ext.toString(16)} function=${fn}`);
        }
        return poweredOff;
      };
      this.#input = (bytes) => this.#core.virtConsoleInput(bytes);
      this.#networkInput = (frame) => this.#core.virtNetInput(frame);
      this.#instructions = () => this.#core.virtInsnCount();
    } else {
      throw new TypeError(`unknown boot mode: ${boot.mode}`);
    }
    this.#core.wispEnable(network.mode === "wisp");
    this.#core.onWrite = (fd, bytes) => this.#emit(fd === 3 ? "export" : "console", bytes);
    this.#core.onNetSend = (frame) => this.#transmitNetwork(frame);
    this.#core.onNetworkTraffic = (detail) => this.#emit("networkTraffic", detail);
    this.#connectNetwork();
  }

  #tick(generation) {
    if (!this.#running || generation !== this.#generation) return;
    try {
      if (this.#runSlice()) {
        this.#running = false;
        this.#emit("stop", { reason: "powered-off" });
        return;
      }
      hostYield(() => this.#tick(generation));
    } catch (error) {
      this.#running = false;
      this.#emit("error", error);
      this.#emit("stop", { reason: "error" });
    }
  }

  #sendConsole(data) {
    this.#assertLive();
    if (!this.#input) throw new Error("this boot mode has no console input");
    const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
    if (!(bytes instanceof Uint8Array)) throw new TypeError("console data must be a string or Uint8Array");
    this.#input(bytes);
  }

  #sendExport(data) {
    this.#assertLive();
    const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
    if (!(bytes instanceof Uint8Array)) throw new TypeError("export data must be a string or Uint8Array");
    this.#core.virtExportInput(bytes);
  }

  #connectNetwork() {
    const network = this.#networkConfig;
    this.#networkSocket?.close();
    this.#networkSocket = null;
    this.#networkChannel?.close();
    this.#networkChannel = null;
    this.#wisp?.close();
    this.#wisp = null;
    this.#core.disconnectHttpRelay();
    if (network.mode === "wsproxy") {
      const socket = new WebSocket(network.url, network.protocols);
      socket.binaryType = "arraybuffer";
      socket.onmessage = (event) => {
        if (typeof event.data !== "string") this.#networkInput?.(new Uint8Array(event.data));
      };
      this.#networkSocket = socket;
    } else if (network.mode === "inbrowser") {
      if (typeof BroadcastChannel === "undefined") {
        throw new Error("inbrowser networking requires BroadcastChannel");
      }
      const channel = new BroadcastChannel(network.channel ?? "rv64.js-network");
      channel.onmessage = ({ data }) => {
        if (data instanceof ArrayBuffer) this.#networkInput?.(new Uint8Array(data));
        else if (ArrayBuffer.isView(data)) {
          this.#networkInput?.(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
        }
      };
      this.#networkChannel = channel;
    } else if (network.mode === "wisp") {
      const wisp = new WispClient(network.url, network.protocols, this.#core);
      this.#core.onWispOpen = (id, address, port) => wisp.open(id, address, port);
      this.#core.onWispData = (id, bytes) => wisp.data(id, bytes);
      this.#core.onWispClose = (id) => wisp.closeGuest(id);
      this.#core.onWispDatagram = (id, address, port, bytes) =>
        wisp.datagram(id, address, port, bytes);
      this.#wisp = wisp;
    } else if (network.mode === "fetch" && network.relayURL) {
      this.#core.connectHttpRelay(network.relayURL);
    }
  }

  #transmitNetwork(frame) {
    if (this.#networkChannel) this.#networkChannel.postMessage(frame);
    const socket = this.#networkSocket;
    if (socket?.readyState === WebSocket.OPEN) socket.send(frame);
    this.#emit("networkTransmit", frame);
  }

  #receiveNetwork(frame) {
    this.#assertLive();
    if (this.#networkConfig.mode !== "external") {
      throw new Error("network.receive is only available in external mode");
    }
    const bytes = frame instanceof Uint8Array ? frame : new Uint8Array(frame);
    this.#networkInput(bytes);
  }

  #emit(event, detail) {
    for (const listener of [...(this.#listeners.get(event) ?? [])]) listener(detail);
  }

  #assertLive() {
    if (this.#destroyed) throw new Error("RV64 instance has been destroyed");
  }
}

class RV64WorkerProxy {
  #worker;
  #listeners = new Map();
  #pending = new Map();
  #nextRequest = 1;
  #running = false;
  #instructions = 0n;
  #destroyed = false;
  #networkMode;
  #proxyURL;

  constructor(worker, networkMode, listeners) {
    this.#worker = worker;
    this.#networkMode = networkMode;
    for (const [event, listener] of Object.entries(listeners ?? {})) {
      this.on(event, listener);
    }
    this.console = Object.freeze({ send: (data) => this.#sendConsole(data) });
    const proxy = this;
    this.network = Object.freeze({
      mode: networkMode,
      get proxyURL() {
        return proxy.#proxyURL;
      },
      receive: (frame) => this.#receiveNetwork(frame),
    });
  }

  static async create(options) {
    if (typeof Worker === "undefined") {
      throw new Error("worker execution requires a browser Worker implementation");
    }
    const execution = options.execution;
    const workerURL = execution.workerURL
      ? new URL(execution.workerURL, import.meta.url)
      : new URL("./rv64.worker.js", import.meta.url);
    // Validate and copy transferable inputs before allocating a Worker so a
    // rejected source (notably Response) cannot leave an idle thread behind.
    const { options: clonedOptions, transfers } = cloneWorkerOptions(options);
    const worker = new Worker(workerURL, { name: "rv64.js", type: "module" });
    const networkMode =
      options.network?.mode ??
      (options.boot?.mode === "bare-metal" ? "none" : "fetch");
    const proxy = new RV64WorkerProxy(worker, networkMode, options.events);
    const created = new Promise((resolve, reject) => {
      let settled = false;
      const fail = (error) => {
        if (settled) {
          proxy.#fail(error);
        } else {
          settled = true;
          worker.terminate();
          reject(error);
        }
      };
      worker.onerror = (event) =>
        fail(event.error ?? new Error(event.message || "rv64 Worker failed"));
      worker.onmessageerror = () => fail(new Error("rv64 Worker sent an unreadable message"));
      worker.onmessage = (event) => {
        if (event.data?.type === "created") {
          settled = true;
          resolve(event.data.state);
        } else if (event.data?.type === "create-error") {
          fail(deserializeWorkerError(event.data.error));
        } else proxy.#handleMessage(event.data);
      };
    });
    worker.postMessage(
      {
        type: "create",
        options: clonedOptions,
        statisticsIntervalMs: execution.statisticsIntervalMs ?? 500,
      },
      transfers,
    );
    try {
      proxy.#applyState(await created);
      return proxy;
    } catch (error) {
      proxy.#destroyed = true;
      worker.terminate();
      throw error;
    }
  }

  get running() {
    return this.#running;
  }

  get instructions() {
    this.#assertLive();
    return this.#instructions;
  }

  async setJitEnabled(enabled) {
    this.#assertLive();
    if (typeof enabled !== "boolean") throw new TypeError("enabled must be a boolean");
    await this.#call("setJitEnabled", enabled);
  }

  async configureJit(options) {
    this.#assertLive();
    await this.#call("configureJit", options);
  }

  async jitStats() {
    this.#assertLive();
    return this.#call("jitStats");
  }

  async jitProfile(limit = 20) {
    this.#assertLive();
    return this.#call("jitProfile", limit);
  }

  on(event, listener) {
    if (!PUBLIC_EVENTS.has(event)) throw new TypeError(`unknown event: ${event}`);
    if (typeof listener !== "function") throw new TypeError("listener must be a function");
    let listeners = this.#listeners.get(event);
    if (!listeners) this.#listeners.set(event, (listeners = new Set()));
    listeners.add(listener);
    return () => listeners.delete(listener);
  }

  async start() {
    this.#assertLive();
    await this.#call("start");
  }

  async stop() {
    this.#assertLive();
    await this.#call("stop");
  }

  async reset() {
    this.#assertLive();
    await this.#call("reset");
  }

  async destroy() {
    if (this.#destroyed) return;
    this.#destroyed = true;
    try {
      await this.#call("destroy", undefined, true);
    } finally {
      this.#worker.terminate();
      this.#listeners.clear();
      for (const { reject } of this.#pending.values()) reject(new Error("RV64 instance destroyed"));
      this.#pending.clear();
    }
  }

  #call(method, value, allowDestroyed = false) {
    if (!allowDestroyed) this.#assertLive();
    const id = this.#nextRequest++;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { reject, resolve });
      this.#worker.postMessage({ id, method, type: "call", value });
    });
  }

  #handleMessage(message) {
    if (!message || typeof message !== "object") return;
    if (message.type === "event") {
      if (message.event === "start") this.#running = true;
      if (message.event === "stop") this.#running = false;
      const detail = message.event === "error" ? deserializeWorkerError(message.detail) : message.detail;
      this.#emit(message.event, detail);
      return;
    }
    if (message.type === "state") {
      this.#applyState(message.state);
      return;
    }
    if (message.type === "result") {
      const pending = this.#pending.get(message.id);
      if (!pending) return;
      this.#pending.delete(message.id);
      this.#applyState(message.state);
      if (message.error) pending.reject(deserializeWorkerError(message.error));
      else pending.resolve(message.value);
      return;
    }
  }

  #applyState(state) {
    if (!state) return;
    this.#running = state.running;
    this.#instructions = BigInt(state.instructions);
    this.#proxyURL = state.proxyURL;
  }

  #sendConsole(data) {
    this.#assertLive();
    const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
    if (!(bytes instanceof Uint8Array)) {
      throw new TypeError("console data must be a string or Uint8Array");
    }
    this.#worker.postMessage({ type: "console", value: bytes });
  }

  #receiveNetwork(frame) {
    this.#assertLive();
    if (this.#networkMode !== "external") {
      throw new Error("network.receive is only available in external mode");
    }
    const bytes = frame instanceof Uint8Array ? frame : new Uint8Array(frame);
    this.#worker.postMessage({ type: "network-receive", value: bytes });
  }

  #emit(event, detail) {
    for (const listener of [...(this.#listeners.get(event) ?? [])]) listener(detail);
  }

  #fail(error) {
    if (this.#destroyed) return;
    this.#running = false;
    this.#emit("error", error);
    this.#emit("stop", { reason: "error" });
    for (const { reject } of this.#pending.values()) reject(error);
    this.#pending.clear();
    this.#worker.terminate();
  }

  #assertLive() {
    if (this.#destroyed) throw new Error("RV64 instance has been destroyed");
  }
}

function cloneWorkerOptions(options) {
  const transfers = [];
  const cloneImage = (source, name) => {
    if (source === undefined || source === "default") return source;
    if (source instanceof Response) {
      throw new TypeError(`${name} cannot be a Response in worker execution mode`);
    }
    if (source && typeof source === "object" && typeof source.url === "string") {
      const base = globalThis.location?.href ?? import.meta.url;
      return { url: new URL(source.url, base).href };
    }
    let bytes;
    if (source instanceof ArrayBuffer) bytes = new Uint8Array(source);
    else if (ArrayBuffer.isView(source)) {
      bytes = new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
    } else {
      throw new TypeError(`${name} must be bytes or { url }`);
    }
    const copy = bytes.slice();
    transfers.push(copy.buffer);
    return copy;
  };

  const boot = { ...options.boot };
  for (const key of ["firmware", "kernel", "initrd", "disk", "image"]) {
    if (key in boot) boot[key] = cloneImage(boot[key], key);
  }
  const network = options.network
    ? {
        ...options.network,
        ...(options.network.mac
          ? { mac: cloneImage(options.network.mac, "network.mac") }
          : {}),
      }
    : undefined;
  return {
    options: {
      wasm: cloneImage(options.wasm, "wasm"),
      boot,
      memoryMB: options.memoryMB,
      ...(network ? { network } : {}),
      execution: { mode: "local" },
    },
    transfers,
  };
}

function deserializeWorkerError(value) {
  const error = new Error(value?.message ?? String(value));
  if (value?.name) error.name = value.name;
  if (value?.stack) error.stack = value.stack;
  return error;
}
