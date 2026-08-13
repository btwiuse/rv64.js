#!/usr/bin/env node

import assert from "node:assert/strict";
import {
  adjustedElapsedSpeedup,
  adjustedNormalizedThroughput,
  constructionDebit,
} from "./amortized-cold-cost.mjs";

const coldRegression = constructionDebit(
  [30, 30, 30, 30, 30, 30, 30],
  [45, 45, 45, 45, 45, 45, 45],
);
assert.equal(coldRegression.debitMs, 15);
assert.equal(coldRegression.creditMs, 0);

const apparentTwoPercent = adjustedElapsedSpeedup(
  Array(7).fill(1000),
  Array(7).fill(980),
  coldRegression.debitMs,
);
assert.ok(apparentTwoPercent.raw.median > 1.02);
assert.ok(apparentTwoPercent.adjusted.median < 1.01);

const coldImprovement = constructionDebit(
  [45, 45, 45, 45, 45, 45, 45],
  [30, 30, 30, 30, 30, 30, 30],
);
assert.equal(coldImprovement.debitMs, 0);
assert.equal(coldImprovement.creditMs, 15);

const throughput = adjustedNormalizedThroughput({
  controlElapsedMs: Array(7).fill(1000),
  candidateElapsedMs: Array(7).fill(970),
  controlWork: Array(7).fill(1_000_000),
  candidateWork: Array(7).fill(1_000_000),
  debitMs: 15,
});
assert.equal(throughput.candidateControlRatios.median, 1000 / 985);

assert.throws(
  () => constructionDebit([1], [1, 2]),
  /equal length/,
);
assert.throws(
  () => adjustedElapsedSpeedup([1], [1], -1),
  /non-negative/,
);

console.log("amortized cold-cost selftest: PASS");
