#!/usr/bin/env node

// Candidate-specific entry point for the frozen R073 seven-pair,
// three-repetition browser gate. The implementation shares the historical
// R072 process-isolation runner but selects an independent schema and exact
// artifact/configuration constants before protocol.json is written.

if (process.argv.length !== 3) {
  throw new Error("usage: node tests/run-wanix-r073-pairs.mjs RESULTS_DIR");
}
process.argv.push("--r073-sampled-only");
await import("./run-wanix-r072-pairs.mjs");

