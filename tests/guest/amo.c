/* Atomic memory operations, in-guest: every AMO* form the JIT compiles, over
 * values that exercise sign boundaries, on both .W and .D widths. The final
 * checksum must be identical with the JIT on and off (tests/amo-diff.mjs). */
typedef unsigned long long u64;
typedef long long i64;
typedef unsigned int u32;

static u64 cell[8];

static u64 mix(u64 h, u64 v) { return (h ^ v) * 0x100000001b3ull; }

void _start(void) {
  u64 h = 0xcbf29ce484222325ull;
  static const i64 seeds[] = {0, 1, -1, 0x7fffffff, -0x80000000LL, 0x123456789abcdefLL,
                              -3, 255, 0x80000000LL, 0xffffffffLL};
  for (int r = 0; r < 200; r++) {
    for (unsigned s = 0; s < sizeof(seeds) / sizeof(seeds[0]); s++) {
      i64 v = seeds[s] + r;
      for (int c = 0; c < 8; c++) cell[c] = (u64)(seeds[(s + c) % 10]);
      i64 o0 = __atomic_fetch_add(&cell[0], v, __ATOMIC_RELAXED);
      i64 o1 = __atomic_exchange_n(&cell[1], v, __ATOMIC_RELAXED);
      i64 o2 = __atomic_fetch_and(&cell[2], v, __ATOMIC_RELAXED);
      i64 o3 = __atomic_fetch_or(&cell[3], v, __ATOMIC_RELAXED);
      i64 o4 = __atomic_fetch_xor(&cell[4], v, __ATOMIC_RELAXED);
      /* 32-bit forms (AMO*.W): sign-extension of the old value matters */
      u32 *w = (u32 *)&cell[5];
      i64 o5 = (int)__atomic_fetch_add(w, (u32)v, __ATOMIC_RELAXED);
      i64 o6 = (int)__atomic_exchange_n(w + 1, (u32)v, __ATOMIC_RELAXED);
      i64 o7 = (int)__atomic_fetch_or((u32 *)&cell[6], (u32)v, __ATOMIC_RELAXED);
      h = mix(h, (u64)o0); h = mix(h, (u64)o1); h = mix(h, (u64)o2);
      h = mix(h, (u64)o3); h = mix(h, (u64)o4); h = mix(h, (u64)o5);
      h = mix(h, (u64)o6); h = mix(h, (u64)o7);
      for (int c = 0; c < 8; c++) h = mix(h, cell[c]);
      /* compare-and-swap drives LR/SC (not compiled: must still be exact) */
      u64 exp = cell[7];
      __atomic_compare_exchange_n(&cell[7], &exp, (u64)v, 0, __ATOMIC_RELAXED, __ATOMIC_RELAXED);
      h = mix(h, cell[7]);
    }
  }
  static char buf[32] = "amo checksum=0x0000000000000000\n";
  for (int i = 0; i < 16; i++) buf[30 - i] = "0123456789abcdef"[(h >> (4 * i)) & 15];
  __asm__ volatile("li a7, 64\n li a0, 1\n mv a1, %0\n li a2, 32\n ecall" ::"r"(buf)
                   : "a0", "a1", "a2", "a7");
  __asm__ volatile("li a7, 93\n li a0, 0\n ecall" ::: "a0", "a7");
}
