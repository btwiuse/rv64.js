const { V86 } = await import("./src/main.js");
const jit = !process.env.DISABLE_JIT;
const emulator = new V86({
    bios: { url: "./bios/seabios.bin" },
    vga_bios: { url: "./bios/vgabios.bin" },
    cdrom: { url: "./images/linux4.iso" },
    autostart: true,
    memory_size: 64 * 1024 * 1024,
    disable_jit: +process.env.DISABLE_JIT,
    log_level: 0,
});
let t0, serial = "";
emulator.bus.register("emulator-started", () => { t0 = Date.now(); });
emulator.add_listener("serial0-output-byte", (byte) => {
    const c = String.fromCharCode(byte);
    if (c < " " && c !== "\n" || c > "~") return;
    process.stdout.write(c);
    serial += c;
    if (serial.endsWith("~% ") || serial.endsWith("/root% ") || serial.endsWith("# ") || serial.endsWith("~ # ")) {
        console.log(`\n[v86 BOOT ${jit?"JIT":"INTERP"}] ${Date.now()-t0}ms`);
        emulator.destroy(); process.exit(0);
    }
});
setTimeout(() => { console.log("\n[timeout]"); process.exit(1); }, 60000);
