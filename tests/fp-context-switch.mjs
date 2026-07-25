// FP context-switch test (ISSUES.md P1): two CONCURRENT processes running
// the same FP-checksummed binary — same physical FP code pages, compiled
// blocks shared across both address spaces — must each produce the correct
// checksum while the kernel context-switches them (mstatus.FS transitions
// Initial -> Dirty per process, lazy FP save/restore, satp switches with
// block survival). A JIT that executed FP with stale FS state, leaked FP
// registers across processes, or mixed up per-process translations would
// corrupt at least one checksum.
//
// (Direct FS=Off trap equivalence is asserted at the interpreter level;
// here every exec exercises the Initial->Dirty transition through the
// hoisted per-block FP gate under JIT.)
//
//   ARTIFACTS=<dir-with-xbench> node tests/fp-context-switch.mjs
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
const binPath = ARTIFACTS && join(ARTIFACTS, "xbench", "rvbench_fs.rv64");
if (!ARTIFACTS || !existsSync(binPath)) {
  console.log("SKIP fp-context-switch (need ARTIFACTS with xbench/rvbench_fs.rv64)");
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

const vm = await RV64.create(wasm);
vm.ex.jit_set_enabled(1);
vm.ex.sys_set_superblock(1); // the config the scorecard runs
let out = "";
vm.onWrite = (fd, b) => (out += new TextDecoder().decode(b));
vm.bootLinux({ bios, kernel, disk: disk.slice() });
for (let i = 0; i < 40000 && !out.includes("~ #"); i++) {
  vm.runSystem(5_000_000n);
  if ((i & 15) === 0) await tick();
}
if (!out.includes("~ #")) { console.log("FAIL fp-context-switch: boot"); process.exit(1); }
vm.consoleInput(enc.encode("stty -echo 2>/dev/null\n"));
for (let i = 0; i < 3000; i++) vm.runSystem(2_000_000n);
const b64 = Buffer.from(await readFile(binPath)).toString("base64");
vm.consoleInput(enc.encode(": > /tmp/b\n"));
for (let i = 0; i < 1500; i++) vm.runSystem(2_000_000n);
for (let o = 0; o < b64.length; o += 512) {
  vm.consoleInput(enc.encode(`printf %s '${b64.slice(o, o + 512)}' >> /tmp/b\n`));
  for (let i = 0; i < 3000; i++) vm.runSystem(2_000_000n);
  await tick();
}
vm.consoleInput(enc.encode("base64 -d /tmp/b > /tmp/c && chmod 755 /tmp/c\n"));
for (let i = 0; i < 12000; i++) vm.runSystem(2_000_000n);

// run TWO copies concurrently; the kernel interleaves them on one hart
out = "";
vm.consoleInput(enc.encode("/tmp/c > /tmp/o1 & /tmp/c > /tmp/o2 & wait; echo ---; cat /tmp/o1 /tmp/o2\n"));
const t = performance.now();
for (let i = 0; i < 4_000_000; i++) {
  vm.runSystem(5_000_000n);
  if ((i & 15) === 0) await tick();
  const done = (out.match(/BENCH_DONE/g) || []).length;
  if (done >= 2 && out.includes("---")) break;
  if (performance.now() - t > 400000) break;
}
const sums = [...out.matchAll(/checksum=(0x[0-9a-f]+)/g)].map((m) => m[1]);
const ok = sums.length === 2 && sums[0] === sums[1] && /^0x[0-9a-f]{8,}$/.test(sums[0]);
console.log(
  ok
    ? `FP CONTEXT SWITCH: PASS (both processes checksum ${sums[0]})`
    : `FP CONTEXT SWITCH: FAIL (checksums: ${sums.join(", ") || "none"})\ntail: ${out.slice(-300)}`,
);
process.exit(ok ? 0 : 1);
