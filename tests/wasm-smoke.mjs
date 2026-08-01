// Node smoke tests for the wasm build: user-mode ELF execution (with JIT),
// JIT module validity, and (if guest images are present) a full Linux boot.
// Run via tests/run-all.sh, or directly:
//   node tests/wasm-smoke.mjs
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64, Stop } = await import(join(root, "web/rv64.js"));
const wasmBytes = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

let failures = 0;
function check(name, ok, detail = "") {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " — " + detail : ""}`);
  if (!ok) failures++;
}

// This project instantiates plain wasm32-unknown-unknown with a deliberately
// small raw ABI. In particular, TLS dependencies must not smuggle in
// wasm-bindgen/externref support that web/rv64.js does not provide.
{
  const expected = [
    "host_http_request",
    "host_jit_register",
    "host_jit_register_async",
    "host_jit_register_batch",
    "host_net_send",
    "host_now_ms",
    "host_random",
    "host_unix_ms",
    "host_write",
  ];
  const imports = WebAssembly.Module.imports(new WebAssembly.Module(wasmBytes));
  const actual = imports
    .filter((item) => item.module === "env" && item.kind === "function")
    .map((item) => item.name)
    .sort();
  const unexpected = imports.filter(
    (item) =>
      item.module !== "env" ||
      item.kind !== "function" ||
      !expected.includes(item.name),
  );
  check(
    "raw wasm import ABI",
    unexpected.length === 0 &&
      actual.length === expected.length &&
      actual.every((name, index) => name === expected[index]),
    actual.join(", "),
  );
}

// ---- user-mode guests under JIT ----
const guests = [
  ["hello-std", ["h", "x"], 0, "sum of squares 1..10 = 385"],
  ["fpu-test", ["f"], 0, "--- 0 failures"],
  ["bench", ["bench", "fast"], 0, null],
];
for (const [name, argv, wantExit, wantOut] of guests) {
  const path = join(
    root,
    `guests/${name}/target/riscv64gc-unknown-linux-musl/release/${name}`,
  );
  if (!existsSync(path)) {
    console.log(`SKIP user-mode ${name} (guest not built)`);
    continue;
  }
  const vm = await RV64.create(wasmBytes);
  let out = "";
  vm.onWrite = (fd, b) => {
    out += new TextDecoder().decode(b);
  };
  vm.loadElf(new Uint8Array(await readFile(path)), argv);
  const stop = vm.runUser(2_000_000_000n);
  const ok =
    stop === Stop.EXITED &&
    vm.userExitCode() === wantExit &&
    (wantOut === null || out.includes(wantOut));
  check(`user-mode ${name}`, ok, `exit=${vm.userExitCode()} jit-blocks=${vm.jitBlocks ?? 0}`);
}

// ---- budget contract: compiled loops must honor the caller's fuel ----
// After tier-up, a 1-instruction budget may overshoot by at most one basic
// block / loop iteration (documented granularity, MAX_BLOCK = 128) — never
// the old fixed LOOP_CAP (16.7M).
{
  const path = join(
    root,
    "guests/bench/target/riscv64gc-unknown-linux-musl/release/bench",
  );
  if (!existsSync(path)) {
    console.log("SKIP budget contract (bench guest not built)");
  } else {
    const vm = await RV64.create(wasmBytes);
    vm.onWrite = () => {};
    vm.loadElf(new Uint8Array(await readFile(path)), ["bench", "fast"]);
    // warm up so hot loops are compiled, then meter single-instruction budgets
    vm.runUser(50_000_000n);
    let worst = 0n;
    for (let i = 0; i < 2000; i++) {
      const before = vm.ex.user_insn_count();
      vm.runUser(1n);
      const d = vm.ex.user_insn_count() - before;
      if (d > worst) worst = d;
    }
    check("budget contract user_run(1)", worst <= 128n, `worst overshoot=${worst}`);
  }
}

// ---- retirement accounting: JIT and interpreter must agree EXACTLY ----
// User mode is deterministic (no interrupts), so insn_count at exit must be
// bit-identical with the JIT on and off. Mid-block bails (FP eligibility,
// budget yields) that under- or over-report retirement break this (see
// PERFORMANCE_PROGRESS.md, "Exact bailout retirement").
{
  const path = join(
    root,
    "guests/bench/target/riscv64gc-unknown-linux-musl/release/bench",
  );
  if (!existsSync(path)) {
    console.log("SKIP retirement differential (bench guest not built)");
  } else {
    const counts = [];
    for (const jit of [0, 1]) {
      const vm = await RV64.create(wasmBytes);
      vm.ex.jit_set_enabled(jit);
      vm.onWrite = () => {};
      vm.loadElf(new Uint8Array(await readFile(path)), ["bench", "fast"]);
      const stop = vm.runUser(2_000_000_000n);
      counts.push(stop === Stop.EXITED ? vm.ex.user_insn_count() : -1n);
    }
    check(
      "retirement differential (jit == interp insn_count)",
      counts[0] === counts[1] && counts[0] > 0n,
      `interp=${counts[0]} jit=${counts[1]}`,
    );
  }
}

// ---- JIT emitter: every module from arbitrary offsets must instantiate ----
{
  const vm = await RV64.create(wasmBytes);
  const elfPath = join(
    root,
    "guests/bench/target/riscv64gc-unknown-linux-musl/release/bench",
  );
  if (existsSync(elfPath)) {
    const elf = new Uint8Array(await readFile(elfPath));
    let emitted = 0,
      bad = 0;
    for (let off = 0; off + 4 < Math.min(elf.length, 4096); off += 2) {
      const ptr = vm.ex.staging_alloc(elf.length);
      new Uint8Array(vm.ex.memory.buffer, ptr, elf.length).set(elf);
      if (vm.ex.jit_translate(0n, BigInt(off)) > 0) {
        emitted++;
        const mod = new Uint8Array(
          vm.ex.memory.buffer,
          vm.ex.jit_out_ptr(),
          vm.ex.jit_out_len(),
        ).slice();
        try {
          new WebAssembly.Module(mod); // instantiation-level validation
        } catch {
          bad++;
        }
      }
    }
    check("jit emitter validity", emitted > 50 && bad === 0, `${emitted} modules, ${bad} invalid`);
  } else {
    console.log("SKIP jit emitter validity (bench guest not built)");
  }
}

// ---- full-system Linux boot (needs web/get-images.sh) ----
{
  const img = (f) => join(root, "web/images", f);
  if (!existsSync(img("bbl64.bin"))) {
    console.log("SKIP linux boot (run web/get-images.sh)");
  } else {
    const vm = await RV64.create(wasmBytes);
    let out = "";
    vm.onWrite = (fd, b) => {
      out += new TextDecoder().decode(b);
    };

    // The browser has no host filesystem, so a 9p export is an in-memory tree
    // built from a tarball the page fetched. Exercising it here is what proves
    // sys_stage_fs_tar + MemFs::load_tar + the 9p device under the JIT.
    const GUEST_MAC = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    const HOST_MAC = [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02];
    const HOST_IP = [10, 0, 2, 2];
    const GUEST_IP = [10, 0, 2, 15];

    // Frames the guest transmits arrive here via the host_net_send import —
    // the same path web/rv64.js hands to a WebSocket relay.
    const sent = [];
    vm.onNetSend = (frame) => {
      sent.push(frame);
      // Answer ARP requests for our address so the guest's stack completes a
      // neighbour entry. ARP needs no checksums, which keeps this small; the
      // full IP/ICMP peer lives in the native net_boot test.
      const reply = arpReply(frame);
      if (reply) vm.netInput(reply);
    };

    vm.bootLinux({
      bios: new Uint8Array(await readFile(img("bbl64.bin"))),
      kernel: new Uint8Array(await readFile(img("kernel-riscv64.bin"))),
      disk: new Uint8Array(await readFile(img("root-riscv64.bin"))),
      fsTar: tarball([["greeting.txt", "from-the-tarball\n"]]),
      fsTag: "hostshare",
      net: true,
    });
    for (let i = 0; i < 20000 && !out.includes("~ #"); i++) {
      vm.runSystem(10_000_000n);
    }
    const gotShell = out.includes("~ #");

    // Run `cmd` and wait for `marker`. The guest echoes what we type, so a
    // marker must not appear verbatim in the command — quoting a character
    // (OK_'x') keeps the typed and printed forms different.
    const run = (cmd, marker, slices = 800) => {
      if (!gotShell) return false;
      if (cmd.includes(marker)) throw new Error(`marker ${marker} would match the echo`);
      vm.consoleInput(new TextEncoder().encode(cmd + "\n"));
      for (let i = 0; i < slices && !out.includes(marker); i++) {
        vm.runSystem(10_000_000n);
      }
      return out.includes(marker);
    };

    const cmdOk = run("echo smoke-$((6*7))", "smoke-42");
    check("linux boot (wasm)", gotShell && cmdOk, `jit-blocks=${vm.jitBlocks ?? 0}`);

    // 9p, in the browser's configuration: mount the tar-backed export and read
    // a file out of it.
    const mounted = run(
      "mkdir -p /9p && mount -t 9p -o trans=virtio,version=9p2000.L hostshare /9p && echo MOUNT_'O'K",
      "MOUNT_OK",
    );
    const read9p = mounted && run("cat /9p/greeting.txt", "from-the-tarball");
    check("virtio-9p over tar (wasm)", mounted && read9p);

    // virtio-net: frames out through host_net_send, frames in through
    // sys_net_input, both under the JIT.
    run(`ifconfig eth0 ${GUEST_IP.join(".")} netmask 255.255.255.0 up && echo UP_'O'K`, "UP_OK");
    // Several attempts so an ARP reply is never racing a single timeout.
    run(`ping -c 3 -W 2 ${HOST_IP.join(".")}`, "packets transmitted", 2000);
    const arpRequest = sent.find(
      (f) =>
        f.length >= 42 &&
        f[12] === 0x08 &&
        f[13] === 0x06 &&
        [...f.slice(38, 42)].join(".") === HOST_IP.join("."),
    );
    const macOnWire =
      arpRequest && [...arpRequest.slice(6, 12)].join(",") === GUEST_MAC.join(",");
    check(
      "virtio-net TX + MAC from config space (wasm)",
      !!arpRequest && macOnWire,
      `frames=${sent.length}`,
    );
    // The guest only records a neighbour entry if it parsed a frame we injected.
    const hostMacStr = HOST_MAC.map((b) => b.toString(16).padStart(2, "0")).join(":");
    const arpTable = run("cat /proc/net/arp", hostMacStr);
    check("virtio-net RX into the guest stack (wasm)", arpTable);

    function tarball(entries) {
      const blocks = [];
      for (const [name, content] of entries) {
        const data = new TextEncoder().encode(content);
        const b = new Uint8Array(512 + Math.ceil(data.length / 512) * 512);
        const put = (off, s) => {
          for (let i = 0; i < s.length; i++) b[off + i] = s.charCodeAt(i);
        };
        put(0, name);
        put(100, "0000644\0"); // mode
        put(124, data.length.toString(8).padStart(11, "0") + "\0"); // size
        b[156] = 0x30; // typeflag '0' = regular file
        put(257, "ustar");
        b.set(data, 512);
        blocks.push(b);
      }
      blocks.push(new Uint8Array(1024)); // end-of-archive
      const total = blocks.reduce((n, b) => n + b.length, 0);
      const tar = new Uint8Array(total);
      let off = 0;
      for (const b of blocks) {
        tar.set(b, off);
        off += b.length;
      }
      return tar;
    }

    /** ARP reply for a request aimed at HOST_IP, else null. */
    function arpReply(f) {
      if (f.length < 42 || f[12] !== 0x08 || f[13] !== 0x06) return null;
      if (f[21] !== 1) return null; // not a request
      if ([...f.slice(38, 42)].join(".") !== HOST_IP.join(".")) return null;
      const senderMac = f.slice(6, 12);
      const senderIp = f.slice(28, 32);
      const r = new Uint8Array(42);
      r.set(senderMac, 0);
      r.set(HOST_MAC, 6);
      r[12] = 0x08;
      r[13] = 0x06;
      r[15] = 1; // htype: ethernet
      r[16] = 0x08; // ptype: ipv4
      r[18] = 6;
      r[19] = 4;
      r[21] = 2; // oper: reply
      r.set(HOST_MAC, 22);
      r.set(HOST_IP, 28);
      r.set(senderMac, 32);
      r.set(senderIp, 38);
      return r;
    }
  }
}

// ---- in-process HTTP proxy over fetch egress (needs web/get-images.sh) ----
//
// A separate boot from the relay test above: with the proxy enabled the guest's
// frames go to the built-in netstack, so host_net_send never fires and the two
// paths cannot share one machine.
{
  const img = (f) => join(root, "web/images", f);
  if (!existsSync(img("bbl64.bin"))) {
    console.log("SKIP http proxy (run web/get-images.sh)");
  } else {
    // A real origin on loopback, reached by the same fetch() a browser uses.
    // Nothing external is involved, but the entire egress path runs.
    const origin = createServer((req, res) => {
      if (req.url === "/hello") {
        res.writeHead(200, { "Content-Type": "text/plain" });
        res.end("FETCH-EGRESS-OK\n");
      } else if (req.url === "/big") {
        // Written in pieces so the response reaches the guest as a stream of
        // body chunks rather than one buffered blob.
        res.writeHead(200);
        for (let i = 0; i < 20; i++) res.write("x".repeat(1000));
        res.end();
      } else {
        res.writeHead(404);
        res.end("nope\n");
      }
    });
    await new Promise((r) => origin.listen(0, "127.0.0.1", r));
    const port = origin.address().port;

    // A deterministic request-level relay. fetch() below is made to fail for
    // one synthetic origin, exactly as a browser does when CORS hides the
    // response; the relay then streams a response through the same wasm ABI.
    let relayRequests = 0;
    class FakeHttpRelay {
      constructor() {
        this.readyState = 0;
        queueMicrotask(() => {
          this.readyState = 1;
          this.onopen?.();
        });
      }
      send(data) {
        const request = new Uint8Array(data);
        if (
          request[0] !== 0x52 ||
          request[1] !== 0x48 ||
          request[2] !== 0x52 ||
          request[3] !== 0x31 ||
          request[4] !== 1
        ) {
          throw new Error("bad request relay frame");
        }
        const id = new DataView(
          request.buffer,
          request.byteOffset,
        ).getBigUint64(8, true);
        relayRequests++;
        const frame = (type, payload = new Uint8Array()) => {
          const out = new Uint8Array(16 + payload.length);
          out.set([0x52, 0x48, 0x52, 0x31]);
          out[4] = type;
          new DataView(out.buffer).setBigUint64(8, id, true);
          out.set(payload, 16);
          return out.buffer;
        };
        const head = new Uint8Array(8);
        new DataView(head.buffer).setUint32(0, 200, true);
        const body = new TextEncoder().encode(`RELAY-FALLBACK-${relayRequests}\n`);
        queueMicrotask(() => {
          this.onmessage?.({ data: frame(2, head) });
          this.onmessage?.({ data: frame(3, body) });
          this.onmessage?.({ data: frame(4) });
        });
      }
      close() {
        this.readyState = 3;
        queueMicrotask(() => this.onclose?.());
      }
    }

    const vm = await RV64.create(wasmBytes);
    let out = "";
    vm.onWrite = (fd, b) => {
      out += new TextDecoder().decode(b);
    };
    vm.connectHttpRelay("ws://test.invalid", {
      WebSocket: FakeHttpRelay,
      timeoutMs: 1000,
    });
    vm.bootLinux({
      bios: new Uint8Array(await readFile(img("bbl64.bin"))),
      kernel: new Uint8Array(await readFile(img("kernel-riscv64.bin"))),
      disk: new Uint8Array(await readFile(img("root-riscv64.bin"))),
      proxy: true,
      // The canned origin is plaintext; a real page served over https needs the
      // default (upgrade), since https pages cannot fetch http:// at all.
      proxyUpgradeHttps: false,
    });

    // fetch() completes on the microtask queue, so the wait loop must yield to
    // the event loop between slices; a synchronous spin would never see it.
    const step = async (marker, rounds) => {
      for (let i = 0; i < rounds && !out.includes(marker); i++) {
        vm.runSystem(10_000_000n);
        await new Promise((r) => setImmediate(r));
      }
      return out.includes(marker);
    };
    const run = async (cmd, marker, rounds = 600) => {
      if (cmd.includes(marker)) throw new Error(`marker ${marker} matches the echo`);
      vm.consoleInput(new TextEncoder().encode(cmd + "\n"));
      return step(marker, rounds);
    };

    const gotShell = await step("~ #", 20000);
    const url = gotShell ? vm.proxyURL() : "";
    check("proxy URL is reported", url.startsWith("http://10.0.2.2:"), url);
    let ok = false;
    let big = false;
    let fallback = false;
    let cached = false;
    let blockedFetches = 0;
    if (gotShell) {
      await run("ifconfig eth0 10.0.2.15 netmask 255.255.255.0 up && echo NET_'O'K", "NET_OK");
      await run(`export http_proxy=${url}; echo ENV_'O'K`, "ENV_OK");
      ok = await run(`wget -q -O- http://127.0.0.1:${port}/hello`, "FETCH-EGRESS-OK");
      // Proves the streamed body reassembles: the origin writes 20 chunks.
      big = await run(`wget -q -O- http://127.0.0.1:${port}/big | wc -c`, "20000");

      const realFetch = globalThis.fetch;
      globalThis.fetch = (resource, init) => {
        if (String(resource).startsWith("http://cors-blocked.invalid")) {
          blockedFetches++;
          return Promise.reject(new TypeError("Failed to fetch"));
        }
        return realFetch(resource, init);
      };
      try {
        fallback = await run(
          "wget -q -O- http://cors-blocked.invalid/first",
          "RELAY-FALLBACK-1",
        );
        // The first safe failure caches this origin, so no second failed fetch
        // is paid and later methods can be routed without duplicate delivery.
        cached = await run(
          "wget -q -O- http://cors-blocked.invalid/second",
          "RELAY-FALLBACK-2",
        );
      } finally {
        globalThis.fetch = realFetch;
      }
    }
    check("http proxy over fetch egress (wasm)", ok, `shell=${gotShell}`);
    check("streamed response through the proxy (wasm)", big);
    check(
      "per-origin fetch-to-relay fallback (wasm)",
      fallback && cached && blockedFetches === 1 && relayRequests === 2,
      `fetch-failures=${blockedFetches} relay-requests=${relayRequests}`,
    );
    vm.disconnectHttpRelay();
    origin.close();
  }
}

console.log(failures === 0 ? "WASM SMOKE: ALL PASS" : `WASM SMOKE: ${failures} FAILURES`);
process.exit(failures === 0 ? 0 : 1);
