export const R079_ROWS = Object.freeze(["boot", "compile", "python"]);
export const R079_REPS = 5;
export const R079_CONTROL_WASM_SHA256 =
  "4160333352b18b233b3bba69858bfdc8b83474d94f2283ff49bf6a8b538ea69d";
export const R079_CANDIDATE_WASM_SHA256 =
  "e43fd0a9f02a7b21b38888f5e64aa12467db1bbf37f1ebfc0e3e4791ab62363a";
export const R079_LOADER_SHA256 =
  "f6a16b0274d6f097322312bf5a16604f133418dec88cf9987f50e6796f11642c";
export const R079_THRESHOLDS = Object.freeze({
  minimumPairedMedian: 0.97,
  minimumBootConfidenceLower: 0.95,
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

export function evaluateR079SourceRestoration(report, reportSha256 = null) {
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
  if (!same(config?.rows, R079_ROWS)) problems.push(`rows changed: ${JSON.stringify(config?.rows)}`);
  if (config?.reps !== R079_REPS) problems.push(`reps=${config?.reps} (expected ${R079_REPS})`);
  const controlKeys = Object.keys(config?.controlConfig ?? {});
  if (!same(controlKeys, ["SCORECARD_V2_REWRITE_WASM"]) ||
      typeof config?.controlConfig?.SCORECARD_V2_REWRITE_WASM !== "string") {
    problems.push(`control config changed: ${JSON.stringify(config?.controlConfig)}`);
  }
  if (!same(config?.candidateConfig, {})) {
    problems.push(`candidate config changed: ${JSON.stringify(config?.candidateConfig)}`);
  }
  if (config?.wasmBySide?.control?.sha256 !== R079_CONTROL_WASM_SHA256) {
    problems.push(`control Wasm changed: ${config?.wasmBySide?.control?.sha256}`);
  }
  if (config?.wasmBySide?.candidate?.sha256 !== R079_CANDIDATE_WASM_SHA256) {
    problems.push(`candidate Wasm changed: ${config?.wasmBySide?.candidate?.sha256}`);
  }
  if (config?.maximumSampleSpread !== R079_THRESHOLDS.maximumSampleSpread) {
    problems.push(`sample-spread threshold changed: ${config?.maximumSampleSpread}`);
  }
  if (config?.hostProbe?.maximumSpread !== R079_THRESHOLDS.maximumHostSpread) {
    problems.push(`host-spread threshold changed: ${config?.hostProbe?.maximumSpread}`);
  }
  if (!(report?.hostProbeSpread > 0) ||
      report.hostProbeSpread > R079_THRESHOLDS.maximumHostSpread) {
    problems.push(`host spread=${report?.hostProbeSpread}`);
  }

  const expectedTrials = R079_ROWS.length * R079_REPS * 2;
  const trials = report?.trials;
  const unique = new Set();
  if (!Array.isArray(trials) || trials.length !== expectedTrials) {
    problems.push(`trials=${trials?.length} (expected ${expectedTrials})`);
  } else {
    for (const trial of trials) {
      const label = `${trial?.row}/${trial?.side}/rep${trial?.rep}`;
      const key = `${trial?.row}/${trial?.side}/${trial?.rep}`;
      if (!R079_ROWS.includes(trial?.row) || !["control", "candidate"].includes(trial?.side) ||
          !Number.isInteger(trial?.rep) || trial.rep < 1 || trial.rep > R079_REPS) {
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
      const expectedEligible = trial.side === "candidate";
      if (result.measurementEligible !== expectedEligible) {
        problems.push(`${label}: measurementEligible=${result.measurementEligible}`);
      }
      const identity = result.runtime?.identity;
      const expectedWasm = trial.side === "control"
        ? R079_CONTROL_WASM_SHA256
        : R079_CANDIDATE_WASM_SHA256;
      if (identity?.loaderSha256 !== R079_LOADER_SHA256) {
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
  for (const key of R079_ROWS) {
    const aggregate = report?.aggregates?.[key];
    const paired = aggregate?.pairedCandidateSpeedup;
    const median = paired?.median;
    const confidence = paired?.medianConfidence95;
    for (const side of ["control", "candidate"]) {
      const values = aggregate?.[side]?.values;
      if (!Array.isArray(values) || values.length !== R079_REPS ||
          values.some((value) => !(value > 0))) {
        problems.push(`${key}/${side}: incomplete timing values`);
      }
      if (!(aggregate?.[side]?.spread > 0) ||
          aggregate[side].spread > R079_THRESHOLDS.maximumSampleSpread) {
        problems.push(`${key}/${side}: spread=${aggregate?.[side]?.spread}`);
      }
    }
    if (!Array.isArray(paired?.raw) || paired.raw.length !== R079_REPS ||
        !Number.isFinite(median) || !Array.isArray(confidence) || confidence.length !== 2) {
      problems.push(`${key}: paired summary incomplete`);
    }
    const minimumMedian = R079_THRESHOLDS.minimumPairedMedian;
    const medianPassed = Number.isFinite(median) && median >= minimumMedian;
    const confidencePassed = key !== "boot" ||
      (Number.isFinite(confidence?.[0]) &&
       confidence[0] >= R079_THRESHOLDS.minimumBootConfidenceLower);
    if (!medianPassed) problems.push(`${key}: paired median=${median} below ${minimumMedian}`);
    if (!confidencePassed) {
      problems.push(
        `${key}: confidence lower=${confidence?.[0]} below ` +
          R079_THRESHOLDS.minimumBootConfidenceLower,
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
    experiment: "R079 source-built R054 restoration validation",
    evaluated: new Date().toISOString(),
    inputReportSha256: reportSha256,
    identities: {
      controlWasmSha256: R079_CONTROL_WASM_SHA256,
      candidateWasmSha256: R079_CANDIDATE_WASM_SHA256,
      loaderSha256: R079_LOADER_SHA256,
    },
    thresholds: R079_THRESHOLDS,
    counts: { trials: trials?.length ?? null, uniqueTrials: unique.size },
    rows,
    gatePassed: problems.length === 0,
    problems,
  };
}
