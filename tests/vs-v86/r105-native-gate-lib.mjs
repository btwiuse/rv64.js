export const R105_REQUIREMENTS = Object.freeze({
  rows: Object.freeze(["boot", "compile", "python"]),
  repetitions: 7,
  wasmSha256: "0593567eb75dfe29dd06cf0cabf0747abfa3b217080e2dd2e8c72ca192469a2d",
  loaderSha256: "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385",
  cadence: "public-one-slice-per-turn",
  hostCpuAffinity: "8-15",
  maximumHostSpread: 1.10,
  minimumBootPairedSpeedup: 1.01,
  minimumBootPairedLowerBound: 1.00,
  minimumBootNormalizedMipsRatio: 1.01,
  minimumProtectedPairedSpeedup: 0.99,
  minimumProtectedConfidenceUpperBound: 1.00,
  compileFingerprint: "24eedf7e06beffd4d3ba1945585588db",
  pythonFingerprint: "832040",
});

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length & 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

const phaseFor = (row) => row === "boot" ? "first" : "steady";

function normalizedMips(trial) {
  const phase = trial?.result?.phases?.[phaseFor(trial.row)];
  return Number(phase?.counters?.guestInstructions) / Number(phase?.value) / 1000;
}

function exactConfig(config, expectedFlag) {
  if (!config || Array.isArray(config) || typeof config !== "object") return false;
  const keys = Object.keys(config).sort();
  return JSON.stringify(keys) === JSON.stringify([
    "SCORECARD_V2_INTEGRATED_SCALAR_T0",
    "SCORECARD_V2_REWRITE_WASM",
  ]) && config.SCORECARD_V2_INTEGRATED_SCALAR_T0 === expectedFlag &&
    typeof config.SCORECARD_V2_REWRITE_WASM === "string";
}

export function evaluateR105(report) {
  const req = R105_REQUIREMENTS;
  const integrityProblems = [];
  const check = (condition, message) => {
    if (!condition) integrityProblems.push(message);
  };

  check(report?.measurementValid === true, "report is not measurement-valid");
  check(report?.authoritative === false, "report unexpectedly claims authority");
  check(Array.isArray(report?.problems) && report.problems.length === 0,
    "report contains measurement problems");
  check(JSON.stringify(report?.configuration?.rows) === JSON.stringify(req.rows),
    "row set or order changed");
  check(report?.configuration?.reps === req.repetitions, "repetition count changed");
  check(exactConfig(report?.configuration?.controlConfig, "0"),
    "control configuration is not exact same-Wasm scalar-off");
  check(exactConfig(report?.configuration?.candidateConfig, "1"),
    "candidate configuration is not exact same-Wasm scalar-on");
  check(report?.configuration?.controlConfig?.SCORECARD_V2_REWRITE_WASM ===
    report?.configuration?.candidateConfig?.SCORECARD_V2_REWRITE_WASM,
  "control and candidate do not name the same Wasm path");
  for (const side of ["control", "candidate"]) {
    check(report?.configuration?.wasmBySide?.[side]?.sha256 === req.wasmSha256,
      `${side} configured Wasm changed`);
  }
  check(Number.isFinite(report?.hostProbeSpread) &&
    report.hostProbeSpread <= req.maximumHostSpread,
  `host spread exceeds ${req.maximumHostSpread}x`);
  check(report?.hostCpuAffinity === req.hostCpuAffinity, "CPU affinity changed");
  check(report?.trials?.length === req.rows.length * req.repetitions * 2,
    "expected exactly 42 retained trials");

  const byKey = new Map();
  for (const trial of report?.trials ?? []) {
    check(req.rows.includes(trial.row), `unexpected trial row ${trial.row}`);
    check(trial.side === "control" || trial.side === "candidate",
      `unexpected trial side ${trial.side}`);
    check(Number.isInteger(trial.rep) && trial.rep >= 1 && trial.rep <= req.repetitions,
      `unexpected trial repetition ${trial.rep}`);
    const prefix = `${trial.side}/${trial.row}/rep${trial.rep}`;
    const result = trial.result;
    check(!trial.error && result, `${prefix}: missing successful result`);
    if (!result) continue;
    check(result.side === "rewrite" && result.row === trial.row,
      `${prefix}: worker identity changed`);
    check(result.measurementEligible === false,
      `${prefix}: diagnostic leg unexpectedly claims product eligibility`);
    check(result.runtime?.identity?.wasmSha256 === req.wasmSha256,
      `${prefix}: runtime Wasm changed`);
    check(result.runtime?.identity?.loaderSha256 === req.loaderSha256,
      `${prefix}: loader changed`);
    check(result.runtime?.schedulerCadence?.name === req.cadence &&
      result.runtime.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
    `${prefix}: scheduler cadence changed`);
    check(result.runtime?.guest?.linux === "6.12.7" &&
      result.runtime.guest.alpine === "3.24.1" &&
      result.runtime.guest.arch === "riscv64", `${prefix}: modern guest changed`);
    check(result.runtime?.requestedPolicy?.name === "production-page",
      `${prefix}: production policy changed`);
    check(Array.isArray(result.runtime?.policyProblems) &&
      result.runtime.policyProblems.length === 0, `${prefix}: policy problem`);
    check(result.runtime?.jitProof?.enabledRequested === true &&
      BigInt(result.runtime?.jitProof?.generatedInstructions ?? 0) > 0n &&
      BigInt(result.runtime?.jitProof?.dispatches ?? 0) > 0n,
    `${prefix}: generated execution missing`);
    const expectedFlag = trial.side === "control" ? 0 : 1;
    check(result.runtime?.diagnostic?.integratedScalarT0 === expectedFlag,
      `${prefix}: integrated scalar mode was not recorded exactly`);
    check(Object.keys(result.runtime?.diagnostic ?? {}).sort().join(",") ===
      "integratedScalarT0,rewriteWasmOverride",
    `${prefix}: unexpected diagnostic override`);
    const phase = result.phases?.[phaseFor(trial.row)];
    check(Number(phase?.value) > 0, `${prefix}: measured phase missing`);
    check(BigInt(phase?.counters?.guestInstructions ?? 0) > 0n,
      `${prefix}: guest-work counter missing`);
    check(Number(trial.hostBeforeMs) > 0 && Number(trial.hostAfterMs) > 0,
      `${prefix}: host probes missing`);
    const key = `${trial.row}/${trial.rep}/${trial.side}`;
    check(!byKey.has(key), `${prefix}: duplicate retained trial`);
    byKey.set(key, trial);
  }

  for (const row of req.rows) {
    for (let rep = 1; rep <= req.repetitions; rep++) {
      const expectedOrder = rep & 1 ? ["control", "candidate"] : ["candidate", "control"];
      const pair = (report?.trials ?? []).filter((trial) =>
        trial.row === row && trial.rep === rep);
      check(JSON.stringify(pair.map((trial) => trial.side)) === JSON.stringify(expectedOrder),
        `${row}/rep${rep}: pair order changed`);
      for (const side of ["control", "candidate"]) {
        check(byKey.has(`${row}/${rep}/${side}`),
          `${side}/${row}/rep${rep}: retained trial missing`);
      }
    }
  }

  for (const row of req.rows) {
    const aggregate = report?.aggregates?.[row];
    check(aggregate, `${row}: aggregate missing`);
    if (!aggregate) continue;
    check(aggregate.control?.values?.length === req.repetitions &&
      aggregate.candidate?.values?.length === req.repetitions,
    `${row}: incomplete side samples`);
    check(aggregate.pairedCandidateSpeedup?.raw?.length === req.repetitions,
      `${row}: incomplete paired samples`);
    check(aggregate.control?.spread <= report.configuration.maximumSampleSpread &&
      aggregate.candidate?.spread <= report.configuration.maximumSampleSpread,
    `${row}: sample spread exceeds scorecard limit`);
    check(JSON.stringify(aggregate.control?.inputs) ===
      JSON.stringify(aggregate.candidate?.inputs), `${row}: input identities differ`);
    check(JSON.stringify(aggregate.control?.fingerprints) ===
      JSON.stringify(aggregate.candidate?.fingerprints), `${row}: outputs differ`);
  }
  check(JSON.stringify(report?.aggregates?.compile?.control?.fingerprints) ===
    JSON.stringify([req.compileFingerprint]), "Compile object fingerprint changed");
  check(JSON.stringify(report?.aggregates?.python?.control?.fingerprints) ===
    JSON.stringify([req.pythonFingerprint]), "Python result fingerprint changed");

  const bootMipsRatios = [];
  for (let rep = 1; rep <= req.repetitions; rep++) {
    const control = byKey.get(`boot/${rep}/control`);
    const candidate = byKey.get(`boot/${rep}/candidate`);
    if (control && candidate) {
      bootMipsRatios.push(normalizedMips(candidate) / normalizedMips(control));
    }
  }
  check(bootMipsRatios.length === req.repetitions,
    "Boot normalized-MIPS pairs are incomplete");
  check(bootMipsRatios.every(Number.isFinite), "Boot normalized-MIPS proof is invalid");

  const boot = report?.aggregates?.boot;
  const compile = report?.aggregates?.compile;
  const python = report?.aggregates?.python;
  const pairedMedian = (row) => row?.pairedCandidateSpeedup?.median ?? 0;
  const pairedLower = (row) => row?.pairedCandidateSpeedup?.medianConfidence95?.[0] ?? 0;
  const pairedUpper = (row) => row?.pairedCandidateSpeedup?.medianConfidence95?.[1] ?? 0;
  const bootMips = bootMipsRatios.length ? median(bootMipsRatios) : 0;
  const checks = {
    integrity: integrityProblems.length === 0,
    bootMedian: pairedMedian(boot) >= req.minimumBootPairedSpeedup,
    bootLowerBound: pairedLower(boot) >= req.minimumBootPairedLowerBound,
    bootNormalizedMips: bootMips >= req.minimumBootNormalizedMipsRatio,
    compileMedian: pairedMedian(compile) >= req.minimumProtectedPairedSpeedup,
    compileNoEstablishedRegression:
      pairedUpper(compile) >= req.minimumProtectedConfidenceUpperBound,
    pythonMedian: pairedMedian(python) >= req.minimumProtectedPairedSpeedup,
    pythonNoEstablishedRegression:
      pairedUpper(python) >= req.minimumProtectedConfidenceUpperBound,
  };
  const admitProductGates = Object.values(checks).every(Boolean);
  const observedRow = (row) => ({
    controlMedianMs: row?.control?.median,
    candidateMedianMs: row?.candidate?.median,
    pairedSpeedup: row?.pairedCandidateSpeedup?.median,
    pairedConfidence95: row?.pairedCandidateSpeedup?.medianConfidence95,
  });
  return {
    schema: 1,
    experiment: "R105 integrated scalar Tier-0 qualified reconfirmation",
    frozenRequirements: req,
    observed: {
      hostProbeSpread: report?.hostProbeSpread,
      boot: {
        ...observedRow(boot),
        pairedNormalizedMipsRatios: bootMipsRatios,
        normalizedMipsRatio: bootMips,
      },
      compile: observedRow(compile),
      python: observedRow(python),
    },
    checks,
    integrityProblems,
    admitProductGates,
    decision: admitProductGates
      ? "advance-to-clean-product-browser-wanix-gates"
      : "reject-and-restore-executable-r085-baseline",
  };
}
