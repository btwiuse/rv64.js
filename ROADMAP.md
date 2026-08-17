# rv64.js roadmap

All six original phases are complete (see DESIGN.md): rv64gcv + privileged
architecture, boots Linux natively and in the browser, JIT live in both run
loops, softfloat F/D with native fast path, riscv-tests 134/134, validated
against Spike/QEMU/TinyEMU. This file tracks what comes next, in rough
priority order. Check items off as they land.

## Performance (the JIT's next tiers)

**Milestone completed 2026-08-16:** the frozen RV64GCV and scalar JIT
scorecards are both 13/13 win-or-match against pinned copy/v86, and the
protected WANIX browser gate passes. RVV 1.0 is supported by the interpreter
and architecture-general JIT lowering. The exact result and release matrix are
recorded in
[`RV64GCV_JIT_13_OF_13_RESULT.md`](docs/jit-rewrite/RV64GCV_JIT_13_OF_13_RESULT.md).
Items 2b and 4 below remain deferred research directions rather than required
work for this milestone.

- [x] **1. Inline-TLB memory ops in full-system JIT blocks** *(done; the
  system-mode win)* — system JIT blocks translate guest loads/stores inline
  (TLB probe → direct RAM on hit, bail to interpreter on miss/MMIO/page-
  cross/store-to-compiled-page), with per-page invalidation. Combined with
  item 2's cheap dispatch and slice=256/threshold=64, this delivers
  **2.8-3.2x on real in-guest compute** (tests/bench-sys.mjs). It looked
  neutral for a while only because the benchmarks were unrepresentative and
  the harness had an echo bug — see the item-2b CORRECTION and
  tests/BASELINE.md.

- [x] **2. Cheap dispatch** *(done 2026-07-22)* — direct-mapped dispatch
  array (replaces HashMap+SipHash per block) + `cpu.jit_flush_gen`
  (drops the per-dispatch pa-verify TLB walk; flush only on satp/SFENCE).
  +6% on user-int+fp; the `jit_set_enabled` diagnostic shows user JIT is
  1.56× over the wasm interpreter. Dispatch is no longer the bottleneck.

- [ ] **2b. Fused JIT software-TLB** *(deferred)* — a dedicated JIT-TLB
  encoding RAM+writable+not-compiled to shrink inline memory ops from ~15 to
  ~4 wasm ops. Attempted twice and reverted (interpreter-fill tax, then
  one-shot-boot dispatch overhead; full post-mortem in git history). Item 1's
  inline TLB already delivers the system win; superseded in priority by the
  register-locals work below.

### Closing the JIT gap to v86 (measured plan, 2026-07-23)

`tests/vs-v86/` benchmarks us head-to-head against copy/v86 on the same
machine. Result: **our interpreter is a peer to v86's, but our JIT is ~11×
(mixed) to ~18× (pure ALU) slower than v86's.** Deep-dive into both codebases
found the gap is 100% implementation maturity — and riscv64 is a *better* JIT
target than x86 (no condition flags → we skip v86's largest subsystem, its
lazy-EFLAGS machinery; fixed-width decode; clean IEEE F/D vs x87). v86 wins
via three techniques; below is our phased plan to adopt them, each measured
with `tests/vs-v86` (baseline vs v86: ALU 50.5s vs 2.9s; mixed 17.1s vs 1.5s).

- [x] **3a. Phase 1 — registers in wasm locals** *(done 2026-07-23)* — GPRs a
  block touches live in `i64` wasm locals (`scan_regs` finds them; prologue
  loads, `push_reg`→`local.get`/`store_post`→`local.set`, every exit and bail
  flushes dirty locals → state). Standalone ~neutral by design (prologue/
  epilogue ran per dispatch); the multiplier is Phase 3.

- [x] **3c. Phase 3 — compile self-loops as one wasm function** *(done
  2026-07-23)* — `detect_self_loop` spots a straight-line ALU body ending in a
  branch back to start_pc and compiles it as one wasm `loop` (ITER counter +
  LOOP_CAP safety yield; dynamic retired count via RETIRED_CELL). Registers
  stay in locals across all iterations; the back-edge is a wasm `br_if`, no
  dispatch. V8 then compiles the loop near-native. **Phase 1+3 result: pure
  ALU went from 17.5× SLOWER than v86 to 0.5× — 2× FASTER — at 100% coverage,
  62× over our own interpreter.** Measured by `tests/vs-v86/bench-jit*.mjs`.

- [x] **3b. Phase 2 — FP arithmetic + FMV inside JIT blocks** *(done
  2026-07-23)* — FADD/FSUB/FMUL/FDIV.D and FMV.D.X/FMV.X.D emitted as inline
  wasm `f64` ops under a runtime eligibility (rm==RNE, sticky-NX) + result-
  normal guard, bailing to softfloat otherwise (FP regs stay in memory).
  **Straight-line FP loop: 0%→73% coverage, 3.0× over interpreter.** But the
  realistic mixed workload barely moves (still ~10× v86) — see below.

- [x] **3d. Remaining v86 gap — general loop-CFG + FP-in-locals** *(done
  2026-07-23)* — three sub-steps landed: **3d-1** FP compares (FLE/FLT/FEQ.D →
  GPR) emitted inline under a NaN/inf guard; **3d-2** `detect_structured_loop`
  generalises Phase 3's straight-line self-loop to a loop WITH internal control
  flow — a body of ALU/FP/user-mem ops plus nested forward if-then branches
  (negated-cond wasm `if`, closed at its target via a `pending_ifs` stack) and
  a back-edge `br_if` — compiled into ONE wasm `loop` (v86's `control_flow.rs`
  structuring, minus the SCC machinery we don't yet need); **3d-3** FP registers
  cached in `i64` wasm locals (Phase 1 for FP), removing per-iteration FP memory
  traffic. **Results:** the branchy FP loop went 0%→100% coverage; pure FP loops
  34-36× over the interpreter (was 3×); the realistic mixed benchmark 15%→59%
  coverage, ~11×→~6.6× v86. The residual mixed gap is the two documented gaps
  below, not the loop-CFG or FP machinery.

- [x] **3e. Full mixed-benchmark coverage — FP load/store + nested loops**
  *(done 2026-07-23)* — closed both structural gaps: **3e-1** inline FLD/FSD
  (double FP load/store, raw 8-byte bit-exact copies); **3e-2** a full
  structured-CFG compiler (`loop_region` detect+validate, `translate_loop`
  emit) that generalises 3d-2's single self-loop to **properly-nested natural
  loops plus forward if-then and forward loop-exit (break)** — nested wasm
  `block`+`loop` pairs with a scope stack computing `br` depths, exact
  retired-instruction accounting. The triple-nested insertion sort (a `break`
  out of the inner `while`) now compiles as one wasm function. **Mixed:
  59%→100% coverage, ~11×→31× over our interpreter, and from ~11× SLOWER than
  v86 to 2.5× FASTER.** Both benchmarks now beat v86 at 100% coverage (ALU 2×,
  mixed 2.5× faster). jit==interp checksums identical; Linux still boots under
  JIT. This closes the v86 performance gap that motivated items 3a–3e —
  **in user mode**. See 3f.

- [x] **3f. Port the 3a–3e JIT wins to SYSTEM mode** *(done 2026-07-23 — the
  real v86 gap: v86 has no user mode, so this is the only apples-to-apples
  comparison)* — **3f-1** enabled `loop_region`/`translate_loop` for system
  blocks. The one subtlety vs user mode: inline-TLB memory ops can *bail*
  mid-iteration (miss/MMIO/store-to-compiled-page), and a loop running millions
  of iterations must report its true retired count on bail or the kernel's
  insn_count-derived clock stalls — added `Ctx::retired_local` so a loop's bail
  stores the live `ITER` accumulator (basic blocks keep the static constant).
  **3f-2** set the system layout's `f_base`/`fcsr_addr` to the live CPU FP file
  (was 0 = disabled), so FP arith/compare/FMV and FLD/FSD (via the bailing
  inline-TLB path) compile in system blocks under the same softfloat fast-path
  guard. **Result — both benchmarks now BEAT v86 in system mode**
  (`tests/vs-v86/compare-sys.mjs`): ALU 27.0s→1.35s (9.8× slower → **2.2×
  faster** than v86); mixed 18.0s→0.70s (11.7× slower → **2.3× faster**). Linux
  still boots under JIT (wasm-smoke ALL PASS); 25/25 cargo tests; jit==interp
  checksums identical. rv64.js now leads v86 in *both* modes.

- [ ] **4. (Optional) residual-based flag recovery** — extend the FP fast
  path to work before NX is set, via error-free transformations (TwoSum
  for add, Dekker splitting for mul — no FMA needed, so wasm-compatible).
  Low priority: sticky-NX already covers real workloads.

The old **item 2b (fused JIT software-TLB)** stays deferred — item 1's inline
TLB already delivers the system-mode win; register-locals (3a) is the bigger
lever now. Its two reverted attempts are documented in git history.

## Validation

- [x] **5. Architecture-test compliance vs Spike** *(done 2026-07-22)* —
  implemented as direct signature comparison (`tests/run-arch-tests.sh`):
  the official riscv-arch-test 3.9.1 suites (I M A C F D Zifencei
  privilege) compile once against a shared HTIF/CLINT env and run on both
  rv64.js (`rv64-isa-test --signature`) and Spike (`+signature=`).
  **193/193 signatures bit-identical.** This is the substance of RISCOF
  (same tests, same golden model, same pass criterion); adopting the
  RISCOF report framework itself remains optional polish.

- [x] **6. Lockstep differential vs Spike** *(done 2026-07-22)* —
  `tests/lockstep.py` + `rv64-isa-test --trace`: per-instruction
  x-register writeback streams diffed against `spike --log-commits`
  (flake builds Spike with `--enable-commitlog`). 109/109 riscv-tests
  lockstep-identical (~24k writebacks; ma_data skipped as the documented
  spec-legal misaligned-access divergence).

- [x] **6b. Modern-system smoke / regression harness** *(done 2026-07-23)* —
  `tests/virt-smoke/run.sh` boots the virt machine with a ~10 KB initramfs
  whose init exercises the paths that were broken (8250 THRE TX interrupt,
  LR/SC-across-trap, live rdtime) and checks for `SMOKE_OK` vs a hang. Inputs
  come from the flake (`.#virt-kernel`, `.#virt-opensbi`; the freestanding init
  is built by the dev-shell cross-gcc) so it's reproducible. **Coverage is layered** because not all bugs are catchable by
  an integration test: the THRE hang is deterministic and caught by the smoke
  test, but the LR/SC reservation and rdtime bugs are probabilistic/latent —
  a simple boot passes even with them reverted — so they are pinned by
  deterministic unit tests (`cpu::tests::trap_invalidates_lr_reservation`,
  `rdtime_derives_live_from_insn_count`). Each guard was validated by
  reverting its fix and confirming it fails. See `tests/virt-smoke/README.md`.

## Features (TinyEMU parity and beyond)

- [x] **7. virtio-9p** *(done 2026-07-25)* — host filesystem sharing. A
  9P2000.L server (`p9.rs`) behind a small `FsBackend` trait, with two backends
  (`p9fs.rs`): **HostFs** exports a real host directory, **MemFs** an in-memory
  tree loadable from a tar archive (the browser's only option — no host
  filesystem there). The transport gained `Backend::Fs`, per-device feature bits
  and ring depth, and **byte/half-width config-space reads** — not optional,
  since Linux reads the mount tag one byte at a time (`virtio_cread_bytes`), so
  a 32-bit-only config space never gets mounted. `--9p DIR [--9p-tag TAG]` on
  both runners; `sys_stage_fs_tar` in the wasm ABI.

  Verified end to end (`tests/p9_boot.rs`): the guest mounts the export, reads
  host files including nested ones, lists directories, creates and appends files
  that land on the host, mkdir/rmdir, and sees host-side changes made after
  mount. wasm-smoke covers the tar-backed browser path under the JIT.

  Two notes for next time. The path-addressed backend is deliberate: 9P read and
  write both carry explicit offsets, so no per-fid seek state is needed; the one
  stateful spot is `Treaddir`, which snapshots per fid. And `mv` cannot be
  tested through the TinyEMU guest — rename(2) fails with ENOSYS there for
  *every* filesystem, tmpfs included — so `Trenameat` is covered in the HostFs
  backend tests instead. Still open: async completion, which a
  fetch/IndexedDB-backed browser export would need.

- [x] **8. virtio-net** *(done 2026-07-25)* — a layer-2 NIC and two ways for its
  frames to leave. `Backend::Net` (device id 1) is RX queue 0, TX queue 1,
  `VIRTIO_NET_F_MAC`, and two bounded mailboxes; it knows no ARP, IP or TCP.
  The per-frame header is **12 bytes** (`virtio_net_hdr_v1`), not 10, because we
  negotiate `VIRTIO_F_VERSION_1` and Linux sizes `vi->hdr_len` on exactly that
  condition — a 10-byte header shifts every frame and nothing parses.

  Transport A, **WebSocket relay** (`ws.rs`): an RFC 6455 client in ~330 lines
  with no dependencies, one binary message per Ethernet frame (websockproxy /
  v86's wire format). `--net ws://HOST:PORT` natively; `connectNet(url)` in the
  browser. Full fidelity — any protocol, any port, TLS end-to-end from the guest
  so the relay is a dumb pipe — but it requires a relay to exist.

  Verified (`tests/net_boot.rs`): the guest pings a host implemented *in the
  test* — a ~60-line ARP + ICMP responder, the smallest peer that proves frames
  cross intact both ways and that the guest's own stack accepts them. Confirmed
  by mutation: shifting RX by two bytes fails it with 100% loss while frames
  still flow.

- [x] **8b. In-browser HTTP proxy** *(done 2026-07-25 — the one that needs no
  infrastructure)* — the relay above is full-fidelity but useless without a
  relay to point at, and in a browser a page cannot open a socket at all. Its
  only egress primitive is `fetch()`, which is request/response shaped. So the
  guest gets an **HTTP proxy** instead of a route: it sets `http_proxy`, hands us
  a hostname and a complete request, and we perform it.

  **See [HANDOFF.md](HANDOFF.md) for a condensed status snapshot.** The RTC,
  TLS-capable Debian guest, `rv64-vboot --proxy`, HTTP/HTTPS integration test,
  and raw-Wasm-compatible TLS termination are complete. This item's full
  narrative continues below.

  Moving up a layer deletes most of a slirp: no DNS (the guest never resolves
  anything — it passes the name to us), no NAT, no per-destination tracking, no
  UDP beyond DHCP. What remains is `netstack.rs` — ARP, DHCP, ICMP echo, and TCP
  terminated to exactly one address:port — and `httpproxy.rs`, which parses
  absolute-URI requests and hands them to an `Egress` trait. Compare TinyEMU's
  `slirp/` at ~8.5k lines.

  Both are pure logic driven by the host loop, so the same code serves native
  (`egress.rs`, raw TCP for HTTP and validating rustls for HTTPS) and browser
  (`fetch`, which brings HTTPS for free). `--proxy` is on both native runners;
  `proxy: true` is in `bootLinux`.

  **Reach is bounded by CORS, and the boundary is better than it sounds.** Only
  hosts sending `Access-Control-Allow-Origin` can be read — but measured, that
  is specifically the API and package web: `api.github.com`,
  `raw.githubusercontent.com`, `api.openai.com`, `registry.npmjs.org` and
  `files.pythonhosted.org` (metadata *and* artifacts), `static.crates.io` all
  send `*`. `deb.debian.org` and ordinary web pages do not. So `pip install` and
  `npm install` are reachable with zero infrastructure; browsing is not.

  Verified (`tests/proxy_boot.rs`): the guest configures eth0 **by DHCP** from
  our own server (real busybox `udhcpc` completes DISCOVER/OFFER/REQUEST/ACK and
  reports the lease), sets `http_proxy`,
  and `wget`s through the proxy — including a 20 KB response whose md5 the guest
  computes and the test compares against the same bytes hashed on the host, a
  404 passed through, and an egress failure surfacing as a readable 502.
  Confirmed by mutation: a corrupted TCP checksum makes the guest drop our
  segments and the fetch fail. wasm-smoke runs the same path through the real
  `fetch()` against a loopback origin, under the JIT.

  Responses **stream**: `Egress` reports head, then body chunks, then end, so an
  SSE or long download reaches the guest as it arrives and nothing is buffered
  whole. `content-length` and `content-encoding` are deliberately dropped from
  the forwarded head — `fetch` decompresses transparently, so the upstream
  length describes the *compressed* body and forwarding either would truncate
  the response or make the guest gunzip plaintext; `Connection: close` delimits
  it instead, which is also what lets it stream without chunked encoding.

  **`CONNECT` + TLS termination is complete (2026-07-26).** `tlsproxy.rs`
  combines rustls with the pure-Rust `oxitls-rustcrypto-provider`, an ephemeral
  Ed25519 CA, and cached per-host certificates. Its public certificate is
  available from `http://rv64-proxy.invalid/ca.der` and, before networking, as
  `/ca.der` on the fixed `rv64-proxy` 9p tag; only guest boot policy decides
  whether to install it. The small vendored `rustls-pki-types` patch changes
  only its wasm clock boundary from wasm-bindgen/web-time to the project's
  existing raw `host_unix_ms` import, while a custom getrandom backend uses
  `host_random`. Release-Wasm inspection shows only the expected raw `env`
  imports and no wasm-bindgen/externref imports.
  `tests/virt-proxy/run.sh` proves the entire RTC + boot-time CA mount/install
  + DHCP + OpenSSL validation + HTTPS curl CONNECT path through the real Debian
  guest.

  **Chunked request bodies are complete (2026-07-26).** The parser waits for
  and decodes the complete chunk stream within the existing body bound before
  crossing the request-shaped `Egress` boundary. It rejects conflicting
  `Content-Length`, stacked transfer codings, malformed chunks, oversized
  decoded bodies, and trailers that `Egress` cannot represent.
  `tests/virt-proxy/run.sh` includes a real chunked curl POST whose decoded body
  is checked by the loopback origin.

  **Per-request relay fallback is complete (2026-07-26).** The raw Ethernet
  websockproxy transport cannot consume a request after the in-process proxy
  has terminated it, so `connectHttpRelay()` uses a separate, request-level
  WebSocket protocol. Browser fetch remains first and infrastructure-free.
  GET/HEAD failures before a response head retry through the relay and cache
  that origin for direct routing; non-idempotent methods require explicit
  `routeHttpViaRelay()` selection, preventing duplicate delivery after an
  ambiguous CORS failure. `web/http-relay.mjs` supplies the loopback-default
  Node relay and restricts browser Origins by default. Tests cover the real
  WebSocket server plus fallback and origin caching through the booted Wasm
  guest.

  **Native HTTPS + boot-time CA delivery are complete (2026-07-26).**
  `NativeEgress` now uses rustls with the host trust bundle for independently
  verified upstream HTTPS, while ordinary native HTTP remains plaintext.
  CONNECT traffic is always reconstructed as HTTPS after decryption instead of
  sharing the browser-only mixed-content upgrade decision. Proxy-enabled
  machines eagerly expose only their ephemeral public CA certificate at
  `/ca.der` on the fixed virtio-9p tag `rv64-proxy`; the signing key remains in
  the proxy. The Debian guest mounts and installs that CA before becoming ready.
  Its E2E reaches real example.com over both schemes and verifies that the HTTPS
  peer presented `CN=example.com`, issued by `rv64.js ephemeral proxy CA`.

- [x] **9. Modern guest images** *(done 2026-07-23)* — a new **virt**-class
  machine (`crates/rv64-system/src/virt.rs`, runner `bin/rv64-vboot`) boots a
  stock **Debian riscv64 6.18 kernel** via **OpenSBI fw_dynamic** (not BBL),
  with a full PLIC, ns16550 UART, sifive-test, CLINT and virtio-mmio, plus a
  modern DTB (incl. `/chosen/rng-seed`). With a virtio-blk/ext4-builtin kernel
  it mounts a real Debian rootfs and runs Debian userland; a debootstrap-built
  rootfs with build-essential **compiles the Linux kernel in-guest** (gcc-14 +
  make; ~100× slower than native under the interpreter — a full build is a
  days-long soak, not for CI). Getting here fixed several real full-system
  bugs — the 8250 THRE transmit interrupt, LR/SC reservation-on-trap, and
  live rdtime; see the smoke harness below and `tests/virt-smoke/README.md`.

- **10. Snapshot save/restore — declined/not planned.** The project has
  explicitly chosen not to pursue snapshot/restore for the current release
  direction.

## Public API and fast boot

- [x] **14. Stable JavaScript facade and TypeScript declarations** *(done
  2026-08-01)* — the
  lifecycle, typed events, image sources, declarations, and executable API
  contract are implemented. Both full-system examples use the facade and the
  former methods are deliberately absent; see [API_DESIGN.md](API_DESIGN.md).
- [ ] **15. One normal platform** — make `riscv-virt` the default public
  platform, unify the system Wasm/run/console surface, and retain the
  TinyEMU-compatible board as explicit `legacy-tinyemu` compatibility rather
  than a peer user-facing Linux mode.
- [x] **16. No-firmware direct Linux boot** *(done 2026-08-01)* — load a kernel/initrd/DTB directly,
  enter Linux in S-mode, and provide the required SBI boundary inside rv64.js,
  with no caller-supplied or executed firmware image. This is a required
  supported option whose primary goal is lower time-to-kernel and
  time-to-shell. Benchmark it against identical OpenSBI boots using the gates
  in `API_DESIGN.md`; select the default from evidence, but retain direct boot
  even if the gain is below 10%. Implemented with emulator-provided SBI 2.0
  Base/TIME/IPI/RFENCE/HSM/SRST. The five-run lean-kernel comparison improved
  readiness 4.3% and retired 1.9% fewer instructions, so it remains an option
  but kernel configuration remains the primary speed path.
- [x] **17. Fast `riscv-virt` guest** — replace the legacy BusyBox preset with
  the measured Alpine machine as the single supported demo. Its kernel starts from a strict
  `allnoconfig` contract in `kernel/rv64-config.nix`: 527 built-ins, no modules,
  and a 3.9 MiB Image. It boots Debian and Alpine through the public Wasm API,
  including the proxy CA/9p/network path and a real `apk update`. Declaring
  rv64.js's efficient misaligned-access contract avoids Linux's emulated-speed
  calibration: five Alpine boots reached readiness in 1,246 ms / 39.92M
  instructions, effectively matching legacy TinyEMU's 1,240 ms / 42.04M. The
  v86 scorecard Boot row now uses paired Linux 6.12.7/Alpine 3.24.1
  initramfs artifacts instead of unrelated guest images; its five-run median
  is 1,534 ms for rv64.js/OpenSBI versus 1,034 ms for v86/SeaBIOS. The release
  guest and browser demo now use Alpine 3.24 with direct boot exclusively.
- [ ] **18. Release-quality terminal and package** — integrate xterm.js through
  the public console API, test the hosted Alpine machine end to end, then
  publish an ESM package containing JavaScript, Wasm, declarations, and source
  maps. The xterm.js integration is complete; packaging remains.
- [ ] **19. Persistent browser disks** — move VM execution into a dedicated
  worker and add a block-backend abstraction. Support OPFS through a synchronous
  access handle in that worker, preferably as a read-only base image plus a
  sparse copy-on-write overlay; retain the in-memory byte-backed backend for
  ephemeral and unsupported environments. The optional dedicated-Worker
  execution facade is implemented (local remains the default); the block
  backend and OPFS storage work remains.

## Housekeeping

- [x] **11. Publish** — public at
  [ibuildthecloud/rv64.js](https://github.com/ibuildthecloud/rv64.js).
- [x] **12. CI** — GitHub Actions enforces formatting, Clippy, guest builds,
  debug/release tests, the Wasm build, and a real fast-Linux browser-API boot.
  The image/oracle-heavy strict gate remains reproducible through Nix.
- [x] **13. Browser demo** — GitHub Pages offers the modern direct-boot
  Alpine/Linux machine backed by versioned release assets.
- [ ] Optionally rename the directory (`~/src/arm64.js` → `rv64.js`).

## Recommended sequencing

1 → 2 → 3 (the perf trilogy) → 7 → 8/8b. All done, including the later
RV64GCV 13/13 closure. Snapshot/restore is not planned. The active
release direction is **14 → 15 → 16 → 17 → 18**: define the stable API, converge
on `riscv-virt`, deliver measured no-firmware direct Linux boot, rebuild the
fast guest on that platform, and finish the terminal/package integration.
Validation items 5–6 remain suite stages 5–6.
