import { pbkdf2Sync } from "node:crypto";

export const CPU_PROBE_SPEC = Object.freeze({
  algorithm: "pbkdf2-sha256",
  iterations: 100_000,
  samples: 7,
  statistic: "minimum",
});

export function median(values) {
  const sorted = values
    .filter((value) => value != null && Number.isFinite(value))
    .sort((a, b) => a - b);
  if (!sorted.length) return null;
  const middle = Math.floor(sorted.length / 2);
  return sorted.length & 1
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
}

export function alternatingOrder(rep, first, second) {
  return rep & 1 ? [second, first] : [first, second];
}

export function pairedOrder(rep, haveBoth = true) {
  return haveBoth ? alternatingOrder(rep, "v86", "rv64") : ["rv64"];
}

export function candidateVerdict(speed) {
  if (speed >= 1.1) return "IMPROVEMENT";
  if (speed <= 1 / 1.1) return "REGRESSION";
  return "TIE";
}

// Fixed native OpenSSL work avoids V8 tier-up in the host-drift probe. Taking
// the minimum of seven short samples tracks available single-thread speed
// while rejecting isolated scheduler/frequency-ramp delays. A sustained host
// slowdown still affects all seven samples and remains subject to the same
// scorecard spread limit.
export function cpuProbe() {
  let best = Infinity;
  for (let rep = 0; rep < CPU_PROBE_SPEC.samples; rep++) {
    const started = performance.now();
    pbkdf2Sync(
      "rv64-scorecard",
      "fixed-probe",
      CPU_PROBE_SPEC.iterations,
      32,
      "sha256",
    );
    best = Math.min(best, performance.now() - started);
  }
  return best;
}
