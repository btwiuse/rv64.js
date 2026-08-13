#!/usr/bin/env node

// Compile exactly one Wasm file in a fresh V8 process. File I/O and process
// startup are deliberately outside the reported interval.

import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

const path = process.argv[2];
if (!path) throw new Error("usage: node fresh-wasm-compile.mjs MODULE.wasm");
const bytes = await readFile(path);
const valid = WebAssembly.validate(bytes);
if (!valid) throw new Error(`invalid WebAssembly module: ${path}`);
const started = performance.now();
await WebAssembly.compile(bytes);
const compileMs = performance.now() - started;
process.stdout.write(JSON.stringify({
  path,
  bytes: bytes.length,
  sha256: createHash("sha256").update(bytes).digest("hex"),
  valid,
  compileMs,
  node: process.version,
  v8: process.versions.v8,
}) + "\n");
