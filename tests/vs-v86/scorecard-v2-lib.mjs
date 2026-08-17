import { createHash } from "node:crypto";

export const SCHEMA = 2;
export const PHASES = ["first", "prime", "steady"];
export const EXECUTION_MODES = Object.freeze(["jit", "interpreter"]);
export const INTERPRETER_SIDES = Object.freeze(["rewrite", "v86"]);

export function parseExecutionMode(raw = "jit") {
  if (!EXECUTION_MODES.includes(raw)) {
    throw new Error(
      `SCORECARD_V2_EXECUTION_MODE must be one of ${EXECUTION_MODES.join(", ")}`,
    );
  }
  return raw;
}

export function phasesFor(row) {
  if (row.family === "boot") return ["first"];
  return PHASES;
}
export const MATCH_FLOOR = 0.95;
export const MAX_SAMPLE_SPREAD = 1.25;
export const MAX_HOST_SPREAD = 1.25;

export const ROWS = Object.freeze([
  { key: "alu", name: "ALU", kind: "duration", family: "compute", rvBinary: "alu.rv64", v86Binary: "alu.i386" },
  { key: "mixed", name: "Mixed", kind: "duration", family: "compute", rvBinary: "rvbench_fs.rv64", v86Binary: "rvbench_fs.i386" },
  { key: "boot", name: "Matched Boot", kind: "duration", family: "boot", phases: ["first"] },
  { key: "python", name: "Python fib(30)", kind: "duration", family: "python" },
  { key: "compile", name: "Compile (tcc -c)", kind: "duration", family: "compile" },
  { key: "numeric", name: "BYTEmark fixed NUMERIC SORT", nbenchName: "NUMERIC SORT", nbenchId: 0, nbenchFlag: "DONUMSORT", kind: "duration", family: "nbench" },
  { key: "string", name: "BYTEmark fixed STRING SORT", nbenchName: "STRING SORT", nbenchId: 1, nbenchFlag: "DOSTRINGSORT", kind: "duration", family: "nbench" },
  { key: "bitfield", name: "BYTEmark fixed BITFIELD", nbenchName: "BITFIELD", nbenchId: 2, nbenchFlag: "DOBITFIELD", kind: "duration", family: "nbench" },
  { key: "fpemul", name: "BYTEmark fixed FP EMULATION", nbenchName: "FP EMULATION", nbenchId: 3, nbenchFlag: "DOEMF", kind: "duration", family: "nbench" },
  { key: "fourier", name: "BYTEmark fixed FOURIER", nbenchName: "FOURIER", nbenchId: 4, nbenchFlag: "DOFOUR", kind: "duration", family: "nbench" },
  { key: "assignment", name: "BYTEmark fixed ASSIGNMENT", nbenchName: "ASSIGNMENT", nbenchId: 5, nbenchFlag: "DOASSIGN", kind: "duration", family: "nbench" },
  { key: "idea", name: "BYTEmark fixed IDEA", nbenchName: "IDEA", nbenchId: 6, nbenchFlag: "DOIDEA", kind: "duration", family: "nbench" },
  { key: "huffman", name: "BYTEmark fixed HUFFMAN", nbenchName: "HUFFMAN", nbenchId: 7, nbenchFlag: "DOHUFF", kind: "duration", family: "nbench" },
]);

export const ROW_BY_KEY = new Map(ROWS.map((row) => [row.key, row]));
export const INTERPRETER_HOLDOUT_ROWS = Object.freeze([
  { key: "holdout_gzip", name: "Holdout: BusyBox gzip", kind: "duration", family: "holdout" },
  { key: "holdout_sort", name: "Holdout: BusyBox sort", kind: "duration", family: "holdout" },
  { key: "holdout_sha256", name: "Holdout: BusyBox SHA-256", kind: "duration", family: "holdout" },
  { key: "holdout_aes", name: "Holdout: OpenSSL AES-256-CTR", kind: "duration", family: "holdout" },
]);
export const HOLDOUT_ROW_BY_KEY = new Map(
  INTERPRETER_HOLDOUT_ROWS.map((row) => [row.key, row]),
);
export const SIDES = Object.freeze(["rewrite", "legacy", "v86"]);

export const GUEST_CONTRACT = Object.freeze({
  linux: "6.12.7",
  alpine: "3.24.1",
  rv64Arch: "riscv64",
  v86Arch: "i686",
});

export const NBENCH_WORKLOAD_CONTRACT = Object.freeze({
  variant: "scorecard-fixed-work-data32-v3",
  crossIsaComparable: true,
  executable: "nbench-fixed",
  selfTimedDiagnosticExecutable: "nbench",
  nativeDiagnosticExecutable: "nbench-native",
  contract: "nbench-workload-contract.json",
  transforms: Object.freeze([
    "nbench-fixed-data32.patch",
    "nbench-fixed-work.patch",
  ]),
  implementationSources: Object.freeze([
    "nbench-rv64-fastmem.c",
  ]),
  fixedParameters: Object.freeze({
    numeric: Object.freeze(["NUMNUMARRAYS=128", "NUMARRAYSIZE=8111", "NUMMINSECONDS=0"]),
    string: Object.freeze(["NUMSTRARRAYS=64", "STRARRAYSIZE=8111", "STRMINSECONDS=0"]),
    bitfield: Object.freeze(["NUMBITOPS=600", "BITFIELDSIZE=32768", "BITMINSECONDS=0"]),
    fpemul: Object.freeze(["EMFARRAYSIZE=3000", "EMFLOOPS=64", "EMFMINSECONDS=0"]),
    fourier: Object.freeze(["FOURSIZE=2000", "FOURMINSECONDS=0"]),
    assignment: Object.freeze(["ASSIGNARRAYS=8", "ASSIGNMINSECONDS=0"]),
    idea: Object.freeze(["IDEARRAYSIZE=4000", "IDEALOOPS=1024", "IDEAMINSECONDS=0"]),
    huffman: Object.freeze(["HUFARRAYSIZE=5000", "HUFFLOOPS=512", "HUFFMINSECONDS=0"]),
  }),
});

export const LEGACY_RELEASE = Object.freeze({
  tag: "v0.1.0",
  loaderSha256: "54df79c8b35cf50bcee34c4af02d7eb02b09e0439b717ee75bb830e733595b12",
  wasmSha256: "001e5158b8f47f981371eae201079e6ec3eb632ab39ba1d0caf9a1ca5412b049",
  modernVirtJit: false,
});

export const LEGACY_MODERN_COMPARATOR = Object.freeze({
  name: "legacy-jit-5b896f9-modern-virt-adapter-v1",
  sourceCommit: "5b896f9",
  adapterPatchSha256: "3d2f5b786ab8483f87257c1d49cbc61edce20a05cbfc0d2b0706fa6e7a82a107",
  loaderSha256: "54df79c8b35cf50bcee34c4af02d7eb02b09e0439b717ee75bb830e733595b12",
  wasmSha256: "274aaab5799386956a8c509434961c4a426066f8fc9f520e994c210affd61709",
  modernVirtJit: true,
});

export const V86_RUNTIME = Object.freeze({
  sourceCommit: "2f1346b0e7d88d4cbbbcc05fe15b4e369c3de23f",
  sourceTreeSha256: "ca8afd71c1444a56c20b1ab63939569329fd5369a1a75760b5dce53fc3ba00f8",
  wasmSha256: "4a1b966e5433187357733bccdd2009eda47e92a8c6d1c984572ad36d1a9cb4e1",
});

export const EXPECTED_PRODUCTION_POLICY = Object.freeze({
  enabled: "1",
  threshold: "131072",
  privilegedThresholdMultiplier: "64",
  privilegedControlEntriesEnabled: "0",
  stableChainEnabled: "1",
  quantum: "1024",
  controlEntriesEnabled: "1",
  inflightLimit: "2",
  multiPageControlPermille: "100",
  pageCap: "2",
  leaderCap: "512",
  regionTlbCacheEnabled: "1",
  regionTlbCacheMinAccesses: "4",
});

export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function parseNewc(archive) {
  const entries = new Map();
  let offset = 0;
  const align4 = (value) => (value + 3) & ~3;
  while (offset + 110 <= archive.length) {
    const header = archive.subarray(offset, offset + 110).toString("ascii");
    if (header.slice(0, 6) !== "070701" && header.slice(0, 6) !== "070702") {
      throw new Error(`invalid newc header at byte ${offset}`);
    }
    const fileSize = Number.parseInt(header.slice(54, 62), 16);
    const nameSize = Number.parseInt(header.slice(94, 102), 16);
    if (!Number.isSafeInteger(fileSize) || !Number.isSafeInteger(nameSize) || nameSize < 1) {
      throw new Error(`invalid newc lengths at byte ${offset}`);
    }
    const nameStart = offset + 110;
    const nameEnd = nameStart + nameSize;
    if (nameEnd > archive.length) throw new Error("truncated newc name");
    const name = archive.subarray(nameStart, nameEnd - 1).toString("utf8").replace(/^\.\//, "");
    const dataStart = align4(nameEnd);
    const dataEnd = dataStart + fileSize;
    if (dataEnd > archive.length) throw new Error(`truncated newc entry ${name}`);
    offset = align4(dataEnd);
    if (name === "TRAILER!!!") {
      // Linux initramfs permits concatenated newc archives. Holdout inputs use
      // a deterministic overlay appended to the frozen scorecard rootfs so we
      // do not rebuild or silently mutate that population.
      while (offset < archive.length && archive[offset] === 0) offset++;
      if (offset >= archive.length) break;
      continue;
    }
    entries.set(name, archive.subarray(dataStart, dataEnd));
  }
  return entries;
}

// Fingerprint the bytes inside the initramfs, not similarly named host files.
// These are the exact benchmark bytes the guest executes.
export function embeddedBenchmarkSha256(rowSpec, initrd) {
  if (rowSpec.family === "boot") return null;
  const entries = parseNewc(initrd);
  if (rowSpec.family === "holdout") {
    const bytes = entries.get(`opt/holdout/${rowSpec.key}`);
    if (!bytes) {
      throw new Error(`holdout initramfs is missing /opt/holdout/${rowSpec.key}`);
    }
    return sha256(bytes);
  }
  const required = (name) => {
    const bytes = entries.get(`opt/scorecard/${name}`);
    if (!bytes) throw new Error(`modern initramfs is missing /opt/scorecard/${name}`);
    return bytes;
  };
  if (rowSpec.family === "compute") return sha256(required(rowSpec.key));
  if (rowSpec.family === "python") return sha256(required("fib.py"));
  if (rowSpec.family === "compile") {
    return sha256(Buffer.concat([required("tcc"), required("w.c")]));
  }
  if (rowSpec.family === "nbench") return sha256(required("nbench"));
  throw new Error(`no embedded benchmark fingerprint rule for ${rowSpec.family}`);
}

export function embeddedWorkloadSha256(rowSpec, initrd, options = {}) {
  const entries = parseNewc(initrd);
  const requiredBytes = (name) => {
    const bytes = entries.get(`opt/scorecard/${name}`);
    if (!bytes) throw new Error(`modern initramfs is missing /opt/scorecard/${name}`);
    return bytes;
  };
  const required = (name) => sha256(requiredBytes(name));
  const identity = {
    benchmark: rowSpec.family === "nbench"
      ? required(options.nbenchExecutable || NBENCH_WORKLOAD_CONTRACT.executable)
      : embeddedBenchmarkSha256(rowSpec, initrd),
  };
  if (rowSpec.family === "nbench") {
    identity.workloadContract = required(NBENCH_WORKLOAD_CONTRACT.contract);
    identity.workloadTransforms = sha256(Buffer.concat(
      NBENCH_WORKLOAD_CONTRACT.transforms.map(requiredBytes),
    ));
    identity.implementationSources = sha256(Buffer.concat(
      NBENCH_WORKLOAD_CONTRACT.implementationSources.map(requiredBytes),
    ));
  }
  return identity;
}

export function median(values) {
  const sorted = values.filter(Number.isFinite).toSorted((a, b) => a - b);
  if (!sorted.length) return null;
  const middle = sorted.length >> 1;
  return sorted.length & 1
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
}

export function sampleSpread(values) {
  const finite = values.filter((value) => Number.isFinite(value) && value > 0);
  if (!finite.length) return null;
  return Math.max(...finite) / Math.min(...finite);
}

export function speedRatio(kind, current, comparison) {
  if (!(current > 0) || !(comparison > 0)) return null;
  return kind === "throughput" ? current / comparison : comparison / current;
}

export function verdict(ratio) {
  if (ratio == null) return "N/A";
  if (ratio >= 1.05) return `WIN ${ratio.toFixed(2)}x`;
  if (ratio >= MATCH_FLOOR) return `MATCH ${ratio.toFixed(2)}x`;
  return `LOSS ${(1 / ratio).toFixed(2)}x behind`;
}

export function balancedOrder(rep, sides = SIDES) {
  if (sides.length < 2) return [...sides];
  const offset = rep % sides.length;
  return [...sides.slice(offset), ...sides.slice(0, offset)];
}

export function parseNbench(output, expectedName) {
  const values = {};
  let pendingName = null;
  let unstable = 0;
  for (const line of output.split("\n")) {
    const named = line.match(/^([A-Z][A-Z ]+?)\s+:\s*([\d.e+]*)/);
    if (named) {
      pendingName = named[1].trim();
      if (named[2]) {
        values[pendingName] = Number(named[2]);
        pendingName = null;
      }
      continue;
    }
    const continued = line.match(/^\s+:\s+([\d.e+]+)\s+:/);
    if (continued && pendingName) {
      values[pendingName] = Number(continued[1]);
      pendingName = null;
    }
    if (/NOT 95 % statistically certain|variation among the individual results/.test(line)) {
      unstable++;
    }
  }
  const sampleCount = [...output.matchAll(/Number of runs:\s*(\d+)/g)].at(-1);
  const standardDeviation = [...output.matchAll(/Absolute standard deviation:\s*([\d.e+-]+)/gi)].at(-1);
  const value = values[expectedName];
  return {
    value: Number.isFinite(value) ? value : null,
    unstable,
    internal: {
      sampleCount: sampleCount ? Number(sampleCount[1]) : null,
      mean: Number.isFinite(value) ? value : null,
      standardDeviation: standardDeviation ? Number(standardDeviation[1]) : null,
      confidencePassed: unstable === 0,
    },
  };
}

const STAT_IDS = Object.freeze([
  0, 1, 3, 4, 5, 8, 10, 11, 12, 13, 17, 25, 31,
  32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
  48, 49,
  73, 74, 75, 76, 77, 78, 79, 80,
  89, 90, 91, 92, 93, 94, 95,
  96, 97, 98,
  99,
  100,
  101,
  102, 103, 104, 105, 106, 107,
  108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118,
  119, 120, 121, 122,
  123, 124, 125, 126, 127, 128, 129,
  134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145,
  146, 147, 148,
  149, 150, 151, 152,
  153, 154, 155, 156, 157, 158, 159, 160, 161,
]);

function safeBigInt(call) {
  try {
    const value = call();
    return typeof value === "bigint" ? value : BigInt(value ?? 0);
  } catch {
    return 0n;
  }
}

export function readRvCounters(vm, instructionCount) {
  const stats = Object.fromEntries(
    STAT_IDS.map((id) => [id, safeBigInt(() => vm.ex.jit_stat(id))]),
  );
  return {
    stats,
    staticT0: Array.from(
      { length: 14 },
      (_, index) => safeBigInt(() => vm.ex.jit_static_t0_stat?.(index) ?? 0n),
    ),
    instructions: safeBigInt(instructionCount),
    host: {
      modules: vm.jitRegCount ?? 0,
      bytes: vm.jitRegBytes ?? 0,
      compileMs: vm.jitCompileMs ?? vm.jitRegMs ?? 0,
      totalMs: vm.jitRegTotalMs ?? 0,
      templateStores: vm.jitTemplateCacheStores ?? 0,
      templateHits: vm.jitTemplateCacheHits ?? 0,
      templateMisses: vm.jitTemplateCacheMisses ?? 0,
    },
  };
}

export function deltaRvCounters(before, after) {
  const stat = (id) => (after.stats[id] - before.stats[id]).toString();
  const staticT0 = (id) => (after.staticT0[id] - before.staticT0[id]).toString();
  const guestInstructions = after.instructions - before.instructions;
  const generated = after.stats[0] - before.stats[0];
  return {
    guestInstructions: guestInstructions.toString(),
    generatedInstructions: generated.toString(),
    directVectorInstructions: stat(152),
    generatedCoveragePercent:
      guestInstructions > 0n
        ? Number((generated * 1_000_000n) / guestInstructions) / 10_000
        : null,
    dispatches: stat(1),
    systemEntries: stat(3),
    dispatchFeedback: {
      zeroRetireSuppressions: stat(153),
      emptyMisses: stat(154),
      tagCollisions: stat(155),
      mappingReverifications: stat(156),
      zeroRetireTracked: stat(157),
      zeroRetireUntracked: stat(158),
      zeroRetireProfileResets: stat(159),
      activeZeroRetireProfiles: after.stats[160].toString(),
      suppressedEntries: after.stats[161].toString(),
    },
    mmu: {
      mappingInvalidations: stat(89),
      changedSatp: stat(90),
      fullTlbClears: stat(91),
      storeJitTlbClears: stat(92),
      sfenceAll: stat(93),
      sfencePage: stat(94),
      sfenceForeignAsid: stat(95),
    },
    interpreterCalls: stat(4),
    interpreterInstructions: stat(5),
    staticT0FastInstructions: staticT0(3),
    staticT0SlowInstructions: staticT0(4),
    staticT0SlowBatches: staticT0(5),
    staticT0FetchFills: staticT0(6),
    staticT0Errors: staticT0(7),
    staticT0SampledInstructions: staticT0(8),
    staticT0Samples: staticT0(9),
    staticT0InterruptPolls: staticT0(10),
    staticT0ShortMarks: staticT0(11),
    staticT0ShortBypasses: staticT0(12),
    staticT0ShortClears: staticT0(13),
    regionIssued: stat(12),
    regionLanded: stat(13),
    regionPending: after.stats[17].toString(),
    regionEntries: stat(25),
    regionPolicy: {
      extensionsIssued: stat(32),
      exitsSampled: stat(33),
      buildMilliseconds: stat(34),
      exitWithoutProfile: stat(35),
      exitsInsideRegion: stat(36),
      extensionsDeferredForCooldown: stat(37),
      extensionsWithoutTarget: stat(38),
      extensionsQueued: stat(39),
      extensionDrainVisits: stat(40),
      extensionDrainWithoutAddressSpace: stat(41),
      regionsDemoted: stat(42),
      batchesBuilt: stat(43),
      batchMembers: stat(44),
      indirectCacheExtensions: stat(45),
    },
    regionCalls: stat(48),
    regionInstructions: stat(49),
    chainHops: stat(80),
    bulkCopyChunks: stat(8),
    timerInterrupts: {
      fromUser: stat(96),
      fromSupervisor: stat(97),
      fromMachine: stat(98),
    },
    denseCopyMembersTranslated: stat(99),
    denseStoreMembersTranslated: stat(100),
    bulkCopyMembersTranslated: stat(101),
    bulkCopyHelper: {
      calls: stat(102),
      staleState: stat(103),
      shortBoundary: stat(104),
      sourceRejected: stat(105),
      destinationRejected: stat(106),
      bytesCopied: stat(107),
    },
    pageTemplateProbe: {
      enabled: after.stats[108].toString(),
      eligible: stat(109),
      exactCodeMatches: stat(110),
      reusable: stat(111),
      crossPhysicalReusable: stat(112),
      retainedTemplates: after.stats[113].toString(),
      matchedRequestedEntries: stat(114),
      matchedCoveredEntries: stat(115),
      matchedMissingEntries: stat(116),
      unionCoveredEntries: stat(117),
      unionMissingEntries: stat(118),
      relocatedMatches: stat(119),
      relocatedRequestedEntries: stat(120),
      relocatedCoveredEntries: stat(121),
      relocatedMissingEntries: stat(122),
    },
    pageTemplateReuse: {
      enabled: after.stats[123].toString(),
      positionIndependentCompiles: stat(124),
      hits: stat(125),
      coveredEntries: stat(126),
      missingEntries: stat(127),
      savedWasmBytes: stat(128),
      eagerPhysicalCandidates: stat(129),
    },
    translation: {
      userNanoseconds: stat(73),
      userAttempts: stat(74),
      userBytes: stat(75),
      systemNanoseconds: stat(76),
      systemAttempts: stat(77),
      systemBytes: stat(78),
    },
    host: {
      modules: after.host.modules - before.host.modules,
      bytes: after.host.bytes - before.host.bytes,
      compileMs: after.host.compileMs - before.host.compileMs,
      totalMs: after.host.totalMs - before.host.totalMs,
      templateStores: after.host.templateStores - before.host.templateStores,
      templateHits: after.host.templateHits - before.host.templateHits,
      templateMisses: after.host.templateMisses - before.host.templateMisses,
    },
  };
}

export function pagePolicySnapshot(vm) {
  if (typeof vm.ex.jit_page_policy_stat !== "function") return null;
  const value = (index) => safeBigInt(() => vm.ex.jit_page_policy_stat(index)).toString();
  return {
    enabled: value(0),
    threshold: value(1),
    quantum: value(2),
    samples: value(3),
    sampledRetired: value(4),
    candidates: value(6),
    queued: value(7),
    pending: value(8),
    issued: value(11),
    landed: value(12),
    failed: value(13),
    compiledMappings: value(14),
    issuedPages: value(19),
    multiPageIssued: value(20),
    rebuilds: value(21),
    controlEntriesEnabled: value(23),
    inflightLimit: value(25),
    multiPageControlPermille: value(34),
    pageCap: value(38),
    leaderCap: value(39),
    regionTailChainEnabled: value(40),
    fetchStraddleForced: value(41),
    fetchStraddleDeferred: value(42),
    regionTlbCacheEnabled: value(43),
    regionTlbCacheMinAccesses: value(44),
    privilegedThresholdMultiplier: value(45),
    userRetired: value(46),
    privilegedRetired: value(47),
    userCandidates: value(48),
    privilegedCandidates: value(49),
    privilegedControlEntriesEnabled: value(50),
    stableChainEnabled: value(51),
  };
}

export function configureRvPolicy(vm, side, rewritePolicy = "production") {
  vm.ex.jit_set_enabled(1);
  if (side === "rewrite") {
    if (!['production', 'compat'].includes(rewritePolicy)) {
      throw new Error(`unknown rewrite policy: ${rewritePolicy}`);
    }
    if (typeof vm.ex.jit_set_page_policy !== "function") {
      throw new Error("rewrite runtime does not export jit_set_page_policy");
    }
    if (rewritePolicy === "compat") {
      vm.ex.jit_set_page_policy(0);
      vm.ex.sys_set_superblock?.(1);
      vm.ex.jit_set_multi_latch?.(1);
      return {
        name: "compat-superblock",
        superblocks: true,
        multiLatch: true,
      };
    }
    vm.ex.jit_set_page_policy(1);
    vm.ex.jit_set_region_tlb_cache?.(1);
    vm.ex.jit_set_region_tlb_cache_min_accesses?.(4);
    if (vm.tailCallsSupported) vm.ex.jit_set_region_tail_chain?.(1);
    return {
      name: "production-page",
      tailCallsSupported: !!vm.tailCallsSupported,
      requestedTailChain: !!vm.tailCallsSupported,
    };
  }
  if (side !== "legacy") throw new Error(`unknown RV64 side: ${side}`);
  vm.ex.sys_set_superblock?.(1);
  vm.ex.jit_set_multi_latch?.(1);
  return {
    name: "legacy-best",
    superblocks: true,
    multiLatch: true,
  };
}

export function parseGuestIdentity(output) {
  const match = output.match(
    /SCORECARD_V2_GUEST linux=([^\s]+) alpine=([^\s]+) arch=([^\s]+)/,
  );
  return match
    ? { linux: match[1], alpine: match[2], arch: match[3] }
    : null;
}

export function validateGuestIdentity(identity, side) {
  const problems = [];
  if (!identity) return ["modern Alpine guest identity marker missing"];
  if (identity.linux !== GUEST_CONTRACT.linux) {
    problems.push(`Linux ${identity.linux} (expected ${GUEST_CONTRACT.linux})`);
  }
  if (identity.alpine !== GUEST_CONTRACT.alpine) {
    problems.push(`Alpine ${identity.alpine} (expected ${GUEST_CONTRACT.alpine})`);
  }
  const expectedArch = side === "v86" ? GUEST_CONTRACT.v86Arch : GUEST_CONTRACT.rv64Arch;
  if (identity.arch !== expectedArch) {
    problems.push(`guest arch ${identity.arch} (expected ${expectedArch})`);
  }
  return problems;
}

export function validateProductionPolicy(snapshot, requested) {
  const problems = [];
  if (!snapshot) return ["production page-policy stats unavailable"];
  for (const [name, expected] of Object.entries(EXPECTED_PRODUCTION_POLICY)) {
    if (snapshot[name] !== expected) {
      problems.push(`production ${name}=${snapshot[name]} (expected ${expected})`);
    }
  }
  const expectedTail = requested.tailCallsSupported ? "1" : "0";
  if (snapshot.regionTailChainEnabled !== expectedTail) {
    problems.push(
      `production regionTailChainEnabled=${snapshot.regionTailChainEnabled} (expected ${expectedTail})`,
    );
  }
  return problems;
}
