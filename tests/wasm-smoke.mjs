// Node smoke tests for the wasm build: user-mode ELF execution (with JIT),
// JIT module validity, and (if guest images are present) a full Linux boot.
// Run via tests/run-all.sh, or directly:
//   node tests/wasm-smoke.mjs
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { RV64, Stop } = await import(join(root, "web/rv64.js"));
const wasmBytes = await readFile(
  join(root, "target/wasm32-unknown-unknown/release/rv64_wasm.wasm"),
);

let failures = 0;
function check(name, ok, detail = "") {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " — " + detail : ""}`);
  if (!ok) failures++;
}

// ---- user-mode guests under JIT ----
const guests = [
  ["hello-std", ["h", "x"], 0, "sum of squares 1..10 = 385"],
  ["fpu-test", ["f"], 0, "--- 0 failures"],
  ["bench", ["bench", "fast"], 0, null],
];
for (const [name, argv, wantExit, wantOut] of guests) {
  const path = join(
    root,
    `guests/${name}/target/riscv64gc-unknown-linux-musl/release/${name}`,
  );
  if (!existsSync(path)) {
    console.log(`SKIP user-mode ${name} (guest not built)`);
    continue;
  }
  const vm = await RV64.create(wasmBytes);
  let out = "";
  vm.onWrite = (fd, b) => {
    out += new TextDecoder().decode(b);
  };
  vm.loadElf(new Uint8Array(await readFile(path)), argv);
  const stop = vm.runUser(2_000_000_000n);
  const ok =
    stop === Stop.EXITED &&
    vm.userExitCode() === wantExit &&
    (wantOut === null || out.includes(wantOut));
  check(`user-mode ${name}`, ok, `exit=${vm.userExitCode()} jit-blocks=${vm.jitBlocks ?? 0}`);
}

// ---- JIT emitter: every module from arbitrary offsets must instantiate ----
{
  const vm = await RV64.create(wasmBytes);
  const elfPath = join(
    root,
    "guests/bench/target/riscv64gc-unknown-linux-musl/release/bench",
  );
  if (existsSync(elfPath)) {
    const elf = new Uint8Array(await readFile(elfPath));
    let emitted = 0,
      bad = 0;
    for (let off = 0; off + 4 < Math.min(elf.length, 4096); off += 2) {
      const ptr = vm.ex.staging_alloc(elf.length);
      new Uint8Array(vm.ex.memory.buffer, ptr, elf.length).set(elf);
      if (vm.ex.jit_translate(0n, BigInt(off)) > 0) {
        emitted++;
        const mod = new Uint8Array(
          vm.ex.memory.buffer,
          vm.ex.jit_out_ptr(),
          vm.ex.jit_out_len(),
        ).slice();
        try {
          new WebAssembly.Module(mod); // instantiation-level validation
        } catch {
          bad++;
        }
      }
    }
    check("jit emitter validity", emitted > 50 && bad === 0, `${emitted} modules, ${bad} invalid`);
  } else {
    console.log("SKIP jit emitter validity (bench guest not built)");
  }
}

// ---- full-system Linux boot (needs web/get-images.sh) ----
{
  const img = (f) => join(root, "web/images", f);
  if (!existsSync(img("bbl64.bin"))) {
    console.log("SKIP linux boot (run web/get-images.sh)");
  } else {
    const vm = await RV64.create(wasmBytes);
    let out = "";
    vm.onWrite = (fd, b) => {
      out += new TextDecoder().decode(b);
    };
    vm.bootLinux({
      bios: new Uint8Array(await readFile(img("bbl64.bin"))),
      kernel: new Uint8Array(await readFile(img("kernel-riscv64.bin"))),
      disk: new Uint8Array(await readFile(img("root-riscv64.bin"))),
    });
    for (let i = 0; i < 20000 && !out.includes("~ #"); i++) {
      vm.runSystem(10_000_000n);
    }
    const gotShell = out.includes("~ #");
    let cmdOk = false;
    if (gotShell) {
      vm.consoleInput(new TextEncoder().encode("echo smoke-$((6*7))\n"));
      for (let i = 0; i < 400 && !out.includes("smoke-42"); i++) {
        vm.runSystem(10_000_000n);
      }
      cmdOk = out.includes("smoke-42");
    }
    check("linux boot (wasm)", gotShell && cmdOk, `jit-blocks=${vm.jitBlocks ?? 0}`);
  }
}

console.log(failures === 0 ? "WASM SMOKE: ALL PASS" : `WASM SMOKE: ${failures} FAILURES`);
process.exit(failures === 0 ? 0 : 1);
