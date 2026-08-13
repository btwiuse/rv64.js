#!/usr/bin/env node

// Select the prospectively frozen schema-5 R075 seven-pair, three-repetition
// preboot browser protocol. The shared runner preserves schemas 1 through 4.

if (process.argv.length !== 3) {
  throw new Error("usage: node tests/run-wanix-r075-pairs.mjs RESULTS_DIR");
}
process.argv.push("--r075-preboot");
await import("./run-wanix-r072-pairs.mjs");
