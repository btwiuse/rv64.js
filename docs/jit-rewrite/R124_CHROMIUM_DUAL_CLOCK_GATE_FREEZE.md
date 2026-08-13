# R124 Chromium Dual-Clock Boot Gate Freeze

Date: 2026-08-10  
Status: frozen before the first R124 browser timing sample

## Purpose

R124's native target is Compile.  Because the fixed-bank implementation also
affects generated execution during Boot, Chromium Boot is a protected product
confirmation under R104.  R107 additionally requires two clocks in the same
fresh Worker/guest leg:

- execution-only, from the first 2,000,000-instruction pump through
  `SCORECARD_V2_READY`; and
- construction-to-marker, from immediately before `RV64.create` through the
  same ready marker.

Immutable asset fetch and SHA-256 verification occur before both clocks.  The
inclusive clock covers real main-Wasm compilation/instantiation, production
policy setup, direct Linux setup, guest execution, and generated-module
compilation.  This gate uses only Linux 6.12.7 and Alpine 3.24.1; it has no
BBL, TinyEMU, or legacy root input.

## Immutable artifacts and browser

- control Wasm `d9f686a9ce4f...`, 4,279,380 bytes;
- candidate Wasm `d017a10f00a8...`, 4,281,786 bytes;
- loader `2cbb264f4dac...`;
- kernel `57d077974820...`;
- initramfs `cbb75afb016d...`;
- Chrome `150.0.7871.186`, V8 `15.0.245.21`, revision
  `0fcdce5f4fdec8d442d7df760cb541f1ca6e446d`; and
- host affinity CPUs 8--15.

Fresh Chrome process, profile, Worker, RV64 instance, and guest are created for
every leg.  Production page policy and public scorecard cadence are unchanged.

## Frozen harness

- host `d64adc239620...`;
- seven-pair runner `641508bb8f60...`;
- analyzer `0ef47f5b2b62...`;
- analyzer selftest `f64bf2a54f35...`;
- source/timer selftest `8015f870afeff...`;
- Worker `15ff6f9f13f4...`;
- page `c05b685b3399...`; and
- fixed cadence library `8d39c83ad01e...`.

Both selftests passed before freeze.  The analyzer separately rejects an
execution-clock regression and a construction-to-marker regression.

## Frozen sampling and decision

Run seven alternating pairs.  Preserve all fourteen legs and use the exact
paired bootstrap of the median.  Both clocks are protected and independently
must satisfy:

- paired median candidate speedup at least `0.99x`; and
- 95% interval upper bound at least `1.00x`, so the evidence does not establish
  a regression.

Every browser/asset/guest/policy/output/generated-execution/retirement/cadence
proof must pass.  The Browser gate earns no additional target credit; it
confirms portability and product non-regression after the verified native
Compile gain.

Run exactly once:

```sh
taskset -c 8-15 node tests/run-r124-chrome-boot-pairs.mjs \
  target/bench/r124-rvc-bank-hybrid/chromium-pairs

node tests/analyze-r124-chrome-boot-pairs.mjs \
  target/bench/r124-rvc-bank-hybrid/chromium-pairs \
  --output=target/bench/r124-rvc-bank-hybrid/chromium-gate.json
```

Do not replace, extend, trim, or rerun a leg after seeing results.  A pass
advances exact `d017a10f...` unchanged to the R094-qualified WANIX gate.  A
failure archives R124 and restores exact `d9f686a9...` without changing the
resident bank or browser rule.
