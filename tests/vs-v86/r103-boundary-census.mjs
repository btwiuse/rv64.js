#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const DIAGNOSTIC = "2294651b34529b44271a180455b0f8a77ad51bb9a20ecaf90e445c5edf1139e7";
const LOADER = "2cbb264f4dac9d32e96254a4b12961c84a862e2bf3b6a5adcba248d2ad7b4385";
const COMPILE_FINGERPRINT = "24eedf7e06beffd4d3ba1945585588db";
const PHASES = ["first", "prime", "steady"];

const [reportPath, outputPath] = process.argv.slice(2);
if (!reportPath || !outputPath) {
  throw new Error("usage: r103-boundary-census.mjs REPORT.json OUTPUT.json");
}
const reportBytes = readFileSync(reportPath);
const report = JSON.parse(reportBytes);
const problems = [];
const check = (condition, message) => {
  if (!condition) problems.push(message);
};
const integer = (value, label) => {
  try {
    const parsed = BigInt(value);
    check(parsed >= 0n, `${label}: negative counter`);
    return parsed;
  } catch {
    problems.push(`${label}: invalid integer counter`);
    return 0n;
  }
};

check(report?.authoritative === false, "profile unexpectedly claims authority");
check(report?.measurementValid === false, "profile unexpectedly claims valid timing");
check(report?.configuration?.sides?.length === 1 &&
  report.configuration.sides[0] === "rewrite", "side set changed");
check(report?.configuration?.rows?.length === 1 &&
  report.configuration.rows[0] === "compile", "row set changed");
check(report?.configuration?.reps === 1, "profile repetition count changed");
check(report?.problems?.length === 1 &&
  report.problems[0].includes("proof-only run entered measurements"),
"unexpected profile problem set");
check(report?.trials?.length === 1, "expected one retained profile trial");

const trial = report?.trials?.[0];
const result = trial?.result;
check(!trial?.error && result, "profile trial did not complete");
check(result?.runtime?.identity?.wasmSha256 === DIAGNOSTIC,
  "diagnostic Wasm identity changed");
check(result?.runtime?.identity?.loaderSha256 === LOADER, "loader changed");
check(result?.runtime?.guest?.linux === "6.12.7" &&
  result.runtime.guest.alpine === "3.24.1" &&
  result.runtime.guest.arch === "riscv64", "modern guest changed");
check(result?.runtime?.requestedPolicy?.name === "production-page",
  "production page policy changed");
check(result?.runtime?.policyProblems?.length === 0, "runtime policy problem");
check(result?.runtime?.schedulerCadence?.name === "public-one-slice-per-turn" &&
  result.runtime.schedulerCadence.rv64SlicesPerEventLoopTurn === 1,
"public scheduler cadence changed");
check(BigInt(result?.runtime?.jitProof?.generatedInstructions ?? 0) > 0n,
  "generated execution missing");

const rows = [];
for (const phase of PHASES) {
  const measured = result?.phases?.[phase];
  const counters = measured?.counters;
  const mix = measured?.profile?.executionMix;
  check(measured?.md5 === COMPILE_FINGERPRINT, `${phase}: Compile output changed`);
  check(counters && mix, `${phase}: profile counters missing`);
  if (!counters || !mix) continue;
  const outer = integer(mix.regionCalls, `${phase}/regionCalls`);
  const hops = integer(counters.chainHops, `${phase}/chainHops`);
  const invocations = integer(mix.boundaryInvocations, `${phase}/boundaryInvocations`);
  const gprLoads = integer(mix.boundaryGprEntryLoads, `${phase}/gprEntryLoads`);
  const gprStores = integer(mix.boundaryGprExitStores, `${phase}/gprExitStores`);
  const fpLoads = integer(mix.boundaryFpEntryLoads, `${phase}/fpEntryLoads`);
  const fpStores = integer(mix.boundaryFpExitStores, `${phase}/fpExitStores`);
  const fcsrLoads = integer(mix.boundaryFcsrEntryLoads, `${phase}/fcsrEntryLoads`);
  const fcsrStores = integer(mix.boundaryFcsrExitStores, `${phase}/fcsrExitStores`);
  check(invocations === outer + hops,
    `${phase}: boundary invocations do not equal outer calls plus chain hops`);
  check(hops > 0n, `${phase}: no successful tail transfers`);
  check(gprLoads > 0n && gprStores > 0n, `${phase}: GPR boundary operations missing`);
  const gprOps = gprLoads + gprStores;
  const maximumOuterGprOps = outer * 62n;
  const chainGprOpsLower = gprOps > maximumOuterGprOps
    ? gprOps - maximumOuterGprOps
    : 0n;
  check(chainGprOpsLower > 0n, `${phase}: no rigorous chain-attributable GPR lower bound`);
  rows.push({
    phase,
    guestInstructions: counters.guestInstructions,
    generatedInstructions: counters.generatedInstructions,
    outerRegionCalls: outer.toString(),
    successfulChainHops: hops.toString(),
    boundaryInvocations: invocations.toString(),
    invocationIdentityDelta: (invocations - outer - hops).toString(),
    chainInvocationFraction: Number(hops) / Number(invocations),
    gprEntryLoads: gprLoads.toString(),
    gprExitStores: gprStores.toString(),
    gprOperations: gprOps.toString(),
    gprOperationsPerInvocation: Number(gprOps) / Number(invocations),
    chainGprOperationsLowerBound: chainGprOpsLower.toString(),
    chainGprOperationsLowerBoundPerHop: Number(chainGprOpsLower) / Number(hops),
    fpEntryLoads: fpLoads.toString(),
    fpExitStores: fpStores.toString(),
    fcsrEntryLoads: fcsrLoads.toString(),
    fcsrExitStores: fcsrStores.toString(),
  });
}
check(rows.length === PHASES.length, "phase set incomplete");

const output = {
  schema: 1,
  experiment: "R103 carried-GPR boundary opportunity census",
  performanceEvidence: false,
  frozenIdentities: {
    diagnosticWasmSha256: DIAGNOSTIC,
    loaderSha256: LOADER,
    compileFingerprint: COMPILE_FINGERPRINT,
  },
  report: {
    path: reportPath,
    sha256: createHash("sha256").update(reportBytes).digest("hex"),
  },
  rows,
  integrityProblems: problems,
  pass: problems.length === 0,
  decision: problems.length === 0
    ? "run-frozen-ordinary-tiered-boundary-proof"
    : "reject-diagnostic-and-restore-baseline",
};
writeFileSync(outputPath, `${JSON.stringify(output, null, 2)}\n`, { flag: "wx" });
console.log(`R103 boundary census: ${output.pass ? "PASS" : "FAIL"}`);
if (problems.length) {
  console.error(problems.join("; "));
  process.exitCode = 1;
}
