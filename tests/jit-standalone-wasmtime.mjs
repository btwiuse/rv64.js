// Standalone engine execution and cold-frontend gate. Wasmtime links each
// frozen JIT module under the name `jit`, a tiny generated module under `env`
// supplies linear memory, and a driver invokes/validates exact architectural
// effects. Real-region modules are separately AOT-compiled in fresh processes.

import { createHash } from "node:crypto";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const provisionedWasmtime = join(root, "target/tools/wasmtime-47.0.3/wasmtime");
const wasmtime = process.env.WASMTIME ||
  (existsSync(provisionedWasmtime) ? provisionedWasmtime : "wasmtime");
const version = spawnSync(wasmtime, ["--version"], { encoding: "utf8" });
if (version.status !== 0) {
  console.log("SKIP standalone Wasmtime gate (set WASMTIME to a Wasmtime 47+ executable)");
  process.exit(process.env.REQUIRE_ALL === "1" ? 2 : 0);
}

// Both emitters replace their output files. A fixed target directory lets two
// otherwise independent release-matrix invocations link a driver from one run
// against a corpus member from the other. Keep the generated pair private to
// this process so the execution gate remains deterministic under concurrency.
const scratchDir = mkdtempSync(join(tmpdir(), "rv64-jit-standalone-"));
process.once("exit", () => rmSync(scratchDir, { recursive: true, force: true }));
const corpusDir = join(scratchDir, "corpus");
const gateDir = join(scratchDir, "gate");
for (const [example, output] of [
  ["emit_backend_corpus", corpusDir],
  ["emit_standalone_gate", gateDir],
]) {
  const generated = spawnSync(
    "cargo", ["run", "--release", "-q", "-p", "rv64-dbt", "--example", example, "--", output],
    { cwd: root, encoding: "utf8" },
  );
  if (generated.status !== 0) throw new Error(generated.stderr || generated.stdout);
}

const variants = [
  "cached", "lazy", "direct", "materialized", "tailcall",
  "cached-memory", "cached-memory-no-tlb", "lazy-memory", "direct-memory",
  "materialized-memory", "tailcall-memory",
];
const executions = [];
for (const variant of variants) {
  const modulePath = join(corpusDir, `${variant}.wasm`);
  const driver = join(gateDir, variant.includes("memory")
    ? "driver-memory.wasm"
    : "driver-integer.wasm");
  const started = performance.now();
  const result = spawnSync(
    wasmtime,
    [
      "run", "--preload", `env=${join(gateDir, "env.wasm")}`,
      "--preload", `jit=${modulePath}`, "--invoke", "check", driver,
    ],
    { encoding: "utf8" },
  );
  const wallMs = performance.now() - started;
  if (result.status !== 0) {
    throw new Error(`${variant}: ${result.stderr || result.stdout || "Wasmtime execution failed"}`);
  }
  const bytes = readFileSync(modulePath);
  executions.push({
    variant,
    bytes: bytes.length,
    hash: createHash("sha256").update(bytes).digest("hex"),
    wallMs,
  });
}

// Compiling every 55-MiB real-region variant through a new CLI process is an
// intentionally separate, opt-in diagnostic. The ordinary gate samples the
// production eager modules at all geometries, retaining per-module raw wall
// latency without polluting execution correctness with arbitrary captured PCs.
const realArg = process.argv.find((argument) => argument.startsWith("--real-corpus="));
const realDir = realArg?.split("=")[1];
const realCompiles = [];
if (realDir) {
  const lines = readFileSync(join(realDir, "manifest.tsv"), "utf8").trim().split("\n");
  const header = lines.shift().split("\t");
  const records = lines.map((line) =>
    Object.fromEntries(header.map((name, index) => [name, line.split("\t")[index]])))
    .filter((record) => record.mode === "eager");
  for (const record of records) {
    const modulePath = join(realDir, record.wasm);
    const started = performance.now();
    const result = spawnSync(wasmtime, ["compile", "-o", "/dev/null", modulePath], {
      encoding: "utf8",
    });
    const wallMs = performance.now() - started;
    if (result.status !== 0) {
      throw new Error(`${record.wasm}: ${result.stderr || result.stdout}`);
    }
    realCompiles.push({
      wasm: record.wasm,
      workload: record.workload,
      pages: Number(record.pages),
      leaderCap: Number(record.leader_cap),
      entries: Number(record.entries),
      bytes: Number(record.bytes),
      wallMs,
    });
  }
}

const report = {
  schema: 1,
  engine: version.stdout.trim(),
  methodology: "fresh-wasmtime-cli-process-per-module/preloaded-env-and-jit/driver-verified-state",
  executions,
  realCompiles,
};
const outputArg = process.argv.find((argument) => argument.startsWith("--output="));
if (outputArg) writeFileSync(outputArg.split("=")[1], `${JSON.stringify(report, null, 2)}\n`);
if (process.argv.includes("--json")) console.log(JSON.stringify(report, null, 2));
else {
  console.log(
    `PASS standalone ${report.engine}: executed ${executions.length} frozen backend modules` +
      (realCompiles.length ? `; AOT-compiled ${realCompiles.length} eager real regions` : ""),
  );
}
