import { readFile } from "node:fs/promises";
const { V86 } = await import("./src/main.js");
const SC = process.env.SC;
const bin = new Uint8Array(await readFile(SC + "/xbench/rvbench_fs.i386"));
const jit = !+process.env.DISABLE_JIT;
const emulator = new V86({
    bios: { url: "./bios/seabios.bin" }, vga_bios: { url: "./bios/vgabios.bin" },
    bzimage: { url: "./images/buildroot-bzimage68.bin" },
    cmdline: ["console=ttyS0", "tsc=reliable"],
    autostart: true, memory_size: 128 * 1024 * 1024,
    filesystem: {},
    disable_jit: +process.env.DISABLE_JIT, log_level: 0,
});
let serial = "", tstart = null, launched = false;
emulator.add_listener("serial0-output-byte", (b) => {
    const c = String.fromCharCode(b); if (c < " " && c !== "\n" || c > "~") return;
    serial += c;
    if (!launched && serial.endsWith("~% ")) {
        launched = true;
        emulator.create_file("rvbench", bin).then(() => {
            emulator.serial0_send("cp /mnt/rvbench /tmp/r && chmod +x /tmp/r && /tmp/r\n");
        });
    }
    if (serial.includes("BENCH_START") && tstart === null) tstart = Date.now();
    if (serial.includes("BENCH_DONE")) {
        const chk = (serial.match(/checksum=0x[0-9a-f]+/) || ["?"])[0];
        console.log(`[v86 COMPUTE ${jit?"JIT":"INTERP"}] ${Date.now()-tstart}ms  ${chk}`);
        emulator.destroy(); process.exit(0);
    }
});
setTimeout(() => { console.log("[timeout] tail:", serial.slice(-300)); process.exit(1); }, 120000);
