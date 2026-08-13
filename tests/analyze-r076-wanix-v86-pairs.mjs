#!/usr/bin/env node

// Analyze only the fixed schema-3 R076 candidate/v86 WANIX guard. The shared
// analyzer retains compatibility with historical schema-2 reports.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

if (process.argv.length < 3 || process.argv.length > 4) {
  throw new Error(
    "usage: node tests/analyze-r076-wanix-v86-pairs.mjs RESULTS_DIR " +
      "[--output=REPORT]",
  );
}
const resultsDirectory = resolve(process.argv[2]);
const protocol = JSON.parse(readFileSync(resolve(resultsDirectory, "protocol.json"), "utf8"));
assert.equal(protocol.schema, 3, "not an R076 candidate/v86 protocol");
await import("./analyze-wanix-pairs.mjs");
