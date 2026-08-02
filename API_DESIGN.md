# JavaScript API and boot design

Status: stable facade and direct Linux boot implemented. The fast-guest and
terminal/package work remains in progress. This document records the
public-API and boot decisions made during release preparation.

## Goals

- Make `riscv-virt` the normal full-system platform.
- Hide the raw Wasm ABI and execution-slice scheduler from ordinary embedders.
- Provide one lifecycle, console, networking, and event API across boot modes.
- Accept image URLs as well as already-loaded bytes and report download
  progress through typed events.
- Ship TypeScript declarations alongside the JavaScript module.
- Support booting a Linux kernel **without a caller-supplied or executed
  firmware image**. The direct path exists primarily to reduce time-to-kernel
  and time-to-shell.
- Retain explicit external-firmware boot for OpenSBI, firmware, and bootloader
  development.
- Keep the TinyEMU-compatible board for legacy images and regression testing
  without making it part of the normal product model.

## Separate platform from boot mode

A platform describes hardware: physical addresses, interrupt controller,
UART, virtio layout, reset device, and device tree. A boot mode describes how
software is placed and entered. They are independent concepts.

The supported internal platform identifiers are:

- `riscv-virt`: default and recommended; the QEMU-style RISC-V virtual board.
- `legacy-tinyemu`: compatibility board for the historical BBL/Linux images.

Normal callers should not need to choose a platform. `riscv-virt` is the
default. Legacy compatibility must be explicit.

The planned boot modes are:

1. `linux-direct`: rv64.js loads Linux, the initrd and DTB, enters the kernel in
   S-mode, and provides the required SBI services without executing a firmware
   image.
2. `firmware`: execute either the packaged default OpenSBI or an image supplied
   by the caller, then enter the following kernel/bootloader stage.
3. `bare-metal`: load an image at an explicit address and enter it in an
   explicit privilege mode, with no Linux or SBI promises.

“No firmware” in the Linux API means no firmware **image or firmware boot
stage**. rv64.js must still perform the platform setup and SBI responsibilities
that firmware normally owns. It must not be implemented as merely setting the
PC to a stock kernel and hoping it runs.

## Target API shape

The exact names can change while the first implementation is reviewed, but the
separation and lifecycle are intentional.

```ts
export type ImageSource =
  | Uint8Array
  | ArrayBuffer
  | Response
  | { url: string };

export type BootConfig =
  | {
      mode: "linux-direct";
      kernel: ImageSource;
      initrd?: ImageSource;
      disk?: ImageSource;
      cmdline?: string;
    }
  | {
      mode: "firmware";
      firmware?: "default" | ImageSource;
      kernel?: ImageSource;
      initrd?: ImageSource;
      disk?: ImageSource;
      cmdline?: string;
    }
  | {
      mode: "bare-metal";
      image: ImageSource;
      loadAddress: bigint;
      entry?: bigint;
      privilege?: "machine" | "supervisor";
    };

export type NetworkConfig =
  | { mode: "none" }
  | { mode: "fetch"; upgradeHttps?: boolean; relayURL?: string; mac?: Uint8Array }
  | { mode: "wsproxy"; url: string; protocols?: string | string[]; mac?: Uint8Array }
  | { mode: "wisp"; url: string; protocols?: string | string[]; mac?: Uint8Array }
  | { mode: "inbrowser"; channel?: string; mac?: Uint8Array }
  | { mode: "external"; mac?: Uint8Array };

const vm = await RV64.create({
  wasm: { url: "./rv64_wasm.wasm" },
  memoryMB: 512,
  boot: {
    mode: "linux-direct",
    kernel: { url: "./Image" },
    disk: { url: "./root.ext4" },
    cmdline: "console=ttyS0 root=/dev/vda rw",
  },
  network: { mode: "fetch" },
});

const unsubscribe = vm.on("console", bytes => terminal.write(bytes));
await vm.start();
vm.console.send(input);
await vm.stop();
unsubscribe();
await vm.destroy();
```

The primary lifecycle is `start`, `stop`, `reset`, `destroy`, and `running`.
The library owns instruction slicing and scheduling. Machine-specific
`runSystem`/`runVirtSystem`, machine-specific console methods, and raw Wasm
exports are not present on the public `RV64` class. Repository architecture,
differential, and performance harnesses explicitly use `RV64Debug`; it is not
an application compatibility API.

The event map should cover at least `ready`, `start`, `stop`, `error`,
`console`, `networkTransmit`, and `downloadProgress`. Listener registration
returns an unsubscribe function. The xterm.js integration belongs in a small
adapter or the demo, not in the emulator core.

Linux defaults to `fetch`: the guest receives a virtio NIC and reaches the
host-side request proxy at `http://10.0.2.2:8080`. `wsproxy` connects the NIC
to a websockproxy-compatible layer-2 relay, `wisp` maps guest TCP/UDP payloads
to WISP v1 streams, `inbrowser` provides a BroadcastChannel LAN, and `external`
exposes outbound frames through `networkTransmit` and accepts inbound frames through
`vm.network.receive()`, and `none` omits the NIC. The optional HTTP
`relayURL` is a request-level CORS fallback and is deliberately distinct from
the layer-2 WebSocket mode. `vm.network.proxyURL` reports the configured proxy
address without callers hardcoding it.

The public surface must not include raw Wasm exports, staging helpers, HTTP
implementation helpers, relay internals, or JIT experiment counters. Tests may
use a separately named unstable debug API.

## Direct Linux boot contract

The first direct implementation targets one hart on `riscv-virt`. It must:

- Validate and place a RISC-V Linux `Image` at its required alignment.
- Place an optional initrd without overlapping the kernel, DTB, or guest RAM
  reserved regions.
- Generate the same truthful `riscv-virt` DTB used by firmware boot.
- Enter Linux in S-mode with `a0 = hartid`, `a1 = dtb`, and `satp = 0`.
- Initialize privilege, delegation, interrupt, timer, and PMP state explicitly.
- Provide enough SBI for the supported kernel. The initial audited set is Base,
  TIME, IPI, RFENCE, HSM (single-hart behavior), and SRST; implement legacy or
  debug-console calls only when the kernel evidence requires them.
- Preserve correct WFI, timer, shutdown, console, virtio, and instruction-count
  behavior under both interpreter and JIT execution.
- Fail with a useful compatibility error for an unsupported kernel or SBI
  request rather than hanging silently.

External OpenSBI remains the conformance oracle for the direct path. Direct and
firmware boots must run the same kernel/initrd workload and reach identical
guest-visible readiness checks.

## Boot-speed evidence

Direct boot is a performance feature, so measure before and after each stage.
For the same Wasm build, browser engine, kernel, command line, RAM, and rootfs,
record at least:

- `create` to first firmware output (firmware mode only)
- `start` to first kernel output
- `start` to root filesystem mounted
- `start` to a fixed guest readiness marker and interactive prompt
- guest instructions retired at each marker
- image download time separately from execution time

Use alternating fresh-page runs with at least five valid repetitions. Report
the median and dispersion. A result below 10% is a tie for default-selection
purposes.

The initial Node harness is `tests/boot-profile.mjs`:

```sh
node tests/boot-profile.mjs fast --reps 5 --out target/boot-fast.json
node --max-old-space-size=2048 tests/boot-profile.mjs modern --reps 5 \
  --out target/boot-modern.json
```

It records asset loading separately, then reports Wasm creation, machine
assembly, milestone wall time, and retired instructions. Browser automation
must use the same schema so Node and browser results remain comparable.

### Initial Node baseline (2026-08-01)

Five fresh-instance repetitions on Node 26.5.0, an AMD Threadripper PRO
5975WX, and Wasm SHA-256 `22c45e31abf366c…` produced:

| Milestone/phase | Fast legacy guest | Modern `riscv-virt` guest |
|---|---:|---:|
| Asset reads (outside boot) | 5.0 ms | 330.4 ms |
| Wasm creation | 7.6 ms | 6.7 ms |
| Machine assembly/image copies | 57.9 ms | 620.0 ms |
| OpenSBI first output | n/a | 651.3 ms / 1.70 Minsns |
| OpenSBI complete/kernel entry | n/a | 729.0 ms / 5.90 Minsns |
| Kernel console banner | not observable | 10,533.5 ms / 501.48 Minsns |
| Root mounted | 999.6 ms / 35.38 Minsns | 11,774.5 ms / 555.48 Minsns |
| Guest ready | 1,275.3 ms / 42.03 Minsns | 12,706.5 ms / 593.13 Minsns |

OpenSBI execution after its first output takes a median 75.8 ms and 4.2
million instructions. From machine assembly completing to kernel entry is
about 109 ms total. The kernel then spends about 9.81 seconds and 495.6 million
instructions before its buffered banner becomes visible. Direct boot can
remove a real stage and remains required, but on this workload its theoretical
firmware-only saving is below the 10% materiality threshold. The dominant
speed project is the modern kernel's silent early initialization, followed by
machine assembly/copying and post-console guest initialization.

The raw local reports are generated under `target/` and are intentionally not
versioned; rerun the commands above for comparisons. PC sampling uses the
diagnostic `virtPc()` API to identify actual kernel entry rather than assuming
that delayed console output marks the handoff.

The no-firmware mode remains a required supported option even if its measured
gain is below 10%. It becomes the default only if it is at least as reliable as
the OpenSBI path and either materially faster or materially simpler for users.
If OpenSBI time is negligible, keep direct boot available but pursue the real
time-to-shell costs: kernel configuration, initramfs/rootfs policy, device
probing, guest init, JIT warm-up, and image delivery.

The first platform-specific kernel pass confirms that priority. Across five
fresh Node runs with the same OpenSBI, Debian root, command line, and Wasm, the
median readiness time fell from 12,706.5 ms to 8,919.8 ms (29.8%) and retired
instructions fell from 593.13 million to 372.99 million (37.1%). The kernel
entry-to-console phase fell from 9,806.4 ms to 6,531.3 ms. This first package
keeps the required disk, network, and 9p paths but disables SMP/NUMA, ACPI,
ftrace/function tracing, debug page-table validation, KFENCE, rude-task RCU,
huge pages, perf, and IPv6. The next
iteration replaces the inherited distro config with an explicit rv64.js-only
config so unused module and hardware selections are absent rather than merely
irrelevant at boot.

Direct boot now enters the same kernel at `0x80200000` in S-mode and provides
SBI 2.0 Base, TIME, IPI, RFENCE, HSM, and SRST services in the emulator. Five
fresh direct runs reached the identical Debian readiness marker with exactly
365.78 million instructions each. Median readiness was 8,536.7 ms versus
8,919.8 ms through OpenSBI (4.3% faster), and 365.78 million versus 372.99
million instructions (1.9% fewer). That confirms the expected sub-10% result:
direct boot remains a supported artifact/API simplification, while kernel and
guest configuration remain the important performance work.

## Implementation sequence

1. Add timestamped boot markers and a repeatable browser/Node comparison for
   external OpenSBI, separating downloads from execution.
2. Add `rv64.d.ts`, typed events, image sources, and lifecycle ownership.
3. Remove the former public calls immediately; migrate repository-only CPU and
   performance harnesses to an explicitly named low-level test binding.
4. Unify the two Rust system machines behind common run, console, networking,
   power, and instruction-count operations; expose one Wasm system ABI.
5. Make `riscv-virt` the default and keep `legacy-tinyemu` behind an explicit
   compatibility option.
6. Add packaged-default OpenSBI so callers can boot by specifying only kernel
   and rootfs inputs.
7. Implement and validate `linux-direct`, including the emulator-provided SBI
   boundary and differential tests against OpenSBI.
8. Compare direct and OpenSBI boot profiles and choose the default from the
   recorded evidence.
9. Build the fast BusyBox preset for `riscv-virt`; both public demos then use
   one platform and differ only in guest images and configuration.
10. Move the demo to xterm.js through the public console API and add end-to-end
    tests for both public presets.
11. Publish a real ESM package containing JavaScript, Wasm, declarations, and
    source maps once the compatibility period is complete.

## Compatibility policy

There is no pre-release JavaScript compatibility period. The former
`RV64.create(wasm)`, `bootLinux`, `bootVirtLinux`, caller-driven slice-running,
and machine-specific console methods were removed from `RV64` before the first
package release. `tests/public-api.mjs` is the executable contract and asserts
that those properties are absent. This lets the first release begin with the
intended API instead of publishing a transition layer.
