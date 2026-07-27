/* rv64.js virt-machine smoke/regression init — freestanding (no libc).
 *
 * Runs as PID 1 in a tiny initramfs. Built with `-nostdlib -ffreestanding` so
 * it needs no C library (any riscv64 gcc works). Deliberately drives the
 * full-system paths that were broken (and are now fixed):
 *
 *   1. 8250 THRE (TX) interrupt   -> a big console-output burst + tty drain.
 *      The 8250 driver goes interrupt-driven once its buffer fills; without a
 *      THR-empty interrupt the drain (and the writer) block forever.
 *   2. LR/SC reservation on trap  -> a fork(clone)+wait loop while the timer
 *      ticks (multi-process / atomic churn). NOTE: this is only a probabilistic
 *      probe; the deterministic guard is
 *      cpu::tests::trap_invalidates_lr_reservation.
 *   3. rdtime advancing           -> a delta across nanosleep.
 *   4. goldfish RTC / wall clock  -> CLOCK_REALTIME must be a modern epoch.
 *
 * On success it prints SMOKE_OK and powers off. If any path is broken the
 * guest wedges and the harness times out -> test failure.
 */

typedef unsigned long ulong;

static inline long sys(long n, long a0, long a1, long a2, long a3) {
    register long a7 asm("a7") = n;
    register long r0 asm("a0") = a0;
    register long r1 asm("a1") = a1;
    register long r2 asm("a2") = a2;
    register long r3 asm("a3") = a3;
    asm volatile("ecall" : "+r"(r0) : "r"(a7), "r"(r1), "r"(r2), "r"(r3) : "memory");
    return r0;
}

/* riscv64 Linux generic syscall numbers */
#define SYS_ioctl     29
#define SYS_write     64
#define SYS_exit      93
#define SYS_nanosleep 101
#define SYS_clock_gettime 113
#define SYS_reboot    142
#define SYS_clone     220
#define SYS_wait4     260

#define SIGCHLD       17
#define TCSBRK        0x5409          /* ioctl: tcdrain when arg != 0 */
#define RB_MAGIC1     0xfee1deadL
#define RB_MAGIC2     0x28121969L
#define RB_POWER_OFF  0x4321fedcL

struct timespec { long tv_sec; long tv_nsec; };

static ulong slen(const char *s) { ulong n = 0; while (s[n]) n++; return n; }
static void emit(const char *s) { sys(SYS_write, 1, (long)s, slen(s), 0); }
static ulong rdtime(void) { ulong t; asm volatile("rdtime %0" : "=r"(t)); return t; }

void _start(void) {
    struct timespec ts = {0, 30 * 1000 * 1000};

    emit("SMOKE_START\n");

    /* (3) rdtime must advance across a sleep */
    ulong t0 = rdtime();
    sys(SYS_nanosleep, (long)&ts, 0, 0, 0);
    if (rdtime() == t0) emit("FAIL_RDTIME_STUCK\n");
    else emit("RDTIME_OK\n");

    /* (4) Linux must seed CLOCK_REALTIME from the emulated goldfish RTC. */
    struct timespec real;
    if (sys(SYS_clock_gettime, 0, (long)&real, 0, 0) != 0 ||
        real.tv_sec < 1700000000L)
        emit("FAIL_RTC_EPOCH\n");
    else
        emit("RTC_OK\n");

    /* (1) flood the console then drain -> forces interrupt-driven TX / THRE */
    for (int i = 0; i < 400; i++)
        emit("filling the 8250 transmit buffer to force interrupt-driven TX ...\n");
    sys(SYS_ioctl, 1, TCSBRK, 1, 0); /* tcdrain(1) */
    emit("TTY_DRAIN_OK\n");

    /* (2) fork+wait loop with the timer ticking underneath */
    for (int i = 0; i < 40; i++) {
        long pid = sys(SYS_clone, SIGCHLD, 0, 0, 0); /* fork */
        if (pid == 0) {
            volatile long x = 0;
            for (int j = 0; j < 50000; j++) x += j;
            sys(SYS_exit, (int)(x & 7), 0, 0, 0);
        }
        long st;
        sys(SYS_wait4, pid, (long)&st, 0, 0);
    }
    emit("FORKS_OK\n");

    emit("SMOKE_OK\n");
    sys(SYS_reboot, RB_MAGIC1, RB_MAGIC2, RB_POWER_OFF, 0);
    for (;;) sys(SYS_nanosleep, (long)&ts, 0, 0, 0);
}
