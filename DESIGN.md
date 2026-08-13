# rv64.js — Design

A RISC-V (RV64) emulator for the browser: a scalar `rv64gc` CPU, a modern
Virt-class Linux machine, and a WebAssembly-only dynamic compiler.

- [TinyEMU](https://bellard.org/tinyemu/) (Fabrice Bellard, MIT — vendored in
  `reference/tinyemu/`) supplied the project's original scope and differential
  oracle. Its BBL/Linux 4.15 machine remains a compatibility path, not the
  current kernel, firmware, or product target.
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
crates/rv64-dbt/        clean-room RV64-to-Wasm dynamic compiler
crates/rv64-system/     legacy and modern Virt full-system machines
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
   FlatMemory (user-mode)      Legacy/Virt buses (full-system)
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
3. ✅ **Extensions**: M, A (LR/SC + AMOs), F/D (softfloat — softfp.rs,
   ported from TinyEMU's softfp.c: exact IEEE 754 fflags, all five
   rounding modes; passes all rv64uf/ud riscv-tests), C (16→32 expander
   in front of the one decoder), Zicsr.
4. ✅ **Privileged architecture**: M/S/U, full CSR file, trap delegation,
   MRET/SRET/WFI/SFENCE.VMA, sv39/sv48 MMU with per-access TLBs, hardware
   A/D, SUM/MXR/MPRV. Interrupt lines are sampled live from the bus each
   step (`Bus::irq_lines`) — level-triggered like real hardware, which is
   what makes timer reprogramming race-free.
   - Full-system scalar interpreter loads/stores can consume the exact fused
     JIT-TLB row already published by the authoritative translation path. A
     hit requires the complete virtual-page, permission-context, and row-index
     proof and uses its live RAM pointer directly; a miss follows the unchanged
     standard TLB and bus path. Mapping changes, backing invalidation, and
     generated-code-page marking clear the capability before it can be reused.
5. ✅ **Devices + boot** (`rv64-system`): the compatibility machine retains
   HTIF/BBL/TinyEMU images. The supported `VirtMachine` provides CLINT, PLIC,
   ns16550, RTC, virtio-mmio block/console/9p/net, and a generated DTB. It boots
   the slim Linux 6.12/Alpine image either directly in S-mode through host SBI
   or through OpenSBI `fw_dynamic` in M-mode.
6. ✅ **Dynamic compiler** (`rv64-dbt`): a clean-room typed-SSA RV64-to-Wasm
   compiler integrated into user, legacy-system, and Virt run loops. T1 emits
   bounded traces and fuel-metered loops; T2 emits register-resident
   multi-entry batches and sparse multi-page regions with bounded indirect
   specialization. Flat and full-system memory, scalar M/A/F/D, exact side
   exits, reservations, code-page generations, and SMC invalidation are
   compiled and differentially tested. Large regions compile asynchronously
   and are revalidated before publication.
   - Guest faults use explicit state materialization and interpreter re-entry;
     a host Wasm trap is never the guest exception path.
   - Full-system loads/stores probe typed translation rows inline, optionally
     refill through Wasm-to-Wasm helpers, and precisely exit for page crossing,
     MMIO, permissions, or compiled-code stores.
   - Related entries remain inside one Wasm function through structured SCC
     lowering with localized dispatch only where required. On engines with
     Wasm tail calls, one table-owning trampoline connects external generated
     successors without making every generated module import the table. Lazy
     state and a small translation proof cache remain measured opt-in variants.

All six product phases are complete. The JIT rewrite's architecture, gates,
policy data, and reproducible commands are recorded in
[`docs/jit-rewrite/`](docs/jit-rewrite/). The portable default is eager
register-resident state with structured SCC lowering, a two-page/512-leader
maximum, sampled control-density admission, and feature-tested frame-free tail
chaining. Generated TLB caching and recursive chaining remain measured opt-in
variants; no browser-brand policy exists.

**virtio-9p** (`p9.rs`/`p9fs.rs`) exports a
host directory or an in-memory tree, and **virtio-net** (`virtio.rs`) carries
frames either to a WebSocket relay (`ws.rs`) or to an **in-browser HTTP proxy**
(`netstack.rs` + `httpproxy.rs`) whose egress is `fetch()` — the only design that
reaches the network from a page with no external infrastructure. An optional,
separate request-level WebSocket relay (`web/http-relay.mjs`) handles
CORS-blocked origins without changing the guest-facing proxy: fetch is tried
first, safe failures fall back, and that origin is then routed directly.
CONNECT is terminated with a per-host certificate signed by an ephemeral
in-process CA. Proxy-enabled machines expose only that CA's public DER file on
the fixed virtio-9p tag `rv64-proxy`, allowing a guest boot policy to install
the exact authority for that run without putting signing keys in guest memory.
Native egress uses rustls and the host trust store for its separate upstream
HTTPS connection; browser egress gets the equivalent behavior from `fetch()`.

## Testing strategy

- **Unit tests** in rv64-core per instruction group (running since phase 1).
- **riscv-tests and riscv-arch-test** conformance, plus per-writeback Spike
  lockstep and QEMU user-mode output differentials.
- **Interpreter/JIT differential testing** over directed and randomized full
  architectural state, memory, faults, Sv39, FP, atomics, and SMC.
- **Generated-Wasm gates** in Node/V8, Chrome/V8, Edge/V8,
  Firefox/SpiderMonkey, and standalone Wasmtime, with exact module hashes and
  lifecycle measurements.
- **Modern full-system gates** for direct-SBI Linux, OpenSBI/Linux, in-guest FP
  context switching and atomics, and a separate marker-driven Virt smoke boot.

## Licensing

- rv64.js: MIT.
- `reference/tinyemu/`: Fabrice Bellard's TinyEMU, MIT — vendored verbatim as
  spec-map and oracle. We port ideas/scope, not code; anything translated
  keeps attribution. Note: **jslinux.org's website JS/wasm bundle is NOT open
  source** — only the TinyEMU C source is. Never copy from the jslinux bundle.

## Naming note

The directory is `~/src/arm64.js` for historical reasons (the project began
as an arm64 emulator idea); the project itself is **rv64.js**.
