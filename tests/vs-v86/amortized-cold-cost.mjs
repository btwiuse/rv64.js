import { summary } from "../statistics.mjs";

function finiteSamples(name, values) {
  if (!Array.isArray(values) || values.length === 0) {
    throw new Error(`${name} must contain at least one sample`);
  }
  if (!values.every((value) => Number.isFinite(value) && value >= 0)) {
    throw new Error(`${name} contains an invalid elapsed value`);
  }
  return values;
}

function equalLength(name, left, right) {
  if (left.length !== right.length) {
    throw new Error(`${name} samples must have equal length`);
  }
}

// Construction happens once before the scorecard's execution-only clock. Use
// the upper end of the paired median-delta interval as a conservative one-time
// debit. A construction improvement is reported but cannot rescue a runtime
// regression or claim execution-parity credit.
export function constructionDebit(controlCreateMs, candidateCreateMs) {
  const control = finiteSamples("control construction", controlCreateMs);
  const candidate = finiteSamples("candidate construction", candidateCreateMs);
  equalLength("construction", control, candidate);
  const pairedDeltaMs = candidate.map((value, index) => value - control[index]);
  const pairedDelta = summary(pairedDeltaMs);
  return {
    control: summary(control),
    candidate: summary(candidate),
    pairedDeltaMs: pairedDelta,
    debitMs: Math.max(0, pairedDelta.medianConfidence95[1]),
    creditMs: Math.max(0, -pairedDelta.medianConfidence95[0]),
    rule: "max(0, upper95(paired median candidate-control construction ms))",
  };
}

// Charge a fresh VM's complete conservative construction debit to every row.
// This is intentionally stricter than amortizing one construction across a
// multi-row session and prevents a target-row win from hiding startup cost.
export function adjustedElapsedSpeedup(controlElapsedMs, candidateElapsedMs, debitMs) {
  const control = finiteSamples("control elapsed", controlElapsedMs);
  const candidate = finiteSamples("candidate elapsed", candidateElapsedMs);
  equalLength("elapsed", control, candidate);
  if (!Number.isFinite(debitMs) || debitMs < 0) {
    throw new Error("construction debit must be finite and non-negative");
  }
  const raw = control.map((value, index) => value / candidate[index]);
  const adjusted = control.map((value, index) => value / (candidate[index] + debitMs));
  return {
    debitMs,
    raw: summary(raw),
    adjusted: summary(adjusted),
    rule: "control elapsed / (candidate elapsed + conservative one-time construction debit)",
  };
}

export function adjustedNormalizedThroughput({
  controlElapsedMs,
  candidateElapsedMs,
  controlWork,
  candidateWork,
  debitMs,
}) {
  const controlElapsed = finiteSamples("control elapsed", controlElapsedMs);
  const candidateElapsed = finiteSamples("candidate elapsed", candidateElapsedMs);
  const controlUnits = finiteSamples("control work", controlWork);
  const candidateUnits = finiteSamples("candidate work", candidateWork);
  equalLength("elapsed", controlElapsed, candidateElapsed);
  equalLength("control elapsed/work", controlElapsed, controlUnits);
  equalLength("candidate elapsed/work", candidateElapsed, candidateUnits);
  if (!Number.isFinite(debitMs) || debitMs < 0) {
    throw new Error("construction debit must be finite and non-negative");
  }
  const ratios = candidateUnits.map((work, index) => {
    const candidateRate = work / (candidateElapsed[index] + debitMs);
    const controlRate = controlUnits[index] / controlElapsed[index];
    return candidateRate / controlRate;
  });
  return {
    debitMs,
    candidateControlRatios: summary(ratios),
  };
}
