export const R078_ROWS = Object.freeze(["boot", "compile", "python"]);
export const R078_REPS = 5;
export const R078_CONTROL_WASM_SHA256 =
  "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c";
export const R078_CANDIDATE_WASM_SHA256 =
  "4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d";
export const R078_LOADER_SHA256 =
  "f1d56b133c39cbaf174604830edbb8f351ad71f574c4f735f66271f23bb889c2";
export const R078_THRESHOLDS = Object.freeze({
  minimumBootPairedMedian: 1.05,
  minimumBootConfidenceLower: 1.00,
  minimumGuardPairedMedian: 0.97,
  maximumSampleSpread: 1.25,
  maximumHostSpread: 1.25,
});

const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);

function positiveCounter(value) {
  if (typeof value !== "string" || !/^\d+$/.test(value)) return false;
  try {
    return BigInt(value) > 0n;
  } catch {
    return false;
  }
}

function zeroCounter(value) {
  if (typeof value !== "string" || !/^\d+$/.test(value)) return false;
  try {
    return BigInt(value) === 0n;
  } catch {
    return false;
  }
}

export function evaluateR078ArtifactRegression(report, reportSha256 = null) {
  const problems = [];
  const config = report?.configuration;
  if (report?.schema !== 1) problems.push(`schema=${report?.schema} (expected 1)`);
  if (report?.authoritative !== false) problems.push("report must be diagnostic, not authoritative");
  if (report?.measurementValid !== true) problems.push("underlying measurement is invalid");
  if (!Array.isArray(report?.problems) || report.problems.length !== 0) {
    problems.push("underlying problems are not empty");
  }
  if (report?.purpose !== "rewrite runtime-configuration or artifact A/B diagnostic") {
    problems.push(`purpose changed: ${report?.purpose}`);
  }
  if (report?.hostCpuAffinity !== "8-15") {
    problems.push(`CPU affinity=${report?.hostCpuAffinity} (expected 8-15)`);
  }
  if (!same(config?.rows, R078_ROWS)) problems.push(`rows changed: ${JSON.stringify(config?.rows)}`);
  if (config?.reps !== R078_REPS) problems.push(`reps=${config?.reps} (expected ${R078_REPS})`);
  if (!same(config?.controlConfig, {})) {
    problems.push(`control config changed: ${JSON.stringify(config?.controlConfig)}`);
  }
  const candidateKeys = Object.keys(config?.candidateConfig ?? {});
  if (!same(candidateKeys, ["SCORECARD_V2_REWRITE_WASM"]) ||
      typeof config?.candidateConfig?.SCORECARD_V2_REWRITE_WASM !== "string") {
    problems.push(`candidate config changed: ${JSON.stringify(config?.candidateConfig)}`);
  }
  if (config?.wasmBySide?.control?.sha256 !== R078_CONTROL_WASM_SHA256) {
    problems.push(`control Wasm changed: ${config?.wasmBySide?.control?.sha256}`);
  }
  if (config?.wasmBySide?.candidate?.sha256 !== R078_CANDIDATE_WASM_SHA256) {
    problems.push(`candidate Wasm changed: ${config?.wasmBySide?.candidate?.sha256}`);
  }
  if (config?.maximumSampleSpread !== R078_THRESHOLDS.maximumSampleSpread) {
    problems.push(`sample-spread threshold changed: ${config?.maximumSampleSpread}`);
  }
  if (config?.hostProbe?.maximumSpread !== R078_THRESHOLDS.maximumHostSpread) {
    problems.push(`host-spread threshold changed: ${config?.hostProbe?.maximumSpread}`);
  }
  if (!(report?.hostProbeSpread > 0) ||
      report.hostProbeSpread > R078_THRESHOLDS.maximumHostSpread) {
    problems.push(`host spread=${report?.hostProbeSpread}`);
  }

  const expectedTrials = R078_ROWS.length * R078_REPS * 2;
  const trials = report?.trials;
  const unique = new Set();
  if (!Array.isArray(trials) || trials.length !== expectedTrials) {
    problems.push(`trials=${trials?.length} (expected ${expectedTrials})`);
  } else {
    for (const trial of trials) {
      const label = `${trial?.row}/${trial?.side}/rep${trial?.rep}`;
      const key = `${trial?.row}/${trial?.side}/${trial?.rep}`;
      if (!R078_ROWS.includes(trial?.row) || !["control", "candidate"].includes(trial?.side) ||
          !Number.isInteger(trial?.rep) || trial.rep < 1 || trial.rep > R078_REPS) {
        problems.push(`invalid trial identity ${label}`);
        continue;
      }
      if (unique.has(key)) problems.push(`duplicate trial ${label}`);
      unique.add(key);
      if (trial.error || !trial.result) {
        problems.push(`${label}: incomplete: ${trial.error}`);
        continue;
      }
      const result = trial.result;
      if (result.side !== "rewrite" || result.row !== trial.row || result.schema !== 2) {
        problems.push(`${label}: worker result mislabeled`);
      }
      const expectedEligible = trial.side === "control";
      if (result.measurementEligible !== expectedEligible) {
        problems.push(`${label}: measurementEligible=${result.measurementEligible}`);
      }
      const identity = result.runtime?.identity;
      const expectedWasm = trial.side === "control"
        ? R078_CONTROL_WASM_SHA256
        : R078_CANDIDATE_WASM_SHA256;
      if (identity?.loaderSha256 !== R078_LOADER_SHA256) {
        problems.push(`${label}: loader=${identity?.loaderSha256}`);
      }
      if (identity?.wasmSha256 !== expectedWasm) {
        problems.push(`${label}: Wasm=${identity?.wasmSha256}`);
      }
      if (!same(result.runtime?.guest, {
        linux: "6.12.7",
        alpine: "3.24.1",
        arch: "riscv64",
      })) {
        problems.push(`${label}: guest changed: ${JSON.stringify(result.runtime?.guest)}`);
      }
      if (!positiveCounter(result.runtime?.jitProof?.generatedInstructions) ||
          !positiveCounter(result.runtime?.jitProof?.dispatches)) {
        problems.push(`${label}: generated execution proof missing`);
      }
      if (result.runtime?.staticSystemT0 !== undefined || result.runtime?.staticT0Proof !== undefined) {
        problems.push(`${label}: static-T0 lifecycle unexpectedly active`);
      }
      const phases = Object.values(result.phases ?? {});
      if (!phases.length) problems.push(`${label}: phases missing`);
      for (const phase of phases) {
        const counters = phase?.counters;
        for (const field of [
          "staticT0FastInstructions",
          "staticT0SlowInstructions",
          "staticT0Errors",
          "staticT0SampledInstructions",
          "staticT0Samples",
          "staticT0ShortMarks",
          "staticT0ShortBypasses",
        ]) {
          if (!zeroCounter(counters?.[field])) {
            problems.push(`${label}: ${field}=${counters?.[field]} (expected zero)`);
          }
        }
      }
    }
  }
  if (unique.size !== expectedTrials) problems.push(`unique trials=${unique.size}`);

  const rows = {};
  for (const key of R078_ROWS) {
    const aggregate = report?.aggregates?.[key];
    const paired = aggregate?.pairedCandidateSpeedup;
    const median = paired?.median;
    const confidence = paired?.medianConfidence95;
    for (const side of ["control", "candidate"]) {
      const values = aggregate?.[side]?.values;
      if (!Array.isArray(values) || values.length !== R078_REPS ||
          values.some((value) => !(value > 0))) {
        problems.push(`${key}/${side}: incomplete timing values`);
      }
      if (!(aggregate?.[side]?.spread > 0) ||
          aggregate[side].spread > R078_THRESHOLDS.maximumSampleSpread) {
        problems.push(`${key}/${side}: spread=${aggregate?.[side]?.spread}`);
      }
    }
    if (!Array.isArray(paired?.raw) || paired.raw.length !== R078_REPS ||
        !Number.isFinite(median) || !Array.isArray(confidence) || confidence.length !== 2) {
      problems.push(`${key}: paired summary incomplete`);
    }
    const minimumMedian = key === "boot"
      ? R078_THRESHOLDS.minimumBootPairedMedian
      : R078_THRESHOLDS.minimumGuardPairedMedian;
    const medianPassed = Number.isFinite(median) && median >= minimumMedian;
    const confidencePassed = key !== "boot" ||
      (Number.isFinite(confidence?.[0]) &&
       confidence[0] >= R078_THRESHOLDS.minimumBootConfidenceLower);
    if (!medianPassed) problems.push(`${key}: paired median=${median} below ${minimumMedian}`);
    if (!confidencePassed) {
      problems.push(
        `${key}: confidence lower=${confidence?.[0]} below ` +
          R078_THRESHOLDS.minimumBootConfidenceLower,
      );
    }
    rows[key] = {
      controlMedian: aggregate?.control?.median ?? null,
      candidateMedian: aggregate?.candidate?.median ?? null,
      pairedMedian: median ?? null,
      confidence95: confidence ?? null,
      medianPassed,
      confidencePassed,
    };
  }

  return {
    schema: 1,
    experiment: "R078 dormant static-T0 artifact regression audit",
    evaluated: new Date().toISOString(),
    inputReportSha256: reportSha256,
    identities: {
      controlWasmSha256: R078_CONTROL_WASM_SHA256,
      candidateWasmSha256: R078_CANDIDATE_WASM_SHA256,
      loaderSha256: R078_LOADER_SHA256,
    },
    thresholds: R078_THRESHOLDS,
    counts: { trials: trials?.length ?? null, uniqueTrials: unique.size },
    rows,
    gatePassed: problems.length === 0,
    problems,
  };
}
