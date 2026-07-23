// Decisive test: run a compute-heavy binary INSIDE booted Linux (system
// mode), A/B JIT on vs off, timing only the workload phase (boot excluded).
// Two workloads: "alu" (register-only) and "mix" (realistic memory+ALU).
// Answers: does the system-mode JIT help a realistic long-running compute
// workload, and does memory density decide it?
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const img = (f) => join(root, "web/images", f);
const imgs = await Promise.all(
  ["bbl64.bin", "kernel-riscv64.bin", "root-riscv64.bin"].map(async (f) => new Uint8Array(await readFile(img(f)))),
);
const bin = new Uint8Array(await readFile(join(root, "guests/syscompute/target/riscv64gc-unknown-linux-musl/release/syscompute")));
const b64 = Buffer.from(bin).toString("base64");
const REF = { alu: "fb7c3a58011655ba", mix: "ab036f91acaa986d" };

const enc = new TextEncoder();

async function boot(jit) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  const st = { vm, out: "" };
  vm.onWrite = (fd, b) => (st.out += new TextDecoder().decode(b));
  vm.bootLinux({ bios: imgs[0], kernel: imgs[1], disk: imgs[2].slice() });
  for (let i = 0; i < 40000 && !st.out.includes("~ #"); i++) vm.runSystem(5_000_000n);
  if (!st.out.includes("~ #")) throw new Error("boot failed");
  return st;
}

// run a shell command, wait for `mark`, return captured output since send.
function sh(st, cmd, mark, maxIter = 300000) {
  st.out = "";
  st.vm.consoleInput(enc.encode(cmd + "; echo " + mark + "\n"));
  const t = performance.now();
  for (let i = 0; i < maxIter && !st.out.includes(mark); i++) st.vm.runSystem(5_000_000n);
  if (!st.out.includes(mark)) throw new Error("cmd timeout: " + cmd.slice(0, 40));
  return { ms: performance.now() - t, out: st.out };
}

function inject(st) {
  // write base64 to /tmp/b64 in chunks, then decode to an executable.
  st.vm.consoleInput(enc.encode(": > /tmp/b64\n"));
  for (let i = 0; i < st.out.length; i++) {} // noop
  const CH = 512;
  for (let o = 0; o < b64.length; o += CH) {
    const chunk = b64.slice(o, o + CH);
    st.vm.consoleInput(enc.encode("printf %s '" + chunk + "' >> /tmp/b64\n"));
    for (let i = 0; i < 4000; i++) st.vm.runSystem(2_000_000n); // let it drain
  }
  sh(st, "base64 -d /tmp/b64 > /tmp/c && chmod +x /tmp/c && echo -n INJ:; wc -c < /tmp/c", "IDONE");
}

async function trial(jit, work) {
  const st = await boot(jit);
  inject(st);
  // warm once (compile), then time
  const r0 = sh(st, "/tmp/c " + work, "W0");
  const ok0 = r0.out.includes(REF[work]);
  const r1 = sh(st, "/tmp/c " + work, "W1");
  const ok1 = r1.out.includes(REF[work]);
  return { ms: Math.min(r0.ms, r1.ms), ok: ok0 && ok1, jitBlocks: Number(st.vm.ex.jit_stat(3)) };
}

const median = (a) => [...a].sort((x, y) => x - y)[(a.length / 2) | 0];
const REPS = 4;
for (const work of ["alu", "mix"]) {
  const on = [], off = [], na = [], nb = [];
  let okAll = true, blocks = 0;
  for (let i = 0; i < REPS; i++) {
    const o = await trial(false, work); off.push(o.ms); okAll &&= o.ok;
    const n = await trial(true, work); on.push(n.ms); okAll &&= n.ok; blocks = n.jitBlocks;
    na.push((await trial(true, work)).ms);
    nb.push((await trial(true, work)).ms);
  }
  const speedup = median(off) / median(on);
  const nullRatio = median(na) / median(nb);
  console.log(
    `${work}: speedup=${speedup.toFixed(3)}x  null=${nullRatio.toFixed(3)}  ` +
      `[off=${median(off).toFixed(0)}ms on=${median(on).toFixed(0)}ms]  correct=${okAll}  jitBlocks=${blocks}`,
  );
}
