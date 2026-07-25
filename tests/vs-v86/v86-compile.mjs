// v86 side of the compile benchmark: boot the Debian i386 bench initramfs with
// bench=tcc, time `tcc -c /w.c` (RUN_START->RUN_DONE), print RESULT ms + md5.
import { readFile } from "node:fs/promises";
const { V86 } = await import("./src/main.js");
const ARTIFACTS = process.env.ARTIFACTS || process.env.SC;
const jit = !+process.env.DISABLE_JIT;
const e = new V86({
  bios: { url: "./bios/seabios.bin" }, vga_bios: { url: "./bios/vgabios.bin" },
  bzimage: { buffer: (await readFile(ARTIFACTS + "/vmlinuz-i386")).buffer },
  initrd: { buffer: (await readFile(ARTIFACTS + "/deb-i386-bench.cpio.gz")).buffer },
  cmdline: "rdinit=/init console=ttyS0 bench=tcc", autostart: true,
  memory_size: 1024 * 1024 * 1024, disable_jit: +process.env.DISABLE_JIT, log_level: 0,
});
// Timed window is RUN_START -> RUN_DONE (tcc only); md5sum prints AFTER the
// window and is captured for correctness before reporting.
let s = "", ts = null, td = null;
e.add_listener("serial0-output-byte", (b) => {
  const c = String.fromCharCode(b); if ((c < " " && c !== "\n") || c > "~") return; s += c;
  if (ts === null && s.includes("RUN_START")) ts = Date.now();
  if (td === null && s.includes("RUN_DONE")) {
    td = Date.now();
    setTimeout(() => { // give md5sum up to 20s past the timed window
      const md5 = (s.match(/RUN_DONE[\s\S]*?([0-9a-f]{32})/) || [, "?"])[1];
      console.log(`RESULT ms=${td - ts} md5=${md5} ${jit ? "JIT" : "INTERP"}`);
      e.destroy(); process.exit(0);
    }, 20000);
  }
});
setTimeout(() => { console.log("RESULT ms= timeout"); process.exit(1); }, 400000);
