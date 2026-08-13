// FP context-switch test (PERFORMANCE_PROGRESS.md): two CONCURRENT processes running
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
import {
  bootModern,
  clearOutput,
  guestCommand,
  loadModernImages,
  machineDiagnostics,
  missingModernImages,
  output,
  pumpUntil,
  transferBinary,
  waitForAlpine,
} from "./modern-linux-harness.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
const binPath = ARTIFACTS && join(ARTIFACTS, "xbench", "rvbench_fs.rv64");
if (!ARTIFACTS || !existsSync(binPath)) {
  console.log("SKIP fp-context-switch (need ARTIFACTS with xbench/rvbench_fs.rv64)");
  process.exit(process.env.REQUIRE_ALL === "1" ? 2 : 0);
}
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const missing = missingModernImages(root);
if (missing.length) {
  console.log(`SKIP fp-context-switch (run web/prepare-images.sh; missing ${missing.join(", ")})`);
  process.exit(process.env.REQUIRE_ALL === "1" ? 2 : 0);
}
const images = await loadModernImages(root);
const enc = new TextEncoder();
const machine = await bootModern({
  RV64,
  wasm,
  images,
  mode: "direct",
  jit: true,
  superblock: true,
});
const { vm } = machine;

if (!await waitForAlpine(machine)) {
  console.log(`FAIL fp-context-switch: boot timeout\n${machineDiagnostics(machine)}`);
  process.exit(1);
}
if (!await guestCommand(
  machine,
  "stty -echo 2>/dev/null; echo FP_'READY'",
  "FP_READY",
)) {
  console.log(`FAIL fp-context-switch: shell setup timeout\n${machineDiagnostics(machine)}`);
  process.exit(1);
}
if (!await transferBinary(machine, await readFile(binPath), "/tmp/c", "FP")) {
  console.log(`FAIL fp-context-switch: transfer/decode timeout\n${machineDiagnostics(machine)}`);
  process.exit(1);
}

// run TWO copies concurrently; the kernel interleaves them on one hart
clearOutput(machine);
vm.virtConsoleInput(enc.encode("/tmp/c > /tmp/o1 & /tmp/c > /tmp/o2 & wait; echo ---; cat /tmp/o1 /tmp/o2\n"));
await pumpUntil(
  machine,
  () => (output(machine).match(/BENCH_DONE/g) || []).length >= 2 && output(machine).includes("---"),
  { slice: 5_000_000n, timeoutMs: 400_000 },
);
const out = output(machine);
const sums = [...out.matchAll(/checksum=(0x[0-9a-f]+)/g)].map((m) => m[1]);
const ok = sums.length === 2 && sums[0] === sums[1] && /^0x[0-9a-f]{8,}$/.test(sums[0]);
console.log(
  ok
    ? `FP CONTEXT SWITCH: PASS (both processes checksum ${sums[0]})`
    : `FP CONTEXT SWITCH: FAIL (checksums: ${sums.join(", ") || "none"})\n${machineDiagnostics(machine)}`,
);
process.exit(ok ? 0 : 1);
