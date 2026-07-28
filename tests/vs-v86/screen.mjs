// Very fast parallel smoke screen for the current rv64 wasm/config.
//
// Parallel contention makes absolute timings unsuitable for claims. Use this
// only to reject broken or dramatically bad ideas, then use ab.mjs for serial
// paired evidence.
//
//   ARTIFACTS=target/bench node tests/vs-v86/screen.mjs compile 4
//   ARTIFACTS=target/bench TRACELVL=0 node tests/vs-v86/screen.mjs numeric 3
import { spawn } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acquireBenchmarkLock } from "./bench-lock.mjs";
import { median } from "./bench-math.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const artifactsArg = process.env.ARTIFACTS || process.env.SC;
if (!artifactsArg) {
  console.error("set ARTIFACTS=<artifacts dir>");
  process.exit(2);
}
const ARTIFACTS = resolve(artifactsArg);
const aliases = { cc: "compile", py: "python", nb: "nbench" };
const requestedRow = (process.argv[2] || "compile").toLowerCase();
const row = aliases[requestedRow] || requestedRow;
const validRows = new Set([
  "alu", "mixed", "boot", "python", "compile", "nbench", "numeric", "string",
  "bitfield", "fpemul", "fourier", "assignment", "idea", "huffman",
]);
if (!validRows.has(row)) {
  console.error(`unknown row "${row}"`);
  process.exit(2);
}
const K = Number(process.argv[3] ?? 4);
if (!Number.isSafeInteger(K) || K < 1) {
  console.error("worker count must be a positive integer");
  process.exit(2);
}
const releaseBenchmarkLock = await acquireBenchmarkLock(ARTIFACTS);

function runOne() {
  return new Promise((resolveRun) => {
    const child = spawn(
      process.execPath,
      [join(root, "tests/vs-v86/rv64-scorecard-worker.mjs"), row],
      { cwd: root, env: { ...process.env, ARTIFACTS, DISABLE_JIT: "0" } },
    );
    let stdout = "";
    child.stdout.on("data", (data) => (stdout += data));
    child.on("close", (code) => {
      const match = stdout.match(/^RESULT_JSON (.+)$/m);
      if (code !== 0 || !match) return resolveRun(null);
      try {
        resolveRun(JSON.parse(match[1]));
      } catch {
        resolveRun(null);
      }
    });
  });
}

const started = performance.now();
const results = (await Promise.all(Array.from({ length: K }, runOne))).filter(Boolean);
const elapsed = (performance.now() - started) / 1000;
if (results.length !== K) {
  console.error(`only ${results.length}/${K} workers completed`);
  process.exitCode = 1;
}
const printValues = (name, values) => {
  const clean = values.filter(Number.isFinite);
  if (!clean.length) return;
  console.log(
    `${name}: median=${median(clean)} min=${Math.min(...clean)} max=${Math.max(...clean)} n=${clean.length}`,
  );
};

console.log(
  `PARALLEL SMOKE SCREEN (${results.length}/${K} boots, ${elapsed.toFixed(1)}s wall)`,
);
if (row === "nbench") {
  const kernels = new Set(results.flatMap((result) => Object.keys(result.value)));
  for (const kernel of kernels) {
    printValues(kernel, results.map((result) => result.value[kernel]));
  }
} else {
  printValues(row, results.map((result) => result.value));
}
const fingerprints = [
  ...new Set(results.map((result) => result.md5 ?? result.checksum).filter(Boolean)),
];
if (fingerprints.length) console.log(`correctness=${fingerprints.join(",")}`);
console.log("REJECTION SCREEN ONLY — use serial ab.mjs before making a claim.");
await releaseBenchmarkLock();
