#!/usr/bin/env node

// R070's authoritative instruction-level differential. Each fresh main Wasm
// instance emits/registers the real auxiliary module, then compares it with
// rv64_core::Cpu::step over all 49,152 compressed prefixes, 65,536 directed-
// family randomized scalar cases, and explicit access/fetch boundaries.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const [{ RV64Debug }, wasm] = await Promise.all([
  import(join(root, "web/rv64.js")),
  readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm")),
]);

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function readUleb(bytes, cursor) {
  let value = 0;
  let shift = 0;
  while (true) {
    const byte = bytes[cursor.offset++];
    value |= (byte & 0x7f) << shift;
    if (!(byte & 0x80)) return value >>> 0;
    shift += 7;
    if (shift > 28) throw new Error("oversized test-only uleb");
  }
}

function codeFunctionCount(bytes) {
  const cursor = { offset: 8 };
  while (cursor.offset < bytes.length) {
    const id = bytes[cursor.offset++];
    const length = readUleb(bytes, cursor);
    const end = cursor.offset + length;
    if (id === 10) return readUleb(bytes, cursor);
    cursor.offset = end;
  }
  throw new Error("module has no code section");
}

async function runOnce() {
  const vm = await RV64Debug.create(wasm);
  const modules = [];
  vm.onJitModule = (bytes, metadata) => modules.push({ bytes, metadata });
  const started = performance.now();
  const code = vm.ex.jit_static_t0_selftest();
  const elapsedMs = performance.now() - started;
  const detail = Array.from(
    { length: 8 },
    (_, index) => vm.ex.jit_static_t0_selftest_detail(index),
  );
  assert.equal(code, 0n, `selftest failure: ${detail.join(",")}`);
  assert.deepEqual(detail, Array(8).fill(0n));
  assert.equal(modules.length, 1, "selftest must register exactly one module");
  const [{ bytes, metadata }] = modules;
  assert.equal(metadata.kind, "single");
  assert.equal(codeFunctionCount(bytes), 1, "static core must have one defined function");
  const module = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(module), [
    { module: "env", name: "memory", kind: "memory" },
  ]);
  assert.deepEqual(WebAssembly.Module.exports(module), [
    { name: "run", kind: "function" },
  ]);
  return {
    hash: sha256(bytes),
    bytes: bytes.length,
    elapsedMs,
    compileMs: vm.jitCompileMs,
    instantiateMs: vm.jitInstantiateMs,
  };
}

const first = await runOnce();
const second = await runOnce();
assert.equal(second.hash, first.hash, "fresh instances emitted different module bytes");
assert.equal(second.bytes, first.bytes);
console.log(JSON.stringify({ status: "pass", first, second }, null, 2));
