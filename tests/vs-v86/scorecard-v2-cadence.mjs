// The public RV64 Emulator and copy/v86 both return to the JavaScript event
// loop after one CPU slice. Keep the scorecard on that same cadence. The
// historical four-slice batch remains available only for causal diagnostics.

function flag(environment, name) {
  const value = environment[name];
  if (value === undefined) return false;
  if (value !== "1") throw new Error(`${name} must be 1 when specified`);
  return true;
}

export function parsePumpCadence(environment = process.env) {
  const legacyExplicitEveryPump = flag(
    environment,
    "SCORECARD_V2_YIELD_EVERY_PUMP",
  );
  const historicalBatched = flag(
    environment,
    "SCORECARD_V2_HISTORICAL_BATCHED_PUMPS",
  );
  if (legacyExplicitEveryPump && historicalBatched) {
    throw new Error(
      "SCORECARD_V2_YIELD_EVERY_PUMP and " +
      "SCORECARD_V2_HISTORICAL_BATCHED_PUMPS conflict",
    );
  }
  return Object.freeze({
    name: historicalBatched
      ? "historical-four-slices-per-turn"
      : "public-one-slice-per-turn",
    rv64SlicesPerEventLoopTurn: historicalBatched ? 4 : 1,
    everyPump: !historicalBatched,
    diagnostic: legacyExplicitEveryPump || historicalBatched,
    legacyExplicitEveryPump,
    historicalBatched,
  });
}

export function shouldYieldAfterPump(iteration, cadence) {
  if (!Number.isSafeInteger(iteration) || iteration < 0) {
    throw new TypeError("pump iteration must be a non-negative safe integer");
  }
  return cadence.everyPump || (iteration & 3) === 0;
}

export function cadenceDiagnostic(cadence) {
  if (!cadence.diagnostic) return null;
  return {
    ...(cadence.legacyExplicitEveryPump ? { yieldEveryPump: true } : {}),
    ...(cadence.historicalBatched ? { historicalBatchedPumps: true } : {}),
  };
}

export function cadenceRecord(cadence) {
  return {
    name: cadence.name,
    rv64SlicesPerEventLoopTurn: cadence.rv64SlicesPerEventLoopTurn,
    reference: "public-rv64-and-v86-event-driven",
  };
}
