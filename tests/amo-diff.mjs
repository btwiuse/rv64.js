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
import {
  bootModern,
  clearOutput,
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
const binPath = ARTIFACTS && join(ARTIFACTS, "xbench", "amo.rv64");
if (!ARTIFACTS || !existsSync(binPath)) {
  console.log("SKIP amo-diff (need ARTIFACTS with xbench/amo.rv64)");
  process.exit(process.env.REQUIRE_ALL === "1" ? 2 : 0);
}
const { RV64Debug: RV64 } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const missing = missingModernImages(root);
if (missing.length) {
  console.log(`SKIP amo-diff (run web/prepare-images.sh; missing ${missing.join(", ")})`);
  process.exit(process.env.REQUIRE_ALL === "1" ? 2 : 0);
}
const images = await loadModernImages(root);
const enc = new TextEncoder();
const binary = await readFile(binPath);

async function run(label, jit, superblock) {
  const machine = await bootModern({
    RV64,
    wasm,
    images,
    mode: "direct",
    jit,
    superblock,
  });
  const { vm } = machine;

  const failed = (phase) => ({
    checksum: null,
    error: `${label} ${phase}; ${machineDiagnostics(machine)}`,
  });

  if (!await waitForAlpine(machine)) return failed("boot timeout");
  if (!await transferBinary(machine, binary, "/tmp/a", "AMO")) {
    return failed("transfer/decode timeout");
  }
  clearOutput(machine);
  vm.virtConsoleInput(enc.encode("/tmp/a\n"));
  await pumpUntil(machine, () => /checksum=0x[0-9a-f]{16}/.test(output(machine)), {
    slice: 2_000_000n,
    timeoutMs: 300_000,
  });
  const out = output(machine);
  const checksum = (out.match(/checksum=(0x[0-9a-f]{16})/) || [])[1] ?? null;
  return checksum ? { checksum, error: null } : failed("guest-program timeout");
}

const interp = await run("interpreter", false, false);
const jit = await run("jit", true, false);
const sb = await run("superblock", true, true);
const ok = interp.checksum && interp.checksum === jit.checksum && interp.checksum === sb.checksum;
console.log(
  ok
    ? `AMO DIFFERENTIAL: PASS (interp == jit == superblock, checksum ${interp.checksum})`
    : `AMO DIFFERENTIAL: FAIL interp=${interp.checksum} jit=${jit.checksum} ` +
      `superblock=${sb.checksum}\n${[interp.error, jit.error, sb.error].filter(Boolean).join("\n")}`,
);
process.exit(ok ? 0 : 1);
