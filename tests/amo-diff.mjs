// Atomics differential (system emulation): the same in-guest program, run with
// the JIT enabled and disabled, must produce an identical checksum. The
// user-mode instruction fuzzer cannot reach AMO — those only compile in the
// system layout — so this is what covers them.
//
//   ARTIFACTS=<dir-with-xbench/amo.rv64> node tests/amo-diff.mjs
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
const binPath = ARTIFACTS && join(ARTIFACTS, "xbench", "amo.rv64");
if (!ARTIFACTS || !existsSync(binPath)) {
  console.log("SKIP amo-diff (need ARTIFACTS with xbench/amo.rv64)");
  process.exit(0);
}
const { RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const img = (f) => readFile(join(root, "web/images", f)).then((b) => new Uint8Array(b));
const [bios, kernel, disk] = await Promise.all([
  img("bbl64.bin"), img("kernel-riscv64.bin"), img("root-riscv64.bin"),
]);
const enc = new TextEncoder();
const tick = () => new Promise((r) => setImmediate(r));
const b64 = Buffer.from(await readFile(binPath)).toString("base64");

async function run(jit, superblock) {
  const vm = await RV64.create(wasm);
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  if (jit && superblock) vm.ex.sys_set_superblock(1);
  let out = "";
  vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
  vm.bootLinux({ bios, kernel, disk: disk.slice() });
  for (let i = 0; i < 60000 && !out.includes("~ #"); i++) {
    vm.runSystem(5_000_000n);
    if ((i & 15) === 0) await tick();
  }
  if (!out.includes("~ #")) return null;
  vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n"));
  for (let i = 0; i < 2000; i++) vm.runSystem(2_000_000n);
  vm.consoleInput(enc.encode(": > /tmp/b\n"));
  for (let i = 0; i < 1500; i++) vm.runSystem(2_000_000n);
  for (let o = 0; o < b64.length; o += 512) {
    vm.consoleInput(enc.encode(`printf %s '${b64.slice(o, o + 512)}' >> /tmp/b\n`));
    for (let i = 0; i < 3000; i++) vm.runSystem(2_000_000n);
    await tick();
  }
  out = "";
  vm.consoleInput(enc.encode("base64 -d /tmp/b > /tmp/a && chmod 755 /tmp/a && /tmp/a\n"));
  const t = performance.now();
  for (let i = 0; i < 4_000_000; i++) {
    vm.runSystem(2_000_000n);
    if ((i & 15) === 0) await tick();
    if (/checksum=0x[0-9a-f]{16}/.test(out)) break;
    if (performance.now() - t > 300000) break;
  }
  return (out.match(/checksum=(0x[0-9a-f]{16})/) || [])[1] ?? null;
}

const interp = await run(false, false);
const jit = await run(true, false);
const sb = await run(true, true);
const ok = interp && interp === jit && interp === sb;
console.log(
  ok
    ? `AMO DIFFERENTIAL: PASS (interp == jit == superblock, checksum ${interp})`
    : `AMO DIFFERENTIAL: FAIL interp=${interp} jit=${jit} superblock=${sb}`,
);
process.exit(ok ? 0 : 1);
