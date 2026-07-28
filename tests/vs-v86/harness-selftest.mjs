import assert from "node:assert/strict";
import {
  alternatingOrder,
  candidateVerdict,
  median,
  pairedOrder,
} from "./bench-math.mjs";

assert.equal(median([]), null);
assert.equal(median([3, 1, 2]), 2);
assert.equal(median([4, 1, 3, 2]), 2.5);
assert.equal(median([null, 7, Number.NaN, 3]), 5);

assert.deepEqual(pairedOrder(0), ["v86", "rv64"]);
assert.deepEqual(pairedOrder(1), ["rv64", "v86"]);
assert.deepEqual(pairedOrder(2, false), ["rv64"]);
assert.deepEqual(alternatingOrder(0, "base", "candidate"), ["base", "candidate"]);
assert.deepEqual(alternatingOrder(1, "base", "candidate"), ["candidate", "base"]);

assert.equal(candidateVerdict(1.1), "IMPROVEMENT");
assert.equal(candidateVerdict(1.099), "TIE");
assert.equal(candidateVerdict(1), "TIE");
assert.equal(candidateVerdict(1 / 1.1), "REGRESSION");
assert.equal(candidateVerdict(0.92), "TIE");

console.log("BENCH HARNESS SELFTEST: PASS");
