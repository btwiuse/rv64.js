#!/usr/bin/env node

// Select the prospectively frozen schema-4 R074 seven-pair, three-repetition
// browser protocol. The shared runner preserves the earlier immutable R072
// and R073 protocol schemas.

if (process.argv.length !== 3) {
  throw new Error("usage: node tests/run-wanix-r074-pairs.mjs RESULTS_DIR");
}
process.argv.push("--r074-short-backoff");
await import("./run-wanix-r072-pairs.mjs");
