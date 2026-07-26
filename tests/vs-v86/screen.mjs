// SCREENING harness — fast parallel first-pass, NOT for scored claims.
//
// Runs K parallel fresh-boot samples of a row (or the whole nbench table)
// and prints per-metric median + spread. Parallel boots share the machine,
// so absolute numbers skew a few percent and V8 background tier-up gets
// less headroom than a lone run — fine for "did my change help?", useless
// for win/loss verdicts. Confirm anything interesting with the serial,
// drift-guarded scorecard (REPS/NBREPS medians).
//
//   node tests/vs-v86/screen.mjs nb  [K]     # all nbench kernels, K boots
//   node tests/vs-v86/screen.mjs cc  [K]     # compile row
//   node tests/vs-v86/screen.mjs py  [K]     # python fib row
// Env passed through to the children: SB, DEMOTE, KEEPMIN, ROTNEST, WASM,
// TRACELVL, ARTIFACTS.
import { spawn } from "node:child_process";

const row = process.argv[2] || "nb";
const K = Math.max(1, +(process.argv[3] || 6));
const SP = process.env.SCREEN_DIR || "/tmp/claude-1000/-home-darren-src-arm64-js/1cdc4884-3763-44a5-97f3-a9990c34339a/scratchpad";
const script = { nb: `${SP}/nb-focus.mjs`, cc: `${SP}/cc-focus.mjs`, py: `${SP}/py-focus.mjs` }[row];
if (!script) {
  console.error("row must be nb|cc|py");
  process.exit(2);
}

const runOne = () =>
  new Promise((resolve) => {
    const p = spawn(process.execPath, [script], { env: process.env });
    let buf = "";
    p.stdout.on("data", (d) => (buf += d));
    p.stderr.on("data", () => {});
    p.on("close", () => resolve(buf));
  });

const t0 = performance.now();
const outs = await Promise.all(Array.from({ length: K }, runOne));
const dt = ((performance.now() - t0) / 1000).toFixed(0);

const median = (a) => {
  const v = [...a].sort((x, y) => x - y);
  return v.length ? v[(v.length / 2) | 0] : null;
};
const fmt = (name, vals) => {
  const v = vals.filter((x) => x != null && Number.isFinite(x));
  if (!v.length) return `${name}: no samples`;
  return `${name}: median=${median(v)} min=${Math.min(...v)} max=${Math.max(...v)} n=${v.length}`;
};

console.log(`SCREENING (${K} parallel boots, ${dt}s wall) — not for scored claims`);
if (row === "nb") {
  const tables = outs.map((o) => {
    try {
      const m = o.match(/\{[\s\S]*?\}/);
      return m ? JSON.parse(m[0]) : null;
    } catch {
      return null;
    }
  }).filter(Boolean);
  const kernels = new Set(tables.flatMap((t) => Object.keys(t)));
  for (const k of kernels) console.log(" " + fmt(k, tables.map((t) => t[k])));
  console.log(` (${tables.length}/${K} runs parsed)`);
} else if (row === "cc") {
  const vals = outs.map((o) => +(o.match(/^ms=(\d+)/m)?.[1] ?? NaN));
  console.log(" " + fmt("compile ms", vals));
} else {
  const vals = outs.map((o) => +(o.match(/fib_ms=(\d+)/)?.[1] ?? NaN));
  console.log(" " + fmt("python fib ms", vals));
}
