import { RV64Debug as RV64 } from "/web/rv64.js";
import { runTimedBoot } from "/r076-browser-boot-lib.mjs";

const variant = new URL(self.location.href).searchParams.get("variant");
if (variant !== "control" && variant !== "candidate") {
  throw new Error(`invalid R076 variant ${variant}`);
}

const expectedHashes = {
  wasm: "28ceaf7bcf63b7267b25c5a9542e111d6b4f9f2b5780517ca14d89895ce10b3c",
  kernel: "57d077974820f7e222bdc42be72a26410e4883eccaf4b18527ebc7404a117ca2",
  initramfs: "cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808",
};
const decoder = new TextDecoder();
const encoder = new TextEncoder();

const nextTask = (() => {
  const waiting = [];
  const channel = new MessageChannel();
  channel.port1.onmessage = () => waiting.shift()?.();
  return () => new Promise((resolve) => {
    waiting.push(resolve);
    channel.port2.postMessage(0);
  });
})();

async function fetchBytes(path) {
  const response = await fetch(path, { cache: "no-store" });
  if (!response.ok) throw new Error(`${path}: fetch failed: ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function sha256(bytes) {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return [...digest].map((value) => value.toString(16).padStart(2, "0")).join("");
}

function delta(before, after, path) {
  let left = before;
  let right = after;
  for (const part of path.split(".")) {
    left = left[part];
    right = right[part];
  }
  return BigInt(right) - BigInt(left);
}

function policyProof(stats) {
  return {
    enabled: stats.pagePolicy.enabled,
    threshold: stats.pagePolicy.threshold,
    privilegedThresholdMultiplier: stats.pagePolicy.privilegedThresholdMultiplier,
    quantum: stats.pagePolicy.quantum,
    controlEntriesEnabled: stats.pagePolicy.controlEntriesEnabled,
    privilegedControlEntriesEnabled: stats.pagePolicy.privilegedControlEntriesEnabled,
    stableChainEnabled: stats.pagePolicy.stableChainEnabled,
    inflightLimit: stats.pagePolicy.inflightLimit,
    multiPageControlPermille: stats.pagePolicy.multiPageControlPermille,
    pageCap: stats.pagePolicy.regionPageCap,
    leaderCap: stats.pagePolicy.regionLeaderCap,
    tailChainEnabled: stats.pagePolicy.regionTailChainEnabled,
    regionTlbCacheEnabled: stats.pagePolicy.regionTlbCacheEnabled,
    regionTlbCacheMinAccesses: stats.pagePolicy.regionTlbCacheMinAccesses,
  };
}

async function main() {
  const [wasm, kernel, initramfs] = await Promise.all([
    fetchBytes("/rv64.wasm"),
    fetchBytes("/kernel"),
    fetchBytes("/initramfs"),
  ]);
  const assetHashes = {
    wasm: await sha256(wasm),
    kernel: await sha256(kernel),
    initramfs: await sha256(initramfs),
  };
  for (const [name, expected] of Object.entries(expectedHashes)) {
    if (assetHashes[name] !== expected) {
      throw new Error(`${name} SHA-256 ${assetHashes[name]} != ${expected}`);
    }
  }

  let output = "";
  const moduleKinds = {};
  let moduleCount = 0;
  let moduleBytes = 0;
  const vm = await RV64.create(wasm);
  vm.onWrite = (_fd, bytes) => {
    output += decoder.decode(bytes, { stream: true });
  };
  vm.onJitModule = (bytes, metadata) => {
    moduleCount++;
    moduleBytes += bytes.length;
    const kind = metadata?.kind ?? "unknown";
    moduleKinds[kind] = (moduleKinds[kind] ?? 0) + 1;
  };
  vm.ex.jit_set_enabled(1);
  vm.ex.jit_set_page_policy(1);
  vm.ex.jit_set_region_tlb_cache?.(1);
  vm.ex.jit_set_region_tlb_cache_min_accesses?.(4);
  if (vm.tailCallsSupported) vm.ex.jit_set_region_tail_chain?.(1);
  vm.bootVirtLinuxDirect({
    kernel,
    initrd: initramfs,
    ramMB: 512,
    cmdline: "console=ttyS0 rdinit=/init",
  });

  const modulesBefore = vm.jitRegCount ?? 0;
  const moduleIndex = vm.ex.jit_static_t0_system_prepare();
  vm.ex.jit_set_static_t0_system(0);
  vm.ex.jit_set_static_t0_sampled(variant === "candidate" ? 1 : 0);
  vm.ex.jit_set_static_t0_sampled_backoff(variant === "candidate" ? 1 : 0);
  const modulesAfter = vm.jitRegCount ?? 0;
  const beforeStats = vm.jitStats();
  const beforeInstructions = vm.virtInsnCount();

  const timing = await runTimedBoot({
    vm,
    ready: () => output.includes("SCORECARD_V2_READY"),
    nextTask,
  });

  const afterInstructions = vm.virtInsnCount();
  const afterStats = vm.jitStats();
  const instructions = afterInstructions - beforeInstructions;
  const generated = delta(beforeStats, afterStats, "generated.retired");
  const interpreted = delta(beforeStats, afterStats, "interpreter.retired");
  const staticFast = delta(beforeStats, afterStats, "staticT0.systemFastRetired");
  const sampled = delta(beforeStats, afterStats, "staticT0.sampledRetired");
  const samples = delta(beforeStats, afterStats, "staticT0.samples");
  const polls = delta(beforeStats, afterStats, "staticT0.interruptPolls");
  const marks = delta(beforeStats, afterStats, "staticT0.shortSampleMarks");
  const bypasses = delta(beforeStats, afterStats, "staticT0.shortSampleBypasses");
  const errors = delta(beforeStats, afterStats, "staticT0.systemErrors");
  const guestMatch = output.match(
    /SCORECARD_V2_GUEST linux=([^\s]+) alpine=([^\s]+) arch=([^\s]+)/,
  );
  if (!guestMatch) throw new Error(`guest identity missing: ${output.slice(-3000)}`);
  if (generated <= 0n || moduleCount <= 0) {
    throw new Error(`generated execution missing: retired=${generated} modules=${moduleCount}`);
  }
  if (generated + interpreted !== instructions) {
    throw new Error(
      `instruction accounting mismatch: ${generated}+${interpreted} != ${instructions}`,
    );
  }

  return {
    schema: 1,
    experiment: "R076 Chrome execution-only modern Boot",
    variant,
    userAgent: navigator.userAgent,
    hardwareConcurrency: navigator.hardwareConcurrency,
    assetHashes,
    guest: { linux: guestMatch[1], alpine: guestMatch[2], arch: guestMatch[3] },
    outputSha256: await sha256(encoder.encode(output)),
    outputMarkers: {
      ready: output.includes("SCORECARD_V2_READY"),
      guest: guestMatch[0],
    },
    timerBoundary: {
      setupBeforeTimer: [
        "asset-fetch-and-sha256",
        "RV64.create",
        "bootVirtLinuxDirect",
        "static-module-prepare-and-enable",
        "initial-counters",
      ],
      timed: "runTimedBoot:first-2M-pump-through-SCORECARD_V2_READY",
    },
    timing,
    instructions: instructions.toString(),
    mips: Number(instructions) / timing.ms / 1_000,
    counters: {
      generated: generated.toString(),
      interpreted: interpreted.toString(),
      dispatches: delta(beforeStats, afterStats, "generated.dispatches").toString(),
      staticFast: staticFast.toString(),
      sampled: sampled.toString(),
      samples: samples.toString(),
      polls: polls.toString(),
      marks: marks.toString(),
      bypasses: bypasses.toString(),
      errors: errors.toString(),
    },
    staticLifecycle: {
      moduleIndex,
      modulesBefore,
      modulesAfter,
      systemEnabled: false,
      sampledEnabled: variant === "candidate",
      sampledBackoffEnabled: variant === "candidate",
    },
    policy: policyProof(afterStats),
    modules: { count: moduleCount, bytes: moduleBytes, kinds: moduleKinds },
  };
}

try {
  postMessage({ type: "result", result: await main() });
} catch (error) {
  postMessage({ type: "error", error: error?.stack ?? String(error) });
}
