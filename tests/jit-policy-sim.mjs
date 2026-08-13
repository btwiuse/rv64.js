#!/usr/bin/env node
// Replay page-heat traces through candidate tier-up/compile queue policies.
// This is intentionally a cost model, not a benchmark: it narrows the policy
// space before expensive browser A/B runs.

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const args = process.argv.slice(2);
const options = Object.fromEntries(args.filter((arg) => arg.startsWith("--")).map((arg) => {
  const [key, ...value] = arg.slice(2).split("=");
  return [key, value.length ? value.join("=") : "true"];
}));
const files = args.filter((arg) => !arg.startsWith("--"));
if (!files.length) {
  console.error(
    "usage: node tests/jit-policy-sim.mjs TRACE.json [...] " +
      "[--unit=mapping] [--thresholds=65536,131072,200000] " +
      "[--compile-ms=1,4,16] [--queues=0,8] [--mips=10] [--speedup=2]",
  );
  process.exit(2);
}

const list = (name, fallback) => (options[name] || fallback).split(",").map(Number);
const units = (options.unit || "mapping").split(",");
const thresholds = list(
  "thresholds",
  "32768,65536,131072,200000,262144,524288,1048576",
);
const compileTimes = list("compile-ms", "1,4,16");
const queueCaps = list("queues", "0,8");
const interpreterMips = Number(options.mips || 10);
const speedup = Number(options.speedup || 2);
for (const unit of units) {
  if (!["page", "mapping", "aspace"].includes(unit)) {
    throw new Error(`unknown unit ${unit}; use page, mapping, or aspace`);
  }
}
if (!(interpreterMips > 0) || !(speedup > 1)) {
  throw new Error("--mips must be positive and --speedup must exceed one");
}

const number = (value) => {
  const result = Number(value);
  if (!Number.isSafeInteger(result) && typeof value !== "number") {
    throw new Error(`trace integer exceeds JavaScript's exact range: ${value}`);
  }
  return result;
};

function normalizePoints(points, total, first, last) {
  const all = total === 0 ? [] : [[first, 1], ...points, [last, total]];
  all.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const result = [];
  for (const [at, heat] of all) {
    const previous = result.at(-1);
    if (previous?.[0] === at) {
      previous[1] = Math.max(previous[1], heat);
    } else if (!previous || heat > previous[1]) {
      result.push([at, heat]);
    }
  }
  if (result.length) result[result.length - 1][1] = total;
  return result;
}

function seriesHeatAt(points, total, at) {
  if (!points.length || at < points[0][0]) return 0;
  if (at >= points.at(-1)[0]) return total;
  let lo = 0;
  let hi = points.length - 1;
  while (lo + 1 < hi) {
    const mid = (lo + hi) >> 1;
    if (points[mid][0] <= at) lo = mid;
    else hi = mid;
  }
  const [at0, heat0] = points[lo];
  const [at1, heat1] = points[hi];
  if (at1 === at0) return heat1;
  return heat0 + (heat1 - heat0) * (at - at0) / (at1 - at0);
}

function seriesThresholdAt(points, threshold) {
  for (let index = 0; index < points.length; index++) {
    const [at1, heat1] = points[index];
    if (heat1 < threshold) continue;
    if (index === 0) return at1;
    const [at0, heat0] = points[index - 1];
    if (heat1 === heat0) return at1;
    return at0 + (at1 - at0) * (threshold - heat0) / (heat1 - heat0);
  }
  return Infinity;
}

function directEntity(id, total, first, last, points, metadata = {}) {
  const normalized = normalizePoints(points, total, first, last);
  return {
    id,
    total,
    first,
    last,
    members: 1,
    ...metadata,
    heatAt: (at) => seriesHeatAt(normalized, total, at),
    thresholdAt: (threshold) => seriesThresholdAt(normalized, threshold),
  };
}

function groupEntities(atoms, keyOf) {
  const groups = new Map();
  for (const atom of atoms) {
    const key = keyOf(atom);
    const group = groups.get(key) || [];
    group.push(atom);
    groups.set(key, group);
  }
  return [...groups].map(([id, members]) => {
    const total = members.reduce((sum, member) => sum + member.total, 0);
    const first = Math.min(...members.map((member) => member.first));
    const last = Math.max(...members.map((member) => member.last));
    const heatAt = (at) => members.reduce((sum, member) => sum + member.heatAt(at), 0);
    return {
      id,
      total,
      first,
      last,
      members: members.length,
      uniquePcs: Math.max(...members.map((member) => member.uniquePcs)),
      uniqueEntries: Math.min(
        Math.max(...members.map((member) => member.uniquePcs)),
        members.reduce((sum, member) => sum + member.uniqueEntries, 0),
      ),
      heatAt,
      thresholdAt(threshold) {
        if (total < threshold) return Infinity;
        let lo = first;
        let hi = last;
        for (let iteration = 0; iteration < 52; iteration++) {
          const mid = (lo + hi) / 2;
          if (heatAt(mid) >= threshold) hi = mid;
          else lo = mid;
        }
        return hi;
      },
    };
  });
}

function buildEntities(trace) {
  const pagePoints = new Map();
  const contextPoints = new Map();
  const contextKey = (page, vpage, satp, mode) => `${page}|${vpage}|${satp}|${mode}`;
  for (const event of trace.events) {
    const kind = number(event.kind);
    if (kind & 1) {
      const points = pagePoints.get(event.page) || [];
      points.push([number(event.at), number(event.pageHeat)]);
      pagePoints.set(event.page, points);
    }
    if (kind & 2) {
      const key = contextKey(event.page, event.vpage, event.satp, event.mode);
      const points = contextPoints.get(key) || [];
      points.push([number(event.at), number(event.contextHeat)]);
      contextPoints.set(key, points);
    }
  }

  const pages = trace.pages.map((page) => directEntity(
    String(page.page),
    number(page.total),
    number(page.first),
    number(page.last),
    pagePoints.get(page.page) || [],
    {
      page: page.page,
      uniquePcs: number(page.uniquePcs),
      uniqueEntries: number(page.uniqueEntries),
    },
  ));
  const atoms = trace.contexts.map((context) => {
    const key = contextKey(context.page, context.vpage, context.satp, context.mode);
    return directEntity(
      key,
      number(context.total),
      number(context.first),
      number(context.last),
      contextPoints.get(key) || [],
      {
        page: context.page,
        vpage: context.vpage,
        satp: context.satp,
        mode: context.mode,
        uniquePcs: number(context.uniquePcs),
        uniqueEntries: number(context.uniqueEntries),
      },
    );
  });
  return {
    page: pages,
    // Generated RV64 code can be reused when both VA and PA agree; the normal
    // dispatch mapping check makes the page-table root itself unnecessary.
    mapping: groupEntities(atoms, (atom) => `${atom.page}|${atom.vpage}`),
    // Current page-superblock bookkeeping uses SATP + virtual page.
    aspace: groupEntities(
      atoms,
      (atom) => `${atom.page}|${atom.vpage}|${atom.satp}`,
    ),
  };
}

function simulate(entities, observed, threshold, compileMs, queueCap) {
  const delay = compileMs * interpreterMips * 1_000;
  const breakEven = delay / (1 - 1 / speedup);
  const candidates = entities
    .filter((entity) => entity.total >= threshold)
    .map((entity) => ({ entity, arrival: entity.thresholdAt(threshold) }))
    .sort((a, b) => a.arrival - b.arrival || b.entity.total - a.entity.total);

  let active = null;
  const queue = [];
  const published = [];
  let dropped = 0;
  let maxQueue = 0;
  const start = (job, at) => ({ ...job, start: at, end: at + delay });
  const finishThrough = (at) => {
    while (active && active.end <= at) {
      published.push(active);
      const finishedAt = active.end;
      active = queue.length ? start(queue.shift(), finishedAt) : null;
    }
  };

  for (const candidate of candidates) {
    finishThrough(candidate.arrival);
    if (!active) active = start(candidate, candidate.arrival);
    else if (queue.length < queueCap) {
      queue.push(candidate);
      maxQueue = Math.max(maxQueue, queue.length);
    } else dropped++;
  }
  finishThrough(Infinity);

  let reused = 0;
  let profitable = 0;
  let almostNoReuse = 0;
  for (const job of published) {
    const future = Math.max(0, job.entity.total - job.entity.heatAt(job.end));
    reused += future;
    if (future >= breakEven) profitable++;
    if (future < Math.min(threshold / 4, 16_384)) almostNoReuse++;
  }
  const idealReuse = candidates.reduce(
    (sum, candidate) => sum + candidate.entity.total - threshold,
    0,
  );
  return {
    threshold,
    compileMs,
    queueCap,
    candidates: candidates.length,
    compiled: published.length,
    dropped,
    maxQueue,
    coveragePct: observed ? reused / observed * 100 : 0,
    idealCoveragePct: observed ? idealReuse / observed * 100 : 0,
    profitable,
    almostNoReuse,
    breakEvenInstructions: breakEven,
    synchronousPauseMs: candidates.length * compileMs,
  };
}

const pct = (value) => `${value.toFixed(2)}%`;
const integer = (value) => Math.round(value).toLocaleString("en-US");
const allResults = [];

for (const file of files) {
  const trace = JSON.parse(await readFile(file, "utf8"));
  if (trace.format !== "rv64-jit-policy-trace-v2") {
    throw new Error(`${file}: expected rv64-jit-policy-trace-v2`);
  }
  const entities = buildEntities(trace);
  const observed = number(trace.meta.observedInstructions);
  const sortedPages = entities.page.map((page) => page.total).sort((a, b) => b - a);
  const topShare = (count) => sortedPages.slice(0, count).reduce((a, b) => a + b, 0) / observed * 100;
  const traceName = `${trace.context.mode}/${trace.context.phase}`;
  console.log(`\n${traceName} — ${integer(observed)} retired instructions`);
  console.log(
    `working set: ${entities.page.length} physical pages, ` +
      `${entities.mapping.length} PA+VA mappings, ${entities.aspace.length} SATP mappings; ` +
      `top page=${pct(topShare(1))}, top 10=${pct(topShare(10))}, top 100=${pct(topShare(100))}`,
  );

  for (const unit of units) {
    const results = [];
    for (const threshold of thresholds) {
      for (const compileMs of compileTimes) {
        for (const queueCap of queueCaps) {
          const result = simulate(
            entities[unit],
            observed,
            threshold,
            compileMs,
            queueCap,
          );
          results.push({ trace: traceName, unit, ...result });
        }
      }
    }
    allResults.push(...results);
    console.log(
      `\nunit=${unit}; interpreter=${interpreterMips} MIPS; assumed generated speedup=${speedup}x`,
    );
    console.log("threshold\tcompile\tqueue\tcandidates\tbuilt\tdropped\tcoverage\tideal\tprofitable\tlow-reuse\tsync-pause");
    for (const result of results) {
      console.log([
        integer(result.threshold),
        `${result.compileMs}ms`,
        result.queueCap,
        result.candidates,
        result.compiled,
        result.dropped,
        pct(result.coveragePct),
        pct(result.idealCoveragePct),
        result.profitable,
        result.almostNoReuse,
        `${integer(result.synchronousPauseMs)}ms`,
      ].join("\t"));
    }
  }
}

if (options.json) {
  const destination = resolve(options.json);
  await mkdir(dirname(destination), { recursive: true });
  await writeFile(destination, JSON.stringify({
    model: {
      interpreterMips,
      speedup,
      note: "Sparse heat events are linearly interpolated; queue overflow is pessimistically never retried.",
    },
    results: allResults,
  }, null, 2) + "\n");
  console.log(`\nwrote ${destination}`);
}
