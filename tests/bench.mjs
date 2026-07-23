// rv64.js performance benchmark (wasm build — where the JIT lives).
// Run: node tests/bench.mjs [--json]
//
// Workloads (each repeated; best-of-N reported to shed V8 tier-up noise):
//   user-int   bench guest, integer xorshift phase (JIT-friendly ALU)
//   user-fp    bench guest, FP phase under RNE (native-FP fast path)
//   boot       Linux boot to shell prompt (mixed system workload)
//   sys-md5    in-guest md5sum compute kernel (system-mode hot loop)
//
// Reported per workload: wall ms, guest Minsn/s, JIT coverage (% of guest
// insns retired inside JIT blocks), dispatches, compiled blocks.
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64 } = await import(join(root, "web/rv64.js"));
const wasmBytes = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);
const REPS = 3;
const json = process.argv.includes("--json");
const results = {};

function stats(vm, t0, t1, insns) {
  const ms = t1 - t0;
  return {
    ms: +ms.toFixed(1),
    minsn_s: +(Number(insns) / ms / 1000).toFixed(1),
    jit_pct: +((Number(vm.ex.jit_stat(0)) / Number(insns)) * 100).toFixed(1),
    dispatches: Number(vm.ex.jit_stat(1)),
    blocks: Number(vm.ex.jit_stat(2)) + Number(vm.ex.jit_stat(3)),
  };
}

function best(runs) {
  return runs.reduce((a, b) => (b.ms < a.ms ? b : a));
}

// ---- user-mode workloads ----
const benchElf = new Uint8Array(
  await readFile(
    join(root, "guests/bench/target/riscv64gc-unknown-linux-musl/release/bench"),
  ),
);

for (const [key, argv] of [
  ["user-int+fp", ["bench", "fast"]],
]) {
  const runs = [];
  for (let r = 0; r < REPS; r++) {
    const vm = await RV64.create(wasmBytes);
    vm.onWrite = () => {};
    vm.loadElf(benchElf, argv);
    const t0 = performance.now();
    vm.runUser(2_000_000_000n);
    runs.push(stats(vm, t0, performance.now(), vm.userInsnCount()));
  }
  results[key] = best(runs);
}

// ---- system-mode workloads ----
const img = (f) => join(root, "web/images", f);
if (existsSync(img("bbl64.bin"))) {
  const [bios, kernel, disk] = await Promise.all(
    ["bbl64.bin", "kernel-riscv64.bin", "root-riscv64.bin"].map(async (f) =>
      new Uint8Array(await readFile(img(f))),
    ),
  );

  const bootRuns = [];
  const shellRuns = [];
  for (let r = 0; r < REPS; r++) {
    const vm = await RV64.create(wasmBytes);
    let out = "";
    vm.onWrite = (fd, b) => {
      out += new TextDecoder().decode(b);
    };
    vm.bootLinux({ bios, kernel, disk: disk.slice() });

    // boot to shell
    const t0 = performance.now();
    const insns0 = vm.sysInsnCount();
    for (let i = 0; i < 40000 && !out.includes("~ #"); i++) {
      vm.runSystem(5_000_000n);
    }
    const t1 = performance.now();
    if (!out.includes("~ #")) throw new Error("boot failed");
    bootRuns.push(stats(vm, t0, t1, vm.sysInsnCount() - insns0));

    // in-guest compute kernel: md5sum over zeros — a tight C loop in
    // busybox (regular control flow, memory-bound), the realistic
    // system-mode JIT target.
    out = "";
    vm.consoleInput(
      new TextEncoder().encode(
        "dd if=/dev/zero bs=1k count=4096 2>/dev/null | md5sum; echo LOOP-EOF\n",
      ),
    );
    const t2 = performance.now();
    const insns2 = vm.sysInsnCount();
    for (let i = 0; i < 60000 && !out.includes("LOOP-EOF"); i++) {
      vm.runSystem(5_000_000n);
    }
    const t3 = performance.now();
    if (!out.includes("LOOP-EOF")) throw new Error("compute kernel failed");
    shellRuns.push(stats(vm, t2, t3, vm.sysInsnCount() - insns2));
  }
  results["boot"] = best(bootRuns);
  results["sys-md5"] = best(shellRuns);
} else {
  console.error("(web/images missing — system workloads skipped)");
}

if (json) {
  console.log(JSON.stringify(results, null, 2));
} else {
  const pad = (s, n) => String(s).padStart(n);
  console.log(
    "workload      |     ms | Minsn/s | jit% | dispatches | blocks",
  );
  console.log(
    "--------------|--------|---------|------|------------|-------",
  );
  for (const [k, v] of Object.entries(results)) {
    console.log(
      `${k.padEnd(13)} | ${pad(v.ms, 6)} | ${pad(v.minsn_s, 7)} | ${pad(v.jit_pct, 4)} | ${pad(v.dispatches, 10)} | ${pad(v.blocks, 6)}`,
    );
  }
}
