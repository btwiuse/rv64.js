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
3. ✅ **Extensions**: M, A (LR/SC + AMOs), F/D (softfloat — softfp.rs,
   ported from TinyEMU's softfp.c: exact IEEE 754 fflags, all five
   rounding modes; passes all rv64uf/ud riscv-tests), C (16→32 expander
   in front of the one decoder), Zicsr.
4. ✅ **Privileged architecture**: M/S/U, full CSR file, trap delegation,
   MRET/SRET/WFI/SFENCE.VMA, sv39/sv48 MMU with per-access TLBs, hardware
   A/D, SUM/MXR/MPRV. Interrupt lines are sampled live from the bus each
   step (`Bus::irq_lines`) — level-triggered like real hardware, which is
   what makes timer reprogramming race-free.
5. ✅ **Devices + boot** (`rv64-system`): CLINT, TinyEMU-minimal PLIC, HTIF,
   virtio-mmio v2 (console + blk + 9p + net), DTB builder, BBL trampoline.
   **Boots TinyEMU's stock Linux 4.15 + buildroot image to an interactive
   shell**, natively (~50 Minsn/s interpreted) and in the browser
   (web/system.html, ~0.9 s to shell in Node). TinyEMU's device set is now fully
   covered; 9p and net landed later as roadmap items 7 and 8.
6. ✅ **JIT** (`rv64-jit`): integrated and live in both run loops. Hot pcs
   (threshold 16 at dispatch points) are translated by the Rust core into
   wasm modules; the JS host instantiates each module and registers its
   function in the core's exported function table; the core dispatches via
   `call_indirect`, chaining up to 64 blocks before returning to the
   interpreter (bounds interrupt latency). Superblock formation follows
   forward plain jumps. Verified: user-mode guests bit-identical under JIT
   (~3x on hot ALU loops vs interpreted wasm), and Linux boots to a working
   shell with system blocks active.
   - **User-mode blocks** include direct guest loads/stores
     (bounds-checked against flat RAM — a wasm trap is the fatal-fault
     path). Invalidation: `riscv_flush_icache` syscall (the architectural
     code-change contract) and fresh `user_load` drop all blocks.
   - **Full-system blocks** are ALU/branch-only, keyed by virtual pc with
     the physical address re-verified through the TLB on every dispatch
     (satp/mapping changes miss safely); guest memory ops end blocks and
     run through the MMU interpreter. Invalidation: SystemBus tracks a
     bitset of compiled code pages — any store to one drops all blocks.
   - Deliberate design point, not a gap: memory ops inside full-system
     blocks (inline-TLB loads/stores) are the next optimization tier,
     exactly as v86 evolved; correctness never depends on it.

All six phases are complete. F/D is softfloat (exact fflags, all rounding
modes) with a native-FP fast path in front of it: FADD/FSUB/FMUL/FDIV run
as host FP instructions (wasm f32/f64 ops in the browser) when rm=RNE and
fflags.NX is already sticky-set — conditions under which no new flag
information is possible — falling back to softfp otherwise. ~2x on
FP-heavy code in wasm, verified by 600k-iteration differential fuzz.

The post-1.0 roadmap lives in [ROADMAP.md](../ROADMAP.md) (perf: inline-TLB
JIT memory ops, block chaining, FP-in-blocks; validation: RISCOF, Spike
lockstep; features: snapshots). virtio-net/9p were intentionally descoped from
phase 5 — the boot target was console + blk, which is what "Linux shell in the
browser" requires. Both landed later: **virtio-9p** (`p9.rs`/`p9fs.rs`) exports a
host directory or an in-memory tree, and **virtio-net** (`virtio.rs`) carries
frames either to a WebSocket relay (`ws.rs`) or to an **in-browser HTTP proxy**
(`netstack.rs` + `httpproxy.rs`) whose egress is `fetch()` — the only design that
reaches the network from a page with no external infrastructure.

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
