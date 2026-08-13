// Independent semantic evaluator for the frozen R077 scorecard promotion gate.
// Keep this deliberately separate from scorecard-v2's report construction and
// validation code: promotion must not trust the producer's goalMet bit.

export const R077_BASELINE_SHA256 =
  "603507e2a54729b490a87965a4c8012aa8b58ff49143a414d665ded8fcce516d";
export const R077_REWRITE_LOADER_SHA256 =
  "d949d8641dd4048ed031c7293ddf9d7b7c911dbc89aa9fa0c29487c21687718b";
export const R077_REWRITE_WASM_SHA256 =
  "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c";

export const R077_ROWS = Object.freeze([
  "alu",
  "mixed",
  "boot",
  "python",
  "compile",
  "numeric",
  "string",
  "bitfield",
  "fpemul",
  "fourier",
  "assignment",
  "idea",
  "huffman",
]);
export const R077_SIDES = Object.freeze(["rewrite", "legacy", "v86"]);
export const R077_THRESHOLDS = Object.freeze({
  minimumV86Matches: 11,
  minimumLegacyMatches: 13,
  matchFloor: 0.95,
  maximumRewriteRowSlowdown: 1.05,
  minimumBootSpeedup: 1.05,
});

const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);

function positiveNumber(value) {
  return Number.isFinite(value) && value > 0;
}

function unsignedCounter(value) {
  if (typeof value !== "string" || !/^\d+$/.test(value)) return null;
  try {
    return BigInt(value);
  } catch {
    return null;
  }
}

function exactConfiguration(report, label, problems) {
  const configuration = report?.configuration;
  if (!configuration) {
    problems.push(`${label}: configuration missing`);
    return;
  }
  if (!same(configuration.sides, R077_SIDES)) {
    problems.push(`${label}: sides changed: ${JSON.stringify(configuration.sides)}`);
  }
  if (!same(configuration.rows, R077_ROWS)) {
    problems.push(`${label}: rows changed: ${JSON.stringify(configuration.rows)}`);
  }
  if (configuration.reps !== 3) {
    problems.push(`${label}: reps=${configuration.reps} (expected 3)`);
  }
  if (configuration.rewritePolicy !== "production") {
    problems.push(
      `${label}: rewritePolicy=${configuration.rewritePolicy} (expected production)`,
    );
  }
  if (configuration.v86ExecutionPreflight !== true) {
    problems.push(`${label}: v86 execution preflight was not enabled`);
  }
}

function validateReportEnvelope(report, label, problems) {
  if (report?.schema !== 2) problems.push(`${label}: schema=${report?.schema} (expected 2)`);
  if (report?.authoritative !== true) problems.push(`${label}: not authoritative`);
  if (report?.measurementValid !== true) problems.push(`${label}: measurement invalid`);
  if (!Array.isArray(report?.problems) || report.problems.length !== 0) {
    problems.push(`${label}: problems are not an empty array`);
  }
  exactConfiguration(report, label, problems);
}

function validateCompleteTrials(report, label, problems) {
  const trials = report?.trials;
  const expectedCount = R077_ROWS.length * R077_SIDES.length * 3;
  if (!Array.isArray(trials) || trials.length !== expectedCount) {
    problems.push(`${label}: trials=${trials?.length} (expected ${expectedCount})`);
    return new Map();
  }

  const byKey = new Map();
  for (const trial of trials) {
    const key = `${trial?.row}/${trial?.side}/${trial?.rep}`;
    if (!R077_ROWS.includes(trial?.row) || !R077_SIDES.includes(trial?.side) ||
        !Number.isInteger(trial?.rep) || trial.rep < 1 || trial.rep > 3) {
      problems.push(`${label}: invalid trial identity ${key}`);
      continue;
    }
    if (byKey.has(key)) {
      problems.push(`${label}: duplicate trial ${key}`);
      continue;
    }
    byKey.set(key, trial);
    if (!trial.result || trial.result.side !== trial.side || trial.result.row !== trial.row) {
      problems.push(`${label}: incomplete or mislabeled trial ${key}`);
    }
    if (trial.result?.measurementEligible !== true) {
      problems.push(`${label}: trial ${key} is not measurement-eligible`);
    }
  }
  for (const row of R077_ROWS) {
    for (const side of R077_SIDES) {
      for (let rep = 1; rep <= 3; rep++) {
        const key = `${row}/${side}/${rep}`;
        if (!byKey.has(key)) problems.push(`${label}: missing trial ${key}`);
      }
    }
  }
  return byKey;
}

function validateRewriteTrial(trial, problems) {
  const label = `candidate: ${trial.row}/rewrite/rep${trial.rep}`;
  const runtime = trial.result?.runtime;
  const identity = runtime?.identity;
  if (identity?.loaderSha256 !== R077_REWRITE_LOADER_SHA256) {
    problems.push(`${label}: loader identity changed: ${identity?.loaderSha256}`);
  }
  if (identity?.wasmSha256 !== R077_REWRITE_WASM_SHA256) {
    problems.push(`${label}: Wasm identity changed: ${identity?.wasmSha256}`);
  }

  const lifecycle = runtime?.staticSystemT0;
  for (const [field, expected] of Object.entries({
    production: true,
    enabled: false,
    sampled: true,
    sampledBackoff: true,
    registeredModules: 1,
  })) {
    if (lifecycle?.[field] !== expected) {
      problems.push(`${label}: staticSystemT0.${field}=${lifecycle?.[field]} (expected ${expected})`);
    }
  }
  if (!Number.isInteger(lifecycle?.index) || lifecycle.index < 0) {
    problems.push(`${label}: staticSystemT0.index=${lifecycle?.index}`);
  }
  if (!Number.isInteger(lifecycle?.modulesBefore) ||
      !Number.isInteger(lifecycle?.modulesAfter) ||
      lifecycle.modulesAfter !== lifecycle.modulesBefore + 1) {
    problems.push(
      `${label}: module lifecycle=${lifecycle?.modulesBefore}->${lifecycle?.modulesAfter}`,
    );
  }

  const proof = runtime?.staticT0Proof;
  for (const field of ["sampledInstructions", "samples", "shortMarks", "shortBypasses"]) {
    const value = unsignedCounter(proof?.[field]);
    if (value === null || value <= 0n) {
      problems.push(`${label}: staticT0Proof.${field}=${proof?.[field]} (expected nonzero)`);
    }
  }
  const errors = unsignedCounter(proof?.errors);
  if (errors === null || errors !== 0n) {
    problems.push(`${label}: staticT0Proof.errors=${proof?.errors} (expected zero)`);
  }

  const generated = unsignedCounter(runtime?.jitProof?.generatedInstructions);
  if (generated === null || generated <= 0n) {
    problems.push(
      `${label}: generated JIT proof=${runtime?.jitProof?.generatedInstructions} (expected nonzero)`,
    );
  }
}

function validateV86ExecutionPreflight(report, problems) {
  const preflight = report?.v86ExecutionPreflight;
  const proof = preflight?.result?.runtime?.jitProof;
  const execution = proof?.executionProbe;
  if (preflight?.result?.side !== "v86" || preflight?.result?.measurementEligible !== false) {
    problems.push("candidate: v86 execution preflight result missing or measurement-eligible");
  }
  if (proof?.enabledRequested !== true || !(proof?.finalizedModules > 0)) {
    problems.push("candidate: v86 preflight generated-module proof missing");
  }
  if (execution?.active !== true || !(execution?.hits > 0) ||
      !(execution?.distinctHitIndexes > 0)) {
    problems.push(
      `candidate: v86 execution proof missing ` +
        `(hits=${execution?.hits}, distinct=${execution?.distinctHitIndexes})`,
    );
  }
}

function aggregatesByKey(report, label, problems) {
  if (!Array.isArray(report?.aggregates) || report.aggregates.length !== R077_ROWS.length) {
    problems.push(
      `${label}: aggregates=${report?.aggregates?.length} (expected ${R077_ROWS.length})`,
    );
    return new Map();
  }
  const byKey = new Map();
  for (const aggregate of report.aggregates) {
    if (!R077_ROWS.includes(aggregate?.key)) {
      problems.push(`${label}: unknown aggregate ${aggregate?.key}`);
    } else if (byKey.has(aggregate.key)) {
      problems.push(`${label}: duplicate aggregate ${aggregate.key}`);
    } else {
      byKey.set(aggregate.key, aggregate);
    }
  }
  for (const key of R077_ROWS) {
    if (!byKey.has(key)) problems.push(`${label}: aggregate ${key} missing`);
  }
  return byKey;
}

export function evaluateR077ScorecardPromotion({
  baseline,
  candidate,
  baselineSha256,
  candidateSha256 = null,
} = {}) {
  const problems = [];
  if (baselineSha256 !== R077_BASELINE_SHA256) {
    problems.push(
      `baseline SHA-256=${baselineSha256} (expected ${R077_BASELINE_SHA256})`,
    );
  }
  validateReportEnvelope(baseline, "baseline", problems);
  validateReportEnvelope(candidate, "candidate", problems);

  const baselineTrials = validateCompleteTrials(baseline, "baseline", problems);
  const candidateTrials = validateCompleteTrials(candidate, "candidate", problems);
  // The size assertion prevents silently accepting a partial map after malformed
  // or duplicate identities.
  if (baselineTrials.size !== 117) problems.push(`baseline: unique complete trials=${baselineTrials.size}`);
  if (candidateTrials.size !== 117) problems.push(`candidate: unique complete trials=${candidateTrials.size}`);
  for (const trial of candidateTrials.values()) {
    if (trial.side === "rewrite") validateRewriteTrial(trial, problems);
  }
  validateV86ExecutionPreflight(candidate, problems);

  const baselineRows = aggregatesByKey(baseline, "baseline", problems);
  const candidateRows = aggregatesByKey(candidate, "candidate", problems);
  const rows = [];
  let v86Matches = 0;
  let legacyMatches = 0;
  for (const key of R077_ROWS) {
    const oldRow = baselineRows.get(key);
    const newRow = candidateRows.get(key);
    const baselineRewrite = oldRow?.sides?.rewrite?.median;
    const rewrite = newRow?.sides?.rewrite?.median;
    const legacy = newRow?.sides?.legacy?.median;
    const v86 = newRow?.sides?.v86?.median;
    for (const [field, value] of [
      ["baseline rewrite median", baselineRewrite],
      ["rewrite median", rewrite],
      ["legacy median", legacy],
      ["v86 median", v86],
    ]) {
      if (!positiveNumber(value)) problems.push(`${key}: ${field}=${value}`);
    }
    const rewriteVsLegacy = positiveNumber(rewrite) && positiveNumber(legacy)
      ? legacy / rewrite
      : null;
    const rewriteVsV86 = positiveNumber(rewrite) && positiveNumber(v86)
      ? v86 / rewrite
      : null;
    const baselineToCurrent = positiveNumber(baselineRewrite) && positiveNumber(rewrite)
      ? baselineRewrite / rewrite
      : null;
    const maximumAllowed = positiveNumber(baselineRewrite)
      ? baselineRewrite * R077_THRESHOLDS.maximumRewriteRowSlowdown
      : null;
    const rowRegressionPassed = positiveNumber(rewrite) && positiveNumber(maximumAllowed)
      ? rewrite <= maximumAllowed
      : false;
    const v86Matched = rewriteVsV86 !== null && rewriteVsV86 >= R077_THRESHOLDS.matchFloor;
    const legacyMatched = rewriteVsLegacy !== null &&
      rewriteVsLegacy >= R077_THRESHOLDS.matchFloor;
    if (v86Matched) v86Matches++;
    if (legacyMatched) legacyMatches++;
    if (!rowRegressionPassed) {
      problems.push(
        `${key}: rewrite median ${rewrite} exceeds R054 +5% limit ${maximumAllowed}`,
      );
    }
    if (positiveNumber(newRow?.rewriteVsLegacy) && rewriteVsLegacy !== null &&
        Math.abs(newRow.rewriteVsLegacy - rewriteVsLegacy) > 1e-9) {
      problems.push(`${key}: reported rewriteVsLegacy is inconsistent with medians`);
    }
    if (positiveNumber(newRow?.rewriteVsV86) && rewriteVsV86 !== null &&
        Math.abs(newRow.rewriteVsV86 - rewriteVsV86) > 1e-9) {
      problems.push(`${key}: reported rewriteVsV86 is inconsistent with medians`);
    }
    rows.push({
      key,
      baselineRewriteMedian: baselineRewrite ?? null,
      rewriteMedian: rewrite ?? null,
      legacyMedian: legacy ?? null,
      v86Median: v86 ?? null,
      baselineToCurrent,
      maximumAllowed,
      rowRegressionPassed,
      rewriteVsLegacy,
      legacyMatched,
      rewriteVsV86,
      v86Matched,
    });
  }

  if (v86Matches < R077_THRESHOLDS.minimumV86Matches) {
    problems.push(
      `copy/v86 matches=${v86Matches}/13 (expected at least ${R077_THRESHOLDS.minimumV86Matches})`,
    );
  }
  if (legacyMatches < R077_THRESHOLDS.minimumLegacyMatches) {
    problems.push(
      `legacy matches=${legacyMatches}/13 (expected ${R077_THRESHOLDS.minimumLegacyMatches})`,
    );
  }

  const boot = rows.find((row) => row.key === "boot");
  const bootSpeedup = boot?.baselineToCurrent ?? null;
  if (!positiveNumber(bootSpeedup) || bootSpeedup < R077_THRESHOLDS.minimumBootSpeedup) {
    problems.push(
      `Boot speedup=${bootSpeedup} (expected at least ${R077_THRESHOLDS.minimumBootSpeedup})`,
    );
  }

  return {
    schema: 1,
    experiment: "R077 production-default authoritative scorecard promotion",
    evaluated: new Date().toISOString(),
    baseline: {
      sha256: baselineSha256 ?? null,
      created: baseline?.created ?? null,
      acceptedMatches: { v86: 11, legacy: 13 },
    },
    candidate: {
      sha256: candidateSha256,
      created: candidate?.created ?? null,
      producerGoalMet: candidate?.goalMet ?? null,
    },
    expectedRewriteIdentity: {
      loaderSha256: R077_REWRITE_LOADER_SHA256,
      wasmSha256: R077_REWRITE_WASM_SHA256,
    },
    thresholds: R077_THRESHOLDS,
    counts: {
      trials: candidate?.trials?.length ?? null,
      aggregates: candidate?.aggregates?.length ?? null,
      v86Matches,
      legacyMatches,
    },
    boot: {
      baselineMedian: boot?.baselineRewriteMedian ?? null,
      candidateMedian: boot?.rewriteMedian ?? null,
      speedup: bootSpeedup,
      passed: positiveNumber(bootSpeedup) &&
        bootSpeedup >= R077_THRESHOLDS.minimumBootSpeedup,
    },
    rows,
    measurementValid: problems.length === 0,
    gatePassed: problems.length === 0,
    problems,
  };
}
