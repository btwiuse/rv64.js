#!/usr/bin/env python3
"""Generate a large, self-contained C translation unit for compiler
benchmarking. No #includes, so `tcc -c` needs only the compiler and this
file (no libc/headers) — isolating pure compiler throughput (lex, parse,
typecheck, codegen, object emission). Scalable via the function count, which
drives the three tiers (quick / medium / soak).

Usage: gen_c.py <n_functions> > workload.c
"""
import sys

n = int(sys.argv[1]) if len(sys.argv) > 1 else 2000

out = []
w = out.append
w("/* generated compiler benchmark translation unit */\n")
w("typedef unsigned long u64;\ntypedef long i64;\n\n")

# A pile of functions with realistic-ish structure: locals, arithmetic,
# branches, loops, and calls between them — exercises the whole front+back end.
for i in range(n):
    w(f"static u64 f{i}(u64 a, u64 b, u64 c) {{\n")
    w("    u64 x = a ^ (b << 3) ^ (c >> 2);\n")
    w("    i64 y = (i64)(a + b) - (i64)c;\n")
    w("    u64 acc = 0;\n")
    w("    for (u64 k = 0; k < (a & 15); k++) {\n")
    w("        x = x * 6364136223846793005UL + 1442695040888963407UL;\n")
    w("        if (x & 1) acc += x ^ y; else acc -= (u64)y + k;\n")
    w("        switch (k & 3) {\n")
    w("            case 0: acc ^= x >> 7; break;\n")
    w("            case 1: acc += b; break;\n")
    w("            case 2: acc = (acc << 1) | (acc >> 63); break;\n")
    w("            default: acc -= c; break;\n")
    w("        }\n")
    w("    }\n")
    # call a couple of earlier functions to create a call graph
    if i > 1:
        w(f"    acc += f{i-1}(x, acc, b) ^ f{i//2}(y, c, x);\n")
    w("    return acc + x - (u64)y;\n")
    w("}\n\n")

# a root that references many, so nothing is trivially dead-eliminated
w("u64 root(u64 seed) {\n    u64 s = seed;\n")
step = max(1, n // 64)
for i in range(0, n, step):
    w(f"    s = f{i}(s, s ^ 0x{i:x}UL, s + {i});\n")
w("    return s;\n}\n")

sys.stdout.write("".join(out))
