function asBigInt(value) {
  try {
    if (typeof value === "bigint") return value;
    if (typeof value === "number") {
      return Number.isFinite(value) ? BigInt(Math.trunc(value)) : 0n;
    }
    return BigInt(value ?? 0);
  } catch {
    return 0n;
  }
}

function asNumber(value) {
  const number = Number(value ?? 0);
  return Number.isFinite(number) ? number : 0;
}

/** Format an exact counter without first narrowing it to a JavaScript number. */
export function formatJitCount(value) {
  const count = asBigInt(value);
  const negative = count < 0n;
  const absolute = negative ? -count : count;
  const units = [
    [1_000_000_000_000_000n, "P"],
    [1_000_000_000_000n, "T"],
    [1_000_000_000n, "B"],
    [1_000_000n, "M"],
    [1_000n, "K"],
  ];
  for (const [divisor, suffix] of units) {
    if (absolute < divisor) continue;
    const scaled = Number(absolute) / Number(divisor);
    const digits = scaled >= 100 ? 0 : scaled >= 10 ? 1 : 2;
    return `${negative ? "-" : ""}${scaled.toFixed(digits)}${suffix}`;
  }
  return count.toString();
}

export function formatJitBytes(value) {
  const bytes = asBigInt(value);
  const units = [
    [1n << 30n, "GiB"],
    [1n << 20n, "MiB"],
    [1n << 10n, "KiB"],
  ];
  for (const [divisor, suffix] of units) {
    if (bytes < divisor) continue;
    const scaled = Number(bytes) / Number(divisor);
    return `${scaled.toFixed(scaled >= 10 ? 1 : 2)} ${suffix}`;
  }
  return `${bytes} B`;
}

export function formatJitDuration(milliseconds) {
  const duration = Math.max(0, asNumber(milliseconds));
  if (duration === 0) return "0 ms";
  if (duration < 1) return "<1 ms";
  if (duration < 1_000) return `${duration.toFixed(duration >= 100 ? 0 : 1)} ms`;
  const seconds = duration / 1_000;
  return `${seconds.toFixed(seconds >= 10 ? 1 : 2)} s`;
}

function plural(count, singular, pluralForm = `${singular}s`) {
  return `${formatJitCount(count)} ${asBigInt(count) === 1n ? singular : pluralForm}`;
}

/**
 * Reduce the exact public jitStats() snapshot to stable, human-readable UI
 * fields. Keeping this pure lets the page update without touching VM state.
 */
export function summarizeJitStats(stats) {
  const generated = asBigInt(stats?.generated?.retired);
  const interpreted = asBigInt(stats?.interpreter?.retired);
  const instructions = asBigInt(stats?.instructions) || generated + interpreted;
  const modules = asBigInt(stats?.loader?.modules);
  const bytes = asBigInt(stats?.loader?.bytes);
  const queued = asBigInt(stats?.pagePolicy?.queued);
  const pending = asBigInt(stats?.regions?.pending);
  const issued = asBigInt(stats?.regions?.issued);
  const landed = asBigInt(stats?.regions?.landed);
  const translationFailures = asBigInt(stats?.regions?.translateFailures);
  const policyFailures = asBigInt(stats?.pagePolicy?.failed);
  const dispatches = asBigInt(stats?.generated?.dispatches);
  const chainHops = asBigInt(stats?.generated?.chainHops);
  const zeroRetire = asBigInt(stats?.generated?.zeroRetireDispatches);
  const suppressed = asBigInt(stats?.generated?.zeroRetireSuppressions);
  const emptyMisses = asBigInt(stats?.generated?.dispatchEmptyMisses);
  const translatedMilliseconds = Number(
    asBigInt(stats?.translation?.userNanoseconds) +
      asBigInt(stats?.translation?.systemNanoseconds),
  ) / 1_000_000;
  const compileMilliseconds = asNumber(stats?.loader?.compileMs);

  let coverage = asNumber(stats?.generatedCoverage);
  if (!coverage && generated > 0n && instructions > 0n) {
    coverage = Number(generated * 1_000_000n / instructions) / 1_000_000;
  }
  const coverageText = `${(Math.max(0, coverage) * 100).toFixed(1)}% generated`;

  let state = "Cold";
  let detail = "Waiting for guest execution";
  if (pending > 0n) {
    state = "Compiling";
    detail = `${plural(pending, "build")} in flight`;
  } else if (queued > 0n) {
    state = "Queued";
    detail = `${plural(queued, "hot region")} waiting to compile`;
  } else if (generated > 0n) {
    state = "Active";
    detail = `${coverageText} coverage`;
  } else if (modules > 0n) {
    state = "Installed";
    detail = `${plural(modules, "module")} ready to execute`;
  } else if (instructions > 0n) {
    state = "Profiling";
    detail = "Finding hot guest code";
  }

  const instructionsPerDispatch = asNumber(stats?.generatedInstructionsPerDispatch);
  const dispatchQuality = dispatches === 0n
    ? "waiting for generated code"
    : `${instructionsPerDispatch.toFixed(1)} insn/dispatch`;
  const pipelineDetails = [`${plural(queued, "region")} queued`];
  if (translationFailures === 0n && policyFailures === 0n) {
    pipelineDetails.push("0 failures");
  } else {
    if (translationFailures > 0n) {
      pipelineDetails.push(plural(translationFailures, "translation failure"));
    }
    if (policyFailures > 0n) {
      pipelineDetails.push(plural(policyFailures, "build failure"));
    }
  }

  return {
    state,
    detail,
    coverage: coverageText,
    execution: `${formatJitCount(generated)} JIT · ${formatJitCount(interpreted)} interpreted`,
    code: `${plural(modules, "module")} · ${formatJitBytes(bytes)}`,
    codeDetail: `${formatJitDuration(translatedMilliseconds)} translate · ${formatJitDuration(compileMilliseconds)} host compile`,
    pipeline: `${formatJitCount(issued)} issued · ${formatJitCount(landed)} landed · ${formatJitCount(pending)} pending`,
    pipelineDetail: pipelineDetails.join(" · "),
    dispatch: `${formatJitCount(dispatches)} dispatches · ${dispatchQuality}`,
    dispatchDetail: `${formatJitCount(chainHops)} direct chains`,
    fallback: `${formatJitCount(zeroRetire)} zero-retire · ${formatJitCount(suppressed)} suppressed`,
    fallbackDetail: `${formatJitCount(emptyMisses)} empty-cache misses`,
  };
}
