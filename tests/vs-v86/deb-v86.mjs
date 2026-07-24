// v86 side of the apples-to-apples python benchmark: boot the Debian i386
// initramfs (mk-v86-debian.sh) and time `python3 /fib.py`. Runs inside the
// copy/v86 checkout (imports ./src/main.js). Env: SC, DISABLE_JIT.
import { readFile } from "node:fs/promises";
const { V86 } = await import("./src/main.js");
const SC = process.env.SC;
const jit = !+process.env.DISABLE_JIT;
const e = new V86({
  bios: { url: "./bios/seabios.bin" }, vga_bios: { url: "./bios/vgabios.bin" },
  bzimage: { buffer: (await readFile(SC + "/vmlinuz-i386")).buffer },
  initrd: { buffer: (await readFile(SC + "/deb-i386.cpio.gz")).buffer },
  cmdline: "rdinit=/init console=ttyS0", autostart: true,
  memory_size: 1024 * 1024 * 1024, disable_jit: +process.env.DISABLE_JIT, log_level: 0,
});
let s = "", ts = null;
e.add_listener("serial0-output-byte", (b) => {
  const c = String.fromCharCode(b); if (c < " " && c !== "\n" || c > "~") return; s += c;
  if (ts === null && s.includes("FIB_START")) ts = Date.now();
  if (s.includes("FIB_DONE")) {
    console.log(`RESULT ms=${Date.now() - ts} chk=${(s.match(/fib\(30\)=\s*\d+/) || ["?"])[0].replace(/\s/g, "")} ${jit ? "JIT" : "INTERP"}`);
    e.destroy(); process.exit(0);
  }
});
setTimeout(() => { console.log("RESULT ms= timeout"); process.exit(1); }, 400000);
