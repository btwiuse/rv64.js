#!/usr/bin/env python3
"""Lockstep differential: compare rv64.js x-register commit streams against
Spike's commit log for riscv-tests binaries.

Usage: tests/lockstep.py [--spike PATH] <test-elf>...

Both sides run the same bare-metal ELF; we normalize each log to a stream
of (pc, reg, value) x-register writebacks starting from the test entry at
0x80000000 (skipping each simulator's own reset preamble) and diff them.
CSR side-effects are intentionally out of scope (models legitimately
differ in WARL details); pc-ordered register dataflow is the strong
equivalence check.
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RUNNER = os.path.join(ROOT, "target/release/rv64-isa-test")

# spike --log-commits line (may carry several writebacks, e.g. CSR ops:
# "core 0: 0 0xPC (0xINSN) c2_frm 0x.. x11 0x.. c1_fflags 0x.."):
SPIKE_LINE = re.compile(r"core\s+\d+: \d+ (0x[0-9a-f]+) \(0x[0-9a-f]+\)(.*)")
SPIKE_XWRITE = re.compile(r"\sx\s*(\d+)\s+(0x[0-9a-f]+)")
# rv64-isa-test --trace line: 0x80000000 x1 0x0
OURS_RE = re.compile(r"(0x[0-9a-f]+) x(\d+) (0x[0-9a-f]+)")


def spike_stream(spike, elf):
    with tempfile.NamedTemporaryFile(suffix=".log", delete=False) as f:
        log = f.name
    subprocess.run(
        [spike, "--isa=rv64gc", "--log-commits", f"--log={log}", elf],
        timeout=120, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    out = []
    for line in open(log):
        m = SPIKE_LINE.match(line)
        if not m:
            continue
        w = SPIKE_XWRITE.search(m.group(2))
        if w and int(w.group(1)) != 0:
            out.append((int(m.group(1), 16), int(w.group(1)), int(w.group(2), 16)))
    os.unlink(log)
    return out


def our_stream(elf):
    with tempfile.NamedTemporaryFile(suffix=".log", delete=False) as f:
        log = f.name
    subprocess.run(
        [RUNNER, "--trace", log, elf],
        timeout=300, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    out = []
    for line in open(log):
        m = OURS_RE.match(line)
        if m:
            out.append((int(m.group(1), 16), int(m.group(2)), int(m.group(3), 16)))
    os.unlink(log)
    return out


def trim_to_entry(stream):
    """Drop each simulator's own reset/trampoline preamble."""
    for i, (pc, _, _) in enumerate(stream):
        if pc >= 0x8000_0000:
            return stream[i:]
    return []


def cut_tohost_spin(stream):
    """Truncate at the write_tohost spin loop: HTIF termination is
    asynchronous in Spike, so the tail repeats a short cycle of identical
    (pc, reg, value) events a simulator-dependent number of times."""
    for i in range(len(stream)):
        for period in (1, 2, 3, 4):
            if i >= period and stream[i] == stream[i - period]:
                return stream[:i]
    return stream


# Spec-legal implementation differences (not bugs): both behaviors are
# architecturally allowed, so the streams legitimately diverge.
KNOWN_DIVERGENT = {
    # rv64.js executes misaligned loads/stores in hardware (same choice as
    # TinyEMU/qemu); default-config Spike traps to the handler instead.
    "rv64ui-p-ma_data",
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--spike", default=os.environ.get("SPIKE", "spike"))
    ap.add_argument("elves", nargs="+")
    args = ap.parse_args()

    total_events = 0
    failed = 0
    skipped = 0
    for elf in args.elves:
        name = os.path.basename(elf)
        if name in KNOWN_DIVERGENT:
            skipped += 1
            print(f"LOCKSTEP-SKIP {name} (spec-legal divergence: misaligned access choice)")
            continue
        s = cut_tohost_spin(trim_to_entry(spike_stream(args.spike, elf)))
        o = cut_tohost_spin(trim_to_entry(our_stream(elf)))
        n = min(len(s), len(o))
        diverged = None
        for i in range(n):
            if s[i] != o[i]:
                diverged = i
                break
        if diverged is None and abs(len(s) - len(o)) <= 4:
            # tiny tail slack: where exactly the spin-cycle cut lands
            print(f"LOCKSTEP-OK   {name} ({n} events)")
            total_events += n
        else:
            failed += 1
            i = diverged if diverged is not None else n
            print(f"LOCKSTEP-FAIL {name} at event {i}:")
            for j in range(max(0, i - 2), min(n, i + 3)):
                sp = s[j] if j < len(s) else None
                us = o[j] if j < len(o) else None
                mark = "  " if sp == us else "->"
                print(f"  {mark} spike={fmt(sp)}  rv64={fmt(us)}")
    print(f"--- {len(args.elves) - failed - skipped}/{len(args.elves) - skipped} tests "
          f"lockstep-identical ({skipped} spec-legal skips), "
          f"{total_events} register writebacks compared")
    sys.exit(1 if failed else 0)


def fmt(e):
    if e is None:
        return "<end>"
    return f"(pc={e[0]:#x} x{e[1]}={e[2]:#x})"


if __name__ == "__main__":
    main()
