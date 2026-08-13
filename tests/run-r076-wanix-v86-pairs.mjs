#!/usr/bin/env node

// Select the fixed R076 Gate-C seven-pair, three-repetition candidate/v86
// protocol. The shared runner retains its historical schema-2 interface.

if (process.argv.length !== 3) {
  throw new Error("usage: node tests/run-r076-wanix-v86-pairs.mjs RESULTS_DIR");
}
process.argv.push("--r076-candidate-v86");
await import("./run-wanix-pairs.mjs");
