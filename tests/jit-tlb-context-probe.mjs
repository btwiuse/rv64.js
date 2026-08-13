// Execute a real generated full-system memory region against three fused-TLB
// states. A stale tag from another privilege context must be a precise
// zero-retire side exit; changing only the tag's low context bits must admit
// the exact same page. This guards the Wasm emitter half of the architectural
// U/S/M isolation proof in rv64-core's unit tests.

import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const corpus = join(root, "target/jit-tlb-context-corpus");
const generated = spawnSync(
  "cargo",
  [
    "run",
    "--release",
    "-q",
    "-p",
    "rv64-dbt",
    "--example",
    "emit_backend_corpus",
    "--",
    corpus,
  ],
  { cwd: root, encoding: "utf8" },
);
if (generated.status !== 0) {
  throw new Error(generated.stderr || generated.stdout || "corpus generation failed");
}

const bytes = await readFile(join(corpus, "cached-memory.wasm"));
const FUEL = 120n; // one complete six-member memory-region cycle
const TLB_ENTRIES = 4096;
const LOAD_TAGS = 512;
const LOAD_OFFSETS = LOAD_TAGS + TLB_ENTRIES * 8;
const STORE_TAGS = LOAD_OFFSETS + TLB_ENTRIES * 8;
const STORE_OFFSETS = STORE_TAGS + TLB_ENTRIES * 8;
const ACCESS_CONTEXT = STORE_OFFSETS + TLB_ENTRIES * 8;
const TEST_PAGE_INDEX = 2;

async function run(context, tag) {
  // The fused 4K-entry load/store rows occupy a little over 128 KiB. This
  // test used the obsolete adjacent-row addresses after the corpus grew from
  // one entry, so it trapped before exercising the context comparison.
  const memory = new WebAssembly.Memory({ initial: 3 });
  const { instance } = await WebAssembly.instantiate(bytes, { env: { memory } });
  const view = new DataView(memory.buffer);
  view.setBigUint64(256, 0x1000n, true); // pc
  view.setBigUint64(264, 0n, true); // retired
  view.setBigUint64(272, FUEL, true);
  view.setBigUint64(20 * 8, 0x2000n, true); // guest VA
  view.setBigUint64(LOAD_TAGS + TEST_PAGE_INDEX * 8, tag, true);
  view.setBigInt64(LOAD_OFFSETS + TEST_PAGE_INDEX * 8, 0x2000n, true);
  view.setBigUint64(STORE_TAGS + TEST_PAGE_INDEX * 8, tag, true);
  view.setBigInt64(STORE_OFFSETS + TEST_PAGE_INDEX * 8, 0x2000n, true);
  view.setBigUint64(ACCESS_CONTEXT, context, true);
  view.setBigUint64(0x4000, 36n, true); // translated load source
  instance.exports.run(0);
  return {
    retired: view.getBigUint64(264, true),
    effect: view.getBigUint64(0x4040, true), // translated store destination
    pc: view.getBigUint64(256, true),
  };
}

const user = await run(0n, 0x2000n);
const staleUserInSupervisor = await run(1n, 0x2000n);
const supervisor = await run(1n, 0x2001n);

function show(state) {
  return JSON.stringify(state, (_key, value) =>
    typeof value === "bigint" ? value.toString() : value
  );
}

if (user.retired !== FUEL || user.effect !== 36n || user.pc !== 0x1000n) {
  throw new Error(`matching user tag failed: ${show(user)}`);
}
if (
  staleUserInSupervisor.retired !== 0n ||
  staleUserInSupervisor.effect !== 0n ||
  staleUserInSupervisor.pc !== 0x1000n
) {
  throw new Error(`stale cross-context tag was consumed: ${show(staleUserInSupervisor)}`);
}
if (
  supervisor.retired !== FUEL ||
  supervisor.effect !== 36n ||
  supervisor.pc !== 0x1000n
) {
  throw new Error(`matching supervisor tag failed: ${show(supervisor)}`);
}

console.log(
  "PASS generated TLB context probe — stale U tag precisely exits in S context",
);
