import { pbkdf2Sync } from "node:crypto";

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
// the minimum of three samples tracks single-thread speed while ignoring an
// isolated scheduler delay.
export function cpuProbe() {
  let best = Infinity;
  for (let rep = 0; rep < 3; rep++) {
    const started = performance.now();
    pbkdf2Sync("rv64-scorecard", "fixed-probe", 100_000, 32, "sha256");
    best = Math.min(best, performance.now() - started);
  }
  return best;
}
