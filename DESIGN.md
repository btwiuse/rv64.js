# rv64.js — Design

A RISC-V (RV64) emulator for the browser: **TinyEMU's scope, v86's architecture.**

- **What to build** comes from [TinyEMU](https://bellard.org/tinyemu/) (Fabrice
  Bellard, MIT — vendored in `reference/tinyemu/`): it is the existence proof
  that a small rv64 machine boots mainline Linux in a browser, and it defines
  the exact ISA subset, CSRs, MMU mode, interrupt controllers, and virtio
  devices that suffice.
- **How to structure it** comes from [copy/v86](https://github.com/copy/v86):
  CPU core in Rust compiled to `wasm32-unknown-unknown`, devices and browser
  glue in plain JS, a flat `extern "C"` export surface (no wasm-bindgen), and
  eventually a JIT that emits wasm modules for hot code.

Why RISC-V over arm64/x86: fixed-scope clean ISA, official conformance tests,
TinyEMU as both blueprint and differential-testing oracle, and known-good
guest images to boot — so we only ever debug the emulator, not the guest.

## Layout

```
Cargo.toml              workspace
crates/rv64-core/       portable CPU core (no_std, no I/O)
crates/rv64-wasm/       wasm export surface: extern "C" over linear memory
web/                    JS loader (rv64.js) + demo page
reference/tinyemu/      vendored TinyEMU source (MIT) — spec map + native oracle
```

## Core architecture

The one load-bearing decision: **`Cpu` is generic over a `Bus` trait.**

```
             ┌────────────────────────────┐
             │  Cpu (decode + execute)    │   identical in both modes
             └─────────────┬──────────────┘
                     trait Bus
              ┌────────────┴─────────────┐
   FlatMemory (user-mode)      SystemBus (full-system, later)
   flat buffer, bounds check   sv39/sv48 walk + TLB → RAM | MMIO
```

- `Bus` (crates/rv64-core/src/bus.rs): read/write 8–64 bits + `fetch32`.
  Addresses are guest-virtual; the impl decides what translation means.
- `Exception` (exception.rs): full privileged-spec cause enum from day one,
  so trap plumbing never changes shape when full-system mode lands.
- `StopReason`: the interpreter returns control to the host on
  ecall/ebreak/trap/budget. In user-mode the host services syscalls; in
  full-system mode ecall/traps will instead vector into the guest kernel.

This is the same seam QEMU uses to share its frontend between `linux-user`
and system emulation. Nothing written for phase 1 is thrown away later.

## wasm surface (v86-style)

`rv64-wasm` exports plain `extern "C"` functions (`init`, `run`, `get_reg`,
`mem_ptr`, …). Guest RAM lives **inside wasm linear memory**; JS views it via
`new Uint8Array(memory.buffer, mem_ptr(), mem_size())`, so loading a kernel
or ELF is one typed-array copy. One emulator per wasm instance (v86's model);
multiple VMs = multiple instantiations.

`web/rv64.js` is the loader class — no bundler, works in browsers and Node.

## Phases

1. **rv64i interpreter** *(done — this scaffold)*: decoder, ALU, branches,
   loads/stores, ecall/ebreak; unit tests; wasm + JS demo run end-to-end.
2. **User-mode Linux**: ELF loader, stack/auxv, `SVC→syscall` shim
   (write/exit/brk/mmap/…) in JS or Rust-native harness → run static
   hello-world, then busybox. Add **riscv-tests** (official ISA suite) and a
   differential harness against the native `reference/tinyemu/temu` build
   and/or QEMU.
3. **Extensions**: M (mul/div), A (atomics), F/D (reuse ideas from TinyEMU's
   softfp), C (compressed — implemented as a 16→32-bit expander in front of
   the one decoder, not a second execute path), Zicsr/Zifencei.
4. **Privileged architecture**: M/S/U modes, CSRs, sv39 (then sv48) MMU with
   TLB behind a `SystemBus`, CLINT (timer/IPI), PLIC (external interrupts).
   Map scope directly from `reference/tinyemu/riscv_machine.c`.
5. **Devices + boot**: virtio-mmio (console, blk, net, 9p — TinyEMU's set,
   `virtio.c`), device tree generation, boot the same kernel/rootfs images
   TinyEMU ships → Linux shell in the browser.
6. **JIT** (v86's endgame): translate hot basic blocks to wasm modules at
   runtime. RISC-V's decode makes this the cleanest possible version of it.

## Testing strategy

- **Unit tests** in rv64-core per instruction group (running since phase 1).
- **riscv-tests / RISCOF** conformance suites from phase 2 — free correctness
  the x86/arm64 world doesn't get.
- **Differential testing**: run the same guest on `temu` (built natively from
  `reference/tinyemu/`) and rv64.js, diff architectural state. TinyEMU is the
  oracle; QEMU as a second opinion.
- Known-good guest images from bellard.org/jslinux so guest bugs are ruled out.

## Licensing

- rv64.js: MIT.
- `reference/tinyemu/`: Fabrice Bellard's TinyEMU, MIT — vendored verbatim as
  spec-map and oracle. We port ideas/scope, not code; anything translated
  keeps attribution. Note: **jslinux.org's website JS/wasm bundle is NOT open
  source** — only the TinyEMU C source is. Never copy from the jslinux bundle.

## Naming note

The directory is `~/src/arm64.js` for historical reasons (the project began
as an arm64 emulator idea); the project itself is **rv64.js**.
