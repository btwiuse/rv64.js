import sys, time
start=None
for line in sys.stdin:
    t=time.time()
    if "BENCH_START" in line and start is None: start=t
    if "checksum=" in line: sys.stderr.write(line)
    if "BENCH_DONE" in line and start is not None:
        print(f"COMPUTE_WALL_MS={ (t-start)*1000:.0f}"); sys.stdout.flush(); break
