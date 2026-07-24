/* Portable fixed-work CPU benchmark for emulator comparison.
 * Deterministic, no timing syscalls — the HOST harness measures wall-clock.
 * Three kernels (integer/memory sort, FP recurrence, memory stride) each do a
 * FIXED amount of work so x86 (v86) and riscv64 (rv64.js) run identical work;
 * lower host wall-clock = faster emulator. Prints a checksum to verify
 * correctness across builds. WORK scales all kernels from one knob.
 */
#include <stdio.h>
#include <stdint.h>

#ifndef WORK
#define WORK 1
#endif

int main(void) {
    const int N = 2048;
    const int SORT_ITERS = 60 * WORK;    /* O(N^2) insertion sort passes */
    const long FP_ITERS  = 40000000L * WORK;
    const long MEM_ITERS = 20000000L * WORK;
    static int32_t a[2048];
    static double  d[2048];

    /* --- integer + memory: xorshift fill then insertion sort, repeated --- */
    uint64_t isum = 0;
    uint32_t rng = 2463534242u;
    for (int it = 0; it < SORT_ITERS; it++) {
        for (int i = 0; i < N; i++) {
            rng ^= rng << 13; rng ^= rng >> 17; rng ^= rng << 5;
            a[i] = (int32_t)rng;
        }
        for (int i = 1; i < N; i++) {
            int32_t k = a[i]; int j = i - 1;
            while (j >= 0 && a[j] > k) { a[j + 1] = a[j]; j--; }
            a[j + 1] = k;
        }
        isum += (uint32_t)a[0] ^ (uint32_t)a[N / 2] ^ (uint32_t)a[N - 1];
    }

    /* --- floating point: two interleaved recurrences (data-dependent) --- */
    double f0 = 1.0, f1 = 0.5;
    for (long i = 0; i < FP_ITERS; i++) {
        f0 = f0 * 0.999999977 + f1 * 1.0000001;
        f1 = f1 * 0.999999983 - f0 * 0.00000005 + 1.0;
        if (f0 > 1e6) f0 *= 1e-6;
        if (f1 > 1e6) f1 *= 1e-6;
    }

    /* --- memory: strided read/modify/write over the double array --- */
    for (int i = 0; i < N; i++) d[i] = (double)a[i];
    double msum = 0;
    for (long i = 0; i < MEM_ITERS; i++) {
        int idx = (int)((uint32_t)(i * 2654435761u) >> 21) & (N - 1);
        d[idx] = d[idx] * 1.0000001 + 1.0;
        msum += d[idx];
    }

    printf("isum=%llu f0=%.6f f1=%.6f msum=%.3f\nBENCH_DONE\n",
           (unsigned long long)isum, f0, f1, msum);
    return 0;
}
