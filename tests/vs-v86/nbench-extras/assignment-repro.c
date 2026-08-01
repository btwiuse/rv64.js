/*
 * Diagnostic-only fixed-work reproducer for nbench ASSIGNMENT's two hot
 * multi-latch scans.  This is not a scorecard row: it exists to make JIT
 * semantics, compilation, entry counts, and engine tiering reproducible
 * without nbench's self-timed confidence loop.
 */
#include <stdint.h>
#include <stdio.h>

enum { N = 101, REPS = 20000 };

static int64_t tableau[N][N];
static uint16_t assigned[N][N];

__attribute__((noinline))
static uint32_t scan_row(unsigned row)
{
    uint32_t zeros = 0;
    uint32_t selected = 0;
    for (unsigned col = 0; col < N; col++) {
        if (tableau[row][col] != 0)
            continue;
        if (assigned[row][col] != 0)
            continue;
        zeros++;
        selected = col;
    }
    return (zeros << 16) | selected;
}

__attribute__((noinline))
static uint32_t scan_col(unsigned col)
{
    uint32_t zeros = 0;
    uint32_t selected = 0;
    for (unsigned row = 0; row < N; row++) {
        if (tableau[row][col] != 0)
            continue;
        if (assigned[row][col] != 0)
            continue;
        zeros++;
        selected = row;
    }
    return (zeros << 16) | selected;
}

int main(void)
{
    uint64_t checksum = UINT64_C(0xcbf29ce484222325);
    for (unsigned row = 0; row < N; row++)
        for (unsigned col = 0; col < N; col++) {
            tableau[row][col] = ((row * 17 + col * 29) % 11) ? 1 : 0;
            assigned[row][col] = ((row * 7 + col * 13) % 19) ? 0 : 2;
        }

    puts("ASSIGN_REPRO_START");
    for (unsigned rep = 0; rep < REPS; rep++) {
        uint32_t a = scan_row(rep % N);
        uint32_t b = scan_col((rep * 37) % N);
        checksum ^= ((uint64_t)a << 32) | b;
        checksum *= UINT64_C(0x100000001b3);
    }
    printf("ASSIGN_REPRO checksum=%016llx reps=%u\n",
           (unsigned long long)checksum, REPS);
    puts("ASSIGN_REPRO_DONE");
    return checksum == UINT64_C(0xf168198e29a44860) ? 0 : 3;
}
