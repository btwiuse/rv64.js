// v86 boot-time benchmark: boot buildroot i686 Linux to a shell prompt,
// report wall-clock ms. Mirrors v86's linux-boot benchmark but on the same
// buildroot image compare-sys.mjs uses, so it's comparable to our boot.
import { readFile } from "node:fs/promises";
const { V86 } = await import("./src/main.js");
const jit = !+process.env.DISABLE_JIT;
const t0 = Date.now();
const emulator = new V86({
    bios: { url: "./bios/seabios.bin" }, vga_bios: { url: "./bios/vgabios.bin" },
    bzimage: { url: "./images/buildroot-bzimage68.bin" },
    cmdline: ["console=ttyS0", "tsc=reliable"],
    autostart: true, memory_size: 128 * 1024 * 1024,
    disable_jit: +process.env.DISABLE_JIT, log_level: 0,
});
let serial = "", done = false;
emulator.add_listener("serial0-output-byte", (b) => {
    const c = String.fromCharCode(b); if (c < " " && c !== "\n" || c > "~") return;
    serial += c;
    if (!done && serial.endsWith("~% ")) {
        done = true;
        console.log(`RESULT ms=${Date.now() - t0} ${jit ? "JIT" : "INTERP"}`);
        emulator.destroy(); process.exit(0);
    }
});
setTimeout(() => { console.log("[timeout] tail:", serial.slice(-200)); process.exit(1); }, 120000);
