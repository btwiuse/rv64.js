export const R096_REQUIREMENTS = Object.freeze({
  rows: Object.freeze(["boot", "compile", "python"]),
  repetitions: 7,
  cadence: "public-one-slice-per-turn",
  maximumHostSpread: 1.10,
  minimumCompilePairedSpeedup: 1.0,
  minimumCompilePairedLowerBound: 1.0,
  maximumGuardElapsedRatio: 1.02,
});

export function evaluateR096(report, expectedWasmSha256) {
  const problems = [];
  const check = (condition, message) => {
    if (!condition) problems.push(message);
  };
  const req = R096_REQUIREMENTS;
  check(report?.measurementValid === true, "report is not measurement-valid");
  check(report?.authoritative === false, "report unexpectedly claims authority");
  check(report?.problems?.length === 0, "report contains measurement problems");
  check(JSON.stringify(report?.configuration?.rows) === JSON.stringify(req.rows),
    "row set or order changed");
  check(report?.configuration?.reps === req.repetitions, "repetition count changed");
  check(JSON.stringify(report?.configuration?.controlConfig) ===
    JSON.stringify({ SCORECARD_V2_TAIL_CHAIN_ACCOUNTING: "1" }),
  "control configuration changed");
  check(JSON.stringify(report?.configuration?.candidateConfig) ===
    JSON.stringify({ SCORECARD_V2_TAIL_CHAIN_ACCOUNTING: "0" }),
  "candidate configuration changed");
  for (const side of ["control", "candidate"]) {
    check(report?.configuration?.wasmBySide?.[side]?.sha256 === expectedWasmSha256,
      `${side} main Wasm changed`);
  }
  check(report?.hostProbeSpread <= req.maximumHostSpread,
    `host spread exceeds ${req.maximumHostSpread}x`);
  check(report?.trials?.length === req.rows.length * req.repetitions * 2,
    "retained trial count changed");

  for (const trial of report?.trials ?? []) {
    const prefix = `${trial.side}/${trial.row}/rep${trial.rep}`;
    const result = trial.result;
    check(!trial.error && result, `${prefix}: missing successful result`);
    if (!result) continue;
    check(result.runtime?.identity?.wasmSha256 === expectedWasmSha256,
      `${prefix}: runtime Wasm changed`);
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
    const expectedAccounting = trial.side === "control" ? "1" : "0";
    check(result.runtime?.effectivePolicy?.tailChainAccountingEnabled === expectedAccounting,
      `${prefix}: effective accounting mode changed`);
    check(String(result.runtime?.diagnostic?.tailChainAccounting) === expectedAccounting,
      `${prefix}: requested accounting mode not recorded`);
    const phase = trial.row === "boot" ? "first" : "steady";
    const hops = BigInt(result.phases?.[phase]?.counters?.chainHops ?? 0);
    check(trial.side === "control" ? hops > 0n : hops === 0n,
      `${prefix}: transfer accounting proof failed (${hops})`);
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
    check(JSON.stringify(aggregate.control?.inputs) ===
      JSON.stringify(aggregate.candidate?.inputs), `${row}: input identities differ`);
    check(JSON.stringify(aggregate.control?.fingerprints) ===
      JSON.stringify(aggregate.candidate?.fingerprints), `${row}: outputs differ`);
  }

  const compile = report?.aggregates?.compile;
  const boot = report?.aggregates?.boot;
  const python = report?.aggregates?.python;
  const compileMedian = compile?.pairedCandidateSpeedup?.median ?? 0;
  const compileLower = compile?.pairedCandidateSpeedup?.medianConfidence95?.[0] ?? 0;
  const elapsedRatio = (row) => row?.candidate?.median / row?.control?.median;
  const checks = {
    integrity: problems.length === 0,
    compileMedian: compileMedian > req.minimumCompilePairedSpeedup,
    compileLowerBound: compileLower >= req.minimumCompilePairedLowerBound,
    bootGuard: elapsedRatio(boot) <= req.maximumGuardElapsedRatio,
    pythonGuard: elapsedRatio(python) <= req.maximumGuardElapsedRatio,
  };
  const advance = Object.values(checks).every(Boolean);
  return {
    schema: 1,
    experiment: "R096",
    frozenRequirements: req,
    observed: {
      hostProbeSpread: report?.hostProbeSpread,
      bootElapsedRatio: elapsedRatio(boot),
      compilePairedSpeedup: compileMedian,
      compilePairedConfidence95: compile?.pairedCandidateSpeedup?.medianConfidence95,
      pythonElapsedRatio: elapsedRatio(python),
    },
    checks,
    integrityProblems: problems,
    advance,
    decision: advance ? "advance-to-product-gates" : "reject-and-remove-r096",
  };
}
