# R125 WANIX Single-Flight Qualification

Date: 2026-08-10  
Status: runtime correction qualified; replacement product gate admitted but not yet sampled

## Why R124 stopped

The frozen R124 WANIX run is invalid and remains invalid. Pair 5 control used
the exact pre-R124 core and timed out after Python and SHA-256 completed. No
partial ratio, replacement leg, or rerun is performance evidence.

Diagnostic-only instrumentation closed the console hypothesis: the harness
observed its visible phase-release token before the stall. The emulator then
completed exactly 33,554,432 write bytes and all read traffic while guest
instructions continued to retire. One external 9P request remained pending for
more than 64 seconds: `T_UNLINKAT` (type 76), tag 0, directory fid 30, flags 0,
for the benchmark temporary file. The exact control core—not the R124
candidate—was active.

This establishes a host integration failure. It does not establish an R124
performance regression.

## General correction

The emulator's external-9P loader remains capable of multiplexing independent
requests. Only the WANIX adapter now serializes delivery into WANIX's
stream-backed Go 9P endpoint. Its JavaScript bridge keeps a FIFO queue, posts
one request at a time, releases the queue only for the matching response, keeps
tag-collision detection, and cannot remain wedged after a synchronous
`postMessage` failure.

This is not keyed to `T_UNLINKAT`, a filename, a guest PC, or a benchmark. It
is a transport capability boundary: an endpoint whose internal path-lock layer
does not safely support multiplexing receives one request at a time, while
other external-9P handlers retain the generic concurrent path.

Relevant source identities are:

- `integrations/wanix/adapter/p9-handler.js`:
  `2f2ff46bdda3b82e46031d63f97b2dcabdf4854fbf06fdafee4cd8b44033a7c6`;
- `integrations/wanix/adapter/main.go`:
  `d1105f7380ad961314a34141aa948e20ed7e37839748f9c8818a8548e407b956`;
- queue test `tests/wanix-p9-singleflight.mjs`:
  `586458aa5a2782c794eeb825aab2013884435f3386b61ad71956f9688ebcc814`.

The generic concurrent-loader regression still passes independently. The new
queue test covers FIFO release, unknown replies, duplicate tags, and recovery
from a failed post. Adapter Go tests pass. Two isolated ordinary Go/Wasm builds
are byte-identical at
`bba6baaf8ddaa8245b00b1dd3d0db7bf72ffc7e48a1e9db3c0e2ab7f3bde8bbe`
(3,530,750 bytes). A deliberately different `-trimpath` build is not pooled
with that build identity.

## Fresh-browser qualification

Elapsed values below are diagnostic qualification only and provide no R124
candidate credit.

First, a loader-side single-flight prototype completed six of six independent
fresh-browser 32 MiB runs. Each had one correct result, exactly 33,554,432 P9
write bytes, at least 33,615,120 read bytes, and no pending request at exit.
Samples were 25.293, 25.355, 25.371, 25.392, 25.429, and 25.133 seconds.

Next, the actual adapter correction was tested with the loader left
concurrent. It completed another six of six independent fresh browsers:

| Run | Seconds | Loader max pending | P9 writes | P9 reads |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 25.612 | 3 | 33,554,432 | 33,660,176 |
| 2 | 25.476 | 3 | 33,554,432 | 33,615,120 |
| 3 | 25.470 | 3 | 33,554,432 | 33,611,024 |
| 4 | 25.343 | 3 | 33,554,432 | 33,635,600 |
| 5 | 25.276 | 3 | 33,554,432 | 33,660,176 |
| 6 | 25.118 | 3 | 33,554,432 | 33,639,696 |

The 1.020x max/min spread is well below the 1.25x guard, and all samples lie
inside R094's independently qualified long-9P envelope. Host 9P service was
19.645--20.107 seconds, consistent with the pre-fix successful samples. This
does not claim an improvement; it shows that the correctness fix has not
created an obvious throughput discontinuity before formal paired measurement.

Logs are preserved under
`target/bench/r124-rvc-bank-hybrid/wanix-hang-diagnostic/`. The failing
concurrent diagnostic, prototype runs, and actual-adapter runs are all retained;
no timing from that directory is eligible for the replacement gate.

## Replacement-gate admission

R125 uses entirely new immutable pages, archives, tools, protocol, and fresh
samples. The common loader remains exact `2cbb264f...`; the common adapter is
exact `bba6baaf...`; the only control/candidate difference is the inner core:

- control `d9f686a9...`, archive `5fe22762302f6414...`;
- R124 candidate `d017a10f...`, archive `88274081a25ee03a...`.

Two independent archive constructions reproduce each archive byte for byte.
The integration guest remains Linux 6.12.7 / Alpine 3.22.5 / Python 3.12.13;
the untouched authoritative scorecard remains Linux 6.12.7 / Alpine 3.24.1.

The replacement gate retains the frozen R124 statistics: seven alternating
fresh-browser pairs, three synchronized repetitions of unchanged Python,
SHA-256, and qualified 32 MiB shared 9P; exact work and active-JIT proofs;
paired median at least 0.99 for every protected row; no confidence-established
regression; and the existing phase/spread bounds. It starts from zero samples
and neither replaces nor pools anything from R124.
