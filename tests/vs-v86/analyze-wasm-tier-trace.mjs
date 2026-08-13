#!/usr/bin/env node

// Parse V8 --trace-wasm-compilation-times output without using the reported
// benchmark wall time. Module addresses are process-local identities. The
// runtime module is the address with the broadest function-index population;
// other nontrivial modules are dynamically generated guest-code modules.

import { readFile } from "node:fs/promises";
import { basename } from "node:path";

const inputs = process.argv.slice(2).map((argument) => {
  const split = argument.indexOf("=");
  return split === -1
    ? { label: basename(argument), path: argument }
    : { label: argument.slice(0, split), path: argument.slice(split + 1) };
});
if (!inputs.length) {
  throw new Error("usage: analyze-wasm-tier-trace.mjs [LABEL=]TRACE.log ...");
}

const compilePattern = /^Compiled function (0x[0-9a-f]+)#(\d+) using (Liftoff|TurboFan), took (\d+) μs and (\d+)(?: \/ (\d+) max\/total)? bytes; bodysize (\d+) codesize (\d+)/;

function percentile(values, fraction) {
  if (!values.length) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * fraction))];
}

function summarizeValues(values) {
  return {
    count: values.length,
    sum: values.reduce((total, value) => total + value, 0),
    min: values.length ? Math.min(...values) : null,
    p50: percentile(values, 0.50),
    p90: percentile(values, 0.90),
    max: values.length ? Math.max(...values) : null,
  };
}

function summarize(label, text) {
  const lines = text.split("\n");
  const modules = new Map();
  const events = [];
  const markers = [];
  let currentMarker = "unmarked";
  let resultLine = null;
  for (const [lineIndex, line] of lines.entries()) {
    if (line.startsWith("ENGINE_TIER_MARKER ")) {
      currentMarker = line.slice("ENGINE_TIER_MARKER ".length);
      markers.push({ line: lineIndex + 1, name: currentMarker });
      continue;
    }
    if (line.startsWith("RESULT_JSON ")) {
      resultLine = lineIndex + 1;
      currentMarker = "after-result";
      continue;
    }
    const match = line.match(compilePattern);
    if (!match) continue;
    const event = {
      line: lineIndex + 1,
      marker: currentMarker,
      module: match[1],
      functionIndex: Number(match[2]),
      compiler: match[3],
      microseconds: Number(match[4]),
      peakBytes: Number(match[5]),
      totalBytes: match[6] ? Number(match[6]) : null,
      bodyBytes: Number(match[7]),
      codeBytes: Number(match[8]),
    };
    events.push(event);
    if (!modules.has(event.module)) modules.set(event.module, new Map());
    const functions = modules.get(event.module);
    if (!functions.has(event.functionIndex)) functions.set(event.functionIndex, []);
    functions.get(event.functionIndex).push(event);
  }
  if (!events.length) throw new Error(`${label}: no V8 compilation events`);

  const moduleRows = [...modules].map(([address, functions]) => {
    const rows = [...functions].map(([functionIndex, functionEvents]) => ({
      functionIndex,
      bodyBytes: functionEvents[0].bodyBytes,
      events: functionEvents,
    }));
    return {
      address,
      functions: rows,
      functionCount: rows.length,
      bodyBytes: rows.reduce((total, row) => total + row.bodyBytes, 0),
    };
  });
  const runtime = moduleRows.toSorted((left, right) =>
    right.functionCount - left.functionCount || right.bodyBytes - left.bodyBytes)[0];
  for (const module of moduleRows) {
    module.kind = module.address === runtime.address
      ? "runtime"
      : module.bodyBytes >= 1024 ? "generated" : "auxiliary";
  }

  function kindSummary(kind) {
    const selectedModules = moduleRows.filter((module) => module.kind === kind);
    const functions = selectedModules.flatMap((module) => module.functions.map((row) => ({
      ...row,
      module: module.address,
    })));
    const selectedEvents = functions.flatMap((row) => row.events);
    const tiered = functions.filter((row) => row.events.some((event) => event.compiler === "TurboFan"));
    const tieredBeforeResult = functions.filter((row) => row.events.some((event) =>
      event.compiler === "TurboFan" && (resultLine === null || event.line < resultLine)));
    const tieredAfterResult = functions.filter((row) => row.events.some((event) =>
      event.compiler === "TurboFan" && resultLine !== null && event.line > resultLine));
    const liftoff = selectedEvents.filter((event) => event.compiler === "Liftoff");
    const turbofan = selectedEvents.filter((event) => event.compiler === "TurboFan");
    return {
      modules: selectedModules.length,
      functions: functions.length,
      bodyBytes: summarizeValues(functions.map((row) => row.bodyBytes)),
      functionsAtLeastOneMiB: functions.filter((row) => row.bodyBytes >= 1024 * 1024).length,
      tieredFunctions: tiered.length,
      tieredBeforeResult: tieredBeforeResult.length,
      tieredAfterResult: tieredAfterResult.length,
      neverTiered: functions.length - tiered.length,
      liftoffCompileMicroseconds: summarizeValues(liftoff.map((event) => event.microseconds)),
      turbofanCompileMicroseconds: summarizeValues(turbofan.map((event) => event.microseconds)),
      turbofanPeakBytes: summarizeValues(turbofan.map((event) => event.peakBytes)),
      largestFunctions: functions.toSorted((left, right) => right.bodyBytes - left.bodyBytes)
        .slice(0, 12).map((row) => ({
          module: row.module,
          functionIndex: row.functionIndex,
          bodyBytes: row.bodyBytes,
          liftoffMicroseconds: row.events.filter((event) => event.compiler === "Liftoff")
            .reduce((total, event) => total + event.microseconds, 0),
          turbofanMicroseconds: row.events.filter((event) => event.compiler === "TurboFan")
            .reduce((total, event) => total + event.microseconds, 0),
          liftoffEvents: row.events.filter((event) => event.compiler === "Liftoff")
            .map((event) => ({ line: event.line, marker: event.marker })),
          turbofanEvents: row.events.filter((event) => event.compiler === "TurboFan")
            .map((event) => ({ line: event.line, marker: event.marker })),
          tieredAfterResult: row.events.some((event) =>
            event.compiler === "TurboFan" && resultLine !== null && event.line > resultLine),
        })),
    };
  }

  const byMarker = Object.fromEntries([...new Set(events.map((event) => event.marker))].map((marker) => {
    const markerEvents = events.filter((event) => event.marker === marker);
    return [marker, {
      events: markerEvents.length,
      liftoffMicroseconds: markerEvents.filter((event) => event.compiler === "Liftoff")
        .reduce((total, event) => total + event.microseconds, 0),
      turbofanMicroseconds: markerEvents.filter((event) => event.compiler === "TurboFan")
        .reduce((total, event) => total + event.microseconds, 0),
    }];
  }));

  return {
    label,
    traceLines: lines.length,
    compilationEvents: events.length,
    moduleAddresses: modules.size,
    runtimeModule: runtime.address,
    resultLine,
    markers,
    byMarker,
    runtime: kindSummary("runtime"),
    generated: kindSummary("generated"),
    auxiliary: kindSummary("auxiliary"),
  };
}

const reports = [];
for (const input of inputs) {
  reports.push(summarize(input.label, await readFile(input.path, "utf8")));
}
process.stdout.write(`${JSON.stringify({
  format: "rv64-v8-wasm-tier-trace-v1",
  generatedClassification: "non-runtime module with at least 1024 total function-body bytes",
  reports,
}, null, 2)}\n`);
