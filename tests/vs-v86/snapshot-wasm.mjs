// Save an immutable wasm candidate plus enough source provenance to reproduce
// it. Run immediately after building, before editing the next candidate.
//
//   ARTIFACTS=target/bench node tests/vs-v86/snapshot-wasm.mjs head-control
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { constants } from "node:fs";
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const artifactsArg = process.env.ARTIFACTS || process.env.SC;
if (!artifactsArg) {
  console.error("set ARTIFACTS=<artifacts dir>");
  process.exit(2);
}
const ARTIFACTS = resolve(artifactsArg);
const label = (process.argv[2] || "").replace(/[^a-zA-Z0-9._-]/g, "-");
if (!label) {
  console.error("usage: snapshot-wasm.mjs <label>");
  process.exit(2);
}
const source =
  (process.env.WASM ? resolve(process.env.WASM) : null) ||
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
const bytes = await readFile(source);
const wasmSha = createHash("sha256").update(bytes).digest("hex");
const git = (...args) => {
  try {
    return execFileSync("git", args, { encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
};
const diff = git("-C", root, "diff", "--binary", "HEAD");
const staged = git("-C", root, "diff", "--binary", "--cached", "HEAD");
const untracked = git(
  "-C",
  root,
  "ls-files",
  "--others",
  "--exclude-standard",
);
const sourceStateHash = createHash("sha256")
  .update(diff)
  .update("\0")
  .update(staged)
  .update("\0");
for (const path of untracked.split("\n").filter(Boolean).sort()) {
  sourceStateHash.update(path).update("\0");
  try {
    sourceStateHash.update(await readFile(join(root, path)));
  } catch {
    sourceStateHash.update("<unreadable>");
  }
  sourceStateHash.update("\0");
}
const sourceState = sourceStateHash.digest("hex");

const outDir = join(ARTIFACTS, "wasm-candidates");
await mkdir(outDir, { recursive: true });
const stem = `${label}-${wasmSha.slice(0, 12)}`;
const wasmOut = join(outDir, `${stem}.wasm`);
const manifestOut = join(outDir, `${stem}.json`);
await copyFile(source, wasmOut, constants.COPYFILE_EXCL);
await writeFile(
  manifestOut,
  JSON.stringify(
    {
      schema: 1,
      label,
      created: new Date().toISOString(),
      wasm: wasmOut,
      wasm_sha256: wasmSha,
      git: git("-C", root, "rev-parse", "HEAD"),
      git_status: git("-C", root, "status", "--short"),
      source_state_sha256: sourceState,
      node: process.version,
    },
    null,
    2,
  ),
  { flag: "wx" },
);
console.log(`saved ${wasmOut}`);
console.log(`manifest ${manifestOut}`);
