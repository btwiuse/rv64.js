// FULL PERFORMANCE SCORECARD — rv64.js vs copy/v86, one command.
//
// SYSTEM EMULATION ONLY: both emulators boot a FULL Linux and run every
// benchmark inside the guest. v86 has no user mode; comparing our user mode
// against v86's system mode was a past mistake — never do it again. The bar
// (user directive): rv64 must WIN or MATCH v86 on EVERY row, including every
// individual nbench kernel. Prints ONE table with a per-row verdict and a
// pass count, then writes a timestamped scorecard-<ts>.md + .json so
// before/after perf work is directly comparable.
//
//   ARTIFACTS=<artifacts> SB=1 nix develop -c node tests/vs-v86/scorecard.mjs
//   FULL=1     include interpreter columns + JIT-over-interp (slow)
//   NBENCH=1   include the BYTEmark suite, rv64 vs v86 (~3 min at NBREPS=1)
//   SKIP_V86=1 rv64 only
//   REPS=N     repeat every wall row N times on both emulators (interleaved)
//   NBREPS=N   repeat the whole nbench table N times on both emulators
//   AUTHORITATIVE=1 require v86, NBENCH=1, REPS>=3 and NBREPS>=3
//
// Rows: ALU / Mixed / Boot / python fib(30) / compile (tcc -c), all rv64-JIT vs
// v86-JIT; plus the NBENCH=1 BYTEmark table. compile + nbench run the SAME source
// on both sides (w.c through tcc@d9d02c5; nbench-byte-2.2.3), one build per ISA.
// Artifacts (build once with setup.sh; DEBIAN=1 for python + v86 compile/nbench):
// $ARTIFACTS/xbench/*, root-nbench.bin, cc-bench.img, deb-riscv64.ext4,
// vmlinuz-i386, deb-i386.cpio.gz, deb-i386-bench.cpio.gz, and a built copy/v86
// checkout at $ARTIFACTS/v86.
import { readFile, writeFile, copyFile, access } from "node:fs/promises";
import { createReadStream } from "node:fs";
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";
import { cpus, platform, release } from "node:os";
import { acquireBenchmarkLock } from "./bench-lock.mjs";
import { cpuProbe, median, pairedOrder } from "./bench-math.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const artifactsArg = process.env.ARTIFACTS || process.env.SC;
if (!artifactsArg) { console.error("set ARTIFACTS=<artifacts dir> (see setup.sh)"); process.exit(2); }
const ARTIFACTS = resolve(artifactsArg);
const releaseBenchmarkLock = await acquireBenchmarkLock(ARTIFACTS);
const V86DIR = process.env.V86DIR || join(ARTIFACTS, "v86");
function envFlag(name, fallback = false) {
  const raw = process.env[name];
  if (raw === undefined) return fallback;
  if (raw !== "0" && raw !== "1") {
    throw new Error(`${name} must be 0 or 1`);
  }
  return raw === "1";
}
function envPositiveInteger(name, fallback, minimum = 1) {
  const value =
    process.env[name] === undefined ? fallback : Number(process.env[name]);
  if (!Number.isSafeInteger(value) || value < minimum) {
    throw new Error(`${name} must be an integer >= ${minimum}`);
  }
  return value;
}
const FULL = envFlag("FULL");
const WANT_NBENCH = envFlag("NBENCH");
const WANT_V86 = !envFlag("SKIP_V86");
const SB = envFlag("SB");
const AUTHORITATIVE = envFlag("AUTHORITATIVE");
const TLBFILL = envPositiveInteger("TLBFILL", 0, 0);
// Every wall-clock row is run in a fresh process on both emulators. Sides are
// paired and their order alternates each repetition, preventing fixed-order
// drift and preventing rv64's background Wasm tier-up from polluting v86.
const REPS = envPositiveInteger("REPS", 1);
const NBREPS = envPositiveInteger("NBREPS", 1);
const NBENCH_TIMEOUT_MS = envPositiveInteger(
  "NBENCH_TIMEOUT_MS",
  360_000,
  60_000,
);

const wasm = await readFile(join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"));
const has = async (p) => { try { await access(p); return true; } catch { return false; } };

// ---------- rv64: isolated worker ----------
function rvSpawn(row, jit, extraEnv = {}) {
  return new Promise((resolve) => {
    const p = spawn(
      process.execPath,
      [join(root, "tests/vs-v86/rv64-scorecard-worker.mjs"), row],
      {
        cwd: root,
        env: {
          ...process.env,
          ARTIFACTS,
          SB: SB ? "1" : "0",
          TLBFILL: String(TLBFILL),
          DISABLE_JIT: jit ? "0" : "1",
          ...extraEnv,
        },
      },
    );
    let stdout = "";
    let stderr = "";
    p.stdout.on("data", (data) => (stdout += data));
    p.stderr.on("data", (data) => (stderr += data));
    p.on("close", (code) => {
      const match = stdout.match(/^RESULT_JSON (.+)$/m);
      if (code !== 0 || !match) {
        log(
          `\n[rv64 ${row} failed: exit=${code}] ${stderr || stdout.slice(-1000)}\n`,
        );
        resolve(null);
        return;
      }
      try {
        resolve(JSON.parse(match[1]));
      } catch (error) {
        log(`\n[rv64 ${row} invalid JSON: ${error.message}]\n`);
        resolve(null);
      }
    });
  });
}

// ---------- v86: spawn its runners in the checkout, parse RESULT ----------
function v86Spawn(script, env) {
  return new Promise((resolve) => {
    const p = spawn("node", ["--max-old-space-size=4096", script], { cwd: V86DIR, env: { ...process.env, ARTIFACTS, ...env } });
    let buf = "", err = "";
    p.stdout.on("data", (d) => (buf += d));
    p.stderr.on("data", (d) => (err += d));
    p.on("close", (code) => {
      const m = buf.match(/RESULT ms=(\d+)/);
      const md5 = (buf.match(/md5=([0-9a-f]{32})/) || [, null])[1];
      const chk = (buf.match(/chk=([^\s]+)/) || [, null])[1];
      if (code !== 0 || !m) {
        log(`\n[v86 ${script} failed: exit=${code}] ${err || buf.slice(-1000)}\n`);
        resolve(null);
      } else {
        resolve({ ms: +m[1], md5, chk });
      }
    });
  });
}
const v86Compute = (bin, jit) => v86Spawn("v86-compute.mjs", { BIN: bin, DISABLE_JIT: jit ? "0" : "1" });
const v86Boot = (jit) => v86Spawn("v86-boottime.mjs", { DISABLE_JIT: jit ? "0" : "1" });
const v86Python = (jit) => v86Spawn("deb-v86.mjs", { DISABLE_JIT: jit ? "0" : "1" });
const v86Compile = (jit) => v86Spawn("v86-compile.mjs", { DISABLE_JIT: jit ? "0" : "1" });
// v86 nbench: parse its self-timed per-kernel iterations/sec from the raw output.
function v86Nbench() {
  return new Promise((resolve) => {
    const p = spawn("node", ["--max-old-space-size=4096", "v86-nbench.mjs"], { cwd: V86DIR, env: { ...process.env, ARTIFACTS } });
    let buf = "", err = "";
    p.stdout.on("data", (d) => (buf += d));
    p.stderr.on("data", (d) => (err += d));
    p.on("close", (code) => {
      if (code !== 0) {
        log(`\n[v86 nbench failed: exit=${code}] ${err || buf.slice(-1000)}\n`);
        resolve(null);
        return;
      }
      const rows = {};
      for (const m of buf.matchAll(/^([A-Z][A-Z ]+?)\s+:\s+([\d.e+]+)\s+:/gm)) rows[m[1].trim()] = +m[2];
      const unstable = (
        buf.match(
          /NOT 95 % statistically certain|variation among the individual results/g,
        ) || []
      ).length;
      resolve({ values: rows, unstable });
    });
  });
}

// copy v86 runners into the checkout (relative ./src, ./bios resolve there)
let haveV86 = WANT_V86;
if (haveV86 && (await has(join(V86DIR, "src/main.js")))) {
  for (const f of ["v86-compute.mjs", "v86-boottime.mjs", "deb-v86.mjs", "v86-compile.mjs", "v86-nbench.mjs"])
    if (await has(join(root, "tests/vs-v86", f))) await copyFile(join(root, "tests/vs-v86", f), join(V86DIR, f));
} else haveV86 = false;
if (AUTHORITATIVE) {
  const preflight = [];
  if (!haveV86) preflight.push("built v86 checkout");
  if (!WANT_NBENCH) preflight.push("NBENCH=1");
  if (!SB) preflight.push("SB=1");
  if (REPS < 3 || !(REPS & 1)) preflight.push("odd REPS>=3");
  if (NBREPS < 3 || !(NBREPS & 1)) preflight.push("odd NBREPS>=3");
  if (preflight.length) {
    await releaseBenchmarkLock();
    console.error(
      `authoritative preflight failed; required: ${preflight.join(", ")}`,
    );
    process.exit(2);
  }
}

// ---------- run ----------
const R = {}; // name -> {rvj, rvi, v8j, v8i}
const log = (m) => process.stderr.write(m);

// HOST-DRIFT GUARD (this host is shared: a concurrent toolchain build took
// the load average to 29 on 24 cores and made the SAME binary measure 3x
// slower). Absolute numbers taken minutes apart are not comparable, and the
// v86-then-rv64 ordering turns any drift into a systematic bias against
// whichever side runs later. So: sample the host before and after every leg
// with a fixed CPU probe, and mark the scorecard INVALID when the probe
// moves more than DRIFT_TOL between the two sides of a row. A row measured
// on a drifting host is not evidence, whichever way it points.
const DRIFT_TOL = 1.25;
// Initialize OpenSSL before the first recorded sample.
cpuProbe();
const probes = [];
const probeNow = (label) => {
  const ms = cpuProbe();
  probes.push({ label, ms });
  return ms;
};

const trials = { wall: [], nbench: [] };
const WALL_ROWS = [
  {
    name: "ALU",
    worker: "alu",
    v86: (jit) => v86Compute("alu.i386", jit),
  },
  {
    name: "Mixed",
    worker: "mixed",
    v86: (jit) => v86Compute("rvbench_fs.i386", jit),
  },
  { name: "Boot", worker: "boot", v86: v86Boot },
  {
    name: "python fib(30)",
    worker: "python",
    available: () => has(join(ARTIFACTS, "deb-riscv64.ext4")),
    v86: v86Python,
  },
  {
    name: "compile (tcc -c)",
    worker: "compile",
    available: () => has(join(ARTIFACTS, "cc-bench.img")),
    v86: v86Compile,
  },
];
const normalizeRv = (result) =>
  result && {
    value: result.value,
    checksum: result.checksum ?? null,
    md5: result.md5 ?? null,
    jit: result.jit ?? null,
    profile: result.profile ?? null,
    input_sha256: result.input_sha256 ?? null,
    wasm_sha256: result.wasm_sha256,
  };
const normalizeV86 = (result) =>
  result && {
    value: result.ms,
    checksum: result.chk?.replace(/^checksum=/, "") ?? null,
    md5: result.md5 ?? null,
  };
async function measuredTrial({ side, kind, row, rep, jit, run }) {
  const label = `${kind}:${row}:jit${+jit}:rep${rep + 1}:${side}`;
  const probeBefore = probeNow(`${label}:before`);
  const started = new Date().toISOString();
  const result = await run();
  const probeAfter = probeNow(`${label}:after`);
  const trial = {
    kind,
    row,
    rep: rep + 1,
    jit,
    side,
    order: trials[kind].filter((entry) =>
      entry.row === row && entry.rep === rep + 1 && entry.jit === jit
    ).length + 1,
    started,
    probe_before_ms: probeBefore,
    probe_after_ms: probeAfter,
    result,
  };
  trials[kind].push(trial);
  return result;
}
function consistent(samples, field) {
  const values = [...new Set(samples.map((sample) => sample?.[field]).filter(Boolean))];
  return values.length === 1 ? values[0] : values.length ? `MISMATCH:${values.join(",")}` : null;
}

// Every wall row gets REPS fresh boots on both sides. Pair order alternates:
// rep 1 v86→rv64, rep 2 rv64→v86, etc. Each rv64 trial exits before the next
// side starts, so background module compilation cannot cross trial boundaries.
for (const jit of FULL ? [false, true] : [true]) {
  for (const spec of WALL_ROWS) {
    if (spec.available && !(await spec.available())) continue;
    const rvSamples = [];
    const v8Samples = [];
    log(`[wall ${spec.name} jit=${+jit}]`);
    for (let rep = 0; rep < REPS; rep++) {
      const sides = pairedOrder(rep, haveV86);
      for (const side of sides) {
        log(` ${rep + 1}${side === "rv64" ? "r" : "v"}`);
        if (side === "rv64") {
          rvSamples.push(
            await measuredTrial({
              side,
              kind: "wall",
              row: spec.name,
              rep,
              jit,
              run: async () => normalizeRv(await rvSpawn(spec.worker, jit)),
            }),
          );
        } else {
          v8Samples.push(
            await measuredTrial({
              side,
              kind: "wall",
              row: spec.name,
              rep,
              jit,
              run: async () => normalizeV86(await spec.v86(jit)),
            }),
          );
        }
      }
    }
    log(" ok\n");
    const result = (R[spec.name] ??= {});
    const rvKey = jit ? "rvj" : "rvi";
    const v8Key = jit ? "v8j" : "v8i";
    result[rvKey] = median(rvSamples.map((sample) => sample?.value));
    result[`${rvKey}_chk`] = consistent(rvSamples, "checksum");
    result[`${rvKey}_md5`] = consistent(rvSamples, "md5");
    if (haveV86) {
      result[v8Key] = median(v8Samples.map((sample) => sample?.value));
      result[`${v8Key}_chk`] = consistent(v8Samples, "checksum");
      result[`${v8Key}_md5`] = consistent(v8Samples, "md5");
    }
  }
}

// nbench is also fresh-process and alternates side order. The raw whole-table
// result for every repetition is retained in `trials.nbench`.
const NB_KERNELS = ["NUMERIC SORT", "STRING SORT", "BITFIELD", "FP EMULATION",
                    "FOURIER", "ASSIGNMENT", "IDEA", "HUFFMAN"];
const medianRows = (runs) => {
  const out = {};
  for (const kernel of NB_KERNELS) {
    const values = runs.map((run) => run?.values?.[kernel]).filter((value) => value != null);
    if (values.length) out[kernel] = median(values);
  }
  return out;
};
let nb = null;
if (WANT_NBENCH && (await has(join(ARTIFACTS, "root-nbench.bin")))) {
  const rvRuns = [];
  const v8Runs = [];
  const wantV8 =
    haveV86 && (await has(join(ARTIFACTS, "deb-i386-bench.cpio.gz")));
  log("[nbench]");
  for (let rep = 0; rep < NBREPS; rep++) {
    const sides = pairedOrder(rep, wantV8);
    for (const side of sides) {
      log(` ${rep + 1}${side === "rv64" ? "r" : "v"}`);
      if (side === "rv64") {
        const result = await measuredTrial({
          side,
          kind: "nbench",
          row: "nbench",
          rep,
          jit: true,
          run: () => rvSpawn("nbench", true),
        });
        rvRuns.push(result && {
          values: result.value,
          unstable: result.unstable ?? 0,
          jit: result.jit,
          wasm_sha256: result.wasm_sha256,
        });
      } else {
        const result = await measuredTrial({
          side,
          kind: "nbench",
          row: "nbench",
          rep,
          jit: true,
          run: () => v86Nbench(),
        });
        v8Runs.push(result);
      }
    }
  }
  log(" ok\n");
  let ni = null;
  if (FULL) {
    log("[nbench interp]");
    ni = (await rvSpawn("nbench", false))?.value ?? null;
    log(" ok\n");
  }
  nb = {
    jit: medianRows(rvRuns),
    int: ni,
    v8: wantV8 ? medianRows(v8Runs) : null,
    unstable: {
      rv64_reps: rvRuns.filter((run) => (run?.unstable ?? 0) > 0).length,
      v86_reps: v8Runs.filter((run) => (run?.unstable ?? 0) > 0).length,
    },
  };
}

// ---------- render ----------
// The bar: rv64 must WIN or MATCH v86 on EVERY row (main benchmarks AND every
// individual nbench kernel — no hiding losses inside a "mixed" summary).
// Speed ratio is uniform across units: >1 = rv64 faster. MATCH allows 5%
// (this host has documented double-digit run-to-run noise; verify borderline
// rows with interleaved median-of-N, never a single run).
const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const ms = (x) => (x == null ? "—" : `${Math.round(x)}ms`);
const speedup = (r) => (r?.rvi && r?.rvj ? (r.rvi / r.rvj).toFixed(1) + "×" : "—");
const order = ["ALU", "Mixed", "Boot", "python fib(30)", "compile (tcc -c)"];

// unified verdict rows: {name, rv, v8, unit, speed} with speed = rv64/v86 (>1 = rv64 faster)
const rows = [];
for (const k of order) {
  const r = R[k]; if (!r) continue;
  rows.push({ name: k, rv: ms(r.rvj), v8: ms(r.v8j), speedup: speedup(r),
              speed: r.rvj && r.v8j ? r.v8j / r.rvj : null });
}
if (nb) for (const k of Object.keys(nb.jit)) {
  const rj = nb.jit[k], vj = nb.v8?.[k];
  rows.push({ name: `nbench ${k}`, rv: rj ?? "—", v8: vj ?? "—", speedup: "—",
              speed: rj && vj ? rj / vj : null }); // iterations/sec: higher = faster
}
const verdict = (s) => s == null ? "—" : s >= 1.05 ? `**WIN** ${s.toFixed(2)}×` : s >= 0.95 ? `**MATCH** ${s.toFixed(2)}×` : `LOSS ${(1 / s).toFixed(2)}× behind`;
const passing = rows.filter((r) => r.speed != null && r.speed >= 0.95);
const scored = rows.filter((r) => r.speed != null);
const failing = scored.filter((r) => r.speed < 0.95);

let md = `# rv64.js vs v86 — SYSTEM-EMULATION scorecard

**SYSTEM EMULATION ONLY.** Both emulators boot a **full Linux** and run every
benchmark **inside the guest** (kernel + userland, JIT vs JIT, host wall-clock
or in-guest self-timing). v86 has no user mode — a user-mode comparison is
meaningless and was a past mistake; nothing user-mode appears in this table.

_${ts}. Speed ratio is rv64/v86 (>1 = rv64 faster). The bar: WIN or MATCH on
EVERY row. MATCH = within 5% (host noise; confirm borderline rows with
interleaved median-of-N runs)._

${AUTHORITATIVE ? "" : "**EXPLORATORY ONLY. Its pass count and row verdicts must not update the baseline.**\n"}

| # | Benchmark | rv64 JIT | v86 JIT | verdict (speed vs v86) |
|--:|---|--:|--:|---|
`;
rows.forEach((r, i) => { md += `| ${i + 1} | ${r.name} | ${r.rv} | ${r.v8} | ${verdict(r.speed)} |\n`; });
if (scored.length) {
  md += `\n**${AUTHORITATIVE ? "Overall" : "Exploratory tally"}: ${passing.length}/${scored.length} rows at win-or-match.**`;
  md += failing.length ? ` Failing: ${failing.map((r) => `${r.name} (${(1 / r.speed).toFixed(1)}× behind)`).join(", ")}.\n` : ` ALL ROWS PASS.\n`;
} else {
  md += `\n**No cross-emulator rows were scored.**\n`;
}
md += `\n_Protocol: ${REPS} fresh-process wall trial(s) per available emulator, paired with alternating side order; ${NBREPS} fresh-process nbench trial(s) per available emulator. Raw trials and probes are retained in the JSON._\n`;
if (!nb) md += `\n_nbench kernels not run (set NBENCH=1) — the bar includes them; a scorecard without them is INCOMPLETE._\n`;
if (!AUTHORITATIVE) md += `\n_Exploratory run. Set AUTHORITATIVE=1 for enforced 3×/3× replication and the complete 13-row manifest._\n`;
if (REPS > 1) {
  const sampleText = (row, side) =>
    trials.wall
      .filter((trial) => trial.row === row && trial.side === side && trial.jit)
      .map((trial) => trial.result?.value)
      .filter((value) => value != null)
      .map((value) => Math.round(value))
      .join(", ");
  md += `\n<details><summary>wall-clock raw samples (ms)</summary>\n\n| Benchmark | rv64 | v86 |\n|---|---:|---:|\n`;
  for (const row of order) {
    if (!R[row]) continue;
    md += `| ${row} | ${sampleText(row, "rv64") || "—"} | ${sampleText(row, "v86") || "—"} |\n`;
  }
  md += `\n</details>\n`;
}
if (FULL) {
  md += `\n<details><summary>interpreter columns (FULL=1)</summary>\n\n| Benchmark | rv64 interp | v86 interp | rv64 JIT/interp |\n|---|--:|--:|--:|\n`;
  for (const k of order) { const r = R[k]; if (!r) continue; md += `| ${k} | ${ms(r.rvi)} | ${ms(r.v8i)} | ${speedup(r)} |\n`; }
  md += `\n</details>\n`;
}
// ---------- enforcement (PERFORMANCE_PROGRESS.md: manifest, correctness, exit code) ----------
// Every required row must have produced numbers on both sides; ALU checksums
// must be bit-identical cross-ISA; Mixed low-32 must match; compile must
// yield an object md5 on both sides. Any violation = nonzero exit — the
// scorecard cannot silently shrink its scope or report a win on wrong output.
const problems = [];
const need = (cond, what) => { if (!cond) problems.push(what); };
if (AUTHORITATIVE) {
  need(haveV86, "authoritative run requires a built v86 checkout");
  need(WANT_NBENCH, "authoritative run requires NBENCH=1");
  need(SB, "authoritative run requires SB=1");
  need(REPS >= 3 && (REPS & 1), `authoritative run requires odd REPS>=3 (got ${REPS})`);
  need(NBREPS >= 3 && (NBREPS & 1), `authoritative run requires odd NBREPS>=3 (got ${NBREPS})`);
  need(scored.length === 13, `authoritative run requires exactly 13 scored rows (got ${scored.length})`);
}
for (const k of order) {
  const r = R[k];
  need(r && r.rvj != null, `${k}: rv64 row missing/failed`);
  if (haveV86) need(r && r.v8j != null, `${k}: v86 row missing/failed`);
  for (const jit of FULL ? [false, true] : [true]) {
    const rvSamples = trials.wall.filter(
      (trial) => trial.row === k && trial.side === "rv64" && trial.jit === jit,
    );
    need(
      rvSamples.length === REPS &&
        rvSamples.every((trial) => trial.result?.value != null),
      `${k}: expected ${REPS} complete rv64 jit=${+jit} samples, got ${rvSamples.filter((trial) => trial.result?.value != null).length}`,
    );
    if (haveV86) {
      const v8Samples = trials.wall.filter(
        (trial) => trial.row === k && trial.side === "v86" && trial.jit === jit,
      );
      need(
        v8Samples.length === REPS &&
          v8Samples.every((trial) => trial.result?.value != null),
        `${k}: expected ${REPS} complete v86 jit=${+jit} samples, got ${v8Samples.filter((trial) => trial.result?.value != null).length}`,
      );
    }
  }
}
need(
  R.ALU?.rvj_chk && !R.ALU.rvj_chk.startsWith("MISMATCH:"),
  `ALU rv64 checksum missing/inconsistent (${R.ALU?.rvj_chk})`,
);
if (haveV86) {
  need(
    R.ALU?.v8j_chk &&
      !R.ALU.v8j_chk.startsWith("MISMATCH:") &&
      R.ALU.rvj_chk === R.ALU.v8j_chk,
    `ALU checksum mismatch (rv=${R.ALU?.rvj_chk} v86=${R.ALU?.v8j_chk})`,
  );
}
need(
  R.Mixed?.rvj_chk && !R.Mixed.rvj_chk.startsWith("MISMATCH:"),
  `Mixed rv64 checksum missing/inconsistent (${R.Mixed?.rvj_chk})`,
);
if (haveV86) {
  need(
    R.Mixed?.v8j_chk &&
      !R.Mixed.v8j_chk.startsWith("MISMATCH:") &&
      R.Mixed.rvj_chk === R.Mixed.v8j_chk,
    `Mixed checksum mismatch (rv=${R.Mixed?.rvj_chk} v86=${R.Mixed?.v8j_chk})`,
  );
}
if (R["python fib(30)"]?.rvj != null) {
  need(
    R["python fib(30)"].rvj_chk === "832040",
    `python: rv64 checksum mismatch (${R["python fib(30)"].rvj_chk})`,
  );
}
if (haveV86 && R["python fib(30)"]?.v8j != null) {
  need(
    R["python fib(30)"].v8j_chk === "fib(30)=832040",
    `python: v86 checksum mismatch (${R["python fib(30)"].v8j_chk})`,
  );
}
{
  const r = R["compile (tcc -c)"];
  if (r) {
    need(
      !r.rvj || (r.rvj_md5 && !r.rvj_md5.startsWith("MISMATCH:")),
      `compile: rv64 object md5 missing/inconsistent (${r.rvj_md5})`,
    );
    if (haveV86) {
      need(
        !r.v8j || (r.v8j_md5 && !r.v8j_md5.startsWith("MISMATCH:")),
        `compile: v86 object md5 missing/inconsistent (${r.v8j_md5})`,
      );
    }
  }
}
const expectedWasm = createHash("sha256").update(wasm).digest("hex");
for (const trial of [...trials.wall, ...trials.nbench]) {
  if (trial.side === "rv64" && trial.result) {
    need(
      trial.result.wasm_sha256 === expectedWasm,
      `${trial.kind} ${trial.row} rep ${trial.rep}: worker used unexpected wasm ${trial.result.wasm_sha256}`,
    );
  }
}
{
  const groups = new Map();
  for (const trial of [...trials.wall, ...trials.nbench]) {
    if (trial.side !== "rv64" || !trial.result) continue;
    const key = `${trial.kind}:${trial.row}:jit${+trial.jit}`;
    const values = groups.get(key) ?? [];
    values.push(trial.result.input_sha256 ?? null);
    groups.set(key, values);
  }
  for (const [key, values] of groups) {
    const fingerprints = new Set(
      values.filter(Boolean).map((value) => JSON.stringify(value)),
    );
    need(
      values.every(Boolean) && fingerprints.size === 1,
      `${key}: workload input hashes are missing or inconsistent`,
    );
  }
}
if (WANT_NBENCH) {
  const KERNELS = ["NUMERIC SORT", "STRING SORT", "BITFIELD", "FP EMULATION",
                   "FOURIER", "ASSIGNMENT", "IDEA", "HUFFMAN"];
  for (const k of KERNELS) {
    need(nb?.jit?.[k] != null, `nbench ${k}: rv64 kernel missing`);
    if (haveV86) need(nb?.v8?.[k] != null, `nbench ${k}: v86 kernel missing`);
  }
  const rvNbench = trials.nbench.filter((trial) => trial.side === "rv64");
  const v8Nbench = trials.nbench.filter((trial) => trial.side === "v86");
  need(
    rvNbench.length === NBREPS &&
      rvNbench.every((trial) =>
        NB_KERNELS.every((kernel) => trial.result?.value?.[kernel] != null)
      ),
    `nbench: expected ${NBREPS} complete rv64 samples`,
  );
  if (haveV86) {
    need(
      v8Nbench.length === NBREPS &&
        v8Nbench.every((trial) =>
          NB_KERNELS.every((kernel) => trial.result?.values?.[kernel] != null)
        ),
      `nbench: expected ${NBREPS} complete v86 samples`,
    );
  }
  // nbench's own statistical check: if it says a kernel's repeats disagreed,
  // invalidate only when a majority of independent boots report instability.
  if ((nb?.unstable?.rv64_reps ?? 0) > NBREPS / 2) {
    problems.push(`nbench: rv64 reported statistical instability in ${nb.unstable.rv64_reps}/${NBREPS} repetitions`);
  }
  if ((nb?.unstable?.v86_reps ?? 0) > NBREPS / 2) {
    problems.push(`nbench: v86 reported statistical instability in ${nb.unstable.v86_reps}/${NBREPS} repetitions`);
  }
}
probeNow("end");
{
  const v = probes.map((p) => p.ms);
  const lo = Math.min(...v), hi = Math.max(...v);
  const drift = hi / Math.max(lo, 1e-9);
  md += `\n_Host CPU probe: ${probes.length} samples, ${lo.toFixed(0)}-${hi.toFixed(0)}ms (spread ${drift.toFixed(2)}x). Full probe series is in the JSON._\n`;
  if (drift > DRIFT_TOL) {
    problems.push(
      `host drifted ${drift.toFixed(2)}x during the run (probe ${lo.toFixed(0)}-${hi.toFixed(0)}ms) — ` +
        `rows are not comparable; rerun on a quiet host`,
      );
  }
  const groups = new Map();
  for (const trial of [...trials.wall, ...trials.nbench]) {
    const key = `${trial.kind}:${trial.row}:jit${+trial.jit}:rep${trial.rep}`;
    const values = groups.get(key) ?? [];
    values.push(trial.probe_before_ms, trial.probe_after_ms);
    groups.set(key, values);
  }
  for (const [key, values] of groups) {
    const pairLo = Math.min(...values);
    const pairHi = Math.max(...values);
    const pairDrift = pairHi / Math.max(pairLo, 1e-9);
    if (pairDrift > DRIFT_TOL) {
      problems.push(
        `${key} host drifted ${pairDrift.toFixed(2)}x within the paired repetition`,
      );
    }
  }
}
if (problems.length) {
  md += `\n**SCORECARD INVALID — ${problems.length} problem(s):** ${problems.join("; ")}\n`;
  process.exitCode = 1;
}

console.log("\n" + md);
await writeFile(join(ARTIFACTS, `scorecard-${ts}.md`), md);
async function sha256File(path) {
  if (!(await has(path))) return null;
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(path);
    stream.on("data", (data) => hash.update(data));
    stream.on("error", reject);
    stream.on("end", () => resolve(hash.digest("hex")));
  });
}
const artifactPaths = {
  bbl64: join(root, "web/images/bbl64.bin"),
  kernel_riscv64: join(root, "web/images/kernel-riscv64.bin"),
  root_riscv64: join(root, "web/images/root-riscv64.bin"),
  alu_rv64: join(ARTIFACTS, "xbench/alu.rv64"),
  alu_i386: join(ARTIFACTS, "xbench/alu.i386"),
  mixed_rv64: join(ARTIFACTS, "xbench/rvbench_fs.rv64"),
  mixed_i386: join(ARTIFACTS, "xbench/rvbench_fs.i386"),
  deb_riscv64: join(ARTIFACTS, "deb-riscv64.ext4"),
  cc_riscv64: join(ARTIFACTS, "cc-bench.img"),
  nbench_riscv64: join(ARTIFACTS, "root-nbench.bin"),
  vmlinuz_i386: join(ARTIFACTS, "vmlinuz-i386"),
  deb_i386: join(ARTIFACTS, "deb-i386.cpio.gz"),
  bench_i386: join(ARTIFACTS, "deb-i386-bench.cpio.gz"),
  v86_wasm: join(V86DIR, "build/v86.wasm"),
};
const artifact_sha256 = Object.fromEntries(
  await Promise.all(
    Object.entries(artifactPaths).map(async ([name, path]) => [
      name,
      await sha256File(path),
    ]),
  ),
);
const git = (...args) => {
  try {
    return execFileSync("git", args, { encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
};
const cpuInfo = cpus();
const procStatus = await readFile("/proc/self/status", "utf8").catch(() => "");
const cpuAffinity =
  procStatus.match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1]?.trim() ?? null;
const provenance = {
  schema: 2,
  git: git("-C", root, "rev-parse", "HEAD"),
  git_dirty: git("-C", root, "status", "--porcelain") !== "",
  v86_git: haveV86 ? git("-C", V86DIR, "rev-parse", "HEAD") : null,
  wasm_sha256: expectedWasm,
  artifact_sha256,
  node: process.version,
  host: {
    platform: platform(),
    release: release(),
    arch: process.arch,
    cpu: cpuInfo[0]?.model ?? "unknown",
    logical_cpus: cpuInfo.length,
    cpu_affinity: cpuAffinity,
  },
  protocol: {
    authoritative: AUTHORITATIVE,
    fresh_process_per_trial: true,
    alternating_pair_order: true,
    reps: REPS,
    nbreps: NBREPS,
  },
  reps: REPS,
  nbreps: NBREPS,
  sb: SB,
  config: {
    sb: SB,
    tlbfill: TLBFILL,
    tracelvl: process.env.TRACELVL ?? null,
    tracewin: process.env.TRACEWIN ?? null,
    ictrig: process.env.ICTRIG ?? null,
    batch: process.env.BATCH ?? null,
    keepmin: process.env.KEEPMIN ?? null,
    demote: process.env.DEMOTE ?? null,
    deftrack: process.env.DEFTRACK ?? null,
    rotnest: process.env.ROTNEST ?? null,
    bcap: process.env.BCAP ?? null,
    bpage: process.env.BPAGE ?? null,
    sbspace: process.env.SBSPACE ?? null,
    multilatch: process.env.MULTILATCH ?? null,
    nbench_timeout_ms: NBENCH_TIMEOUT_MS,
  },
  nbench: WANT_NBENCH,
  full: FULL,
};
await writeFile(
  join(ARTIFACTS, `scorecard-${ts}.json`),
  JSON.stringify(
    {
      ts,
      system_emulation: true,
      authoritative: AUTHORITATIVE,
      valid: problems.length === 0,
      provenance,
      results: R,
      nbench: nb,
      trials,
      probes,
      pass: `${passing.length}/${scored.length}`,
      problems,
    },
    null,
    2,
  ),
);
console.log(`saved ${join(ARTIFACTS, `scorecard-${ts}.md`)} (+ .json)`);
await releaseBenchmarkLock();
