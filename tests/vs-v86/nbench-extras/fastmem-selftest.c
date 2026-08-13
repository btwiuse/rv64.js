#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

void *memmove(void *dst, const void *src, size_t n);

enum { BUFFER_SIZE = 12288 };

static unsigned char actual[BUFFER_SIZE] __attribute__((aligned(4096)));
static unsigned char expected[BUFFER_SIZE] __attribute__((aligned(4096)));

static void reference_move(volatile unsigned char *dst,
                           const volatile unsigned char *src,
                           size_t n)
{
    if (dst < src || dst >= src + n) {
        while (n--) *dst++ = *src++;
    } else {
        dst += n;
        src += n;
        while (n--) *--dst = *--src;
    }
}

static void reset_buffers(unsigned seed)
{
    uint32_t state = UINT32_C(0x9e3779b9) ^ seed;
    size_t i;
    for (i = 0; i < BUFFER_SIZE; i++) {
        state = state * UINT32_C(1664525) + UINT32_C(1013904223);
        actual[i] = expected[i] = (unsigned char)(state >> 24);
    }
}

static int run_case(size_t dst, size_t src, size_t n, unsigned seed,
                    uint64_t *checksum)
{
    size_t i;
    reset_buffers(seed);
    reference_move(expected + dst, expected + src, n);
    memmove(actual + dst, actual + src, n);
    for (i = 0; i < BUFFER_SIZE; i++) {
        if (actual[i] != expected[i]) {
            printf("FASTMEM_FAIL dst=%zu src=%zu n=%zu at=%zu got=%u expected=%u\n",
                   dst, src, n, i, actual[i], expected[i]);
            return 0;
        }
        *checksum = (*checksum ^ actual[i]) * UINT64_C(0x100000001b3);
    }
    return 1;
}

int main(void)
{
    static const size_t long_lengths[] = {
        0, 1, 7, 8, 9, 15, 16, 17, 31, 32, 63, 64, 65,
        127, 128, 129, 255, 256, 257, 511, 512, 513, 1023, 1024, 4097
    };
    uint64_t checksum = UINT64_C(0xcbf29ce484222325);
    unsigned cases = 0;
    size_t n, src_delta, dst_delta, i;

    for (n = 0; n <= 96; n++) {
        for (src_delta = 0; src_delta < 16; src_delta++) {
            for (dst_delta = 0; dst_delta < 16; dst_delta++) {
                if (!run_case(2048 + dst_delta, 2048 + src_delta, n,
                              cases, &checksum)) return 2;
                cases++;
            }
        }
    }

    for (i = 0; i < sizeof(long_lengths) / sizeof(long_lengths[0]); i++) {
        n = long_lengths[i];
        for (src_delta = 0; src_delta < 16; src_delta++) {
            for (dst_delta = 0; dst_delta < 16; dst_delta++) {
                size_t src = 4096 - 8 + src_delta;
                size_t dst = 6144 - 8 + dst_delta;
                if (!run_case(dst, src, n, cases, &checksum)) return 3;
                cases++;
                if (!run_case(src, dst, n, cases, &checksum)) return 4;
                cases++;
            }
        }
    }

    printf("FASTMEM_PASS cases=%u checksum=%016llx\n",
           cases, (unsigned long long)checksum);
    return 0;
}
