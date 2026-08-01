// System-mode JIT benchmark: run a compute-heavy binary INSIDE booted
// Linux, A/B JIT on vs off, timing only the workload phase. This is the
// representative test the earlier md5/boot/shell benchmarks lacked — a
// long-running compute workload with hot-code reuse, which is what a JIT
// is actually for.
//
// Two workloads isolate the memory-density variable:
//   alu : register-only xorshift accumulate (pure decode-elimination win)
//   mix : repeated transform over a 256 KiB array (loads+stores+ALU+branch)
//
// The guest tty echoes input, so we DISABLE echo (stty -echo) and wait for
// the binary's actual 16-hex checksum — never a shell word-mark, which
// would match the echoed command and measure nothing.
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const img = (f) => join(root, "web/images", f);
const imgs = await Promise.all(
  ["bbl64.bin", "kernel-riscv64.bin", "root-riscv64.bin"].map(async (f) => new Uint8Array(await readFile(img(f)))),
);
const bin = new Uint8Array(
  await readFile(join(root, "guests/syscompute/target/riscv64gc-unknown-linux-musl/release/syscompute")),
);
const b64 = Buffer.from(bin).toString("base64");
const enc = new TextEncoder();
const REF = { alu: "fb7c3a58011655ba", mix: "ab036f91acaa986d" };

async function bootAndInject(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  let out = "";
  vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  const st = { vm, get out() { return out; }, set out(v) { out = v; } };
  vm.bootLinux({ bios: imgs[0], kernel: imgs[1], disk: imgs[2].slice() });
  for (let i = 0; i < 40000 && !out.includes("~ #"); i++) vm.runSystem(5_000_000n);
  if (!out.includes("~ #")) throw new Error("boot failed");
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n"));
  for (let i = 0; i < 3000; i++) vm.runSystem(2_000_000n);
  vm.consoleInput(enc.encode(": > /tmp/b64\n"));
  for (let i = 0; i < 2000; i++) vm.runSystem(2_000_000n);
  for (let o = 0; o < b64.length; o += 512) {
    vm.consoleInput(enc.encode("printf %s '" + b64.slice(o, o + 512) + "' >> /tmp/b64\n"));
    for (let i = 0; i < 4000; i++) vm.runSystem(2_000_000n);
  }
  vm.consoleInput(enc.encode("base64 -d /tmp/b64 > /tmp/c && chmod 755 /tmp/c\n"));
  for (let i = 0; i < 20000; i++) vm.runSystem(2_000_000n);
  return st;
}

function runWorkload(st, work) {
  st.out = "";
  st.vm.consoleInput(enc.encode("/tmp/c " + work + "\n"));
  const t = performance.now();
  for (let i = 0; i < 400000; i++) {
    st.vm.runSystem(5_000_000n);
    if (/[0-9a-f]{16}/.test(st.out)) break;
    if (performance.now() - t > 60000) break;
  }
  const h = (st.out.match(/[0-9a-f]{16}/) || ["<none>"])[0];
  return { ms: performance.now() - t, ok: h === REF[work], hash: h };
}

async function trial(jit, work) {
  const st = await bootAndInject(jit);
  runWorkload(st, work); // warm (compile)
  return runWorkload(st, work); // timed
}

const median = (a) => [...a].sort((x, y) => x - y)[(a.length / 2) | 0];
const REPS = 3;
console.log("workload  speedup   null   [jit-off  jit-on]   correct");
for (const work of ["alu", "mix"]) {
  const off = [], on = [], na = [], nb = [];
  let ok = true;
  for (let i = 0; i < REPS; i++) {
    const a = await trial(false, work); off.push(a.ms); ok &&= a.ok;
    const b = await trial(true, work); on.push(b.ms); ok &&= b.ok;
    na.push((await trial(true, work)).ms);
    nb.push((await trial(true, work)).ms);
  }
  const sp = median(off) / median(on);
  const nl = median(na) / median(nb);
  console.log(
    `${work.padEnd(9)} ${sp.toFixed(2)}x   ${nl.toFixed(3)}  ` +
      `[${median(off).toFixed(0)}ms ${median(on).toFixed(0)}ms]   ${ok ? "yes" : "NO"}`,
  );
}
