// arch-python (v86's python fib(30) benchmark) on riscv64: boot a Debian
// riscv64 rootfs (assembled by mk-debian-rootfs.sh) under our JIT machine and
// time `python3 /fib.py`, JIT vs interpreter, with the wall-clock time source.
//   ARTIFACTS=<dir-with-deb-rootfs.ext4> node tests/vs-v86/deb-python.mjs
import { readFile } from "node:fs/promises";
const root = "/home/darren/src/arm64.js";
const { RV64 } = await import(root + "/web/rv64.js");
const wasm = await readFile(root + "/target/wasm32-unknown-unknown/release/rv64_wasm.wasm");
const img = f => readFile(f).then(b => new Uint8Array(b));
const [bios,kernel,disk] = await Promise.all([
  img(root+"/web/images/bbl64.bin"), img(root+"/web/images/kernel-riscv64.bin"), img(process.env.ARTIFACTS+"/deb-rootfs.ext4")]);
const enc = new TextEncoder();
async function run(jit){
  const vm = await RV64.create(wasm); vm.ex.jit_set_enabled(jit?1:0); vm.ex.sys_set_wallclock(1);
  let out=""; vm.onWrite=(fd,b)=> out+=new TextDecoder().decode(b);
  vm.bootLinux({ bios, kernel, disk: disk.slice(), cmdline:"console=hvc0 root=/dev/vda rw init=/binit.sh", ramMB:512 });
  for (let i=0;i<400000 && !out.includes("BENCH_READY");i++) vm.runSystem(3_000_000n);
  out="";
  vm.consoleInput(enc.encode("/usr/bin/python3 /fib.py\n"));
  const t=performance.now(); let ts=null,td=null;
  for (let i=0;i<4_000_000;i++){ vm.runSystem(4_000_000n);
    if(ts===null && out.includes("FIB_START")) ts=performance.now();
    if(out.includes("FIB_DONE")){ td=performance.now(); break; }
    if(performance.now()-t>180000) break; }
  return {full: td?td-t:null, fib: ts&&td?td-ts:null, out};
}
for (const jit of [false,true]){
  const r=await run(jit);
  console.log(`${jit?"JIT   ":"INTERP"}  full(cmd->done)=${r.full?r.full.toFixed(0):"?"}ms  fib-only=${r.fib?r.fib.toFixed(0):"?"}ms  | ${(r.out.match(/fib\(30\)=\s*\d+/)||["?"])[0]}`);
}
