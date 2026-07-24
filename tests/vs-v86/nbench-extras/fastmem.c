/* Tuned word-wise memmove/memcpy for the riscv64 build — the ISA-fair
 * counterpart of musl's hand-written i386 asm string routines (the generic C
 * fallback musl uses on riscv64 is the bottleneck of STRING SORT). */
#include <stddef.h>
#include <stdint.h>
static void *fwd(unsigned char *d, const unsigned char *s, size_t n) {
    unsigned char *d0 = d;
    if (n >= 16 && (((uintptr_t)d ^ (uintptr_t)s) & 7) == 0) {
        while ((uintptr_t)d & 7) { *d++ = *s++; n--; }
        uint64_t *dw = (uint64_t *)d; const uint64_t *sw = (const uint64_t *)s;
        while (n >= 64) { dw[0]=sw[0]; dw[1]=sw[1]; dw[2]=sw[2]; dw[3]=sw[3];
                          dw[4]=sw[4]; dw[5]=sw[5]; dw[6]=sw[6]; dw[7]=sw[7];
                          dw += 8; sw += 8; n -= 64; }
        while (n >= 8) { *dw++ = *sw++; n -= 8; }
        d = (unsigned char *)dw; s = (const unsigned char *)sw;
    }
    while (n--) *d++ = *s++;
    return d0;
}
static void *bwd(unsigned char *d, const unsigned char *s, size_t n) {
    unsigned char *d0 = d; d += n; s += n;
    if (n >= 16 && (((uintptr_t)d ^ (uintptr_t)s) & 7) == 0) {
        while ((uintptr_t)d & 7) { *--d = *--s; n--; }
        uint64_t *dw = (uint64_t *)d; const uint64_t *sw = (const uint64_t *)s;
        while (n >= 64) { dw -= 8; sw -= 8; n -= 64;
                          dw[7]=sw[7]; dw[6]=sw[6]; dw[5]=sw[5]; dw[4]=sw[4];
                          dw[3]=sw[3]; dw[2]=sw[2]; dw[1]=sw[1]; dw[0]=sw[0]; }
        while (n >= 8) { *--dw = *--sw; n -= 8; }
        d = (unsigned char *)dw; s = (const unsigned char *)sw;
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
