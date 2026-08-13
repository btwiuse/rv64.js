#!/usr/bin/env node

// One process owns one fresh guest and one scorecard-v2 trial. Non-boot rows
// execute FIRST, PRIME, and STEADY in that guest. The host drains outstanding
// generated-Wasm compilation between phases; only STEADY is scored.

import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  LEGACY_MODERN_COMPARATOR,
  HOLDOUT_ROW_BY_KEY,
  ROW_BY_KEY,
  SCHEMA,
  SIDES,
  V86_RUNTIME,
  configureRvPolicy,
  deltaRvCounters,
  embeddedWorkloadSha256,
  NBENCH_WORKLOAD_CONTRACT,
  pagePolicySnapshot,
  parseExecutionMode,
  phasesFor,
  parseGuestIdentity,
  parseNbench,
  readRvCounters,
  sha256,
  validateGuestIdentity,
  validateProductionPolicy,
} from "./scorecard-v2-lib.mjs";
import {
  cadenceDiagnostic,
  cadenceRecord,
  parsePumpCadence,
  shouldYieldAfterPump,
} from "./scorecard-v2-cadence.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const artifacts = resolve(process.env.ARTIFACTS || "");
if (!process.env.ARTIFACTS) throw new Error("set ARTIFACTS");
const side = process.argv[2];
const rowKey = process.argv[3];
if (!SIDES.includes(side)) throw new Error(`side must be one of ${SIDES.join(", ")}`);
const row = ROW_BY_KEY.get(rowKey) ?? HOLDOUT_ROW_BY_KEY.get(rowKey);
if (!row) throw new Error(`unknown row ${rowKey}`);

const legacyRoot = resolve(
  process.env.LEGACY_ROOT || join(root, "target/scorecard-v2-legacy"),
);
const v86dir = resolve(process.env.V86DIR || join(artifacts, "v86"));
const timeoutMs = Number(process.env.SCORECARD_V2_TIMEOUT_MS || 900_000);
const progressIntervalMs = Number(process.env.SCORECARD_V2_PROGRESS_MS || 0);
const executionMode = parseExecutionMode(process.env.SCORECARD_V2_EXECUTION_MODE);
const interpreterOnly = executionMode === "interpreter";
const holdoutMode = row.family === "holdout";
const auditPopulation = process.env.SCORECARD_V2_INPUT_POPULATION || "scorecard-v2-modern";
const allowedPopulations = [
  "scorecard-v2-modern",
  "scorecard-v2-rv64gcv-v1",
  "stock-musl-v1",
  "stock-musl-rv64gcv-v1",
];
if (!allowedPopulations.includes(auditPopulation)) {
  throw new Error(`SCORECARD_V2_INPUT_POPULATION must be one of ${allowedPopulations.join(", ")}`);
}
const stockMuslMode = auditPopulation.startsWith("stock-musl-");
const rv64gcvJitMode = auditPopulation === "scorecard-v2-rv64gcv-v1";
if (holdoutMode && (!interpreterOnly || !["rewrite", "v86"].includes(side))) {
  throw new Error("interpreter holdouts require interpreter mode and rewrite/v86");
}
if (stockMuslMode && (!interpreterOnly || !["rewrite", "v86"].includes(side))) {
  throw new Error("stock-musl-v1 requires interpreter mode and rewrite/v86");
}
if (rv64gcvJitMode && (interpreterOnly || !["rewrite", "v86"].includes(side))) {
  throw new Error("scorecard-v2-rv64gcv-v1 requires JIT mode and rewrite/v86");
}
const rv64InitramfsPath = holdoutMode
  ? resolve(process.env.INTERPRETER_HOLDOUT_RV64_INITRAMFS || "")
  : stockMuslMode
    ? resolve(process.env.INTERPRETER_AUDIT_RV64_INITRAMFS || "")
  : rv64gcvJitMode
    ? resolve(
        process.env.SCORECARD_V2_RV64GCV_RV64_INITRAMFS ||
        process.env.INTERPRETER_AUDIT_RV64_INITRAMFS ||
        join(artifacts, "interpreter-stock-musl-rv64gcv-v1/interpreter-stock-musl-riscv64.cpio"),
      )
  : join(artifacts, "scorecard-v2-modern-riscv64.cpio");
const x86InitramfsPath = holdoutMode
  ? resolve(process.env.INTERPRETER_HOLDOUT_X86_INITRAMFS || "")
  : stockMuslMode
    ? resolve(process.env.INTERPRETER_AUDIT_X86_INITRAMFS || "")
  : rv64gcvJitMode
    ? resolve(
        process.env.SCORECARD_V2_RV64GCV_X86_INITRAMFS ||
        process.env.INTERPRETER_AUDIT_X86_INITRAMFS ||
        join(artifacts, "interpreter-stock-musl-rv64gcv-v1/interpreter-stock-musl-x86.cpio"),
      )
  : join(artifacts, "scorecard-v2-modern-x86.cpio");
const rv64KernelPath = resolve(
  process.env.RV64_KERNEL_IMAGE ||
  (rv64gcvJitMode
    ? join(artifacts, "interpreter-stock-musl-rv64gcv-v1/rv64gcv-linux-Image")
    : join(root, "web/images/alpine/Image")),
);
if (holdoutMode &&
    (!process.env.INTERPRETER_HOLDOUT_RV64_INITRAMFS ||
     !process.env.INTERPRETER_HOLDOUT_X86_INITRAMFS)) {
  throw new Error("set both INTERPRETER_HOLDOUT_RV64_INITRAMFS and INTERPRETER_HOLDOUT_X86_INITRAMFS");
}
if (stockMuslMode &&
    (!process.env.INTERPRETER_AUDIT_RV64_INITRAMFS ||
     !process.env.INTERPRETER_AUDIT_X86_INITRAMFS)) {
  throw new Error("set both INTERPRETER_AUDIT_RV64_INITRAMFS and INTERPRETER_AUDIT_X86_INITRAMFS");
}
const inputPopulation = holdoutMode ? "sealed-interpreter-holdouts-v1" : auditPopulation;
// copy/v86 and the public RV64 scheduler both return through the event loop
// after one CPU slice. The scored harness now matches that product cadence;
// the historical four-slice batch is retained only as an explicit diagnostic.
const pumpCadence = parsePumpCadence();
const pumpCadenceDiagnostic = cadenceDiagnostic(pumpCadence);
const nbenchFixedArraysDiagnostic = process.env.SCORECARD_V2_NBENCH_FIXED_ARRAYS === "1";
const stringArraysDiagnostic = process.env.SCORECARD_V2_STRING_ARRAYS === undefined
  ? null
  : Number(process.env.SCORECARD_V2_STRING_ARRAYS);
const nbenchVariant = process.env.SCORECARD_V2_NBENCH_VARIANT || "fixed";
if (!new Set(["fixed", "selftimed", "native"]).has(nbenchVariant)) {
  throw new Error("SCORECARD_V2_NBENCH_VARIANT must be fixed, selftimed, or native");
}
if (nbenchVariant !== "fixed" && row.family !== "nbench") {
  throw new Error("SCORECARD_V2_NBENCH_VARIANT only applies to BYTEmark rows");
}
const nbenchNativeDiagnostic = nbenchVariant === "native";
const nbenchSelfTimedDiagnostic = nbenchVariant === "selftimed";
const nbenchUnscoredDiagnostic = nbenchNativeDiagnostic || nbenchSelfTimedDiagnostic;
const disableRvJitDiagnostic = process.env.SCORECARD_V2_DISABLE_RV_JIT === "1";
const disableRvJit = interpreterOnly || disableRvJitDiagnostic;
const pageThresholdDiagnostic = process.env.SCORECARD_V2_PAGE_THRESHOLD === undefined
  ? null
  : Number(process.env.SCORECARD_V2_PAGE_THRESHOLD);
const privilegedThresholdMultiplierDiagnostic =
  process.env.SCORECARD_V2_PRIVILEGED_THRESHOLD_MULTIPLIER === undefined
    ? null
    : Number(process.env.SCORECARD_V2_PRIVILEGED_THRESHOLD_MULTIPLIER);
const pageCapDiagnostic = process.env.SCORECARD_V2_REGION_PAGE_CAP === undefined
  ? null
  : Number(process.env.SCORECARD_V2_REGION_PAGE_CAP);
const leaderCapDiagnostic = process.env.SCORECARD_V2_REGION_LEADER_CAP === undefined
  ? null
  : Number(process.env.SCORECARD_V2_REGION_LEADER_CAP);
const inflightDiagnostic = process.env.SCORECARD_V2_PAGE_INFLIGHT === undefined
  ? null
  : Number(process.env.SCORECARD_V2_PAGE_INFLIGHT);
const controlEntriesDiagnostic = process.env.SCORECARD_V2_CONTROL_ENTRIES === undefined
  ? null
  : Number(process.env.SCORECARD_V2_CONTROL_ENTRIES);
const privilegedControlEntriesDiagnostic =
  process.env.SCORECARD_V2_PRIVILEGED_CONTROL_ENTRIES === undefined
    ? null
    : Number(process.env.SCORECARD_V2_PRIVILEGED_CONTROL_ENTRIES);
const stablePageChainDiagnostic = process.env.SCORECARD_V2_STABLE_PAGE_CHAIN === undefined
  ? null
  : Number(process.env.SCORECARD_V2_STABLE_PAGE_CHAIN);
const interpreterFusedMemoryDiagnostic =
  process.env.SCORECARD_V2_INTERPRETER_FUSED_MEMORY === undefined
    ? null
    : Number(process.env.SCORECARD_V2_INTERPRETER_FUSED_MEMORY);
const integratedScalarT0Diagnostic =
  process.env.SCORECARD_V2_INTEGRATED_SCALAR_T0 === undefined
    ? null
    : Number(process.env.SCORECARD_V2_INTEGRATED_SCALAR_T0);
const staticSystemT0Diagnostic =
  process.env.SCORECARD_V2_STATIC_SYSTEM_T0 === undefined
    ? null
    : Number(process.env.SCORECARD_V2_STATIC_SYSTEM_T0);
const sampledStaticT0Diagnostic =
  process.env.SCORECARD_V2_SAMPLED_STATIC_T0 === undefined
    ? null
    : Number(process.env.SCORECARD_V2_SAMPLED_STATIC_T0);
const sampledStaticT0BackoffDiagnostic =
  process.env.SCORECARD_V2_SAMPLED_STATIC_T0_BACKOFF === undefined
    ? null
    : Number(process.env.SCORECARD_V2_SAMPLED_STATIC_T0_BACKOFF);
const pagePolicyDiagnostic = [
  pageThresholdDiagnostic,
  privilegedThresholdMultiplierDiagnostic,
  pageCapDiagnostic,
  leaderCapDiagnostic,
  inflightDiagnostic,
  controlEntriesDiagnostic,
  privilegedControlEntriesDiagnostic,
  stablePageChainDiagnostic,
].some((value) => value !== null);
const clockDiagnostic = process.env.SCORECARD_V2_CLOCK_DIAGNOSTIC === "1";
const execPid1Diagnostic = process.env.SCORECARD_V2_EXEC_PID1 === "1";
const userJitOnlyDiagnostic = process.env.SCORECARD_V2_USER_JIT_ONLY === "1";
const blockJitDiagnostic = process.env.SCORECARD_V2_BLOCK_JIT_ONLY === "1";
const profileDiagnostic = process.env.SCORECARD_V2_PROFILE === "1";
const engineProfileDir = process.env.SCORECARD_V2_ENGINE_PROFILE_DIR
  ? resolve(process.env.SCORECARD_V2_ENGINE_PROFILE_DIR)
  : null;
const engineProfileInterval = Number(process.env.SCORECARD_V2_ENGINE_PROFILE_INTERVAL || 250);
const engineTierMarkersDiagnostic = process.env.SCORECARD_V2_ENGINE_TIER_MARKERS === "1";
const engineVariantDiagnostic = process.env.SCORECARD_V2_ENGINE_VARIANT || null;
const jitModuleCaptureDir = process.env.SCORECARD_V2_CAPTURE_JIT_MODULES
  ? resolve(process.env.SCORECARD_V2_CAPTURE_JIT_MODULES)
  : null;
const rewriteWasmOverride = process.env.SCORECARD_V2_REWRITE_WASM
  ? resolve(process.env.SCORECARD_V2_REWRITE_WASM)
  : null;
const profileSampleShift = Number(process.env.SCORECARD_V2_PROFILE_SHIFT || 8);
const regionTlbCacheDiagnostic = process.env.SCORECARD_V2_REGION_TLB_CACHE === "1";
const pageTemplateProbeDiagnostic = process.env.SCORECARD_V2_PAGE_TEMPLATE_PROBE === "1";
const pageTemplateReuseDiagnostic = process.env.SCORECARD_V2_PAGE_TEMPLATE_REUSE === "1";
const fastmemPreflightDiagnostic = process.env.SCORECARD_V2_FASTMEM_PREFLIGHT === "1";
const regionTlbCacheMinAccesses = Number(
  process.env.SCORECARD_V2_REGION_TLB_CACHE_MIN_ACCESSES || 2,
);
const v86ExecutionProofMode = process.env.SCORECARD_V2_V86_EXEC_PROOF === "1";
const rewritePolicy = process.env.SCORECARD_V2_REWRITE_POLICY || "production";
if (!['production', 'compat'].includes(rewritePolicy)) {
  throw new Error("SCORECARD_V2_REWRITE_POLICY must be production or compat");
}
if (interpreterOnly && v86ExecutionProofMode) {
  throw new Error("SCORECARD_V2_V86_EXEC_PROOF is incompatible with interpreter mode");
}
if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 60_000) {
  throw new Error("SCORECARD_V2_TIMEOUT_MS must be an integer >= 60000");
}
if (!Number.isSafeInteger(progressIntervalMs) || progressIntervalMs < 0) {
  throw new Error("SCORECARD_V2_PROGRESS_MS must be a non-negative integer");
}
if (
  pageThresholdDiagnostic !== null &&
  (!Number.isSafeInteger(pageThresholdDiagnostic) ||
    pageThresholdDiagnostic < 1 || pageThresholdDiagnostic > 0xffff_ffff)
) {
  throw new Error("SCORECARD_V2_PAGE_THRESHOLD must be an integer from 1 through 4294967295");
}
if (
  privilegedThresholdMultiplierDiagnostic !== null &&
  (!Number.isSafeInteger(privilegedThresholdMultiplierDiagnostic) ||
    privilegedThresholdMultiplierDiagnostic < 1 ||
    privilegedThresholdMultiplierDiagnostic > 1024)
) {
  throw new Error(
    "SCORECARD_V2_PRIVILEGED_THRESHOLD_MULTIPLIER must be an integer from 1 through 1024",
  );
}
if (pageCapDiagnostic !== null &&
    (!Number.isSafeInteger(pageCapDiagnostic) || pageCapDiagnostic < 1 || pageCapDiagnostic > 16)) {
  throw new Error("SCORECARD_V2_REGION_PAGE_CAP must be an integer from 1 through 16");
}
if (leaderCapDiagnostic !== null &&
    (!Number.isSafeInteger(leaderCapDiagnostic) ||
      leaderCapDiagnostic < 2 || leaderCapDiagnostic > 4096)) {
  throw new Error("SCORECARD_V2_REGION_LEADER_CAP must be an integer from 2 through 4096");
}
if (inflightDiagnostic !== null &&
    (!Number.isSafeInteger(inflightDiagnostic) || inflightDiagnostic < 1 || inflightDiagnostic > 8)) {
  throw new Error("SCORECARD_V2_PAGE_INFLIGHT must be an integer from 1 through 8");
}
if (controlEntriesDiagnostic !== null &&
    controlEntriesDiagnostic !== 0 && controlEntriesDiagnostic !== 1) {
  throw new Error("SCORECARD_V2_CONTROL_ENTRIES must be 0 or 1");
}
if (privilegedControlEntriesDiagnostic !== null &&
    privilegedControlEntriesDiagnostic !== 0 && privilegedControlEntriesDiagnostic !== 1) {
  throw new Error("SCORECARD_V2_PRIVILEGED_CONTROL_ENTRIES must be 0 or 1");
}
if (stablePageChainDiagnostic !== null &&
    stablePageChainDiagnostic !== 0 && stablePageChainDiagnostic !== 1) {
  throw new Error("SCORECARD_V2_STABLE_PAGE_CHAIN must be 0 or 1");
}
if (interpreterFusedMemoryDiagnostic !== null &&
    interpreterFusedMemoryDiagnostic !== 0 && interpreterFusedMemoryDiagnostic !== 1) {
  throw new Error("SCORECARD_V2_INTERPRETER_FUSED_MEMORY must be 0 or 1");
}
if (integratedScalarT0Diagnostic !== null &&
    integratedScalarT0Diagnostic !== 0 && integratedScalarT0Diagnostic !== 1) {
  throw new Error("SCORECARD_V2_INTEGRATED_SCALAR_T0 must be 0 or 1");
}
if (staticSystemT0Diagnostic !== null &&
    staticSystemT0Diagnostic !== 0 && staticSystemT0Diagnostic !== 1) {
  throw new Error("SCORECARD_V2_STATIC_SYSTEM_T0 must be 0 or 1");
}
if (sampledStaticT0Diagnostic !== null &&
    sampledStaticT0Diagnostic !== 0 && sampledStaticT0Diagnostic !== 1) {
  throw new Error("SCORECARD_V2_SAMPLED_STATIC_T0 must be 0 or 1");
}
if (sampledStaticT0Diagnostic !== null && staticSystemT0Diagnostic === null) {
  throw new Error("SCORECARD_V2_SAMPLED_STATIC_T0 requires SCORECARD_V2_STATIC_SYSTEM_T0");
}
if (sampledStaticT0BackoffDiagnostic !== null &&
    sampledStaticT0BackoffDiagnostic !== 0 && sampledStaticT0BackoffDiagnostic !== 1) {
  throw new Error("SCORECARD_V2_SAMPLED_STATIC_T0_BACKOFF must be 0 or 1");
}
if (sampledStaticT0BackoffDiagnostic !== null && sampledStaticT0Diagnostic === null) {
  throw new Error(
    "SCORECARD_V2_SAMPLED_STATIC_T0_BACKOFF requires SCORECARD_V2_SAMPLED_STATIC_T0",
  );
}
if (sampledStaticT0BackoffDiagnostic === 1 && sampledStaticT0Diagnostic !== 1) {
  throw new Error("sampled static-T0 backoff requires sampled static T0 enabled");
}
if (!Number.isSafeInteger(profileSampleShift) || profileSampleShift < 0 || profileSampleShift > 20) {
  throw new Error("SCORECARD_V2_PROFILE_SHIFT must be an integer from 0 through 20");
}
if (jitModuleCaptureDir && side !== "rewrite") {
  throw new Error("SCORECARD_V2_CAPTURE_JIT_MODULES currently requires rewrite");
}
if (
  !Number.isSafeInteger(engineProfileInterval) ||
  engineProfileInterval < 50 ||
  engineProfileInterval > 10_000
) {
  throw new Error("SCORECARD_V2_ENGINE_PROFILE_INTERVAL must be an integer from 50 through 10000");
}
if (
  stringArraysDiagnostic !== null &&
  (!Number.isSafeInteger(stringArraysDiagnostic) ||
    stringArraysDiagnostic < 1 || stringArraysDiagnostic > 4096)
) {
  throw new Error("SCORECARD_V2_STRING_ARRAYS must be an integer from 1 through 4096");
}
if (
  !Number.isSafeInteger(regionTlbCacheMinAccesses) ||
  regionTlbCacheMinAccesses < 1 ||
  regionTlbCacheMinAccesses > 64
) {
  throw new Error("SCORECARD_V2_REGION_TLB_CACHE_MIN_ACCESSES must be an integer from 1 through 64");
}
if (execPid1Diagnostic && (!clockDiagnostic || row.family !== "python")) {
  throw new Error("SCORECARD_V2_EXEC_PID1 requires the Python clock diagnostic");
}
if (userJitOnlyDiagnostic && row.family === "boot") {
  throw new Error(
    "SCORECARD_V2_USER_JIT_ONLY is not a valid boot diagnostic: one interpreter " +
    "slice can cross supervisor/user transitions",
  );
}
if (fastmemPreflightDiagnostic && (side === "v86" || rowKey !== "string")) {
  throw new Error("SCORECARD_V2_FASTMEM_PREFLIGHT requires rewrite/string or legacy/string");
}

const tick = () => new Promise((done) => setImmediate(done));
const decoder = new TextDecoder();
const encoder = new TextEncoder();
const fixedNbenchParameters = row.family === "nbench"
  ? NBENCH_WORKLOAD_CONTRACT.fixedParameters[rowKey].map((parameter) =>
      rowKey === "string" && stringArraysDiagnostic !== null &&
        parameter.startsWith("NUMSTRARRAYS=")
        ? `NUMSTRARRAYS=${stringArraysDiagnostic}`
        : parameter
    )
  : null;
const exactBuffer = (bytes) => bytes.buffer.slice(
  bytes.byteOffset,
  bytes.byteOffset + bytes.byteLength,
);

let activeEngineProfile = null;

function engineTierMarker(stage) {
  if (engineTierMarkersDiagnostic) {
    process.stdout.write(`ENGINE_TIER_MARKER side=${side} row=${rowKey} stage=${stage}\n`);
  }
}

async function writeCapturedJitModules(directory, captures) {
  if (!directory) return null;
  await mkdir(directory, { recursive: true });
  const unique = new Map();
  for (const capture of captures) {
    let entry = unique.get(capture.sha256);
    if (!entry) {
      entry = { sha256: capture.sha256, bytes: capture.bytes, occurrences: [] };
      unique.set(capture.sha256, entry);
    }
    entry.occurrences.push({ phase: capture.phase, metadata: capture.metadata });
  }
  const modules = [];
  for (const entry of unique.values()) {
    const filename = `${entry.sha256}.wasm`;
    await writeFile(join(directory, filename), entry.bytes);
    modules.push({
      sha256: entry.sha256,
      bytes: entry.bytes.length,
      filename,
      occurrences: entry.occurrences,
    });
  }
  modules.sort((left, right) => right.bytes - left.bytes ||
    left.sha256.localeCompare(right.sha256));
  const manifest = {
    format: "rv64-scorecard-v2-jit-module-capture-v1",
    captures: captures.length,
    uniqueModules: modules.length,
    modules,
  };
  const manifestPath = join(directory, "manifest.json");
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  return { manifestPath, captures: captures.length, uniqueModules: modules.length };
}

function installJitModuleCapture(vm) {
  const captures = [];
  if (!jitModuleCaptureDir) return captures;
  vm.onJitModule = (bytes, metadata) => {
    captures.push({
      bytes,
      sha256: sha256(bytes),
      phase: vm.scorecardPhase ?? "unattributed",
      metadata: JSON.parse(JSON.stringify(metadata, (_key, value) =>
        typeof value === "bigint" ? value.toString() : value)),
    });
  };
  return captures;
}

async function startEngineProfile(phase) {
  if (!engineProfileDir) return;
  if (activeEngineProfile) throw new Error("engine CPU profile already active");
  const { Session } = await import("node:inspector/promises");
  const session = new Session();
  session.connect();
  await session.post("Profiler.enable");
  await session.post("Profiler.setSamplingInterval", { interval: engineProfileInterval });
  await session.post("Profiler.start");
  activeEngineProfile = { phase, session };
}

async function stopEngineProfile(phase) {
  if (!engineProfileDir) return null;
  if (!activeEngineProfile || activeEngineProfile.phase !== phase) {
    throw new Error(`engine CPU profile phase mismatch: ${phase}`);
  }
  const { session } = activeEngineProfile;
  const { profile } = await session.post("Profiler.stop");
  await session.post("Profiler.disable");
  session.disconnect();
  activeEngineProfile = null;
  await mkdir(engineProfileDir, { recursive: true });
  const filename = `${side}-${rowKey}-${phase}.cpuprofile`;
  const path = join(engineProfileDir, filename);
  await writeFile(path, JSON.stringify(profile));
  return {
    path,
    intervalMicroseconds: engineProfileInterval,
    samples: profile.samples?.length ?? 0,
  };
}

function makeConsole() {
  let text = "";
  const watches = new Set();
  const append = (bytes) => {
    text += decoder.decode(bytes, { stream: true });
    const now = performance.now();
    for (const watch of watches) {
      if (watch.time === null && text.indexOf(watch.marker, watch.offset) !== -1) {
        watch.time = now;
      }
    }
  };
  return {
    append,
    get text() { return text; },
    watch(marker, offset = text.length) {
      const watch = { marker, offset, time: null };
      watches.add(watch);
      return watch;
    },
    segment(offset) { return text.slice(offset); },
  };
}

function applyPagePolicyDiagnostics(vm) {
  if (!pagePolicyDiagnostic) return;
  if (side !== "rewrite") throw new Error("page-policy diagnostics require rewrite");
  if (pageThresholdDiagnostic !== null) vm.ex.jit_set_page_threshold(pageThresholdDiagnostic);
  if (privilegedThresholdMultiplierDiagnostic !== null) {
    vm.ex.jit_set_privileged_page_threshold_multiplier(
      privilegedThresholdMultiplierDiagnostic,
    );
  }
  if (pageCapDiagnostic !== null) vm.ex.jit_set_region_page_cap(pageCapDiagnostic);
  if (leaderCapDiagnostic !== null) vm.ex.jit_set_region_leader_cap(leaderCapDiagnostic);
  if (inflightDiagnostic !== null) vm.ex.jit_set_page_inflight_limit(inflightDiagnostic);
  if (controlEntriesDiagnostic !== null) {
    vm.ex.jit_set_page_control_entries(controlEntriesDiagnostic);
  }
  if (privilegedControlEntriesDiagnostic !== null) {
    vm.ex.jit_set_page_privileged_control_entries(privilegedControlEntriesDiagnostic);
  }
  if (stablePageChainDiagnostic !== null) {
    vm.ex.jit_set_page_stable_chain(stablePageChainDiagnostic);
  }
}

function applyInterpreterFusedMemoryDiagnostic(vm) {
  if (interpreterFusedMemoryDiagnostic === null) return;
  if (side !== "rewrite") throw new Error("interpreter fused-memory diagnostic requires rewrite");
  if (typeof vm.ex.jit_set_interpreter_fused_memory !== "function") {
    throw new Error("rewrite runtime lacks jit_set_interpreter_fused_memory");
  }
  vm.ex.jit_set_interpreter_fused_memory(interpreterFusedMemoryDiagnostic);
}

function applyIntegratedScalarT0Diagnostic(vm) {
  if (integratedScalarT0Diagnostic === null) return;
  if (side !== "rewrite") throw new Error("integrated scalar T0 diagnostic requires rewrite");
  if (typeof vm.ex.jit_set_integrated_scalar_t0 !== "function") {
    throw new Error("rewrite runtime lacks jit_set_integrated_scalar_t0");
  }
  vm.ex.jit_set_integrated_scalar_t0(integratedScalarT0Diagnostic);
}

function prepareStaticSystemT0Diagnostic(vm) {
  const diagnostic = staticSystemT0Diagnostic !== null ||
    sampledStaticT0Diagnostic !== null || sampledStaticT0BackoffDiagnostic !== null;
  if (!diagnostic) return null;
  if (side !== "rewrite") throw new Error("static system T0 diagnostic requires rewrite");
  if (typeof vm.ex.jit_static_t0_system_prepare !== "function" ||
      typeof vm.ex.jit_set_static_t0_system !== "function" ||
      (sampledStaticT0Diagnostic !== null &&
       typeof vm.ex.jit_set_static_t0_sampled !== "function") ||
      (sampledStaticT0BackoffDiagnostic !== null &&
       typeof vm.ex.jit_set_static_t0_sampled_backoff !== "function")) {
    throw new Error("rewrite runtime lacks static system T0 lifecycle exports");
  }
  const modulesBefore = vm.jitRegCount ?? 0;
  const index = vm.ex.jit_static_t0_system_prepare();
  if (index < 0) throw new Error("static system T0 auxiliary module did not register");
  const modulesAfter = vm.jitRegCount ?? 0;
  if (modulesAfter !== modulesBefore + 1) {
    throw new Error(
      `static system T0 lifecycle registered ${modulesAfter - modulesBefore} modules, expected 1`,
    );
  }
  vm.ex.jit_set_static_t0_system(staticSystemT0Diagnostic);
  if (sampledStaticT0Diagnostic !== null) {
    vm.ex.jit_set_static_t0_sampled(sampledStaticT0Diagnostic);
  }
  if (sampledStaticT0BackoffDiagnostic !== null) {
    vm.ex.jit_set_static_t0_sampled_backoff(sampledStaticT0BackoffDiagnostic);
  }
  vm.scorecardStaticT0 = {
    enabled: staticSystemT0Diagnostic === 1,
    sampled: sampledStaticT0Diagnostic === 1,
    sampledBackoff: sampledStaticT0BackoffDiagnostic === 1,
    index,
    modulesBefore,
    modulesAfter,
  };
  return vm.scorecardStaticT0;
}

function readStaticT0RuntimeProof(vm, lifecycle) {
  if (!lifecycle) return null;
  return {
    sampledInstructions: vm.ex.jit_static_t0_stat(8).toString(),
    samples: vm.ex.jit_static_t0_stat(9).toString(),
    shortMarks: vm.ex.jit_static_t0_stat(11).toString(),
    shortBypasses: vm.ex.jit_static_t0_stat(12).toString(),
    errors: vm.ex.jit_static_t0_stat(7).toString(),
  };
}

async function waitUntil(predicate, pump, timeout = timeoutMs, onProgress = null) {
  const deadline = performance.now() + timeout;
  let nextProgress = performance.now() + progressIntervalMs;
  let iteration = 0;
  while (!predicate()) {
    pump();
    if (onProgress && progressIntervalMs && performance.now() >= nextProgress) {
      onProgress();
      nextProgress = performance.now() + progressIntervalMs;
    }
    const shouldYield = shouldYieldAfterPump(iteration, pumpCadence);
    iteration++;
    if (shouldYield) await tick();
    if (performance.now() > deadline) return false;
  }
  return true;
}

function rvInstructionCounter(vm, machine) {
  return machine === "virt"
    ? () => vm.virtInsnCount?.() ?? 0n
    : () => vm.sysInsnCount?.() ?? 0n;
}

async function settleRv(vm, pump, timeout = 30_000) {
  const started = performance.now();
  const before = pagePolicySnapshot(vm);
  const deadline = started + timeout;
  let stable = 0;
  while (performance.now() < deadline) {
    const pending = Number(vm.ex.sys_pending_builds?.() ?? 0);
    const policy = pagePolicySnapshot(vm);
    const queued = Number(policy?.queued ?? 0);
    const policyPending = Number(policy?.pending ?? 0);
    if (pending === 0 && queued === 0 && policyPending === 0) {
      if (++stable >= 3) {
        return {
          complete: true,
          ms: performance.now() - started,
          policyBefore: before,
          policyAfter: policy,
        };
      }
      await tick();
      continue;
    }
    stable = 0;
    // A queued page-policy candidate is issued at an execution boundary. The
    // shell is idle here, so this does not warm the measured guest program.
    if (queued || (!pending && policyPending)) pump();
    await tick();
  }
  return {
    complete: false,
    ms: performance.now() - started,
    policyBefore: before,
    policyAfter: pagePolicySnapshot(vm),
  };
}

function rvPhaseCommand(family, phase) {
  const label = phase.toUpperCase();
  const begin = `echo SCORECARD_V2_${label}_"BEGIN"`;
  const exit = "echo SCORECARD_V2_\"EXIT\"=$rc";
  const complete = `echo SCORECARD_V2_${label}_"COMPLETE"`;
  if (family === "compute" || family === "holdout") {
    const directory = family === "holdout" ? "holdout" : "scorecard";
    return `${begin}; /opt/${directory}/${rowKey}; rc=$?; ${exit}; ${complete}\n`;
  }
  if (family === "python") {
    if (clockDiagnostic) {
      const probe = "import glob,os,runpy,time;" +
        "s=lambda p:tuple(map(int,open(p).read().split()[13:15]));" +
        "k=lambda p:tuple(map(int,open(p).read().split()[:3])) if os.path.exists(p) else (0,0,0);" +
        "g=lambda:tuple(map(int,open(\"/proc/stat\").readline().split()[1:]));" +
        "h=lambda:{int(p.split(\"/\")[2]):s(p) for p in glob.glob(\"/proc/[0-9]*/stat\")};" +
        "c=s(\"/proc/self/stat\");p=s(\"/proc/1/stat\");j=k(\"/proc/self/schedstat\");q=g();" +
        "v=h();a=time.process_time();b=time.monotonic();" +
        "runpy.run_path(\"/opt/scorecard/fib.py\");" +
        "d=s(\"/proc/self/stat\");r=s(\"/proc/1/stat\");l=k(\"/proc/self/schedstat\");z=g();w=h();" +
        "e=sorted(((i,w[i][0]-v.get(i,(0,0))[0],w[i][1]-v.get(i,(0,0))[1]) " +
        "for i in w),key=lambda x:x[1]+x[2],reverse=True);" +
        "print(\"CLOCK_DIAGNOSTIC process=%.9f monotonic=%.9f " +
        "uticks=%d sticks=%d p1uticks=%d p1sticks=%d cpu=%s sched=%s tasks=%s\"%" +
        "(time.process_time()-a,time.monotonic()-b,d[0]-c[0],d[1]-c[1]," +
        "r[0]-p[0],r[1]-p[1],\",\".join(map(str,(z[i]-q[i] for i in range(len(q)))))," +
        "\",\".join(map(str,(l[i]-j[i] for i in range(3))))," +
        "\";\".join(\"%d:%d:%d\"%x for x in e[:8]))," +
        "flush=True)";
      if (execPid1Diagnostic) return `${begin}; exec /usr/bin/python3 -c '${probe}'\n`;
      return `${begin}; /usr/bin/python3 -c '${probe}'; rc=$?; ${exit}; ${complete}\n`;
    }
    return `${begin}; /usr/bin/python3 /opt/scorecard/fib.py; rc=$?; ${exit}; ${complete}\n`;
  }
  if (family === "compile") {
    return `${begin}; rm -f /tmp/w.o; echo RUN_"START"; /opt/scorecard/tcc -c /opt/scorecard/w.c -o /tmp/w.o 2>/tmp/tcc-error; rc=$?; echo RUN_"DONE"; md5sum /tmp/w.o; ${exit}; ${complete}\n`;
  }
  // nbench uppercases its -c argument. Run from /tmp and use the already
  // uppercase relative name C; an absolute /tmp/C silently becomes /TMP/C.
  const executable = nbenchNativeDiagnostic
    ? NBENCH_WORKLOAD_CONTRACT.nativeDiagnosticExecutable
    : nbenchSelfTimedDiagnostic
      ? NBENCH_WORKLOAD_CONTRACT.selfTimedDiagnosticExecutable
      : NBENCH_WORKLOAD_CONTRACT.executable;
  return `${begin}; cd /tmp && /opt/scorecard/${executable} -cC; rc=$?; ${exit}; ${complete}\n`;
}

function rvPhaseMarkers(family, phase) {
  if (family === "compute" || family === "holdout") return ["BENCH_START", "BENCH_DONE"];
  if (family === "python") return ["FIB_START", "FIB_DONE"];
  if (family === "compile") return ["RUN_START", "RUN_DONE"];
  if (family === "nbench") {
    return [
      `SCORECARD_V2_${phase.toUpperCase()}_BEGIN`,
      `SCORECARD_V2_${phase.toUpperCase()}_COMPLETE`,
    ];
  }
  return [null, null];
}

function phaseResult(rowSpec, phase, segment, startWatch, endWatch, counters, hostMs) {
  const exit = segment.match(/SCORECARD_V2_EXIT=(\d+)/)?.[1];
  if (!execPid1Diagnostic && exit !== "0") {
    throw new Error(
      `${side}/${rowKey}/${phase} guest exit=${exit ?? "missing"}: ` +
      JSON.stringify(segment.slice(-4000)),
    );
  }
  if (rowSpec.family === "nbench") {
    if (nbenchUnscoredDiagnostic) {
      const parsed = parseNbench(segment, rowSpec.nbenchName);
      if (!(parsed.value > 0)) {
        throw new Error(
          `${side}/${rowKey}/${phase} nbench value missing: ` +
          JSON.stringify(segment.slice(-4000)),
        );
      }
      return { value: parsed.value, unit: "iterations_per_second", ...parsed, counters, hostMs };
    }
    if (!segment.includes(`SCORECARD_FIXED_WORK fid=${rowSpec.nbenchId}`)) {
      throw new Error(`${side}/${rowKey}/${phase} fixed-work execution proof missing`);
    }
    if (startWatch?.time === null || endWatch?.time === null) {
      throw new Error(`${side}/${rowKey}/${phase} fixed-work timing markers missing`);
    }
    return {
      value: endWatch.time - startWatch.time,
      unit: "ms",
      fixedWork: {
        fid: rowSpec.nbenchId,
        parameters: fixedNbenchParameters,
      },
      counters,
      hostMs,
    };
  }
  if (startWatch?.time === null || endWatch?.time === null) {
    throw new Error(`${side}/${rowKey}/${phase} timing markers missing`);
  }
  const result = {
    value: endWatch.time - startWatch.time,
    unit: "ms",
    counters,
    hostMs,
  };
  if (rowSpec.family === "compute" || rowSpec.family === "holdout") {
    result.checksum = segment.match(/checksum=(0x[0-9a-f]+)/)?.[1] ?? null;
    if (!result.checksum) throw new Error(`${side}/${rowKey}/${phase} checksum missing`);
  } else if (rowSpec.family === "python") {
    result.checksum = segment.match(/fib\(30\)=\s*(\d+)/)?.[1] ?? null;
    if (result.checksum !== "832040") throw new Error(`${side}/${rowKey}/${phase} bad fib result`);
    if (clockDiagnostic) {
      const clock = segment.match(
        /CLOCK_DIAGNOSTIC process=([\d.]+) monotonic=([\d.]+) uticks=(\d+) sticks=(\d+) p1uticks=(\d+) p1sticks=(\d+) cpu=([\d,]+) sched=([\d,]+) tasks=([^\r\n]+)/,
      );
      if (!clock) throw new Error(`${side}/${rowKey}/${phase} clock diagnostic missing`);
      result.clockDiagnostic = {
        processSeconds: Number(clock[1]),
        monotonicSeconds: Number(clock[2]),
        userTicks: Number(clock[3]),
        systemTicks: Number(clock[4]),
        pid1UserTicks: Number(clock[5]),
        pid1SystemTicks: Number(clock[6]),
        globalCpuTicks: clock[7].split(",").map(Number),
        schedstatDeltas: clock[8].split(",").map(Number),
        taskTickDeltas: clock[9],
      };
      process.stderr.write(
        `[scorecard-v2 clock] ${JSON.stringify(result.clockDiagnostic)}\n`,
      );
    }
  } else if (rowSpec.family === "compile") {
    result.md5 = segment.match(/([0-9a-f]{32})\s+\/tmp\/w\.o/)?.[1] ?? null;
    if (!result.md5) throw new Error(`${side}/${rowKey}/${phase} object hash missing`);
  }
  return result;
}

function readRvProfile(vm) {
  if (!profileDiagnostic) return null;
  const stat = (index) => Number(vm.ex.jit_stat(index));
  const dispatch = [];
  const transitions = [];
  for (let index = 0; index < 8192; index++) {
    const calls = Number(vm.ex.dprof_get(1, index));
    if (calls) {
      const retired = Number(vm.ex.dprof_get(2, index));
      dispatch.push({
        pc: `0x${vm.ex.dprof_get(0, index).toString(16)}`,
        calls,
        retired,
        instructionsPerCall: retired / calls,
      });
    }
    const edges = Number(vm.ex.eprof_get(2, index));
    if (edges) {
      const retired = Number(vm.ex.eprof_get(3, index));
      transitions.push({
        source: `0x${vm.ex.eprof_get(0, index).toString(16)}`,
        target: `0x${vm.ex.eprof_get(1, index).toString(16)}`,
        edges,
        retired,
        instructionsPerEdge: retired / edges,
      });
    }
  }
  const fallback = [];
  for (let index = 0; index < 1024; index++) {
    const stretches = Number(vm.ex.ihist_get(1, index));
    if (stretches) {
      fallback.push({
        key: `0x${vm.ex.ihist_get(0, index).toString(16)}`,
        stretches,
        interpretedInstructions: Number(vm.ex.ihist_get(2, index)),
      });
    }
  }
  dispatch.sort((a, b) => b.calls - a.calls);
  transitions.sort((a, b) => b.edges - a.edges);
  fallback.sort((a, b) => b.interpretedInstructions - a.interpretedInstructions);
  return {
    sampleShift: profileSampleShift,
    executionMix: {
      blockCalls: stat(46),
      blockInstructions: stat(47),
      regionCalls: stat(48),
      regionInstructions: stat(49),
      // Exact emitted structured-member entries and their scheduled static
      // instruction volume. Actual retirement remains regionInstructions;
      // their delta exposes precise side exits and accelerated loop helpers.
      structuredMemberEntries: stat(130),
      structuredScheduledInstructions: stat(131),
      structuredX2WriteMemberEntries: stat(132),
      structuredStackMemoryInstructions: stat(133),
      // Lift metrics: ordinary ALU/other, loads, stores, control, FP.
      instructionClasses: Array.from({ length: 5 }, (_, index) => stat(50 + index)),
      // Loads u8/u16/u32/u64, stores u8/u16/u32/u64, FP loads, FP stores.
      memoryClasses: Array.from({ length: 10 }, (_, index) => stat(55 + index)),
      // Conditional, direct, indirect control transfers.
      controlClasses: Array.from({ length: 3 }, (_, index) => stat(65 + index)),
      // add/sub, shifts, comparisons, logic, mul/div/rem.
      aluClasses: Array.from({ length: 5 }, (_, index) => stat(68 + index)),
    },
    topDispatch: dispatch.slice(0, 30),
    topTransitions: transitions.slice(0, 30),
    topFallback: fallback.slice(0, 30),
  };
}

async function loadRv(sideName) {
  const rewrite = sideName === "rewrite";
  const variantRoot = rewrite ? root : legacyRoot;
  const loaderPath = rewrite
    ? join(variantRoot, "web/rv64.js")
    : join(variantRoot, "web/rv64.js");
  const wasmPath = rewrite
    ? rewriteWasmOverride ??
      join(variantRoot, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm")
    : join(variantRoot, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
  const [loader, loaderBytes, wasm] = await Promise.all([
    import(pathToFileURL(loaderPath).href),
    readFile(loaderPath),
    readFile(wasmPath),
  ]);
  const identity = {
    root: variantRoot,
    loaderSha256: sha256(loaderBytes),
    wasmSha256: sha256(wasm),
    ...(rewrite ? {} : { comparator: LEGACY_MODERN_COMPARATOR.name }),
  };
  if (!rewrite && (
    identity.loaderSha256 !== LEGACY_MODERN_COMPARATOR.loaderSha256 ||
    identity.wasmSha256 !== LEGACY_MODERN_COMPARATOR.wasmSha256
  )) {
    throw new Error(
      `legacy release identity mismatch: loader=${identity.loaderSha256} ` +
      `wasm=${identity.wasmSha256}`,
    );
  }
  if (!rewrite && !LEGACY_MODERN_COMPARATOR.modernVirtJit) {
    throw new Error("legacy comparator is not approved for generated execution on VirtMachine");
  }
  return {
    RV64Debug: loader.RV64Debug,
    wasm,
    identity,
  };
}

async function loadModernRvInputs() {
  const [kernel, initrd] = await Promise.all([
    readFile(rv64KernelPath),
    readFile(rv64InitramfsPath),
  ]);
  return { kernel, initrd };
}

function configureRvExecution(vm) {
  const configuredPolicy = configureRvPolicy(vm, side, rewritePolicy);
  if (disableRvJit) vm.ex.jit_set_enabled(0);
  if (!interpreterOnly) return configuredPolicy;
  return {
    name: "interpreter-only",
    disableJit: true,
    configuredBasePolicy: configuredPolicy.name,
  };
}

function proveRvInterpreterBypass(vm, label) {
  const value = (candidate) => typeof candidate === "bigint"
    ? candidate
    : BigInt(candidate ?? 0);
  const activity = {
    generatedInstructions: value(vm.ex.jit_stat(0)),
    dispatches: value(vm.ex.jit_stat(1)),
    systemEntries: value(vm.ex.jit_stat(3)),
    fallbackCalls: value(vm.ex.jit_stat(4)),
    fallbackInstructions: value(vm.ex.jit_stat(5)),
    regionsIssued: value(vm.ex.jit_stat(12)),
    regionsLanded: value(vm.ex.jit_stat(13)),
    regionEntries: value(vm.ex.jit_stat(25)),
    regionBatches: value(vm.ex.jit_stat(43)),
    userTranslationAttempts: value(vm.ex.jit_stat(74)),
    systemTranslationAttempts: value(vm.ex.jit_stat(77)),
    registeredModules: value(vm.jitRegCount),
    registeredBytes: value(vm.jitRegBytes),
    pendingBuilds: value(vm.ex.sys_pending_builds?.()),
  };
  const nonzero = Object.entries(activity).filter(([, count]) => count !== 0n);
  if (nonzero.length) {
    throw new Error(
      `${label} interpreter-only proof failed: ` +
      nonzero.map(([name, count]) => `${name}=${count}`).join(" "),
    );
  }
  return Object.fromEntries(
    Object.entries(activity).map(([name, count]) => [name, count.toString()]),
  );
}

async function bootRvModernMachine(vm, console, inputs) {
  vm.onWrite = (_fd, bytes) => console.append(bytes);
  vm.bootVirtLinuxDirect({
    kernel: new Uint8Array(inputs.kernel),
    initrd: new Uint8Array(inputs.initrd),
    cmdline: "console=ttyS0 rdinit=/init",
    ramMB: 512,
  });
  prepareStaticSystemT0Diagnostic(vm);
  const ready = await waitUntil(
    () => console.text.includes("SCORECARD_V2_READY"),
    () => vm.runVirtSystem(2_000_000n),
    180_000,
  );
  if (!ready) throw new Error(`${side}/${rowKey} modern Linux boot failed: ${console.text.slice(-3000)}`);
  const guest = parseGuestIdentity(console.text);
  const guestProblems = validateGuestIdentity(guest, side);
  if (guestProblems.length) {
    throw new Error(`${side}/${rowKey} guest contract failed: ${guestProblems.join("; ")}`);
  }
  return guest;
}

async function prepareRvWorkload(vm, console) {
  const inputs = await loadModernRvInputs();
  const guest = await bootRvModernMachine(vm, console, inputs);
  vm.virtConsoleInput(encoder.encode("stty -echo 2>/dev/null; echo SCORECARD_V2_SHELL_READY\n"));
  const shellReady = await waitUntil(
    () => console.text.includes("SCORECARD_V2_SHELL_READY"),
    () => vm.runVirtSystem(500_000n),
    30_000,
  );
  if (!shellReady) throw new Error(`${side}/${rowKey} modern Alpine shell setup failed`);

  if (fastmemPreflightDiagnostic) {
    vm.virtConsoleInput(encoder.encode(
      "/opt/scorecard/fastmem-selftest; rc=$?; echo SCORECARD_V2_FASTMEM_EXIT=$rc\n",
    ));
    const passed = await waitUntil(
      () => console.text.includes("FASTMEM_PASS") &&
        console.text.includes("SCORECARD_V2_FASTMEM_EXIT=0"),
      () => vm.runVirtSystem(500_000n),
      120_000,
    );
    if (!passed) {
      throw new Error(`${side}/${rowKey} fastmem preflight failed: ${console.text.slice(-3000)}`);
    }
  }

  if (row.family === "nbench") {
    const setup = [
      "CUSTOMRUN=T",
      "ALLSTATS=T",
      `${row.nbenchFlag}=T`,
      ...(!nbenchUnscoredDiagnostic
        ? fixedNbenchParameters
        : []),
      ...(nbenchFixedArraysDiagnostic ? ["NUMNUMARRAYS=1"] : []),
    ]
      .map((line, index) => `echo ${line} ${index ? ">>" : ">"} /tmp/C`)
      .join("; ");
    vm.virtConsoleInput(encoder.encode(`${setup}; echo SCORECARD_V2_NBENCH_READY\n`));
    const nbenchReady = await waitUntil(
      () => console.text.includes("SCORECARD_V2_NBENCH_READY"),
      () => vm.runVirtSystem(500_000n),
      30_000,
    );
    if (!nbenchReady) throw new Error(`${side}/${rowKey} nbench setup failed`);
  }
  return {
    kernelSha256: sha256(inputs.kernel),
    initramfsSha256: sha256(inputs.initrd),
    guest,
    workloadSha256: embeddedWorkloadSha256(row, inputs.initrd, {
      nbenchExecutable: nbenchNativeDiagnostic
        ? NBENCH_WORKLOAD_CONTRACT.nativeDiagnosticExecutable
        : nbenchSelfTimedDiagnostic
          ? NBENCH_WORKLOAD_CONTRACT.selfTimedDiagnosticExecutable
          : NBENCH_WORKLOAD_CONTRACT.executable,
    }),
  };
}

async function runRvBoot() {
  const loaded = await loadRv(side);
  const coldStarted = performance.now();
  const vm = await loaded.RV64Debug.create(loaded.wasm);
  const capturedJitModules = installJitModuleCapture(vm);
  if (jitModuleCaptureDir) vm.scorecardPhase = "boot";
  const requestedPolicy = configureRvExecution(vm);
  applyPagePolicyDiagnostics(vm);
  applyInterpreterFusedMemoryDiagnostic(vm);
  applyIntegratedScalarT0Diagnostic(vm);
  if (pageTemplateProbeDiagnostic) {
    if (side !== "rewrite") throw new Error("page-template probe requires rewrite");
    vm.ex.jit_set_page_template_probe(1);
    vm.scorecardPhase = "boot";
  }
  if (pageTemplateReuseDiagnostic) {
    if (side !== "rewrite") throw new Error("page-template reuse requires rewrite");
    vm.ex.jit_set_page_template_reuse(1);
  }
  if (profileDiagnostic) {
    vm.ex.dprof_set_sample_shift(profileSampleShift);
    vm.ex.dprof_set(1);
  }
  const console = makeConsole();
  vm.onWrite = (_fd, bytes) => console.append(bytes);
  const { kernel, initrd } = await loadModernRvInputs();
  const counter = rvInstructionCounter(vm, "virt");
  vm.bootVirtLinuxDirect({
    kernel: new Uint8Array(kernel),
    initrd: new Uint8Array(initrd),
    ramMB: 512,
    cmdline: "console=ttyS0 rdinit=/init",
  });
  const staticT0Lifecycle = prepareStaticSystemT0Diagnostic(vm);
  const before = readRvCounters(vm, counter);
  await startEngineProfile("first");
  const started = performance.now();
  const ready = await waitUntil(
    () => console.text.includes("SCORECARD_V2_READY"),
    () => vm.runVirtSystem(2_000_000n),
    180_000,
  );
  if (!ready) throw new Error(`${side}/boot timeout: ${console.text.slice(-3000)}`);
  const value = performance.now() - started;
  const coldReadyMs = performance.now() - coldStarted;
  const engineProfile = await stopEngineProfile("first");
  const after = readRvCounters(vm, counter);
  const readySettle = await settleRv(vm, () => vm.runVirtSystem(250_000n));
  if (!readySettle.complete) throw new Error(`${side}/boot post-ready JIT did not settle`);
  const guest = parseGuestIdentity(console.text);
  const guestProblems = validateGuestIdentity(guest, side);
  if (guestProblems.length) throw new Error(`${side}/boot guest contract failed: ${guestProblems.join("; ")}`);
  const policy = pagePolicySnapshot(vm);
  const jitModuleCapture = await writeCapturedJitModules(
    jitModuleCaptureDir,
    capturedJitModules,
  );
  const counters = deltaRvCounters(before, after);
  const interpreterActivity = disableRvJit
    ? proveRvInterpreterBypass(vm, `${side}/boot`)
    : null;
  const staticT0Proof = readStaticT0RuntimeProof(vm, staticT0Lifecycle);
  if (!disableRvJit &&
      (BigInt(counters.generatedInstructions) === 0n || BigInt(counters.dispatches) === 0n)) {
    throw new Error(
      `${side}/boot JIT proof failed: generated=${counters.generatedInstructions} ` +
      `dispatches=${counters.dispatches}`,
    );
  }
  if (staticT0Lifecycle?.enabled &&
      (BigInt(counters.staticT0FastInstructions) === 0n ||
       BigInt(counters.staticT0Errors) !== 0n)) {
    throw new Error(
      `${side}/boot static T0 proof failed: fast=${counters.staticT0FastInstructions} ` +
      `errors=${counters.staticT0Errors}`,
    );
  }
  if (staticT0Lifecycle?.sampled &&
      BigInt(counters.staticT0SampledInstructions) === 0n) {
    throw new Error(`${side}/boot sampled static T0 proof failed: no sampled retirement`);
  }
  if (staticT0Lifecycle?.sampledBackoff &&
      (BigInt(staticT0Proof.shortMarks) === 0n ||
       BigInt(staticT0Proof.shortBypasses) === 0n)) {
    throw new Error(
      `${side}/boot sampled static T0 backoff proof failed: ` +
      `marks=${staticT0Proof.shortMarks} bypasses=${staticT0Proof.shortBypasses}`,
    );
  }
  return {
    schema: SCHEMA,
    measurementEligible:
      !disableRvJitDiagnostic && !pagePolicyDiagnostic &&
      interpreterFusedMemoryDiagnostic === null &&
      integratedScalarT0Diagnostic === null &&
      staticSystemT0Diagnostic === null && sampledStaticT0Diagnostic === null &&
      sampledStaticT0BackoffDiagnostic === null &&
      !profileDiagnostic &&
      !engineProfileDir &&
      !rewriteWasmOverride &&
      !jitModuleCaptureDir &&
      !pageTemplateProbeDiagnostic &&
      !pageTemplateReuseDiagnostic &&
      !pumpCadence.diagnostic,
    side,
    row: rowKey,
    kind: row.kind,
    phases: {
      first: {
        value,
        coldReadyMs,
        unit: "ms",
        counters,
        ...(engineProfile ? { engineProfile } : {}),
        ...(profileDiagnostic ? { profile: readRvProfile(vm) } : {}),
      },
    },
    settle: [{ after: "ready", ...readySettle }],
    runtime: {
      engine: process.version,
      executionMode,
      schedulerCadence: cadenceRecord(pumpCadence),
      identity: loaded.identity,
      guest,
      requestedPolicy,
      effectivePolicy: policy,
      ...(staticT0Lifecycle ? { staticSystemT0: staticT0Lifecycle } : {}),
      ...(staticT0Proof ? { staticT0Proof } : {}),
      ...(pageTemplateProbeDiagnostic
        ? { pageTemplateAsyncModules: vm.jitAsyncModuleDiagnostics ?? [] }
        : {}),
      policyProblems: !disableRvJit && side === "rewrite" &&
        requestedPolicy.name === "production-page"
        ? validateProductionPolicy(policy, requestedPolicy)
        : [],
      jitProof: {
        enabledRequested: !disableRvJit,
        generatedInstructions: counters.generatedInstructions,
        dispatches: counters.dispatches,
        requirement: disableRvJit
          ? interpreterOnly
            ? "interpreter-only-no-jit-activity"
            : "diagnostic-interpreter-bypass"
          : "generated-code-executed",
        ...(interpreterActivity
          ? { inactiveProof: true, activity: interpreterActivity }
          : {}),
      },
      diagnostic:
        disableRvJitDiagnostic || pagePolicyDiagnostic ||
        interpreterFusedMemoryDiagnostic !== null ||
        integratedScalarT0Diagnostic !== null ||
        staticSystemT0Diagnostic !== null || sampledStaticT0Diagnostic !== null ||
        sampledStaticT0BackoffDiagnostic !== null ||
        profileDiagnostic ||
        engineProfileDir ||
        rewriteWasmOverride ||
        jitModuleCaptureDir ||
        pageTemplateProbeDiagnostic || pageTemplateReuseDiagnostic ||
        pumpCadence.diagnostic
        ? {
            ...(disableRvJitDiagnostic ? { rvJitDisabled: true } : {}),
            ...(interpreterFusedMemoryDiagnostic !== null
              ? { interpreterFusedMemory: interpreterFusedMemoryDiagnostic }
              : {}),
            ...(integratedScalarT0Diagnostic !== null
              ? { integratedScalarT0: integratedScalarT0Diagnostic }
              : {}),
            ...(staticSystemT0Diagnostic !== null
              ? { staticSystemT0: staticSystemT0Diagnostic }
              : {}),
            ...(sampledStaticT0Diagnostic !== null
              ? { sampledStaticT0: sampledStaticT0Diagnostic }
              : {}),
            ...(sampledStaticT0BackoffDiagnostic !== null
              ? { sampledStaticT0Backoff: sampledStaticT0BackoffDiagnostic }
              : {}),
            ...(pageThresholdDiagnostic !== null
              ? { pageThreshold: pageThresholdDiagnostic }
              : {}),
            ...(privilegedThresholdMultiplierDiagnostic !== null
              ? { privilegedThresholdMultiplier: privilegedThresholdMultiplierDiagnostic }
              : {}),
            ...(pageCapDiagnostic !== null ? { regionPageCap: pageCapDiagnostic } : {}),
            ...(leaderCapDiagnostic !== null ? { regionLeaderCap: leaderCapDiagnostic } : {}),
            ...(inflightDiagnostic !== null ? { pageInflight: inflightDiagnostic } : {}),
            ...(controlEntriesDiagnostic !== null
              ? { controlEntries: controlEntriesDiagnostic }
              : {}),
            ...(privilegedControlEntriesDiagnostic !== null
              ? { privilegedControlEntries: privilegedControlEntriesDiagnostic }
              : {}),
            ...(stablePageChainDiagnostic !== null
              ? { stablePageChain: stablePageChainDiagnostic }
              : {}),
            ...(profileDiagnostic ? { executionProfile: true, profileSampleShift } : {}),
            ...(engineProfileDir
              ? { engineCpuProfile: true, engineProfileInterval }
              : {}),
            ...(rewriteWasmOverride ? { rewriteWasmOverride } : {}),
            ...(jitModuleCapture ? { jitModuleCapture } : {}),
            ...(pageTemplateProbeDiagnostic ? { pageTemplateProbe: true } : {}),
            ...(pageTemplateReuseDiagnostic ? { pageTemplateReuse: true } : {}),
            ...pumpCadenceDiagnostic,
          }
        : null,
    },
    inputSha256: {
      kernel: sha256(kernel),
      initramfs: sha256(initrd),
    },
  };
}

async function runRvWorkload() {
  const loaded = await loadRv(side);
  const vm = await loaded.RV64Debug.create(loaded.wasm);
  const capturedJitModules = installJitModuleCapture(vm);
  engineTierMarker("runtime-created");
  const requestedPolicy = configureRvExecution(vm);
  applyPagePolicyDiagnostics(vm);
  applyInterpreterFusedMemoryDiagnostic(vm);
  applyIntegratedScalarT0Diagnostic(vm);
  if (userJitOnlyDiagnostic) vm.ex.jit_set_supervisor_enabled(0);
  if (blockJitDiagnostic) {
    vm.ex.jit_set_page_policy(0);
    vm.ex.sys_set_superblock?.(0);
    vm.ex.jit_set_multi_latch?.(0);
    vm.ex.jit_set_region_chain?.(0);
    vm.ex.jit_set_region_tail_chain?.(0);
  }
  if (regionTlbCacheDiagnostic) {
    vm.ex.jit_set_region_tlb_cache(1);
    vm.ex.jit_set_region_tlb_cache_min_accesses(regionTlbCacheMinAccesses);
  }
  if (pageTemplateProbeDiagnostic) {
    if (side !== "rewrite") throw new Error("page-template probe requires rewrite");
    vm.ex.jit_set_page_template_probe(1);
    vm.scorecardPhase = "boot";
  }
  if (pageTemplateReuseDiagnostic) {
    if (side !== "rewrite") throw new Error("page-template reuse requires rewrite");
    vm.ex.jit_set_page_template_reuse(1);
  }
  // Structured profiling is encoded into generated functions. Enable it
  // before boot so modules compiled while reaching the shell carry the
  // diagnostic counters; each measured phase resets the same stable cells.
  if (profileDiagnostic) {
    vm.ex.dprof_set_sample_shift(profileSampleShift);
    vm.ex.dprof_set(1);
  }
  const console = makeConsole();
  if (jitModuleCaptureDir) vm.scorecardPhase = "boot";
  engineTierMarker("boot-start");
  const input = await prepareRvWorkload(vm, console);
  engineTierMarker("boot-ready");
  const pump = () => vm.runVirtSystem(250_000n);
  const bootSettle = await settleRv(vm, pump);
  if (!bootSettle.complete) throw new Error(`${side}/${rowKey} boot JIT did not settle`);
  engineTierMarker("boot-settled");
  const counter = rvInstructionCounter(vm, "virt");
  const phases = {};
  const settles = [{ after: "boot", ...bootSettle }];
  for (const phase of clockDiagnostic ? ["steady"] : phasesFor(row)) {
    if (pageTemplateProbeDiagnostic || jitModuleCaptureDir) vm.scorecardPhase = phase;
    if (profileDiagnostic) {
      vm.ex.dprof_set_sample_shift(profileSampleShift);
      vm.ex.dprof_set(1);
    }
    const offset = console.text.length;
    const [startMarker, endMarker] = rvPhaseMarkers(row.family, phase);
    const startWatch = startMarker ? console.watch(startMarker, offset) : null;
    const endWatch = endMarker ? console.watch(endMarker, offset) : null;
    const completeMarker = execPid1Diagnostic
      ? "CLOCK_DIAGNOSTIC process="
      : `SCORECARD_V2_${phase.toUpperCase()}_COMPLETE`;
    const completeWatch = console.watch(completeMarker, offset);
    const before = readRvCounters(vm, counter);
    const progressStart = counter();
    const timerSbiStart = vm.ex.virt_sbi_call_count?.(2) ?? 0n;
    const timerUserStart = vm.ex.jit_stat(96);
    const timerSupervisorStart = vm.ex.jit_stat(97);
    let pumpCalls = 0;
    engineTierMarker(`${phase}-start`);
    await startEngineProfile(phase);
    const hostStarted = performance.now();
    vm.virtConsoleInput(encoder.encode(rvPhaseCommand(row.family, phase)));
    const complete = await waitUntil(
      () => completeWatch.time !== null,
      () => {
        pumpCalls++;
        vm.runVirtSystem(2_000_000n);
      },
      timeoutMs,
      () => {
        const current = counter();
        process.stderr.write(
          `[scorecard-v2 progress] side=${side} row=${rowKey} phase=${phase} ` +
          `host_ms=${Math.round(performance.now() - hostStarted)} ` +
          `pc=0x${vm.virtPc().toString(16)} ` +
          `instructions=${current - progressStart} ` +
          `pumps=${pumpCalls} ` +
          `generated=${vm.ex.jit_stat(0) - before.stats[0]} ` +
          `dispatches=${vm.ex.jit_stat(1) - before.stats[1]} ` +
          `timer_sbi=${(vm.ex.virt_sbi_call_count?.(2) ?? 0n) - timerSbiStart} ` +
          `timer_from_user=${vm.ex.jit_stat(96) - timerUserStart} ` +
          `timer_from_supervisor=${vm.ex.jit_stat(97) - timerSupervisorStart}\n`,
        );
      },
    );
    if (!complete) throw new Error(`${side}/${rowKey}/${phase} timed out: ${console.text.slice(-3000)}`);
    engineTierMarker(`${phase}-guest-complete`);
    const hostMs = performance.now() - hostStarted;
    const engineProfile = await stopEngineProfile(phase);
    const after = readRvCounters(vm, counter);
    const segment = console.segment(offset);
    try {
      phases[phase] = phaseResult(
        row,
        phase,
        segment,
        startWatch,
        endWatch,
        deltaRvCounters(before, after),
        hostMs,
      );
    } catch (error) {
      const failureCapture = await writeCapturedJitModules(
        jitModuleCaptureDir,
        capturedJitModules,
      );
      if (failureCapture) {
        error.message += `; failing JIT modules captured in ${failureCapture.manifestPath}`;
      }
      throw error;
    }
    if (engineProfile) phases[phase].engineProfile = engineProfile;
    if (profileDiagnostic) phases[phase].profile = readRvProfile(vm);
    if (execPid1Diagnostic) {
      settles.push({
        after: phase,
        complete: false,
        skipped: true,
        reason: "PID 1 was replaced for the task-accounting diagnostic",
      });
    } else {
      const settled = await settleRv(vm, pump);
      settles.push({ after: phase, ...settled });
      if (!settled.complete) throw new Error(`${side}/${rowKey}/${phase} JIT did not settle`);
      engineTierMarker(`${phase}-settled`);
    }
  }
  const effectivePolicy = pagePolicySnapshot(vm);
  const jitModuleCapture = await writeCapturedJitModules(
    jitModuleCaptureDir,
    capturedJitModules,
  );
  const pageTemplateRelocationPairs = pageTemplateProbeDiagnostic
    ? Array.from(
        { length: vm.ex.jit_page_template_pair_count() },
        (_, index) => ({
          currentVirtualPage: `0x${vm.ex.jit_page_template_pair(index, 0).toString(16)}`,
          templateVirtualPage: `0x${vm.ex.jit_page_template_pair(index, 1).toString(16)}`,
          currentPhysicalPage: `0x${vm.ex.jit_page_template_pair(index, 2).toString(16)}`,
          templatePhysicalPage: `0x${vm.ex.jit_page_template_pair(index, 3).toString(16)}`,
          requestedEntries: vm.ex.jit_page_template_pair(index, 4).toString(),
          coveredEntries: vm.ex.jit_page_template_pair(index, 5).toString(),
        }),
      )
    : null;
  const lastTimerTrap = clockDiagnostic
    ? Array.from({ length: 13 }, (_, index) => vm.ex.virt_last_timer_trap(index).toString())
    : null;
  if (lastTimerTrap) {
    process.stderr.write(`[scorecard-v2 timer-trap] ${JSON.stringify(lastTimerTrap)}\n`);
  }
  const generatedInstructions = Object.values(phases)
    .reduce((total, phase) => total + BigInt(phase.counters.generatedInstructions), 0n);
  const dispatches = Object.values(phases)
    .reduce((total, phase) => total + BigInt(phase.counters.dispatches), 0n);
  const staticT0Fast = Object.values(phases)
    .reduce((total, phase) => total + BigInt(phase.counters.staticT0FastInstructions), 0n);
  const staticT0Errors = Object.values(phases)
    .reduce((total, phase) => total + BigInt(phase.counters.staticT0Errors), 0n);
  const sampledStaticT0Instructions = Object.values(phases)
    .reduce(
      (total, phase) => total + BigInt(phase.counters.staticT0SampledInstructions),
      0n,
    );
  const staticT0Proof = readStaticT0RuntimeProof(vm, vm.scorecardStaticT0);
  const interpreterActivity = disableRvJit
    ? proveRvInterpreterBypass(vm, `${side}/${rowKey}`)
    : null;
  if (!disableRvJit && (generatedInstructions === 0n || dispatches === 0n)) {
    throw new Error(
      `${side}/${rowKey} JIT proof failed: generated=${generatedInstructions} dispatches=${dispatches}`,
    );
  }
  if (vm.scorecardStaticT0?.enabled && (staticT0Fast === 0n || staticT0Errors !== 0n)) {
    throw new Error(
      `${side}/${rowKey} static T0 proof failed: fast=${staticT0Fast} errors=${staticT0Errors}`,
    );
  }
  if (vm.scorecardStaticT0?.sampled && sampledStaticT0Instructions === 0n) {
    throw new Error(`${side}/${rowKey} sampled static T0 proof failed: no sampled retirement`);
  }
  if (vm.scorecardStaticT0?.sampledBackoff &&
      (BigInt(staticT0Proof.shortMarks) === 0n ||
       BigInt(staticT0Proof.shortBypasses) === 0n)) {
    throw new Error(
      `${side}/${rowKey} sampled static T0 backoff proof failed: ` +
      `marks=${staticT0Proof.shortMarks} bypasses=${staticT0Proof.shortBypasses}`,
    );
  }
  return {
    schema: SCHEMA,
    measurementEligible:
      !nbenchFixedArraysDiagnostic && !nbenchUnscoredDiagnostic &&
      !disableRvJitDiagnostic && !pagePolicyDiagnostic &&
      interpreterFusedMemoryDiagnostic === null &&
      integratedScalarT0Diagnostic === null &&
      staticSystemT0Diagnostic === null && sampledStaticT0Diagnostic === null &&
      sampledStaticT0BackoffDiagnostic === null &&
      !clockDiagnostic &&
      !execPid1Diagnostic && !userJitOnlyDiagnostic && !blockJitDiagnostic &&
      !profileDiagnostic && !engineProfileDir && !rewriteWasmOverride &&
      !engineTierMarkersDiagnostic &&
      !engineVariantDiagnostic && !jitModuleCaptureDir &&
      !regionTlbCacheDiagnostic &&
      !pageTemplateProbeDiagnostic && !pageTemplateReuseDiagnostic &&
      !fastmemPreflightDiagnostic &&
      !pumpCadence.diagnostic &&
      stringArraysDiagnostic === null,
    side,
    row: rowKey,
    kind: row.kind,
    phases,
    settle: settles,
    runtime: {
      engine: process.version,
      executionMode,
      schedulerCadence: cadenceRecord(pumpCadence),
      identity: loaded.identity,
      guest: input.guest,
      requestedPolicy,
      effectivePolicy,
      ...(vm.scorecardStaticT0 ? { staticSystemT0: vm.scorecardStaticT0 } : {}),
      ...(staticT0Proof ? { staticT0Proof } : {}),
      ...(pageTemplateRelocationPairs ? { pageTemplateRelocationPairs } : {}),
      ...(pageTemplateProbeDiagnostic
        ? { pageTemplateAsyncModules: vm.jitAsyncModuleDiagnostics ?? [] }
        : {}),
      ...(row.family === "nbench"
        ? {
            workload: nbenchNativeDiagnostic
              ? { variant: "native-long", crossIsaComparable: false }
              : nbenchSelfTimedDiagnostic
                ? { variant: "self-timed", crossIsaComparable: false }
                : NBENCH_WORKLOAD_CONTRACT,
          }
        : {}),
      policyProblems: !disableRvJit && side === "rewrite" &&
        requestedPolicy.name === "production-page"
        ? validateProductionPolicy(effectivePolicy, requestedPolicy)
        : [],
      jitProof: {
        enabledRequested: !disableRvJit,
        generatedInstructions: generatedInstructions.toString(),
        dispatches: dispatches.toString(),
        requirement: disableRvJit
          ? interpreterOnly
            ? "interpreter-only-no-jit-activity"
            : "diagnostic-interpreter-bypass"
          : "generated-code-executed",
        ...(interpreterActivity
          ? { inactiveProof: true, activity: interpreterActivity }
          : {}),
      },
      diagnostic:
        nbenchFixedArraysDiagnostic || nbenchUnscoredDiagnostic ||
        disableRvJitDiagnostic || pagePolicyDiagnostic ||
        interpreterFusedMemoryDiagnostic !== null ||
        integratedScalarT0Diagnostic !== null ||
        staticSystemT0Diagnostic !== null || sampledStaticT0Diagnostic !== null ||
        sampledStaticT0BackoffDiagnostic !== null ||
        clockDiagnostic ||
        execPid1Diagnostic || userJitOnlyDiagnostic || blockJitDiagnostic
        || profileDiagnostic || engineProfileDir || rewriteWasmOverride ||
        engineTierMarkersDiagnostic ||
        engineVariantDiagnostic || jitModuleCaptureDir ||
        regionTlbCacheDiagnostic ||
        pageTemplateProbeDiagnostic || pageTemplateReuseDiagnostic ||
        fastmemPreflightDiagnostic ||
        pumpCadence.diagnostic ||
        stringArraysDiagnostic !== null
        ? {
            ...(interpreterFusedMemoryDiagnostic !== null
              ? { interpreterFusedMemory: interpreterFusedMemoryDiagnostic }
              : {}),
            ...(integratedScalarT0Diagnostic !== null
              ? { integratedScalarT0: integratedScalarT0Diagnostic }
              : {}),
            ...(staticSystemT0Diagnostic !== null
              ? { staticSystemT0: staticSystemT0Diagnostic }
              : {}),
            ...(sampledStaticT0Diagnostic !== null
              ? { sampledStaticT0: sampledStaticT0Diagnostic }
              : {}),
            ...(sampledStaticT0BackoffDiagnostic !== null
              ? { sampledStaticT0Backoff: sampledStaticT0BackoffDiagnostic }
              : {}),
            ...(nbenchFixedArraysDiagnostic
              ? { nbenchFixedArrays: 1, reason: "calibration-clock isolation" }
              : {}),
            ...(nbenchNativeDiagnostic
              ? { nbenchVariant: "native-long", reason: "cross-ISA-width-mismatch" }
              : {}),
            ...(nbenchSelfTimedDiagnostic
              ? { nbenchVariant: "self-timed", reason: "guest-clock-selects-work" }
              : {}),
            ...(disableRvJitDiagnostic ? { rvJitDisabled: true } : {}),
            ...(pageThresholdDiagnostic !== null
              ? { pageThreshold: pageThresholdDiagnostic }
              : {}),
            ...(privilegedThresholdMultiplierDiagnostic !== null
              ? { privilegedThresholdMultiplier: privilegedThresholdMultiplierDiagnostic }
              : {}),
            ...(pageCapDiagnostic !== null ? { regionPageCap: pageCapDiagnostic } : {}),
            ...(leaderCapDiagnostic !== null ? { regionLeaderCap: leaderCapDiagnostic } : {}),
            ...(inflightDiagnostic !== null ? { pageInflight: inflightDiagnostic } : {}),
            ...(controlEntriesDiagnostic !== null
              ? { controlEntries: controlEntriesDiagnostic }
              : {}),
            ...(privilegedControlEntriesDiagnostic !== null
              ? { privilegedControlEntries: privilegedControlEntriesDiagnostic }
              : {}),
            ...(stablePageChainDiagnostic !== null
              ? { stablePageChain: stablePageChainDiagnostic }
              : {}),
            ...(clockDiagnostic ? { guestClockProbe: true } : {}),
            ...(lastTimerTrap ? { lastTimerTrap } : {}),
            ...(execPid1Diagnostic ? { benchmarkExecsAsPid1: true } : {}),
            ...(userJitOnlyDiagnostic ? { generatedSupervisorCodeDisabled: true } : {}),
            ...(blockJitDiagnostic ? { generatedRegionsDisabled: true } : {}),
            ...(profileDiagnostic ? { executionProfile: true, profileSampleShift } : {}),
            ...(engineProfileDir
              ? { engineCpuProfile: true, engineProfileInterval }
              : {}),
            ...(engineTierMarkersDiagnostic ? { engineTierMarkers: true } : {}),
            ...(engineVariantDiagnostic ? { engineVariant: engineVariantDiagnostic } : {}),
            ...(jitModuleCapture ? { jitModuleCapture } : {}),
            ...(rewriteWasmOverride ? { rewriteWasmOverride } : {}),
            ...(regionTlbCacheDiagnostic
              ? { regionTlbCache: true, regionTlbCacheMinAccesses }
              : {}),
            ...(pageTemplateProbeDiagnostic ? { pageTemplateProbe: true } : {}),
            ...(pageTemplateReuseDiagnostic ? { pageTemplateReuse: true } : {}),
            ...(fastmemPreflightDiagnostic ? { fastmemPreflight: true } : {}),
            ...pumpCadenceDiagnostic,
            ...(stringArraysDiagnostic !== null
              ? { stringArrays: stringArraysDiagnostic, reason: "scaling-guard" }
              : {}),
          }
        : null,
    },
    inputSha256: {
      kernel: input.kernelSha256,
      initramfs: input.initramfsSha256,
      ...input.workloadSha256,
    },
  };
}

function installInstantiateTracker() {
  const original = WebAssembly.instantiate;
  let pending = 0;
  let calls = 0;
  WebAssembly.instantiate = function (...args) {
    calls++;
    let result;
    try {
      result = Reflect.apply(original, this, args);
    } catch (error) {
      throw error;
    }
    if (!result || typeof result.then !== "function") return result;
    pending++;
    return result.finally(() => { pending--; });
  };
  return {
    get pending() { return pending; },
    get calls() { return calls; },
    restore() { WebAssembly.instantiate = original; },
  };
}

async function settleV86(tracker, timeout = 30_000) {
  const started = performance.now();
  const beforeCalls = tracker.calls;
  const deadline = started + timeout;
  let stable = 0;
  while (performance.now() < deadline) {
    if (tracker.pending === 0) {
      if (++stable >= 3) {
        return {
          complete: true,
          ms: performance.now() - started,
          instantiateCalls: tracker.calls - beforeCalls,
          pending: tracker.pending,
        };
      }
    } else {
      stable = 0;
    }
    await tick();
  }
  return {
    complete: false,
    ms: performance.now() - started,
    instantiateCalls: tracker.calls - beforeCalls,
    pending: tracker.pending,
  };
}

async function createV86(options, console, tracker) {
  process.chdir(v86dir);
  const { V86 } = await import(pathToFileURL(join(v86dir, "src/main.js")).href);
  const emulator = new V86({
    ...options,
    autostart: false,
    disable_jit: interpreterOnly,
    log_level: 0,
  });
  emulator.add_listener("serial0-output-byte", (byte) => {
    const character = String.fromCharCode(byte);
    if ((character >= " " && character <= "~") || character === "\n") {
      console.append(Uint8Array.of(byte));
    }
  });
  await new Promise((done) => emulator.add_listener("emulator-ready", done));
  const exports = emulator.v86.cpu.wm.exports;
  if (typeof exports.get_jit_config !== "function" || typeof exports.set_jit_config !== "function") {
    throw new Error("pinned v86 build does not expose JIT configuration");
  }
  const expectedDisabled = interpreterOnly ? 1 : 0;
  exports.set_jit_config(0, expectedDisabled);
  if (exports.get_jit_config(0) !== expectedDisabled) {
    throw new Error(
      `v86 JIT configuration mismatch: disabled=${exports.get_jit_config(0)} ` +
      `expected=${expectedDisabled}`,
    );
  }
  return emulator;
}

function observeV86Jit(emulator, tracker) {
  const cpu = emulator.v86.cpu;
  const exports = cpu.wm.exports;
  const startedCalls = tracker.calls;
  let finalizedModules = 0;
  let finalizedBytes = 0;
  let maxCacheSize = exports.jit_get_cache_size?.() ?? null;
  cpu.test_hook_did_finalize_wasm = (bytes) => {
    finalizedModules++;
    finalizedBytes += bytes.byteLength;
    const cacheSize = exports.jit_get_cache_size?.() ?? null;
    if (cacheSize !== null) maxCacheSize = Math.max(maxCacheSize ?? 0, cacheSize);
  };
  return {
    snapshot() {
      return {
        disabled: exports.get_jit_config(0),
        instantiateCalls: tracker.calls - startedCalls,
        finalizedModules,
        finalizedBytes,
        cacheSize: exports.jit_get_cache_size?.() ?? null,
        maxCacheSize,
      };
    },
  };
}

function validateV86JitProof(proof, label, requireWorkloadModules = false) {
  if (interpreterOnly) {
    const activity = {
      finalizedModules: proof.finalizedModules,
      finalizedBytes: proof.finalizedBytes,
      cacheSize: proof.cacheSize,
      maxCacheSize: proof.maxCacheSize,
      ...(requireWorkloadModules
        ? {
            workloadFinalizedModules: proof.workloadFinalizedModules,
            workloadFinalizedBytes: proof.workloadFinalizedBytes,
          }
        : {}),
    };
    const nonzero = Object.entries(activity).filter(([, count]) => count !== 0);
    if (proof.disabled !== 1 || nonzero.length) {
      throw new Error(
        `${label} interpreter-only proof failed: disabled=${proof.disabled} ` +
        nonzero.map(([name, count]) => `${name}=${count}`).join(" "),
      );
    }
    proof.inactiveProof = true;
    proof.activity = activity;
    return proof;
  }
  if (
    proof.disabled !== 0 ||
    !(requireWorkloadModules ? proof.workloadFinalizedModules > 0 : proof.finalizedModules > 0)
  ) {
    throw new Error(`${label} JIT proof failed: ${JSON.stringify(proof)}`);
  }
  return proof;
}

const V86_EXECUTION_WRAPPER = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
  0x01, 0x08, 0x02, 0x60, 0x01, 0x7f, 0x00, 0x60, 0x00, 0x00,
  0x02, 0x13, 0x02,
  0x01, 0x65, 0x05, 0x69, 0x6e, 0x6e, 0x65, 0x72, 0x00, 0x00,
  0x01, 0x65, 0x03, 0x68, 0x69, 0x74, 0x00, 0x01,
  0x03, 0x02, 0x01, 0x00,
  0x07, 0x05, 0x01, 0x01, 0x66, 0x00, 0x02,
  0x0a, 0x0a, 0x01, 0x08, 0x00, 0x10, 0x01, 0x20, 0x00, 0x10, 0x00, 0x0b,
]);

function makeV86ExecutionProbe(emulator) {
  const table = emulator.v86.cpu.wm.wasm_table;
  const wrapperModule = new WebAssembly.Module(V86_EXECUTION_WRAPPER);
  const nativeSet = table.set.bind(table);
  const hadOwnSet = Object.hasOwn(table, "set");
  const priorOwnSet = hadOwnSet ? table.set : null;
  let active = false;
  let installed = 0;
  let hits = 0;
  const hitIndexes = new Set();

  return {
    start() {
      if (active) return;
      active = true;
      table.set = (index, value) => {
        if (value === null || typeof value !== "function") {
          return nativeSet(index, value);
        }
        installed++;
        let armed = true;
        const wrapper = new WebAssembly.Instance(wrapperModule, {
          e: {
            inner: value,
            hit: () => {
              if (armed) {
                armed = false;
                hits++;
                hitIndexes.add(index);
                // Restore the uninstrumented generated function before it is
                // dispatched again. This run is proof-only regardless, but a
                // single boundary crossing avoids changing tier behaviour.
                nativeSet(index, value);
              }
            },
          },
        }).exports.f;
        return nativeSet(index, wrapper);
      };
    },
    snapshot() {
      return { active, installed, hits, distinctHitIndexes: hitIndexes.size };
    },
    restore() {
      if (!active) return;
      if (hadOwnSet) table.set = priorOwnSet;
      else delete table.set;
      active = false;
    },
  };
}

async function loadV86Firmware() {
  const [bios, vga] = await Promise.all([
    readFile(join(v86dir, "bios/seabios.bin")),
    readFile(join(v86dir, "bios/vgabios.bin")),
  ]);
  return { bios, vga };
}

async function v86SourceTreeSha256() {
  const sourceRoot = join(v86dir, "src");
  const files = [];
  async function walk(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await walk(path);
      else if (entry.isFile() && path.endsWith(".js")) files.push(path);
    }
  }
  await walk(sourceRoot);
  files.sort();
  const chunks = [];
  for (const path of files) {
    chunks.push(Buffer.from(relative(sourceRoot, path)), Buffer.from([0]), await readFile(path), Buffer.from([0]));
  }
  return sha256(Buffer.concat(chunks));
}

async function loadV86RuntimeIdentity() {
  const [wasm, sourceTreeSha256] = await Promise.all([
    readFile(join(v86dir, "build/v86.wasm")),
    v86SourceTreeSha256(),
  ]);
  const identity = {
    sourceCommit: V86_RUNTIME.sourceCommit,
    sourceTreeSha256,
    wasmSha256: sha256(wasm),
  };
  if (
    identity.sourceTreeSha256 !== V86_RUNTIME.sourceTreeSha256 ||
    identity.wasmSha256 !== V86_RUNTIME.wasmSha256
  ) {
    throw new Error(
      `v86 runtime identity mismatch: source=${identity.sourceTreeSha256} ` +
      `wasm=${identity.wasmSha256}`,
    );
  }
  return identity;
}

function v86PhaseResult(rowSpec, phase, segment, startWatch, endWatch, calls, hostMs) {
  if (rowSpec.family === "nbench") {
    if (nbenchUnscoredDiagnostic) {
      const parsed = parseNbench(segment, rowSpec.nbenchName);
      if (!(parsed.value > 0)) {
        throw new Error(
          `v86/${rowKey}/${phase} nbench value missing: ` +
          JSON.stringify(segment.slice(-4000)),
        );
      }
      return { value: parsed.value, unit: "iterations_per_second", ...parsed, host: { instantiateCalls: calls }, hostMs };
    }
    if (!segment.includes(`SCORECARD_FIXED_WORK fid=${rowSpec.nbenchId}`)) {
      throw new Error(`v86/${rowKey}/${phase} fixed-work execution proof missing`);
    }
    if (startWatch?.time === null || endWatch?.time === null) {
      throw new Error(`v86/${rowKey}/${phase} fixed-work timing markers missing`);
    }
    return {
      value: endWatch.time - startWatch.time,
      unit: "ms",
      fixedWork: {
        fid: rowSpec.nbenchId,
        parameters: fixedNbenchParameters,
      },
      host: { instantiateCalls: calls },
      hostMs,
    };
  }
  if (startWatch?.time === null || endWatch?.time === null) {
    throw new Error(
      `v86/${rowKey}/${phase} timing markers missing: ${JSON.stringify(segment.slice(-1200))}`,
    );
  }
  const result = {
    value: endWatch.time - startWatch.time,
    unit: "ms",
    host: { instantiateCalls: calls },
    hostMs,
  };
  if (rowSpec.family === "compute" || rowSpec.family === "holdout") {
    result.checksum = segment.match(/checksum=(0x[0-9a-f]+)/)?.[1] ?? null;
  } else if (rowSpec.family === "python") {
    result.checksum = segment.match(/fib\(30\)=\s*(\d+)/)?.[1] ?? null;
  } else if (rowSpec.family === "compile") {
    result.md5 = segment.match(/([0-9a-f]{32})\s+\/tmp\/w\.o/)?.[1] ?? null;
  }
  return result;
}

async function runV86Boot() {
  const tracker = installInstantiateTracker();
  try {
    const console = makeConsole();
    const identity = await loadV86RuntimeIdentity();
    const { bios, vga } = await loadV86Firmware();
    const [kernel, initrd] = await Promise.all([
      readFile(join(artifacts, "matched-linux-x86-bzImage")),
      readFile(x86InitramfsPath),
    ]);
    const emulator = await createV86({
      bios: { buffer: exactBuffer(bios) },
      vga_bios: { buffer: exactBuffer(vga) },
      bzimage: { buffer: exactBuffer(kernel) },
      initrd: { buffer: exactBuffer(initrd) },
      cmdline: "console=ttyS0 rdinit=/init mitigations=off tsc=reliable",
      memory_size: 512 * 1024 * 1024,
    }, console, tracker);
    const jit = observeV86Jit(emulator, tracker);
    // Read outside the timed interval. Boot remains scored by wall time; this
    // modulo-2^32 counter only separates emulator throughput from the very
    // different amount of x86/RV64 guest work needed to reach the same marker.
    const instructionsBefore = emulator.get_instruction_counter();
    await startEngineProfile("first");
    const started = performance.now();
    emulator.run();
    const ready = await waitUntil(
      () => console.text.includes("SCORECARD_V2_READY"),
      () => {},
      180_000,
    );
    if (!ready) throw new Error(`v86/boot timeout: ${console.text.slice(-3000)}`);
    const value = performance.now() - started;
    const guestInstructionsModulo2To32 =
      (emulator.get_instruction_counter() - instructionsBefore) >>> 0;
    const engineProfile = await stopEngineProfile("first");
    const readySettle = await settleV86(tracker);
    if (!readySettle.complete) throw new Error("v86/boot post-ready JIT did not settle");
    const guest = parseGuestIdentity(console.text);
    const guestProblems = validateGuestIdentity(guest, side);
    if (guestProblems.length) throw new Error(`v86/boot guest contract failed: ${guestProblems.join("; ")}`);
    const jitProof = validateV86JitProof({
      ...jit.snapshot(),
      enabledRequested: !interpreterOnly,
      requirement: interpreterOnly
        ? "interpreter-only-no-jit-activity"
        : "enabled-and-generated-code-installed; execution-preflight-required",
    }, "v86/boot");
    emulator.destroy();
    return {
      schema: SCHEMA,
      measurementEligible: !engineProfileDir && !pumpCadence.diagnostic,
      side,
      row: rowKey,
      kind: row.kind,
      phases: {
        first: {
          value,
          unit: "ms",
          guestInstructionsModulo2To32,
          host: { instantiateCalls: tracker.calls },
          ...(engineProfile ? { engineProfile } : {}),
        },
      },
      settle: [{ after: "ready", ...readySettle }],
      runtime: {
        engine: process.version,
        executionMode,
        schedulerCadence: cadenceRecord(pumpCadence),
        identity,
        guest,
        requestedPolicy: interpreterOnly
          ? { name: "v86-interpreter-only", disableJit: true }
          : { name: "v86-default-jit", disableJit: false },
        ...(engineProfileDir || pumpCadence.diagnostic
          ? {
              diagnostic: {
                ...(engineProfileDir ? { engineCpuProfile: true, engineProfileInterval } : {}),
                ...pumpCadenceDiagnostic,
              },
            }
          : {}),
        jitProof,
      },
      inputSha256: {
        bios: sha256(bios),
        vgaBios: sha256(vga),
        kernel: sha256(kernel),
        initramfs: sha256(initrd),
      },
    };
  } finally {
    tracker.restore();
  }
}

async function runV86Workload() {
  const tracker = installInstantiateTracker();
  let executionProbe;
  try {
    const console = makeConsole();
    const identity = await loadV86RuntimeIdentity();
    const { bios, vga } = await loadV86Firmware();
    const [kernel, initrd] = await Promise.all([
      readFile(join(artifacts, "matched-linux-x86-bzImage")),
      readFile(x86InitramfsPath),
    ]);
    const emulator = await createV86({
      bios: { buffer: exactBuffer(bios) },
      vga_bios: { buffer: exactBuffer(vga) },
      bzimage: { buffer: exactBuffer(kernel) },
      initrd: { buffer: exactBuffer(initrd) },
      cmdline: "rdinit=/init console=ttyS0 mitigations=off tsc=reliable",
      memory_size: 512 * 1024 * 1024,
    }, console, tracker);
    engineTierMarker("runtime-created");
    const jit = observeV86Jit(emulator, tracker);
    engineTierMarker("boot-start");
    emulator.run();
    const ready = await waitUntil(
      () => console.text.includes("SCORECARD_V2_READY"),
      () => {},
      180_000,
    );
    if (!ready) throw new Error(`v86/${rowKey} modern Alpine boot failed: ${console.text.slice(-3000)}`);
    engineTierMarker("boot-ready");
    const guest = parseGuestIdentity(console.text);
    const guestProblems = validateGuestIdentity(guest, side);
    if (guestProblems.length) throw new Error(`v86/${rowKey} guest contract failed: ${guestProblems.join("; ")}`);
    emulator.serial0_send("stty -echo 2>/dev/null; echo SCORECARD_V2_SHELL_READY\n");
    const shellReady = await waitUntil(
      () => console.text.includes("SCORECARD_V2_SHELL_READY"),
      () => {},
      30_000,
    );
    if (!shellReady) throw new Error(`v86/${rowKey} shell setup failed`);
    if (row.family === "nbench") {
      const setup = [
        "CUSTOMRUN=T",
        "ALLSTATS=T",
        `${row.nbenchFlag}=T`,
        ...(!nbenchUnscoredDiagnostic
          ? fixedNbenchParameters
          : []),
      ]
        .map((line, index) => `echo ${line} ${index ? ">>" : ">"} /tmp/C`)
        .join("; ");
      emulator.serial0_send(`${setup}; echo SCORECARD_V2_NBENCH_READY\n`);
      const nbenchReady = await waitUntil(
        () => console.text.includes("SCORECARD_V2_NBENCH_READY"),
        () => {},
        30_000,
      );
      if (!nbenchReady) throw new Error(`v86/${rowKey} nbench setup failed`);
    }
    const bootSettle = await settleV86(tracker);
    if (!bootSettle.complete) throw new Error(`v86/${rowKey} boot JIT did not settle`);
    engineTierMarker("boot-settled");
    const workloadJitBaseline = jit.snapshot();
    executionProbe = makeV86ExecutionProbe(emulator);
    if (v86ExecutionProofMode) executionProbe.start();
    const phases = {};
    const settles = [{ after: "boot", ...bootSettle }];
  for (const phase of phasesFor(row)) {
      const offset = console.text.length;
      const label = phase.toUpperCase();
      const [startMarker, endMarker] = rvPhaseMarkers(row.family, phase);
      const startWatch = startMarker ? console.watch(startMarker, offset) : null;
      const endWatch = endMarker ? console.watch(endMarker, offset) : null;
      const completeWatch = console.watch(`SCORECARD_V2_${label}_COMPLETE`, offset);
      const callsBefore = tracker.calls;
      const jitBefore = jit.snapshot();
      // v86 exposes a 32-bit architectural instruction counter. The workload
      // read is outside the serial-marker interval and therefore cannot change
      // the scored duration. Record the modulo delta as diagnostic evidence;
      // do not treat it as exact for phases that may retire 2^32 instructions.
      const instructionsBefore = emulator.get_instruction_counter();
      engineTierMarker(`${phase}-start`);
      await startEngineProfile(phase);
      const hostStarted = performance.now();
      emulator.serial0_send(rvPhaseCommand(row.family, phase));
      const complete = await waitUntil(() => completeWatch.time !== null, () => {}, timeoutMs);
      if (!complete) throw new Error(`v86/${rowKey}/${phase} timeout: ${console.text.slice(-3000)}`);
      engineTierMarker(`${phase}-guest-complete`);
      const hostMs = performance.now() - hostStarted;
      const engineProfile = await stopEngineProfile(phase);
      const segment = console.segment(offset);
      phases[phase] = v86PhaseResult(
        row,
        phase,
        segment,
        startWatch,
        endWatch,
        tracker.calls - callsBefore,
        hostMs,
      );
      if (engineProfile) phases[phase].engineProfile = engineProfile;
      phases[phase].guestInstructionsModulo2To32 =
        (emulator.get_instruction_counter() - instructionsBefore) >>> 0;
      const jitAfter = jit.snapshot();
      phases[phase].host.jit = {
        finalizedModules: jitAfter.finalizedModules - jitBefore.finalizedModules,
        finalizedBytes: jitAfter.finalizedBytes - jitBefore.finalizedBytes,
        cacheSizeAfter: jitAfter.cacheSize,
        disabled: jitAfter.disabled,
      };
      if (segment.match(/SCORECARD_V2_EXIT=(\d+)/)?.[1] !== "0") {
        throw new Error(`v86/${rowKey}/${phase} guest failure`);
      }
      if ((row.family === "compute" || row.family === "holdout") && !phases[phase].checksum) {
        throw new Error(`v86/${rowKey}/${phase} checksum missing`);
      }
      if (row.family === "python" && phases[phase].checksum !== "832040") {
        throw new Error(`v86/${rowKey}/${phase} bad fib result`);
      }
      if (row.family === "compile" && !phases[phase].md5) {
        throw new Error(`v86/${rowKey}/${phase} object hash missing`);
      }
      const settled = await settleV86(tracker);
      settles.push({ after: phase, ...settled });
      if (!settled.complete) throw new Error(`v86/${rowKey}/${phase} JIT did not settle`);
      engineTierMarker(`${phase}-settled`);
    }
    const jitProof = {
      ...jit.snapshot(),
      enabledRequested: !interpreterOnly,
      requirement: interpreterOnly
        ? "interpreter-only-no-jit-activity"
        : "enabled-and-generated-code-installed; execution-preflight-required",
    };
    jitProof.workloadFinalizedModules =
      jitProof.finalizedModules - workloadJitBaseline.finalizedModules;
    jitProof.workloadFinalizedBytes =
      jitProof.finalizedBytes - workloadJitBaseline.finalizedBytes;
    jitProof.executionProbe = executionProbe.snapshot();
    validateV86JitProof(jitProof, `v86/${rowKey}`, true);
    if (v86ExecutionProofMode && !(jitProof.executionProbe.hits > 0)) {
      throw new Error(`v86/${rowKey} generated-code dispatch proof failed: ${JSON.stringify(jitProof)}`);
    }
    executionProbe.restore();
    emulator.destroy();
    return {
      schema: SCHEMA,
      measurementEligible:
        !v86ExecutionProofMode && !nbenchUnscoredDiagnostic && !engineProfileDir &&
        !engineTierMarkersDiagnostic && !engineVariantDiagnostic &&
        !pumpCadence.diagnostic &&
        stringArraysDiagnostic === null,
      side,
      row: rowKey,
      kind: row.kind,
      phases,
      settle: settles,
      runtime: {
        engine: process.version,
        executionMode,
        schedulerCadence: cadenceRecord(pumpCadence),
        identity,
        guest,
        requestedPolicy: interpreterOnly
          ? { name: "v86-interpreter-only", disableJit: true }
          : { name: "v86-default-jit", disableJit: false },
        ...(stringArraysDiagnostic !== null || engineProfileDir || engineTierMarkersDiagnostic ||
          engineVariantDiagnostic || pumpCadence.diagnostic
          ? {
              diagnostic: {
                ...(stringArraysDiagnostic !== null
                  ? { stringArrays: stringArraysDiagnostic, reason: "scaling-guard" }
                  : {}),
                ...(engineProfileDir
                  ? { engineCpuProfile: true, engineProfileInterval }
                  : {}),
                ...(engineTierMarkersDiagnostic ? { engineTierMarkers: true } : {}),
                ...(engineVariantDiagnostic ? { engineVariant: engineVariantDiagnostic } : {}),
                ...pumpCadenceDiagnostic,
              },
            }
          : {}),
        ...(row.family === "nbench"
          ? {
              workload: nbenchNativeDiagnostic
                ? { variant: "native-long", crossIsaComparable: false }
                : nbenchSelfTimedDiagnostic
                  ? { variant: "self-timed", crossIsaComparable: false }
                  : NBENCH_WORKLOAD_CONTRACT,
            }
          : {}),
        jitProof,
      },
      inputSha256: {
        bios: sha256(bios),
        vgaBios: sha256(vga),
        kernel: sha256(kernel),
        initramfs: sha256(initrd),
        ...embeddedWorkloadSha256(row, initrd, {
          nbenchExecutable: nbenchNativeDiagnostic
            ? NBENCH_WORKLOAD_CONTRACT.nativeDiagnosticExecutable
            : nbenchSelfTimedDiagnostic
              ? NBENCH_WORKLOAD_CONTRACT.selfTimedDiagnosticExecutable
              : NBENCH_WORKLOAD_CONTRACT.executable,
        }),
      },
    };
  } finally {
    executionProbe?.restore();
    tracker.restore();
  }
}

let result;
if (side === "v86") {
  if (row.family === "boot") result = await runV86Boot();
  else result = await runV86Workload();
} else if (row.family === "boot") {
  result = await runRvBoot();
} else {
  result = await runRvWorkload();
}

result.runtime.inputPopulation = inputPopulation;
console.log(`RESULT_JSON ${JSON.stringify(result)}`);
