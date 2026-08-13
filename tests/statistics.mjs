// Deterministic descriptive statistics for benchmark reports. Confidence
// bounds use a fixed-seed nonparametric bootstrap of the sample median so raw
// reports are reproducible byte-for-byte for identical inputs.

export function quantile(values, fraction) {
  const ordered = [...values].sort((left, right) => left - right);
  if (ordered.length === 0) return 0;
  return ordered[Math.min(ordered.length - 1, Math.floor(fraction * ordered.length))];
}

export function median(values) {
  const ordered = [...values].sort((left, right) => left - right);
  if (ordered.length === 0) return 0;
  const middle = Math.floor(ordered.length / 2);
  return ordered.length & 1
    ? ordered[middle]
    : (ordered[middle - 1] + ordered[middle]) / 2;
}

function medianConfidence95(values) {
  const finite = values.filter(Number.isFinite);
  if (finite.length === 0) return null;
  if (finite.length === 1) return [finite[0], finite[0]];
  let state = (0x9e37_79b9 ^ finite.length) >>> 0;
  const randomIndex = () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) % finite.length;
  };
  const bootstrapped = [];
  for (let iteration = 0; iteration < 4096; iteration++) {
    const sample = Array.from({ length: finite.length }, () => finite[randomIndex()]);
    bootstrapped.push(median(sample));
  }
  return [quantile(bootstrapped, 0.025), quantile(bootstrapped, 0.975)];
}

export function summary(values) {
  return {
    min: Math.min(...values),
    median: median(values),
    medianConfidence95: medianConfidence95(values),
    p95: quantile(values, 0.95),
    max: Math.max(...values),
    raw: values,
  };
}
