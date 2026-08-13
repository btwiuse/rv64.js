import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64Debug } = await import(join(root, "web/rv64.js"));
const wasm = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);
const vm = await RV64Debug.create(wasm);
if (typeof vm.ex.sbtest_structured_fuel_coalescing !== "function") {
  throw new Error("build rv64-wasm with --features jit-test-exports");
}
const result = vm.ex.sbtest_structured_fuel_coalescing();
if (result !== 0n) {
  throw new Error(`structured fuel coalescing failed: 0x${result.toString(16)}`);
}
console.log("JIT STRUCTURED FUEL COALESCING: PASS (4 entries x 65 fuel grants)");
