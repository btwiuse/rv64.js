// Diagnostic-only Wasm boundary microbenchmark. Compares the current style
// (N guest registers loaded/stored through linear memory by every callee) with
// an internal multi-value ABI carrying the same N i64 values through a typed
// indirect call. This is deliberately not a scorecard row.

const uleb = (n) => {
  const out = [];
  do {
    let b = n & 0x7f;
    n = Math.floor(n / 128);
    if (n) b |= 0x80;
    out.push(b);
  } while (n);
  return out;
};
const sleb = (n) => {
  const out = [];
  let more = true;
  while (more) {
    let b = n & 0x7f;
    n >>= 7;
    more = !((n === 0 && !(b & 0x40)) || (n === -1 && (b & 0x40)));
    if (more) b |= 0x80;
    out.push(b);
  }
  return out;
};
const vec = (xs) => [...uleb(xs.length), ...xs.flat()];
const str = (s) => vec([...new TextEncoder().encode(s)]);
const section = (id, body) => [id, ...uleb(body.length), ...body];
const type = (params, results) => [0x60, ...vec(params), ...vec(results)];
const localGet = (i) => [0x20, ...uleb(i)];
const localSet = (i) => [0x21, ...uleb(i)];
const i32Const = (n) => [0x41, ...sleb(n)];
const i64Const = (n) => [0x42, ...sleb(n)];
const load64 = (off) => [0x29, ...uleb(3), ...uleb(off)];
const store64 = (off) => [0x37, ...uleb(3), ...uleb(off)];

function moduleBytes(nregs, multi) {
  const I32 = 0x7f, I64 = 0x7e;
  const calleeType = multi
    ? type(Array(nregs).fill(I64), Array(nregs).fill(I64))
    : type([I32], []);
  const runType = type([I32], [I64]);

  const callee = [];
  for (let i = 0; i < nregs; i++) {
    if (multi) {
      callee.push(...localGet(i), ...i64Const(i + 1), 0x7c); // i64.add
    } else {
      callee.push(
        ...localGet(0),
        ...localGet(0),
        ...load64(i * 8),
        ...i64Const(i + 1),
        0x7c,
        ...store64(i * 8),
      );
    }
  }
  callee.push(0x0b);

  // run(iterations): initialize state, loop through typed indirect calls,
  // then XOR every value into a checksum so all lanes remain observable.
  const run = [];
  const counter = 1;
  const firstValue = 2;
  for (let i = 0; i < nregs; i++) {
    if (multi) {
      run.push(...i64Const(i + 1), ...localSet(firstValue + i));
    } else {
      run.push(...i32Const(0), ...i64Const(i + 1), ...store64(i * 8));
    }
  }
  run.push(...localGet(0), ...localSet(counter), 0x03, 0x40); // loop void
  if (multi) {
    for (let i = 0; i < nregs; i++) run.push(...localGet(firstValue + i));
    run.push(...i32Const(0), 0x11, ...uleb(0), 0x00); // call_indirect type 0
    for (let i = nregs - 1; i >= 0; i--) run.push(...localSet(firstValue + i));
  } else {
    run.push(...i32Const(0), ...i32Const(0), 0x11, ...uleb(0), 0x00);
  }
  run.push(
    ...localGet(counter), ...i32Const(1), 0x6b, // i32.sub
    0x22, ...uleb(counter), // local.tee
    0x0d, 0x00, // br_if loop
    0x0b,
  );
  for (let i = 0; i < nregs; i++) {
    if (multi) run.push(...localGet(firstValue + i));
    else run.push(...i32Const(0), ...load64(i * 8));
    if (i) run.push(0x85); // i64.xor
  }
  run.push(0x0b);

  const runLocals = multi
    ? vec([[...uleb(1), I32], [...uleb(nregs), I64]])
    : vec([[...uleb(1), I32]]);
  const bodies = vec([
    [...uleb(callee.length + 1), 0x00, ...callee],
    [...uleb(runLocals.length + run.length), ...runLocals, ...run],
  ]);
  return new Uint8Array([
    0, 0x61, 0x73, 0x6d, 1, 0, 0, 0,
    ...section(1, vec([calleeType, runType])),
    ...section(3, vec([uleb(0), uleb(1)])),
    ...section(4, vec([[0x70, 0x00, ...uleb(1)]])),
    ...section(5, vec([[0x00, ...uleb(1)]])),
    ...section(7, vec([[...str("run"), 0x00, ...uleb(1)]])),
    ...section(9, vec([[0x00, ...i32Const(0), 0x0b, ...vec([uleb(0)])]])),
    ...section(10, bodies),
  ]);
}

const median = (xs) => [...xs].sort((a, b) => a - b)[xs.length >> 1];
const madPct = (xs) => {
  const m = median(xs);
  return 100 * median(xs.map((x) => Math.abs(x - m))) / m;
};

function bench(nregs, multi) {
  const bytes = moduleBytes(nregs, multi);
  const t0 = performance.now();
  const mod = new WebAssembly.Module(bytes);
  const instance = new WebAssembly.Instance(mod);
  const buildMs = performance.now() - t0;
  const run = instance.exports.run;
  for (let i = 0; i < 20; i++) run(10_000);
  let iterations = 100_000;
  while (iterations < 25_600_000) {
    const s = performance.now();
    run(iterations);
    if (performance.now() - s >= 150) break;
    iterations *= 2;
  }
  const samples = [];
  let checksum = 0n;
  for (let i = 0; i < 7; i++) {
    const s = performance.now();
    checksum = run(iterations);
    samples.push(performance.now() - s);
  }
  // Each mode calibrates independently, so verify both with common fixed work.
  checksum = run(12_345);
  return {
    mode: multi ? "multivalue" : "memory",
    nregs,
    iterations,
    ns_per_call: median(samples) * 1e6 / iterations,
    mad_pct: madPct(samples),
    build_ms: buildMs,
    wasm_bytes: bytes.length,
    checksum: `0x${BigInt.asUintN(64, checksum).toString(16)}`,
  };
}

const results = [];
for (const n of [8, 16, 31]) {
  const memory = bench(n, false);
  const multivalue = bench(n, true);
  if (memory.checksum !== multivalue.checksum) {
    throw new Error(`checksum mismatch at ${n}: ${memory.checksum} != ${multivalue.checksum}`);
  }
  results.push(memory, multivalue);
  console.log(
    `${String(n).padStart(2)} regs  memory=${memory.ns_per_call.toFixed(2)} ns  ` +
    `multivalue=${multivalue.ns_per_call.toFixed(2)} ns  ` +
    `speedup=${(memory.ns_per_call / multivalue.ns_per_call).toFixed(2)}x`,
  );
}
console.log(`ABI_MICROBENCH_JSON ${JSON.stringify({ diagnostic_only: true, results })}`);
