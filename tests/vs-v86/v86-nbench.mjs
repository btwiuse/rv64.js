// v86 side of nbench: boot Debian i386 bench initramfs with bench=nbench and
// echo nbench's own output (the self-timed per-kernel iterations/sec). Prints
// the raw report between NBENCH_BEGIN/END for the scorecard to parse.
import { readFile } from "node:fs/promises";
const { V86 } = await import("./src/main.js");
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
const e = new V86({
  bios: { url: "./bios/seabios.bin" }, vga_bios: { url: "./bios/vgabios.bin" },
  bzimage: { buffer: (await readFile(ARTIFACTS + "/vmlinuz-i386")).buffer },
  initrd: { buffer: (await readFile(ARTIFACTS + "/deb-i386-bench.cpio.gz")).buffer },
  cmdline: "rdinit=/init console=ttyS0 bench=nbench", autostart: true,
  memory_size: 1024 * 1024 * 1024, disable_jit: +process.env.DISABLE_JIT, log_level: 0,
});
let s = "";
console.log("NBENCH_BEGIN");
e.add_listener("serial0-output-byte", (b) => {
  const c = String.fromCharCode(b); if ((c < " " && c !== "\n") || c > "~") return;
  s += c; process.stdout.write(c);
  if (s.includes("Trademarks") || s.includes("RUN_DONE")) {
    console.log("\nNBENCH_END"); e.destroy(); process.exit(0);
  }
});
setTimeout(() => { console.log("\nNBENCH_END timeout"); process.exit(1); }, 600000);
