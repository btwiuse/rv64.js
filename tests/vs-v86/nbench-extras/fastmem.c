/* Tuned word-wise memmove/memcpy for the riscv64 build — the ISA-fair
 * counterpart of musl's hand-written i386 asm string routines (the generic C
 * fallback musl uses on riscv64 is the bottleneck of STRING SORT).
 *
 * rv64.js implements the RISC-V platform's permitted misaligned scalar loads
 * and stores, including page-crossing accesses. Use that advertised platform
 * capability here: requiring equal source/destination alignment makes a
 * memmove-heavy string sort fall back to one byte per loop whenever a string
 * changes length. */
#include <stddef.h>
#include <stdint.h>

static inline uint64_t load64u(const unsigned char *p) {
    uint64_t value;
    __asm__ volatile ("ld %0, 0(%1)" : "=r"(value) : "r"(p) : "memory");
    return value;
}

static inline void store64u(unsigned char *p, uint64_t value) {
    __asm__ volatile ("sd %0, 0(%1)" : : "r"(value), "r"(p) : "memory");
}

/* Keep the base pointers live and encode block offsets in the memory
 * instructions. Separate tiny asm statements make LLVM materialize sixteen
 * temporary addresses around an eight-word block. */
static inline void copy64f(unsigned char *d, const unsigned char *s) {
    uint64_t value;
    __asm__ volatile (
        "ld %[v], 0(%[s])\n\t"  "sd %[v], 0(%[d])\n\t"
        "ld %[v], 8(%[s])\n\t"  "sd %[v], 8(%[d])\n\t"
        "ld %[v], 16(%[s])\n\t" "sd %[v], 16(%[d])\n\t"
        "ld %[v], 24(%[s])\n\t" "sd %[v], 24(%[d])\n\t"
        "ld %[v], 32(%[s])\n\t" "sd %[v], 32(%[d])\n\t"
        "ld %[v], 40(%[s])\n\t" "sd %[v], 40(%[d])\n\t"
        "ld %[v], 48(%[s])\n\t" "sd %[v], 48(%[d])\n\t"
        "ld %[v], 56(%[s])\n\t" "sd %[v], 56(%[d])"
        : [v] "=&r" (value)
        : [s] "r" (s), [d] "r" (d)
        : "memory");
}

static inline void copy64b(unsigned char *d, const unsigned char *s) {
    uint64_t value;
    __asm__ volatile (
        "ld %[v], -8(%[s])\n\t"  "sd %[v], -8(%[d])\n\t"
        "ld %[v], -16(%[s])\n\t" "sd %[v], -16(%[d])\n\t"
        "ld %[v], -24(%[s])\n\t" "sd %[v], -24(%[d])\n\t"
        "ld %[v], -32(%[s])\n\t" "sd %[v], -32(%[d])\n\t"
        "ld %[v], -40(%[s])\n\t" "sd %[v], -40(%[d])\n\t"
        "ld %[v], -48(%[s])\n\t" "sd %[v], -48(%[d])\n\t"
        "ld %[v], -56(%[s])\n\t" "sd %[v], -56(%[d])\n\t"
        "ld %[v], -64(%[s])\n\t" "sd %[v], -64(%[d])"
        : [v] "=&r" (value)
        : [s] "r" (s), [d] "r" (d)
        : "memory");
}

static void *fwd(unsigned char *d, const unsigned char *s, size_t n) {
    unsigned char *d0 = d;
    if (n >= 16) {
        while ((uintptr_t)d & 7) { *d++ = *s++; n--; }
        while (n >= 64) {
            copy64f(d, s);
            d += 64; s += 64; n -= 64;
        }
        while (n >= 8) { store64u(d, load64u(s)); d += 8; s += 8; n -= 8; }
    }
    while (n--) *d++ = *s++;
    return d0;
}
static void *bwd(unsigned char *d, const unsigned char *s, size_t n) {
    unsigned char *d0 = d; d += n; s += n;
    if (n >= 16) {
        while ((uintptr_t)d & 7) { *--d = *--s; n--; }
        while (n >= 64) {
            copy64b(d, s);
            d -= 64; s -= 64; n -= 64;
        }
        while (n >= 8) { d -= 8; s -= 8; n -= 8; store64u(d, load64u(s)); }
    }
    while (n--) *--d = *--s;
    return d0;
}
void *memmove(void *dst, const void *src, size_t n) {
    unsigned char *d = dst; const unsigned char *s = (const unsigned char *)src;
    if (d == s || n == 0) return dst;
    return (uintptr_t)d - (uintptr_t)s >= n ? fwd(d, s, n) : bwd(d, s, n);
}
void *memcpy(void *dst, const void *src, size_t n) { return fwd(dst, (const unsigned char *)src, n); }
