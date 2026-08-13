export const R100_REQUIREMENTS = Object.freeze({
  rows: Object.freeze(["boot", "compile", "python"]),
  repetitions: 5,
  controlWasmSha256: "d9f686a9ce4f54734f310cf5789f900af0c9b7067539e0251c429f376dd3df6d",
  candidateWasmSha256: "c36da489ebe3e2f15d960a1ad393b808e9ff285dc099d4988c745e0e81065b32",
  loaderSha256: "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385",
  cadence: "public-one-slice-per-turn",
  hostCpuAffinity: "8-15",
  maximumHostSpread: 1.10,
  minimumCompilePairedSpeedup: 1.03,
  minimumCompilePairedLowerBound: 1.00,
  minimumCompileNormalizedMipsRatio: 1.00,
  maximumGuardElapsedRatio: 1.03,
  compileFingerprint: "24eedf7e06beffd4d3ba1945585588db",
  pythonFingerprint: "832040",
});

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

function phaseForRow(row) {
  return row === "boot" ? "first" : "steady";
}

function normalizedMips(trial) {
  const phase = trial?.result?.phases?.[phaseForRow(trial.row)];
  const instructions = Number(phase?.counters?.guestInstructions);
  const elapsedMs = Number(phase?.value);
  return instructions / elapsedMs / 1000;
}

export function evaluateR100(report) {
  const req = R100_REQUIREMENTS;
  const integrityProblems = [];
  const check = (condition, message) => {
    if (!condition) integrityProblems.push(message);
  };

  check(report?.measurementValid === true, "report is not measurement-valid");
  check(report?.authoritative === false, "report unexpectedly claims authority");
  check(report?.problems?.length === 0, "report contains measurement problems");
  check(
    JSON.stringify(report?.configuration?.rows) === JSON.stringify(req.rows),
    "row set or order changed",
  );
  check(report?.configuration?.reps === req.repetitions, "repetition count changed");
  check(report?.configuration?.wasmBySide?.control?.sha256 === req.controlWasmSha256,
    "control artifact changed");
  check(report?.configuration?.wasmBySide?.candidate?.sha256 === req.candidateWasmSha256,
    "candidate artifact changed");
  check(Object.keys(report?.configuration?.controlConfig ?? {}).length === 1 &&
    typeof report.configuration.controlConfig.SCORECARD_V2_REWRITE_WASM === "string",
  "control configuration is not the sole Wasm override");
  check(Object.keys(report?.configuration?.candidateConfig ?? {}).length === 1 &&
    typeof report.configuration.candidateConfig.SCORECARD_V2_REWRITE_WASM === "string",
  "candidate configuration is not the sole Wasm override");
  check(report?.hostProbeSpread <= req.maximumHostSpread,
    `host spread exceeds ${req.maximumHostSpread}x`);
  check(report?.hostCpuAffinity === req.hostCpuAffinity, "CPU affinity changed");
  check(report?.trials?.length === req.rows.length * req.repetitions * 2,
    "expected exactly 30 retained trials");

  const byKey = new Map();
  for (const trial of report?.trials ?? []) {
    const prefix = `${trial.side}/${trial.row}/rep${trial.rep}`;
    const result = trial.result;
    check(!trial.error && result, `${prefix}: missing successful result`);
    if (!result) continue;
    const expectedHash = trial.side === "control"
      ? req.controlWasmSha256
      : req.candidateWasmSha256;
    check(result.runtime?.identity?.wasmSha256 === expectedHash,
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
    check(result.runtime?.policyProblems?.length === 0, `${prefix}: policy problem`);
    check(BigInt(result.runtime?.jitProof?.generatedInstructions ?? 0) > 0n,
      `${prefix}: generated execution not proved`);
    const phase = result.phases?.[phaseForRow(trial.row)];
    check(Number(phase?.value) > 0, `${prefix}: measured phase missing`);
    check(BigInt(phase?.counters?.guestInstructions ?? 0) > 0n,
      `${prefix}: guest-work counter missing`);
    check(trial.hostBeforeMs > 0 && trial.hostAfterMs > 0, `${prefix}: host probes missing`);
    byKey.set(`${trial.row}/${trial.rep}/${trial.side}`, trial);
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

  const compileMipsRatios = [];
  for (let rep = 1; rep <= req.repetitions; rep++) {
    const control = byKey.get(`compile/${rep}/control`);
    const candidate = byKey.get(`compile/${rep}/candidate`);
    if (control && candidate) {
      compileMipsRatios.push(normalizedMips(candidate) / normalizedMips(control));
    }
  }
  check(compileMipsRatios.length === req.repetitions,
    "Compile normalized-MIPS pairs are incomplete");

  const boot = report?.aggregates?.boot;
  const compile = report?.aggregates?.compile;
  const python = report?.aggregates?.python;
  const compileSpeedup = compile?.pairedCandidateSpeedup?.median ?? 0;
  const compileLower = compile?.pairedCandidateSpeedup?.medianConfidence95?.[0] ?? 0;
  const compileNormalizedMipsRatio = compileMipsRatios.length
    ? median(compileMipsRatios)
    : 0;
  const elapsedRatio = (row) => row?.candidate?.median / row?.control?.median;
  const checks = {
    integrity: integrityProblems.length === 0,
    compileMedian: compileSpeedup >= req.minimumCompilePairedSpeedup,
    compileLowerBound: compileLower >= req.minimumCompilePairedLowerBound,
    compileNormalizedMips:
      compileNormalizedMipsRatio >= req.minimumCompileNormalizedMipsRatio,
    bootGuard: elapsedRatio(boot) <= req.maximumGuardElapsedRatio,
    pythonGuard: elapsedRatio(python) <= req.maximumGuardElapsedRatio,
  };
  const admitProductGates = Object.values(checks).every(Boolean);
  return {
    schema: 1,
    experiment: "R100 interleaved fused-TLB",
    frozenRequirements: req,
    observed: {
      hostProbeSpread: report?.hostProbeSpread,
      boot: {
        controlMedianMs: boot?.control?.median,
        candidateMedianMs: boot?.candidate?.median,
        elapsedRatio: elapsedRatio(boot),
        pairedSpeedup: boot?.pairedCandidateSpeedup?.median,
        pairedConfidence95: boot?.pairedCandidateSpeedup?.medianConfidence95,
      },
      compile: {
        controlMedianMs: compile?.control?.median,
        candidateMedianMs: compile?.candidate?.median,
        elapsedRatio: elapsedRatio(compile),
        pairedSpeedup: compileSpeedup,
        pairedConfidence95: compile?.pairedCandidateSpeedup?.medianConfidence95,
        pairedNormalizedMipsRatios: compileMipsRatios,
        normalizedMipsRatio: compileNormalizedMipsRatio,
      },
      python: {
        controlMedianMs: python?.control?.median,
        candidateMedianMs: python?.candidate?.median,
        elapsedRatio: elapsedRatio(python),
        pairedSpeedup: python?.pairedCandidateSpeedup?.median,
        pairedConfidence95: python?.pairedCandidateSpeedup?.medianConfidence95,
      },
    },
    checks,
    integrityProblems,
    admitProductGates,
    decision: admitProductGates
      ? "advance-to-strict-browser-wanix-product-gates"
      : "reject-and-restore-executable-r085-baseline",
  };
}
