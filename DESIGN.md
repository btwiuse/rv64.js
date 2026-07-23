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

1. ✅ **rv64i interpreter**: decoder, ALU, branches, loads/stores,
   ecall/ebreak; unit tests; wasm + JS demo run end-to-end.
2. ✅ **User-mode Linux** (`rv64-linux`, `rv64-run`): ELF loader (ET_EXEC +
   static-pie), Linux ABI stack/auxv, ~45-syscall shim. Runs no_std and
   full-Rust-std musl static binaries, native and in the browser.
3. ✅ **Extensions**: M, A (LR/SC + AMOs), F/D (host-float; fflags
   approximated — softfloat pass still open, TinyEMU's softfp.c is the
   model), C (16→32 expander in front of the one decoder), Zicsr.
4. ✅ **Privileged architecture**: M/S/U, full CSR file, trap delegation,
   MRET/SRET/WFI/SFENCE.VMA, sv39/sv48 MMU with per-access TLBs, hardware
   A/D, SUM/MXR/MPRV. Interrupt lines are sampled live from the bus each
   step (`Bus::irq_lines`) — level-triggered like real hardware, which is
   what makes timer reprogramming race-free.
5. ✅ **Devices + boot** (`rv64-system`): CLINT, TinyEMU-minimal PLIC, HTIF,
   virtio-mmio v2 (console + blk), DTB builder, BBL trampoline. **Boots
   TinyEMU's stock Linux 4.15 + buildroot image to an interactive shell**,
   natively (~50 Minsn/s interpreted) and in the browser (web/system.html,
   ~0.9 s to shell in Node). Not yet ported from TinyEMU: virtio-net, 9p.
6. 🚧 **JIT** (`rv64-jit`): v1 pipeline proven end-to-end — Rust translates
   basic blocks (ALU/branches/JAL/JALR, compressed included) into wasm
   modules against a shared register-file memory; JS instantiates and
   dispatches by pc; unsupported instructions end the block and fall back
   to the interpreter (the v86 tiering seam). Still open: dispatcher
   integration into `sys_run` hot loop, guest load/store translation with
   inline TLB, block invalidation on SFENCE.VMA/self-modifying code,
   block chaining, hotness tiering.

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
